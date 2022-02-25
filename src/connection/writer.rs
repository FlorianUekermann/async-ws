use crate::connection::waker::new_waker;
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
            Some(inner) => {
                let waker = new_waker(Arc::downgrade(inner));
                let mut inner = inner.lock().unwrap();
                inner.writer_waker = Some(cx.waker().clone());
                inner.poll_write(&mut Context::from_waker(&waker), buf)
            }
            None => Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &self.inner {
            Some(inner) => {
                let waker = new_waker(Arc::downgrade(inner));
                let mut inner = inner.lock().unwrap();
                inner.writer_waker = Some(cx.waker().clone());
                inner.poll_flush(&mut Context::from_waker(&waker))
            }
            None => Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe))),
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &self.inner {
            Some(inner) => {
                let waker = new_waker(Arc::downgrade(inner));
                let mut inner = inner.lock().unwrap();
                inner.writer_waker = Some(cx.waker().clone());
                let p = inner.poll_close_writer(&mut Context::from_waker(&waker));
                if let Poll::Ready(Ok(())) = &p {
                    inner.detach_writer();
                    drop(inner);
                    self.inner.take();
                }
                p
            }
            None => Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe))),
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Drop for WsMessageWriter<T> {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.lock().unwrap().detach_writer();
        }
    }
}
