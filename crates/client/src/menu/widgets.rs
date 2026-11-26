//! Reusable tactical RTS interface pieces, authored as Bevy scenes.
//!
//! Styled after classic armored RTS command decks (beveled steel plates, riveted header bars,
//! recessed dark display wells, brushed brass primary controls, and physical press depression).
//! Every widget is a function returning `impl Scene`, so pages compose them with `@widget(...)`
//! and patch individual fields where needed. Colors and bevels follow from state in
//! [`update_looks`]: scenes only choose a [`Tone`].

use bevy::{
    ecs::template::TemplateContext,
    input_focus::{
        InputFocus, InputFocusVisible,
        tab_navigation::{TabGroup, TabIndex},
    },
    picking::hover::Hovered,
    prelude::*,
    scene::EntityScene,
    text::{EditableText, EditableTextFilter, FontSize, FontWeight, LineHeight, TextCursorStyle},
    ui::{InteractionDisabled, Pressed},
    ui_widgets::{Button, ScrollArea, TextInput},
};

use super::Action;

pub const FONT: &str = "fonts/Exo2.ttf";

/// Map previews never grow taller than this, so the actions below them stay in view.
pub const PREVIEW_MAX_HEIGHT: f32 = 300.0;

/// Smallest touch target side, in logical pixels.
pub const TOUCH_TARGET: f32 = 48.0;

// Armored command-deck palette: gunmetal steel, brushed brass, phosphor status lamps, and chalk.
pub const SCREEN: Color = Color::srgb_u8(13, 18, 21);
pub const RAIL: Color = Color::srgb_u8(19, 26, 30);
pub const PANEL: Color = Color::srgb_u8(26, 35, 40);
pub const PANEL_HEADER: Color = Color::srgb_u8(17, 23, 27);
pub const BAR: Color = Color::srgba(0.067, 0.094, 0.110, 0.98);
pub const FIELD: Color = Color::srgb_u8(12, 17, 20);
pub const EDGE: Color = Color::srgb_u8(54, 70, 77);
pub const BEVEL_LIGHT: Color = Color::srgb_u8(94, 118, 127);
pub const BEVEL_DARK: Color = Color::srgb_u8(8, 11, 13);
pub const INK: Color = Color::srgb_u8(237, 240, 232);
pub const INK_MUTED: Color = Color::srgb_u8(146, 166, 170);
pub const INK_DISABLED: Color = Color::srgb_u8(86, 102, 107);
/// Brushed tactical brass: primary actions, invite codes, active tabs, and keyboard focus.
pub const SIGNAL: Color = Color::srgb_u8(224, 172, 68);
pub const BRASS_LIGHT: Color = Color::srgb_u8(246, 208, 126);
pub const BRASS_DARK: Color = Color::srgb_u8(132, 92, 26);
pub const GOOD: Color = Color::srgb_u8(102, 204, 122);
pub const WARNING: Color = Color::srgb_u8(232, 168, 56);
pub const DANGER: Color = Color::srgb_u8(226, 92, 80);
pub const DANGER_LIGHT: Color = Color::srgb_u8(245, 134, 124);
pub const DANGER_DARK: Color = Color::srgb_u8(116, 36, 31);
pub const SCRIM: Color = Color::srgba(0.024, 0.035, 0.043, 0.88);

/// Asymmetric 3D metallic bevel with top-left catchlight and bottom-right shadow.
pub const fn bevel(light: Color, dark: Color) -> BorderColor {
    BorderColor {
        top: light,
        left: light,
        right: dark,
        bottom: dark,
    }
}

/// Uniform four-sided border color usable in `const` contexts.
pub const fn solid_border(color: Color) -> BorderColor {
    bevel(color, color)
}

/// Raised gunmetal steel plate bevel.
pub const fn bevel_raised() -> BorderColor {
    bevel(BEVEL_LIGHT, BEVEL_DARK)
}

/// Recessed dark display-well bevel.
pub const fn bevel_recessed() -> BorderColor {
    bevel(BEVEL_DARK, BEVEL_LIGHT)
}

/// Precomputed bevel constants for direct use inside `bsn!` scenes.
pub const BEVEL_RAISED: BorderColor = bevel_raised();
pub const BEVEL_RECESSED: BorderColor = bevel_recessed();

