use crate::connection::waker::{new_waker, Wakers};
use crate::connection::writer::WsMessageWriter;
use crate::connection::{ WsConnectionInner};
use crate::message::WsMessageKind;
use futures::{AsyncRead, AsyncWrite, Future};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::ops::DerefMut;

#[derive(Clone)]
pub struct WsSend<T: AsyncRead + AsyncWrite + Unpin> {
    kind: WsMessageKind,
    parent: Arc<Mutex<(WsConnectionInner<T>,Wakers)>>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsSend<T> {
    pub(crate) fn new(parent: &Arc<Mutex<(WsConnectionInner<T>, Wakers)>>, kind: WsMessageKind) -> Self {
        Self {
            kind,
            parent: parent.clone(),
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Future for WsSend<T> {
    type Output = Option<WsMessageWriter<T>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut guard = self.parent.lock().unwrap();
        let (inner, wakers) = guard.deref_mut();
        wakers.send_waker = Some(cx.waker().clone());
        let waker = new_waker(Arc::downgrade(&self.parent));
        match inner.poll_next_writer(self.kind, &mut Context::from_waker(&waker)) {
            Poll::Ready(true) => Poll::Ready(Some(WsMessageWriter::new(self.kind, &self.parent))),
            Poll::Ready(false) => Poll::Ready(None),
            Poll::Pending => Poll::Pending
        }
    }
}
