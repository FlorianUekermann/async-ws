use futures::prelude::*;
use crate::connection::WsConnection;
use std::task::{Context, Poll};
use std::pin::Pin;
use std::io;

struct WsMessageReader<'a, T: AsyncRead + AsyncWrite + Unpin> {
    connection: &'a mut WsConnection<T>,
    state: WsMessageReaderState,
}

struct WsMessageReaderState {

}

impl<'a, T: AsyncRead + AsyncWrite + Unpin> AsyncRead for WsMessageReader<'a, T> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        todo!()
    }
}
