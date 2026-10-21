//! Reusable interface pieces, authored as Bevy scenes.
//!
//! Every widget is a function returning `impl Scene`, so pages compose them with `@widget(...)`
//! and patch individual fields where needed. Colors follow from state in [`update_looks`]:
//! scenes only choose a [`Tone`].

use bevy::{
    ecs::template::TemplateContext,
    input_focus::{
        InputFocus, InputFocusVisible,
        tab_navigation::{TabGroup, TabIndex},
    },
    picking::hover::Hovered,
    prelude::*,
    text::{EditableText, FontSize, FontWeight, LineHeight, TextCursorStyle},
    ui::{InteractionDisabled, Pressed},
    ui_widgets::{Button, TextInput},
};

use super::Action;

pub const FONT: &str = "fonts/Exo2.ttf";

/// Map previews never grow taller than this, so the actions below them stay in view.
pub const PREVIEW_MAX_HEIGHT: f32 = 300.0;

/// Smallest touch target side, in logical pixels.
pub const TOUCH_TARGET: f32 = 48.0;

// Field-table palette: graphite, slate, chalk, and ocher. Terrain supplies the vivid color.
pub const SCREEN: Color = Color::srgb_u8(28, 38, 41);
pub const PANEL: Color = Color::srgb_u8(37, 50, 53);
pub const RAIL: Color = Color::srgb_u8(23, 32, 35);
pub const BAR: Color = Color::srgba(0.090, 0.125, 0.137, 0.97);
pub const EDGE: Color = Color::srgb_u8(66, 83, 86);
pub const FIELD: Color = Color::srgb_u8(24, 34, 37);
pub const FIELD_EDGE: Color = Color::srgb_u8(104, 125, 128);
pub const INK: Color = Color::srgb_u8(237, 239, 225);
pub const INK_MUTED: Color = Color::srgb_u8(168, 185, 182);
pub const INK_DISABLED: Color = Color::srgb_u8(112, 132, 132);
/// Ocher: primary actions, invite codes, selections, and keyboard focus.
pub const SIGNAL: Color = Color::srgb_u8(223, 184, 105);
pub const DANGER: Color = Color::srgb_u8(244, 148, 133);
pub const GOOD: Color = Color::srgb_u8(160, 206, 170);
pub const SCRIM: Color = Color::srgba(0.035, 0.055, 0.063, 0.86);

/// Visual weight of an interactive element.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    /// Filled: the main action of its group.
    Primary,
    /// Outlined: everything else.
    #[default]
    Secondary,
    /// Outlined in red: destructive actions.
    Danger,
    /// A sidebar destination.
    Nav,
    /// A selectable row in a list.
    Item,
}

/// Label text and icons recolored with their button's state.
#[derive(Component, Clone, Default)]
pub struct ButtonInk;

/// A list row or option that stays highlighted while its action's value is chosen.
#[derive(Component, Clone, Default)]
pub struct Selectable;

/// A text field of the menu.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Field {
    #[default]
    Url,
    Code,
    Name,
}

/// Text that follows the menu model. Recomputed when the model changes.
#[derive(Component, Clone)]
pub struct Bind(pub fn(&super::View) -> String);

impl Default for Bind {
    fn default() -> Self {
        Self(|_| String::new())
    }
}

/// Shown only while the function returns true.
#[derive(Component, Clone)]
pub struct ShowIf(pub fn(&super::View) -> bool);

impl Default for ShowIf {
    fn default() -> Self {
        Self(|_| true)
    }
}

/// The game font at `size`, resolved when the scene spawns.
fn game_font(size: f32) -> impl Fn(&mut TemplateContext) -> Result<TextFont> + Clone {
    move |context| {
        Ok(TextFont {
            font: context.resource::<AssetServer>().load(FONT).into(),
            font_size: FontSize::Px(size),
            weight: if size >= 22.0 {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            },
            ..default()
        })
    }
}

/// A map image sized to its container's width, at most [`PREVIEW_MAX_HEIGHT`] tall, keeping
/// this width-to-height ratio. Flex layout cannot cap an aspect-ratio box's height without
/// distorting it or reserving the uncapped height.
#[derive(Component, Clone, Default)]
pub struct FitPreview(pub f32);