/// Raised brushed-brass CTA bevel.
pub const fn bevel_brass() -> BorderColor {
    bevel(BRASS_LIGHT, BRASS_DARK)
}

/// Depressed brushed-brass CTA bevel.
pub const fn bevel_brass_pressed() -> BorderColor {
    bevel(BRASS_DARK, BRASS_LIGHT)
}

/// Raised crimson destructive-button bevel.
pub const fn bevel_danger() -> BorderColor {
    bevel(DANGER_LIGHT, DANGER_DARK)
}

/// Depressed crimson destructive-button bevel.
pub const fn bevel_danger_pressed() -> BorderColor {
    bevel(DANGER_DARK, DANGER_LIGHT)
}

/// Visual weight of an interactive element.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    /// Brushed brass plate: the main command of its group.
    Primary,
    /// Beveled gunmetal steel push-button.
    #[default]
    Secondary,
    /// Armored crimson plate: destructive or abort actions.
    Danger,
    /// Top command-deck navigation tab.
    Nav,
    /// A selectable plate in a roster, list, or option bar.
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
    HostBind,
    HostAssets,
}

impl Field {
    /// Whether this field accepts `c` from the platform keyboard.
    pub const fn accepts(self, c: char) -> bool {
        match self {
            Self::Url | Self::HostBind => !c.is_whitespace() && !c.is_control(),
            Self::Code => c.is_ascii_alphanumeric(),
            Self::Name | Self::HostAssets => !c.is_control(),
        }
    }

    /// Characters this field accepts from the platform keyboard.
    pub fn filter(self) -> EditableTextFilter {
        EditableTextFilter::new(move |c| self.accepts(c))
    }
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
            weight: if size >= 18.0 {
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

/// A small armored rivet dot for panel headers and console frames.
pub fn rivet() -> impl Scene {
    bsn! {
        Node {
            width: px(5),
            height: px(5),
            flex_shrink: 0.0,
            border: UiRect::all(px(1)),
        }
        BackgroundColor(PANEL)
        BEVEL_RAISED
    }
}

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
    bsn! { @text(content, 28.0, INK) }
}

/// Small explanatory text.
pub fn hint(content: impl Into<String>) -> impl Scene {
    let content = content.into();
    bsn! { @text(content, 13.0, INK_MUTED) }
}

/// Tactical page header bar with a brass vertical marker and steel underline.
pub fn section_bar(title: impl Into<String>, subtitle: Option<String>) -> impl Scene {
    let title = title.into();
    let sub: Vec<Box<dyn SceneList>> =
        subtitle.map_or_default(|s| vec![Box::new(EntityScene(hint(s))) as Box<dyn SceneList>]);
    bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Stretch,
            column_gap: px(12),
            padding: UiRect::bottom(px(10)),
            border: UiRect::bottom(px(2)),
        }
        BorderColor { bottom: EDGE, ..BorderColor::all(Color::NONE) }
        Children [
            Node {
                width: px(4),
                flex_shrink: 0.0,
            }
            BackgroundColor(SIGNAL)
            --
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(3),
                flex_grow: 1.0,
                min_width: px(0),
            }
            Children [
                @heading(title)
                --
                {sub}
            ]
        ]
    }
}

/// A touch-sized beveled button that sends `action` when activated. Not in the tab order.
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
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(1)),
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

/// A selectable armored list plate with a title and a detail line.
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
            border: UiRect { left: px(3), top: px(2), right: px(2), bottom: px(2) },
            border_radius: BorderRadius::all(px(1)),
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

/// A beveled gunmetal armor plate.
pub fn panel(contents: impl SceneList) -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(18)),
            row_gap: px(14),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(1)),
        }
        BackgroundColor(PANEL)
        BEVEL_RAISED
        Children [ {contents} ]
    }
}

