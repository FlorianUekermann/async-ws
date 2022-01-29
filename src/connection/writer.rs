use crate::connection::WsConnectionInner;
use crate::message::WsMessageKind;
use futures::{AsyncRead, AsyncWrite};
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

pub struct WsMessageWriter<T: AsyncRead + AsyncWrite + Unpin> {
    kind: WsMessageKind,
    inner: Option<Arc<Mutex<WsConnectionInner<T>>>>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsMessageWriter<T> {
    pub(crate) fn new(kind: WsMessageKind, inner: &Arc<Mutex<WsConnectionInner<T>>>) -> Self {
        Self {
            kind,
            inner: Some(inner.clone()),
        }
    }
    pub fn kind(&self) -> WsMessageKind {
        self.kind
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncWrite for WsMessageWriter<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &self.inner {
            Some(inner) => inner.lock().unwrap().poll_write(cx, buf, false),
            None => Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &self.inner {
            Some(inner) => match inner.lock().unwrap().poll_write(cx, &[], true) {
                Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
                _ => Poll::Ready(Ok(())),
            },
            None => Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe))),
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Drop for WsMessageWriter<T> {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.lock().unwrap().detach_reader();
        }
    }
}
