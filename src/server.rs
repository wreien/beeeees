//! The primary game server that interacts with players and observers.

use std::time::Duration;

use anyhow::Result;
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

use crate::game::{self, world::Direction, BeeID, Moves, Player};

/// An event to be passed to the active game.
#[derive(Debug)]
pub enum GameEvent {
    /// Add a new player to the game.
    Create {
        /// The player ID that's getting added.
        player: Player,
        /// Passes the result of adding the player.
        result: oneshot::Sender<Result<()>>,
    },
    /// Move the player's bees within the game.
    Move {
        /// The player requesting the move.
        player: Player,
        /// The bees to be moved.
        moves: Vec<(BeeID, Direction)>,
    },
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
    updates: broadcast::Sender<game::Serializer>,
) {
    let mut next_moves = Moves::new();
    let mut interval = time::interval(tick_rate);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            // handle any events sent in
            event = events.recv() => {
                match event {
                    Some(GameEvent::Create{ player, result }) => {
                        result.send(state.add_player(player)).unwrap();
                    },
                    Some(GameEvent::Move { player, moves }) => {
                        for (bee, direction) in moves {
                            next_moves.insert((player, bee), direction);
                        }
                    },
                    None => return,
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
        .map_err(anyhow::Error::new)
}

/// Manage a single client socket.
///
/// Handles the `socket` associated with `player`.
/// Transmits events to the associated game using `events`,
/// and passes along any `updates` it receives back to the socket.
pub async fn handle_client(
    mut socket: TcpStream,
    events: mpsc::Sender<GameEvent>,
    mut updates: broadcast::Receiver<game::Serializer>,
) -> Result<()> {
    let player = Player::new();
    let (reader, mut writer) = socket.split();
    let mut lines = BufReader::new(reader).lines();

    // TODO: manage initial handshaking

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
                    Err(RecvError::Closed) => {
                        write_json(&mut writer, json!({"type": "done"})).await?;
                        break;
                    },
                }
            }
            line = lines.next_line() => {
                let line = match line? {
                    Some(x) => x,
                    _ => break,
                };
                match serde_json::from_str::<Vec<ReadFrame>>(&line) {
                    Ok(moves) => {
                        let moves = moves.into_iter().map(|m| (m.bee, m.direction)).collect();
                        events.send(GameEvent::Move { player, moves }).await?;
                    }
                    Err(_) => {
                        write_json(&mut writer, json!({"type": "error", "msg": "bad input"})).await?;
                    }
                }
            }
        }
    }

    Ok(())
}
