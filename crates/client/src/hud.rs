//! Command HUD: a bottom bar with the selection count, a control hint, and order buttons.

use bevy::{
    picking::hover::Hovered,
    prelude::*,
    text::LineHeight,
    ui::{InteractionDisabled, Pressed},
    ui_widgets::{Activate, Button, observe},
};

use crate::{
    font::GameFont,
    game::{AppState, Match},
    input::Selection,
};

/// Height of the HUD bar in logical pixels. World input should ignore pointers over it.
pub const HUD_HEIGHT: f32 = 88.0;

/// Smallest touch target side, in logical pixels.
const TOUCH_TARGET: f32 = 48.0;

const BAR: Color = Color::srgba(0.055, 0.094, 0.157, 0.94);
const BAR_EDGE: Color = Color::srgb(0.204, 0.278, 0.373);
const INK: Color = Color::srgb(0.886, 0.914, 0.945);
const INK_MUTED: Color = Color::srgb(0.596, 0.667, 0.753);
const INK_DISABLED: Color = Color::srgb(0.337, 0.404, 0.490);

/// Fill and border colors for one button state.
struct Look {
    fill: Color,
    edge: Color,
    ink: Color,
}

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::InGame), spawn_hud)
            .add_systems(
                Update,
                (
                    update_selection_text.run_if(resource_changed::<Selection>),
                    update_button_availability,
                    update_button_looks,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// The selection count line.
#[derive(Component)]
struct CountText;

/// The control hint line.
#[derive(Component)]
struct HintText;

/// Buttons that act on the current selection and are disabled without one.
#[derive(Component)]
struct NeedsSelection;

/// Visual weight of a HUD button.
#[derive(Component, Clone, Copy)]
enum ButtonStyle {
    /// Filled: the main order.
    Primary,
    /// Outlined: selection housekeeping.
    Secondary,
}

/// Button children recolored with the button state (label text and icons).
#[derive(Component)]
struct ButtonInk;

fn spawn_hud(mut commands: Commands, font: Res<GameFont>, selection: Res<Selection>) {
    let font = font.0.clone();

    commands.spawn((
        Name::new("Command HUD"),
        DespawnOnExit(AppState::InGame),
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            bottom: px(0),
            height: px(HUD_HEIGHT),
            padding: UiRect::axes(px(12), px(10)),
            border: UiRect::top(px(1)),
            column_gap: px(12),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            ..default()
        },
        BackgroundColor(BAR),
        BorderColor::all(BAR_EDGE),
        children![
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    flex_basis: px(0),
                    min_width: px(0),
                    row_gap: px(4),
                    ..default()
                },
                children![
                    (
                        CountText,
                        Text::new(count_label(&selection)),
                        text_font(&font, 17.0),
                        TextColor(INK),
                    ),
                    (
                        HintText,
                        Text::new(hint(&selection)),
                        text_font(&font, 13.0),
                        TextColor(INK_MUTED),
                        LineHeight::RelativeToFont(1.25),
                    ),
                ],
            ),
            (
                Node {
                    flex_shrink: 0.0,
                    column_gap: px(8),
                    ..default()
                },
                children![
                    (
                        hud_button(ButtonStyle::Primary),
                        observe(stop_selected),
                        children![stop_glyph(), button_label(&font, "Stop")],
                    ),
                    (
                        hud_button(ButtonStyle::Secondary),
                        observe(clear_selection),
                        children![button_label(&font, "Clear")],
                    ),
                ],
            ),
        ],
    ));
}

