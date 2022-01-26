use crate::frame::{FrameDecoderState, FramePayloadReaderState, WsDataFrame, WsControlFramePayload, WsFrame, WsControlFrame, WsControlFrameKind};
use utf8::Incomplete;
use futures::{AsyncRead, AsyncWrite};
use futures::task::{Context, Poll};
use crate::connection::{WsConnectionError};
use crate::message::WsMessageKind;
use std::ops::Deref;

pub(super) enum DecodeState {
    WaitingForMessageStart {
        frame_decoder: FrameDecoderState,
    },
    WaitingForMessageContinuation {
        frame_decoder: FrameDecoderState,
        utf8_validator: Option<Incomplete>,
    },
    ReadingDataFramePayload {
        payload_reader: FramePayloadReaderState,
        fin: bool,
        utf8_validator: Option<Incomplete>,
    },
    Closed(WsControlFramePayload),
    Failed,
}

#[derive(Debug)]
pub(crate) enum DecodeEvent {
    MessageStart(WsMessageKind),
    Control(WsControlFrame),
    InternalProgress,
    ReadProgress(usize, bool),
    Failure(WsConnectionError),
    Pending,
}

impl DecodeState {
    pub(crate) fn new() -> Self {
        Self::WaitingForMessageStart {
            frame_decoder: FrameDecoderState::new()
        }
    }
    pub fn poll<T: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> DecodeEvent {
        match self {
            DecodeState::WaitingForMessageStart { frame_decoder } | DecodeState::WaitingForMessageContinuation { frame_decoder, .. } => {
                match frame_decoder.poll(transport, cx) {
                    Poll::Ready(Ok(frame)) => match frame {
                        WsFrame::Control(frame) => self.process_control_frame(frame),
                        WsFrame::Data(frame) => self.process_data_frame_head(frame),
                    }
                    Poll::Pending => DecodeEvent::Pending,
                    Poll::Ready(Err(err)) => {
                        *self = Self::Failed;
                        DecodeEvent::Failure(WsConnectionError::FrameDecodeError(err))
                    }
                }
            }
            DecodeState::ReadingDataFramePayload { payload_reader, fin, utf8_validator } => {
                match payload_reader.poll_read(transport, cx, buf) {
                    Poll::Ready(Ok(n)) => {
                        let frame_finished = payload_reader.finished();
                        if let Some(validator) = utf8_validator {
                            if let Some(err) = Self::process_utf8(validator, &buf[0..n], *fin && frame_finished) {
                                *self = Self::Failed;
                                return DecodeEvent::Failure(err);
                            }
                        }
                        if frame_finished {
                            *self = match fin {
                                true => Self::new(),
                                false => Self::WaitingForMessageContinuation { frame_decoder: FrameDecoderState::new(), utf8_validator: *utf8_validator }
                            }
                        }
                        DecodeEvent::ReadProgress(n, frame_finished)
                    }
                    Poll::Pending => DecodeEvent::Pending,
                    Poll::Ready(Err(err)) => {
                        *self = Self::Failed;
                        DecodeEvent::Failure(WsConnectionError::Io(err))
                    }
                }
            }
            _ => panic!("idk")
        }
    }

    fn process_data_frame_head(&mut self, frame: WsDataFrame) -> DecodeEvent {
        match (self.deref(), frame.kind.message_kind()) {
            (Self::WaitingForMessageStart { .. }, Some(kind)) => {
                *self = Self::ReadingDataFramePayload {
                    payload_reader: FramePayloadReaderState::new(frame.mask, frame.payload_len),
                    fin: frame.fin,
                    utf8_validator: match kind {
                        WsMessageKind::Text => Some(Incomplete::empty()),
                        WsMessageKind::Binary => None,
                    },
                };
                DecodeEvent::MessageStart(kind)
            }
            (Self::WaitingForMessageContinuation { utf8_validator, .. }, None) => {
                *self = Self::ReadingDataFramePayload {
                    payload_reader: FramePayloadReaderState::new(frame.mask, frame.payload_len),
                    fin: frame.fin,
                    utf8_validator: *utf8_validator,
                };
                DecodeEvent::InternalProgress
            }
            _ => {
                *self = Self::Failed;
                DecodeEvent::Failure(WsConnectionError::UnexpectedFrameKind(frame.kind))
            }
        }
    }
    fn process_control_frame(&mut self, frame: WsControlFrame) -> DecodeEvent {
        match frame.kind {
            WsControlFrameKind::Close => {
                *self = DecodeState::Closed(frame.payload)
            }
            _ => {}
        }
        DecodeEvent::Control(frame)
    }
    fn process_utf8(state: &mut Incomplete, input: &[u8], fin: bool) -> Option<WsConnectionError> {
        for byte in input {
            if let Some((Err(_), _)) = state.try_complete(std::slice::from_ref(byte)) {
                return Some(WsConnectionError::InvalidUtf8);
            }
        }
        if fin && !state.is_empty() {
            return Some(WsConnectionError::IncompleteUtf8);
        }
        None
    }
}