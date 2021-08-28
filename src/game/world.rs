//! Types used to describe the game world.

use std::{convert::TryFrom, iter::from_fn, ops::Index};

use anyhow::{bail, Context, Error};
use rand::{distributions::WeightedIndex, prelude::*};
use serde::{Deserialize, Serialize};

use super::{entity::Flower, Config};

/// Represents the cardinal directions on the plane.
///
/// See also [`World`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Direction {
    North,
    East,
    South,
    West,
}

/// A position on the [`World`] grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    /// The horizontal position; 0 is closest to the left.
    pub x: i32,
    /// The vertical position; 0 is closest to the bottom.
    pub y: i32,
}

impl Position {
    /// Create a new position from an `x` and `y` coordinate.
    #[must_use]
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// Get the next tile immediately in the given direction.
    #[must_use]
    pub fn step(self, dir: Direction) -> Position {
        match dir {
            Direction::North => Position::new(self.x, self.y + 1),
            Direction::East => Position::new(self.x + 1, self.y),
            Direction::South => Position::new(self.x, self.y - 1),
            Direction::West => Position::new(self.x - 1, self.y),
        }
    }
}

/// Different kinds of tiles on the map.
///
/// These are unchanging and constant throughout the duration of a game.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Tile {
    /// Normal terroritory. Can spawn flowers.
    Grass,
    /// Very "flowerful" terrain.
    Garden,
    /// Passable terrain, but cannot spawn flowers: water, footpaths, etc.
    Neutral,
    /// Passable terrain, cannot spawn flowers; cars can drive through.
    Road,
    /// Impassable terrain
    Block,
    /// Can spawn hives on it, but will not spawn flowers etc.
    SpawnPoint,
}

impl Tile {
    /// Whether this tile can be passed through by bees.
    #[must_use]
    pub fn is_passable(self) -> bool {
        !matches!(self, Self::Block)
    }

    /// The weighting for how likely flowers are to spawn on this tile.
    ///
    /// Higher values mean more likely to spawn here, if a flower should be spawned.
    /// A value of `0.0` makes it impossible.
    #[must_use]
    pub fn spawn_weight(self) -> f64 {
        match self {
            Self::Grass => 0.3,
            Self::Garden => 1.0,
            _ => 0.0,
        }
    }

    /// Returns `true` if the tile is a [`SpawnPoint`][`Tile::SpawnPoint`].
    #[must_use]
    pub fn is_spawn_point(self) -> bool {
        matches!(self, Self::SpawnPoint)
    }
}

/// Stores the world map for the game.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "WorldDeserializer")]
pub struct World {
    /// The width of the map, in number of tiles.
    pub width: i32,
    /// The height of the map, in number of tiles.
    pub height: i32,
    /// The contents of the map. Row-major, with the first cell at the bottom-left.
    map: Vec<Tile>,
    /// Cache the spawn weights of each tile.
    #[serde(skip_serializing)]
    weights: WeightedIndex<f64>,
}

impl Index<Position> for World {
    type Output = Tile;
    #[must_use]
    fn index(&self, pos: Position) -> &Self::Output {
        &self.map[self.pos_to_index(pos)]
    }
}

impl World {
    /// Create a new world.
    ///
    /// The `map` must be a row-major set of tiles, of size `width` by `height`.
    /// The first element is the bottom-left corner of the world, i.e. index `(0, 0)`.
    ///
    /// # Errors
    ///
    /// `width` and `height` must be positive integers,
    /// such that `width * height == map.len()`.
    /// There must also be some tiles that can be used to spawn flowers.
    ///
    /// # TODO
    ///
    /// More error checking for bad game maps (e.g. no spawn points)
    pub fn new(width: i32, height: i32, map: Vec<Tile>) -> Result<Self, Error> {
        if width <= 0 || height <= 0 {
            bail!("dims ({}, {}) are not both >= 0", width, height);
        }

        let expected_dim = (width as usize)
            .checked_mul(height as usize)
            .with_context(|| format!("dims ({}, {}) overflow usize", width, height))?;
        if map.len() != expected_dim {
            bail!("dims ({}, {}) != map length ({})", width, height, map.len());
        }

        let weights = map.iter().copied().map(Tile::spawn_weight);
        let weights = WeightedIndex::new(weights).context("couldn't create map weightings")?;

        Ok(Self {
            width,
            height,
            map,
            weights,
        })
    }

