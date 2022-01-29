use crate::connection::WsConnectionInner;
use futures::{AsyncRead, AsyncWrite};
use std::sync::{Mutex, Arc};
use std::task::{Context, Poll};
use std::pin::Pin;
use std::io;
use crate::message::WsMessageKind;

pub struct WsMessageReader<T: AsyncRead + AsyncWrite + Unpin> {
    kind: WsMessageKind,
    inner: Option<Arc<Mutex<WsConnectionInner<T>>>>
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsMessageReader<T> {
    pub (crate) fn new(kind: WsMessageKind, inner: &Arc<Mutex<WsConnectionInner<T>>>) -> Self {
        Self { kind, inner: Some(inner.clone()) }
    }
    pub fn kind(&self) -> WsMessageKind {
        self.kind
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncRead for WsMessageReader<T> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if let Some(inner) = &this.inner {
            let (n, fin) = match inner.lock().unwrap().poll_read(cx, buf) {
                Poll::Ready(r) => match r {
                    Ok(r) => r,
                    Err(err) => return Poll::Ready(Err(err)),
                }
                Poll::Pending => return Poll::Pending
            };
            if fin {
                this.inner.take();
            }
            return Poll::Ready(Ok(n))
        }
        Poll::Ready(Ok(0))
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Drop for WsMessageReader<T> {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.lock().unwrap().detach_reader();
        }
    }
}
