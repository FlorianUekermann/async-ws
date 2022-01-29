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
        frame: Option<FrameInProgress>,
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
    sent: usize,
    filled: usize,
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
            sent: 0,
            filled: FRAME_BUFFER_PAYLOAD_OFFSET,
        }
    }
    fn append_data(&mut self, buf: &[u8]) -> usize {
        if self.sent > 0 {
            return 0;
        }
        let payload_len = self.buffer.len() - self.filled;
        let append = buf.len().min(payload_len);
        let target_slice = &mut self.buffer[self.filled..self.filled + append];
        target_slice.copy_from_slice(&buf[..append]);
        payload_mask(self.mask, payload_len, target_slice);
        self.filled += append;
        append
    }
    fn finalize_and_get_offset(&mut self, fin: bool) -> usize {
        let head = FrameHead {
            fin,
            opcode: self.kind.opcode(),
            mask: self.mask,
            payload_len: (self.filled - FRAME_BUFFER_PAYLOAD_OFFSET) as u64,
        };
        let offset = FRAME_BUFFER_PAYLOAD_OFFSET - head.len_bytes();
        head.encode(&mut self.buffer[offset..FRAME_BUFFER_PAYLOAD_OFFSET]);
        offset
    }
    fn full(&self) -> bool {
        self.filled == self.buffer.len()
    }
    fn poll_write<T: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
        fin: bool,
    ) -> Poll<io::Result<()>> {
        let mut offset = self.sent;
        if offset == 0 {
            offset = self.finalize_and_get_offset(fin);
        }
        match Pin::new(&mut *transport).poll_write(cx, &self.buffer[offset..self.filled]) {
            Poll::Ready(Ok(n)) => {
                if n != 0 {
                    self.sent = offset + n;
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl EncodeState {
    pub fn new() -> EncodeState {
        EncodeState::Sending {
            frame: None,
            next_data_frame_kind: None,
            queued_control: None,
        }
    }
    pub fn start_message(&mut self, kind: WsMessageKind) -> bool {
        match self {
            Sending {
                next_data_frame_kind,
                ..
            } => {
                if next_data_frame_kind.is_some() {
                    return false;
                }
                *next_data_frame_kind = Some(kind.frame_kind());
                true
            }
            _ => panic!("idk"),
        }
    }
    pub fn append_frame(&mut self, buf: &[u8], fin: bool, mask: bool) -> usize {
        match self {
            Sending {
                next_data_frame_kind,
                frame,
                ..
            } => {
                if let Some(kind) = *next_data_frame_kind {
                    if fin {
                        *next_data_frame_kind = None;
                    } else if buf.len() == 0 {
                        return 0;
                    } else {
                        *next_data_frame_kind = Some(WsDataFrameKind::Continuation);
                    }
                    frame
                        .get_or_insert_with(|| FrameInProgress::new(kind.frame_kind(), mask))
                        .append_data(buf)
                } else {
                    panic!("idk")
                }
            }
            _ => panic!("idk"),
        }
    }

    pub fn poll<T: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
    ) -> Option<io::Error> {
        loop {
            match self {
                Sending {
                    frame,
                    next_data_frame_kind,
                    queued_control,
                } => {
                    if let Some(frame_in_progress) = frame {
                        let fin = frame_in_progress.kind.is_control()
                            || *next_data_frame_kind != Some(WsDataFrameKind::Continuation);
                        if queued_control.is_some() || frame_in_progress.full() || fin {
                            match frame_in_progress.poll_write(transport, cx, fin) {
                                Poll::Ready(Ok(())) => {
                                    if frame_in_progress.sent == frame_in_progress.filled {
                                        *frame = None
                                    }
                                }
                                Poll::Ready(Err(err)) => {
                                    *self = Self::Failed;
                                    return Some(err);
                                }
                                Poll::Pending => return None,
                            }
                            continue;
                        }
                    }
                    return None;
                }
                _ => panic!("idk"),
            }
        }
    }
}
