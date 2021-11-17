// use crate::frame::{FrameHeadDecodeState, FramePayloadReaderState, WsControlFramePayload, FrameHeadParseError, WsFrame};
// use futures::prelude::*;
// use std::task::{Context, Poll};
// use std::pin::Pin;
//
// #[derive(Debug)]
// pub struct FrameDecoder<T: AsyncRead + Unpin> {
//     transport: T,
//     state: FrameDecoderState,
// }
//
//
// #[derive(Debug)]
// pub(crate) enum FrameDecoderState {
//     Head(FrameHeadDecodeState),
//     ControlPayload {
//         payload: WsControlFramePayload,
//         reader: FramePayloadReaderState,
//     },
// }
//
// impl<T: AsyncRead + Unpin> Future for FrameDecoder<T> {
//     type Output = Result<WsFrame<T>, FrameDecodeError>;
//
//     fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
//         match &mut self.state {
//             FrameDecoderState::Head(state) => {
//                 state.poll(self.transport, cx)
//             }
//             FrameDecoderState::ControlPayload { payload, reader } => {
//                 reader.poll_read(self.transport)
//             }
//         }
//     }
// }

use crate::frame::FrameHeadParseError;

#[derive(thiserror::Error, Debug)]
pub enum FrameDecodeError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    ParseErr(#[from] FrameHeadParseError),
}