/// A beveled armor plate with a riveted dark header bar and brass accent trim.
pub fn armored_panel(title: impl Into<String>, contents: impl SceneList) -> impl Scene {
    let title = title.into();
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(1)),
        }
        BackgroundColor(PANEL)
        BEVEL_RAISED
        Children [
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(10),
                padding: UiRect::axes(px(14), px(9)),
                border: UiRect::bottom(px(2)),
            }
            BackgroundColor(PANEL_HEADER)
            BorderColor { bottom: BRASS_DARK, ..BorderColor::all(Color::NONE) }
            Children [
                Node { width: px(4), height: px(14), flex_shrink: 0.0 }
                BackgroundColor(SIGNAL)
                --
                @text(title, 16.0, INK)
                Node { flex_grow: 1.0, min_width: px(0) }
                --
                @rivet()
            ]
            --
            Node {
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(16)),
                row_gap: px(12),
            }
            Children [ {contents} ]
        ]
    }
}

/// A recessed dark steel display well for lists or telemetry.
pub fn well(contents: impl SceneList) -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(8)),
            row_gap: px(6),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(1)),
        }
        BackgroundColor(FIELD)
        BEVEL_RECESSED
        Children [ {contents} ]
    }
}

/// A recessed LCD readout strip for invite codes, QUIC URLs, or telemetry strings.
pub fn readout_box(value: impl Into<String>) -> impl Scene {
    let value = value.into();
    bsn! {
        Node {
            padding: UiRect::axes(px(12), px(9)),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(1)),
        }
        BackgroundColor(FIELD)
        BEVEL_RECESSED
        Children [ @text(value, 13.0, SIGNAL) ]
    }
}

/// A list beside its armored detail panel. The detail wraps below the list on narrow screens.
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
                flex_basis: px(300),
                min_width: px(0),
                padding: UiRect::all(px(8)),
                row_gap: px(6),
                border: UiRect::all(px(2)),
                border_radius: BorderRadius::all(px(1)),
            }
            BackgroundColor(FIELD)
            BEVEL_RECESSED
            Children [ {list} ]
            --
            Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                flex_basis: px(300),
                min_width: px(0),
                padding: UiRect::all(px(16)),
                row_gap: px(12),
                border: UiRect::all(px(2)),
                border_radius: BorderRadius::all(px(1)),
            }
            BackgroundColor(PANEL)
            BEVEL_RAISED
            Children [ {detail} ]
        ]
    }
}

/// A recon map preview framed inside a recessed dark well with brass targeting brackets.
pub fn map_image(image: Handle<Image>, aspect: f32) -> impl Scene {
    bsn! {
        ImageNode { image: {image} }
        FitPreview(aspect)
        Node {
            align_self: AlignSelf::Center,
            margin: UiRect::vertical(px(4)),
            border: UiRect::all(px(2)),
        }
        BEVEL_RECESSED
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
            width: px(14),
            height: px(14),
            left: {if left { px(-5) } else { auto() }},
            right: {if left { auto() } else { px(-5) }},
            top: {if top { px(-5) } else { auto() }},
            bottom: {if top { auto() } else { px(-5) }},
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

/// A recessed tactical placeholder in place of a preview or empty list.
pub fn empty_preview(message: impl Into<String>) -> impl Scene {
    let message = message.into();
    bsn! {
        Node {
            width: percent(100),
            min_height: px(112),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: UiRect::all(px(14)),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(1)),
        }
        BackgroundColor(FIELD)
        BEVEL_RECESSED
        Children [ @hint(message) ]
    }
}

/// An armored status readout lamp; the word carries the meaning without color alone.
pub fn badge(label: impl Into<String>, color: Color) -> impl Scene {
    let label = label.into();
    bsn! {
        Node {
            padding: UiRect::axes(px(10), px(5)),
            column_gap: px(8),
            align_items: AlignItems::Center,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(1)),
        }
        BackgroundColor(FIELD)
        BEVEL_RECESSED
        Children [
            Node { width: px(8), height: px(8), flex_shrink: 0.0 } BackgroundColor(color)
            --
            @text(label, 13.0, INK) TextLayout { linebreak: LineBreak::NoWrap }
        ]
    }
}

/// A labeled recessed single-line text input. Touch taps focus it and open the platform keyboard.
pub fn text_field(field: Field, value: &str, max_characters: usize) -> impl Scene {
    let editable = EditableText {
        max_characters: Some(max_characters),
        ..EditableText::new(value)
    };
    let filter = field.filter();
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
            selection_color: Color::srgb_u8(62, 86, 94),
            unfocused_selection_color: EDGE,
            selected_text_color: {Some(INK)},
            selection_radius: 1.0,
        }
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            height: px(TOUCH_TARGET),
            padding: UiRect::axes(px(12), px(12)),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(1)),
        }
        BackgroundColor(FIELD)
        BEVEL_RECESSED
        Outline { width: px(2), offset: px(2), color: Color::NONE }
        on(super::on_field_edit)
        on(super::submit_on_enter)
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

