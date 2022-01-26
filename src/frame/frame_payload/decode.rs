use crate::frame::payload_mask;
use futures::prelude::*;
use std::convert::TryFrom;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Debug)]
pub struct FramePayloadReader<T: AsyncRead + Unpin> {
    transport: T,
    state: FramePayloadReaderState,
}

impl<T: AsyncRead + Unpin> FramePayloadReader<T> {
    pub fn into_inner(self) -> T {
        self.transport
    }
    pub fn checkpoint(self) -> (T, FramePayloadReaderState) {
        (self.transport, self.state)
    }
}

#[derive(Debug)]
pub struct FramePayloadReaderState {
    mask: [u8; 4],
    payload_len: u64,
    completion: u64,
}

impl FramePayloadReaderState {
    pub fn new(mask: [u8; 4], payload_len: u64) -> Self {
        Self {
            mask,
            payload_len,
            completion: 0,
        }
    }
    pub fn restore<T: AsyncRead + Unpin>(self, transport: T) -> FramePayloadReader<T> {
        FramePayloadReader {
            transport,
            state: self,
        }
    }
    pub fn poll_read<T: AsyncRead + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        if self.payload_len <= self.completion || buf.len() == 0 {
            return Poll::Ready(Ok(0));
        }
        let min = match usize::try_from(self.payload_len - self.completion) {
            Ok(remainder) => remainder.min(buf.len()),
            Err(_) => buf.len(),
        };
        match Pin::new(transport).poll_read(cx, &mut buf[0..min]) {
            Poll::Ready(Ok(n)) => match n {
                0 => Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof))),
                n => {
                    payload_mask(self.mask, self.completion as usize, buf);
                    self.completion += n as u64;
                    Poll::Ready(Ok(n))
                }
            },
            p => p,
        }
    }
    pub fn finished(&self) -> bool {
        self.payload_len == self.completion
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for FramePayloadReader<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let Self { transport, state } = self.get_mut();
        state.poll_read(transport, cx, buf)
    }
}
