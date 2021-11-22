mod decode;
mod encode;
mod frame_head;
mod frame_payload;

pub use decode::*;
pub use encode::*;
pub use frame_head::*;
pub use frame_payload::*;

use futures::prelude::*;

#[derive(Copy, Clone, Debug)]
pub enum WsFrame {
    Control(WsControlFrame),
    Data(WsDataFrame),
}

impl WsFrame {
    pub fn decode<T: AsyncRead + Unpin>(transport: T) -> FrameDecoder<T> {
        FrameDecoderState::new().restore(transport)
    }
}

#[derive(Copy, Clone, Debug)]
pub enum WsFrameKind {
    Control(WsControlFrameKind),
    Data(WsDataFrameKind),
}

#[derive(Copy, Clone, Debug)]
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
}

#[derive(Copy, Clone, Debug)]
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
}

#[derive(Copy, Clone, Debug)]
pub struct WsDataFrame {
    pub(crate) kind: WsDataFrameKind,
    pub(crate) fin: bool,
    pub(crate) mask: [u8; 4],
    pub(crate) payload_len: u64,
}

impl WsDataFrame {
    pub fn payload_reader<T: AsyncRead + Unpin>(&self, transport: T) -> FramePayloadReader<T> {
        FramePayloadReaderState::new(self.mask, self.payload_len).restore(transport)
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
}
