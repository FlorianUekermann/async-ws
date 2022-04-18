use async_http_codec::{RequestHeadEncoder, ResponseHeadEncoder};
use async_net_server_utils::tcp::TcpIncoming;
use async_ws::connection::{WsConfig, WsConnection};
use async_ws::http::{upgrade_request, upgrade_response};
use futures_lite::StreamExt;
use std::net::Ipv4Addr;
use std::time::Duration;

type TcpStream = async_net_server_utils::tcp::TcpStream;

#[allow(dead_code)]
async fn start_server(timeout: Option<Duration>) -> WsConnection<TcpStream> {
    let mut http_incoming = TcpIncoming::bind((Ipv4Addr::UNSPECIFIED, 23523))
        .unwrap()
        .http();
    let (tcp_stream, request) = http_incoming.next().await.unwrap();
    let resp_head = upgrade_response(&request).unwrap().into_parts().0;
    let _tcp_stream = ResponseHeadEncoder::default()
        .encode(tcp_stream, resp_head)
        .await
        .unwrap();
    let mut config = WsConfig::server();
    if let Some(timeout) = timeout {
        config.timeout = timeout;
    }
    todo!()
}

#[allow(dead_code)]
async fn start_client(_timeout: Option<Duration>) -> WsConnection<TcpStream> {
    let req_head = upgrade_request().body(()).unwrap().into_parts().0;
    let tcp_stream = TcpStream::connect((Ipv4Addr::LOCALHOST, 23523))
        .await
        .unwrap();
    RequestHeadEncoder::default()
        .encode(tcp_stream, req_head)
        .await
        .unwrap();
    todo!()
}

#[allow(dead_code)]
async fn start_server_and_client() -> (WsConnection<TcpStream>, WsConnection<TcpStream>) {
    todo!()
}
