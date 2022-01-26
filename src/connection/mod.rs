mod decode;

use crate::frame::{
    FrameDecodeError,
    WsDataFrameKind
};
use crate::message::WsMessageKind;
use futures::prelude::*;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use crate::connection::decode::{ DecodeState, DecodeEvent};

pub struct WsConfig {
    pub mask: bool,
    _private: (),
}

impl WsConfig {
    pub fn client() -> Self {
        Self {
            mask: true,
            _private: (),
        }
    }
    pub fn server() -> Self {
        Self {
            mask: false,
            _private: (),
        }
    }
}

pub struct WsConnectionState {
    _config: WsConfig,
    decode_state: DecodeState,
    tmp: bool,
    // write_state: WsConnectionWriteState,
}

impl WsConnectionState {
    fn with_config(config: WsConfig) -> Self {
        Self {
            tmp: false,
            _config: config,
            decode_state: DecodeState::new(),
        }
    }
    // pub fn start_message(&mut self, kind: WsMessageKind) {
    //     if self.writing.is_some() {
    //         self.transition_to_sending(true)
    //     }
    //     self.writing = Some(kind.frame_kind())
    // }
    // fn transition_to_sending(&mut self, fin: bool) {
    //     if !fin && self.data_queued == 8 {
    //         return;
    //     }
    //     if let Some(frame_kind) = self.writing {
    //         self.data_sending = true;
    //         let head = FrameHead {
    //             fin,
    //             opcode: frame_kind.opcode(),
    //             mask: self.gen_mask(),
    //             payload_len: (self.data_queued - 8) as u64,
    //         };
    //         self.data_sent = 8 - head.len_bytes();
    //         head.encode(&mut self.data_buffer[self.data_sent..]);
    //         self.writing = match fin {
    //             true => None,
    //             false => Some(WsDataFrameKind::Continuation),
    //         };
    //     }
    // }
    // fn poll_write_transport<T: AsyncWrite + Unpin>(
    //     &mut self,
    //     transport: &mut T,
    //     cx: &mut Context<'_>,
    //     force_flush: bool,
    // ) -> Poll<io::Result<()>> {
    //     if !self.close_state.open_for_sending() {
    //         return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
    //     }
    //     while self.data_sending {
    //         match Pin::new(&mut *transport)
    //             .poll_write(cx, &self.data_buffer[self.data_sent..self.data_queued])
    //         {
    //             Poll::Ready(Ok(n)) => {
    //                 self.data_sent += n;
    //                 if self.data_sent == self.data_queued {
    //                     self.data_sent = 0;
    //                     self.data_queued = 8;
    //                     self.data_sending = false;
    //                 }
    //             }
    //             Poll::Ready(Err(err)) => {
    //                 self.delayed_next_err = self.close_state.write_err(err);
    //                 return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
    //             }
    //             Poll::Pending => return Poll::Pending,
    //         }
    //     }
    //     while self.control_sent < self.control_queued
    //         || self.queued_pong.is_some()
    //         || self.close_state.queued()
    //     {
    //         if self.control_sent == self.control_queued {
    //             let mut queued = match self.queued_pong.take() {
    //                 None => None,
    //                 Some(payload) => Some((WsOpcode::Pong, payload)),
    //             };
    //             if let Some(payload) = self.close_state.unqueue() {
    //                 queued = Some((WsOpcode::Close, payload));
    //             }
    //             if let Some((opcode, payload)) = queued {
    //                 self.control_queued = WsFrame::encode(
    //                     FrameHead {
    //                         fin: true,
    //                         opcode,
    //                         mask: self.gen_mask(),
    //                         payload_len: payload.len() as u64,
    //                     },
    //                     payload.data(),
    //                     &mut self.control_buffer,
    //                 );
    //                 self.control_sent = 0;
    //             }
    //         }
    //         let write_slice = &self.control_buffer[self.control_sent..self.control_queued];
    //         match Pin::new(&mut *transport).poll_write(cx, write_slice) {
    //             Poll::Ready(Ok(n)) => self.control_sent += n,
    //             Poll::Ready(Err(err)) => {
    //                 self.delayed_next_err = self.close_state.write_err(err);
    //                 return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
    //             }
    //             Poll::Pending => return Poll::Pending,
    //         }
    //     }
    //     if self.control_queued != 0 || force_flush {
    //         match Pin::new(transport).poll_flush(cx) {
    //             Poll::Ready(Ok(())) => {
    //                 self.control_sent = 0;
    //                 self.control_queued = 0;
    //             }
    //             Poll::Ready(Err(err)) => {
    //                 self.delayed_next_err = self.close_state.write_err(err);
    //                 return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
    //             }
    //             Poll::Pending => return Poll::Pending,
    //         }
    //     }
    //     Poll::Ready(Ok(()))
    // }

