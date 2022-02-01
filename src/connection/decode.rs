use crate::connection::WsConnectionError;
use crate::frame::{
    FrameDecoderState, FramePayloadReaderState, WsControlFrame, WsControlFrameKind,
    WsControlFramePayload, WsFrame,
};
use crate::message::WsMessageKind;
use futures::task::{Context, Poll};
use futures::{AsyncRead, AsyncWrite};
use std::ops::ControlFlow;
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
    Closed(WsControlFramePayload),
    Failed(WsConnectionError),
}

enum Event {
    Control(WsControlFrame),
    Read(usize),
    Err(WsConnectionError),
    Blocked,
}

impl DecodeState {
    pub(crate) fn new() -> Self {
        Self::WaitingForMessageStart {
            frame_decoder: FrameDecoderState::new(),
        }
    }
    // Returns Break(Pending) if progress is blocked by transport or the decoder is stuck in a state
    // which requires a transition that isn't always appropriate, such as consuming a message start.
    // Returns Break(Ready) if data has been written to the buffer.
    // Returns Continue(frame) if a control frame has been received.
    pub fn poll<T: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> ControlFlow<Poll<usize>, WsControlFrame> {
        loop {
            if let ControlFlow::Break(event) = self.poll_loop_body(transport, cx, buf) {
                break match event {
                    Event::Control(frame) => ControlFlow::Continue(frame),
                    Event::Read(n) => ControlFlow::Break(Poll::Ready(n)),
                    Event::Err(err) => {
                        *self = Self::Failed(err);
                        ControlFlow::Break(Poll::Pending)
                    }
                    Event::Blocked => ControlFlow::Break(Poll::Pending),
                };
            }
        }
    }
    fn poll_loop_body<T: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> ControlFlow<Event> {
        match self {
            DecodeState::WaitingForMessageStart { frame_decoder } => {
                match Self::ready_ok_or_break(frame_decoder.poll(transport, cx))? {
                    WsFrame::Control(frame) => self.process_control_frame(frame),
                    WsFrame::Data(frame) => match frame.kind.message_kind() {
                        Some(kind) => {
                            *self = Self::MessageStart {
                                kind,
                                first_frame_mask: frame.mask,
                                fin: frame.fin,
                                first_frame_payload_len: frame.payload_len,
                            };
                            ControlFlow::Continue(())
                        }
                        None => ControlFlow::Break(Event::Err(
                            WsConnectionError::UnexpectedFrameKind(frame.kind.into()),
                        )),
                    },
                }
            }
            DecodeState::WaitingForMessageContinuation {
                frame_decoder,
                utf8: utf8_validator,
            } => match Self::ready_ok_or_break(frame_decoder.poll(transport, cx))? {
                WsFrame::Control(frame) => self.process_control_frame(frame),
                WsFrame::Data(frame) => match frame.kind.message_kind() {
                    None => {
                        *self = Self::ReadingDataFramePayload {
                            payload: frame.payload_reader(),
                            fin: frame.fin,
                            utf8: *utf8_validator,
                        };
                        ControlFlow::Continue(())
                    }
                    Some(_) => ControlFlow::Break(Event::Err(
                        WsConnectionError::UnexpectedFrameKind(frame.kind.into()),
                    )),
                },
            },
            DecodeState::ReadingDataFramePayload { payload, fin, utf8 } => {
                let n = Self::ready_ok_or_break(payload.poll_read(transport, cx, buf))?;
                let frame_finished = payload.finished();
                Self::validate_utf8(utf8, &buf[0..n], *fin && frame_finished)?;
                if frame_finished {
                    *self = match fin {
                        true => Self::MessageEnd,
                        false => Self::WaitingForMessageContinuation {
                            frame_decoder: FrameDecoderState::new(),
                            utf8: *utf8,
                        },
                    };
                }
                match n {
                    0 => ControlFlow::Break(Event::Blocked),
                    _ => ControlFlow::Break(Event::Read(n)),
                }
            }
            _ => ControlFlow::Break(Event::Blocked),
        }
    }

    fn ready_ok_or_break<T, E: Into<WsConnectionError>>(
        p: Poll<Result<T, E>>,
    ) -> ControlFlow<Event, T> {
        match p {
            Poll::Ready(r) => match r {
                Ok(ok) => ControlFlow::Continue(ok),
                Err(err) => ControlFlow::Break(Event::Err(err.into())),
            },
            Poll::Pending => ControlFlow::Break(Event::Blocked),
        }
    }
    fn process_control_frame(&mut self, frame: WsControlFrame) -> ControlFlow<Event> {
        if frame.kind == WsControlFrameKind::Close {
            *self = DecodeState::Closed(frame.payload);
        }
        ControlFlow::Break(Event::Control(frame))
    }
    fn validate_utf8(
        state: &mut Option<Incomplete>,
        input: &[u8],
        fin: bool,
    ) -> ControlFlow<Event> {
        if let Some(state) = state {
            for byte in input {
                if let Some((Err(_), _)) = state.try_complete(std::slice::from_ref(byte)) {
                    return ControlFlow::Break(Event::Err(WsConnectionError::InvalidUtf8));
                }
            }
            if fin && !state.is_empty() {
                return ControlFlow::Break(Event::Err(WsConnectionError::IncompleteUtf8));
            }
        }
        ControlFlow::Continue(())
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
    pub fn at_message_end(&self) -> bool {
        match self {
            Self::MessageEnd => true,
            _ => false,
        }
    }
    pub fn take_message_end(&mut self) -> bool {
        if self.at_message_end() {
            *self = Self::new();
            return true;
        }
        false
    }
}
