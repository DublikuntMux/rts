//! Runtime localization: bundled locales, external user files, and server-synced overlays.

use std::collections::BTreeMap;

use bevy::{
    asset::{LoadState, LoadedFolder},
    prelude::*,
};
use rts_assets::LocaleData;

use crate::{library::Library, settings::Settings, storage::Store};

/// Fallback language code when no translation exists for the selected language.
pub const DEFAULT_LANGUAGE: &str = "en";
const LOCALES_FOLDER: &str = "locales";
const LOCALE_SUFFIX: &str = ".lang.ron";

/// Loaded translation catalogs and the active language.
///
/// Lookup checks layers from highest to lowest priority:
/// 1. Server-synced overlay for the active language
/// 2. External disk file (`<data_dir>/locales/*.lang.ron`) for the active language
/// 3. Bundled asset (`assets/locales/*.lang.ron`) for the active language
/// 4. The same three layers for [`DEFAULT_LANGUAGE`] (`"en"`)
/// 5. The raw lookup key itself
#[derive(Resource, Clone, Debug)]
pub struct Localization {
    active: String,
    bundled: BTreeMap<String, LocaleData>,
    external: BTreeMap<String, LocaleData>,
    server: BTreeMap<String, LocaleData>,
    revision: u32,
}

impl Default for Localization {
    fn default() -> Self {
        let mut bundled = BTreeMap::new();
        let english = LocaleData::parse_str(include_str!("../../../assets/locales/en.lang.ron"))
            .expect("bundled English locale is valid");
        bundled.insert(english.code.clone(), english);
        Self {
            active: DEFAULT_LANGUAGE.to_owned(),
            bundled,
            external: BTreeMap::new(),
            server: BTreeMap::new(),
            revision: 0,
        }
    }
}

impl Localization {
    /// Currently selected language code (e.g. `"en"`, `"de"`, `"es"`, `"uk"`).
    #[must_use]
    pub fn active(&self) -> &str {
        &self.active
    }

    /// Monotonically increasing revision bumped whenever the active language or any table changes.
    #[must_use]
    pub const fn revision(&self) -> u32 {
        self.revision
    }

