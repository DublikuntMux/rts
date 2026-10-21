//! The in-game session bar: what is running, the latest message, and Save and Leave buttons.

use bevy::prelude::*;

use crate::{
    game::AppState,
    menu::{
        Action, DialogHost, View,
        widgets::{BAR, EDGE, INK, INK_MUTED, ShowIf, Tone, bound_text, plain_button},
    },
    session::{MatchKind, SessionAction},
};

/// Height of the in-game session bar in logical pixels, before interface scaling. World input
/// should ignore pointers over it.
pub const SESSION_BAR_HEIGHT: f32 = 56.0;

pub struct SessionBarPlugin;

impl Plugin for SessionBarPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::InGame), spawn_session_bar);
    }
}

fn spawn_session_bar(mut commands: Commands) {
    commands.spawn_scene(session_bar());
}

fn session_bar() -> impl Scene {
    let solo = ShowIf(|view| view.status.kind == MatchKind::Solo);
    let dialog_shown = ShowIf(|view| view.menu.dialog.is_some());
    bsn! {
        #SessionBar
        DespawnOnExit<AppState>(AppState::InGame)
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            height: px(SESSION_BAR_HEIGHT),
            padding: UiRect::axes(px(12), px(4)),
            border: UiRect::bottom(px(1)),
            column_gap: px(8),
            align_items: AlignItems::Center,
        }
        BackgroundColor(BAR)
        BorderColor::all(EDGE)
        Children [
            Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                flex_shrink: 1.0,
                flex_basis: px(0),
                min_width: px(0),
                max_height: percent(100),
                row_gap: px(2),
                overflow: Overflow::clip(),
            }
            Children [
                @bound_text(bar_title, 15.0, INK) TextLayout { linebreak: LineBreak::NoWrap }
                --
                @bound_text(bar_message, 12.0, INK_MUTED)
            ]
            --
            Node { flex_shrink: 0.0, column_gap: px(8) }
            Children [
                @bar_button("Save", SessionAction::SaveGame)
                solo
                --
                @bar_button("Save replay", SessionAction::SaveReplay)
                --
                @bar_button("Leave", SessionAction::Leave)
            ]
            --
            DialogHost
            dialog_shown
            GlobalZIndex(5)
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                top: px(0),
                height: vh(100),
            }
        ]
    }
}

/// A compact bar button. It isn't tab-focusable, so in-game keys never activate it.
fn bar_button(label: &'static str, action: SessionAction) -> impl Scene {
    bsn! {
        @plain_button(label, Action::Game(action), Tone::Secondary)
        Node { padding: UiRect::horizontal(px(10)) }
    }
}

fn bar_title(view: &View) -> String {
    match view.status.kind {
        MatchKind::Playback => "Replay".into(),
        MatchKind::Online => "Online".into(),
        MatchKind::Solo => "Solo".into(),
    }
}

fn bar_message(view: &View) -> String {
    view.status.message.clone()
}
