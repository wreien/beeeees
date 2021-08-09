//! The primary game server that interacts with players and observers.

use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, Error, Result};
use serde::Deserialize;
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::{
        broadcast::{self, error::RecvError},
        mpsc, oneshot,
    },
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
        moves: Vec<(BeeID, Direction)>,
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
                        let result = state
                            .add_player(player)
                            .map(|()| (updates.subscribe(), world.clone()));
                        response.send(result).unwrap();
                    },
                    Some(GameEvent::Move { player, moves }) => {
                        for (bee, direction) in moves {
                            next_moves.insert((player, bee), direction);
                        }
                    },
                    Some(GameEvent::Finish) | None => break,
                }
            },
            // go to the next state
            _ = interval.tick() => {
                state.tick(&next_moves);
                // ignore errors of nobody connected yet
                let _ = updates.send(state.make_serializer());
                next_moves.clear();
            }
        }
    }
}

/// Write a newline-terminated JSON payload to the given sink.
async fn write_json<W>(writer: &mut W, payload: serde_json::Value) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    writer
        .write_all(format!("{}\n", payload).as_bytes())
        .await
        .map_err(Error::new)
}

/// Manage a single client socket.
///
/// Handles the `socket` associated with `player`.
/// Transmits events to the associated game using `events`.
///
/// The `_shutdown` channel is used to determine when the client has closed cleanly.
pub async fn handle_client(
    mut socket: TcpStream,
    events: mpsc::Sender<GameEvent>,
    _shutdown: mpsc::Sender<()>,
) -> Result<()> {
    let (reader, mut writer) = socket.split();
    let mut lines = BufReader::new(reader).lines();
    let finished_msg = "game already finished";

    // TODO: better registration (e.g. ping the player for some helpful info?)

    let player = Player::new();
    let (response, register_rx) = oneshot::channel();
    if let Err(e) = events.send(GameEvent::Create { player, response }).await {
        write_json(&mut writer, json!({"type": "error", "msg": finished_msg})).await?;
        writer.shutdown().await?;
        return Err(anyhow!(e));
    }

    let mut updates = match register_rx.await.map_err(|_| anyhow!(finished_msg)) {
        Ok(Ok((updates, world))) => {
            let payload = json!({
                "type": "registration",
                "world": *world,
                "player_id": player,
            });
            write_json(&mut writer, payload).await?;
            updates
        }
        Ok(Err(e)) | Err(e) => {
            write_json(&mut writer, json!({"type": "error", "msg": e.to_string()})).await?;
            writer.shutdown().await?;
            return Err(e);
        }
    };

    loop {
        /// Type used to parse a frame from the input.
        #[derive(Deserialize)]
        struct ReadFrame {
            pub bee: BeeID,
            pub direction: Direction,
        }

        tokio::select! {
            res = updates.recv() => {
                match res {
                    Ok(state) => {
                        // TODO: filter to only things relevant for this player?
                        write_json(&mut writer, json!({"type": "update", "data": state})).await?;
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        let msg = format!("lagging behind: skipped {} update(s)", skipped);
                        write_json(&mut writer, json!({"type": "warning", "msg": msg})).await?;
                    },
                    Err(RecvError::Closed) => break,
                }
            }
            line = lines.next_line() => {
                let line = match line? {
                    Some(x) => x,
                    None => break,
                };
                match serde_json::from_str::<Vec<ReadFrame>>(&line) {
                    Ok(moves) => {
                        let moves = moves.into_iter().map(|m| (m.bee, m.direction)).collect();
                        if let Err(_) = events.send(GameEvent::Move { player, moves }).await {
                            break;
                        }
                    }
                    Err(_) => {
                        write_json(&mut writer, json!({"type": "warning", "msg": "bad input, ignored"})).await?;
                    }
                }
            }
        }
    }

    write_json(&mut writer, json!({"type": "done"})).await?;
    writer.shutdown().await?;

    Ok(())
}
