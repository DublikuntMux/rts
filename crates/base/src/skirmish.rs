//! Match objectives for maps with headquarters.

use std::hash::{Hash, Hasher};

use crate::{LoadError, PlayerId, Sim, SkirmishMap, UnitId, economy::Economy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchOutcome {
    Winner(PlayerId),
    Draw,
}

impl Hash for MatchOutcome {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Winner(player) => {
                0_u8.hash(state);
                player.hash(state);
            }
            Self::Draw => 1_u8.hash(state),
        }
    }
}

#[derive(Debug, Clone, Hash)]
pub struct Skirmish {
    headquarters: Vec<UnitId>,
    outcome: Option<MatchOutcome>,
    pub(super) economy: Economy,
}

impl Sim {
    pub(crate) fn setup_skirmish(&mut self, map: &SkirmishMap) -> Result<(), LoadError> {
        if self.players.len() != 2 {
            return Err(LoadError::invalid("skirmish requires exactly two players"));
        }
        let kind = self
            .rules
            .unit_kind("headquarters")
            .filter(|&kind| {
                let unit = self.rules.unit(kind);
                unit.building && unit.size == self.rules.tile_capacity()
            })
            .ok_or_else(|| LoadError::invalid("skirmish requires a headquarters building type"))?;
        let mut headquarters = vec![None; self.players.len()];
        for (id, unit) in self.units().filter(|(_, unit)| unit.kind == kind) {
            let slot = &mut headquarters[usize::from(unit.owner.0)];
            if slot.replace(id).is_some() {
                return Err(LoadError::invalid(
                    "each skirmish player needs one headquarters",
                ));
            }
        }
        let headquarters = headquarters
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| LoadError::invalid("each skirmish player needs one headquarters"))?;
        self.skirmish = Some(Skirmish {
            headquarters,
            outcome: None,
            economy: Economy::new(self, map)?,
        });
        Ok(())
    }

    /// Whether the map has a skirmish objective.
    #[must_use]
    pub const fn is_skirmish(&self) -> bool {
        self.skirmish.is_some()
    }

    /// The owner's living headquarters, if present.
    #[must_use]
    pub fn headquarters(&self, owner: PlayerId) -> Option<UnitId> {
        self.skirmish
            .as_ref()?
            .headquarters
            .get(usize::from(owner.0))
            .copied()
            .filter(|id| self.units.contains_key(id))
    }

    /// The result of a completed skirmish.
    #[must_use]
    pub fn outcome(&self) -> Option<MatchOutcome> {
        self.skirmish.as_ref()?.outcome
    }

    pub(crate) fn update_outcome(&mut self) {
        let Some(skirmish) = &mut self.skirmish else {
            return;
        };
        skirmish.outcome = match (
            self.units.contains_key(&skirmish.headquarters[0]),
            self.units.contains_key(&skirmish.headquarters[1]),
        ) {
            (true, true) => None,
            (true, false) => Some(MatchOutcome::Winner(PlayerId(0))),
            (false, true) => Some(MatchOutcome::Winner(PlayerId(1))),
            (false, false) => Some(MatchOutcome::Draw),
        };
        if skirmish.outcome.is_some() {
            self.commands.clear();
        }
    }
}
