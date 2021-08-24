//! The primary game server that interacts with players and observers.

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{anyhow, Result};
use futures::{future, Sink, SinkExt, Stream, StreamExt};
use log::{debug, info, trace, warn};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::game::{
    self,
    world::{Direction, World},
    Player,
};

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
        moves: Vec<(game::BeeID, Option<Direction>)>,
    },
    /// Finish the game.
    Finish,
}

/// Stores the communication channels required to operate a client.
#[derive(Debug, Clone)]
pub struct ClientChannels {
    pub events: mpsc::Sender<GameEvent>,
    pub players: Arc<Mutex<HashMap<String, Player>>>,
    pub shutdown: mpsc::Sender<()>,
}

impl ClientChannels {
    /// Creates a new set of communication channels.
    /// 
    /// Returns a triple containing:
    /// - The send half of the channels, used by the server
    /// - A receiver for game events
    /// - A receiver used to pause until shutdown
    #[must_use]
    pub fn new() -> (Self, mpsc::Receiver<GameEvent>, mpsc::Receiver<()>) {
        let (events_tx, events_rx) = mpsc::channel(16);
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let result = Self { 
            events: events_tx,
            players: Default::default(),
            shutdown: shutdown_tx,
        };

        (result, events_rx, shutdown_rx)
    }
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
pub async fn play_game(
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
                    for (bee, direction) in moves {
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
                trace!("Server tick");
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
/// The `player` can be an [observer][Player::observer];
/// in that case the player is not added to the game,
/// but we still subscribe to the receiver.
async fn register<S, E>(
    player: Player,
    sink: &mut S,
    addr: SocketAddr,
    events: &mpsc::Sender<GameEvent>,
) -> Result<broadcast::Receiver<game::Serializer>>
where
    S: Sink<serde_json::Value, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    trace!("Registering {} ({})", player, addr);
    let finished_msg = "Game already finished";

    let (response, register_rx) = oneshot::channel();
    if let Err(e) = events.send(GameEvent::AddPlayer { player, response }).await {
        let msg = finished_msg;
        sink.send(json!({"type": "error", "msg": msg})).await?;
        sink.close().await?;
        return Err(anyhow!(e));
    }

    match register_rx.await.map_err(|_| anyhow!(finished_msg)) {
        Ok(Ok((updates, world))) => {
            info!("Registered {} as {}", addr, player);
            let payload = json!({
                "type": "registration",
                "world": *world,
                "player": player,
            });
            sink.send(payload).await?;
            Ok(updates)
        }
        Ok(Err(e)) | Err(e) => {
            let msg = e.to_string();
            sink.send(json!({"type": "error", "msg": msg})).await?;
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
    socket: S,
    addr: SocketAddr,
    channels: ClientChannels,
) -> Result<()>
where
    S: Sink<String, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let events = channels.events;
    let mut sink = socket.with(|x: serde_json::Value| future::ok::<_, E>(x.to_string()));
    let mut updates = register(Player::observer(), &mut sink, addr, &events).await?;

    loop {
        // Note: we don't really care about lagging for observers
        // but worth logging a warning anyway, just in case
        use broadcast::error::RecvError::{Closed, Lagged};
        match updates.recv().await {
            Ok(state) => sink.send(json!({"type": "update", "data": state})).await?,
            Err(Lagged(skipped)) => warn!("{} lagging, skipped {} update(s)", addr, skipped),
            Err(Closed) => break,
        }
    }

    sink.send(json!({"type": "done"})).await?;
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
pub async fn handle_player<S, E>(
    socket: S,
    addr: SocketAddr,
    channels: ClientChannels,
) -> Result<()>
where
    S: Stream<Item = Result<String, E>> + Sink<String, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let (mut sink, mut stream) = socket.split();
    let name = match stream.next().await {
        Some(Ok(name)) => name,
        Some(Err(e)) => {
            let msg = e.to_string();
            sink.send(json!({"type": "error", "msg": msg}).to_string())
                .await?;
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

    let ClientChannels { events, players, .. } = channels;

    let player = *players
        .lock()
        .unwrap()
        .entry(name)
        .or_insert_with(Player::new);

    let mut sink = sink.with(|x: serde_json::Value| future::ok::<_, E>(x.to_string()));
    let updates = register(player, &mut sink, addr, &events).await?;

    // split into separate function so we can catch errors and send disconnection notices
    match player_processing_loop(player, &mut sink, stream, updates, &events).await {
        Ok(_) => {
            sink.send(json!({"type": "done"})).await?;
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
    T: Sink<serde_json::Value, Error = E> + Unpin,
    R: Stream<Item = Result<String, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    loop {
        tokio::select! {
            res = updates.recv() => match res {
                // TODO: filter to only things relevant for this player?
                Ok(state) => {
                    sink.send(json!({"type": "update", "data": state})).await?;
                },
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    let msg = format!("Lagging behind: skipped {} update(s)", skipped);
                    warn!("{} {}", player, msg);
                    sink.send(json!({"type": "warning", "msg": msg})).await?;
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
    packet: Result<String, E>,
    sink: &mut S,
    events: &mpsc::Sender<GameEvent>,
) -> Result<(), E>
where
    S: Sink<serde_json::Value, Error = E> + Unpin,
    E: std::error::Error,
{
    /// Type used to parse a frame from the input.
    #[derive(Deserialize)]
    struct ReadFrame {
        pub bee: game::BeeID,
        #[serde(default)]
        pub direction: Option<Direction>,
    }

    match packet {
        Ok(input) => match serde_json::from_str::<Vec<ReadFrame>>(&input) {
            Ok(moves) => {
                let moves = moves.into_iter().map(|m| (m.bee, m.direction)).collect();
                let result = events.send(GameEvent::Move { player, moves }).await;
                if result.is_err() {
                    debug!("{} failed to send move event", player);
                }
            }
            Err(e) => {
                debug!("Bad input from {}: {} with input '{}'", player, e, input);
                let msg = "Bad input";
                sink.send(json!({"type": "warning", "msg": msg})).await?;
            }
        },
        Err(e) => {
            let msg = "Bad packet";
            sink.send(json!({"type": "warning", "msg": msg})).await?;
            // log the warning after sending it,
            // to not double up on errors due to connection failure
            warn!("Bad packet from {}: {}", player, e);
        }
    }

    Ok(())
}