/// The spinning connection indicator.
#[derive(Component, Clone, Default)]
pub struct Spinner;

/// A run of text in the game font.
pub fn text(content: impl Into<String>, size: f32, color: Color) -> impl Scene {
    let content = Text::new(content);
    bsn! {
        content
        template(game_font(size))
        TextColor(color)
        LineHeight::RelativeToFont(1.3)
    }
}

/// A page or dialog heading.
pub fn heading(content: impl Into<String>) -> impl Scene {
    let content = content.into();
    bsn! { @text(content, 30.0, INK) }
}

/// Small explanatory text.
pub fn hint(content: impl Into<String>) -> impl Scene {
    let content = content.into();
    bsn! { @text(content, 13.0, INK_MUTED) }
}

/// A touch-sized button that sends `action` when activated. Not in the tab order.
pub fn plain_button(label: impl Into<String>, action: Action, tone: Tone) -> impl Scene {
    let label = label.into();
    let name = Name::new(label.clone());
    bsn! {
        name
        action
        tone
        Button
        Hovered
        Node {
            min_height: px(TOUCH_TARGET),
            min_width: px(TOUCH_TARGET),
            padding: UiRect::axes(px(16), px(8)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(2)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            column_gap: px(8),
        }
        Outline { width: px(2), offset: px(2), color: Color::NONE }
        on(super::activate)
        Children [
            @text(label, 15.0, INK)
            ButtonInk
            TextLayout { linebreak: LineBreak::NoWrap }
        ]
    }
}

/// A focusable, touch-sized button that sends `action` when activated.
pub fn button(label: impl Into<String>, action: Action, tone: Tone) -> impl Scene {
    let label = label.into();
    bsn! {
        @plain_button(label, action, tone)
        TabIndex(0)
    }
}

/// A button that fills the free width of its row.
pub fn wide_button(label: impl Into<String>, action: Action, tone: Tone) -> impl Scene {
    let label = label.into();
    bsn! {
        @button(label, action, tone)
        Node { flex_grow: 1.0, flex_basis: px(0), min_width: px(120) }
    }
}

/// A selectable list row with a title and a detail line.
pub fn list_item(
    title: impl Into<String>,
    detail: impl Into<String>,
    action: Action,
) -> impl Scene {
    let (title, detail) = (title.into(), detail.into());
    let name = Name::new(title.clone());
    bsn! {
        name
        action
        Tone::Item
        Selectable
        Button
        Hovered
        TabIndex(0)
        Node {
            flex_direction: FlexDirection::Column,
            min_height: px(TOUCH_TARGET + 8.0),
            padding: UiRect::axes(px(14), px(8)),
            border: UiRect { left: px(3), ..UiRect::all(px(1)) },
            border_radius: BorderRadius::all(px(2)),
            justify_content: JustifyContent::Center,
            row_gap: px(2),
        }
        Outline { width: px(2), offset: px(2), color: Color::NONE }
        on(super::activate)
        Children [
            @text(title, 15.0, INK) ButtonInk
            --
            @text(detail, 12.0, INK_MUTED)
        ]
    }
}

/// A row of buttons that wraps on narrow screens.
pub fn row(contents: impl SceneList) -> impl Scene {
    bsn! {
        Node {
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            column_gap: px(8),
            row_gap: px(8),
        }
        Children [ {contents} ]
    }
}

/// A vertical stack.
pub fn column(gap: f32, contents: impl SceneList) -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(gap),
        }
        Children [ {contents} ]
    }
}

/// A bordered box.
pub fn panel(contents: impl SceneList) -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(20)),
            row_gap: px(14),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
        }
        BackgroundColor(PANEL)
        BorderColor::all(EDGE)
        Children [ {contents} ]
    }
}

