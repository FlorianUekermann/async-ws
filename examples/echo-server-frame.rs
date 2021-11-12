use async_http_codec::head::encode::ResponseHeadEncoder;
use async_net_server_utils::tcp::TcpIncoming;
use async_ws::frame_head::FrameHeadDecoder;
use async_ws::http::{is_upgrade_request, upgrade_response};
use futures_lite::future::block_on;
use futures_lite::prelude::*;
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

            let frame_head_decoder = FrameHeadDecoder::default();
            loop {
                let frame_head = frame_head_decoder
                    .decode_ref(&mut transport)
                    .await
                    .unwrap()
                    .1;
                let mut frame_payload = Vec::new();
                let mut frame_payload_reader = frame_head.payload_reader(&mut transport);
                frame_payload_reader
                    .read_to_end(&mut frame_payload)
                    .await
                    .unwrap();
                log::info!("received frame: {:?}", &frame_payload)
            }
        }
    })
}
