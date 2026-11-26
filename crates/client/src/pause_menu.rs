//! The pause button and in-game pause menu.

use bevy::{
    input::InputSystems,
    input_focus::InputFocus,
    picking::hover::Hovered,
    prelude::*,
    ui::Pressed,
    ui_widgets::{Activate, Button, observe},
    window::PrimaryWindow,
};

use crate::{
    font::GameFont,
    game::{AppState, Match},
    hud::{HudLayout, PhoneHud, Targeting},
    locale::Localization,
    menu::{
        Action, DialogHost, Menu, MenuActions, View,
        widgets::{
            INK, INK_MUTED, ShowIf, Tone, bound_text, button_look, dialog_with_title, wide_button,
        },
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
            .add_systems(
                OnEnter(AppState::InGame),
                (reset_pause, spawn_pause_menu, spawn_replay_fog),
            )
            .add_systems(OnExit(AppState::InGame), reset_pause)
            .add_systems(
                PreUpdate,
                pause_on_escape
                    .after(InputSystems)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(
                Update,
                (show_pause_menu, update_replay_fog)
                    .after(MenuActions)
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

#[derive(Component)]
struct ReplayFogButton;

#[derive(Component)]
struct ReplayFogLabel;

fn spawn_replay_fog(mut commands: Commands, game: Option<Res<Match>>, font: Option<Res<GameFont>>) {
    let Some(font) = font.filter(|_| game.as_deref().is_some_and(Match::can_toggle_replay_fog))
    else {
        return;
    };
    commands.spawn((
        Name::new("Replay fog"),
        DespawnOnExit(AppState::InGame),
        ReplayFogButton,
        Button,
        Hovered::default(),
        GlobalZIndex(11),
        Node {
            position_type: PositionType::Absolute,
            top: px(12),
            right: px(12),
            min_height: px(48),
            padding: UiRect::horizontal(px(12)),
            border: UiRect::all(px(2)),
            display: Display::None,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor::default(),
        BorderColor::default(),
        observe(toggle_replay_fog),
        children![(
            ReplayFogLabel,
            Pickable::IGNORE,
            Text::default(),
            crate::hud::text_font(&font.0, 13.0),
            TextColor(INK),
        )],
    ));
}

fn toggle_replay_fog(
    _: On<Activate>,
    mut game: ResMut<Match>,
    menu: Res<Menu>,
    phone: Option<Res<PhoneHud>>,
) {
    if menu.dialog.is_none() && phone.as_deref().is_none_or(|phone| phone.panel.is_none()) {
        game.toggle_replay_fog();
    }
}

type FogButtonStyle<'a> = (
    &'a mut Node,
    &'a Hovered,
    Has<Pressed>,
    &'a mut BackgroundColor,
    &'a mut BorderColor,
);

fn update_replay_fog(
    game: Option<Res<Match>>,
    window: Query<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    menu: Res<Menu>,
    phone: Option<Res<PhoneHud>>,
    loc: Res<Localization>,
    mut buttons: Query<FogButtonStyle, With<ReplayFogButton>>,
    mut labels: Query<(&mut Text, &mut TextColor), With<ReplayFogLabel>>,
) {
    let (Some(game), Ok(window)) = (game, window.single()) else {
        return;
    };
    let hud = HudLayout::new(window.size() / scale.0, false, None);
    let visible =
        menu.dialog.is_none() && phone.as_deref().is_none_or(|phone| phone.panel.is_none());
    let key = if game.view.intro().fog_disabled {
        "replay.show_fog"
    } else {
        "replay.hide_fog"
    };
    for (mut node, hover, pressed, mut fill, mut border) in &mut buttons {
        node.top = px(if hud.phone {
            hud.minimap.max.y + 8.0
        } else {
            12.0
        });
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        let look = button_look(Tone::Secondary, false, hover.get(), pressed, false);
        fill.set_if_neq(BackgroundColor(look.fill));
        border.set_if_neq(look.border);
        for (mut label, mut ink) in &mut labels {
            label.set_if_neq(Text::new(loc.t(key)));
            ink.set_if_neq(TextColor(look.ink));
        }
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
    let restart = ShowIf(|view| view.status.can_restart() && view.status.result.is_none());
    let play_again = ShowIf(|view| view.status.can_restart() && view.status.result.is_some());
    let dialog_shown = ShowIf(|view| view.menu.dialog.is_some());
    let resume_label = loc.t("pause.resume").to_owned();
    let restart_label = loc.t("pause.restart").to_owned();
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
                @wide_button(restart_label, Action::Game(SessionAction::Restart), Tone::Secondary)
                restart
                --
                @wide_button(play_again_label, Action::Game(SessionAction::Restart), Tone::Primary)
                play_again
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
