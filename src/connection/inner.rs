use crate::connection::encode::EncodeReady;

use crate::connection::config::WsConfig;
use crate::connection::open::{Open, OpenReady};
use crate::connection::WsConnectionError;
use crate::connection::WsConnectionInner::ClosedError;
use crate::frame::WsControlFramePayload;
use crate::message::WsMessageKind;
use futures::prelude::*;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

fn broken_pipe<T>() -> Poll<io::Result<T>> {
    Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()))
}

#[derive(Debug)]
pub(crate) enum InnerRxReady {
    MessageStart,
    MessageData,
    MessageEnd,
}

#[derive(Debug)]
pub(crate) enum InnerTxReady {
    FlushedFrames,
    FlushedMessages,
    Buffering,
}

pub(crate) enum WsConnectionInner<T: AsyncRead + AsyncWrite + Unpin> {
    Open(Open<T>),
    ClosedError(Arc<WsConnectionError>),
    ClosedOk(WsControlFramePayload),
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsConnectionInner<T> {
    pub(crate) fn with_config(transport: T, config: WsConfig) -> Self {
        Self::Open(Open::with_config(transport, config))
    }
    pub fn err(&self) -> Option<Arc<WsConnectionError>> {
        match self {
            ClosedError(err) => Some(err.clone()),
            _ => None,
        }
    }
    pub fn poll(
        &mut self,
        cx: &mut Context,
    ) -> Option<(&mut Open<T>, Poll<InnerRxReady>, Poll<InnerTxReady>)> {
        let open = match self {
            Self::Open(open) => open,
            _ => return None,
        };
        let (p_rx, p_tx) = open.poll(cx);
        let p_rx = match p_rx {
            Poll::Ready(OpenReady::Error) => {
                *self = ClosedError(open.take_rx_err().unwrap().into());
                return None;
            }
            Poll::Ready(OpenReady::Done) => return None,
            Poll::Pending => Poll::Pending,
            Poll::Ready(OpenReady::MessageStart) => Poll::Ready(InnerRxReady::MessageStart),
            Poll::Ready(OpenReady::MessageData) => Poll::Ready(InnerRxReady::MessageData),
            Poll::Ready(OpenReady::MessageEnd) => Poll::Ready(InnerRxReady::MessageEnd),
        };
        let p_tx = match p_tx {
            Poll::Ready(EncodeReady::Error) => {
                *self = ClosedError(open.take_tx_err().unwrap().into());
                return None;
            }
            Poll::Ready(EncodeReady::Done) => return None,
            Poll::Pending => Poll::Pending,
            Poll::Ready(EncodeReady::Buffering) => Poll::Ready(InnerTxReady::Buffering),
            Poll::Ready(EncodeReady::FlushedFrames) => Poll::Ready(InnerTxReady::FlushedFrames),
            Poll::Ready(EncodeReady::FlushedMessages) => Poll::Ready(InnerTxReady::FlushedMessages),
        };
        // Remove this when non-lexical lifetimes become stable.
        let open = match self {
            Self::Open(open) => open,
            _ => unreachable!(),
        };
        Some((open, p_rx, p_tx))
    }
    pub(crate) fn poll_next_writer(
        &mut self,
        kind: WsMessageKind,
        cx: &mut Context,
    ) -> Poll<Option<WsMessageKind>> {
        let (open, _p_rx, p_tx) = match self.poll(cx) {
            None => return Poll::Ready(None),
            Some(x) => x,
        };
        match p_tx {
            Poll::Ready(InnerTxReady::FlushedMessages) => {
                open.encode_state.start_message(kind);
                Poll::Ready(Some(kind))
            }
            _ => Poll::Pending,
        }
    }
    pub(crate) fn poll_write(
        &mut self,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut total = 0usize;
        while total != buf.len() {
            let (open, _p_rx, p_tx) = match self.poll(cx) {
                None => return broken_pipe(),
                Some(x) => x,
            };
            match p_tx {
                Poll::Ready(InnerTxReady::FlushedMessages) => return broken_pipe(),
                Poll::Pending => match total {
                    0 => return Poll::Pending,
                    n => return Poll::Ready(Ok(n)),
                },
                Poll::Ready(InnerTxReady::Buffering | InnerTxReady::FlushedFrames) => {
                    total += open
                        .encode_state
                        .append_data(&buf[total..], open.config.mask)
                }
            }
        }
        Poll::Ready(Ok(total))
    }
    pub(crate) fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            let (open, _p_rx, p_tx) = match self.poll(cx) {
                None => return broken_pipe(),
                Some(x) => x,
            };
            match p_tx {
                Poll::Ready(InnerTxReady::FlushedFrames | InnerTxReady::FlushedMessages) => {
                    return Poll::Ready(Ok(()));
                }
                Poll::Ready(InnerTxReady::Buffering) => open.encode_state.start_flushing(),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
    pub(crate) fn poll_close_writer(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            let (open, _p_rx, p_tx) = match self.poll(cx) {
                None => return broken_pipe(),
                Some(x) => x,
            };
            match p_tx {
                Poll::Ready(InnerTxReady::FlushedMessages) => {
                    return Pin::new(&mut open.transport).poll_flush(cx);
                }
                Poll::Pending => return Poll::Pending,
                Poll::Ready(InnerTxReady::Buffering | InnerTxReady::FlushedFrames) => {
                    open.encode_state.end_message(open.config.mask)
                }
            }
        }
    }
    pub(crate) fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let (open, p_rx, _p_tx) = match self.poll(cx) {
            None => return broken_pipe(),
            Some(x) => x,
        };
        let p = open.poll_read(cx, buf);
        if let Poll::Ready(Err(_)) = p {
            self.poll(cx);
        }
        p
    }
    pub(crate) fn poll_next_reader(&mut self, cx: &mut Context<'_>) -> Poll<Option<WsMessageKind>> {
        let (open, p_rx, _p_tx) = match self.poll(cx) {
            None => return Poll::Ready(None),
            Some(x) => x,
        };
        if !open.reader_is_attached {
            if let Poll::Ready(InnerRxReady::MessageStart) = p_rx {
                let kind = open.decode_state.take_message_start().unwrap();
                open.reader_is_attached = true;
                return Poll::Ready(Some(kind));
            }
        }
        Poll::Pending
    }
    pub(crate) fn detach_reader(&mut self) {
        if let WsConnectionInner::Open(open) = self {
            open.reader_is_attached = false;
        }
    }
    pub(crate) fn detach_writer(&mut self) {
        if let WsConnectionInner::Open(open) = self {
            open.encode_state.end_message(true);
        }
    }
}
