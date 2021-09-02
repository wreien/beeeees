//! A coöperative multiplayer network-based game,
//! where players must control swarms of bees to collect as much pollen as possible.
//!
//! Developed for Reboot 2021.
//!
//! ## Configuration
//!
//! Configuration is managed via a combination of
//! command-line arguments for the works of the server as a whole, and
//! a configuration file defining the rules and environment for the game.
//!
//! Command-line arguments are specified by the [`Opts`] structure,
//! and its documentation can be viewed by running `./beeeees --help`.
//!
//! Game configuration is serialised to and from a [`game::Config`] instance.
//!
//! ## Code layout
//!
//! The root module collects and parses command-line arguments,
//! reads any config files (if appropriate),
//! and creates and runs the game server,
//! as well as a TCP communication port and a website interface.
//!
//! Structures used for processing and running the game itself
//! can be found in the [`game`] module.
//! All game logic is defined and controlled here.
//!
//! The communication protocols, as well as the game server,
//! are defined in the [`server`] module.
//!
//! ## Logging
//!
//! The server uses [`env_logger`] to manage logs;
//! refer to its documentation for details on this works.

#![allow(dead_code)]
#![allow(rustdoc::private_intra_doc_links)]

mod game;
mod server;

use std::{fs::File, io::BufReader, net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use futures::{future, Sink, SinkExt, Stream, TryStreamExt};
use log::{debug, error, info};
use structopt::{clap::AppSettings, StructOpt};
use tokio::{net::TcpListener, signal};
use tokio_util::codec::{Decoder, LinesCodec, LinesCodecError};
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

/// Create a TCP server hosted at the given address.
///
/// Clients are initialized using the provided `client_info`.
/// Runs until it receives a shutdown signal over `client_info`.
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
        let socket = use_json_protocol(socket);

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

/// Create a web server hosted at the given address.
///
/// This serves the website used to observer the game,
/// and provides the websocket interface.
/// Clients are initialized using the provided `client_info`.
/// Server runs until it receives a shutdown signal over `client_info`.
///
/// The served files should be accessible from a folder `./website`,
/// relative to the program's current directory.
async fn make_web_server(addr: SocketAddr, client_info: server::ClientState) {
    let mut signal = client_info.get_shutdown_notifier();

    // transform a WebSocket into a stream matching the protocol
    let prepare = |socket: warp::ws::WebSocket| {
        let socket = socket
            .try_take_while(|msg| future::ok(!msg.is_close()))
            .try_filter_map(|msg| future::ok(msg.to_str().map(String::from).ok()))
            .with(|s| future::ok(Message::text(s)));
        use_json_protocol(socket)
    };

    let to_websocket = warp::addr::remote()
        .map(|addr: Option<SocketAddr>| addr.expect("no socket address available"))
        .and(warp::ws())
        .and(warp::any().map(move || client_info.clone()));

    let play = warp::path("play").and(to_websocket.clone()).map(
        move |addr: SocketAddr, ws: warp::ws::Ws, channels| {
            ws.on_upgrade(move |socket| async move {
                tokio::spawn(async move {
                    let socket = prepare(socket);
                    if let Err(x) = server::handle_player(socket, addr, channels).await {
                        error!("When handling ws://./play for {}: {:?}", addr, x);
                    }
                });
            })
        },
    );

    let observe = warp::path("observe").and(to_websocket).map(
        move |addr: SocketAddr, ws: warp::ws::Ws, channels| {
            ws.on_upgrade(move |socket| async move {
                tokio::spawn(async move {
                    let socket = prepare(socket);
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

    server.await;
}

/// Error type used to combine many kinds of protocol errors.
///
/// Just forwards implementations to the stored error.
/// Used by [`use_json_protocol`] to erase the type of
/// the stream's errors.
#[derive(Debug)]
enum ProtocolError {
    Codec(LinesCodecError),
    Serde(serde_json::Error),
    Warp(warp::Error),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolError::Codec(ref err) => err.fmt(f),
            ProtocolError::Serde(ref err) => err.fmt(f),
            ProtocolError::Warp(ref err) => err.fmt(f),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<LinesCodecError> for ProtocolError {
    fn from(err: LinesCodecError) -> Self {
        Self::Codec(err)
    }
}

impl From<warp::Error> for ProtocolError {
    fn from(err: warp::Error) -> Self {
        Self::Warp(err)
    }
}

impl From<serde_json::Error> for ProtocolError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serde(err)
    }
}

/// Convert a stream over [`String`] into
/// a stream over [`server::protocol`] types,
/// using [`serde_json`] as a serializer/deserializer.
///
/// This allows the stream to be used as the parameter
/// to functions like [`server::handle_player`].
///
/// Errors are coerced to [`ProtocolError`] for consistency.
fn use_json_protocol<S, E>(
    socket: S,
) -> impl Stream<Item = Result<server::protocol::Receive, ProtocolError>>
       + Sink<server::protocol::Send, Error = ProtocolError>
       + Unpin
where
    S: Stream<Item = Result<String, E>> + Sink<String, Error = E> + Unpin,
    E: Into<ProtocolError>,
{
    socket
        .err_into()
        .sink_err_into()
        .and_then(|line| {
            future::ready(serde_json::from_str(&line).map_err(|e| {
                debug!("Couldn't parse {}: {}", line, e);
                ProtocolError::from(e)
            }))
        })
        .with(|s| future::ready(serde_json::to_string(&s).map_err(ProtocolError::from)))
}
