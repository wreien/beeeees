//! Implementations of entity actions.

use std::collections::HashMap;

use global_counter::primitive::fast::ApproxCounterU64;
use rand::Rng;
use serde::{Deserialize, Serialize};

use super::{
    world::{Direction, Position, World},
    Config, Player,
};

/// Uniquely identifies a bee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BeeID(u64);

impl BeeID {
    /// Create a new bee identifier.
    ///
    /// The generated player will have a unique ID for this execution of the application.
    /// Note that IDs generated will be duplicated across different executions.
    #[must_use]
    pub fn new() -> Self {
        static BEE_COUNTER: ApproxCounterU64 = ApproxCounterU64::new(0, 20);
        BEE_COUNTER.inc();
        BeeID(BEE_COUNTER.get())
    }
}

/// Represents a set of bee actions made on a given turn.
///
/// Moves are indexed on pairs of the player who made the action,
/// and the ID for the bee that should move.
/// Moves where the player making the move and
/// the player controlling the affected bee don't match
/// are ignored.
///
/// Each bee can only get one action each turn.
///
/// An action is simply the direction the bee should move
/// for this game turn, if possible.
pub type Moves = HashMap<(Player, BeeID), Direction>;

/// A bee controlled by a player. Moves around the map and collects pollen
/// at the player's direction.
#[derive(Debug, Clone, Serialize)]
pub struct Bee {
    /// Uniquely identifies the bee.
    pub id: BeeID,
    /// Who controls this bee.
    pub player: Player,
    /// Where the bee currently is on the map.
    pub position: Position,
    /// How much pollen the bee currently has collected.
    pub pollen: i32,
    /// The amount of energy the bee has left to live.
    pub energy: i32,
    /// Where the last flower the bee collected pollen from was.
    #[serde(skip)]
    pub last_flower: Option<Position>,
}

impl Bee {
    /// Spawn a new bee at the given position.
    #[must_use]
    pub fn new(id: BeeID, player: Player, position: Position) -> Self {
        Self {
            id,
            player,
            position,
            pollen: 0,
            energy: 50,
            last_flower: None,
        }
    }

    /// Find the direction for the bee to move and go there, if possible.
    ///
    /// Regardless of success or not, expends one energy each turn.
    pub fn step(&mut self, moves: &Moves, world: &World) {
        if let Some(&dir) = moves.get(&(self.player, self.id)) {
            let new_pos = self.position.step(dir);
            match world.get(new_pos) {
                Some(tile) if tile.is_passable() => self.position = new_pos,
                _ => {}
            }
        }
        self.energy -= 1;
    }

    /// Rest the bee, while visiting a hive.
    pub fn rest(&mut self) {
        self.pollen = 0;
        self.last_flower = None;
        self.energy = (self.energy + 5).min(50);
    }

    /// Intermingle pollen with any flowers you're on.
    ///
    /// Bees on flowers transfer one unit of pollen each turn;
    /// if the flower has not been pollinated, and the bee has pollen,
    /// instead pollinates the flower.
    pub fn transfer_pollen(&mut self, flowers: &mut [Flower]) {
        let on_living_flower = |f: &&mut Flower| f.position == self.position && f.pollen > 0;
        if let Some(flower) = flowers.iter_mut().find(on_living_flower) {
            // TODO: handle another flower respawning right here?
            let here = Some(flower.position);
            if self.pollen > 0 && !flower.is_pollinated && self.last_flower != here {
                self.pollen -= 1;
                flower.is_pollinated = true;
            } else {
                flower.pollen -= 1;
                self.pollen += 1;
                self.last_flower = here;
            }
        }
    }

    /// Whether the bee is alive.
    ///
    /// If out of energy, or colliding with a bird or a car, the bee is dead.
    #[must_use]
    pub fn is_alive(&self, birds: &[Bird], cars: &[Car]) -> bool {
        self.energy > 0
            && birds.iter().all(|bird| bird.position != self.position)
            && cars.iter().all(|car| car.position != self.position)
    }
}

/// A player's hive. Each player will have exactly one hive.
///
/// Also tracks unique per-player information.
#[derive(Debug, Clone, Serialize)]
pub struct Hive {
    /// The player owning this hive.
    pub player: Player,
    /// Where the hive is on the map.
    pub position: Position,
    /// How much pollen this hive has collected so far.
    #[serde(skip)]
    score: i32,
}

impl Hive {
    /// Spawn a new hive at the given position.
    ///
    /// Returns a hive and any initial bees to be constructed at the hive.
    pub fn new(player: Player, position: Position) -> (Self, impl Iterator<Item = Bee>) {
        (
            Hive {
                player,
                position,
                score: 0,
            },
            (0..3).map(move |_| Bee::new(BeeID::new(), player, position)),
        )
    }

    /// The current amount of pollen stored by this particular hive.
    #[must_use]
    pub fn score(&self) -> i32 {
        self.score
    }

    /// Maybe spawn a bee at this hive.
    #[must_use]
    pub fn spawn_bee<R: Rng + ?Sized>(&self, rng: &mut R, config: &Config) -> Option<Bee> {
        rng.gen_bool(config.bee_spawn_chance)
            .then(|| Bee::new(BeeID::new(), self.player, self.position))
    }

    /// Find any of our bees on this hive.
    /// Transfer their pollen and increase our score.
    pub fn handle_bees(&mut self, bees: &mut [Bee]) {
        for bee in bees {
            if (bee.position, bee.player) == (self.position, self.player) {
                self.score += bee.pollen;
                bee.rest();
            }
        }
    }
}

/// A flower which can be visited to collect pollen.
///
/// When it runs out of pollen, the flower "dies".
/// If the flower was previously pollinated when it dies,
/// it will spawn a new flower nearby.
#[derive(Debug, Clone, Serialize)]
pub struct Flower {
    /// The location of the flower on the map.
    pub position: Position,
    /// How much pollen the flower has remaining.
    pub pollen: i32,
    /// Whether this flower has been pollinated.
    pub is_pollinated: bool,
}

impl Flower {
    /// Spawn a new flower at the given position, with an initial amount of pollen.
    #[must_use]
    pub fn new(position: Position, pollen: i32) -> Self {
        Self {
            position,
            pollen,
            is_pollinated: false,
        }
    }
}

/// A bird that flies around and eats any bees it passes.
#[derive(Debug, Clone, Serialize)]
pub struct Bird {
    pub position: Position,
}

impl Bird {
    pub fn step(&mut self, _world: &World) {
        todo!()
    }
}

/// A car that drives around on roads, killing any bees it crosses over.
#[derive(Debug, Clone, Serialize)]
pub struct Car {
    pub position: Position,
    pub facing: Direction,
}

impl Car {
    pub fn step(&mut self, _world: &World) {
        todo!()
    }
}
