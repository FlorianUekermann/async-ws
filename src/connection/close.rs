use crate::connection::WsConnectionError;
use crate::frame::WsControlFramePayload;
use std::{error, io};

pub(crate) enum CloseState {
    None,
    Queued(WsControlFramePayload),
    Sent(WsControlFramePayload),
    ReceivedQueued(WsControlFramePayload),
    ReceivedSent(WsControlFramePayload),
}

impl CloseState {
    pub(crate) fn receive(&mut self, payload: WsControlFramePayload) {
        match self {
            Self::None => *self = Self::ReceivedQueued(payload),
            Self::Queued(payload) => *self = Self::ReceivedQueued(*payload),
            Self::Sent(payload) => *self = Self::ReceivedSent(*payload),
            _ => panic!("duplicate incoming close frame"),
        }
    }
    pub(crate) fn queue(&mut self, payload: WsControlFramePayload) {
        match self {
            Self::None => *self = Self::Queued(payload),
            _ => panic!("duplicate outgoing close frame"),
        }
    }
    pub(crate) fn queued(&self) -> bool {
        match self {
            Self::Queued(_) | Self::ReceivedQueued(_) => true,
            _ => false,
        }
    }
    pub(crate) fn unqueue(&mut self) -> Option<WsControlFramePayload> {
        match self {
            Self::Queued(payload) => {
                let payload = *payload;
                *self = Self::Sent(payload);
                Some(payload)
            }
            Self::ReceivedQueued(payload) => {
                let payload = *payload;
                *self = Self::ReceivedSent(payload);
                Some(payload)
            }
            _ => None,
        }
    }
    pub(crate) fn receive_err<E: error::Error>(&mut self, err: E) -> E {
        match self {
            Self::None => *self = Self::ReceivedQueued((1002, &err).into()),
            CloseState::Queued(payload) => *self = Self::ReceivedQueued(*payload),
            CloseState::Sent(payload) => *self = Self::ReceivedSent(*payload),
            CloseState::ReceivedQueued(_) => {}
            CloseState::ReceivedSent(_) => {}
        }
        return err;
    }
    pub(crate) fn write_err(&mut self, err: io::Error) -> Option<WsConnectionError> {
        match self {
            Self::None => *self = Self::ReceivedSent((1002, &err).into()),
            CloseState::Queued(payload) => *self = Self::ReceivedSent(*payload),
            CloseState::Sent(payload) => *self = Self::ReceivedSent(*payload),
            CloseState::ReceivedQueued(payload) => *self = Self::ReceivedSent(*payload),
            CloseState::ReceivedSent(_) => {}
        }
        return Some(WsConnectionError::Io(err));
    }
    pub(crate) fn open_for_sending(&self) -> bool {
        match self {
            Self::None | Self::ReceivedQueued(_) | Self::Queued(_) => true,
            Self::Sent(_) | Self::ReceivedSent(_) => false,
        }
    }
    pub(crate) fn open_for_receiving(&self) -> bool {
        match self {
            CloseState::None | CloseState::Queued(_) | CloseState::Sent(_) => true,
            CloseState::ReceivedQueued(_) | CloseState::ReceivedSent(_) => false,
        }
    }
}
