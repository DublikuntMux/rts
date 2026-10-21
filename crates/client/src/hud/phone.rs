//! Phone controls float over the battlefield; larger panels open only on request.

use bevy::{
    picking::hover::Hovered,
    prelude::*,
    ui_widgets::{Activate, Button, observe},
    window::PrimaryWindow,
};

use super::*;
use crate::menu::widgets::{FIELD_EDGE, PANEL, SCRIM};

#[derive(Component)]
pub(super) struct DesktopOnly;
#[derive(Component)]
pub(super) struct SelectionButton;
#[derive(Component)]
pub(super) struct SelectionCount;
#[derive(Component)]
pub(super) struct Backdrop;
#[derive(Component)]
pub(super) struct PanelTitle;
#[derive(Component)]
pub(super) struct TargetHint;

pub(super) fn reset(mut phone: ResMut<PhoneHud>) {
    phone.panel = None;
}

pub(super) fn spawn_selection_button(parent: &mut ChildSpawnerCommands, font: &Handle<Font>) {
    parent
        .spawn((
            Name::new("Selected units"),
            SelectionButton,
            Button,
            Hovered::default(),
            Node {
                width: px(48),
                height: px(48),
                padding: UiRect::all(px(6)),
                border: UiRect::all(px(1)),
                display: Display::None,
                ..default()
            },
            ImageNode::default(),
            BackgroundColor(FIELD),
            BorderColor::all(FIELD_EDGE),
            observe(open_details),
        ))
        .with_children(|button| {
            button.spawn((
                SelectionCount,
                Pickable::IGNORE,
                Text::new(""),
                text_font(font, 11.0),
                TextColor(INK),
                BackgroundColor(RAIL),
                Node {
                    position_type: PositionType::Absolute,
                    bottom: px(1),
                    right: px(3),
                    padding: UiRect::horizontal(px(2)),
                    ..default()
                },
            ));
        });
}

pub(super) fn spawn_overlays(parent: &mut ChildSpawnerCommands, font: &Handle<Font>) {
    parent
        .spawn((
            Backdrop,
            Name::new("Phone HUD panel"),
            GlobalZIndex(3),
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: percent(100),
                height: percent(100),
                display: Display::None,
                ..default()
            },
            BackgroundColor(SCRIM),
        ))
        .with_children(|overlay| {
            overlay.spawn((
                Pickable::IGNORE,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    right: px(0),
                    top: px(0),
                    height: px(64),
                    ..default()
                },
                BackgroundColor(RAIL),
            ));
            overlay.spawn((
                PanelTitle,
                Pickable::IGNORE,
                Text::new(""),
                text_font(font, 18.0),
                TextColor(INK),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(16),
                    top: px(24),
                    ..default()
                },
            ));
            overlay
                .spawn((
                    Button,
                    Hovered::default(),
                    Name::new("Close HUD panel"),
                    GlobalZIndex(5),
                    Node {
                        position_type: PositionType::Absolute,
                        right: px(8),
                        top: px(8),
                        width: px(48),
                        height: px(48),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(px(1)),
                        ..default()
                    },
                    BackgroundColor(RAIL),
                    BorderColor::all(FIELD_EDGE),
                    observe(close_panel),
                ))
                .with_children(|button| {
                    button.spawn((
                        Pickable::IGNORE,
                        Text::new("Close"),
                        text_font(font, 12.0),
                        TextColor(INK),
                    ));
                });
        });
    parent.spawn((
        TargetHint,
        Pickable::IGNORE,
        Text::new("Choose a destination"),
        text_font(font, 13.0),
        TextColor(SIGNAL),
        BackgroundColor(BAR),
        Node {
            position_type: PositionType::Absolute,
            left: px(8),
            bottom: px(8),
            padding: UiRect::axes(px(8), px(5)),
            display: Display::None,
            ..default()
        },
    ));
}

fn close_panel(_: On<Activate>, mut phone: ResMut<PhoneHud>) {
    phone.panel = None;
}

fn open_details(
    _: On<Activate>,
    window: Single<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    selection: Res<Selection>,
    pause: Res<PauseMenu>,
    mut phone: ResMut<PhoneHud>,
) {
    if !pause.open && phone_layout(window.size() / scale.0) && !selection.units.is_empty() {
        phone.panel = Some(PhonePanel::Selection);
    }
}

