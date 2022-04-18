use anyhow::bail;
use async_http_codec::ResponseHeadEncoder;
use async_net_server_utils::tcp::{TcpIncoming, TcpStream};
use async_ws::connection::{WsConfig, WsConnection, WsMessageReader, WsSend};
use async_ws::http::{is_upgrade_request, upgrade_response};
use futures::executor::{LocalPool, LocalSpawner};
use futures::prelude::*;
use futures::task::{LocalSpawnExt, SpawnExt};
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
            let spawner_clone = spawner.clone();
            spawner
                .spawn_local(async move {
                    if is_upgrade_request(&request) {
                        log::info!("upgrade request received");
                        let result = ws_handler(transport, request, spawner_clone).await;
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
    let resp_head = Response::builder()
        .header("Content-Length", HeaderValue::from(CLIENT_HTML.len()))
        .header("Connection", HeaderValue::from_static("close"))
        .body(())?
        .into_parts()
        .0;
    ResponseHeadEncoder::default()
        .encode(&mut transport, resp_head)
        .await?;
    transport.write_all(CLIENT_HTML.as_ref()).await?;
    transport.close().await?;
    Ok(())
}

async fn ws_handler(
    mut transport: TcpStream,
    request: Request<()>,
    spawner: LocalSpawner,
) -> anyhow::Result<()> {
    let resp_head = upgrade_response(&request).unwrap().into_parts().0;
    ResponseHeadEncoder::default()
        .encode(&mut transport, resp_head)
        .await?;
    let mut ws = WsConnection::with_config(transport, WsConfig::server());
    while let Some(reader) = ws.next().await {
        log::info!("new {:?} message", reader.kind());
        let ws_send = ws.send(reader.kind());
        spawner
            .spawn(async {
                if let Err(err) = msg_handler(reader, ws_send).await {
                    log::error!("message handler error: {:?}", err);
                };
            })
            .unwrap()
    }
    match ws.err() {
        None => log::info!("websocket closed without error"),
        Some(err) => log::error!("websocket closed with error: {:?}", err),
    }
    Ok(())
}

async fn msg_handler(
    mut reader: WsMessageReader<TcpStream>,
    ws_send: WsSend<TcpStream>,
) -> anyhow::Result<()> {
    let mut writer = match ws_send.await {
        Some(w) => w,
        None => bail!("ws closed unexpectedly while sending"),
    };
    let n = futures::io::copy(&mut reader, &mut writer).await?;
    writer.close().await?;
    log::info!("echoed {:?} message with {} bytes", reader.kind(), n);
    Ok(())
}
