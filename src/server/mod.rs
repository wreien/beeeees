//! The primary game server that interacts with players and observers.

pub mod protocol;

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{anyhow, Result};
use futures::{Future, Sink, SinkExt, Stream, StreamExt};
use log::{debug, info, trace, warn};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::game::{self, world::World, Player};

/// Used to receive and respond to a shutdown signal.
#[derive(Debug, Clone)]
pub struct Shutdown {
    /// Used to receive a shutdown signal.
    signal: watch::Receiver<bool>,
}

impl Shutdown {
    /// Creates a new shutdown signaller and receiver pair.
    ///
    /// Returns a new [`Shutdown`]
    /// as well as the write half to a `watch` used to initiate the shutdown.
    fn new() -> (Self, watch::Sender<bool>) {
        let (shutdown_signal_tx, shutdown_signal_rx) = watch::channel(false);
        (
            Self {
                signal: shutdown_signal_rx,
            },
            shutdown_signal_tx,
        )
    }

    /// Returns whether a shutdown has been signalled.
    pub fn should_shutdown(&self) -> bool {
        *self.signal.borrow()
    }

    /// Blocks until a shutdown is signalled.
    ///
    /// If a shutdown has already been signalled before this is called,
    /// will return immediately.
    ///
    /// This function is cancel-safe;
    /// that is, it can be used in `tokio::select`
    /// and if another branch is taken you are guaranteed
    /// that you haven't missed a notification.
    pub async fn recv(&mut self) {
        if self.should_shutdown() {
            return;
        }

        let _ = self.signal.changed().await;
    }
}

/// Stores the state associated with a client.
///
/// Stores the communication channels required to operate a client.
/// Also manages an instance of a [`Shutdown`]
/// that can be retrieved using [`ClientState::get_shutdown_notifier`].
#[derive(Debug, Clone)]
pub struct ClientState {
    /// Send events to the current game.
    events: mpsc::Sender<GameEvent>,
    /// Map of player names to player IDs.
    players: Arc<Mutex<HashMap<String, Player>>>,
    /// Used to receive notifications of impending shutdown.
    signal: Shutdown,
    /// Unused; when dropped signals that shutdown has finished successfully.
    _shutdown_complete: mpsc::Sender<()>,
}

impl ClientState {
    /// Get a copy of the shutdown notifier used by the client.
    pub fn get_shutdown_notifier(&self) -> Shutdown {
        self.signal.clone()
    }
}

/// Data representing a game server.
///
/// Created using [`make_game_server`].
pub struct GameServer<Server: Future, Shutdown: Future> {
    /// A future used to run the server.
    pub server: Server,
    /// Channel information used to communicate with the server.
    pub client_info: ClientState,
    /// A future that can be awaited to clean up the server.
    pub shutdown: Shutdown,
}

/// Construct a new game server.
///
/// Returns a pair with the state used to create and manage new clients,
/// and a future that can be awaited to initiate a clean shutdown.
///
/// After the future completes all clients will have shut down.
pub fn make_game_server(
    state: game::State,
    tick_rate: Duration,
) -> GameServer<impl Future<Output = ()>, impl Future<Output = ()>> {
    let (events_tx, events_rx) = mpsc::channel(16);
    let (shutdown_complete_tx, mut shutdown_complete_rx) = mpsc::channel(1);
    let (signal, shutdown_signal_tx) = Shutdown::new();

    let server = play_game(state, tick_rate, events_rx);

    let client_info = ClientState {
        events: events_tx.clone(),
        players: Default::default(),
        signal,
        _shutdown_complete: shutdown_complete_tx,
    };

    let shutdown = async move {
        debug!("Sending shutdown signal");
        let _ = shutdown_signal_tx.send(true);
        let _ = events_tx.send(GameEvent::Finish).await;

        debug!("Waiting for clients to clean up");
        let _ = shutdown_complete_rx.recv().await;
    };

    GameServer {
        server,
        client_info,
        shutdown,
    }
}

/// The information passed back by the game on successful creation.
pub type GameEventResponse = (broadcast::Receiver<game::Serializer>, Arc<World>);

/// An event to be passed to the active game.
#[derive(Debug)]
pub enum GameEvent {
    /// Add a player or observer to the game.
    ///
    /// Also used for reconnecting players who have previously disconnected.
    AddPlayer {
        /// The player ID that's getting added.
        ///
        /// If [`player.is_observer()`][Player::observer]
        /// then the player is added as an "observer";
        /// they will not have a spawn point generated for them,
        /// and as such will have no control over the game.
        /// They will still receive updates, however.
        player: Player,
        /// Used to respond back on the status of the request.
        ///
        /// If the player was successfully added,
        /// provides the receiving end of a channel for game state updates
        /// and a reference to the (immutable) tile map for the game.
        response: oneshot::Sender<Result<GameEventResponse>>,
    },
    /// Notify that a player has disconnected early from the game.
    ///
    /// Used to determine whether to admit a reconnecting player.
    Disconnect {
        /// The player that disconnected.
        player: Player,
    },
    /// Move the player's bees within the game.
    Move {
        /// The player requesting the move.
        player: Player,
        /// The bees to be moved.
        moves: Vec<protocol::Move>,
    },
    /// Finish the game.
    Finish,
}