/// A list beside its detail panel. The detail wraps below the list on narrow screens.
pub fn split(list: impl SceneList, detail: impl SceneList) -> impl Scene {
    bsn! {
        Node {
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::FlexStart,
            column_gap: px(16),
            row_gap: px(16),
        }
        Children [
            Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                flex_basis: px(280),
                min_width: px(0),
                row_gap: px(8),
            }
            Children [ {list} ]
            --
            Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                flex_basis: px(280),
                min_width: px(0),
                padding: UiRect::all(px(14)),
                row_gap: px(10),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
            }
            BackgroundColor(PANEL)
            BorderColor::all(EDGE)
            Children [ {detail} ]
        ]
    }
}

/// A map image with plotting-table corner marks that keeps its proportions.
pub fn map_image(image: Handle<Image>, aspect: f32) -> impl Scene {
    bsn! {
        ImageNode { image: {image} }
        FitPreview(aspect)
        Node {
            align_self: AlignSelf::Center,
            margin: UiRect::vertical(px(4)),
            border: UiRect::all(px(1)),
        }
        BorderColor::all(EDGE)
        Children [
            @preview_corner(true, true)
            --
            @preview_corner(false, true)
            --
            @preview_corner(true, false)
            --
            @preview_corner(false, false)
        ]
    }
}

fn preview_corner(left: bool, top: bool) -> impl Scene {
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            width: px(12),
            height: px(12),
            left: {if left { px(-4) } else { auto() }},
            right: {if left { auto() } else { px(-4) }},
            top: {if top { px(-4) } else { auto() }},
            bottom: {if top { auto() } else { px(-4) }},
            border: UiRect {
                left: {if left { px(2) } else { px(0) }},
                right: {if left { px(0) } else { px(2) }},
                top: {if top { px(2) } else { px(0) }},
                bottom: {if top { px(0) } else { px(2) }},
            },
        }
        BorderColor::all(SIGNAL)
    }
}

/// A placeholder in place of a preview.
pub fn empty_preview(message: impl Into<String>) -> impl Scene {
    let message = message.into();
    bsn! {
        Node {
            width: percent(100),
            min_height: px(120),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: UiRect::all(px(12)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(2)),
        }
        BorderColor::all(EDGE)
        Children [ @hint(message) ]
    }
}

/// A state label with a square lamp; the word carries the meaning without color.
pub fn badge(label: impl Into<String>, color: Color) -> impl Scene {
    let label = label.into();
    bsn! {
        Node {
            padding: UiRect::axes(px(8), px(4)),
            column_gap: px(8),
            align_items: AlignItems::Center,
        }
        Children [
            Node { width: px(7), height: px(7), flex_shrink: 0.0 } BackgroundColor(color)
            --
            @text(label, 13.0, INK) TextLayout { linebreak: LineBreak::NoWrap }
        ]
    }
}

/// A labeled single-line text input. Touch taps open the keypad instead of a platform keyboard.
pub fn text_field(field: Field, value: &str, max_characters: usize) -> impl Scene {
    let editable = EditableText {
        max_characters: Some(max_characters),
        ..EditableText::new(value)
    };
    let filter = super::keypad::filter(field);
    bsn! {
        field
        TextInput
        TabIndex(0)
        editable
        filter
        TextLayout { linebreak: LineBreak::NoWrap }
        template(game_font(16.0))
        TextColor(INK)
        TextCursorStyle {
            color: SIGNAL,
            selection_color: Color::srgb_u8(75, 96, 95),
            unfocused_selection_color: EDGE,
            selected_text_color: {Some(INK)},
            selection_radius: 2.0,
        }
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            height: px(TOUCH_TARGET),
            // Centers one 16 px line (1.2 line height) inside the border.
            padding: UiRect::axes(px(12), px(13)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(2)),
        }
        BackgroundColor(FIELD)
        BorderColor::all(FIELD_EDGE)
        Outline { width: px(2), offset: px(2), color: Color::NONE }
        on(super::on_field_edit)
        on(super::submit_on_enter)
        on(super::keypad::open_on_touch)
    }
}

/// Text that follows the model.
pub fn bound_text(bind: fn(&super::View) -> String, size: f32, color: Color) -> impl Scene {
    let bind = Bind(bind);
    bsn! { @text("", size, color) bind }
}

