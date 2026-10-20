//! Four-neighbor A* with a fully specified search order.

use std::{cmp::Reverse, collections::BinaryHeap};

use crate::{IVec2, Rules, TILE, Terrain};

/// Tile-center route, including both endpoints. Callers validate world positions first.
pub fn find_path(
    terrain: &Terrain,
    rules: &Rules,
    start: IVec2,
    goal: IVec2,
) -> Option<Vec<IVec2>> {
    let walkable = |tile| {
        terrain
            .get(tile)
            .is_some_and(|id| rules.terrain(id).walkable)
    };
    if !walkable(start) || !walkable(goal) {
        return None;
    }
    let width = terrain.size().x;
    let index = |tile: IVec2| usize::try_from(tile.y * width + tile.x).unwrap();
    let tile = |index: usize| {
        let index = i32::try_from(index).unwrap();
        IVec2::new(index % width, index / width)
    };
    let heuristic = |pos: IVec2| pos.x.abs_diff(goal.x) + pos.y.abs_diff(goal.y);
    let mut costs = vec![u32::MAX; terrain.tiles().len()];
    let mut parents = vec![usize::MAX; costs.len()];
    let mut open = BinaryHeap::new();
    let start_index = index(start);
    let goal_index = index(goal);
    costs[start_index] = 0;
    // Lowest total cost, then lowest remaining distance, then row-major tile index.
    open.push(Reverse((heuristic(start), heuristic(start), start_index)));
    while let Some(Reverse((estimate, remaining, current))) = open.pop() {
        if estimate != costs[current] + remaining {
            continue;
        }
        if current == goal_index {
            let mut route = vec![goal];
            let mut current = current;
            while current != start_index {
                current = parents[current];
                route.push(tile(current));
            }
            route.reverse();
            return Some(
                route
                    .into_iter()
                    .map(|tile| tile * TILE + IVec2::splat(TILE / 2))
                    .collect(),
            );
        }
        let pos = tile(current);
        for offset in [IVec2::NEG_Y, IVec2::NEG_X, IVec2::X, IVec2::Y] {
            let neighbor = pos + offset;
            if !walkable(neighbor) {
                continue;
            }
            let next = index(neighbor);
            let cost = costs[current] + 1;
            if cost < costs[next] {
                costs[next] = cost;
                parents[next] = current;
                let remaining = heuristic(neighbor);
                open.push(Reverse((cost + remaining, remaining, next)));
            }
        }
    }
    None
}
