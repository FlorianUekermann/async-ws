use crate::frame::{
    FrameHeadDecodeState, FrameHeadParseError, FramePayloadReaderState, WsControlFrame,
    WsControlFramePayload, WsDataFrame, WsFrame, WsFrameKind,
};
use futures::prelude::*;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Debug)]
pub struct FrameDecoder<T: AsyncRead + Unpin> {
    transport: Option<T>,
    state: FrameDecoderState,
}

impl<T: AsyncRead + Unpin> FrameDecoder<T> {
    pub fn checkpoint(self) -> Option<(T, FrameDecoderState)> {
        let (transport, state) = (self.transport, self.state);
        transport.map(|transport| (transport, state))
    }
}

#[derive(Debug)]
pub enum FrameDecoderState {
    Head(FrameHeadDecodeState),
    ControlPayload {
        frame: WsControlFrame,
        reader: FramePayloadReaderState,
    },
}

impl FrameDecoderState {
    pub fn new() -> Self {
        Self::Head(FrameHeadDecodeState::new())
    }
    pub fn restore<T: AsyncRead + Unpin>(self, transport: T) -> FrameDecoder<T> {
        FrameDecoder {
            transport: Some(transport),
            state: self,
        }
    }
    fn poll<T: AsyncRead + Unpin>(
        &mut self,
        transport: &mut Option<T>,
        cx: &mut Context<'_>,
    ) -> Poll<<FrameDecoder<T> as Future>::Output> {
        loop {
            match self {
                FrameDecoderState::Head(state) => match state.poll(transport, cx) {
                    Poll::Ready(Ok((t, frame_head))) => match frame_head.opcode.frame_kind() {
                        WsFrameKind::Data(frame_kind) => {
                            return Poll::Ready(Ok((
                                t,
                                WsFrame::Data(WsDataFrame {
                                    kind: frame_kind,
                                    fin: frame_head.fin,
                                    mask: frame_head.mask,
                                    payload_len: frame_head.payload_len,
                                }),
                            )))
                        }
                        WsFrameKind::Control(frame_kind) => {
                            *self = FrameDecoderState::ControlPayload {
                                frame: WsControlFrame {
                                    kind: frame_kind,
                                    payload: WsControlFramePayload {
                                        len: 0,
                                        buffer: [0u8; 125],
                                    },
                                },
                                reader: FramePayloadReaderState::new(
                                    frame_head.mask,
                                    frame_head.payload_len,
                                ),
                            };
                            *transport = Some(t);
                        }
                    },
                    Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                    Poll::Pending => return Poll::Pending,
                },
                FrameDecoderState::ControlPayload { frame, reader } => {
                    let mut t = transport.take().unwrap();
                    match reader.poll_read(&mut t, cx, &mut frame.payload.buffer) {
                        Poll::Ready(Ok(n)) => {
                            if n == 0 {
                                return Poll::Ready(Ok((t, WsFrame::Control(*frame))));
                            }
                            *transport = Some(t);
                            frame.payload.len += n as u8
                        }
                        Poll::Ready(Err(err)) => return Poll::Ready(Err(err.into())),
                        Poll::Pending => {
                            *transport = Some(t);
                            return Poll::Pending;
                        }
                    }
                }
            }
        }
    }
}

impl<T: AsyncRead + Unpin> Future for FrameDecoder<T> {
    type Output = Result<(T, WsFrame), FrameDecodeError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let Self { transport, state } = self.get_mut();
        state.poll(transport, cx)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum FrameDecodeError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    ParseErr(#[from] FrameHeadParseError),
}