/// A ring that turns while the client waits for the server.
pub fn spinner() -> impl Scene {
    bsn! {
        Spinner
        UiTransform
        Node {
            width: px(36),
            height: px(36),
            border: UiRect::all(px(4)),
            border_radius: BorderRadius::all(percent(50)),
        }
        BorderColor { top: SIGNAL, right: EDGE, bottom: EDGE, left: EDGE }
    }
}

/// A modal dialog over a dimmed screen.
pub fn dialog(title: impl Into<String>, contents: impl SceneList) -> impl Scene {
    let title = title.into();
    let modal = TabGroup::modal();
    bsn! {
        modal
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: UiRect::all(px(16)),
        }
        BackgroundColor(SCRIM)
        Children [
            Node {
                flex_direction: FlexDirection::Column,
                width: percent(100),
                max_width: px(460),
                padding: UiRect::all(px(20)),
                row_gap: px(14),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
            }
            BackgroundColor(PANEL)
            BorderColor::all(FIELD_EDGE)
            Children [
                @text(title, 18.0, INK)
                --
                {contents}
            ]
        ]
    }
}

/// Fill, border, and text colors for one control state.
pub struct Look {
    pub fill: Color,
    pub edge: Color,
    pub ink: Color,
}

impl Look {
    const fn new(fill: Color, edge: Color, ink: Color) -> Self {
        Self { fill, edge, ink }
    }
}

#[expect(
    clippy::fn_params_excessive_bools,
    reason = "the four flags are independent interaction states"
)]
pub const fn button_look(
    tone: Tone,
    selected: bool,
    hovered: bool,
    pressed: bool,
    disabled: bool,
) -> Look {
    const RAISED: Color = Color::srgb_u8(48, 66, 69);
    const FILLED: Color = Color::srgb_u8(61, 81, 82);
    if disabled {
        return Look::new(PANEL, EDGE, INK_DISABLED);
    }
    match (tone, selected, pressed, hovered) {
        (Tone::Primary, _, true, _) => Look::new(Color::srgb_u8(191, 150, 77), SIGNAL, RAIL),
        (Tone::Primary, _, false, true) => Look::new(Color::srgb_u8(241, 206, 143), SIGNAL, RAIL),
        (Tone::Primary, _, false, false) => Look::new(SIGNAL, SIGNAL, RAIL),
        (Tone::Danger, _, true, _) => Look::new(Color::srgb_u8(94, 52, 47), DANGER, INK),
        (Tone::Danger, _, false, true) => Look::new(Color::srgb_u8(66, 43, 40), DANGER, INK),
        (Tone::Danger, _, false, false) => Look::new(Color::NONE, DANGER, DANGER),
        (Tone::Nav | Tone::Item, true, _, _) => Look::new(FILLED, SIGNAL, INK),
        (_, _, true, _) => Look::new(FILLED, INK_MUTED, INK),
        (_, _, false, true) => Look::new(RAISED, FIELD_EDGE, INK),
        (Tone::Item, false, false, false) => Look::new(FIELD, EDGE, INK),
        (Tone::Nav, false, false, false) => Look::new(Color::NONE, Color::NONE, INK_MUTED),
        (_, _, false, false) => Look::new(Color::NONE, FIELD_EDGE, INK),
    }
}

const fn field_look(focused: bool, disabled: bool) -> Look {
    if disabled {
        Look::new(FIELD, EDGE, INK_DISABLED)
    } else if focused {
        Look::new(FIELD, SIGNAL, INK)
    } else {
        Look::new(FIELD, FIELD_EDGE, INK)
    }
}

type ButtonState<'a> = (
    Entity,
    &'a Tone,
    Option<&'a Action>,
    &'a Hovered,
    Has<Pressed>,
    Has<InteractionDisabled>,
    &'a mut BackgroundColor,
    &'a mut BorderColor,
    &'a mut Outline,
    &'a Children,
);

type FieldState<'a> = (
    Entity,
    Has<InteractionDisabled>,
    &'a mut BackgroundColor,
    &'a mut BorderColor,
    &'a mut TextColor,
    &'a mut Outline,
);

