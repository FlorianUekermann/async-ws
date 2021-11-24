use async_http_codec::head::encode::ResponseHeadEncoder;
use async_net_server_utils::tcp::{TcpIncoming, TcpStream};
use async_ws::connection::{WsConfig, WsConnection};
use async_ws::http::{is_upgrade_request, upgrade_response};
use futures::executor::LocalPool;
use futures::prelude::*;
use futures::task::LocalSpawnExt;
use http::{HeaderValue, Request, Response};
use log::LevelFilter;
use simple_logger::SimpleLogger;
use std::net::Ipv4Addr;

const CLIENT_HTML: &str = include_str!("./echo-client.html");

fn main() {
    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .init()
        .unwrap();

    let mut pool = LocalPool::new();
    let spawner = pool.spawner();
    pool.run_until(async move {
        let mut http_incoming = TcpIncoming::bind((Ipv4Addr::UNSPECIFIED, 8080))
            .unwrap()
            .http();
        while let Some((transport, request)) = http_incoming.next().await {
            spawner
                .spawn_local(async move {
                    if is_upgrade_request(&request) {
                        log::info!("upgrade request received");
                        let result = handle_ws_upgrade(transport, request).await;
                        log::info!("connection closed: {:?}", result)
                    } else {
                        log::info!("serve html: {:?}", serve_html(transport).await);
                    }
                })
                .unwrap()
        }
    })
}

async fn serve_html(mut transport: TcpStream) -> anyhow::Result<()> {
    let response = Response::builder()
        .header("Content-Length", HeaderValue::from(CLIENT_HTML.len()))
        .header("Connection", HeaderValue::from_static("close"))
        .body(())?;
    ResponseHeadEncoder::default()
        .encode(&mut transport, response)
        .await?;
    transport.write_all(CLIENT_HTML.as_ref()).await?;
    transport.close().await?;
    Ok(())
}

async fn handle_ws_upgrade(mut transport: TcpStream, request: Request<()>) -> anyhow::Result<()> {
    let response = upgrade_response(&request).unwrap();
    ResponseHeadEncoder::default()
        .encode(&mut transport, response)
        .await?;
    let mut ws = WsConnection::with_config(transport, WsConfig::server());
    while let Some(event) = ws.next().await {
        let message_kind = event?;
        let mut payload = Vec::new();
        ws.read_to_end(&mut payload).await?;
        log::info!(
            "received {:?} message with {} bytes",
            message_kind,
            payload.len()
        );
        ws.start_message(message_kind);
        ws.write_all(&payload).await?;
        ws.close().await?;
    }
    Ok(())
}
