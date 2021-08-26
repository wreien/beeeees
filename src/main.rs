#![allow(dead_code)]

mod game;
mod server;

use std::{fs::File, io::BufReader, net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use futures::{future, Future, SinkExt, TryStreamExt};
use log::{debug, error, info};
use structopt::{clap::AppSettings, StructOpt};
use tokio::{net::TcpListener, signal};
use tokio_util::codec::{Decoder, LinesCodec};
use warp::{ws::Message, Filter};

/// Simple bees game.
///
/// A coöperative multiplayer game, where players must control swarms of bees
/// to collect as much pollen as possible. Developed for Reboot 2021.
#[derive(Debug, StructOpt)]
#[structopt(
    name = "beeeees",
    version_short = "v",
    setting(AppSettings::UnifiedHelpMessage),
    setting(AppSettings::DeriveDisplayOrder)
)]
struct Opts {
    /// Path to a config file with game parameters to load.
    #[structopt(parse(from_os_str))]
    config_file: Option<PathBuf>,

    /// Write missing options to the provided config file, creating it if it doesn't exist.
    #[structopt(short, long, requires("config-file"))]
    dump_config: bool,

    /// Address to bind the TCP listener.
    #[structopt(short, long, default_value = "127.0.0.1:49998", value_name = "ADDRESS")]
    tcp_addr: SocketAddr,

    /// Address to host the website.
    #[structopt(short, long, default_value = "127.0.0.1:8080", value_name = "ADDRESS")]
    web_addr: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    let Opts {
        config_file,
        dump_config,
        tcp_addr,
        web_addr,
    } = Opts::from_args();

    let config = config_file.as_ref().map_or_else(
        || Ok(game::Config::default()),
        |path| {
            // using std (blocking) types is OK here, as we have not started any async work
            let buf = BufReader::new(File::open(path).context("Could not open config file")?);
            serde_json::from_reader(buf).context("Could not parse config file")
        },
    );
    let config = config?;

    if dump_config {
        let path = config_file.expect("config-file is required by -d");
        let output = File::create(&path).context("Could not create specified config file")?;
        serde_json::to_writer_pretty(output, &config).context("Could not write to config file")?;
        let path = path.to_string_lossy();
        println!("Dumped current configuration options to {}", path);
        return Ok(());
    }

    let state = game::State::new(config);
    let tick_rate = Duration::from_secs(2);

    let game_server = server::make_game_server(state, tick_rate);
    tokio::spawn(game_server.server);

    let client_info = game_server.client_info;
    let tcpserver = tokio::spawn(make_tcp_server(tcp_addr, client_info.clone()));
    let webserver = tokio::spawn(make_web_server(web_addr, client_info.clone()));
    info!("Listening on tcp://{} and http://{}", tcp_addr, web_addr);

    // we're done with the channels, drop now to assist in cleanup later
    drop(client_info);

    let _ = signal::ctrl_c().await;

    info!("Interrupt requested, cleaning up...");
    game_server.shutdown.await;

    debug!("Ensuring external servers have cleaned up");
    webserver.await?;
    tcpserver.await?;

    Ok(())
}

async fn make_tcp_server(addr: SocketAddr, client_info: server::ClientState) {
    let tcp_listener = TcpListener::bind(addr)
        .await
        .expect("Couldn't bind to address");
    let mut shutdown = client_info.get_shutdown_notifier();

    loop {
        let (socket, addr) = tokio::select! {
            result = tcp_listener.accept() => result.expect("Couldn't accept new client"),
            _ = shutdown.recv() => break,
        };

        let socket = LinesCodec::new_with_max_length(8192).framed(socket);
        let channels = client_info.clone();
        tokio::spawn(async move {
            info!("Handling new connection with address {}", addr);
            if let Err(x) = server::handle_player(socket, addr, channels).await {
                error!("When handling TCP for {}: {:?}", addr, x);
            }
        });
    }

    debug!("TCP server shutting down");
}

fn make_web_server(addr: SocketAddr, client_info: server::ClientState) -> impl Future<Output = ()> {
    let mut signal = client_info.get_shutdown_notifier();

    let to_websocket = warp::addr::remote()
        .map(|addr: Option<SocketAddr>| addr.expect("no socket address available"))
        .and(warp::ws())
        .and(warp::any().map(move || client_info.clone()));

    let play = warp::path("play").and(to_websocket.clone()).map(
        |addr: SocketAddr, ws: warp::ws::Ws, channels| {
            ws.on_upgrade(move |socket| async move {
                tokio::spawn(async move {
                    let socket = socket
                        .try_take_while(|msg| future::ok(!msg.is_close()))
                        .try_filter_map(|msg| future::ok(msg.to_str().ok().map(String::from)))
                        .with(|s: String| future::ok::<_, warp::Error>(Message::text(s)));
                    if let Err(x) = server::handle_player(socket, addr, channels).await {
                        error!("When handling ws://./play for {}: {:?}", addr, x);
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
                    if let Err(x) = server::handle_observer(socket, addr, channels).await {
                        error!("When handling ws://./observe for {}: {:?}", addr, x);
                    }
                });
            })
        },
    );

    let server = warp::serve(play.or(observe).or(warp::fs::dir("./website")));

    let (_, server) = server.bind_with_graceful_shutdown(addr, async move {
        signal.recv().await;
        debug!("Web server shutting down");
    });

    server
}
