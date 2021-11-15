use crate::frame::{FrameHead, FrameHeadParseError};
use futures_lite::prelude::*;
use futures_lite::AsyncRead;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Copy, Clone, Debug)]
pub struct FrameHeadDecoder {}

impl FrameHeadDecoder {
    pub fn decode<T: AsyncRead + Unpin>(self, transport: T) -> FrameHeadDecode<T> {
        FrameHeadDecode {
            buffer: [0u8; 14],
            buffer_len: 0,
            transport: Some(transport),
            _decoder: self,
        }
    }
}

impl Default for FrameHeadDecoder {
    fn default() -> Self {
        Self {}
    }
}

pub struct FrameHeadDecode<T: AsyncRead + Unpin> {
    buffer: [u8; 14],
    buffer_len: usize,
    transport: Option<T>,
    _decoder: FrameHeadDecoder,
}

impl<T: AsyncRead + Unpin> Future for FrameHeadDecode<T> {
    type Output = Result<(T, FrameHead), FrameHeadDecodeError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut transport = self.transport.take().unwrap();
        loop {
            let min = match FrameHead::parse(&self.buffer[0..self.buffer_len]) {
                Ok(info) => {
                    return Poll::Ready(Ok((transport, info)));
                }
                Err(FrameHeadParseError::Incomplete(min)) => min,
                Err(err) => {
                    return Poll::Ready(Err(err.into()));
                }
            };
            let buffer_len = self.buffer_len;
            match Pin::new(&mut transport).poll_read(cx, &mut self.buffer[buffer_len..min]) {
                Poll::Ready(Ok(n)) => self.buffer_len += n,
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err.into())),
                Poll::Pending => {
                    self.transport = Some(transport);
                    return Poll::Pending;
                }
            }
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum FrameHeadDecodeError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    ParseErr(#[from] FrameHeadParseError),
}
