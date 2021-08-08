#![allow(dead_code)]

mod game;
mod server;

use std::time::Duration;

use anyhow::Result;
use tokio::{
    net::TcpListener,
    sync::{broadcast, mpsc},
};

use game::{world::World, Config, State};

#[tokio::main]
async fn main() -> Result<()> {
    let addr = "127.0.0.1:49998";
    let listener = TcpListener::bind(addr).await?;
    println!("Listening on {}", addr);

    let state = State::new(default_world()?, Config::default());
    let tick_rate = Duration::from_secs(2);
    let (events_tx, events_rx) = mpsc::channel(16);
    let (updates, _) = broadcast::channel(1);

    {
        let updates = updates.clone();
        tokio::spawn(server::play_game(state, tick_rate, events_rx, updates));
    }

    loop {
        let (socket, addr) = listener.accept().await?;
        let events_tx = events_tx.clone();
        let updates_rx = updates.subscribe();
        tokio::spawn(async move {
            println!("Handling new connection with address {}", addr);
            if let Err(x) = server::handle_client(socket, events_tx, updates_rx).await {
                println!("error: {:?}", x);
            }
        });
    }
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
