#![allow(dead_code)]

mod game;
mod server;

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use futures::{future, SinkExt, TryStreamExt};
use log::{debug, error, info};
use tokio::{net::TcpListener, signal, sync::mpsc};
use tokio_util::codec::{Decoder, LinesCodec};
use warp::{ws::Message, Filter};

use game::{world::World, Config};

use crate::server::handle_observer;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    let addr = "127.0.0.1:49998";
    let listener = TcpListener::bind(addr).await?;
    info!("Listening on {}", addr);

    let state = game::State::new(default_world()?, Config::default());
    let tick_rate = Duration::from_secs(2);
    let (events_tx, events_rx) = mpsc::channel(16);
    tokio::spawn(server::play_game(state, tick_rate, events_rx));

    let players = Arc::new(Mutex::new(HashMap::new()));
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

    let to_websocket = {
        let events = events_tx.clone();
        let players = players.clone();
        let shutdown = shutdown_tx.clone();
        let shared_state = warp::any()
            .map(move || (events.clone(), players.clone(), shutdown.clone()))
            .untuple_one();

        warp::addr::remote()
            .map(|addr: Option<SocketAddr>| addr.expect("no socket address available"))
            .and(warp::ws())
            .and(shared_state)
    };

    let play = warp::path("play").and(to_websocket.clone()).map(
        |addr: SocketAddr, ws: warp::ws::Ws, events, players, shutdown| {
            ws.on_upgrade(move |socket| async move {
                tokio::spawn(async move {
                    let socket = socket
                        .try_take_while(|msg| future::ok(!msg.is_close()))
                        .try_filter_map(|msg| future::ok(msg.to_str().ok().map(String::from)))
                        .with(|s: String| future::ok::<_, warp::Error>(Message::text(s)));
                    let fut = server::handle_player(socket, addr, events, players, shutdown);
                    if let Err(x) = fut.await {
                        error!("When handling ws://play for {}: {:?}", addr, x);
                    }
                });
            })
        },
    );

    let observe = warp::path("observe").and(to_websocket.clone()).map(
        |addr: SocketAddr, ws: warp::ws::Ws, events, _, shutdown| {
            ws.on_upgrade(move |socket| async move {
                tokio::spawn(async move {
                    let socket =
                        socket.with(|s: String| future::ok::<_, warp::Error>(Message::text(s)));
                    if let Err(x) = handle_observer(socket, addr, events, shutdown).await {
                        error!("When handling ws://observe for {}: {:?}", addr, x);
                    }
                });
            })
        },
    );

    let routes = play.or(observe).or(warp::fs::dir("./website"));

    tokio::spawn(warp::serve(routes).run("127.0.0.1:8080".parse::<SocketAddr>()?));

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (socket, addr) = result?;
                let socket = LinesCodec::new_with_max_length(8192).framed(socket);
                let events_tx = events_tx.clone();
                let players = players.clone();
                let shutdown_tx = shutdown_tx.clone();
                tokio::spawn(async move {
                    info!("Handling new connection with address {}", addr);
                    let fut = server::handle_player(socket, addr, events_tx, players, shutdown_tx);
                    if let Err(x) = fut.await {
                        error!("When handling TCP for {}: {:?}", addr, x);
                    }
                });
            },
            _ = signal::ctrl_c() => {
                info!("Interrupt requested, cleaning up...");
                break;
            }
        }
    }

    // stop any currently running game server
    // we ignore any failures, since that just means the server is already closed
    debug!("Requesting server to finish");
    let _ = events_tx.send(server::GameEvent::Finish).await;

    // wait for all client processes to finish cleanly
    debug!("Waiting for clients to clean up");
    drop(shutdown_tx);
    let _ = shutdown_rx.recv().await;

    Ok(())
}

#[rustfmt::skip]
fn default_world() -> Result<World> {
    use game::world::Tile::*;
    World::new(4, 4, vec![
        Grass, Grass, Grass, Grass,
        Grass, SpawnPoint, Grass, Grass,
        Grass, Grass, SpawnPoint, Grass,
        Grass, Grass, Grass, Grass,
    ])
}
