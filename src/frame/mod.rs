mod decode;
mod frame_head;
mod frame_payload;

pub use decode::*;
pub use frame_head::*;
pub use frame_payload::*;

use crate::message::WsMessageKind;
use futures::prelude::*;
use std::error::Error;
use std::io::Cursor;
use std::io::Write;
use strum::Display;

#[derive(Copy, Clone, Debug)]
pub enum WsFrame {
    Control(WsControlFrame),
    Data(WsDataFrame),
}

impl WsFrame {
    pub fn decode<T: AsyncRead + Unpin>(transport: T) -> FrameDecoder<T> {
        FrameDecoderState::new().restore(transport)
    }
    // Writes the frame `buffer` and returns the number of bytes written. Panics if
    // `buffer` is too small or payload size in frame head does not match provided payload.
    pub fn encode(frame_head: FrameHead, frame_payload: &[u8], buffer: &mut [u8]) -> usize {
        assert_eq!(frame_head.payload_len, frame_payload.len() as u64);
        let total = frame_head.len_bytes() + frame_payload.len();
        frame_head.encode(buffer);
        let payload_buffer = &mut buffer[frame_head.len_bytes()..total];
        payload_buffer.copy_from_slice(frame_payload);
        payload_mask(frame_head.mask, 0, payload_buffer);
        return total;
    }
    pub fn encode_vec(frame_head: FrameHead, frame_payload: &[u8]) -> Vec<u8> {
        let mut buffer = vec![0u8; frame_head.len_bytes() + frame_payload.len()];
        WsFrame::encode(frame_head, frame_payload, &mut *buffer);
        buffer
    }
}

#[derive(Copy, Clone, Debug)]
pub enum WsFrameKind {
    Control(WsControlFrameKind),
    Data(WsDataFrameKind),
}

impl WsFrameKind {
    pub fn max_payload_len(self) -> u64 {
        match self {
            WsFrameKind::Control(_) => 125,
            WsFrameKind::Data(_) => 1073741824,
        }
    }
    pub fn opcode(self) -> WsOpcode {
        match self {
            WsFrameKind::Control(frame) => frame.opcode(),
            WsFrameKind::Data(frame) => frame.opcode(),
        }
    }
    pub fn is_control(self) -> bool {
        match self {
            WsFrameKind::Control(_) => true,
            WsFrameKind::Data(_) => false,
        }
    }
}

#[derive(Display, Copy, Clone, Debug, Eq, PartialEq)]
pub enum WsDataFrameKind {
    Text,
    Binary,
    Continuation,
}

impl WsDataFrameKind {
    pub fn opcode(self) -> WsOpcode {
        match self {
            WsDataFrameKind::Text => WsOpcode::Text,
            WsDataFrameKind::Binary => WsOpcode::Binary,
            WsDataFrameKind::Continuation => WsOpcode::Continuation,
        }
    }
    pub fn message_kind(self) -> Option<WsMessageKind> {
        match self {
            WsDataFrameKind::Text => Some(WsMessageKind::Text),
            WsDataFrameKind::Binary => Some(WsMessageKind::Binary),
            WsDataFrameKind::Continuation => None,
        }
    }
    pub fn frame_kind(self) -> WsFrameKind {
        WsFrameKind::Data(self)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WsControlFrameKind {
    Ping,
    Pong,
    Close,
}

impl WsControlFrameKind {
    pub fn opcode(self) -> WsOpcode {
        match self {
            WsControlFrameKind::Ping => WsOpcode::Ping,
            WsControlFrameKind::Pong => WsOpcode::Pong,
            WsControlFrameKind::Close => WsOpcode::Close,
        }
    }
    pub fn frame_kind(self) -> WsFrameKind {
        WsFrameKind::Control(self)
    }
}

#[derive(Copy, Clone, Debug)]
pub struct WsDataFrame {
    pub(crate) kind: WsDataFrameKind,
    pub(crate) fin: bool,
    pub(crate) mask: [u8; 4],
    pub(crate) payload_len: u64,
}

impl WsDataFrame {
    pub fn payload_reader(&self) -> FramePayloadReaderState {
        FramePayloadReaderState::new(self.mask, self.payload_len)
    }
    pub fn kind(&self) -> WsDataFrameKind {
        self.kind
    }
    pub fn fin(&self) -> bool {
        self.fin
    }
    pub fn mask(&self) -> [u8; 4] {
        self.mask
    }
    pub fn payload_len(&self) -> u64 {
        self.payload_len
    }
}

#[derive(Copy, Clone, Debug)]
pub struct WsControlFrame {
    pub(crate) kind: WsControlFrameKind,
    pub(crate) payload: WsControlFramePayload,
}

impl WsControlFrame {
    pub fn new(kind: WsControlFrameKind, payload: &[u8]) -> Self {
        let payload = WsControlFramePayload::new(payload);
        Self { kind, payload }
    }
    pub fn payload(&self) -> &[u8] {
        &self.payload.data()
    }
    pub fn kind(&self) -> WsControlFrameKind {
        self.kind
    }
    pub fn head(&self, mask: [u8; 4]) -> FrameHead {
        FrameHead {
            fin: true,
            opcode: self.kind.opcode(),
            mask,
            payload_len: self.payload().len() as u64,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct WsControlFramePayload {
    pub(crate) len: u8,
    pub(crate) buffer: [u8; 125],
}

impl WsControlFramePayload {
    pub(crate) fn new(data: &[u8]) -> Self {
        let len = data.len().min(125);
        let payload = &data[0..len];
        let mut buffer = [0u8; 125];
        buffer[0..len].copy_from_slice(payload);
        Self {
            len: len as u8,
            buffer,
        }
    }
    pub(crate) fn data(&self) -> &[u8] {
        &self.buffer[0..self.len()]
    }
    pub(crate) fn len(&self) -> usize {
        self.len as usize
    }
    pub(crate) fn close_body(&self) -> Result<Option<(u16, &str)>, CloseBodyError> {
        match self.len() {
            0 => Ok(None),
            1 => Err(CloseBodyError::BodyTooShort),
            _ => {
                let data = self.data();
                let code = u16::from_be_bytes([data[0], data[1]]);
                match code {
                    0..=999 | 1004..=1006 | 1016..=2999 => Err(CloseBodyError::InvalidCode),
                    code => match std::str::from_utf8(&data[2..]) {
                        Ok(reason) => Ok(Some((code, reason))),
                        Err(_) => Err(CloseBodyError::InvalidUtf8),
                    },
                }
            }
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum CloseBodyError {
    #[error("close frame body is too short")]
    BodyTooShort,
    #[error("invalid utf8 in close body reason")]
    InvalidUtf8,
    #[error("invalid close frame body code")]
    InvalidCode,
}

impl<E: Error> From<(u16, &E)> for WsControlFramePayload {
    fn from(err: (u16, &E)) -> Self {
        let mut buffer = [0u8; 125];
        buffer[0..2].copy_from_slice(&err.0.to_be_bytes());
        let mut cursor = Cursor::new(&mut buffer[2..]);
        write!(cursor, "{}", err.1).ok();
        let len = 2 + cursor.position() as u8;
        WsControlFramePayload { len, buffer }
    }
}