    /// Switches the active language code.
    pub fn set_active(&mut self, code: &str) {
        if self.active != code {
            code.clone_into(&mut self.active);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Registers a bundled locale loaded from `assets/locales/`.
    pub fn register_bundled(&mut self, locale: LocaleData) {
        let changed = self.bundled.get(&locale.code) != Some(&locale);
        self.bundled.insert(locale.code.clone(), locale);
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Registers an external locale loaded from `<data_dir>/locales/`.
    pub fn register_external(&mut self, locale: LocaleData) {
        let changed = self.external.get(&locale.code) != Some(&locale);
        self.external.insert(locale.code.clone(), locale);
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Applies a server-synced locale overlay for the current server connection.
    pub fn apply_server_locale(&mut self, locale: LocaleData) {
        let changed = self.server.get(&locale.code) != Some(&locale);
        self.server.insert(locale.code.clone(), locale);
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Clears all server-synced locale overlays (on disconnect or when starting a local match).
    pub fn clear_server_locales(&mut self) {
        if !self.server.is_empty() {
            self.server.clear();
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Loads external `*.lang.ron` files from `store`'s `locales/` directory.
    pub fn load_external_from_store(&mut self, store: &Store) {
        let names = match store.list(LOCALES_FOLDER, LOCALE_SUFFIX) {
            Ok(mut names) => {
                names.sort();
                names
            }
            Err(error) => {
                warn!("cannot list external locales: {error}");
                return;
            }
        };
        for stem in names {
            let key = format!("{LOCALES_FOLDER}/{stem}{LOCALE_SUFFIX}");
            match store.read(&key) {
                Ok(Some(text)) => match LocaleData::parse_str(&text) {
                    Ok(locale) => self.register_external(locale),
                    Err(error) => warn!("ignoring invalid locale `{key}`: {error}"),
                },
                Ok(None) => {}
                Err(error) => warn!("cannot read locale `{key}`: {error}"),
            }
        }
    }

    /// Available languages as `(code, display_name)`, with English first and the rest sorted.
    #[must_use]
    pub fn languages(&self) -> Vec<(String, String)> {
        let mut names = BTreeMap::<&str, &str>::new();
        for (code, locale) in &self.bundled {
            names.insert(code.as_str(), locale.name.as_str());
        }
        for (code, locale) in &self.external {
            names.insert(code.as_str(), locale.name.as_str());
        }
        for (code, locale) in &self.server {
            names.insert(code.as_str(), locale.name.as_str());
        }
        let mut list: Vec<(String, String)> = names
            .into_iter()
            .map(|(code, name)| (code.to_owned(), name.to_owned()))
            .collect();
        list.sort_by(
            |a, b| match (a.0 == DEFAULT_LANGUAGE, b.0 == DEFAULT_LANGUAGE) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.0.cmp(&b.0),
            },
        );
        list
    }

    /// Looks up a localized string by `key`, falling back to English and then to `key`.
    #[must_use]
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.lookup(&self.active, key)
            .or_else(|| {
                (self.active != DEFAULT_LANGUAGE)
                    .then(|| self.lookup(DEFAULT_LANGUAGE, key))
                    .flatten()
            })
            .unwrap_or(key)
    }

    /// Looks up a localized template and substitutes `{name}` placeholders.
    #[must_use]
    pub fn fmt(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut output = self.t(key).to_owned();
        for (name, value) in args {
            let needle = format!("{{{name}}}");
            if output.contains(&needle) {
                output = output.replace(&needle, value);
            }
        }
        output
    }

    /// Returns a localized display name for a unit rule ID (e.g. `"soldier"` -> `"Rifle squad"`),
    /// falling back to capitalizing the ID when no `unit.<id>` translation exists.
    #[must_use]
    pub fn unit_name(&self, id: &str) -> String {
        let key = format!("unit.{id}");
        let translated = self.t(&key);
        if translated != key {
            return translated.to_owned();
        }
        let mut chars = id.chars();
        chars.next().map_or_else(String::new, |first| {
            first
                .to_uppercase()
                .chain(chars)
                .collect::<String>()
                .replace('_', " ")
        })
    }

    fn lookup(&self, code: &str, key: &str) -> Option<&str> {
        self.server
            .get(code)
            .and_then(|locale| locale.strings.get(key))
            .or_else(|| {
                self.external
                    .get(code)
                    .and_then(|locale| locale.strings.get(key))
            })
            .or_else(|| {
                self.bundled
                    .get(code)
                    .and_then(|locale| locale.strings.get(key))
            })
            .map(String::as_str)
    }
}

pub struct LocalePlugin;

impl Plugin for LocalePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Localization>()
            .add_systems(Startup, (load_bundled_locales, load_external_locales))
            .add_systems(
                Update,
                (
                    collect_bundled_locales.run_if(resource_exists::<PendingLocales>),
                    sync_language_from_settings.run_if(resource_exists::<Settings>),
                ),
            );
    }
}

#[derive(Resource)]
struct PendingLocales(Handle<LoadedFolder>);

fn load_bundled_locales(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(PendingLocales(assets.load_folder(LOCALES_FOLDER)));
}

fn load_external_locales(library: Res<Library>, mut loc: ResMut<Localization>) {
    loc.load_external_from_store(library.store());
}

fn collect_bundled_locales(
    mut commands: Commands,
    pending: Res<PendingLocales>,
    folders: Res<Assets<LoadedFolder>>,
    locales: Res<Assets<LocaleData>>,
    assets: Res<AssetServer>,
    mut loc: ResMut<Localization>,
) {
    if let Some(LoadState::Failed(error)) = assets.get_load_state(pending.0.id()) {
        warn!("cannot load bundled locales folder: {error}");
        commands.remove_resource::<PendingLocales>();
        return;
    }
    let Some(folder) = folders.get(&pending.0) else {
        return;
    };
    let handles: Vec<Handle<LocaleData>> = folder
        .handles
        .iter()
        .filter_map(|handle| handle.clone().try_typed().ok())
        .collect();
    if handles.iter().any(|handle| {
        assets
            .get_load_state(handle)
            .is_some_and(|state| state.is_loading())
    }) {
        return;
    }
    for handle in &handles {
        if let Some(locale) = locales.get(handle) {
            loc.register_bundled(locale.clone());
        }
    }
    commands.remove_resource::<PendingLocales>();
}

fn sync_language_from_settings(settings: Res<Settings>, mut loc: ResMut<Localization>) {
    if loc.active() != settings.language {
        loc.set_active(&settings.language);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::temporary_store;

    #[test]
    fn layers_fall_back_and_server_overlays_can_be_cleared() {
        let mut loc = Localization::default();
        assert_eq!(loc.t("nav.singleplayer"), "Singleplayer");
        assert_eq!(loc.unit_name("soldier"), "Rifle squad");
        assert_eq!(loc.unit_name("heavy_tank"), "Heavy tank");

        let german =
            LocaleData::parse_str(include_str!("../../../assets/locales/de.lang.ron")).unwrap();
        loc.register_bundled(german);
        loc.set_active("de");
        assert_eq!(loc.t("nav.singleplayer"), "Einzelspieler");
        assert_eq!(
            loc.fmt("solo.selected_map", &[("map", "Frontier")]),
            "Ausgewählt: Frontier"
        );

        // External file adds a new language with only a partial key set; missing keys fall back to en.
        let store = temporary_store("locales");
        store
            .write(
                "locales/fr.lang.ron",
                r#"(code: "fr", name: "Français", strings: {"nav.quit": "Quitter"})"#,
            )
            .unwrap();
        loc.load_external_from_store(&store);
        loc.set_active("fr");
        assert_eq!(loc.t("nav.quit"), "Quitter");
        assert_eq!(loc.t("nav.singleplayer"), "Singleplayer");

        // Server overlay overrides active language and reverts cleanly on clear.
        let server_fr = LocaleData::parse_str(
            r#"(code: "fr", name: "Français", strings: {"nav.quit": "Quitter le serveur", "unit.soldier": "Infanterie"})"#,
        )
        .unwrap();
        let rev_before = loc.revision();
        loc.apply_server_locale(server_fr);
        assert_ne!(loc.revision(), rev_before);
        assert_eq!(loc.t("nav.quit"), "Quitter le serveur");
        assert_eq!(loc.unit_name("soldier"), "Infanterie");

        loc.clear_server_locales();
        assert_eq!(loc.t("nav.quit"), "Quitter");
        assert_eq!(loc.unit_name("soldier"), "Rifle squad");

        let codes: Vec<_> = loc.languages().into_iter().map(|(c, _)| c).collect();
        assert_eq!(codes, ["en", "de", "fr"]);
    }
}
