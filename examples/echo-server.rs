use async_http_codec::head::encode::ResponseHeadEncoder;
use async_net_server_utils::tcp::TcpIncoming;
use async_ws::frame::{FrameEncoder, FrameHeadDecoder, WsOpcode};
use async_ws::http::{is_upgrade_request, upgrade_response};
use futures::executor::block_on;
use futures::future::block_on;
use futures::prelude::*;
use http::{HeaderValue, Response};
use log::LevelFilter;
use simple_logger::SimpleLogger;
use std::net::Ipv4Addr;

const CLIENT_HTML: &str = include_str!("./echo-client.html");

fn main() {
    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .init()
        .unwrap();

    block_on(async {
        let response_encoder = ResponseHeadEncoder::default();
        let mut http_incoming = TcpIncoming::bind((Ipv4Addr::UNSPECIFIED, 8080))
            .unwrap()
            .http();
        while let Some((mut transport, request)) = http_incoming.next().await {
            if !is_upgrade_request(&request) {
                let response = Response::builder()
                    .header("Content-Length", HeaderValue::from(CLIENT_HTML.len()))
                    .header("Connection", HeaderValue::from_static("close"))
                    .body(())
                    .unwrap();
                response_encoder
                    .encode_ref(&mut transport, response)
                    .await
                    .unwrap();
                transport.write_all(CLIENT_HTML.as_ref()).await.unwrap();
                transport.close().await.unwrap();
                continue;
            }
            log::info!("upgrade request received");
            let response = upgrade_response(&request).unwrap();
            response_encoder
                .encode_ref(&mut transport, response)
                .await
                .unwrap();
        }
    })
}