    /// Convert a position into an index
    #[must_use]
    fn pos_to_index(&self, pos: Position) -> usize {
        assert!(pos.x >= 0 && pos.x < self.width);
        assert!(pos.y >= 0 && pos.y < self.height);
        pos.x as usize + self.width as usize * pos.y as usize
    }

    /// Convert an index into a position
    #[must_use]
    fn index_to_pos(&self, index: usize) -> Position {
        Position {
            x: (index % self.width as usize) as i32,
            y: (index / self.width as usize) as i32,
        }
    }

    /// Get the tile at the specified position, or `None` if out of bounds.
    #[must_use]
    pub fn get(&self, pos: Position) -> Option<&Tile> {
        if pos.x >= 0 && pos.x < self.width && pos.y >= 0 && pos.y < self.height {
            self.map.get(self.pos_to_index(pos))
        } else {
            None
        }
    }

    /// Get a random position to spawn a new flower in.
    ///
    /// Will not spawn a flower in any of the positions of existing `flowers`.
    pub(super) fn spawn_flowers<'a, R: Rng + ?Sized>(
        &'a self,
        rng: &'a mut R,
        config: &'a Config,
        flowers: &[Flower],
    ) -> impl Iterator<Item = Flower> + 'a {
        let mut updates: Vec<_> = flowers
            .iter()
            .map(|f| (self.pos_to_index(f.position), &0_f64))
            .collect();
        updates.sort_unstable_by_key(|x| x.0);
        let mut dist = self.weights.clone();

        from_fn(move || {
            if rng.gen_bool(config.flower_spawn_chance) {
                // update the weight distribution
                dist.update_weights(updates.as_slice()).ok()?;

                // get the next index (and prepare to fix up the weights for next time)
                let index = dist.sample(rng);
                updates = vec![(index, &0_f64)];

                let position = self.index_to_pos(index);
                let pollen = rng.gen_range(config.flower_initial_pollen.clone());
                Some(Flower::new(position, pollen))
            } else {
                None
            }
        })
    }

    /// List all tile that can be used as spawn points for player hives.
    #[must_use]
    pub fn get_spawn_points(&self) -> Vec<Position> {
        self.map
            .iter()
            .enumerate()
            .filter_map(|(index, tile)| tile.is_spawn_point().then(|| self.index_to_pos(index)))
            .collect()
    }
}

impl Default for World {
    #[rustfmt::skip]
    fn default() -> Self {
        use Tile::{Grass as g, SpawnPoint as S};
        World::new(7, 7, vec![
            g, g, g, g, g, g, g,
            g, S, g, g, g, g, g,
            g, g, g, g, g, g, g,
            g, g, g, g, g, S, g,
            g, g, g, g, g, g, g,
            g, S, g, g, g, g, g,
            g, g, g, g, g, g, g,
        ]).expect("Failed to create default world")
    }
}

/// Intermediary type used to deserialise a [`World`], handling any errors.
#[derive(Deserialize)]
struct WorldDeserializer {
    /// See [`World::width`].
    width: i32,
    /// See [`World::height`].
    height: i32,
    /// See [`World::map`].
    map: Vec<Tile>,
}

impl TryFrom<WorldDeserializer> for World {
    type Error = Error;
    fn try_from(
        WorldDeserializer { width, height, map }: WorldDeserializer,
    ) -> Result<Self, Self::Error> {
        World::new(width, height, map)
    }
}
