use crate::frame::mask;
use futures_lite::prelude::*;
use std::convert::TryFrom;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Copy, Clone, Debug)]
pub struct FramePayloadReader<T: AsyncRead + Unpin> {
    transport: T,
    mask: [u8; 4],
    payload_len: u64,
    completion: u64,
}

impl<T: AsyncRead + Unpin> FramePayloadReader<T> {
    pub fn new(transport: T, mask: [u8; 4], payload_len: u64) -> Self {
        Self {
            transport,
            mask,
            payload_len,
            completion: 0,
        }
    }
    pub fn into_inner(self) -> T {
        self.transport
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for FramePayloadReader<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        if self.payload_len <= self.completion || buf.len() == 0 {
            return Poll::Ready(Ok(0));
        }
        let max = match usize::try_from(self.payload_len - self.completion) {
            Ok(remainder) => remainder.max(buf.len()),
            Err(_) => buf.len(),
        };
        match Pin::new(&mut self.transport).poll_read(cx, &mut buf[0..max]) {
            Poll::Ready(Ok(n)) => match n {
                0 => Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof))),
                n => {
                    mask(self.mask, self.completion as usize, buf);
                    self.completion += n as u64;
                    Poll::Ready(Ok(n))
                }
            },
            p => p,
        }
    }
}
