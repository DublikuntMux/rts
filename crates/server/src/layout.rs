//! Server-private random skirmish layouts.
//!
//! A bundled map is public: every client ships with it, so fog of war cannot hide its terrain,
//! deposits, or the enemy start. By default the server therefore generates a fresh two-player
//! skirmish layout for each match from a seed only the server knows. Players learn it only by
//! exploring.

use std::{collections::VecDeque, fmt::Write};

use rand::{RngExt, SeedableRng, rngs::StdRng};
use rts_net::MatchSetup;

/// Name shown in room lists for generated layouts.
pub const RANDOM_MAP_NAME: &str = "Random frontier";

const GRASS: char = '.';
const DIRT: char = ',';
const SAND: char = ':';
const WATER: char = '~';
const ROCK: char = '#';
/// Tiles around each start kept open for the base.
const BASE_RADIUS: i32 = 5;

/// Builds a valid two-player skirmish setup with `rules` from `seed`.
///
/// # Errors
/// Returns an error if the rules cannot host the generated layout, for example because they
/// lack the standard terrain symbols or skirmish units.
pub fn random_setup(rules: &str, seed: u64) -> Result<MatchSetup, String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut last_error = String::new();
    for _ in 0..32 {
        let map = generate(&mut rng);
        let setup = MatchSetup {
            rules: rules.to_owned(),
            map,
            seed,
            players: 2,
        };
        match setup.simulation() {
            Ok(_) => return Ok(setup),
            Err(error) => last_error = error.to_string(),
        }
    }
    Err(format!("cannot generate a random layout: {last_error}"))
}

struct Grid {
    width: i32,
    height: i32,
    tiles: Vec<char>,
}

impl Grid {
    fn get(&self, x: i32, y: i32) -> Option<char> {
        (x >= 0 && y >= 0 && x < self.width && y < self.height)
            .then(|| self.tiles[usize::try_from(y * self.width + x).expect("inside grid")])
    }

    fn set(&mut self, x: i32, y: i32, tile: char) {
        if x >= 0 && y >= 0 && x < self.width && y < self.height {
            self.tiles[usize::try_from(y * self.width + x).expect("inside grid")] = tile;
        }
    }

    fn open(&self, x: i32, y: i32) -> bool {
        self.get(x, y)
            .is_some_and(|tile| tile != WATER && tile != ROCK)
    }

    /// Paints a blob by random walk.
    fn blob(&mut self, rng: &mut StdRng, tile: char, steps: u32) {
        let (mut x, mut y) = (
            rng.random_range(0..self.width),
            rng.random_range(0..self.height),
        );
        for _ in 0..steps {
            for (dx, dy) in [(0, 0), (1, 0), (0, 1)] {
                self.set(x + dx, y + dy, tile);
            }
            x = (x + rng.random_range(-1..=1)).clamp(0, self.width - 1);
            y = (y + rng.random_range(-1..=1)).clamp(0, self.height - 1);
        }
    }

    fn clear_around(&mut self, (cx, cy): (i32, i32), radius: i32) {
        for y in cy - radius..=cy + radius {
            for x in cx - radius..=cx + radius {
                if (x - cx).pow(2) + (y - cy).pow(2) <= radius * radius + radius && !self.open(x, y)
                {
                    self.set(x, y, GRASS);
                }
            }
        }
    }