    // fn poll_write<T: AsyncWrite + Unpin>(
    //     &mut self,
    //     transport: &mut T,
    //     cx: &mut Context<'_>,
    //     buf: &[u8],
    // ) -> Poll<io::Result<usize>> {
    //     if self.writing.is_none() {
    //         return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
    //     };
    //     if !self.data_sending && self.data_queued == self.data_buffer.len() {
    //         self.transition_to_sending(false)
    //     }
    //     match self.poll_write_transport(transport, cx, false) {
    //         Poll::Ready(Ok(())) => {}
    //         Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
    //         Poll::Pending => return Poll::Pending,
    //     }
    //     let n = min(self.data_buffer.len() - self.data_queued, buf.len());
    //     self.data_buffer[self.data_queued..self.data_queued + n].copy_from_slice(&buf[..n]);
    //     self.data_queued += n;
    //     Poll::Ready(Ok(n))
    // }
    //
    // fn gen_mask(&self) -> [u8; 4] {
    //     match self.config.mask {
    //         true => thread_rng().next_u32().to_ne_bytes(),
    //         false => [0u8, 0u8, 0u8, 0u8],
    //     }
    // }
    //
    // fn poll_flush<T: AsyncWrite + Unpin>(
    //     &mut self,
    //     transport: &mut T,
    //     cx: &mut Context<'_>,
    // ) -> Poll<io::Result<()>> {
    //     self.transition_to_sending(false);
    //     self.poll_write_transport(transport, cx, true)
    // }
    //
    // fn poll_close<T: AsyncWrite + Unpin>(
    //     &mut self,
    //     transport: &mut T,
    //     cx: &mut Context<'_>,
    // ) -> Poll<io::Result<()>> {
    //     self.transition_to_sending(true);
    //     self.poll_write_transport(transport, cx, true)
    // }
}

pub struct WsConnection<T: AsyncRead + AsyncWrite + Unpin> {
    transport: T,
    state: WsConnectionState,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsConnection<T> {
    // pub fn start_message(&mut self, kind: WsMessageKind) {
    //     self.state.start_message(kind)
    // }
    pub fn with_config(transport: T, config: WsConfig) -> Self {
        Self {
            transport,
            state: WsConnectionState::with_config(config),
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncRead for WsConnection<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let Self { transport, state, .. } = self.get_mut();
        if state.tmp {
            return Poll::Ready(Ok(0))
        }
        loop {
            match dbg!(state.decode_state.poll(transport, cx, buf)) {
                DecodeEvent::MessageStart(_) => panic!("unexpected message"),
                DecodeEvent::Control(_) => continue,
                DecodeEvent::InternalProgress => continue,
                DecodeEvent::ReadProgress(n, fin) => {
                    state.tmp = fin;
                    break Poll::Ready(Ok(n))
                },
                DecodeEvent::Failure(_) => break Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
                DecodeEvent::Pending => break Poll::Pending,
            }
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Stream for WsConnection<T> {
    type Item = Result<WsMessageKind, WsConnectionError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Self { transport, state, .. } = self.get_mut();
        loop {
            match dbg!(state.decode_state.poll(transport, cx, &mut [])) {
                DecodeEvent::MessageStart(kind) => {
                    state.tmp = false;
                    break Poll::Ready(Some(Ok(kind)))
                },
                DecodeEvent::Control(_) => continue,
                DecodeEvent::InternalProgress => continue,
                DecodeEvent::ReadProgress(_, _) => panic!("unexpected read"),
                DecodeEvent::Failure(err) => break Poll::Ready(Some(Err(err))),
                DecodeEvent::Pending => break Poll::Pending,
            }
        }
    }
}

// impl<T: AsyncRead + AsyncWrite + Unpin> AsyncWrite for WsConnection<T> {
//     fn poll_write(
//         self: Pin<&mut Self>,
//         cx: &mut Context<'_>,
//         buf: &[u8],
//     ) -> Poll<io::Result<usize>> {
//         let Self { transport, state, .. } = self.get_mut();
//         state.poll_write(transport, cx, buf)
//     }
//
//     fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
//         let Self { transport, state, .. } = self.get_mut();
//         state.poll_flush(transport, cx)
//     }
//
//     fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
//         let Self { transport, state, .. } = self.get_mut();
//         state.poll_close(transport, cx)
//     }
// }

#[derive(thiserror::Error, Debug)]
pub enum WsConnectionError {
    #[error("invalid utf8 in text message")]
    InvalidUtf8,
    #[error("incomplete utf8 in text message")]
    IncompleteUtf8,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    FrameDecodeError(#[from] FrameDecodeError),
    #[error("unexpected: {0:?}")]
    UnexpectedFrameKind(WsDataFrameKind),
}