type ConsoleNode<'a> = (
    &'a mut Node,
    Has<Console>,
    Has<SelectionPanel>,
    Has<CommandCard>,
    Option<&'a CommandSlot>,
    Has<SelectionButton>,
    Has<DesktopOnly>,
    Has<Backdrop>,
    Has<TargetHint>,
    Has<Roster>,
);
type Surface<'a> = (
    &'a mut BackgroundColor,
    Option<&'a mut BorderColor>,
    Has<Console>,
    Has<CommandCard>,
    Has<SelectionPanel>,
);

pub(super) fn resize_hud(
    window: Single<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    selection: Res<Selection>,
    mut phone: ResMut<PhoneHud>,
    mut focus: Option<ResMut<bevy::input_focus::InputFocus>>,
    targeting: Res<MoveTargeting>,
    mut nodes: Query<ConsoleNode>,
) {
    let size = window.size() / scale.0;
    if phone.panel.is_some()
        && (!phone_layout(size)
            || selection.units.is_empty() && phone.panel == Some(PhonePanel::Selection))
    {
        phone.panel = None;
    }
    if phone.is_changed()
        && phone.panel.is_none()
        && let Some(focus) = focus.as_deref_mut()
    {
        focus.clear();
    }
    let layout = HudLayout::new(size, !selection.units.is_empty(), phone.panel);
    for (
        mut node,
        root,
        details,
        card,
        slot,
        selection_button,
        hotkey,
        backdrop,
        target_hint,
        roster,
    ) in &mut nodes
    {
        if root {
            node.height = if layout.phone {
                percent(100)
            } else {
                px(HUD_HEIGHT)
            };
            node.border = if layout.phone {
                UiRect::ZERO
            } else {
                UiRect::top(px(1))
            };
        }
        if details {
            place_details(&mut node, &layout, phone.panel);
        }
        if card {
            place_commands(&mut node, &layout);
        }
        if let Some(slot) = slot {
            place_slot(&mut node, &layout, slot.0);
        }
        if selection_button {
            node.display = if layout.phone {
                Display::Flex
            } else {
                Display::None
            };
            node.grid_column = GridPlacement::start(if layout.landscape { 1 } else { 2 });
            node.grid_row = GridPlacement::start(1);
        }
        if roster {
            place_roster(&mut node, &layout);
        }
        if hotkey {
            node.display = if layout.phone {
                Display::None
            } else {
                Display::Flex
            };
        }
        if backdrop {
            node.display = if layout.phone && phone.panel.is_some() {
                Display::Flex
            } else {
                Display::None
            };
        }
        if target_hint {
            node.display = if layout.phone && targeting.0 && phone.panel.is_none() {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
}

pub(super) fn update_surfaces(
    window: Single<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    mut surfaces: Query<Surface>,
) {
    let layout = HudLayout::new(window.size() / scale.0, false, None);
    for (mut fill, edge, root, card, details) in &mut surfaces {
        if root {
            fill.set_if_neq(BackgroundColor(if layout.phone {
                Color::NONE
            } else {
                BAR
            }));
            if let Some(mut edge) = edge {
                edge.set_if_neq(BorderColor::all(if layout.phone {
                    Color::NONE
                } else {
                    EDGE
                }));
            }
        } else if card {
            fill.set_if_neq(BackgroundColor(if layout.phone {
                Color::NONE
            } else {
                RAIL
            }));
        } else if details {
            fill.set_if_neq(BackgroundColor(if layout.phone {
                PANEL
            } else {
                Color::NONE
            }));
        }
    }
}

fn place_slot(node: &mut Node, layout: &HudLayout, index: usize) {
    node.display = if !layout.phone || matches!(index, 0 | 1 | 8) {
        Display::Flex
    } else {
        Display::None
    };
    let (col, row) = if layout.landscape {
        match index {
            0 => (4, 1),
            1 => (3, 1),
            _ => (2, 1),
        }
    } else {
        match index {
            0 => (2, 2),
            1 => (1, 2),
            _ => (1, 1),
        }
    };
    node.grid_column = if layout.phone {
        GridPlacement::start(col)
    } else {
        default()
    };
    node.grid_row = if layout.phone {
        GridPlacement::start(row)
    } else {
        default()
    };
}

fn place_roster(node: &mut Node, layout: &HudLayout) {
    node.flex_wrap = if layout.phone {
        FlexWrap::Wrap
    } else {
        FlexWrap::NoWrap
    };
    node.row_gap = px(4);
    node.height = if layout.phone { auto() } else { px(48) };
    node.min_height = px(48);
    node.max_height = if layout.phone { auto() } else { px(48) };
    node.overflow = if layout.phone {
        Overflow::visible()
    } else {
        Overflow::clip()
    };
}

fn place_details(node: &mut Node, layout: &HudLayout, panel: Option<PhonePanel>) {
    node.overflow = if layout.phone {
        Overflow::scroll_y()
    } else {
        Overflow::clip()
    };
    node.display = if !layout.phone || panel == Some(PhonePanel::Selection) {
        Display::Flex
    } else {
        Display::None
    };
    node.position_type = if layout.phone {
        PositionType::Absolute
    } else {
        PositionType::Relative
    };
    node.left = if layout.phone {
        px(layout.details.min.x)
    } else {
        auto()
    };
    node.top = if layout.phone {
        px(layout.details.min.y)
    } else {
        auto()
    };
    node.width = if layout.phone {
        px(layout.details.width())
    } else {
        auto()
    };
    node.height = if layout.phone {
        px(layout.details.height())
    } else {
        auto()
    };
    node.flex_grow = if layout.phone { 0.0 } else { 1.0 };
    node.flex_basis = if layout.phone { auto() } else { px(0) };
}

fn place_commands(node: &mut Node, layout: &HudLayout) {
    node.display = if !layout.phone || layout.commands.is_some() {
        Display::Grid
    } else {
        Display::None
    };
    node.position_type = if layout.phone {
        PositionType::Absolute
    } else {
        PositionType::Relative
    };
    node.padding = if layout.phone {
        UiRect::ZERO
    } else {
        UiRect::all(px(12))
    };
    node.left = if layout.phone {
        px(layout.commands.map_or(0.0, |rect| rect.min.x))
    } else {
        auto()
    };
    node.top = if layout.phone {
        px(layout.commands.map_or(0.0, |rect| rect.min.y))
    } else {
        auto()
    };
    node.width = px(if layout.phone {
        if layout.landscape { 210.0 } else { 102.0 }
    } else {
        184.0
    });
    node.height = if layout.phone {
        px(if layout.landscape { 48.0 } else { 102.0 })
    } else {
        percent(100)
    };
    node.grid_template_columns = RepeatedGridTrack::px(
        if layout.phone {
            if layout.landscape { 4 } else { 2 }
        } else {
            3
        },
        48.0,
    );
    node.grid_template_rows = RepeatedGridTrack::px(
        if layout.phone && layout.landscape {
            1
        } else if layout.phone {
            2
        } else {
            3
        },
        48.0,
    );
    node.column_gap = px(if layout.phone { 6 } else { 8 });
    node.row_gap = node.column_gap;
}

type SelectionStyle<'a> = (
    &'a mut ImageNode,
    &'a Hovered,
    Has<Pressed>,
    &'a mut BackgroundColor,
    &'a mut BorderColor,
);

pub(super) fn update_controls(
    selection: Res<Selection>,
    game: Res<Match>,
    phone: Res<PhoneHud>,
    assets: Res<AssetServer>,
    graphics: Res<Graphics>,
    mut button: Query<SelectionStyle, With<SelectionButton>>,
    mut count: Query<&mut Text, (With<SelectionCount>, Without<PanelTitle>)>,
    mut title: Query<&mut Text, (With<PanelTitle>, Without<SelectionCount>)>,
) {
    for (mut image, hovered, pressed, mut fill, mut edge) in &mut button {
        if let Some(unit) = selection.units.first().and_then(|id| game.sim.unit(*id)) {
            image.image = assets.load(game.sim.rules().unit(unit.kind).sprite.clone());
            image.color = graphics.player_color(unit.owner.0);
        }
        let look = button_look(Tone::Item, false, hovered.get(), pressed, false);
        fill.set_if_neq(BackgroundColor(look.fill));
        edge.set_if_neq(BorderColor::all(look.edge));
    }
    for mut label in &mut count {
        label.set_if_neq(Text::new(selection.units.len().to_string()));
    }
    for mut label in &mut title {
        label.set_if_neq(Text::new(if phone.panel == Some(PhonePanel::Map) {
            "Battlefield"
        } else {
            "Selected units"
        }));
    }
}