    /// Open tiles reachable from `from`.
    fn reachable(&self, from: (i32, i32)) -> Vec<bool> {
        let mut seen = vec![false; self.tiles.len()];
        let mut queue = VecDeque::from([from]);
        while let Some((x, y)) = queue.pop_front() {
            if !self.open(x, y) {
                continue;
            }
            let index = usize::try_from(y * self.width + x).expect("inside grid");
            if std::mem::replace(&mut seen[index], true) {
                continue;
            }
            queue.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
        }
        seen
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one pass from terrain to placements reads best in order"
)]
fn generate(rng: &mut StdRng) -> String {
    let width = rng.random_range(36..=46);
    let height = rng.random_range(24..=32);
    let mut grid = Grid {
        width,
        height,
        tiles: vec![GRASS; usize::try_from(width * height).expect("small map")],
    };
    let area = u32::try_from(width * height).expect("small map");
    for _ in 0..rng.random_range(4..=7) {
        grid.blob(rng, DIRT, area / 40);
    }
    for _ in 0..rng.random_range(3..=6) {
        grid.blob(rng, SAND, area / 60);
    }
    for _ in 0..rng.random_range(2..=4) {
        grid.blob(rng, WATER, area / 30);
    }
    for _ in 0..rng.random_range(3..=6) {
        grid.blob(rng, ROCK, area / 70);
    }
    // One start in each half at a random height; either slot may take either side.
    let margin = BASE_RADIUS + 1;
    let west = (
        rng.random_range(margin..=width / 4),
        rng.random_range(margin..height - margin),
    );
    let east = (
        rng.random_range(width - width / 4..width - margin),
        rng.random_range(margin..height - margin),
    );
    let starts = if rng.random_bool(0.5) {
        [west, east]
    } else {
        [east, west]
    };
    for start in starts {
        grid.clear_around(start, BASE_RADIUS);
    }
    // Carve a winding road so the starts always connect.
    let (mut x, mut y) = starts[0];
    while (x, y) != starts[1] {
        grid.set(x, y, DIRT);
        grid.set(x, y + 1, DIRT);
        if x != starts[1].0 && (y == starts[1].1 || rng.random_bool(0.7)) {
            x += (starts[1].0 - x).signum();
        } else {
            y += (starts[1].1 - y).signum();
        }
    }
    let reachable = grid.reachable(starts[0]);
    let mut deposits = Vec::new();
    let mut taken = Vec::new();
    for &(sx, sy) in &starts {
        taken.push((sx, sy));
        for (dx, dy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
            taken.push((sx + dx, sy + dy));
        }
    }
    let mut place = |rng: &mut StdRng, center: (i32, i32), min: i32, max: i32| {
        for _ in 0..200 {
            let x = center.0 + rng.random_range(-max..=max);
            let y = center.1 + rng.random_range(-max..=max);
            let distance = (x - center.0).pow(2) + (y - center.1).pow(2);
            let index = usize::try_from(y * width + x).ok();
            if distance >= min * min
                && distance <= max * max
                && grid.open(x, y)
                && index.is_some_and(|index| reachable.get(index) == Some(&true))
                && !taken.contains(&(x, y))
            {
                taken.push((x, y));
                return Some((x, y));
            }
        }
        None
    };
    for &start in &starts {
        for (kind, min, max, stock) in [
            ("Wood", 3, 4, 1000),
            ("Gold", 3, 4, 800),
            ("Wood", 6, 9, 700),
            ("Gold", 6, 9, 600),
        ] {
            if let Some(pos) = place(rng, start, min, max) {
                deposits.push((kind, pos, stock));
            }
        }
    }
    let middle = (
        i32::midpoint(starts[0].0, starts[1].0),
        i32::midpoint(starts[0].1, starts[1].1),
    );
    for kind in ["Wood", "Gold"] {
        if let Some(pos) = place(rng, middle, 0, 6) {
            deposits.push((kind, pos, 500));
        }
    }

    let mut text = format!("(\n    name: {RANDOM_MAP_NAME:?},\n    players: [\n");
    for (x, y) in starts {
        let _ = writeln!(text, "        (start: ({x}.5, {y}.5)),");
    }
    text.push_str("    ],\n    tiles: [\n");
    for row in grid
        .tiles
        .chunks(usize::try_from(width).expect("small map"))
    {
        let _ = writeln!(text, "        \"{}\",", row.iter().collect::<String>());
    }
    text.push_str("    ],\n    units: [\n");
    for (owner, (x, y)) in starts.into_iter().enumerate() {
        let _ = writeln!(
            text,
            "        (kind: \"headquarters\", owner: {owner}, pos: ({x}.5, {y}.5)),"
        );
        for (dx, dy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
            let _ = writeln!(
                text,
                "        (kind: \"worker\", owner: {owner}, pos: ({}.5, {}.5)),",
                x + dx,
                y + dy
            );
        }
    }
    text.push_str("    ],\n    skirmish: Some((deposits: [\n");
    for (kind, (x, y), stock) in deposits {
        let _ = writeln!(
            text,
            "        (kind: {kind}, pos: ({x}.5, {y}.5), remaining: {stock}),"
        );
    }
    text.push_str("    ])),\n)\n");
    text
}

#[cfg(test)]
mod tests {
    use rts_base::MapData;

    use super::*;

    const RULES: &str = include_str!("../../../assets/data/core.rules.ron");

    #[test]
    fn seeds_produce_valid_varied_layouts() {
        let mut starts = Vec::new();
        for seed in 0..24 {
            let setup = random_setup(RULES, seed).unwrap();
            let sim = setup.simulation().unwrap();
            assert!(sim.is_skirmish());
            assert_eq!(random_setup(RULES, seed).unwrap(), setup, "deterministic");
            let map = MapData::from_ron(&setup.map).unwrap();
            assert_eq!(map.name, RANDOM_MAP_NAME);
            starts.push((map.players[0].start, map.tiles.clone()));
        }
        starts.sort_by_key(|(start, _)| (start.x, start.y));
        starts.dedup_by_key(|(start, _)| *start);
        assert!(starts.len() > 12, "starts vary between layouts");
        let first = random_setup(RULES, 1).unwrap();
        assert_ne!(first.map, random_setup(RULES, 2).unwrap().map);
    }

    #[test]
    fn rules_without_skirmish_content_are_reported() {
        let rules = RULES.replace("headquarters", "hq");
        assert!(random_setup(&rules, 0).is_err());
    }
}