/// Colors buttons and fields from their state. Labels only ever get a [`TextColor`].
pub fn update_looks(
    view: super::ViewParam,
    focus: Res<InputFocus>,
    focus_visible: Res<InputFocusVisible>,
    mut buttons: Query<ButtonState, Without<Field>>,
    mut fields: Query<FieldState, (With<Field>, Without<Tone>)>,
    mut inks: Query<&mut TextColor, (With<ButtonInk>, Without<Field>)>,
) {
    let view = view.view();
    let focused = focus.get();
    let ring = |entity| {
        if focus_visible.0 && focused == Some(entity) {
            SIGNAL
        } else {
            Color::NONE
        }
    };
    for (
        entity,
        &tone,
        action,
        hovered,
        pressed,
        disabled,
        mut fill,
        mut edge,
        mut outline,
        children,
    ) in &mut buttons
    {
        let selected = action.is_some_and(|action| action.selected(&view));
        let look = button_look(tone, selected, hovered.get(), pressed, disabled);
        fill.set_if_neq(BackgroundColor(look.fill));
        let border = if selected && matches!(tone, Tone::Nav | Tone::Item) {
            BorderColor {
                left: SIGNAL,
                ..BorderColor::all(EDGE)
            }
        } else {
            BorderColor::all(look.edge)
        };
        edge.set_if_neq(border);
        if outline.color != ring(entity) {
            outline.color = ring(entity);
        }
        for &child in children {
            if let Ok(mut ink) = inks.get_mut(child) {
                ink.set_if_neq(TextColor(look.ink));
            }
        }
    }
    for (entity, disabled, mut fill, mut edge, mut ink, mut outline) in &mut fields {
        let look = field_look(focused == Some(entity), disabled);
        fill.set_if_neq(BackgroundColor(look.fill));
        edge.set_if_neq(BorderColor::all(look.edge));
        ink.set_if_neq(TextColor(look.ink));
        if outline.color != Color::NONE {
            outline.color = Color::NONE;
        }
    }
}

/// Sizes [`FitPreview`] images from their container's width in the last layout.
pub fn fit_previews(
    mut previews: Query<(&FitPreview, &ChildOf, &mut Node)>,
    parents: Query<&ComputedNode>,
) {
    for (fit, parent, mut node) in &mut previews {
        let Ok(parent) = parents.get(parent.parent()) else {
            continue;
        };
        let available = parent.content_box().width() * parent.inverse_scale_factor;
        if available <= 0.0 || fit.0 <= 0.0 {
            continue;
        }
        let width = available.min(PREVIEW_MAX_HEIGHT * fit.0).floor();
        let (width, height) = (px(width), px((width / fit.0).floor()));
        if node.width != width || node.height != height {
            node.width = width;
            node.height = height;
        }
    }
}

/// Turns connection spinners.
pub fn spin(time: Res<Time>, mut spinners: Query<&mut UiTransform, With<Spinner>>) {
    for mut transform in &mut spinners {
        transform.rotation = Rot2::radians(time.elapsed_secs() * 5.0);
    }
}

/// Keeps [`Bind`] texts and [`ShowIf`] visibility in step with the model.
pub fn update_bindings(
    view: super::ViewParam,
    mut texts: Query<(&Bind, &mut Text)>,
    mut shown: Query<(&ShowIf, &mut Node)>,
) {
    let view = view.view();
    for (bind, mut text) in &mut texts {
        let value = (bind.0)(&view);
        if text.0 != value {
            text.0 = value;
        }
    }
    for (show, mut node) in &mut shown {
        let display = if (show.0)(&view) {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
}

/// Keeps keyboard activation consistent with disabled looks.
pub fn update_availability(
    mut commands: Commands,
    view: super::ViewParam,
    buttons: Query<(Entity, &Action, Has<InteractionDisabled>)>,
) {
    let view = view.view();
    for (entity, action, disabled) in &buttons {
        let disable = !action.enabled(&view);
        if disable && !disabled {
            commands.entity(entity).insert(InteractionDisabled);
        } else if !disable && disabled {
            commands.entity(entity).remove::<InteractionDisabled>();
        }
    }
}
