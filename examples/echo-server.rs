use async_http_codec::{RequestHead, ResponseHead};
use async_net::{TcpListener, TcpStream};
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
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 8080))
            .await
            .unwrap();
        let mut incoming = listener.incoming();
        while let Some(Ok(stream)) = incoming.next().await {
            spawner
                .spawn_local(async move {
                    let (transport, head) = match RequestHead::decode(stream).await {
                        Ok(x) => x,
                        Err(err) => {
                            log::error!("request decode error: {:?}", err);
                            return;
                        }
                    };
                    let request = Request::from_parts(head.into(), transport);
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

async fn serve_html(request: Request<TcpStream>) -> anyhow::Result<()> {
    let resp_head = Response::builder()
        .header("Content-Length", HeaderValue::from(CLIENT_HTML.len()))
        .header("Connection", HeaderValue::from_static("close"))
        .body(())?
        .into_parts()
        .0;
    let (_request_head, mut transport) = request.into_parts();
    ResponseHead::from(resp_head).encode(&mut transport).await?;
    transport.write_all(CLIENT_HTML.as_ref()).await?;
    transport.close().await?;
    Ok(())
}

async fn ws_handler(request: Request<TcpStream>) -> anyhow::Result<()> {
    let resp_head = upgrade_response(&request).unwrap().into_parts().0;
    let (_request_head, mut transport) = request.into_parts();
    ResponseHead::from(resp_head).encode(&mut transport).await?;
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
