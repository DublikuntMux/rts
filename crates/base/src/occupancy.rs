//! Derived tile usage. Rebuilt from living units, never cached in simulation state.

use crate::{IVec2, tile_of};

pub struct Occupancy {
    size: IVec2,
    used: Vec<u32>,
}

impl Occupancy {
    pub(crate) fn new(size: IVec2) -> Self {
        Self {
            size,
            used: vec![0; usize::try_from(size.x * size.y).unwrap()],
        }
    }

    fn index(&self, tile: IVec2) -> Option<usize> {
        (tile.cmpge(IVec2::ZERO).all() && tile.cmplt(self.size).all())
            .then(|| usize::try_from(tile.y * self.size.x + tile.x).unwrap())
    }

    pub(crate) fn fits(&self, tile: IVec2, size: u16, capacity: u16) -> bool {
        self.index(tile)
            .is_some_and(|index| self.used[index] + u32::from(size) <= u32::from(capacity))
    }

    pub(crate) fn add(&mut self, pos: IVec2, size: u16) {
        let index = self.index(tile_of(pos)).unwrap();
        self.used[index] += u32::from(size);
    }

    pub(crate) fn remove(&mut self, pos: IVec2, size: u16) {
        let index = self.index(tile_of(pos)).unwrap();
        self.used[index] -= u32::from(size);
    }
}
