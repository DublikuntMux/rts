//! The pause button and in-game pause menu.

use bevy::{
    input::InputSystems, input_focus::InputFocus, picking::hover::Hovered, prelude::*,
    ui_widgets::Button,
};

use crate::{
    game::AppState,
    hud::{PhoneHud, Targeting},
    locale::Localization,
    menu::{
        Action, DialogHost, Menu, MenuActions, View,
        widgets::{INK, INK_MUTED, ShowIf, Tone, bound_text, dialog_with_title, wide_button},
    },
    session::{MatchKind, MatchResult, SessionAction, SessionStatus},
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
            .init_resource::<Localization>()
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
    mut targeting: Option<ResMut<Targeting>>,
    mut phone: Option<ResMut<PhoneHud>>,
    status: Res<SessionStatus>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        if let Some(phone) = phone.as_deref_mut()
            && phone.panel.take().is_some()
        {
            return;
        }
        if let Some(targeting) = targeting.as_deref_mut()
            && *targeting != Targeting::None
        {
            *targeting = Targeting::None;
            return;
        }
        if menu.dialog.take().is_none() && status.result.is_none() {
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

fn spawn_pause_menu(mut commands: Commands, loc: Res<Localization>) {
    commands.spawn_scene(pause_menu(&loc));
}

fn pause_menu(loc: &Localization) -> impl Scene {
    let resumable = ShowIf(|view| view.status.result.is_none());
    let can_leave = resumable.clone();
    let completed = ShowIf(|view| view.status.result.is_some());
    let solo = ShowIf(|view| view.status.can_save_game());
    let restart = ShowIf(|view| view.status.can_restart());
    let dialog_shown = ShowIf(|view| view.menu.dialog.is_some());
    let resume_label = loc.t("pause.resume").to_owned();
    let play_again_label = loc.t("pause.play_again").to_owned();
    let save_game_label = loc.t("pause.save_game").to_owned();
    let save_replay_label = loc.t("pause.save_replay").to_owned();
    let leave_label = loc.t("pause.leave").to_owned();
    let main_menu_label = loc.t("pause.main_menu").to_owned();
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
            @dialog_with_title(bound_text(panel_title, 22.0, INK), bsn_list! {
                @bound_text(pause_message, 14.0, INK_MUTED)
                --
                @bound_text(session_message, 13.0, INK_MUTED)
                --
                @wide_button(resume_label, Action::Resume, Tone::Primary)
                resumable
                --
                @wide_button(play_again_label, Action::Game(SessionAction::Restart), Tone::Primary)
                restart
                --
                @wide_button(save_game_label, Action::Game(SessionAction::SaveGame), Tone::Secondary)
                solo
                --
                @wide_button(save_replay_label, Action::Game(SessionAction::SaveReplay), Tone::Secondary)
                --
                @wide_button(leave_label, Action::Game(SessionAction::Leave), Tone::Danger)
                can_leave
                --
                @wide_button(main_menu_label, Action::Game(SessionAction::Leave), Tone::Secondary)
                completed
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
    let tone = Tone::Secondary;
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
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(1)),
            column_gap: px(5),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
        }
        Outline { width: px(2), offset: px(2), color: Color::NONE }
        on(crate::menu::activate)
        Children [
            Node { width: px(4), height: px(16) }
            BackgroundColor(INK)
            Pickable::IGNORE
            --
            Node { width: px(4), height: px(16) }
            BackgroundColor(INK)
            Pickable::IGNORE
        ]
    }
}

fn pause_message(view: &View) -> String {
    if let Some(result) = view.status.result {
        return view.loc.t(result.detail_key()).to_owned();
    }
    let key = match view.status.kind {
        MatchKind::Online => "pause.online_msg",
        MatchKind::Playback => "pause.replay_msg",
        MatchKind::Solo => "pause.solo_msg",
    };
    view.loc.t(key).to_owned()
}

fn panel_title(view: &View) -> String {
    let key = view
        .status
        .result
        .map_or("pause.title", MatchResult::title_key);
    view.loc.t(key).to_owned()
}

fn session_message(view: &View) -> String {
    view.status.message.clone()
}