/// A riveted tactical modal dialog over a dimmed screen.
pub fn dialog(title: impl Into<String>, contents: impl SceneList) -> impl Scene {
    dialog_with_title(text(title, 18.0, INK), contents)
}

pub fn dialog_with_title(title: impl Scene, contents: impl SceneList) -> impl Scene {
    let title = EntityScene(title);
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
                max_width: px(480),
                max_height: percent(100),
                overflow: Overflow::scroll_y(),
                border: UiRect::all(px(2)),
                border_radius: BorderRadius::all(px(1)),
            }
            ScrollArea
            on(super::scroll_on_touch_drag)
            on(super::scroll_focus_into_view)
            BackgroundColor(PANEL)
            BEVEL_RAISED
            Children [
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(10),
                    padding: UiRect::axes(px(18), px(12)),
                    border: UiRect::bottom(px(2)),
                    flex_shrink: 0.0,
                }
                BackgroundColor(PANEL_HEADER)
                BorderColor { bottom: SIGNAL, ..BorderColor::all(Color::NONE) }
                Children [
                    Node { width: px(4), height: px(18), flex_shrink: 0.0 }
                    BackgroundColor(SIGNAL)
                    --
                    Node { flex_grow: 1.0, min_width: px(0) }
                    Children [ {title} ]
                    --
                    @rivet()
                ]
                --
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(px(18)),
                    row_gap: px(14),
                }
                Children [ {contents} ]
            ]
        ]
    }
}

/// Fill, 3D beveled border, and text colors for one control state.
pub struct Look {
    pub fill: Color,
    pub border: BorderColor,
    pub ink: Color,
}