/// A selection order button. [`update_button_looks`] colors it and its [`ButtonInk`] children.
fn hud_button(style: ButtonStyle) -> impl Bundle {
    (
        Button,
        Hovered::default(),
        NeedsSelection,
        style,
        Node {
            min_width: px(64),
            height: px(TOUCH_TARGET),
            padding: UiRect::horizontal(px(12)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
            column_gap: px(6),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
    )
}

fn button_label(font: &Handle<Font>, label: &'static str) -> impl Bundle {
    (ButtonInk, Text::new(label), text_font(font, 15.0))
}

/// A square stop glyph drawn as a node, so it doesn't depend on font coverage.
fn stop_glyph() -> impl Bundle {
    (
        ButtonInk,
        Node {
            width: px(8),
            height: px(8),
            ..default()
        },
        BackgroundColor(INK),
    )
}

fn text_font(font: &Handle<Font>, size: f32) -> TextFont {
    TextFont {
        font: font.clone().into(),
        font_size: FontSize::Px(size),
        ..default()
    }
}

fn count_label(selection: &Selection) -> String {
    match selection.units.len() {
        0 => "No selection".into(),
        count => format!("{count} selected"),
    }
}

/// A short hint for the next useful action. It wraps to two lines at 320 px wide.
fn hint(selection: &Selection) -> &'static str {
    match (selection.touch_mode, selection.units.is_empty()) {
        (false, true) => "Click or drag to select units.",
        (false, false) => "Right-click ground to move.",
        (true, true) => "Tap or drag to select units.",
        (true, false) => "Tap ground to move. Two fingers pan.",
    }
}

const fn look(style: ButtonStyle, hovered: bool, pressed: bool, disabled: bool) -> Look {
    if disabled {
        return Look {
            fill: Color::srgb(0.071, 0.114, 0.180),
            edge: Color::srgb(0.157, 0.212, 0.286),
            ink: INK_DISABLED,
        };
    }
    match (style, pressed, hovered) {
        (ButtonStyle::Primary, true, _) => Look {
            fill: Color::srgb(0.471, 0.592, 0.722),
            edge: Color::srgb(0.682, 0.776, 0.871),
            ink: Color::srgb(0.043, 0.078, 0.133),
        },
        (ButtonStyle::Primary, false, true) => Look {
            fill: Color::srgb(0.239, 0.329, 0.447),
            edge: Color::srgb(0.553, 0.651, 0.765),
            ink: INK,
        },
        (ButtonStyle::Primary, false, false) => Look {
            fill: Color::srgb(0.180, 0.259, 0.361),
            edge: Color::srgb(0.369, 0.467, 0.592),
            ink: INK,
        },
        (ButtonStyle::Secondary, true, _) => Look {
            fill: Color::srgb(0.180, 0.259, 0.361),
            edge: Color::srgb(0.682, 0.776, 0.871),
            ink: INK,
        },
        (ButtonStyle::Secondary, false, true) => Look {
            fill: Color::srgb(0.106, 0.161, 0.235),
            edge: Color::srgb(0.553, 0.651, 0.765),
            ink: INK,
        },
        (ButtonStyle::Secondary, false, false) => Look {
            fill: Color::NONE,
            edge: Color::srgb(0.302, 0.392, 0.506),
            ink: INK_MUTED,
        },
    }
}

fn stop_selected(_: On<Activate>, selection: Res<Selection>, mut game: ResMut<Match>) {
    selection.stop(&mut game);
}

fn clear_selection(_: On<Activate>, mut selection: ResMut<Selection>) {
    selection.units.clear();
}

fn update_selection_text(
    selection: Res<Selection>,
    mut count: Query<&mut Text, (With<CountText>, Without<HintText>)>,
    mut hint_text: Query<&mut Text, (With<HintText>, Without<CountText>)>,
) {
    for mut text in &mut count {
        text.set_if_neq(Text::new(count_label(&selection)));
    }
    for mut text in &mut hint_text {
        text.set_if_neq(Text::new(hint(&selection)));
    }
}

fn update_button_availability(
    mut commands: Commands,
    selection: Res<Selection>,
    game: Res<Match>,
    buttons: Query<(Entity, Has<InteractionDisabled>), With<NeedsSelection>>,
) {
    let disable = selection.units.is_empty() || !game.accepts_orders;
    for (entity, disabled) in &buttons {
        if disable && !disabled {
            commands.entity(entity).insert(InteractionDisabled);
        } else if !disable && disabled {
            commands.entity(entity).remove::<InteractionDisabled>();
        }
    }
}

type ButtonState<'a> = (
    &'a ButtonStyle,
    &'a Hovered,
    Has<Pressed>,
    Has<InteractionDisabled>,
    &'a mut BackgroundColor,
    &'a mut BorderColor,
    &'a Children,
);

type InkColors<'a> = (Option<&'a mut TextColor>, Option<&'a mut BackgroundColor>);

fn update_button_looks(
    mut buttons: Query<ButtonState>,
    mut inks: Query<InkColors, (With<ButtonInk>, Without<ButtonStyle>)>,
) {
    for (&style, hovered, pressed, disabled, mut fill, mut edge, children) in &mut buttons {
        let look = look(style, hovered.get(), pressed, disabled);
        fill.set_if_neq(BackgroundColor(look.fill));
        edge.set_if_neq(BorderColor::all(look.edge));
        for &child in children {
            let Ok((text, icon)) = inks.get_mut(child) else {
                continue;
            };
            if let Some(mut text) = text {
                text.set_if_neq(TextColor(look.ink));
            } else if let Some(mut icon) = icon {
                icon.set_if_neq(BackgroundColor(look.ink));
            }
        }
    }
}
