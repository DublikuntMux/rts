use glam::IVec2;
use serde::Deserialize;

use crate::{
    LoadError,
    coords::{TILE, de_tile_pos, tile_of},
    rules::{Rules, TerrainId},
};

/// Largest supported map side, in tiles.
pub const MAX_MAP_SIDE: i32 = 1024;

/// A map file (`*.map.ron`), as written by the editor.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapData {
    pub name: String,
    /// Player slots. Units refer to players by index into this list.
    pub players: Vec<PlayerSlot>,
    /// Terrain rows from top to bottom, one terrain symbol per tile.
    pub tiles: Vec<String>,
    /// Units present when the match starts.
    #[serde(default)]
    pub units: Vec<UnitPlacement>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerSlot {
    /// Start position in world units (written in tiles).
    #[serde(deserialize_with = "de_tile_pos")]
    pub start: IVec2,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitPlacement {
    /// Unit type ID from the rules.
    pub kind: String,
    /// Index into [`MapData::players`].
    pub owner: u8,
    /// Position in world units (written in tiles).
    #[serde(deserialize_with = "de_tile_pos")]
    pub pos: IVec2,
}

impl MapData {
    /// Parses a map file. Validation against the rules happens in [`crate::Sim::new`].
    ///
    /// # Errors
    ///
    /// Returns an error if the text is not valid RON for this format.
    pub fn from_ron(text: &str) -> Result<Self, LoadError> {
        Ok(ron::from_str(text)?)
    }
}

/// The terrain grid of a running match.
#[derive(Debug, Clone)]
pub struct Terrain {
    /// Size in tiles.
    size: IVec2,
    /// Row-major, top row first.
    tiles: Vec<TerrainId>,
}

impl Terrain {
    pub(crate) fn parse(rows: &[String], rules: &Rules) -> Result<Self, LoadError> {
        let width = rows.first().map_or(0, |row| row.chars().count());
        let size = IVec2::new(
            i32::try_from(width).unwrap_or(i32::MAX),
            i32::try_from(rows.len()).unwrap_or(i32::MAX),
        );
        if size.min_element() < 1 || size.max_element() > MAX_MAP_SIDE {
            return Err(LoadError::invalid(format!(
                "map is {}x{} tiles; each side must be 1..={MAX_MAP_SIDE}",
                size.x, size.y
            )));
        }
        let mut tiles = Vec::with_capacity(width * rows.len());
        for (y, row) in rows.iter().enumerate() {
            let before = tiles.len();
            for (x, symbol) in row.chars().enumerate() {
                let id = rules.terrain_by_symbol(symbol).ok_or_else(|| {
                    LoadError::invalid(format!(
                        "unknown terrain symbol `{symbol}` at tile ({x}, {y})"
                    ))
                })?;
                tiles.push(id);
            }
            if tiles.len() - before != width {
                return Err(LoadError::invalid(format!(
                    "row {y} has {} tiles; expected {width}",
                    tiles.len() - before
                )));
            }
        }
        Ok(Self { size, tiles })
    }

    /// Size in tiles.
    #[must_use]
    pub const fn size(&self) -> IVec2 {
        self.size
    }

    /// Size in world units.
    #[must_use]
    pub fn world_size(&self) -> IVec2 {
        self.size * TILE
    }

    /// Terrain IDs row by row, top row first.
    #[must_use]
    pub fn tiles(&self) -> &[TerrainId] {
        &self.tiles
    }

    /// Terrain at a tile, or `None` outside the map.
    #[must_use]
    pub fn get(&self, tile: IVec2) -> Option<TerrainId> {
        if tile.cmplt(IVec2::ZERO).any() || tile.cmpge(self.size).any() {
            return None;
        }
        let index = usize::try_from(tile.y * self.size.x + tile.x).ok()?;
        self.tiles.get(index).copied()
    }

    /// Whether ground units may stand at a world position.
    #[must_use]
    pub fn is_walkable(&self, pos: IVec2, rules: &Rules) -> bool {
        self.get(tile_of(pos))
            .is_some_and(|id| rules.terrain(id).walkable)
    }
}
