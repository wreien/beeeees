#![allow(dead_code)]

mod game;
mod server;

use std::time::Duration;

use anyhow::Result;
use log::{debug, error, info};
use tokio::{net::TcpListener, signal, sync::mpsc};
use tokio_util::codec::{Decoder, LinesCodec};

use game::{world::World, Config};

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

    let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (socket, addr) = result?;
                let socket = LinesCodec::new_with_max_length(8192).framed(socket);
                let events_tx = events_tx.clone();
                let shutdown_tx = shutdown_tx.clone();
                tokio::spawn(async move {
                    info!("Handling new connection with address {}", addr);
                    let fut = server::handle_client(
                        socket,
                        addr,
                        events_tx,
                        shutdown_tx,
                    );
                    if let Err(x) = fut.await {
                        error!("When handling {}: {:?}", addr, x);
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
