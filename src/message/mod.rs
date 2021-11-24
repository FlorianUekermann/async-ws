use crate::frame::WsDataFrameKind;

#[derive(Copy, Clone, Debug)]
pub enum WsMessageKind {
    Binary,
    Text,
}

impl WsMessageKind {
    pub fn frame_kind(&self) -> WsDataFrameKind {
        match self {
            WsMessageKind::Binary => WsDataFrameKind::Binary,
            WsMessageKind::Text => WsDataFrameKind::Text,
        }
    }
}
