use crate::connection::waker::{new_waker, Wakers};
use crate::connection::WsConnectionInner;
use crate::message::WsMessageKind;
use futures::{AsyncRead, AsyncWrite};
use std::io;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

pub struct WsMessageWriter<T: AsyncRead + AsyncWrite + Unpin> {
    kind: WsMessageKind,
    parent: Option<Arc<Mutex<(WsConnectionInner<T>, Wakers)>>>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsMessageWriter<T> {
    pub(crate) fn new(
        kind: WsMessageKind,
        parent: &Arc<Mutex<(WsConnectionInner<T>, Wakers)>>,
    ) -> Self {
        Self {
            kind,
            parent: Some(parent.clone()),
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
        match &self.parent {
            Some(parent) => {
                let waker = new_waker(Arc::downgrade(parent));
                let mut guard = parent.lock().unwrap();
                let (inner, wakers) = guard.deref_mut();
                wakers.writer_waker = Some(cx.waker().clone());
                inner.poll_write(&mut Context::from_waker(&waker), buf)
            }
            None => Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &self.parent {
            Some(parent) => {
                let waker = new_waker(Arc::downgrade(parent));
                let mut guard = parent.lock().unwrap();
                let (inner, wakers) = guard.deref_mut();
                wakers.writer_waker = Some(cx.waker().clone());
                inner.poll_flush(&mut Context::from_waker(&waker))
            }
            None => Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe))),
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &self.parent {
            Some(parent) => {
                let waker = new_waker(Arc::downgrade(parent));
                let mut guard = parent.lock().unwrap();
                let (inner, wakers) = guard.deref_mut();
                wakers.writer_waker = Some(cx.waker().clone());
                let p = inner.poll_close_writer(&mut Context::from_waker(&waker));
                if let Poll::Ready(Ok(())) = &p {
                    inner.detach_writer();
                    drop(guard);
                    self.parent.take();
                }
                p
            }
            None => Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe))),
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Drop for WsMessageWriter<T> {
    fn drop(&mut self) {
        if let Some(parent) = self.parent.take() {
            let mut guard = parent.lock().unwrap();
            let (inner, wakers) = guard.deref_mut();
            inner.detach_writer();
            wakers.writer_waker.take();
        }
    }
}
