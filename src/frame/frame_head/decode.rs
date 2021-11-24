use crate::frame::decode::FrameDecodeError;
use crate::frame::{FrameHead, FrameHeadParseError};
use futures::prelude::*;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Debug)]
pub struct FrameHeadDecode<T: AsyncRead + Unpin> {
    transport: Option<T>,
    state: FrameHeadDecodeState,
}

impl<T: AsyncRead + Unpin> FrameHeadDecode<T> {
    pub fn checkpoint(self) -> Option<(T, FrameHeadDecodeState)> {
        let (transport, state) = (self.transport, self.state);
        transport.map(|transport| (transport, state))
    }
}

#[derive(Debug)]
pub struct FrameHeadDecodeState {
    buffer: [u8; 14],
    buffer_len: usize,
}

impl FrameHeadDecodeState {
    pub fn new() -> Self {
        Self {
            buffer: [0u8; 14],
            buffer_len: 0,
        }
    }
    pub fn restore<T: AsyncRead + Unpin>(self, transport: T) -> FrameHeadDecode<T> {
        FrameHeadDecode {
            transport: Some(transport),
            state: self,
        }
    }
    pub fn poll<T: AsyncRead + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
    ) -> Poll<Result<FrameHead, FrameDecodeError>> {
        loop {
            let min = match FrameHead::parse(&self.buffer[0..self.buffer_len]) {
                Ok(info) => return Poll::Ready(Ok(info)),
                Err(FrameHeadParseError::Incomplete(min)) => min,
                Err(err) => return Poll::Ready(Err(err.into())),
            };
            let buffer_len = self.buffer_len;
            let read_window = &mut self.buffer[buffer_len..min];
            match Pin::new(&mut *transport).poll_read(cx, read_window) {
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(Err(FrameDecodeError::Io(
                        io::ErrorKind::UnexpectedEof.into(),
                    )))
                }
                Poll::Ready(Ok(n)) => self.buffer_len += n,
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err.into())),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<T: AsyncRead + Unpin> Future for FrameHeadDecode<T> {
    type Output = Result<(T, FrameHead), FrameDecodeError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut transport = self.transport.take().unwrap();
        match self.state.poll(&mut transport, cx) {
            Poll::Ready(Ok(frame_head)) => Poll::Ready(Ok((transport, frame_head))),
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Pending => {
                self.transport = Some(transport);
                Poll::Pending
            }
        }
    }
}
