use crate::connection::encode::EncodeState::Sending;
use crate::frame::{
    payload_mask, FrameHead, WsControlFrame, WsControlFramePayload, WsDataFrameKind, WsFrameKind,
};
use crate::message::WsMessageKind;
use futures::task::{Context, Poll};
use futures::{io, AsyncRead, AsyncWrite};
use rand::{thread_rng, RngCore};
use std::pin::Pin;

const FRAME_BUFFER_PAYLOAD_OFFSET: usize = 8;

#[derive(Debug)]
pub(crate) enum EncodeState {
    Sending {
        frame_in_progress: Option<FrameInProgress>,
        next_data_frame_kind: Option<WsDataFrameKind>,
        queued_control: Option<WsControlFrame>,
    },
    Closed(WsControlFramePayload),
    Failed,
}

#[derive(Debug)]
pub struct FrameInProgress {
    kind: WsFrameKind,
    buffer: [u8; 1300],
    mask: [u8; 4],
    flushed: Option<usize>,
    filled: usize,
}

#[derive(Debug)]
enum FrameInProgressReady {
    Buffering,
    Flushed,
}

impl FrameInProgress {
    fn new(kind: WsFrameKind, mask: bool) -> Self {
        FrameInProgress {
            kind,
            buffer: [0u8; 1300],
            mask: match mask {
                true => thread_rng().next_u32().to_ne_bytes(),
                false => [0u8, 0u8, 0u8, 0u8],
            },
            flushed: None,
            filled: FRAME_BUFFER_PAYLOAD_OFFSET,
        }
    }
    fn new_control(control: WsControlFrame, mask: bool) -> Self {
        let mut frame = Self::new(control.kind().frame_kind(), mask);
        frame.append_data(control.payload());
        frame.start_flushing(true);
        frame
    }
    fn append_data(&mut self, buf: &[u8]) -> usize {
        assert!(self.flushed.is_none());
        let payload_len = self.buffer.len() - self.filled;
        let append = buf.len().min(payload_len);
        let target_slice = &mut self.buffer[self.filled..self.filled + append];
        target_slice.copy_from_slice(&buf[..append]);
        payload_mask(self.mask, payload_len, target_slice);
        self.filled += append;
        if append < buf.len() {
            self.start_flushing(false)
        }
        append
    }
    fn start_flushing(&mut self, fin: bool) {
        assert!(self.flushed.is_none());
        let head = FrameHead {
            fin,
            opcode: self.kind.opcode(),
            mask: self.mask,
            payload_len: (self.filled - FRAME_BUFFER_PAYLOAD_OFFSET) as u64,
        };
        let offset = FRAME_BUFFER_PAYLOAD_OFFSET - head.len_bytes();
        head.encode(&mut self.buffer[offset..FRAME_BUFFER_PAYLOAD_OFFSET]);
        self.flushed = Some(offset);
    }
    fn poll<T: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<FrameInProgressReady>> {
        let mut offset = match self.flushed {
            None => return Poll::Ready(Ok(FrameInProgressReady::Buffering)),
            Some(offset) => offset,
        };
        loop {
            match Pin::new(&mut *transport).poll_write(cx, &self.buffer[offset..self.filled]) {
                Poll::Ready(Ok(n)) => {
                    offset += n;
                    self.flushed = Some(offset);
                    if offset == self.filled {
                        return Poll::Ready(Ok(FrameInProgressReady::Flushed));
                    }
                }
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum EncodeStateReady {
    Buffering,
    FlushedFrame,
    FlushedMessage,
}

impl EncodeState {
    pub fn new() -> EncodeState {
        EncodeState::Sending {
            frame_in_progress: None,
            next_data_frame_kind: None,
            queued_control: None,
        }
    }
    pub fn start_message(&mut self, kind: WsMessageKind) {
        if let Sending { next_data_frame_kind, frame_in_progress: None, .. } = self {
            assert!(next_data_frame_kind.is_none());
            *next_data_frame_kind = Some(kind.frame_kind());
        } else {
            unreachable!()
        }
    }
    pub fn end_message(&mut self, mask: bool) {
        if let Sending { next_data_frame_kind, frame_in_progress, .. } = self {
            if let Some(kind) = next_data_frame_kind.take() {
                if let Some(frame) = frame_in_progress {
                    frame.start_flushing(true);
                } else {
                    let mut frame = FrameInProgress::new(kind.frame_kind(), mask);
                    frame.start_flushing(true);
                    *frame_in_progress = Some(frame);
                }
            }
        }
    }
    pub fn append_data(&mut self, buf: &[u8], mask: bool) -> usize {
        if let Sending { next_data_frame_kind, frame_in_progress, .. } = self {
            let kind = next_data_frame_kind.replace(WsDataFrameKind::Continuation).unwrap();
            frame_in_progress
                .get_or_insert_with(|| FrameInProgress::new(kind.frame_kind(), mask))
                .append_data(buf)
        } else {
            unreachable!()
        }
    }
    pub fn start_flushing(&mut self) {
        if let Sending { frame_in_progress: Some(frame), .. } = self {
            frame.start_flushing(false);
        } else {
            unreachable!()
        }
    }
    pub fn poll<T: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
        mask: bool,
    ) -> Poll<io::Result<EncodeStateReady>> {
        loop {
            match self {
                Sending {
                    frame_in_progress,
                    next_data_frame_kind,
                    queued_control,
                } => {
                    if let Some(frame) = frame_in_progress {
                        match frame.poll(transport, cx) {
                            Poll::Ready(Ok(FrameInProgressReady::Buffering)) =>
                                return Poll::Ready(Ok(EncodeStateReady::Buffering)),
                            Poll::Ready(Ok(FrameInProgressReady::Flushed)) => *frame_in_progress = None,
                            Poll::Ready(Err(err)) => {
                                *self = Self::Failed;
                                return Poll::Ready(Err(err));
                            }
                            Poll::Pending => return Poll::Pending,
                        }
                    } else if let Some(control) = queued_control.take() {
                        *frame_in_progress = Some(FrameInProgress::new_control(control, mask));
                    } else {
                        return match next_data_frame_kind {
                            Some(_) => Poll::Ready(Ok(EncodeStateReady::FlushedFrame)),
                            None => Poll::Ready(Ok(EncodeStateReady::FlushedMessage)),
                        };
                    }
                }
                _ => panic!("idk"),
            }
        }
    }
}
