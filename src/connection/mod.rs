mod close;

use crate::connection::close::CloseState;
use crate::frame::{
    FrameDecodeError, FrameDecoderState, FrameHead, FramePayloadReaderState, WsControlFrame,
    WsControlFrameKind, WsControlFramePayload, WsDataFrame, WsDataFrameKind, WsFrame, WsOpcode,
};
use crate::message::WsMessageKind;
use futures::prelude::*;
use rand::{thread_rng, RngCore};
use std::cmp::min;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

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
    config: WsConfig,
    frame_decode: FrameDecoderState,
    frame_payload_reader: Option<FramePayloadReaderState>,
    frame_fin: bool,
    close_state: CloseState,
    delayed_next_err: Option<WsConnectionError>,
    control_buffer: [u8; 132],
    control_sent: usize,
    control_queued: usize,
    queued_pong: Option<WsControlFramePayload>,
    data_buffer: [u8; 1300],
    data_sending: bool,
    data_sent: usize,
    data_queued: usize,
    writing: Option<WsDataFrameKind>,
}

impl WsConnectionState {
    fn with_config(config: WsConfig) -> Self {
        Self {
            config,
            frame_decode: FrameDecoderState::new(),
            frame_payload_reader: None,
            frame_fin: true,
            close_state: CloseState::None,
            delayed_next_err: None,
            control_buffer: [0u8; 132],
            control_sent: 0,
            control_queued: 0,
            queued_pong: None,
            data_buffer: [0u8; 1300],
            data_sending: false,
            data_sent: 0,
            data_queued: 8,
            writing: None,
        }
    }
    pub fn start_message(&mut self, kind: WsMessageKind) {
        if self.writing.is_some() {
            self.transition_to_sending(true)
        }
        self.writing = Some(kind.frame_kind())
    }
    pub fn handle_control_frame(&mut self, frame: WsControlFrame) {
        let WsControlFrame { kind, payload } = frame;
        match kind {
            WsControlFrameKind::Ping => self.queued_pong = Some(payload),
            WsControlFrameKind::Pong => log::info!("pong"),
            WsControlFrameKind::Close => self.close_state.receive(payload),
        }
    }
    pub fn poll_read<T: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
        mut buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let mut filled = 0usize;
        loop {
            match self.poll_write_transport(transport, cx, false) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            }
            if self.frame_payload_reader.is_none() && self.frame_fin {
                return Poll::Ready(Ok(0));
            }
            if !self.close_state.open_for_receiving() {
                return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
            }
            if buf.len() == 0 {
                return Poll::Ready(Ok(filled));
            }
            match &mut self.frame_payload_reader {
                Some(reader) => match reader.poll_read(transport, cx, buf) {
                    Poll::Ready(Ok(0)) => {
                        self.frame_payload_reader = None;
                        if self.frame_fin {
                            return Poll::Ready(Ok(filled));
                        }
                    }
                    Poll::Ready(Ok(n)) => {
                        buf = &mut buf[n..];
                        filled += n
                    }
                    Poll::Ready(Err(err)) => {
                        let err = WsConnectionError::Io(err);
                        self.delayed_next_err = Some(self.close_state.receive_err(err));
                        return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
                    }
                    Poll::Pending => match filled {
                        0 => return Poll::Pending,
                        _ => return Poll::Ready(Ok(filled)),
                    },
                },
                None => match self.frame_decode.poll(transport, cx) {
                    Poll::Ready(Ok(frame)) => {
                        self.frame_decode = FrameDecoderState::new();
                        match frame {
                            WsFrame::Control(frame) => self.handle_control_frame(frame),
                            WsFrame::Data(WsDataFrame {
                                kind,
                                mask,
                                payload_len,
                                fin,
                            }) => match kind {
                                WsDataFrameKind::Continuation => {
                                    self.frame_payload_reader =
                                        Some(FramePayloadReaderState::new(mask, payload_len));
                                    self.frame_fin = fin;
                                }
                                kind => {
                                    let err = WsConnectionError::UnexpectedFrameKind(kind);
                                    self.delayed_next_err = Some(self.close_state.receive_err(err));
                                    return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
                                }
                            },
                        }
                    }
                    Poll::Ready(Err(err)) => {
                        let err = WsConnectionError::FrameDecodeError(err);
                        self.delayed_next_err = Some(self.close_state.receive_err(err));
                        return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
                    }
                    Poll::Pending => {}
                },
            }
        }
    }
    pub fn poll_next<T: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
    ) -> Poll<Option<<WsConnection<T> as Stream>::Item>> {
        if let Some(err) = self.delayed_next_err.take() {
            return Poll::Ready(Some(Err(err)));
        }
        if !self.close_state.open_for_receiving() {
            return Poll::Ready(None);
        }
        loop {
            match self.poll_write_transport(transport, cx, false) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(err)) => return Poll::Ready(Some(Err(err.into()))),
                Poll::Pending => return Poll::Pending,
            }
            if !self.close_state.open_for_receiving() {
                return Poll::Ready(None);
            }
            if self.frame_payload_reader.is_some() || !self.frame_fin {
                return Poll::Pending;
            }
            match self.frame_decode.poll(transport, cx) {
                Poll::Ready(Ok(frame)) => {
                    self.frame_decode = FrameDecoderState::new();
                    match frame {
                        WsFrame::Control(frame) => self.handle_control_frame(frame),
                        WsFrame::Data(WsDataFrame {
                            kind,
                            mask,
                            payload_len,
                            fin,
                        }) => match kind.message_kind() {
                            Some(kind) => {
                                self.frame_payload_reader =
                                    Some(FramePayloadReaderState::new(mask, payload_len));
                                self.frame_fin = fin;
                                return Poll::Ready(Some(Ok(kind)));
                            }
                            None => {
                                let err = WsConnectionError::UnexpectedFrameKind(kind);
                                return Poll::Ready(Some(Err(self.close_state.receive_err(err))));
                            }
                        },
                    }
                }
                Poll::Ready(Err(err)) => {
                    let err = WsConnectionError::FrameDecodeError(err);
                    return Poll::Ready(Some(Err(self.close_state.receive_err(err))));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }

    fn transition_to_sending(&mut self, fin: bool) {
        if !fin && self.data_queued == 8 {
            return;
        }
        if let Some(frame_kind) = self.writing {
            self.data_sending = true;
            let head = FrameHead {
                fin,
                opcode: frame_kind.opcode(),
                mask: self.gen_mask(),
                payload_len: (self.data_queued - 8) as u64,
            };
            self.data_sent = 8 - head.len_bytes();
            head.encode(&mut self.data_buffer[self.data_sent..]);
            self.writing = match fin {
                true => None,
                false => Some(WsDataFrameKind::Continuation),
            };
        }
    }

    fn poll_write_transport<T: AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
        force_flush: bool,
    ) -> Poll<io::Result<()>> {
        if !self.close_state.open_for_sending() {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        while self.data_sending {
            match Pin::new(&mut *transport)
                .poll_write(cx, &self.data_buffer[self.data_sent..self.data_queued])
            {
                Poll::Ready(Ok(n)) => {
                    self.data_sent += n;
                    if self.data_sent == self.data_queued {
                        self.data_sent = 0;
                        self.data_queued = 8;
                        self.data_sending = false;
                    }
                }
                Poll::Ready(Err(err)) => {
                    self.delayed_next_err = self.close_state.write_err(err);
                    return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        while self.control_sent < self.control_queued
            || self.queued_pong.is_some()
            || self.close_state.queued()
        {
            if self.control_sent == self.control_queued {
                let mut queued = match self.queued_pong.take() {
                    None => None,
                    Some(payload) => Some((WsOpcode::Pong, payload)),
                };
                if let Some(payload) = self.close_state.unqueue() {
                    queued = Some((WsOpcode::Close, payload));
                }
                if let Some((opcode, payload)) = queued {
                    self.control_queued = WsFrame::encode(
                        FrameHead {
                            fin: true,
                            opcode,
                            mask: self.gen_mask(),
                            payload_len: payload.len() as u64,
                        },
                        payload.data(),
                        &mut self.control_buffer,
                    );
                    self.control_sent = 0;
                }
            }
            let write_slice = &self.control_buffer[self.control_sent..self.control_queued];
            match Pin::new(&mut *transport).poll_write(cx, write_slice) {
                Poll::Ready(Ok(n)) => self.control_sent += n,
                Poll::Ready(Err(err)) => {
                    self.delayed_next_err = self.close_state.write_err(err);
                    return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        if self.control_queued != 0 || force_flush {
            match Pin::new(transport).poll_flush(cx) {
                Poll::Ready(Ok(())) => {
                    self.control_sent = 0;
                    self.control_queued = 0;
                }
                Poll::Ready(Err(err)) => {
                    self.delayed_next_err = self.close_state.write_err(err);
                    return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        Poll::Ready(Ok(()))
    }

    fn poll_write<T: AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.writing.is_none() {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        };
        if !self.data_sending && self.data_queued == self.data_buffer.len() {
            self.transition_to_sending(false)
        }
        match self.poll_write_transport(transport, cx, false) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
            Poll::Pending => return Poll::Pending,
        }
        let n = min(self.data_buffer.len() - self.data_queued, buf.len());
        self.data_buffer[self.data_queued..self.data_queued + n].copy_from_slice(&buf[..n]);
        self.data_queued += n;
        Poll::Ready(Ok(n))
    }

    fn gen_mask(&self) -> [u8; 4] {
        match self.config.mask {
            true => thread_rng().next_u32().to_ne_bytes(),
            false => [0u8, 0u8, 0u8, 0u8],
        }
    }

    fn poll_flush<T: AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        self.transition_to_sending(false);
        self.poll_write_transport(transport, cx, true)
    }

    fn poll_close<T: AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        self.transition_to_sending(true);
        self.poll_write_transport(transport, cx, true)
    }
}

pub struct WsConnection<T: AsyncRead + AsyncWrite + Unpin> {
    transport: T,
    state: WsConnectionState,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsConnection<T> {
    pub fn start_message(&mut self, kind: WsMessageKind) {
        self.state.start_message(kind)
    }
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
        let Self { transport, state } = self.get_mut();
        state.poll_read(transport, cx, buf)
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Stream for WsConnection<T> {
    type Item = Result<WsMessageKind, WsConnectionError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Self { transport, state } = self.get_mut();
        state.poll_next(transport, cx)
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncWrite for WsConnection<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let Self { transport, state } = self.get_mut();
        state.poll_write(transport, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let Self { transport, state } = self.get_mut();
        state.poll_flush(transport, cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let Self { transport, state } = self.get_mut();
        state.poll_close(transport, cx)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum WsConnectionError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    FrameDecodeError(#[from] FrameDecodeError),
    #[error("unexpected: {0:?}")]
    UnexpectedFrameKind(WsDataFrameKind),
}
