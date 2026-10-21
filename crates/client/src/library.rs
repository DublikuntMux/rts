//! Saved games and the replay library.
//!
//! Both shelves store recorded matches ([`Replay`]): a replay is watched, and a saved game is a
//! solo match that continues after its recorded turns are played back. Each record has a small
//! metadata document for listing and previews and a data document with the replay itself.

use bevy::prelude::*;
use rts_base::Sim;
use rts_net::{MatchSetup, Replay};
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
    /// Rules and map, for the preview.
    pub setup: MatchSetup,
}

impl RecordMeta {
    fn new(name: String, replay: &Replay) -> Self {
        Self {
            name,
            map: map_name(&replay.setup),
            players: replay.setup.players,
            ticks: u32::try_from(replay.turns.len()).unwrap_or(u32::MAX),
            saved_at: now_millis(),
            setup: replay.setup.clone(),
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

    /// Stores `replay` under the next free automatic name, such as "Save 03".
    ///
    /// # Errors
    /// Returns an error for an invalid recording or a storage failure.
    pub fn add(&mut self, kind: Shelf, replay: &Replay) -> Result<Record, String> {
        let data = replay
            .to_ron()
            .map_err(|error| format!("Cannot save: {error}"))?;
        let record = Record {
            id: self.free_id(kind),
            meta: RecordMeta::new(self.next_name(kind), replay),
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

    /// Reads and verifies a stored recording. Also returns the simulation after its last turn.
    ///
    /// # Errors
    /// Returns an error for a missing, unreadable, or corrupt record.
    pub fn load(&self, kind: Shelf, id: &str) -> Result<(Replay, Sim), String> {
        let text = self
            .store
            .read(&format!("{}/{id}{DATA}", kind.folder()))?
            .ok_or("This file no longer exists.")?;
        Replay::from_ron_with_sim(&text).map_err(|error| format!("Cannot load: {error}"))
    }

    /// Reads a stored recording's text without verifying it, for export.
    ///
    /// # Errors
    /// Returns an error for a missing or unreadable record.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn load_text(&self, kind: Shelf, id: &str) -> Result<String, String> {
        self.store
            .read(&format!("{}/{id}{DATA}", kind.folder()))?
            .ok_or_else(|| "This file no longer exists.".into())
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use rts_base::PlayerId;
    use rts_net::{PlayerInput, Turn};

    use super::*;
    use crate::storage::tests::temporary_store;

    pub fn replay(turns: u32) -> Replay {
        let setup = MatchSetup {
            rules: include_str!("../../../assets/data/core.rules.ron").into(),
            map: include_str!("../../../assets/maps/proving_grounds.map.ron").into(),
            seed: 0,
            players: 2,
        };
        let mut sim = setup.simulation().unwrap();
        let mut replay = Replay::new(setup).unwrap();
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
        replay
    }

    #[test]
    fn records_round_trip_number_and_delete() {
        let store = temporary_store("library");
        let mut library = Library::new(store.clone());
        let first = library.add(Shelf::Saves, &replay(3)).unwrap();
        let second = library.add(Shelf::Saves, &replay(5)).unwrap();
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
        let (loaded, sim) = reopened.load(Shelf::Saves, &first.id).unwrap();
        assert_eq!(loaded, replay(3));
        assert_eq!(sim.tick(), 3);

        library.delete(Shelf::Saves, &second.id).unwrap();
        assert_eq!(library.records(Shelf::Saves).len(), 1);
        assert!(library.load(Shelf::Saves, &second.id).is_err());
        assert_eq!(
            library.add(Shelf::Saves, &replay(1)).unwrap().meta.name,
            "Save 02"
        );
    }

    #[test]
    fn corrupt_records_are_skipped_or_rejected() {
        let store = temporary_store("corrupt");
        let mut library = Library::new(store.clone());
        let record = library.add(Shelf::Replays, &replay(2)).unwrap();
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
