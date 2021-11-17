use crate::frame::{FrameHeadDecodeState, FramePayloadReaderState};

#[derive(Debug)]
pub(crate) enum ReadState {
    Head(FrameHeadDecodeState),
    Payload(FramePayloadReaderState),
}
