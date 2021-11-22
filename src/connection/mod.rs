use crate::frame::{
    FrameDecodeError, FrameDecoderState, FramePayloadReaderState, WsControlFrame,
    WsControlFrameKind, WsControlFramePayload, WsDataFrame, WsDataFrameKind, WsFrame,
};
use futures::prelude::*;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

pub struct WsConfig {
    pub mask: bool,
    _private: (),
}

impl WsConfig {
    pub fn client() -> Self {
        Self {
            mask: true,
            _private: (),
        }
    }
    pub fn server() -> Self {
        Self {
            mask: false,
            _private: (),
        }
    }
}

pub struct WsConnectionState {
    config: WsConfig,
    frame_decode: FrameDecoderState,
    frame_payload_reader: Option<FramePayloadReaderState>,
    frame_fin: bool,
    closed: Option<WsControlFramePayload>,
    queued_pong: Option<WsControlFramePayload>,
    error: Option<Option<WsConnectionError>>,
}

impl WsConnectionState {
    fn with_config(config: WsConfig) -> Self {
        Self {
            config,
            frame_decode: FrameDecoderState::new(),
            frame_payload_reader: None,
            frame_fin: true,
            closed: None,
            queued_pong: None,
            error: None,
        }
    }
    pub fn poll_read<T: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
        mut buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let mut filled = 0usize;
        loop {
            self.poll_write(transport, cx);
            if self.frame_payload_reader.is_none() && self.frame_fin {
                return Poll::Ready(Ok(0));
            }
            if self.error.is_some() {
                return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into()));
            }
            if buf.len() == 0 {
                return Poll::Ready(Ok(filled));
            }
            match &mut self.frame_payload_reader {
                Some(reader) => match reader.poll_read(transport, cx, buf) {
                    Poll::Ready(Ok(0)) => {
                        self.frame_payload_reader = None;
                        if self.frame_fin {
                            return Poll::Ready(Ok(filled));
                        }
                    }
                    Poll::Ready(Ok(n)) => {
                        buf = &mut buf[n..];
                        filled += n
                    }
                    Poll::Ready(Err(err)) => self.error = Some(Some(err.into())),
                    Poll::Pending => match filled {
                        0 => return Poll::Pending,
                        _ => return Poll::Ready(Ok(filled)),
                    },
                },
                None => match self.frame_decode.poll(transport, cx) {
                    Poll::Ready(Ok(frame)) => {
                        self.frame_decode = FrameDecoderState::new();
                        match frame {
                            WsFrame::Control(WsControlFrame { kind, payload }) => match kind {
                                WsControlFrameKind::Ping => self.queued_pong = Some(payload),
                                WsControlFrameKind::Pong => log::info!("pong"),
                                WsControlFrameKind::Close => self.closed = Some(payload),
                            },
                            WsFrame::Data(WsDataFrame {
                                kind,
                                mask,
                                payload_len,
                                fin,
                            }) => match kind {
                                WsDataFrameKind::Continuation => {
                                    self.frame_payload_reader =
                                        Some(FramePayloadReaderState::new(mask, payload_len));
                                    self.frame_fin = fin;
                                }
                                kind => {
                                    self.error =
                                        Some(Some(WsConnectionError::UnexpectedFrameKind(kind)))
                                }
                            },
                        }
                    }
                    Poll::Ready(Err(err)) => self.error = Some(Some(err.into())),
                    Poll::Pending => {}
                },
            }
        }
    }
    pub fn poll_next<T: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
    ) -> Poll<Option<<WsConnection<T> as Stream>::Item>> {
        loop {
            self.poll_write(transport, cx);
            if let Some(err) = &mut self.error {
                return Poll::Ready(err.take().map(|err| Err(err)));
            }
            if self.frame_payload_reader.is_some() || !self.frame_fin {
                return Poll::Pending;
            }
            match self.frame_decode.poll(transport, cx) {
                Poll::Ready(Ok(frame)) => {
                    self.frame_decode = FrameDecoderState::new();
                    match frame {
                        WsFrame::Control(WsControlFrame { kind, payload }) => match kind {
                            WsControlFrameKind::Ping => self.queued_pong = Some(payload),
                            WsControlFrameKind::Pong => log::info!("pong"),
                            WsControlFrameKind::Close => self.closed = Some(payload),
                        },
                        WsFrame::Data(WsDataFrame {
                            kind,
                            mask,
                            payload_len,
                            fin,
                        }) => match kind {
                            WsDataFrameKind::Text | WsDataFrameKind::Binary => {
                                self.frame_payload_reader =
                                    Some(FramePayloadReaderState::new(mask, payload_len));
                                self.frame_fin = fin;
                                return Poll::Ready(Some(Ok(kind)));
                            }
                            kind => {
                                self.error =
                                    Some(Some(WsConnectionError::UnexpectedFrameKind(kind)))
                            }
                        },
                    }
                }
                Poll::Ready(Err(err)) => self.error = Some(Some(err.into())),
                Poll::Pending => {}
            }
        }
    }
    fn poll_write<T: AsyncWrite + Unpin>(&mut self, _transport: &mut T, _cx: &mut Context<'_>) {}
}

pub struct WsConnection<T: AsyncRead + AsyncWrite + Unpin> {
    transport: T,
    state: WsConnectionState,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsConnection<T> {
    pub fn with_config(transport: T, config: WsConfig) -> Self {
        Self {
            transport,
            state: WsConnectionState::with_config(config),
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncRead for WsConnection<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let Self { transport, state } = self.get_mut();
        state.poll_read(transport, cx, buf)
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Stream for WsConnection<T> {
    type Item = Result<WsDataFrameKind, WsConnectionError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Self { transport, state } = self.get_mut();
        state.poll_next(transport, cx)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum WsConnectionError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    FrameDecodeError(#[from] FrameDecodeError),
    #[error("unexpected: {0:?}")]
    UnexpectedFrameKind(WsDataFrameKind),
}
