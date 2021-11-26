mod decode;

pub use decode::*;

use crate::frame::{
    FramePayloadReader, FramePayloadReaderState, WsControlFrameKind, WsDataFrameKind, WsFrameKind,
};
use futures::prelude::*;

#[derive(Copy, Clone, Debug)]
pub enum WsOpcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}

impl WsOpcode {
    pub fn frame_kind(self) -> WsFrameKind {
        match self {
            WsOpcode::Continuation => WsFrameKind::Data(WsDataFrameKind::Continuation),
            WsOpcode::Text => WsFrameKind::Data(WsDataFrameKind::Text),
            WsOpcode::Binary => WsFrameKind::Data(WsDataFrameKind::Binary),
            WsOpcode::Close => WsFrameKind::Control(WsControlFrameKind::Close),
            WsOpcode::Ping => WsFrameKind::Control(WsControlFrameKind::Ping),
            WsOpcode::Pong => WsFrameKind::Control(WsControlFrameKind::Pong),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct FrameHead {
    pub fin: bool,
    pub opcode: WsOpcode,
    pub mask: [u8; 4],
    pub payload_len: u64,
}

impl FrameHead {
    pub fn decode<T: AsyncRead + Unpin>(transport: T) -> FrameHeadDecode<T> {
        FrameHeadDecodeState::new().restore(transport)
    }
    pub fn payload_reader<T: AsyncRead + Unpin>(self, transport: T) -> FramePayloadReader<T> {
        FramePayloadReaderState::new(self.mask, self.payload_len).restore(transport)
    }
    pub fn parse(buffer: &[u8]) -> Result<FrameHead, FrameHeadParseError> {
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
        let fin = match buffer[0] & 0xF0 {
            0x00 => false,
            0x80 => true,
            _ => return Err(FrameHeadParseError::RsvBit),
        };
        let opcode = match buffer[0] & 0x0F {
            0x0 => WsOpcode::Continuation,
            0x1 => WsOpcode::Text,
            0x2 => WsOpcode::Binary,
            0x8 => WsOpcode::Close,
            0x9 => WsOpcode::Ping,
            0xA => WsOpcode::Pong,
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
        if let WsFrameKind::Control(_) = opcode.frame_kind() {
            if !fin {
                return Err(FrameHeadParseError::FragmentedControl);
            }
        }
        if payload_len > opcode.frame_kind().max_payload_len() {
            return Err(FrameHeadParseError::PayloadLengthTooLong);
        }
        Ok(FrameHead {
            fin,
            opcode,
            mask,
            payload_len,
        })
    }
    // Length of the encoded frame head in bytes ([2..14]).
    pub fn len_bytes(&self) -> usize {
        let extra_payload_len_bytes = match self.payload_len {
            0..=125 => 0usize,
            126..=65536 => 2usize,
            _ => 8usize,
        };
        2 + extra_payload_len_bytes + self.masked() as usize * 4
    }
    pub fn masked(&self) -> bool {
        self.mask != [0u8, 0u8, 0u8, 0u8]
    }
    // Writes the frame head to `buffer`.
    // Panics if `buffer` is too small (see [len_bytes()][`Self::len_bytes()`]).
    pub fn encode(&self, buffer: &mut [u8]) {
        buffer[0] = self.fin as u8 * 0x80;
        buffer[0] += match self.opcode {
            WsOpcode::Continuation => 0x0,
            WsOpcode::Text => 0x1,
            WsOpcode::Binary => 0x2,
            WsOpcode::Close => 0x8,
            WsOpcode::Ping => 0x9,
            WsOpcode::Pong => 0xA,
        };
        buffer[1] = match self.payload_len {
            0..=125 => self.payload_len as u8,
            126..=65536 => 126u8,
            _ => 127u8,
        };
        match buffer[1] {
            126 => buffer[2..4].copy_from_slice(&(self.payload_len as u16).to_be_bytes()),
            127 => buffer[2..10].copy_from_slice(&self.payload_len.to_be_bytes()),
            _ => {}
        }
        if self.masked() {
            let mask_buffer = match buffer[1] {
                126 => &mut buffer[4..8],
                127 => &mut buffer[10..14],
                _ => &mut buffer[2..6],
            };
            mask_buffer.copy_from_slice(&self.mask);
            buffer[1] += 128;
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
    #[error("payload length too long")]
    PayloadLengthTooLong,
    #[error("fragmented control message")]
    FragmentedControl,
}
