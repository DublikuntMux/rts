//! The pause button and in-game pause menu.

use bevy::{
    input::InputSystems, input_focus::InputFocus, picking::hover::Hovered, prelude::*,
    ui_widgets::Button,
};

use crate::{
    game::AppState,
    hud::{MoveTargeting, PhoneHud},
    menu::{
        Action, DialogHost, Menu, MenuActions, View,
        widgets::{INK, INK_MUTED, ShowIf, Tone, bound_text, dialog, wide_button},
    },
    session::{MatchKind, SessionAction},
};

/// Logical pixel bounds before interface scaling. Only this rectangle excludes world input.
pub const PAUSE_BUTTON_BOUNDS: Rect = Rect::new(12.0, 12.0, 60.0, 60.0);

#[derive(Resource, Default)]
pub struct PauseMenu {
    pub open: bool,
}

/// Dialogs also capture world input, including when opened independently of the pause menu.
pub fn world_input_blocked(pause: Option<&PauseMenu>, menu: Option<&Menu>) -> bool {
    pause.is_some_and(|pause| pause.open) || menu.is_some_and(|menu| menu.dialog.is_some())
}

pub struct PauseMenuPlugin;

impl Plugin for PauseMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PauseMenu>()
            .add_systems(OnEnter(AppState::InGame), (reset_pause, spawn_pause_menu))
            .add_systems(OnExit(AppState::InGame), reset_pause)
            .add_systems(
                PreUpdate,
                pause_on_escape
                    .after(InputSystems)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(
                Update,
                show_pause_menu
                    .after(MenuActions)
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn reset_pause(mut pause: ResMut<PauseMenu>) {
    pause.open = false;
}

fn pause_on_escape(
    keys: Res<ButtonInput<KeyCode>>,
    mut pause: ResMut<PauseMenu>,
    mut menu: ResMut<Menu>,
    mut targeting: Option<ResMut<MoveTargeting>>,
    mut phone: Option<ResMut<PhoneHud>>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        if let Some(phone) = phone.as_deref_mut()
            && phone.panel.take().is_some()
        {
            return;
        }
        if let Some(targeting) = targeting.as_deref_mut()
            && targeting.0
        {
            targeting.0 = false;
            return;
        }
        if menu.dialog.take().is_none() {
            pause.open = !pause.open;
        }
    }
}

#[derive(Component, Clone, Default)]
struct PausePanel;

fn show_pause_menu(
    pause: Res<PauseMenu>,
    mut focus: ResMut<InputFocus>,
    mut panels: Query<&mut Node, With<PausePanel>>,
) {
    if pause.is_changed() && !pause.open {
        focus.clear();
    }
    for mut node in &mut panels {
        node.display = if pause.open {
            Display::Flex
        } else {
            Display::None
        };
    }
}

fn spawn_pause_menu(mut commands: Commands) {
    commands.spawn_scene(pause_menu());
}

fn pause_menu() -> impl Scene {
    let solo = ShowIf(|view| view.status.kind == MatchKind::Solo);
    let dialog_shown = ShowIf(|view| view.menu.dialog.is_some());
    bsn! {
        DespawnOnExit<AppState>(AppState::InGame)
        Pickable::IGNORE
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
        }
        Children [
            @pause_button()
            --
            @dialog("Pause menu", bsn_list! {
                @bound_text(pause_message, 14.0, INK_MUTED)
                --
                @bound_text(session_message, 13.0, INK_MUTED)
                --
                @wide_button("Resume", Action::Resume, Tone::Primary)
                --
                @wide_button("Save game", Action::Game(SessionAction::SaveGame), Tone::Secondary)
                solo
                --
                @wide_button("Save replay", Action::Game(SessionAction::SaveReplay), Tone::Secondary)
                --
                @wide_button("Leave", Action::Game(SessionAction::Leave), Tone::Danger)
            })
            PausePanel
            GlobalZIndex(10)
            Node { display: Display::None }
            --
            DialogHost
            dialog_shown
            GlobalZIndex(11)
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                top: px(0),
                bottom: px(0),
            }
        ]
    }
}

fn pause_button() -> impl Scene {
    let action = Action::TogglePause;
    let tone = Tone::Item;
    bsn! {
        Name::new("Pause")
        action
        tone
        Button
        Hovered
        GlobalZIndex(2)
        Node {
            position_type: PositionType::Absolute,
            left: px(PAUSE_BUTTON_BOUNDS.min.x),
            top: px(PAUSE_BUTTON_BOUNDS.min.y),
            width: px(PAUSE_BUTTON_BOUNDS.width()),
            height: px(PAUSE_BUTTON_BOUNDS.height()),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(2)),
            column_gap: px(4),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
        }
        Outline { width: px(2), offset: px(2), color: Color::NONE }
        on(crate::menu::activate)
        Children [
            Node { width: px(3), height: px(14) }
            BackgroundColor(INK)
            Pickable::IGNORE
            --
            Node { width: px(3), height: px(14) }
            BackgroundColor(INK)
            Pickable::IGNORE
        ]
    }
}

fn pause_message(view: &View) -> String {
    match view.status.kind {
        MatchKind::Online => "Online match continues while this menu is open.".into(),
        MatchKind::Playback => "Replay paused.".into(),
        MatchKind::Solo => "Game paused.".into(),
    }
}

fn session_message(view: &View) -> String {
    view.status.message.clone()
}
