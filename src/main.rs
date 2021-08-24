#![allow(dead_code)]

mod game;
mod server;

use std::{net::SocketAddr, time::Duration};

use anyhow::Result;
use futures::{future, Future, SinkExt, TryStreamExt};
use log::{debug, error, info};
use tokio::{net::TcpListener, signal, sync::oneshot};
use tokio_util::codec::{Decoder, LinesCodec};
use warp::{ws::Message, Filter};

use crate::{
    game::{world::World, Config},
    server::{handle_observer, handle_player},
};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    let tcp_addr: SocketAddr = "127.0.0.1:49998".parse()?;
    let web_addr: SocketAddr = "0.0.0.0:80".parse()?;

    let (channels, events, mut clients_completed) = server::ClientChannels::new();

    let state = game::State::new(default_world()?, Config::default());
    let tick_rate = Duration::from_secs(2);
    let game_server = tokio::spawn(server::play_game(state, tick_rate, events));

    let (webserver, web_addr, webserver_shutdown_tx) = make_webserver(web_addr, channels.clone());
    let (tcpserver, tcpserver_shutdown_tx) = make_tcp_server(tcp_addr, channels.clone());

    let webserver = tokio::spawn(webserver);
    let tcpserver = tokio::spawn(tcpserver);
    info!("Listening on tcp://{} and http://{}", tcp_addr, web_addr);

    let _ = signal::ctrl_c().await;
    info!("Interrupt requested, cleaning up...");

    // stop any currently running game server
    // we ignore any failures, since that just means the server is already closed
    debug!("Requesting server to finish");
    let _ = channels.events.send(server::GameEvent::Finish).await;
    let _ = webserver_shutdown_tx.send(());
    let _ = tcpserver_shutdown_tx.send(());

    let _ = webserver.await?;
    let _ = tcpserver.await?;
    let _ = game_server.await?;

    // wait for all client processes to finish cleanly
    debug!("Waiting for clients to clean up");
    drop(channels);
    let _ = clients_completed.recv().await;

    Ok(())
}

fn make_tcp_server(addr: SocketAddr, channels: server::ClientChannels) 
    -> (impl Future<Output = Result<()>>, oneshot::Sender<()>)
{
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

    let fut = async move {
        let tcp_listener = TcpListener::bind(addr).await?;

        loop {
            let (socket, addr) = tokio::select! {
                result = tcp_listener.accept() => result?,
                _ = &mut shutdown_rx => break,
            };
            
            let socket = LinesCodec::new_with_max_length(8192).framed(socket);
            let channels = channels.clone();
            tokio::spawn(async move {
                info!("Handling new connection with address {}", addr);
                if let Err(x) = server::handle_player(socket, addr, channels).await {
                    error!("When handling TCP for {}: {:?}", addr, x);
                }
            });
        }

        debug!("TCP server shutting down");
        Ok(())
    };

    (fut, shutdown_tx)
}

fn make_webserver(
    addr: SocketAddr,
    channels: server::ClientChannels,
) -> (impl Future<Output = ()>, SocketAddr, oneshot::Sender<()>) {
    let to_websocket = warp::addr::remote()
        .map(|addr: Option<SocketAddr>| addr.expect("no socket address available"))
        .and(warp::ws())
        .and(warp::any().map(move || channels.clone()));

    let play = warp::path("play").and(to_websocket.clone()).map(
        |addr: SocketAddr, ws: warp::ws::Ws, channels| {
            ws.on_upgrade(move |socket| async move {
                tokio::spawn(async move {
                    let socket = socket
                        .try_take_while(|msg| future::ok(!msg.is_close()))
                        .try_filter_map(|msg| future::ok(msg.to_str().ok().map(String::from)))
                        .with(|s: String| future::ok::<_, warp::Error>(Message::text(s)));
                    if let Err(x) = handle_player(socket, addr, channels).await {
                        error!("When handling ws://play for {}: {:?}", addr, x);
                    }
                });
            })
        },
    );

    let observe = warp::path("observe").and(to_websocket).map(
        |addr: SocketAddr, ws: warp::ws::Ws, channels| {
            ws.on_upgrade(move |socket| async move {
                tokio::spawn(async move {
                    let socket =
                        socket.with(|s: String| future::ok::<_, warp::Error>(Message::text(s)));
                    if let Err(x) = handle_observer(socket, addr, channels).await {
                        error!("When handling ws://observe for {}: {:?}", addr, x);
                    }
                });
            })
        },
    );

    let server = warp::serve(play.or(observe).or(warp::fs::dir("./website")));

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (addr, server) = server.bind_with_graceful_shutdown(addr, async move {
        let _ = shutdown_rx.await;
        debug!("Web server shutting down")
    });

    (server, addr, shutdown_tx)
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