/// Runs an instance of the game.
///
/// Will update the game `state` at a constant rate,
/// as denoted by `tick_rate`.
/// User input can be provided via `events`,
/// and the current game state will be regularly broadcast via `updates`.
/// If the `events` channel closes the game will finish.
///
/// # TODO
///
/// Support a "client-driven" pipeline
/// instead of the existing "server-driven" one;
/// that is, rather than tick at a constant speed and leave players behind,
/// always tick at the rate of the slowest connection
/// (with `tick_rate` as a maximum speed).
async fn play_game(
    mut state: game::State,
    tick_rate: Duration,
    mut events: mpsc::Receiver<GameEvent>,
) {
    let mut next_moves = game::Moves::new();
    let mut interval = tokio::time::interval(tick_rate);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut active_players = HashSet::new();
    let (updates, _) = broadcast::channel(1);
    let world = Arc::new(state.world().clone());

    loop {
        tokio::select! {
            // handle any events sent in
            event = events.recv() => match event {
                Some(GameEvent::AddPlayer{ player, response }) => {
                    trace!("Adding player {}", player);
                    let result = if player.is_observer() {
                        Ok(())
                    } else {
                        state.add_player(player).and_then(|_| {
                            if active_players.insert(player) {
                                Ok(())
                            } else {
                                Err(anyhow!("Duplicate player ID"))
                            }
                        })
                    };
                    let get_data = |_| (updates.subscribe(), world.clone());
                    response.send(result.map(get_data)).unwrap();
                },
                Some(GameEvent::Disconnect { player }) => {
                    debug!("Disconnecting {}", player);
                    if !active_players.remove(&player) {
                        warn!("Disconnecting {} that wasn't active?", player);
                    }
                }
                Some(GameEvent::Move { player, moves }) => {
                    assert!(!player.is_observer());
                    for protocol::Move { bee, direction } in moves {
                        if let Some(direction) = direction {
                            next_moves.insert((player, bee), direction);
                        } else {
                            next_moves.remove(&(player, bee));
                        }
                    }
                },
                Some(GameEvent::Finish) | None => break,
            },
            // go to the next state
            _ = interval.tick() => {
                trace!("Server tick: {:?}", next_moves);
                state.tick(&next_moves);
                // ignore errors of nobody connected yet
                let _ = updates.send(state.make_serializer());
                next_moves.clear();
            }
        }
    }

    info!("Game server shutting down");
}

/// Register the given `player` into the game,
/// using the `events` channel.
///
/// Notifies the player of their registration (or any issues)
/// via the provided `sink`.
///
/// Returns a receiver to be used to monitor any updates to the game state.
///
/// The `player` can be an [observer][`Player::observer`];
/// in that case the player is not added to the game,
/// but we still subscribe to the receiver.
async fn register<S, E>(
    player: Player,
    sink: &mut S,
    addr: SocketAddr,
    events: &mpsc::Sender<GameEvent>,
) -> Result<broadcast::Receiver<game::Serializer>>
where
    S: Sink<protocol::Send, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    trace!("Registering {} ({})", player, addr);
    let finished_msg = "Game already finished";

    let (response, register_rx) = oneshot::channel();
    if let Err(e) = events.send(GameEvent::AddPlayer { player, response }).await {
        let msg = String::from(finished_msg);
        sink.send(protocol::Send::Error { msg }).await?;
        sink.close().await?;
        return Err(anyhow!(e));
    }

    match register_rx.await.map_err(|_| anyhow!(finished_msg)) {
        Ok(Ok((updates, world))) => {
            info!("Registered {} as {}", addr, player);
            let msg = protocol::Send::Registration { world, player };
            sink.send(msg).await?;
            Ok(updates)
        }
        Ok(Err(e)) | Err(e) => {
            let msg = e.to_string();
            sink.send(protocol::Send::Error { msg }).await?;
            sink.close().await?;
            Err(e)
        }
    }
}

/// Manage a single observation socket.
///
/// We only take a sink, since we don't care about input we get.
/// The `events` is used to subscribe to the associated game.
///
/// The `_shutdown` channel is used to determine when the client has closed cleanly.
pub async fn handle_observer<S, E>(
    mut sink: S,
    addr: SocketAddr,
    channels: ClientState,
) -> Result<()>
where
    S: Sink<protocol::Send, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let events = channels.events;
    let mut updates = register(Player::observer(), &mut sink, addr, &events).await?;

    loop {
        // Note: we don't really care about lagging for observers
        // but worth logging a warning anyway, just in case
        use broadcast::error::RecvError::{Closed, Lagged};
        match updates.recv().await {
            Ok(data) => sink.send(protocol::Send::Update { data }).await?,
            Err(Lagged(skipped)) => warn!("{} lagging, skipped {} update(s)", addr, skipped),
            Err(Closed) => break,
        }
    }

    sink.send(protocol::Send::Done).await?;
    sink.close().await?;

    info!("Successfully closed observer ({})", addr);
    Ok(())
}

