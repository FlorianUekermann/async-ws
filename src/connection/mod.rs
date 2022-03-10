mod config;
mod decode;
mod encode;
mod inner;
mod open;
mod reader;
mod send;
mod waker;
mod writer;

pub use crate::connection::reader::WsMessageReader;
pub use crate::connection::send::WsSend;
pub use crate::connection::writer::WsMessageWriter;

use crate::connection::config::WsConfig;
use crate::connection::inner::WsConnectionInner;
use crate::connection::waker::{new_waker, Wakers};
use crate::frame::{FrameDecodeError, WsDataFrameKind};
use crate::message::WsMessageKind;
use futures::prelude::*;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

#[derive(Clone)]
pub struct WsConnection<T: AsyncRead + AsyncWrite + Unpin> {
    parent: Arc<Mutex<(WsConnectionInner<T>, Wakers)>>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsConnection<T> {
    pub fn with_config(transport: T, config: WsConfig) -> Self {
        Self {
            parent: Arc::new(Mutex::new((
                WsConnectionInner::with_config(transport, config),
                Wakers::default(),
            ))),
        }
    }
    pub fn send(&self, kind: WsMessageKind) -> WsSend<T> {
        WsSend::new(&self.parent, kind)
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Stream for WsConnection<T> {
    type Item = Result<WsMessageReader<T>, WsConnectionError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut guard = self.parent.lock().unwrap();
        let (inner, wakers) = guard.deref_mut();
        wakers.stream_waker = Some(cx.waker().clone());
        let waker = new_waker(Arc::downgrade(&self.parent));
        inner
            .poll_next_reader(&mut Context::from_waker(&waker))
            .map(|o| o.map(|r| r.map(|kind| WsMessageReader::new(kind, &self.parent))))
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
    #[error("timeout")]
    Timeout,
    #[error("unexpected frame kind")]
    UnexpectedFrameKind(#[from] WsDataFrameKind),
}
