//! The primary game server that interacts with players and observers.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use futures::{future, Sink, SinkExt, Stream, StreamExt};
use log::{debug, info, trace, warn};
use serde::Deserialize;
use serde_json::json;
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time,
};

use crate::game::{
    self,
    world::{Direction, World},
    BeeID, Moves, Player,
};

/// The information passed back by the game on successful creation.
pub type GameEventResponse = (broadcast::Receiver<game::Serializer>, Arc<World>);

/// An event to be passed to the active game.
#[derive(Debug)]
pub enum GameEvent {
    /// Add a new player to the game.
    Create {
        /// The player ID that's getting added.
        player: Player,
        /// Used to respond back on the status of the request.
        ///
        /// If the player was successfully added,
        /// provides the receiving end of a channel for game state updates
        /// and a reference to the (immutable) tile map for the game.
        response: oneshot::Sender<Result<GameEventResponse>>,
    },
    /// Move the player's bees within the game.
    Move {
        /// The player requesting the move.
        player: Player,
        /// The bees to be moved.
        moves: Vec<(BeeID, Option<Direction>)>,
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
pub async fn play_game(
    mut state: game::State,
    tick_rate: Duration,
    mut events: mpsc::Receiver<GameEvent>,
) {
    let mut next_moves = Moves::new();
    let mut interval = time::interval(tick_rate);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    let (updates, _) = broadcast::channel(1);
    let world = Arc::new(state.world().clone());

    loop {
        tokio::select! {
            // handle any events sent in
            event = events.recv() => {
                match event {
                    Some(GameEvent::Create{ player, response }) => {
                        trace!("Adding player {:?}", player);
                        let result = state
                            .add_player(player)
                            .map(|()| (updates.subscribe(), world.clone()));
                        response.send(result).unwrap();
                    },
                    Some(GameEvent::Move { player, moves }) => {
                        for (bee, direction) in moves {
                            if let Some(direction) = direction {
                                next_moves.insert((player, bee), direction);
                            } else {
                                next_moves.remove(&(player, bee));
                            }
                        }
                    },
                    Some(GameEvent::Finish) | None => break,
                }
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

/// Manage a single client socket.
///
/// Handles the `socket` associated with `player`.
/// Transmits events to the associated game using `events`.
///
/// The `_shutdown` channel is used to determine when the client has closed cleanly.
pub async fn handle_client<S, E>(
    socket: S,
    addr: SocketAddr,
    events: mpsc::Sender<GameEvent>,
    _shutdown: mpsc::Sender<()>,
) -> Result<()>
where
    S: Stream<Item = Result<String, E>> + Sink<String, Error = E>,
    E: std::error::Error + Send + Sync + 'static,
{
    let (sink, mut stream) = socket.split();
    let mut sink = sink.with(|x: serde_json::Value| future::ok::<_, E>(x.to_string()));
    let finished_msg = "Game already finished";

    // TODO: better registration (e.g. ping the player for some helpful info?)

    let player = Player::new();
    let (response, register_rx) = oneshot::channel();
    if let Err(e) = events.send(GameEvent::Create { player, response }).await {
        let msg = finished_msg;
        sink.send(json!({"type": "error", "msg": msg})).await?;
        sink.close().await?;
        return Err(anyhow!(e));
    }

    let mut updates = match register_rx.await.map_err(|_| anyhow!(finished_msg)) {
        Ok(Ok((updates, world))) => {
            info!("Registered {} as {:?}", addr, player);
            let payload = json!({
                "type": "registration",
                "world": *world,
                "player": player,
            });
            sink.send(payload).await?;
            updates
        }
        Ok(Err(e)) | Err(e) => {
            let msg = e.to_string();
            sink.send(json!({"type": "error", "msg": msg})).await?;
            sink.close().await?;
            return Err(e);
        }
    };

    loop {
        tokio::select! {
            res = updates.recv() => {
                match res {
                    Ok(state) => {
                        // TODO: filter to only things relevant for this player?
                        sink.send(json!({"type": "update", "data": state})).await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        let msg = format!("Lagging behind: skipped {} update(s)", skipped);
                        warn!("{:?} {}", player, msg);
                        sink.send(json!({"type": "warning", "msg": msg})).await?;
                    },
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            Some(packet) = stream.next() => {
                process_packet(player, packet, &mut sink, &events).await?;
            }
            else => break,
        }
    }

    sink.send(json!({"type": "done"})).await?;
    sink.close().await?;

    info!("Successfully closed {:?} ({})", player, addr);
    Ok(())
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
        pub bee: BeeID,
        #[serde(default)]
        pub direction: Option<Direction>,
    }

    match packet {
        Ok(input) => match serde_json::from_str::<Vec<ReadFrame>>(&input) {
            Ok(moves) => {
                let moves = moves.into_iter().map(|m| (m.bee, m.direction)).collect();
                let result = events.send(GameEvent::Move { player, moves }).await;
                if result.is_err() {
                    debug!("{:?} failed to send move event", player);
                }
            }
            Err(e) => {
                debug!("Bad input from {:?}: {} with input '{}'", player, e, input);
                let msg = "Bad input";
                sink.send(json!({"type": "warning", "msg": msg})).await?;
            }
        },
        Err(e) => {
            let msg = "Bad packet";
            sink.send(json!({"type": "warning", "msg": msg})).await?;
            // log the warning after sending it,
            // to not double up on errors due to connection failure
            warn!("Bad packet from {:?}: {}", player, e);
        }
    }

    Ok(())
}