/// Manage a single client socket.
///
/// Handles the `socket` associated with `player`.
/// Transmits events to the associated game using `events`.
///
/// The `_shutdown` channel is used to determine when the client has closed cleanly.
pub async fn handle_player<S, E>(socket: S, addr: SocketAddr, channels: ClientState) -> Result<()>
where
    S: Stream<Item = Result<protocol::Receive, E>> + Sink<protocol::Send, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut shutdown = channels.get_shutdown_notifier();
    let (mut sink, mut stream) = socket.split();

    let packet = tokio::select! {
        packet = stream.next() => packet,
        _ = shutdown.recv() => {
            let msg = String::from("Game already finished");
            sink.send(protocol::Send::Error { msg }).await?;
            sink.close().await?;
            return Ok(());
        },
    };

    let name = match packet {
        Some(Ok(protocol::Receive::Register { name })) => name,
        Some(Ok(other)) => {
            let msg = String::from("Expected registration");
            sink.send(protocol::Send::Error { msg }).await?;
            sink.close().await?;
            return Err(anyhow!(format!("Expected registration, got {:?}", other)));
        }
        Some(Err(e)) => {
            let msg = e.to_string();
            sink.send(protocol::Send::Error { msg }).await?;
            sink.close().await?;
            return Err(anyhow!(e));
        }
        None => {
            return Err(anyhow!("Far side closed when collecting name."));
        }
    };

    if name.is_empty() {
        warn!("No name provided, downgrading {} to observer", addr);
        return handle_observer(sink, addr, channels).await;
    }

    let ClientState {
        events, players, ..
    } = channels;

    let player = *players
        .lock()
        .unwrap()
        .entry(name)
        .or_insert_with(Player::new);

    let updates = register(player, &mut sink, addr, &events).await?;

    // split into separate function so we can catch errors and send disconnection notices
    match player_processing_loop(player, &mut sink, stream, updates, &events).await {
        Ok(_) => {
            sink.send(protocol::Send::Done).await?;
            sink.close().await?;

            info!("Successfully closed {} ({})", player, addr);
            Ok(())
        }
        Err(e) => {
            if events.send(GameEvent::Disconnect { player }).await.is_err() {
                debug!("{} failed to send disconnection notice", player);
            }
            Err(e)
        }
    }
}

/// Implement the main processing loop for a player connection.
///
/// Only finishes if either an error occurs or if the game shuts down.
async fn player_processing_loop<T, R, E>(
    player: Player,
    sink: &mut T,
    mut stream: R,
    mut updates: broadcast::Receiver<game::Serializer>,
    events: &mpsc::Sender<GameEvent>,
) -> Result<()>
where
    T: Sink<protocol::Send, Error = E> + Unpin,
    R: Stream<Item = Result<protocol::Receive, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    loop {
        tokio::select! {
            res = updates.recv() => match res {
                // TODO: filter to only things relevant for this player?
                Ok(data) => {
                    sink.send(protocol::Send::Update{ data }).await?;
                },
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    let msg = format!("Lagging behind: skipped {} update(s)", skipped);
                    warn!("{} {}", player, msg);
                    sink.send(protocol::Send::Warning{ msg }).await?;
                },
                Err(broadcast::error::RecvError::Closed) => {
                    return Ok(());
                },
            },
            packet = stream.next() => match packet {
                Some(packet) => process_packet(player, packet, sink, events).await?,
                None => return Err(anyhow!("Far side closed when processing packets.")),
            },
        }
    }
}

/// Process a packet received from a player.
///
/// Ignores any errors sending results to the game,
/// since this means that the game should be entering shutdown anyway.
async fn process_packet<S, E>(
    player: Player,
    packet: Result<protocol::Receive, E>,
    sink: &mut S,
    events: &mpsc::Sender<GameEvent>,
) -> Result<(), E>
where
    S: Sink<protocol::Send, Error = E> + Unpin,
    E: std::error::Error,
{
    match packet {
        Ok(protocol::Receive::Moves { moves }) => {
            trace!("Parsed {}'s message: {:?}", player, moves);
            let result = events.send(GameEvent::Move { player, moves }).await;
            if result.is_err() {
                debug!("{} failed to send move event", player);
            }
        }
        Ok(protocol::Receive::Register { .. }) => {
            debug!("Bad input from {}: registration", player);
            let msg = String::from("Bad input");
            sink.send(protocol::Send::Warning { msg }).await?;
        }
        Err(e) => {
            debug!("Bad input from {}: {}", player, e);
            let msg = String::from("Bad input");
            sink.send(protocol::Send::Warning { msg }).await?;
        }
    }

    Ok(())
}
