use crate::connection::WsConnectionError;
use crate::frame::{
    FrameDecoderState, FramePayloadReaderState, WsControlFrame, WsControlFrameKind,
    WsFrame,
};
use crate::message::WsMessageKind;
use futures::task::{Context, Poll};
use futures::{AsyncRead, AsyncWrite};

use utf8::Incomplete;


pub(super) enum DecodeState {
    WaitingForMessageStart {
        frame_decoder: FrameDecoderState,
    },
    MessageStart {
        kind: WsMessageKind,
        first_frame_mask: [u8; 4],
        first_frame_payload_len: u64,
        fin: bool,
    },
    WaitingForMessageContinuation {
        frame_decoder: FrameDecoderState,
        utf8: Option<Incomplete>,
    },
    ReadingDataFramePayload {
        payload: FramePayloadReaderState,
        fin: bool,
        utf8: Option<Incomplete>,
    },
    MessageEnd,
    Control {
        frame: WsControlFrame,
        continue_message: Option<Option<Incomplete>>,
    },
    Err(WsConnectionError),
    Done,
}

#[derive(Copy, Clone, Debug)]
pub(crate) enum DecodeReady {
    Control(WsControlFrameKind),
    MessageStart,
    MessageData,
    MessageEnd,
    Error,
    Done,
}

impl DecodeState {
    pub(crate) fn new() -> Self {
        Self::WaitingForMessageStart {
            frame_decoder: FrameDecoderState::new(),
        }
    }
    pub(crate) fn poll<T: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
    ) -> Poll<DecodeReady> {
        match self {
            DecodeState::WaitingForMessageStart { frame_decoder } => {
                match frame_decoder.poll(transport, cx) {
                    Poll::Ready(Ok(WsFrame::Control(frame))) => {
                        *self = Self::Control { frame, continue_message: None };
                        Poll::Ready(DecodeReady::Control(frame.kind()))
                    }
                    Poll::Ready(Ok(WsFrame::Data(frame))) => match frame.kind.message_kind() {
                        Some(kind) => {
                            *self = Self::MessageStart {
                                kind,
                                first_frame_mask: frame.mask,
                                fin: frame.fin,
                                first_frame_payload_len: frame.payload_len,
                            };
                            Poll::Ready(DecodeReady::MessageStart)
                        }
                        None => {
                            self.set_err(frame.kind().into());
                            Poll::Ready(DecodeReady::Error)
                        },
                    },
                    Poll::Ready(Err(err)) => {
                        self.set_err(err.into());
                        Poll::Ready(DecodeReady::Error)
                    }
                    Poll::Pending => Poll::Pending
                }
            }
            DecodeState::WaitingForMessageContinuation {
                frame_decoder,
                utf8,
            } => match frame_decoder.poll(transport, cx) {
                Poll::Ready(Ok(WsFrame::Control(frame))) => {
                    *self = Self::Control { frame, continue_message: Some(*utf8) };
                    Poll::Ready(DecodeReady::Control(frame.kind()))
                }
                Poll::Ready(Ok(WsFrame::Data(frame))) => match frame.kind.message_kind() {
                    None => {
                        *self = Self::ReadingDataFramePayload {
                            payload: frame.payload_reader(),
                            fin: frame.fin,
                            utf8: *utf8,
                        };
                        Poll::Ready(DecodeReady::MessageData)
                    }
                    Some(_) => {
                        self.set_err(frame.kind.into());
                        Poll::Ready(DecodeReady::Error)
                    },
                },
                Poll::Ready(Err(err)) => {
                    self.set_err(err.into());
                    Poll::Ready(DecodeReady::Error)
                }
                Poll::Pending => Poll::Pending
            },
            DecodeState::ReadingDataFramePayload { .. } => Poll::Ready(DecodeReady::MessageData),
            DecodeState::Err(_) => Poll::Ready(DecodeReady::Error),
            DecodeState::Done => Poll::Ready(DecodeReady::Done),
            DecodeState::Control { frame, .. } => Poll::Ready(DecodeReady::Control(frame.kind())),
            DecodeState::MessageStart { .. } => Poll::Ready(DecodeReady::MessageStart),
            DecodeState::MessageEnd { .. } => Poll::Ready(DecodeReady::MessageEnd),
        }
    }
    pub(crate) fn poll_read<T: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<usize> {
        match self {
            DecodeState::ReadingDataFramePayload { payload, fin, utf8 } => {
                let n = match payload.poll_read(transport, cx, buf) {
                    Poll::Ready(Ok(n)) => n,
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(err)) => {
                        self.set_err(err.into());
                        return Poll::Ready(0);
                    }
                };
                let frame_finished = payload.finished();
                if let Err(err) = Self::validate_utf8(utf8, &buf[0..n], *fin && frame_finished) {
                    self.set_err(err)
                }
                if frame_finished {
                    *self = match fin {
                        true => Self::MessageEnd,
                        false => Self::WaitingForMessageContinuation {
                            frame_decoder: FrameDecoderState::new(),
                            utf8: *utf8,
                        },
                    };
                }
                Poll::Ready(n)
            }
            _ => Poll::Ready(0),
        }
    }
    pub fn set_err(&mut self, err: WsConnectionError) {
        *self = Self::Err(err)
    }
    fn validate_utf8(
        state: &mut Option<Incomplete>,
        input: &[u8],
        fin: bool,
    ) -> Result<(), WsConnectionError> {
        if let Some(state) = state {
            for byte in input {
                if let Some((Err(_), _)) = state.try_complete(std::slice::from_ref(byte)) {
                    return Err(WsConnectionError::InvalidUtf8);
                }
            }
            if fin && !state.is_empty() {
                return Err(WsConnectionError::IncompleteUtf8);
            }
        }
        Ok(())
    }
    pub fn take_message_start(&mut self) -> Option<WsMessageKind> {
        if let Self::MessageStart {
            kind,
            first_frame_mask,
            first_frame_payload_len,
            fin,
        } = self
        {
            let kind = *kind;
            *self = Self::ReadingDataFramePayload {
                payload: FramePayloadReaderState::new(*first_frame_mask, *first_frame_payload_len),
                fin: *fin,
                utf8: match kind {
                    WsMessageKind::Binary => None,
                    WsMessageKind::Text => Some(Incomplete::empty()),
                },
            };
            return Some(kind);
        }
        None
    }
    pub fn take_message_end(&mut self) -> bool {
        match self {
            Self::MessageEnd => {
                *self = Self::new();
                true
            }
            _ => false,
        }
    }
    pub fn take_control(&mut self) -> Option<WsControlFrame> {
        match self {
            Self::Control{frame, continue_message} => {
                let frame = *frame;
                *self = match continue_message {
                    Some(utf8) => Self::WaitingForMessageContinuation{
                        frame_decoder: FrameDecoderState::new(),
                        utf8: *utf8
                    },
                    None => Self::WaitingForMessageStart { frame_decoder: FrameDecoderState::new() }
                };
                Some(frame)
            }
            _ => None,
        }
    }
}
