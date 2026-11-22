//! Saved games and the replay library.
//!
//! Both shelves store recorded matches ([`Recording`]): a replay is watched, and a saved game is a
//! solo match that continues after its recorded turns are played back. Each record has a small
//! metadata document for listing and previews and a data document with the replay itself.

use bevy::prelude::*;
use rts_ai::Difficulty;
use rts_net::{MatchSetup, Recording};
use serde::{Deserialize, Serialize};

use crate::storage::{Store, now_millis};

const META: &str = ".meta.ron";
const DATA: &str = ".data.ron";
/// Recursion bound for metadata documents.
const RON_RECURSION_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shelf {
    Saves,
    Replays,
}

impl Shelf {
    const fn folder(self) -> &'static str {
        match self {
            Self::Saves => "saves",
            Self::Replays => "replays",
        }
    }

    /// Prefix of automatic record names.
    const fn noun(self) -> &'static str {
        match self {
            Self::Saves => "Save",
            Self::Replays => "Replay",
        }
    }
}

/// What the browsers show without loading the recording.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordMeta {
    pub name: String,
    pub map: String,
    pub players: u8,
    /// Simulation ticks recorded.
    pub ticks: u32,
    /// Milliseconds since the Unix epoch.
    pub saved_at: u64,
    /// Rules and map, for the preview. Empty for a player-perspective recording, which never
    /// holds the whole map.
    #[serde(default, skip_serializing_if = "is_empty")]
    pub setup: MatchSetup,
    /// The opponent's difficulty in a saved game. Saves from before difficulty levels
    /// resume at the default level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<Difficulty>,
}

