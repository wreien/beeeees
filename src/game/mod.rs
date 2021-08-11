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

use std::{fmt, ops::RangeInclusive, sync::Arc};

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
        static PLAYER_COUNTER: CounterU64 = CounterU64::new(1);
        Player(PLAYER_COUNTER.inc())
    }

    /// Create an observer player.
    ///
    /// This is not a "real" player for the purposes of the game;
    /// they cannot perform any actions and have no impact on the game,
    /// but can subscribe to view the current game state.
    #[must_use]
    pub fn observer() -> Self {
        Player(0)
    }

    /// Whether this ID indicates an observer or not.
    #[must_use]
    pub fn is_observer(&self) -> bool {
        self.0 == 0
    }
}

impl Default for Player {
    fn default() -> Self {
        Player::new()
    }
}

impl fmt::Display for Player {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_observer() {
            write!(f, "Observer")
        } else {
            write!(f, "Player({})", self.0)
        }
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

/// Manage mutable entities in the game.
#[derive(Debug, Clone, Serialize)]
struct Entities {
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

impl Entities {
    /// Create the set of entities for the game with given world.
    ///
    /// # TODO
    ///
    /// Do more than just "create nothing"; in particular, should create birds and cars.
    #[must_use]
    fn new<R: Rng + ?Sized>(_rng: &mut R, _world: &World) -> Self {
        Entities {
            bees: Vec::new(),
            hives: Vec::new(),
            flowers: Vec::new(),
            birds: Vec::new(),
            cars: Vec::new(),
        }
    }

    /// Perform one game tick. See also [`State::tick`].
    fn tick<R: Rng + ?Sized>(
        &mut self,
        config: &Config,
        rng: &mut R,
        world: &World,
        moves: &Moves,
    ) {
        // move animated entities
        for bee in &mut self.bees {
            bee.step(moves, world);
        }
        for bird in &mut self.birds {
            bird.step(world);
        }
        for car in &mut self.cars {
            car.step(world);
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
        let new_flowers = world.spawn_flowers(rng, config, &self.flowers);
        self.flowers.extend(new_flowers);

        // clean out any dead flowers
        // TODO: handle pollination (use drain_filter)
        self.flowers.retain(|f| f.pollen > 0);

        // each hive has a small chance of creating a new bee
        let new_bees = self.hives.iter().filter_map(|h| h.spawn_bee(rng, config));
        self.bees.extend(new_bees);
    }
}

/// The current game state.
#[derive(Debug)]
pub struct State {
    /// The tile map.
    world: World,

    /// Configuration for parameters and chances.
    config: Config,
    /// Available spawn points remaining.
    spawn_points: Vec<Position>,
    /// This state's random number generator.
    rng: StdRng,

    /// The current entities alive in the game.
    entities: Entities,
}

impl State {
    /// Create a new game.
    #[must_use]
    pub fn new(world: World, config: Config) -> State {
        let spawn_points = world.get_spawn_points();
        let mut rng = StdRng::from_entropy();

        // TODO: generate a bunch of entities to start with
        let entities = Entities::new(&mut rng, &world);

        State {
            world,
            config,
            spawn_points,
            rng,
            entities,
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
        self.entities.hives.iter().map(Hive::score).sum()
    }

    /// Get an independent serialisable view of the current state of the game.
    ///
    /// The returned serializer only represents
    /// the mutable members of the game.
    /// In addition it is a new block of memory,
    /// and is no longer tied to the game state;
    /// as such, mutating the game will not change the result
    /// of serialising the returned object.
    ///
    /// The returned object is safe to send across threads.
    ///
    /// If you want to serialize the current state of the game straight away,
    /// and don't require this extra functionality,
    /// consider using [`serialize`][State::serialize] instead.
    #[must_use]
    pub fn make_serializer(&self) -> Serializer {
        Serializer(Arc::new(self.entities.clone()))
    }

    /// Get a dependent serialisable view of the current state of the game.
    ///
    /// The returned serializer only represents
    /// the mutable members of the game.
    /// This only borrows the existing game state,
    /// and must be used straight away.
    ///
    /// If you need to cache the current state to serialise later,
    /// possibly on different threads,
    /// consider using [`make_serializer`][State::make_serializer] instead.
    #[must_use]
    pub fn serialize(&self) -> impl Serialize + '_ {
        &self.entities
    }

    /// Add a player to the game, starting them with a hive and some bees.
    ///
    /// # Errors
    ///
    /// May fail if there are no more available spawn points.
    pub fn add_player(&mut self, player: Player) -> Result<()> {
        assert!(!player.is_observer());
        let position = self
            .spawn_points
            .pop()
            .context("Could not add player: no more available spawn points")?;
        let (hive, bees) = Hive::new(player, position);
        self.entities.hives.push(hive);
        self.entities.bees.extend(bees);
        Ok(())
    }

    /// Perform one game tick. User input is taken in `moves`.
    pub fn tick(&mut self, moves: &Moves) {
        self.entities
            .tick(&self.config, &mut self.rng, &self.world, moves)
    }
}

/// A thread-safe cached serializer for a game state.
///
/// Refer to [`State::make_serializer`] for more details.
#[derive(Debug, Clone)]
pub struct Serializer(Arc<Entities>);

impl Serialize for Serializer {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}
