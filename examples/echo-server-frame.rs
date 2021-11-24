use async_http_codec::head::encode::ResponseHeadEncoder;
use async_net_server_utils::tcp::TcpIncoming;
use async_ws::frame::{FrameHead, WsControlFrameKind, WsFrame, WsOpcode};
use async_ws::http::{is_upgrade_request, upgrade_response};
use futures::executor::block_on;
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
                    .encode(&mut transport, response)
                    .await
                    .unwrap();
                transport.write_all(CLIENT_HTML.as_ref()).await.unwrap();
                transport.close().await.unwrap();
                continue;
            }
            log::info!("upgrade request received");
            let response = upgrade_response(&request).unwrap();
            response_encoder
                .encode(&mut transport, response)
                .await
                .unwrap();
            loop {
                let frame = WsFrame::decode(&mut transport).await.unwrap().1;
                match frame {
                    WsFrame::Control(frame) => {
                        log::info!("received control frame: {:?}", &frame);
                        match frame.kind() {
                            WsControlFrameKind::Ping => {
                                let buffer = WsFrame::encode_vec(
                                    FrameHead {
                                        fin: true,
                                        opcode: WsOpcode::Pong,
                                        mask: [0u8, 0u8, 0u8, 0u8],
                                        payload_len: frame.payload().len() as u64,
                                    },
                                    frame.payload(),
                                );
                                transport.write_all(&buffer).await.unwrap();
                            }
                            WsControlFrameKind::Pong => {}
                            WsControlFrameKind::Close => break,
                        }
                    }
                    WsFrame::Data(frame) => {
                        log::info!("received data frame: {:?}", &frame);
                        let mut data = Vec::new();
                        frame
                            .payload_reader(&mut transport)
                            .read_to_end(&mut data)
                            .await
                            .unwrap();
                        log::info!("read data frame payload: {:?}", &data);
                        let buffer = WsFrame::encode_vec(
                            FrameHead {
                                fin: frame.fin(),
                                opcode: frame.kind().opcode(),
                                mask: [0u8, 0u8, 0u8, 0u8],
                                payload_len: data.len() as u64,
                            },
                            &data,
                        );
                        transport.write_all(&buffer).await.unwrap();
                    }
                }
            }
        }
    })
}