impl RecordMeta {
    fn new(name: String, recording: &Recording, difficulty: Option<Difficulty>) -> Self {
        let (map, setup) = match recording {
            Recording::Match(replay) => (map_name(&replay.setup), replay.setup.clone()),
            Recording::View(replay) => (replay.map.clone(), MatchSetup::default()),
        };
        Self {
            name,
            map,
            players: recording.players(),
            ticks: recording.ticks(),
            saved_at: now_millis(),
            setup,
            difficulty,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub id: String,
    pub meta: RecordMeta,
}

/// Stored saves and replays, newest first.
#[derive(Resource)]
pub struct Library {
    store: Store,
    saves: Vec<Record>,
    replays: Vec<Record>,
}

impl Default for Library {
    fn default() -> Self {
        Self::new(Store::default())
    }
}

impl Library {
    pub fn new(store: Store) -> Self {
        let mut library = Self {
            store,
            saves: Vec::new(),
            replays: Vec::new(),
        };
        for kind in [Shelf::Saves, Shelf::Replays] {
            if let Err(error) = library.refresh(kind) {
                warn!("cannot list {}: {error}", kind.folder());
            }
        }
        library
    }

    pub const fn store(&self) -> &Store {
        &self.store
    }

    pub fn records(&self, kind: Shelf) -> &[Record] {
        match kind {
            Shelf::Saves => &self.saves,
            Shelf::Replays => &self.replays,
        }
    }

    pub fn get(&self, kind: Shelf, id: &str) -> Option<&Record> {
        self.records(kind).iter().find(|record| record.id == id)
    }

    /// Stores `replay` under the next free automatic name, such as "Save 03". Saved games
    /// also keep the opponent's `difficulty`.
    ///
    /// # Errors
    /// Returns an error for an invalid recording or a storage failure.
    pub fn add(
        &mut self,
        kind: Shelf,
        replay: &Recording,
        difficulty: Option<Difficulty>,
    ) -> Result<Record, String> {
        if kind == Shelf::Saves && !matches!(replay, Recording::Match(_)) {
            return Err("Only local matches can be saved.".into());
        }
        let data = replay
            .to_ron()
            .map_err(|error| format!("Cannot save: {error}"))?;
        let record = Record {
            id: self.free_id(kind),
            meta: RecordMeta::new(self.next_name(kind), replay, difficulty),
        };
        let meta = ron::to_string(&record.meta).map_err(|error| format!("Cannot save: {error}"))?;
        let folder = kind.folder();
        // Data first: a listed record always has its data.
        self.store
            .write(&format!("{folder}/{}{DATA}", record.id), &data)?;
        if let Err(error) = self
            .store
            .write(&format!("{folder}/{}{META}", record.id), &meta)
        {
            let _ = self.store.delete(&format!("{folder}/{}{DATA}", record.id));
            return Err(error);
        }
        self.records_mut(kind).insert(0, record.clone());
        Ok(record)
    }

    /// Reads and verifies a stored recording.
    ///
    /// # Errors
    /// Returns an error for a missing, unreadable, or corrupt record.
    pub fn load(&self, kind: Shelf, id: &str) -> Result<Recording, String> {
        let text = self
            .store
            .read(&format!("{}/{id}{DATA}", kind.folder()))?
            .ok_or("This file no longer exists.")?;
        Recording::from_ron(&text)
            .map(|(recording, _)| recording)
            .map_err(|error| format!("Cannot load: {error}"))
    }

    /// # Errors
    /// Returns an error when storage refuses the deletion.
    pub fn delete(&mut self, kind: Shelf, id: &str) -> Result<(), String> {
        let folder = kind.folder();
        self.store.delete(&format!("{folder}/{id}{META}"))?;
        self.store.delete(&format!("{folder}/{id}{DATA}"))?;
        self.records_mut(kind).retain(|record| record.id != id);
        Ok(())
    }

    /// Re-reads one kind's metadata. Unreadable records are skipped.
    ///
    /// # Errors
    /// Returns an error when storage cannot be listed.
    pub fn refresh(&mut self, kind: Shelf) -> Result<(), String> {
        let folder = kind.folder();
        let mut records: Vec<Record> = self
            .store
            .list(folder, META)?
            .into_iter()
            .filter_map(|id| {
                let text = self.store.read(&format!("{folder}/{id}{META}")).ok()??;
                let meta = ron::Options::default()
                    .with_recursion_limit(RON_RECURSION_LIMIT)
                    .from_str(&text)
                    .inspect_err(|error| warn!("skipping {folder}/{id}: {error}"))
                    .ok()?;
                Some(Record { id, meta })
            })
            .collect();
        records.sort_by(|a, b| {
            b.meta
                .saved_at
                .cmp(&a.meta.saved_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        *self.records_mut(kind) = records;
        Ok(())
    }

    const fn records_mut(&mut self, kind: Shelf) -> &mut Vec<Record> {
        match kind {
            Shelf::Saves => &mut self.saves,
            Shelf::Replays => &mut self.replays,
        }
    }

    /// One more than the highest automatic number in use.
    fn next_name(&self, kind: Shelf) -> String {
        let noun = kind.noun();
        let next = self
            .records(kind)
            .iter()
            .filter_map(|record| {
                record
                    .meta
                    .name
                    .strip_prefix(noun)?
                    .trim()
                    .parse::<u32>()
                    .ok()
            })
            .max()
            .unwrap_or(0)
            + 1;
        format!("{noun} {next:02}")
    }

    fn free_id(&self, kind: Shelf) -> String {
        let mut stamp = now_millis();
        loop {
            let id = format!("{stamp:013}");
            if self.get(kind, &id).is_none() {
                return id;
            }
            stamp += 1;
        }
    }
}

/// The map's display name from its setup, or a placeholder.
fn is_empty(setup: &MatchSetup) -> bool {
    *setup == MatchSetup::default()
}

pub fn map_name(setup: &MatchSetup) -> String {
    rts_base::MapData::from_ron(&setup.map).map_or_else(|_| "Unknown map".into(), |map| map.name)
}

/// A tick count as `m:ss` or `h:mm:ss`.
pub fn duration(ticks: u32) -> String {
    let seconds = ticks / rts_base::TICK_RATE;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use rts_base::{PlayerId, PlayerKnowledge};
    use rts_net::{PlayerInput, Replay, Turn, ViewReplay};

    use super::*;
    use crate::storage::tests::temporary_store;

    pub fn replay(turns: u32) -> Recording {
        let setup = MatchSetup {
            rules: include_str!("../../../assets/data/core.rules.ron").into(),
            map: include_str!("../../../assets/maps/proving_grounds.map.ron").into(),
            seed: 0,
            players: 2,
        };
        let mut replay = Replay::new(setup).unwrap();
        let mut sim = replay.simulation().unwrap();
        for tick in 1..=turns {
            let turn = Turn {
                tick,
                inputs: (0..2)
                    .map(|player| PlayerInput {
                        player: PlayerId(player),
                        commands: Vec::new(),
                    })
                    .collect(),
            };
            turn.apply(&mut sim).unwrap();
            replay.push(turn, sim.state_hash()).unwrap();
        }
        Recording::Match(replay)
    }

    fn view_replay(turns: u32) -> Recording {
        let setup = MatchSetup {
            rules: include_str!("../../../assets/data/core.rules.ron").into(),
            map: include_str!("../../../assets/maps/frontier.map.ron").into(),
            seed: 0,
            players: 2,
        };
        let mut sim = setup.simulation().unwrap();
        let mut knowledge = PlayerKnowledge::new(&sim, PlayerId(1));
        let mut replay =
            ViewReplay::new("Frontier".into(), setup.rules, knowledge.intro(&sim)).unwrap();
        replay.push(knowledge.observe(&sim)).unwrap();
        for _ in 0..turns {
            sim.step();
            replay.push(knowledge.observe(&sim)).unwrap();
        }
        Recording::View(replay)
    }

    #[test]
    fn player_perspective_replays_keep_no_map_and_cannot_be_saves() {
        let store = temporary_store("perspective");
        let mut library = Library::new(store.clone());
        let recording = view_replay(4);
        assert!(library.add(Shelf::Saves, &recording, None).is_err());
        let record = library.add(Shelf::Replays, &recording, None).unwrap();
        assert_eq!(record.meta.map, "Frontier");
        assert_eq!(record.meta.ticks, 4);
        assert_eq!(record.meta.setup, MatchSetup::default());
        let meta = store
            .read(&format!("replays/{}.meta.ron", record.id))
            .unwrap()
            .unwrap();
        assert!(!meta.contains("setup") && !meta.contains("tiles"));
        assert_eq!(
            Library::new(store).records(Shelf::Replays),
            std::slice::from_ref(&record)
        );
        assert_eq!(library.load(Shelf::Replays, &record.id).unwrap(), recording);
    }

    #[test]
    fn records_round_trip_number_and_delete() {
        let store = temporary_store("library");
        let mut library = Library::new(store.clone());
        let first = library
            .add(Shelf::Saves, &replay(3), Some(Difficulty::Hard))
            .unwrap();
        let second = library.add(Shelf::Saves, &replay(5), None).unwrap();
        assert_eq!(first.meta.name, "Save 01");
        assert_eq!(second.meta.name, "Save 02");
        assert_eq!(second.meta.map, "Proving Grounds");
        assert_eq!(second.meta.ticks, 5);
        assert_ne!(first.id, second.id);
        assert_eq!(library.records(Shelf::Replays), []);

        let reopened = Library::new(store);
        assert_eq!(
            reopened.records(Shelf::Saves),
            library.records(Shelf::Saves)
        );
        assert_eq!(reopened.records(Shelf::Saves)[0].id, second.id);
        assert_eq!(
            reopened
                .get(Shelf::Saves, &first.id)
                .unwrap()
                .meta
                .difficulty,
            Some(Difficulty::Hard)
        );
        assert_eq!(reopened.load(Shelf::Saves, &first.id).unwrap(), replay(3));

        library.delete(Shelf::Saves, &second.id).unwrap();
        assert_eq!(library.records(Shelf::Saves).len(), 1);
        assert!(library.load(Shelf::Saves, &second.id).is_err());
        assert_eq!(
            library
                .add(Shelf::Saves, &replay(1), None)
                .unwrap()
                .meta
                .name,
            "Save 02"
        );
    }

    #[test]
    fn metadata_without_difficulty_still_lists() {
        let store = temporary_store("legacy");
        let mut library = Library::new(store.clone());
        let record = library.add(Shelf::Saves, &replay(1), None).unwrap();
        let text = store
            .read(&format!("saves/{}.meta.ron", record.id))
            .unwrap()
            .unwrap();
        assert!(!text.contains("difficulty"));
        let reopened = Library::new(store);
        assert_eq!(reopened.records(Shelf::Saves)[0].meta.difficulty, None);
    }

    #[test]
    fn corrupt_records_are_skipped_or_rejected() {
        let store = temporary_store("corrupt");
        let mut library = Library::new(store.clone());
        let record = library.add(Shelf::Replays, &replay(2), None).unwrap();
        store.write("replays/broken.meta.ron", "not ron").unwrap();
        store
            .write(&format!("replays/{}.data.ron", record.id), "(version: 0)")
            .unwrap();
        library.refresh(Shelf::Replays).unwrap();
        assert_eq!(library.records(Shelf::Replays).len(), 1);
        assert!(library.load(Shelf::Replays, &record.id).is_err());
    }

    #[test]
    fn durations_use_minutes_and_hours() {
        assert_eq!(duration(0), "0:00");
        assert_eq!(duration(20 * 75), "1:15");
        assert_eq!(duration(20 * 3_725), "1:02:05");
    }
}
