use async_http_codec::{BodyDecode, ResponseHeadEncoder};
use async_web_server::tcp::{TcpIncoming, TcpStream};
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
        while let Some(request) = http_incoming.next().await {
            spawner
                .spawn_local(async move {
                    if is_upgrade_request(&request) {
                        log::info!("upgrade request received");
                        let result = ws_handler(request).await;
                        log::info!("connection closed: {:?}", result)
                    } else {
                        log::info!("serve html: {:?}", serve_html(request).await);
                    }
                })
                .unwrap()
        }
    })
}

async fn serve_html(request: Request<BodyDecode<TcpStream>>) -> anyhow::Result<()> {
    let resp_head = Response::builder()
        .header("Content-Length", HeaderValue::from(CLIENT_HTML.len()))
        .header("Connection", HeaderValue::from_static("close"))
        .body(())?
        .into_parts()
        .0;
    let (_request_head, body) = request.into_parts();
    let mut transport = body.checkpoint().0;
    ResponseHeadEncoder::default()
        .encode(&mut transport, resp_head)
        .await?;
    transport.write_all(CLIENT_HTML.as_ref()).await?;
    transport.close().await?;
    Ok(())
}

async fn ws_handler(request: Request<BodyDecode<TcpStream>>) -> anyhow::Result<()> {
    let resp_head = upgrade_response(&request).unwrap().into_parts().0;
    let (_request_head, body) = request.into_parts();
    let mut transport = body.checkpoint().0;
    ResponseHeadEncoder::default()
        .encode(&mut transport, resp_head)
        .await?;
    let mut ws = WsConnection::with_config(transport, WsConfig::server());
    while let Some(mut reader) = ws.next().await {
        let mut writer = match ws.send(reader.kind()).await {
            None => break,
            Some(w) => w,
        };
        let n = futures::io::copy(&mut reader, &mut writer).await?;
        log::info!("echoed {:?} message with {} bytes", reader.kind(), n);
        writer.close().await?;
    }
    match ws.err() {
        None => Ok(()),
        Some(err) => Err(err.into()),
    }
}
