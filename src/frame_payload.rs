use futures_lite::prelude::*;
use std::convert::TryFrom;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

#[pin_project::pin_project]
#[derive(Copy, Clone, Debug)]
pub struct FramePayloadReader<T: AsyncRead> {
    #[pin]
    transport: T,
    mask: [u8; 4],
    payload_len: u64,
    completion: u64,
}

impl<T: AsyncRead> FramePayloadReader<T> {
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

impl<T: AsyncRead> AsyncRead for FramePayloadReader<T> {
    fn poll_read(
        self: Pin<&mut Self>,
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
        let this = self.project();
        match this.transport.poll_read(cx, &mut buf[0..max]) {
            Poll::Ready(Ok(n)) => match n {
                0 => Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof))),
                n => {
                    let mut mask_off = *this.completion as usize;
                    for byte in buf[0..n].iter_mut() {
                        *byte ^= this.mask[mask_off & 3];
                        mask_off = mask_off.wrapping_add(1);
                    }
                    *this.completion += n as u64;
                    Poll::Ready(Ok(n))
                }
            },
            p => p,
        }
    }
}
