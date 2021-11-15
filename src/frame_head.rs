use crate::frame_payload::FramePayloadReader;
use futures_lite::prelude::*;
use futures_lite::AsyncRead;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Copy, Clone, Debug)]
pub enum Opcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}

#[derive(Copy, Clone, Debug)]
pub struct FrameHeadDecoder {}

impl FrameHeadDecoder {
    pub fn decode<T: AsyncRead + Unpin>(self, transport: T) -> FrameHeadDecode<T> {
        FrameHeadDecode {
            buffer: [0u8; 14],
            buffer_len: 0,
            transport: Some(transport),
            decoder: self,
        }
    }
}

impl Default for FrameHeadDecoder {
    fn default() -> Self {
        Self {}
    }
}

#[derive(Copy, Clone, Debug)]
pub struct FrameHead {
    pub fin: bool,
    pub opcode: Opcode,
    pub mask: [u8; 4],
    pub payload_len: u64,
}

impl FrameHead {
    pub fn payload_reader<T: AsyncRead + Unpin>(self, transport: T) -> FramePayloadReader<T> {
        FramePayloadReader::new(transport, self.mask, self.payload_len)
    }
}

pub struct FrameHeadDecode<T: AsyncRead + Unpin> {
    buffer: [u8; 14],
    buffer_len: usize,
    transport: Option<T>,
    #[allow(dead_code)]
    decoder: FrameHeadDecoder,
}

impl<T: AsyncRead + Unpin> Future for FrameHeadDecode<T> {
    type Output = anyhow::Result<(T, FrameHead)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut transport = this.transport.take().unwrap();
        loop {
            let min = match parse_frame_info(&this.buffer[0..this.buffer_len]) {
                Ok(info) => {
                    return Poll::Ready(Ok((transport, info)));
                }
                Err(FrameHeadParseError::Incomplete(min)) => min,
                Err(err) => {
                    return Poll::Ready(Err(err.into()));
                }
            };
            match Pin::new(&mut transport).poll_read(cx, &mut this.buffer[this.buffer_len..min]) {
                Poll::Ready(Ok(n)) => this.buffer_len += n,
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err.into())),
                Poll::Pending => {
                    this.transport = Some(transport);
                    return Poll::Pending;
                }
            }
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum FrameHeadParseError {
    #[error("incomplete, need at least {0} bytes")]
    Incomplete(usize),
    #[error("one or more RSV bit is set")]
    RsvBit,
    #[error("invalid opcode")]
    InvalidOpcode(u8),
}

pub fn parse_frame_info(buffer: &[u8]) -> Result<FrameHead, FrameHeadParseError> {
    if buffer.len() < 2 {
        return Err(FrameHeadParseError::Incomplete(2));
    }
    let (masked, extra_payload_len_bytes) = match buffer[1] {
        0..=125 => (false, 0usize),
        126 => (false, 2usize),
        127 => (false, 8usize),
        128..=253 => (true, 0usize),
        254 => (true, 2usize),
        255 => (true, 8usize),
    };
    let expected_buffer_len = 2 + extra_payload_len_bytes + (masked as usize) * 4;
    if buffer.len() < expected_buffer_len {
        return Err(FrameHeadParseError::Incomplete(expected_buffer_len));
    }
    let fin = match buffer[0] & 15 {
        0 => false,
        1 => true,
        _ => return Err(FrameHeadParseError::RsvBit),
    };
    let opcode = match buffer[0] >> 4 {
        0 => Opcode::Continuation,
        1 => Opcode::Text,
        2 => Opcode::Binary,
        8 => Opcode::Close,
        9 => Opcode::Ping,
        10 => Opcode::Pong,
        n => return Err(FrameHeadParseError::InvalidOpcode(n)),
    };
    let mut payload_len = [0u8; 8];
    match extra_payload_len_bytes {
        0 => payload_len[7] = buffer[1] & 127,
        2 => payload_len[6..8].copy_from_slice(&buffer[2..4]),
        8 => payload_len.copy_from_slice(&buffer[2..10]),
        _ => unreachable!(),
    };
    let payload_len = u64::from_be_bytes(payload_len);
    let mut mask = [0u8; 4];
    if masked {
        mask.copy_from_slice(&buffer[2 + extra_payload_len_bytes..6 + extra_payload_len_bytes])
    }
    Ok(FrameHead {
        fin,
        opcode,
        mask,
        payload_len,
    })
}
