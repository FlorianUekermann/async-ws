use crate::connection::writer::WsMessageWriter;
use crate::connection::{WsConnectionError, WsConnectionInner};
use crate::message::WsMessageKind;
use futures::{AsyncRead, AsyncWrite, Future};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use crate::connection::waker::new_waker;

#[derive(Clone)]
pub struct WsSend<T: AsyncRead + AsyncWrite + Unpin> {
    kind: WsMessageKind,
    inner: Arc<Mutex<WsConnectionInner<T>>>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsSend<T> {
    pub(crate) fn new(inner: &Arc<Mutex<WsConnectionInner<T>>>, kind: WsMessageKind) -> Self {
        Self {
            kind,
            inner: inner.clone(),
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Future for WsSend<T> {
    type Output = Result<WsMessageWriter<T>, WsConnectionError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let waker = new_waker(Arc::downgrade(&self.inner));
        let mut inner = self.inner.lock().unwrap();
        inner.send_waker = Some(cx.waker().clone());
        inner.poll_next_writer(self.kind, &mut Context::from_waker(&waker))
            .map(|r| r.map(|()| WsMessageWriter::new(self.kind, &self.inner)))
    }
}
