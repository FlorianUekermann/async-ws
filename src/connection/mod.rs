mod decode;
mod encode;

use crate::connection::decode::{DecodeEvent, DecodeState};
use crate::connection::encode::EncodeState;
use crate::frame::{FrameDecodeError, WsDataFrameKind};
use crate::message::WsMessageKind;
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
    decode_state: DecodeState,
    encode_state: EncodeState,
    tmp: bool,
    // write_state: WsConnectionWriteState,
}

impl WsConnectionState {
    fn with_config(config: WsConfig) -> Self {
        Self {
            tmp: false,
            config: config,
            decode_state: DecodeState::new(),
            encode_state: EncodeState::new(),
        }
    }
}

pub struct WsConnection<T: AsyncRead + AsyncWrite + Unpin> {
    transport: T,
    state: WsConnectionState,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsConnection<T> {
    pub fn start_message(&mut self, kind: WsMessageKind) {
        dbg!();
        if !self.state.encode_state.start_message(kind) {
            panic!("adsfasdfa")
        }
    }
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
        let Self {
            transport, state, ..
        } = self.get_mut();
        if state.tmp {
            return Poll::Ready(Ok(0));
        }
        loop {
            match dbg!(state.decode_state.poll(transport, cx, buf)) {
                DecodeEvent::MessageStart(_) => panic!("unexpected message"),
                DecodeEvent::Control(_) => continue,
                DecodeEvent::InternalProgress => continue,
                DecodeEvent::ReadProgress(n, fin) => {
                    state.tmp = fin;
                    break Poll::Ready(Ok(n));
                }
                DecodeEvent::Failure(_) => {
                    break Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()))
                }
                DecodeEvent::Pending => break Poll::Pending,
            }
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Stream for WsConnection<T> {
    type Item = Result<WsMessageKind, WsConnectionError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Self {
            transport, state, ..
        } = self.get_mut();
        loop {
            match dbg!(state.decode_state.poll(transport, cx, &mut [])) {
                DecodeEvent::MessageStart(kind) => {
                    state.tmp = false;
                    break Poll::Ready(Some(Ok(kind)));
                }
                DecodeEvent::Control(_) => continue,
                DecodeEvent::InternalProgress => continue,
                DecodeEvent::ReadProgress(_, _) => panic!("unexpected read"),
                DecodeEvent::Failure(err) => break Poll::Ready(Some(Err(err))),
                DecodeEvent::Pending => break Poll::Pending,
            }
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncWrite for WsConnection<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let Self { transport, state } = self.get_mut();
        match state.encode_state.poll(transport, cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(err)) => panic!("write err: {:?}", err),
            Poll::Pending => return Poll::Pending,
        }
        dbg!(&state.encode_state);
        return Poll::Ready(Ok(dbg!(state.encode_state.append_frame(
            buf,
            false,
            state.config.mask
        ))));
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        panic!("idk")
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let Self {
            transport, state, ..
        } = self.get_mut();
        state
            .encode_state
            .append_frame(&[], true, state.config.mask);
        match dbg!(state.encode_state.poll(transport, cx)) {
            Poll::Ready(Ok(())) => return Poll::Ready(Ok(())),
            Poll::Ready(Err(err)) => panic!(err),
            Poll::Pending => return Poll::Pending,
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum WsConnectionError {
    #[error("invalid utf8 in text message")]
    InvalidUtf8,
    #[error("incomplete utf8 in text message")]
    IncompleteUtf8,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    FrameDecodeError(#[from] FrameDecodeError),
    #[error("unexpected: {0:?}")]
    UnexpectedFrameKind(WsDataFrameKind),
}
