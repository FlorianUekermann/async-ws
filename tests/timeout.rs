use crate::common::start_server_ws_and_client_transport;
use async_io::Timer;
use async_ws::connection::{WsConfig, WsConnection, WsConnectionError};
use async_ws::frame::{FrameDecoderState, WsControlFrame, WsControlFrameKind, WsFrame};
use futures::executor::block_on;
use futures::future::join;
use futures::prelude::*;
use futures_lite::future::race;
use smol_timeout::TimeoutExt;
use std::time::{Duration, Instant};

mod common;

const ONE_MS: Duration = Duration::from_millis(1);
const TWO_MS: Duration = Duration::from_millis(2);
const THREE_MS: Duration = Duration::from_millis(3);
const TEN_MS: Duration = Duration::from_millis(10);

#[test]
fn idle_keepalive() {
    block_on(async {
        let (mut server, client) = start_server_ws_and_client_transport(Some(ONE_MS)).await;
        let mut client = WsConnection::with_config(client, WsConfig::client());
        let result = join(server.next(), client.next()).timeout(TEN_MS).await;
        assert!(result.is_none())
    })
}

#[test]
fn idle_keepalive_failure() {
    block_on(async {
        let (mut server, client) = start_server_ws_and_client_transport(Some(ONE_MS)).await;
        let result = server.next().timeout(TEN_MS).await;
        drop(client);
        if let Some(None) = result {
            if let Some(WsConnectionError::Timeout) = server.err().as_deref() {
                return;
            }
            panic!("expected timeout error, got: {:?}", server.err())
        }
        panic!("expected end of stream")
    })
}

#[test]
fn idle_keepalive_timing() {
    block_on(async {
        let (mut server, mut client) = start_server_ws_and_client_transport(Some(TWO_MS)).await;
        let mut start = Instant::now();
        let completed = race(
            async {
                server.next().await;
                false
            },
            async {
                for _ in 0..10 {
                    let frame = FrameDecoderState::new()
                        .restore(&mut client)
                        .await
                        .unwrap()
                        .1;
                    let frame = match frame {
                        WsFrame::Control(frame) => frame,
                        WsFrame::Data(_) => panic!("unexpected data frame"),
                    };
                    let frame_kind = frame.kind();
                    if frame_kind != WsControlFrameKind::Ping {
                        panic!("unexpected control frame kind: {:?}", frame_kind)
                    }
                    let elapsed = start.elapsed();
                    if elapsed < TWO_MS {
                        panic!("ping arrived early: {:?}", elapsed)
                    }
                    if elapsed > THREE_MS {
                        panic!("ping arrived late: {:?}", elapsed)
                    }
                    Timer::after(ONE_MS).await;
                    let pong = WsControlFrame::new(WsControlFrameKind::Pong, frame.payload());
                    let pong_buffer = WsFrame::encode_vec(pong.head([1, 1, 1, 1]), pong.payload());
                    client.write_all(&pong_buffer).await.unwrap();
                    start = Instant::now();
                }
                true
            },
        )
        .await;
        assert!(completed);
    })
}
