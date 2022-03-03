mod decode;
mod encode;
mod reader;
mod send;
mod waker;
mod writer;

use crate::connection::decode::{DecodeState, DecodeReady};
use crate::connection::encode::{EncodeReady, EncodeState};
pub use crate::connection::reader::WsMessageReader;
pub use crate::connection::send::WsSend;
use crate::connection::waker::new_waker;
pub use crate::connection::writer::WsMessageWriter;
use crate::frame::{
    FrameDecodeError, WsControlFrame, WsControlFrameKind, WsControlFramePayload, WsDataFrameKind,
};
use crate::message::WsMessageKind;
use futures::prelude::*;
use futures::task::Waker;
use std::io;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use async_io::Timer;
use std::time::{Instant, Duration};

fn broken_pipe<T>() -> Poll<io::Result<T>> {
    Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()))
}

pub struct WsConfig {
    pub mask: bool,
    pub timeout: Duration,
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

struct Open<T: AsyncRead + AsyncWrite + Unpin> {
    config: WsConfig,
    transport: T,
    reader_is_attached: bool,
    timeout: Option<(Timer, bool)>,
    decode_state: DecodeState,
    encode_state: EncodeState,
}

impl<T: AsyncRead + AsyncWrite + Unpin> Open<T> {
    fn check_timeout<T>(&mut self, cx: &mut Context, e: T) -> Poll<T> {
        let ping_timer = match &mut self.timeout {
            None => self.timeout.insert((Timer::interval(self.config.timeout), false)),
            Some(ping_timer) => ping_timer,
        };
        if let Poll::Ready(_) = ping_timer.0.poll_next() {
            if ping_timer.1 {
                self.decode_state.set_err(WsConnectionError::Timeout);
                return Poll::Ready(e)
            }
            self.encode_state.queue_control(WsControlFrame::new(WsControlFrameKind::Ping, &[]));
            ping_timer.1 = true;
        }
        Poll::Pending
    }
    fn poll(&mut self, cx: &mut Context) -> (Poll<DecodeReady>, Poll<EncodeReady>) {
        loop {
            let pd = self.decode_state.poll(&mut self.transport, cx);
            let pd  = match pd {
                Poll::Pending => self.check_timeout(cx, DecodeReady::Error),
                Poll::Ready(DecodeReady::MessageData) => Poll::Ready(DecodeReady::MessageData),
                Poll::Ready(r) => {
                    self.timeout.take();
                    if let DecodeReady::Control(_kind) = r {
                        let mut control = self.decode_state.take_control().unwrap();
                        match control.kind() {
                            WsControlFrameKind::Ping => {
                                control.kind = WsControlFrameKind::Pong;
                                self.encode_state.queue_control(control);
                            }
                            WsControlFrameKind::Pong => {}
                            WsControlFrameKind::Close => self.encode_state.queue_control(control),
                        }
                        continue
                    }
                    Poll::Ready(r)
                },
            };
            let pe = self.encode_state.poll(&mut self.transport, cx, self.config.mask);
            return (pd, pe)
        }
    }
}

pub(crate) enum WsConnectionInner<T: AsyncRead + AsyncWrite + Unpin> {
    Open(Open<T>),
    ClosedError(WsConnectionError),
    ClosedErrorConsumed,
    ClosedSuccessfully(WsControlFramePayload),
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsConnectionInner<T> {
    fn with_config(transport: T, config: WsConfig) -> Self {
        Self::Open(Open {
            config,
            transport,
            reader_is_attached: false,
            timeout: None,
            decode_state: DecodeState::new(),
            encode_state: EncodeState::new(),
        })
    }


    pub(crate) fn poll_next_writer(
        &mut self,
        kind: WsMessageKind,
        cx: &mut Context,
    ) -> Poll<bool> {
        let open = match self {
            Self::Open(open) => open,
            _ => return Poll::Ready(false),
        };
        match open.encode_state.poll(&mut self.transport, cx, self.config.mask) {
            Poll::Ready(EncodeReady::FlushedMessages) => {
                self.encode_state.start_message(kind);
                Poll::Ready(true)
            }
            Poll::Ready(EncodeReady::Error | EncodeReady::Done) => Poll::Ready(false),
            Poll::Ready(EncodeReady::Buffering | EncodeReady::FlushedFrames) | Poll::Pending => {
                Poll::Pending
            }
        }
    }
    pub(crate) fn poll_write(
        &mut self,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let open = match self {
            Self::Open(open) => open,
            _ => return broken_pipe(),
        };
        let mut total = 0usize;
        while total != buf.len() {
            match open
                .encode_state
                .poll(&mut self.transport, cx, self.config.mask)
            {
                Poll::Ready(EncodeReady::Error | EncodeReady::Done) => return broken_pipe(),
                Poll::Pending => return Poll::Pending,
                Poll::Ready(EncodeReady::FlushedMessages) => return broken_pipe(),
                Poll::Ready(EncodeReady::Buffering | EncodeReady::FlushedFrames) => {}
            }
            total += open
                .encode_state
                .append_data(&buf[total..], self.config.mask)
        }
        Poll::Ready(Ok(total))
    }
    pub(crate) fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let open = match self {
            Self::Open(open) => open,
            _ => return broken_pipe(),
        };
        loop {
            match open
                .encode_state
                .poll(&mut open.transport, cx, open.config.mask)
            {
                Poll::Ready(EncodeReady::FlushedFrames | EncodeReady::FlushedMessages) => {
                    return Poll::Ready(Ok(()));
                }
                Poll::Ready(EncodeReady::Error | EncodeReady::Done) => return broken_pipe(),
                Poll::Pending => return Poll::Pending,
                Poll::Ready(EncodeReady::Buffering) => open.encode_state.start_flushing(),
            }
        }
    }
    pub(crate) fn poll_close_writer(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let open = match self {
            Self::Open(open) => open,
            _ => return broken_pipe(),
        };
        loop {
            match open
                .encode_state
                .poll(&mut open.transport, cx, open.config.mask)
            {
                Poll::Ready(EncodeReady::FlushedMessages) => {
                    return Pin::new(&mut open.transport).poll_flush(cx);
                }
                Poll::Ready(EncodeReady::Error | EncodeReady::Done) => return broken_pipe(),
                Poll::Pending => return Poll::Pending,
                Poll::Ready(EncodeReady::Buffering | EncodeReady::FlushedFrames) => {
                    self.encode_state.end_message(open.config.mask)
                }
            }
        }
    }
    pub(crate) fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let open = match self {
            Self::Open(open) => open,
            _ => return broken_pipe(),
        };
        loop {
            let _ = open
                .encode_state
                .poll(&mut open.transport, cx, open.config.mask);
            match self.decode_state.poll(&mut open.transport, cx, buf) {
                ControlFlow::Continue(frame) => open.handle_control_frame(frame),
                ControlFlow::Break(Poll::Ready(n)) => return Poll::Ready(Ok(n)),
                ControlFlow::Break(Poll::Pending) => {
                    if open.decode_state.take_message_end() {
                        open.reader_is_attached = false;
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
        let open = match self {
            Self::Open(open) => open,
            _ => return broken_pipe(),
        };

        loop {
            let _ = self
                .encode_state
                .poll(&mut self.transport, cx, self.config.mask);
            if self.reader_is_attached {
                return Poll::Pending;
            }
            match self
                .decode_state
                .poll(&mut self.transport, cx, &mut [0u8; 1300])
            {
                ControlFlow::Continue(frame) => self.handle_control_frame(frame),
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
    fn handle_control_frame(&mut self, mut frame: WsControlFrame) {
        match frame.kind() {
            WsControlFrameKind::Ping => frame.kind = WsControlFrameKind::Pong,
            WsControlFrameKind::Pong => return,
            WsControlFrameKind::Close => {}
        }
        self.encode_state.queue_control(frame)
    }
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
    #[error("timeout: {0:?}")]
    Timeout,
    #[error("unexpected: {0:?}")]
    UnexpectedFrameKind(WsDataFrameKind),
}
