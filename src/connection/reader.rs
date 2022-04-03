use crate::connection::waker::{new_waker, Wakers};
use crate::connection::WsConnectionInner;
use crate::message::WsMessageKind;
use futures::{AsyncRead, AsyncWrite};
use std::io;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

pub struct WsMessageReader<T: AsyncRead + AsyncWrite + Unpin> {
    kind: WsMessageKind,
    parent: Option<Arc<Mutex<(WsConnectionInner<T>, Wakers)>>>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsMessageReader<T> {
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

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncRead for WsMessageReader<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        if buf.len() == 0 {
            return Poll::Ready(Ok(0));
        }
        if let Some(parent) = &self.parent {
            let waker = new_waker(Arc::downgrade(parent));
            let mut guard = parent.lock().unwrap();
            let (inner, wakers) = guard.deref_mut();
            wakers.reader_waker = Some(cx.waker().clone());
            let n = match inner.poll_read(&mut Context::from_waker(&waker), buf) {
                Poll::Ready(r) => match r {
                    Ok(r) => r,
                    Err(err) => {
                        wakers.wake();
                        return Poll::Ready(Err(err));
                    }
                },
                Poll::Pending => return Poll::Pending,
            };
            if n == 0 {
                inner.detach_reader();
                drop(guard);
                self.parent.take();
            }
            return Poll::Ready(Ok(n));
        }
        Poll::Ready(Ok(0))
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Drop for WsMessageReader<T> {
    fn drop(&mut self) {
        if let Some(parent) = self.parent.take() {
            let mut guard = parent.lock().unwrap();
            let (inner, wakers) = guard.deref_mut();
            inner.detach_reader();
            wakers.reader_waker.take();
        }
    }
}
