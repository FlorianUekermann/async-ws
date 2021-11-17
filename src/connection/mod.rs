mod read_state;
mod message_reader;

use futures::prelude::*;
use crate::frame::{WsControlFramePayload, FrameHeadDecodeState, FramePayloadReaderState, FrameDecodeError};
use std::task::{Context, Poll};
use std::pin::Pin;
use std::io;
use crate::connection::read_state::ReadState;

pub struct WsConfig {
    pub max_message_size: usize,
    _private: ()
}

const DEFAULT_MAX_MESSAGE_SIZE: usize = 1<<26;

impl WsConfig {
    fn client() -> Self {
        Self {
            max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
            _private: ()
        }
    }
    fn server() -> Self {
        Self {
            max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
            _private: ()
        }
    }
}

pub struct WsConnection<T: AsyncRead + AsyncWrite + Unpin> {
    config: WsConfig,
    transport: T,
    read_state: ReadState,
    closed: Option<WsControlFramePayload>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsConnection<T> {
    fn with_config(transport: T, config: WsConfig) -> Self {
        Self {
            config,
            transport,
            read_state: ReadState::Head(FrameHeadDecodeState::new()),
            closed: None,
        }
    }
    fn client(transport: T) -> Self {
        Self::with_config(transport,WsConfig::client())
    }
    fn server(transport: T) -> Self {
        Self::with_config(transport,WsConfig::server())
    }
    fn poll_read(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match &mut self.read_state {
            ReadState::Head(state) => {
                match state.poll(&mut self.transport, cx) {
                    Poll::Ready(Ok((_, frame_head))) => {
                        self.read_state = ReadState::Payload(FramePayloadReaderState::new(frame_head.mask, frame_head.payload_len));

                    }
                    Poll::Ready(Err(err)) => return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, err))),
                    Poll::Pending => return Poll::Pending,
                }
            }
            ReadState::Payload(state) => todo!(),
        };
        todo!()
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Stream for WsConnection<T> {
    type Item = ();

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        todo!()
    }
}

#[derive(thiserror::Error, Debug)]
pub enum WsConnectionError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    FrameHeadDecodeErr(#[from] FrameDecodeError),
}
