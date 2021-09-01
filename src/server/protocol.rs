use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::game::{
    self,
    world::{Direction, World},
};

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Send {
    Registration {
        world: Arc<World>,
        player: game::Player,
    },
    Update {
        data: game::Serializer,
    },
    Warning {
        msg: String,
    },
    Error {
        msg: String,
    },
    Done,
}

#[derive(Debug, Deserialize)]
pub struct Move {
    pub bee: game::BeeID,
    #[serde(default)]
    pub direction: Option<Direction>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Receive {
    Moves { moves: Vec<Move> },
    Register { name: String },
}
