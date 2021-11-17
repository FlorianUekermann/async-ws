mod decode;
mod encode;
mod frame_head;
mod frame_payload;

pub use encode::*;
pub use frame_head::*;
pub use frame_payload::*;

use futures::prelude::*;

pub enum WsFrame<T: AsyncRead + Unpin> {
    Control(WsControlFrame),
    Data {
        kind: WsDataFrameKind,
        payload: FramePayloadReader<T>,
    },
}

#[derive(Copy, Clone, Debug)]
pub enum WsDataFrameKind {
    Text,
    Binary,
}

#[derive(Copy, Clone, Debug)]
pub enum WsControlFrameKind {
    Ping,
    Pong,
    Close,
}

#[derive(Copy, Clone, Debug)]
pub struct WsControlFrame {
    kind: WsControlFrameKind,
    payload: WsControlFramePayload,
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
    pub fn opcode(&self) -> WsOpcode {
        match self.kind {
            WsControlFrameKind::Ping => WsOpcode::Ping,
            WsControlFrameKind::Pong => WsOpcode::Pong,
            WsControlFrameKind::Close => WsOpcode::Close,
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
}
