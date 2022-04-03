use crate::connection::decode::{DecodeReady, DecodeState};
use crate::connection::encode::{EncodeReady, EncodeState};
use crate::connection::{WsConfig, WsConnectionError};
use crate::frame::{WsControlFrame, WsControlFrameKind, WsControlFramePayload};
use async_io::Timer;
use futures::prelude::*;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Copy, Clone, Debug)]
pub(crate) enum OpenReady {
    MessageStart,
    MessageData,
    MessageEnd,
    Error,
    Done,
}

pub(crate) struct Open<T: AsyncRead + AsyncWrite + Unpin> {
    pub(crate) config: WsConfig,
    pub(crate) transport: T,
    pub(crate) reader_is_attached: bool,
    timeout: Option<(Timer, bool)>,
    pub decode_state: DecodeState,
    pub encode_state: EncodeState,
    received_close: Option<WsControlFramePayload>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> Open<T> {
    pub(crate) fn with_config(transport: T, config: WsConfig) -> Self {
        Self {
            config,
            transport,
            reader_is_attached: false,
            timeout: None,
            decode_state: DecodeState::new(),
            encode_state: EncodeState::new(),
            received_close: None,
        }
    }
    pub(crate) fn take_rx_err(&mut self) -> Option<WsConnectionError> {
        self.decode_state.take_err()
    }
    pub(crate) fn take_tx_err(&mut self) -> Option<WsConnectionError> {
        self.encode_state.take_err()
    }
    fn check_timeout<U>(&mut self, cx: &mut Context, e: U) -> Poll<U> {
        let ping_timer = match &mut self.timeout {
            None => self
                .timeout
                .insert((Timer::interval(self.config.timeout), false)),
            Some(ping_timer) => ping_timer,
        };
        if let Poll::Ready(_) = Pin::new(&mut ping_timer.0).poll_next(cx) {
            if ping_timer.1 {
                self.decode_state.set_err(WsConnectionError::Timeout);
                return Poll::Ready(e);
            }
            self.encode_state
                .queue_control(WsControlFrame::new(WsControlFrameKind::Ping, &[]));
            ping_timer.1 = true;
        }
        Poll::Pending
    }
    pub(crate) fn poll(&mut self, cx: &mut Context) -> (Poll<OpenReady>, Poll<EncodeReady>) {
        loop {
            let pd = self.decode_state.poll(&mut self.transport, cx);
            let pd = match pd {
                Poll::Pending => self.check_timeout(cx, OpenReady::Error),
                Poll::Ready(DecodeReady::MessageData) => {
                    if !self.reader_is_attached {
                        match self
                            .decode_state
                            .poll_read(&mut self.transport, cx, &mut [0u8; 1300])
                        {
                            Poll::Ready(_) => {
                                self.timeout.take();
                                continue;
                            }
                            Poll::Pending => self.check_timeout(cx, OpenReady::Error),
                        }
                    } else {
                        Poll::Ready(OpenReady::MessageData)
                    }
                }
                Poll::Ready(DecodeReady::MessageEnd) => {
                    self.timeout.take();
                    if !self.reader_is_attached {
                        self.decode_state.take_message_end();
                        self.reader_is_attached = false;
                        continue;
                    } else {
                        Poll::Ready(OpenReady::MessageEnd)
                    }
                }
                Poll::Ready(DecodeReady::MessageStart) => {
                    self.timeout.take();
                    Poll::Ready(OpenReady::MessageStart)
                }
                Poll::Ready(DecodeReady::Error) => Poll::Ready(OpenReady::Error),
                Poll::Ready(DecodeReady::Done) => Poll::Ready(OpenReady::Done),
                Poll::Ready(DecodeReady::Control(_)) => {
                    self.timeout.take();
                    let mut control = self.decode_state.take_control().unwrap();
                    match control.kind() {
                        WsControlFrameKind::Ping => {
                            control.kind = WsControlFrameKind::Pong;
                            self.encode_state.queue_control(control);
                        }
                        WsControlFrameKind::Pong => {}
                        WsControlFrameKind::Close => {
                            self.received_close = Some(control.payload);
                            self.encode_state.queue_control(control)
                        }
                    }
                    continue;
                }
            };
            let pe = self
                .encode_state
                .poll(&mut self.transport, cx, self.config.mask);
            return (pd, pe);
        }
    }
    pub(crate) fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            let (pd, _pe) = self.poll(cx);
            return match pd {
                Poll::Ready(OpenReady::MessageData) => {
                    match self.decode_state.poll_read(&mut self.transport, cx, buf) {
                        Poll::Ready(n) => match n {
                            0 => continue,
                            n => Poll::Ready(Ok(n)),
                        },
                        Poll::Pending => {
                            self.check_timeout(cx, Err(io::ErrorKind::BrokenPipe.into()))
                        }
                    }
                }
                Poll::Ready(OpenReady::MessageEnd) => {
                    self.decode_state.take_message_end();
                    self.reader_is_attached = false;
                    Poll::Ready(Ok(0))
                }
                Poll::Pending => Poll::Pending,
                Poll::Ready(OpenReady::Error) => {
                    let err = self.take_rx_err().unwrap();
                    self.decode_state.set_err(err);
                    return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
                }
                Poll::Ready(r) => unreachable!("{:?} is impossible during read", r),
            };
        }
    }
}
