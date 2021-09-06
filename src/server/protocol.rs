//! Defines the structures used for the server's communication protocol.

use std::{sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};

use crate::game::{
    self,
    world::{Direction, World},
};

/// Serialize a duration as a single [`f64`] representing the number of seconds.
fn serialize_duration_as_f64<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    duration.as_secs_f64().serialize(serializer)
}

/// Messages sent from the server.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Send {
    /// Sent on initial handshake,
    /// and provides any initial/immutable information
    /// about the game state.
    Registration {
        /// The world map.
        world: Arc<World>,
        /// A unique integer denoting the client's identifier.
        player: game::Player,
        /// The expected tick rate of the server.
        #[serde(serialize_with = "serialize_duration_as_f64")]
        tick_rate: Duration,
    },
    /// Sent regularly, providing an updated view of the current game state.
    ///
    /// Returns all relevant mutable state information at the current time.
    /// (No immutable data: that is covered in the initial registration.)
    Update {
        /// The mutable game data.
        data: game::Serializer,
    },
    /// Sent when an ignorable issue has occurred.
    ///
    /// The client's connection will still be maintained.
    Warning {
        /// A human-readable description of the issue.
        msg: String,
    },
    /// Sent when a fatal error has occurred.
    ///
    /// This will be sent as the last message before stream closure.
    Error {
        /// A human-readable description of the error.
        msg: String,
    },
    /// Sent on game shutdown.
    ///
    /// This will be sent as the last message before stream closure.
    Done,
}

/// Messages received from the client.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Receive {
    /// Sent on initial handshake,
    /// registers the player with the server
    /// and provides any appropriate metadata.
    Register {
        /// The player's name.
        ///
        /// Should be unique, and is used to allow reconnecting to an existing session
        /// if the player had disconnected earlier for whatever reason.
        name: String,
    },
    /// A set of bee movements to be made on the next tick.
    ///
    /// Multiple sets of these can be passed before a tick occurs:
    /// in that case, the last set of messages is the "winner",
    /// and overwrites previous instructions as applicable.
    ///
    /// Bees without actions for this tick will not move anywhere.
    /// This is equivalent to passing a direction of `None`.
    Moves {
        /// The set of moves to perform.
        moves: Vec<Move>,
    },
}

/// A single movement for a bee.
#[derive(Debug, Deserialize)]
pub struct Move {
    /// The bee that is moving.
    pub bee: game::BeeID,
    /// The direction the bee should move.
    /// `None` indicates that no movement should be made.
    #[serde(default)]
    pub direction: Option<Direction>,
}