impl Look {
    const fn new(fill: Color, border: BorderColor, ink: Color) -> Self {
        Self { fill, border, ink }
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
    const STEEL_IDLE: Color = Color::srgb_u8(34, 46, 52);
    const STEEL_HOVER: Color = Color::srgb_u8(46, 62, 70);
    const STEEL_PRESSED: Color = Color::srgb_u8(20, 28, 32);
    const STEEL_SELECTED: Color = Color::srgb_u8(40, 54, 62);
    const BRASS_HOVER: Color = Color::srgb_u8(242, 194, 96);
    const BRASS_PRESSED: Color = Color::srgb_u8(180, 134, 44);
    const CRIMSON_IDLE: Color = Color::srgb_u8(76, 30, 28);
    const CRIMSON_HOVER: Color = Color::srgb_u8(108, 40, 36);
    const CRIMSON_PRESSED: Color = Color::srgb_u8(54, 21, 19);
    const NAV_IDLE: Color = Color::srgb_u8(22, 30, 35);
    const NAV_HOVER: Color = Color::srgb_u8(34, 46, 53);

    if disabled {
        return Look::new(PANEL, solid_border(EDGE), INK_DISABLED);
    }
    match (tone, selected, pressed, hovered) {
        (Tone::Primary, _, true, _) => Look::new(BRASS_PRESSED, bevel_brass_pressed(), RAIL),
        (Tone::Primary, _, false, true) => Look::new(BRASS_HOVER, bevel_brass(), RAIL),
        (Tone::Primary, _, false, false) => Look::new(SIGNAL, bevel_brass(), RAIL),

        (Tone::Danger, _, true, _) => Look::new(CRIMSON_PRESSED, bevel_danger_pressed(), INK),
        (Tone::Danger, _, false, true) => Look::new(CRIMSON_HOVER, bevel_danger(), INK),
        (Tone::Danger, _, false, false) => Look::new(CRIMSON_IDLE, bevel_danger(), INK),

        (Tone::Nav, true, _, _) => Look::new(
            STEEL_SELECTED,
            BorderColor {
                top: BEVEL_LIGHT,
                left: BEVEL_LIGHT,
                right: BEVEL_DARK,
                bottom: SIGNAL,
            },
            BRASS_LIGHT,
        ),
        (Tone::Item, true, _, _) => Look::new(
            STEEL_SELECTED,
            BorderColor {
                left: SIGNAL,
                top: BEVEL_DARK,
                right: BEVEL_LIGHT,
                bottom: BEVEL_LIGHT,
            },
            INK,
        ),
        (Tone::Nav | Tone::Item, false, true, _) | (Tone::Secondary, _, true, _) => {
            Look::new(STEEL_PRESSED, bevel_recessed(), INK)
        }
        (Tone::Nav, false, false, true) => Look::new(NAV_HOVER, bevel_raised(), INK),
        (Tone::Nav, false, false, false) => Look::new(
            NAV_IDLE,
            BorderColor {
                top: EDGE,
                left: EDGE,
                right: BEVEL_DARK,
                bottom: BEVEL_DARK,
            },
            INK_MUTED,
        ),
        (Tone::Item, false, false, true) | (Tone::Secondary, _, false, true) => {
            Look::new(STEEL_HOVER, bevel_raised(), INK)
        }
        (Tone::Item, false, false, false) => Look::new(
            Color::srgb_u8(20, 28, 33),
            bevel(Color::srgb_u8(56, 72, 80), BEVEL_DARK),
            INK,
        ),
        (Tone::Secondary, _, false, false) => Look::new(STEEL_IDLE, bevel_raised(), INK),
    }
}

const fn field_look(focused: bool, disabled: bool) -> Look {
    if disabled {
        Look::new(FIELD, solid_border(EDGE), INK_DISABLED)
    } else if focused {
        Look::new(FIELD, solid_border(SIGNAL), INK)
    } else {
        Look::new(FIELD, bevel_recessed(), INK)
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
        edge.set_if_neq(look.border);
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
        edge.set_if_neq(look.border);
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
/// Bevy ignores releases while a button is disabled, so settle its press when disabling it.
pub fn clear_disabled_button_press(
    event: On<Add<InteractionDisabled>>,
    pressed: Query<(), (With<Button>, With<Pressed>)>,
    mut commands: Commands,
) {
    if pressed.contains(event.entity) {
        commands.entity(event.entity).remove::<Pressed>();
    }
}

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

#[cfg(test)]
mod tests {
    use super::{
        BEVEL_DARK, BEVEL_LIGHT, Field, SIGNAL, Tone, bevel_brass, bevel_brass_pressed,
        bevel_raised, bevel_recessed, button_look,
    };

    #[test]
    fn fields_accept_their_typed_characters() {
        assert!(Field::Url.accepts('a'));
        assert!(Field::Url.accepts(':'));
        assert!(!Field::Url.accepts(' '));
        assert!(!Field::Url.accepts('\n'));

        assert!(Field::Code.accepts('A'));
        assert!(Field::Code.accepts('7'));
        assert!(!Field::Code.accepts('/'));

        assert!(Field::Name.accepts(' '));
        assert!(Field::Name.accepts('\''));
        assert!(!Field::Name.accepts('\t'));

        assert!(Field::HostBind.accepts('0'));
        assert!(Field::HostBind.accepts(':'));
        assert!(!Field::HostBind.accepts(' '));

        assert!(Field::HostAssets.accepts('/'));
        assert!(Field::HostAssets.accepts(' '));
        assert!(!Field::HostAssets.accepts('\n'));
    }

    #[test]
    fn buttons_invert_metallic_bevels_when_pressed_and_highlight_active_tabs() {
        assert_eq!(
            button_look(Tone::Secondary, false, false, false, false).border,
            bevel_raised()
        );
        assert_eq!(
            button_look(Tone::Secondary, false, false, true, false).border,
            bevel_recessed()
        );
        assert_eq!(
            button_look(Tone::Primary, false, false, false, false).border,
            bevel_brass()
        );
        assert_eq!(
            button_look(Tone::Primary, false, false, true, false).border,
            bevel_brass_pressed()
        );
        let nav = button_look(Tone::Nav, true, false, false, false);
        assert_eq!(nav.border.bottom, SIGNAL);
        assert_eq!(nav.border.top, BEVEL_LIGHT);
        assert_eq!(nav.border.right, BEVEL_DARK);
        let item = button_look(Tone::Item, true, false, false, false);
        assert_eq!(item.border.left, SIGNAL);
    }
}
