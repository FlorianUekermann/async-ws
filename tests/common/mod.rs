use async_web_server::tcp::{TcpIncoming, TcpStream};
use async_ws::connection::{WsConfig, WsConnection};
use futures::future::join;
use futures::prelude::*;
use std::net::Ipv4Addr;
use std::time::Duration;
async fn start_server_ws(
    mut tcp_incoming: TcpIncoming,
    timeout: Option<Duration>,
) -> WsConnection<TcpStream> {
    let tcp_stream = tcp_incoming.next().await.unwrap();
    let mut config = WsConfig::server();
    if let Some(timeout) = timeout {
        config.timeout = timeout;
    }
    WsConnection::with_config(tcp_stream, config)
}

async fn start_client_transport(port: u16) -> TcpStream {
    TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap()
}

pub async fn start_server_ws_and_client_transport(
    server_timeout: Option<Duration>,
) -> (WsConnection<TcpStream>, TcpStream) {
    let tcp_incoming = TcpIncoming::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = tcp_incoming.local_addr().unwrap().port();
    join(
        start_server_ws(tcp_incoming, server_timeout),
        start_client_transport(port),
    )
    .await
}
