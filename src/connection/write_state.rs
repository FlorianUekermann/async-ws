use crate::frame::{WsControlFramePayload, WsDataFrameKind};

pub struct WsWriteConfig {
    pub(crate) mask: bool,
    _private: (),
}

pub struct WsConnectionWriteState {
    config: WsWriteConfig,
    control_buffer: [u8; 132],
    control_sent: usize,
    control_queued: usize,

    data_buffer: [u8; 1300],
    data_sending: bool,
    data_sent: usize,
    data_queued: usize,
    writing: Option<WsDataFrameKind>,

    queued_pong: Option<WsControlFramePayload>,
}

impl WsConnectionWriteState {
    fn with_config(config: WsWriteConfig) -> Self {
        WsConnectionWriteState {
            config,
            control_buffer: [0u8; 132],
            control_sent: 0,
            control_queued: 0,
            data_buffer: [0u8; 1300],
            data_sending: false,
            data_sent: 0,
            data_queued: 0,
            writing: None,
            queued_pong: None
        }
    }
}