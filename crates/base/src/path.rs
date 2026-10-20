//! Four-neighbor A* weighted by travel time, with a fully specified search order.

use std::{cmp::Reverse, collections::BinaryHeap};

use crate::{IVec2, Rules, TILE, Terrain};

/// Tile-center route, including both endpoints. Callers validate world positions first.
pub fn find_path(
    terrain: &Terrain,
    rules: &Rules,
    start: IVec2,
    goal: IVec2,
    can_enter: &impl Fn(IVec2) -> bool,
) -> Option<Vec<IVec2>> {
    let traversable = |tile| {
        terrain
            .get(tile)
            .is_some_and(|id| rules.terrain(id).speed_modifier > 0)
            // Approach a full destination and wait outside it. Movement still checks capacity
            // before entering it; intermediate occupied tiles must be routed around.
            && (tile == start || tile == goal || can_enter(tile))
    };
    if !traversable(start) || !traversable(goal) {
        return None;
    }
    let width = terrain.size().x;
    let index = |tile: IVec2| usize::try_from(tile.y * width + tile.x).unwrap();
    let tile = |index: usize| {
        let index = i32::try_from(index).unwrap();
        IVec2::new(index % width, index / width)
    };
    let tile_cost = |tile| {
        rules
            .terrain(terrain.get(tile).unwrap())
            .travel_cost()
            .unwrap()
    };
    let minimum_cost = rules
        .terrain_types()
        .iter()
        .filter_map(crate::TerrainDef::travel_cost)
        .min()
        .unwrap();
    // Even the fastest terrain needs this much time per edge. An unscaled Manhattan
    // heuristic would overestimate routes through terrain faster than normal.
    let heuristic =
        |pos: IVec2| u64::from(pos.x.abs_diff(goal.x) + pos.y.abs_diff(goal.y)) * 2 * minimum_cost;
    let mut costs = vec![u64::MAX; terrain.tiles().len()];
    let mut parents = vec![usize::MAX; costs.len()];
    let mut open = BinaryHeap::new();
    let start_index = index(start);
    let goal_index = index(goal);
    costs[start_index] = 0;
    // Lowest total travel cost, then lowest remaining time, then row-major tile index.
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
            if !traversable(neighbor) {
                continue;
            }
            let next = index(neighbor);
            // A center-to-center edge spends half its distance in each tile. The common
            // TILE / 2 distance factor can be omitted from every path cost.
            let cost = costs[current] + tile_cost(pos) + tile_cost(neighbor);
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
