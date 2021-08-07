//! Structures to actually run the game.
//!
//! The main point of interest is the [`State`] structure,
//! which stores all relevant state for a game execution.
//! The game progresses when the driver calls the [`State::tick`] function,
//! providing any relevant user input, which advances the game by one turn.
//!
//! To create a [`State`] you need to provide two pieces of information:
//! a [world][world::World] with the tileset for this game,
//! and some [configuration][Config] to set up various parameters for the game.
//! See their documentation for more information.
//!
//! User input is provided by the [`Moves`] type.
//! This is a map from the target bee to the desired action.
//! A specific bee is targeted using [`Player`] and [`BeeID`] values.
//! Any moves which do not specify a valid target are ignored.
//!
//! The game never finishes.
//! In theory, as long as input is provided, a game could run forever.
//! However, a driver may wish to set a "finish" point,
//! for example a certain number of ticks.
//! However, this is up to the driver to decide and implement.

mod entity;
pub mod world;

use std::ops::RangeInclusive;

use anyhow::{Context, Result};
use global_counter::primitive::exact::CounterU64;
use rand::prelude::*;
use serde::{Deserialize, Serialize};

use entity::{Bee, Bird, Car, Flower, Hive};
pub use entity::{BeeID, Moves};

use self::world::{Position, World};

/// Uniquely identifies a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Player(u64);

impl Player {
    /// Create a new player.
    ///
    /// The generated player will have a unique ID for this execution of the application.
    /// Note that IDs generated will be duplicated across different executions.
    #[must_use]
    pub fn new() -> Self {
        static PLAYER_COUNTER: CounterU64 = CounterU64::new(0);
        Player(PLAYER_COUNTER.inc())
    }
}

impl Default for Player {
    fn default() -> Self {
        Player::new()
    }
}

/// Configure game rules and constants.
#[derive(Debug)]
pub struct Config {
    /// Chance that a flower will spawn each turn.
    pub flower_spawn_chance: f64,
    /// The initial pollen value for a newly spawned flower.
    pub flower_initial_pollen: RangeInclusive<i32>,
    /// How likely a player is to spawn a new bee each turn.
    pub bee_spawn_chance: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            flower_spawn_chance: 0.05,
            flower_initial_pollen: 3..=5,
            bee_spawn_chance: 0.03,
        }
    }
}

/// The current game state.
#[derive(Debug, Serialize)]
pub struct State {
    /// The tile map.
    world: World,

    /// Configuration for parameters and chances.
    #[serde(skip)]
    config: Config,
    /// Available spawn points remaining.
    #[serde(skip)]
    spawn_points: Vec<Position>,
    /// This state's random number generator.
    #[serde(skip)]
    rng: StdRng,

    /// The currently living bees.
    bees: Vec<Bee>,
    /// The active player hives.
    hives: Vec<Hive>,
    /// The currently living flowers.
    flowers: Vec<Flower>,
    /// All birds in the game.
    birds: Vec<Bird>,
    /// All cars in the game.
    cars: Vec<Car>,
}

impl State {
    /// Create a new game.
    #[must_use]
    pub fn new(world: World, config: Config) -> State {
        let spawn_points = world.get_spawn_points();
        let rng = StdRng::from_entropy();

        // TODO: generate a bunch of entities to start with

        State {
            world,
            config,
            spawn_points,
            rng,
            bees: Vec::new(),
            hives: Vec::new(),
            flowers: Vec::new(),
            birds: Vec::new(),
            cars: Vec::new(),
        }
    }

    /// View the state's world information.
    #[must_use]
    pub fn world(&self) -> &world::World {
        &self.world
    }

    /// Get the current score of pollen collected.
    #[must_use]
    pub fn total_score(&self) -> i32 {
        self.hives.iter().map(Hive::score).sum()
    }

    /// Add a player to the game, starting them with a hive and some bees.
    ///
    /// # Errors
    ///
    /// May fail if there are no more available spawn points.
    pub fn add_player(&mut self, player: Player) -> Result<()> {
        let position = self
            .spawn_points
            .pop()
            .context("Could not add player: no more available spawn points")?;
        let (hive, bees) = Hive::new(player, position);
        self.hives.push(hive);
        self.bees.extend(bees);
        Ok(())
    }

    /// Perform one game tick. User input is taken in `moves`.
    pub fn tick(&mut self, moves: &Moves) {
        let rng = &mut self.rng;
        let config = &self.config;

        // move animated entities
        for bee in &mut self.bees {
            bee.step(moves, &self.world);
        }
        for bird in &mut self.birds {
            bird.step(&self.world);
        }
        for car in &mut self.cars {
            car.step(&self.world);
        }

        // bees on their own hives transfer pollen and increase score
        for hive in &mut self.hives {
            hive.handle_bees(&mut self.bees);
        }

        // filter dead bees
        let birds = &self.birds;
        let cars = &self.cars;
        self.bees.retain(|b| b.is_alive(birds, cars));

        // transfer pollen between bees and flowers
        for bee in &mut self.bees {
            bee.transfer_pollen(&mut self.flowers);
        }

        // spawn new flowers with small chance each turn
        let new_flowers = self.world.spawn_flowers(rng, config, &self.flowers);
        self.flowers.extend(new_flowers);

        // clean out any dead flowers
        // TODO: handle pollination (use drain_filter)
        self.flowers.retain(|f| f.pollen > 0);

        // each hive has a small chance of creating a new bee
        let new_bees = self.hives.iter().filter_map(|h| h.spawn_bee(rng, config));
        self.bees.extend(new_bees);
    }
}
