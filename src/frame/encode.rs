use crate::frame::{mask, FrameHead, Opcode};
use futures_lite::prelude::*;
use rand::prelude::*;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Copy, Clone, Debug)]
pub struct FrameEncoder<R: RngCore = StdRng> {
    pub mask_rng: Option<R>,
}

impl FrameEncoder<StdRng> {
    pub fn client() -> Self {
        Self {
            mask_rng: Some(StdRng::from_entropy()),
        }
    }
    pub fn server() -> Self {
        Self { mask_rng: None }
    }
}

impl FrameEncoder {
    pub fn encode<'a, T: AsyncWrite + Unpin>(
        &mut self,
        transport: T,
        opcode: Opcode,
        fin: bool,
        payload: &'a mut [u8],
    ) -> FrameEncode<'a, T> {
        let head = FrameHead {
            fin,
            opcode,
            mask: self
                .mask_rng
                .as_mut()
                .map_or([0u8, 0u8, 0u8, 0u8], |rng| rng.next_u32().to_ne_bytes()),
            payload_len: payload.len() as u64,
        };
        FrameEncode::new(transport, head, payload)
    }
}

#[derive(Copy, Clone, Debug)]
pub struct FrameEncode<'a, T: AsyncWrite + Unpin> {
    transport: Option<T>,
    head: [u8; 14],
    head_len: usize,
    head_written: usize,
    payload: &'a [u8],
    payload_written: usize,
}

impl<'a, T: AsyncWrite + Unpin> FrameEncode<'a, T> {
    pub fn new(transport: T, head: FrameHead, payload: &'a mut [u8]) -> Self {
        mask(head.mask, 0, payload);
        Self::with_masked_payload(transport, head, payload)
    }
    pub fn with_masked_payload(transport: T, head: FrameHead, masked_payload: &'a [u8]) -> Self {
        assert_eq!(masked_payload.len() as u64, head.payload_len);
        let mut head_buf = [0u8; 14];
        head.encode(&mut head_buf);
        FrameEncode {
            transport: Some(transport),
            head: head_buf,
            head_len: head.len_bytes(),
            head_written: 0,
            payload: masked_payload,
            payload_written: 0,
        }
    }
}

impl<T: AsyncWrite + Unpin> Future for FrameEncode<'_, T> {
    type Output = io::Result<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut transport = self.transport.take().unwrap();
        loop {
            let remaining = match self.head_written < self.head_len {
                true => &self.head[self.head_written..self.head_len],
                false => &self.payload[self.payload_written..],
            };
            match Pin::new(&mut transport).poll_write(cx, remaining) {
                Poll::Ready(Ok(n)) => {
                    match self.head_written < self.head_len {
                        true => self.head_written += n,
                        false => self.payload_written += n,
                    }
                    if self.head_written == self.head_len
                        && self.payload_written == self.payload.len()
                    {
                        return Poll::Ready(Ok(transport));
                    }
                }
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => {
                    self.transport = Some(transport);
                    return Poll::Pending;
                }
            }
        }
    }
}
