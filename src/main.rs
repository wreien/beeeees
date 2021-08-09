#![allow(dead_code)]

mod game;
mod server;

use std::time::Duration;

use anyhow::Result;
use tokio::{net::TcpListener, signal, sync::mpsc};

use game::{world::World, Config};

#[tokio::main]
async fn main() -> Result<()> {
    let addr = "127.0.0.1:49998";
    let listener = TcpListener::bind(addr).await?;
    println!("Listening on {}", addr);

    let state = game::State::new(default_world()?, Config::default());
    let tick_rate = Duration::from_secs(2);
    let (events_tx, events_rx) = mpsc::channel(16);
    tokio::spawn(server::play_game(state, tick_rate, events_rx));

    let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (socket, addr) = result?;
                let events_tx = events_tx.clone();
                let shutdown_tx = shutdown_tx.clone();
                tokio::spawn(async move {
                    println!("Handling new connection with address {}", addr);
                    let fut = server::handle_client(
                        socket,
                        events_tx,
                        shutdown_tx,
                    );
                    if let Err(x) = fut.await {
                        println!("error: {:?}", x);
                    }
                });
            },
            _ = signal::ctrl_c() => {
                println!("Interrupt requested, cleaning up...");
                break;
            }
        }
    }

    // stop any currently running game server
    // we ignore any failures, since that just means the server is already closed
    let _ = events_tx.send(server::GameEvent::Finish).await;

    // wait for all client processes to finish cleanly
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
