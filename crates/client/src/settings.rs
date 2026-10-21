//! Player settings, stored with the other documents and applied as they change.

use bevy::{
    prelude::*,
    window::{MonitorSelection, PrimaryWindow, WindowMode},
};
use serde::{Deserialize, Serialize};

use crate::{camera::RtsCamera, library::Library};

const KEY: &str = "settings.ron";
/// Interface scales offered in Settings, in percent.
pub const UI_SCALES: [u16; 4] = [75, 100, 125, 150];
/// Screen-edge width that scrolls the camera when edge scrolling is on.
const EDGE_MARGIN: f32 = 8.0;

#[derive(Resource, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Shown to other players in online rooms.
    pub name: String,
    /// One of [`UI_SCALES`].
    pub ui_scale: u16,
    pub edge_scroll: bool,
    pub fullscreen: bool,
    /// The last server this client connected to.
    pub recent_server: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: "Player".into(),
            ui_scale: 100,
            edge_scroll: true,
            fullscreen: false,
            recent_server: None,
        }
    }
}

impl Settings {
    fn from_ron(text: &str) -> Result<Self, String> {
        let mut settings: Self = ron::from_str(text).map_err(|error| error.to_string())?;
        if !UI_SCALES.contains(&settings.ui_scale) {
            settings.ui_scale = 100;
        }
        if rts_net::validate_name(&settings.name).is_err() {
            settings.name = Self::default().name;
        }
        Ok(settings)
    }

    /// Whether this platform can switch to fullscreen.
    pub const fn fullscreen_supported() -> bool {
        !cfg!(any(target_arch = "wasm32", target_os = "android"))
    }
}

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, load_settings).add_systems(
            Update,
            (save_settings, apply_settings).run_if(resource_changed::<Settings>),
        );
    }
}

fn load_settings(mut commands: Commands, library: Res<Library>) {
    let settings = match library.store().read(KEY) {
        Ok(Some(text)) => Settings::from_ron(&text).unwrap_or_else(|error| {
            warn!("ignoring invalid {KEY}: {error}");
            Settings::default()
        }),
        Ok(None) => Settings::default(),
        Err(error) => {
            warn!("cannot read {KEY}: {error}");
            Settings::default()
        }
    };
    commands.insert_resource(settings);
}

fn save_settings(settings: Res<Settings>, library: Res<Library>) {
    if settings.is_added() {
        return;
    }
    let result = ron::to_string(&*settings)
        .map_err(|error| error.to_string())
        .and_then(|text| library.store().write(KEY, &text));
    if let Err(error) = result {
        warn!("cannot save {KEY}: {error}");
    }
}

fn apply_settings(
    settings: Res<Settings>,
    mut scale: ResMut<UiScale>,
    mut cameras: Query<&mut RtsCamera>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
) {
    let wanted = f32::from(settings.ui_scale) / 100.0;
    if (scale.0 - wanted).abs() > f32::EPSILON {
        scale.0 = wanted;
    }
    for mut camera in &mut cameras {
        camera.edge_margin = if settings.edge_scroll {
            EDGE_MARGIN
        } else {
            0.0
        };
    }
    if Settings::fullscreen_supported() {
        let mode = if settings.fullscreen {
            WindowMode::BorderlessFullscreen(MonitorSelection::Current)
        } else {
            WindowMode::Windowed
        };
        if window.mode != mode {
            window.mode = mode;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_settings_round_trip_and_repair_bad_values() {
        let settings = Settings {
            name: "Ada".into(),
            ui_scale: 125,
            edge_scroll: false,
            fullscreen: true,
            recent_server: Some("ws://10.0.0.2:9001".into()),
        };
        let text = ron::to_string(&settings).unwrap();
        assert_eq!(Settings::from_ron(&text).unwrap(), settings);
        let repaired = Settings::from_ron("(name: \" bad \", ui_scale: 333)").unwrap();
        assert_eq!(repaired.name, "Player");
        assert_eq!(repaired.ui_scale, 100);
        assert!(repaired.edge_scroll);
        assert!(Settings::from_ron("(unknown: 1)").is_err());
    }
}
