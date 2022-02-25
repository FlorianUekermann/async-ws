mod decode;
mod encode;
mod reader;
mod send;
mod waker;
mod writer;

use crate::connection::decode::DecodeState;
use crate::connection::encode::{EncodeState, EncodeStateReady};
pub use crate::connection::reader::WsMessageReader;
pub use crate::connection::send::WsSend;
use crate::connection::waker::new_waker;
pub use crate::connection::writer::WsMessageWriter;
use crate::frame::{FrameDecodeError, WsControlFrame, WsDataFrameKind};
use crate::message::WsMessageKind;
use futures::prelude::*;
use futures::task::Waker;
use std::io;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
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

pub(crate) struct WsConnectionInner<T: AsyncRead + AsyncWrite + Unpin> {
    config: WsConfig,
    transport: T,
    pub(crate) reader_is_attached: bool,
    decode_state: DecodeState,
    encode_state: EncodeState,
    stream_waker: Option<Waker>,
    send_waker: Option<Waker>,
    writer_waker: Option<Waker>,
    reader_waker: Option<Waker>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsConnectionInner<T> {
    fn with_config(transport: T, config: WsConfig) -> Self {
        Self {
            config,
            transport,
            reader_is_attached: false,
            decode_state: DecodeState::new(),
            encode_state: EncodeState::new(),
            stream_waker: None,
            send_waker: None,
            writer_waker: None,
            reader_waker: None,
        }
    }
    pub(crate) fn poll_next_writer(
        &mut self,
        kind: WsMessageKind,
        cx: &mut Context,
    ) -> Poll<Result<(), WsConnectionError>> {
        match self
            .encode_state
            .poll(&mut self.transport, cx, self.config.mask)
        {
            Poll::Ready(Ok(EncodeStateReady::FlushedMessages)) => {
                self.encode_state.start_message(kind);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(err)) => Poll::Ready(Err(err.into())),
            Poll::Ready(Ok(EncodeStateReady::Buffering | EncodeStateReady::FlushedFrames))
            | Poll::Pending => Poll::Pending,
        }
    }
    pub(crate) fn poll_write(
        &mut self,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut total = 0usize;
        while total != buf.len() {
            match self
                .encode_state
                .poll(&mut self.transport, cx, self.config.mask)
            {
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Ok(EncodeStateReady::FlushedMessages)) => {
                    return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()))
                }
                Poll::Ready(Ok(EncodeStateReady::Buffering | EncodeStateReady::FlushedFrames)) => {}
            }
            total += self
                .encode_state
                .append_data(&buf[total..], self.config.mask)
        }
        Poll::Ready(Ok(total))
    }
    pub(crate) fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            match self
                .encode_state
                .poll(&mut self.transport, cx, self.config.mask)
            {
                Poll::Ready(Ok(
                    EncodeStateReady::FlushedFrames | EncodeStateReady::FlushedMessages,
                )) => return Poll::Ready(Ok(())),
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Ok(EncodeStateReady::Buffering)) => self.encode_state.start_flushing(),
            }
        }
    }
    pub(crate) fn poll_close_writer(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            match dbg!(self
                .encode_state
                .poll(&mut self.transport, cx, self.config.mask))
            {
                Poll::Ready(Ok(EncodeStateReady::FlushedMessages)) => {
                    return Pin::new(&mut self.transport).poll_flush(cx)
                }
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Ok(EncodeStateReady::Buffering | EncodeStateReady::FlushedFrames)) => {
                    self.encode_state.end_message(self.config.mask)
                }
            }
        }
    }
    pub(crate) fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            if let Poll::Ready(Err(err)) =
                self.encode_state
                    .poll(&mut self.transport, cx, self.config.mask)
            {
                panic!("err: {:?}", err);
            }
            match self.decode_state.poll(&mut self.transport, cx, buf) {
                ControlFlow::Continue(frame) => Self::handle_control_frame(frame),
                ControlFlow::Break(Poll::Ready(n)) => return Poll::Ready(Ok(n)),
                ControlFlow::Break(Poll::Pending) => {
                    if self.decode_state.take_message_end() {
                        self.reader_is_attached = false;
                        return Poll::Ready(Ok(0));
                    }
                    return Poll::Pending;
                }
            }
        }
    }
    fn poll_next_reader(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<WsMessageKind, WsConnectionError>>> {
        dbg!();
        loop {
            if let Poll::Ready(Err(err)) =
                self.encode_state
                    .poll(&mut self.transport, cx, self.config.mask)
            {
                panic!("err: {:?}", err);
            }
            if self.reader_is_attached {
                return Poll::Pending;
            }
            match self
                .decode_state
                .poll(&mut self.transport, cx, &mut [0u8; 1300])
            {
                ControlFlow::Continue(frame) => Self::handle_control_frame(frame),
                ControlFlow::Break(Poll::Ready(_)) => {}
                ControlFlow::Break(Poll::Pending) => {
                    if let Some(kind) = self.decode_state.take_message_start() {
                        self.reader_is_attached = true;
                        return Poll::Ready(Some(Ok(kind)));
                    }
                    if !self.decode_state.take_message_end() {
                        return Poll::Pending;
                    }
                }
            }
        }
    }
    pub(crate) fn detach_reader(&mut self) {
        self.reader_is_attached = false;
        self.reader_waker.take();
        self.stream_waker.take().map(Waker::wake);
    }
    pub(crate) fn detach_writer(&mut self) {
        self.encode_state.end_message(true);
        self.writer_waker.take();
        self.send_waker.take().map(Waker::wake);
    }
    fn handle_control_frame(frame: WsControlFrame) {}
}

#[derive(Clone)]
pub struct WsConnection<T: AsyncRead + AsyncWrite + Unpin> {
    inner: Arc<Mutex<WsConnectionInner<T>>>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsConnection<T> {
    pub fn with_config(transport: T, config: WsConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(WsConnectionInner::with_config(
                transport, config,
            ))),
        }
    }
    pub fn send(&self, kind: WsMessageKind) -> WsSend<T> {
        WsSend::new(&self.inner, kind)
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Stream for WsConnection<T> {
    type Item = Result<WsMessageReader<T>, WsConnectionError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        dbg!();
        let waker = new_waker(Arc::downgrade(&self.inner));
        let mut inner = self.inner.lock().unwrap();
        inner.stream_waker = Some(cx.waker().clone());
        inner
            .poll_next_reader(&mut Context::from_waker(&waker))
            .map(|o| o.map(|r| r.map(|kind| WsMessageReader::new(kind, &self.inner))))
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
