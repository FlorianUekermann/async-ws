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
    closed: Option<WsControlFramePayload>,
    error: Option<Option<WsConnectionError>>,
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
            closed: None,
            error: None,
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
        if let Some(frame_kind) = self.writing {
            self.transition_to_sending(frame_kind, true)
        }
        self.writing = Some(kind.frame_kind())
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
            if self.error.is_some() {
                return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into()));
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
                    Poll::Ready(Err(err)) => self.error = Some(Some(err.into())),
                    Poll::Pending => match filled {
                        0 => return Poll::Pending,
                        _ => return Poll::Ready(Ok(filled)),
                    },
                },
                None => match self.frame_decode.poll(transport, cx) {
                    Poll::Ready(Ok(frame)) => {
                        self.frame_decode = FrameDecoderState::new();
                        match frame {
                            WsFrame::Control(WsControlFrame { kind, payload }) => match kind {
                                WsControlFrameKind::Ping => self.queued_pong = Some(payload),
                                WsControlFrameKind::Pong => log::info!("pong"),
                                WsControlFrameKind::Close => self.closed = Some(payload),
                            },
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
                                    self.error =
                                        Some(Some(WsConnectionError::UnexpectedFrameKind(kind)))
                                }
                            },
                        }
                    }
                    Poll::Ready(Err(err)) => self.error = Some(Some(err.into())),
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
        loop {
            match self.poll_write_transport(transport, cx, false) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(err)) => {
                    self.error = Some(Some(err.into()));
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
            if let Some(err) = &mut self.error {
                return Poll::Ready(err.take().map(|err| Err(err)));
            }
            if self.frame_payload_reader.is_some() || !self.frame_fin {
                return Poll::Pending;
            }
            match self.frame_decode.poll(transport, cx) {
                Poll::Ready(Ok(frame)) => {
                    self.frame_decode = FrameDecoderState::new();
                    match frame {
                        WsFrame::Control(WsControlFrame { kind, payload }) => match kind {
                            WsControlFrameKind::Ping => self.queued_pong = Some(payload),
                            WsControlFrameKind::Pong => log::info!("pong"),
                            WsControlFrameKind::Close => self.closed = Some(payload),
                        },
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
                                self.error =
                                    Some(Some(WsConnectionError::UnexpectedFrameKind(kind)))
                            }
                        },
                    }
                }
                Poll::Ready(Err(err)) => self.error = Some(Some(err.into())),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
    fn poll_write_transport<T: AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
        force_flush: bool,
    ) -> Poll<io::Result<()>> {
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
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            }
        }
        while self.control_sent < self.control_queued || self.queued_pong.is_some() {
            if self.control_sent == self.control_queued {
                if let Some(payload) = self.queued_pong.take() {
                    let head = FrameHead {
                        fin: true,
                        opcode: WsOpcode::Pong,
                        mask: self.gen_mask(),
                        payload_len: payload.len() as u64,
                    };
                    self.control_sent = 0;
                    self.control_queued =
                        WsFrame::encode(head, payload.data(), &mut self.control_buffer)
                }
            }
            match Pin::new(&mut *transport).poll_write(
                cx,
                &self.control_buffer[self.control_sent..self.control_queued],
            ) {
                Poll::Ready(Ok(n)) => self.control_sent += n,
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            }
        }
        if self.control_queued != 0 || force_flush {
            match Pin::new(transport).poll_flush(cx) {
                Poll::Ready(Ok(())) => {
                    self.control_sent = 0;
                    self.control_queued = 0;
                }
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
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
        let frame_kind = match self.writing {
            None => return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
            Some(frame_kind) => frame_kind,
        };
        if !self.data_sending && self.data_queued == self.data_buffer.len() {
            self.transition_to_sending(frame_kind, false);
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
    fn transition_to_sending(&mut self, frame_kind: WsDataFrameKind, fin: bool) {
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
        }
    }
    fn poll_flush<T: AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        let frame_kind = match self.writing {
            None => return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
            Some(frame_kind) => frame_kind,
        };
        self.transition_to_sending(frame_kind, false);
        self.poll_write_transport(transport, cx, true)
    }
    fn poll_close<T: AsyncWrite + Unpin>(
        &mut self,
        transport: &mut T,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        if let Some(frame_kind) = self.writing {
            self.transition_to_sending(frame_kind, true);
        }
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
