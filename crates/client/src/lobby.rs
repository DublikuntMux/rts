//! Lobby screen and in-game session bar.
//!
//! The lobby (in [`AppState::Menu`]) starts solo matches, creates or joins online rooms, and
//! manages replays. The session bar (in [`AppState::InGame`]) shows the match status with Save
//! replay and Leave buttons. Both only read [`SessionStatus`] and send [`SessionAction`]s;
//! the session module does the work.
//!
//! Bevy's text input has no on-screen keyboard yet, so a touch on a lobby field opens a
//! touch keypad instead. Mouse and physical keyboards keep using the text input directly.

use bevy::{
    input::keyboard::{Key, KeyboardInput},
    input_focus::{
        AutoFocus, FocusGained, FocusedInput, InputFocus, InputFocusVisible,
        tab_navigation::{TabGroup, TabIndex, TabNavigationPlugin},
    },
    picking::{
        events::{PointerClick, PointerDrag},
        hover::Hovered,
        pointer::{PointerButton, PointerId},
    },
    prelude::*,
    text::{
        EditableText, EditableTextFilter, LetterSpacing, LineHeight, TextCursorStyle, TextEdit,
        TextEditChange, TextReadWriteMode,
    },
    ui::{InteractionDisabled, Pressed},
    ui_widgets::{Activate, Button, ScrollArea, ScrollIntoView, TextInput, observe},
};

use crate::{
    font::GameFont,
    game::AppState,
    session::{SessionAction, SessionStatus},
};

/// Height of the in-game session bar in logical pixels. World input should ignore pointers over it.
pub const SESSION_BAR_HEIGHT: f32 = 56.0;

/// Relay address shown until the player types another one.
const DEFAULT_RELAY_URL: &str = "ws://127.0.0.1:9001";
const ROOM_CODE_LEN: usize = 6;
const MAX_URL_LEN: usize = 256;
/// Punctuation on the touch keypad for server addresses.
const URL_SYMBOLS: [char; 13] = [
    ':', '/', '.', '-', '_', '?', '&', '=', '%', '+', '[', ']', '#',
];
/// Keypad keys per row. Six 48 px keys fit a 320 px screen.
const KEYPAD_COLUMNS: u16 = 6;

/// Smallest touch target side, in logical pixels.
const TOUCH_TARGET: f32 = 48.0;
/// The lobby column never grows wider than this.
const COLUMN_MAX_WIDTH: f32 = 440.0;

const SCREEN: Color = Color::srgb(0.031, 0.059, 0.102);
const PANEL: Color = Color::srgb(0.055, 0.094, 0.157);
const BAR: Color = Color::srgba(0.055, 0.094, 0.157, 0.94);
const EDGE: Color = Color::srgb(0.204, 0.278, 0.373);
const FIELD: Color = Color::srgb(0.031, 0.055, 0.094);
const FIELD_EDGE: Color = Color::srgb(0.302, 0.392, 0.506);
const INK: Color = Color::srgb(0.886, 0.914, 0.945);
const INK_MUTED: Color = Color::srgb(0.596, 0.667, 0.753);
const INK_DISABLED: Color = Color::srgb(0.337, 0.404, 0.490);
/// Signal amber: the room code plate and keyboard focus.
const SIGNAL: Color = Color::srgb(0.910, 0.722, 0.290);

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<TabNavigationPlugin>() {
            app.add_plugins(TabNavigationPlugin);
        }
        app.init_resource::<LobbyForm>()
            .init_resource::<TouchKeypad>()
            .add_systems(OnEnter(AppState::Menu), spawn_lobby)
            .add_systems(OnExit(AppState::Menu), reset_keypad)
            .add_systems(OnEnter(AppState::InGame), spawn_session_bar)
            .add_systems(
                Update,
                (
                    update_lobby_text,
                    update_room_panel,
                    update_field_availability,
                    sync_keypad.after(update_field_availability),
                )
                    .run_if(in_state(AppState::Menu)),
            )
            .add_systems(Update, update_bar_text.run_if(in_state(AppState::InGame)))
            .add_systems(
                Update,
                (update_button_availability, update_looks)
                    .chain()
                    .after(update_field_availability)
                    .run_if(not(in_state(AppState::Loading))),
            );
    }
}

/// Text typed into the lobby fields. Kept across visits to the lobby.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
struct LobbyForm {
    url: String,
    /// Normalized room code: uppercase letters and digits.
    code: String,
}

impl Default for LobbyForm {
    fn default() -> Self {
        Self {
            url: DEFAULT_RELAY_URL.into(),
            code: String::new(),
        }
    }
}

impl LobbyForm {
    fn url(&self) -> &str {
        self.url.trim()
    }

    fn url_valid(&self) -> bool {
        let url = self.url();
        ["ws://", "wss://"]
            .iter()
            .any(|scheme| url.len() > scheme.len() && url.starts_with(scheme))
    }

    fn code_valid(&self) -> bool {
        self.code.len() == ROOM_CODE_LEN
            && self
                .code
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    }
}

/// A button and the session action it sends.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Solo,
    Host,
    Join,
    Start,
    Leave,
    ReplayLast,
    SaveReplay,
    LoadReplay,
}

impl Control {
    fn enabled(self, status: &SessionStatus, form: &LobbyForm) -> bool {
        let can_connect = !status.busy && status.room_code.is_none();
        match self {
            Self::Solo | Self::LoadReplay => !status.busy,
            Self::Host => can_connect && form.url_valid(),
            Self::Join => can_connect && form.url_valid() && form.code_valid(),
            Self::Start => status.can_start,
            Self::Leave => status.busy || status.room_code.is_some() || status.running,
            Self::ReplayLast | Self::SaveReplay => status.has_replay,
        }
    }

    fn action(self, form: &LobbyForm) -> SessionAction {
        match self {
            Self::Solo => SessionAction::Solo,
            Self::Host => SessionAction::Host {
                url: form.url().into(),
            },
            Self::Join => SessionAction::Join {
                url: form.url().into(),
                code: form.code.clone(),
            },
            Self::Start => SessionAction::Start,
            Self::Leave => SessionAction::Leave,
            Self::ReplayLast => SessionAction::ReplayLast,
            Self::SaveReplay => SessionAction::SaveReplay,
            Self::LoadReplay => SessionAction::LoadReplay,
        }
    }
}

/// Visual weight of a button.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum ButtonStyle {
    /// Filled: the main action of its group.
    Primary,
    /// Outlined: everything else.
    Secondary,
}

/// Button label text, recolored with the button state.
#[derive(Component)]
struct ButtonInk;

/// A lobby text field.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Url,
    Code,
}

/// Lobby status line under the title.
#[derive(Component)]
struct LobbyStatusText;

/// Hint under the online fields explaining what to do next.
#[derive(Component)]
struct OnlineHintText;

/// Shown only while in a room.
#[derive(Component)]
struct RoomPanel;

#[derive(Component)]
struct RoomCodeText;

#[derive(Component)]
struct RoomRoleText;

#[derive(Component)]
struct RoomCountText;

/// Session bar headline: what is running.
#[derive(Component)]
struct BarTitleText;

/// Session bar detail: the latest session message.
#[derive(Component)]
struct BarMessageText;

/// Fill, border, and text colors for one control state.
struct Look {
    fill: Color,
    edge: Color,
    ink: Color,
}

// ---------------------------------------------------------------------------
// Lobby
// ---------------------------------------------------------------------------

fn spawn_lobby(
    mut commands: Commands,
    font: Res<GameFont>,
    form: Res<LobbyForm>,
    status: Res<SessionStatus>,
) {
    let font = &font.0;

    let title = (
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            ..default()
        },
        children![
            (
                Text::new("RTS"),
                text_font(font, 32.0),
                TextColor(INK),
                LetterSpacing::Px(2.0),
            ),
            (
                LobbyStatusText,
                Text::new(lobby_status(&status)),
                text_font(font, 15.0),
                TextColor(INK_MUTED),
                LineHeight::RelativeToFont(1.35),
            ),
        ],
    );

    let solo = (
        button(font, Control::Solo, ButtonStyle::Primary, "Play solo"),
        AutoFocus,
    );

    let online = online_section(font, &form, &status);
    let replays = replay_section(font);

    commands.spawn((
        Name::new("Lobby"),
        DespawnOnExit(AppState::Menu),
        TabGroup::new(0),
        ScrollArea,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            overflow: Overflow::scroll_y(),
            ..default()
        },
        BackgroundColor(SCREEN),
        observe(scroll_on_touch_drag),
        observe(scroll_focus_into_view),
        children![(
            Node {
                width: percent(100),
                max_width: px(COLUMN_MAX_WIDTH),
                flex_direction: FlexDirection::Column,
                flex_shrink: 0.0,
                padding: UiRect::axes(px(16), px(24)),
                row_gap: px(20),
                ..default()
            },
            children![title, solo, online, replays],
        )],
    ));
}

/// Server address, room creation and joining, and the current room.
fn online_section(font: &Handle<Font>, form: &LobbyForm, status: &SessionStatus) -> impl Bundle {
    section(
        font,
        "Online match",
        (
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(12),
                ..default()
            },
            children![
                labeled_field(font, "Server address", Field::Url, &form.url),
                button(font, Control::Host, ButtonStyle::Secondary, "Create room"),
                (
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: px(6),
                        ..default()
                    },
                    children![
                        field_label(font, "Room code"),
                        (
                            Node {
                                column_gap: px(8),
                                align_items: AlignItems::Stretch,
                                ..default()
                            },
                            children![
                                field(font, Field::Code, &form.code),
                                sized_button(
                                    font,
                                    Control::Join,
                                    ButtonStyle::Secondary,
                                    "Join room",
                                    0.0
                                ),
                            ],
                        ),
                    ],
                ),
                (
                    OnlineHintText,
                    Text::new(online_hint(status, form)),
                    text_font(font, 13.0),
                    TextColor(INK_MUTED),
                    LineHeight::RelativeToFont(1.35),
                ),
                room_panel(font, status),
                (
                    button_row(),
                    children![
                        button(font, Control::Start, ButtonStyle::Primary, "Start match"),
                        button(font, Control::Leave, ButtonStyle::Secondary, "Leave room"),
                    ],
                ),
            ],
        ),
    )
}

fn replay_section(font: &Handle<Font>) -> impl Bundle {
    section(
        font,
        "Replays",
        (
            button_row(),
            children![
                button(
                    font,
                    Control::ReplayLast,
                    ButtonStyle::Secondary,
                    "Replay last"
                ),
                button(
                    font,
                    Control::SaveReplay,
                    ButtonStyle::Secondary,
                    "Save replay"
                ),
                button(
                    font,
                    Control::LoadReplay,
                    ButtonStyle::Secondary,
                    if cfg!(target_arch = "wasm32") {
                        "Load replay"
                    } else {
                        "Load saved replay"
                    }
                ),
            ],
        ),
    )
}

/// A titled panel.
fn section(font: &Handle<Font>, heading: &'static str, body: impl Bundle) -> impl Bundle {
    (
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(16)),
            row_gap: px(14),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        BackgroundColor(PANEL),
        BorderColor::all(EDGE),
        children![
            (Text::new(heading), text_font(font, 18.0), TextColor(INK)),
            body,
        ],
    )
}

/// A row of buttons that wraps on narrow screens.
fn button_row() -> Node {
    Node {
        flex_wrap: FlexWrap::Wrap,
        column_gap: px(8),
        row_gap: px(8),
        ..default()
    }
}

/// Room code, role, and player count. Hidden until the player is in a room.
fn room_panel(font: &Handle<Font>, status: &SessionStatus) -> impl Bundle {
    (
        RoomPanel,
        Node {
            display: room_display(status),
            flex_direction: FlexDirection::Column,
            padding: UiRect::axes(px(14), px(12)),
            row_gap: px(4),
            border: UiRect::left(px(3)),
            border_radius: BorderRadius::all(px(4)),
            ..default()
        },
        BackgroundColor(FIELD),
        BorderColor::all(SIGNAL),
        children![
            (
                Text::new("Room code"),
                text_font(font, 13.0),
                TextColor(INK_MUTED),
            ),
            (
                RoomCodeText,
                Text::new(status.room_code.clone().unwrap_or_default()),
                text_font(font, 32.0),
                TextColor(SIGNAL),
                LetterSpacing::Px(6.0),
            ),
            (
                RoomRoleText,
                Text::new(room_role(status)),
                text_font(font, 15.0),
                TextColor(INK),
                LineHeight::RelativeToFont(1.35),
            ),
            (
                RoomCountText,
                Text::new(room_count(status)),
                text_font(font, 13.0),
                TextColor(INK_MUTED),
            ),
        ],
    )
}

fn field_label(font: &Handle<Font>, label: &'static str) -> impl Bundle {
    (
        Text::new(label),
        text_font(font, 13.0),
        TextColor(INK_MUTED),
    )
}

fn labeled_field(
    font: &Handle<Font>,
    label: &'static str,
    kind: Field,
    value: &str,
) -> impl Bundle {
    (
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            ..default()
        },
        children![field_label(font, label), field(font, kind, value)],
    )
}

/// A single-line text field built on Bevy's [`TextInput`] widget.
fn field(font: &Handle<Font>, kind: Field, value: &str) -> impl Bundle {
    let (max_characters, filter) = match kind {
        Field::Url => (
            MAX_URL_LEN,
            EditableTextFilter::new(|c| !c.is_whitespace() && !c.is_control()),
        ),
        Field::Code => (
            ROOM_CODE_LEN,
            EditableTextFilter::new(|c| c.is_ascii_alphanumeric()),
        ),
    };
    (
        Name::new(match kind {
            Field::Url => "Server address",
            Field::Code => "Room code",
        }),
        kind,
        TextInput,
        TabIndex(0),
        EditableText {
            max_characters: Some(max_characters),
            ..EditableText::new(value)
        },
        filter,
        (TextLayout::no_wrap(), text_font(font, 16.0), TextColor(INK)),
        TextCursorStyle {
            color: SIGNAL,
            selection_color: Color::srgb(0.239, 0.329, 0.447),
            unfocused_selection_color: Color::srgb(0.157, 0.212, 0.286),
            selected_text_color: Some(INK),
            selection_radius: 2.0,
        },
        Node {
            flex_grow: 1.0,
            flex_basis: px(0),
            min_width: px(0),
            height: px(TOUCH_TARGET),
            // Centers one 16 px line (1.2 line height) inside the border.
            padding: UiRect::axes(px(12), px(13)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
            ..default()
        },
        BackgroundColor(FIELD),
        BorderColor::all(FIELD_EDGE),
        Outline::new(px(2), px(2), Color::NONE),
        observe(on_field_edit),
        observe(submit_on_enter),
        observe(open_keypad_on_touch),
    )
}

/// A focusable, touch-sized button that fills its row and sends `control`'s action when activated.
fn button(
    font: &Handle<Font>,
    control: Control,
    style: ButtonStyle,
    label: &'static str,
) -> impl Bundle {
    sized_button(font, control, style, label, 1.0)
}

fn sized_button(
    font: &Handle<Font>,
    control: Control,
    style: ButtonStyle,
    label: &'static str,
    flex_grow: f32,
) -> impl Bundle {
    (
        Name::new(label),
        control,
        style,
        Button,
        Hovered::default(),
        TabIndex(0),
        Node {
            flex_grow,
            flex_shrink: 0.0,
            min_width: px(112),
            min_height: px(TOUCH_TARGET),
            padding: UiRect::axes(px(16), px(8)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        Outline::new(px(2), px(2), Color::NONE),
        observe(send_action),
        children![(
            ButtonInk,
            Text::new(label),
            text_font(font, 15.0),
            TextLayout::no_wrap(),
        )],
    )
}

fn text_font(font: &Handle<Font>, size: f32) -> TextFont {
    TextFont {
        font: font.clone().into(),
        font_size: FontSize::Px(size),
        ..default()
    }
}

fn lobby_status(status: &SessionStatus) -> String {
    if status.message.is_empty() {
        "Play solo, or create a room and share its code.".into()
    } else {
        status.message.clone()
    }
}

/// What the player can do next with the online controls.
fn online_hint(status: &SessionStatus, form: &LobbyForm) -> &'static str {
    if status.room_code.is_some() {
        if status.can_start {
            "Everyone is here. Start the match when ready."
        } else if status.player == 0 {
            "Share the room code. Start becomes available when the room is full."
        } else {
            "Waiting for the host to start the match."
        }
    } else if status.busy {
        "Connecting to the server."
    } else if !form.url_valid() {
        "Enter a server address that starts with ws:// or wss://."
    } else if !form.code.is_empty() && !form.code_valid() {
        "Room codes have six letters or digits."
    } else {
        "Create a room, or enter a code from the host to join theirs."
    }
}

const fn room_display(status: &SessionStatus) -> Display {
    if status.room_code.is_some() {
        Display::Flex
    } else {
        Display::None
    }
}

fn room_role(status: &SessionStatus) -> String {
    let number = u16::from(status.player) + 1;
    if status.player == 0 {
        format!("You are the host, player {number}.")
    } else {
        format!("You are player {number}.")
    }
}

fn room_count(status: &SessionStatus) -> String {
    format!("{} of {} players joined", status.joined, status.capacity)
}

fn send_action(
    activate: On<Activate>,
    controls: Query<&Control>,
    status: Res<SessionStatus>,
    form: Res<LobbyForm>,
    mut actions: MessageWriter<SessionAction>,
) {
    let Ok(&control) = controls.get(activate.entity) else {
        return;
    };
    if control.enabled(&status, &form) {
        actions.write(control.action(&form));
    }
}

/// Stores field edits in [`LobbyForm`], uppercasing the room code in place.
fn on_field_edit(
    change: On<TextEditChange>,
    mut fields: Query<(&Field, &mut EditableText)>,
    mut form: ResMut<LobbyForm>,
) {
    let Ok((field, mut text)) = fields.get_mut(change.event_target()) else {
        return;
    };
    let value = text.value().to_string();
    match field {
        Field::Url => {
            if form.url != value {
                form.url = value;
            }
        }
        Field::Code => {
            let code = value.to_ascii_uppercase();
            // ASCII case changes keep byte offsets, so the cursor and selection stay valid.
            if code != value && !text.is_composing() {
                text.editor_mut().set_text(&code);
            }
            if form.code != code {
                form.code = code;
            }
        }
    }
}

/// Enter in the room code field joins the room.
fn submit_on_enter(
    input: On<FocusedInput<KeyboardInput>>,
    fields: Query<(&Field, &EditableText)>,
    joins: Query<(Entity, &Control, Has<InteractionDisabled>)>,
    mut commands: Commands,
) {
    let event = &input.input;
    if event.logical_key != Key::Enter || !event.state.is_pressed() || event.repeat {
        return;
    }
    let Ok((Field::Code, text)) = fields.get(input.focused_entity) else {
        return;
    };
    if text.is_composing() {
        return;
    }
    if let Some((join, ..)) = joins
        .iter()
        .find(|&(_, &control, disabled)| control == Control::Join && !disabled)
    {
        commands.trigger(Activate { entity: join });
    }
}

/// Touch scrolling for the lobby column. Mouse wheels scroll through [`ScrollArea`].
fn scroll_on_touch_drag(
    drag: On<PointerDrag>,
    mut areas: Query<(&ComputedNode, &mut ScrollPosition), With<ScrollArea>>,
) {
    if !matches!(drag.pointer.id, PointerId::Touch(_)) {
        return;
    }
    if let Ok((node, mut scroll)) = areas.get_mut(drag.event_target()) {
        scroll_by(node, &mut scroll, drag.delta.y);
    }
}

fn scroll_by(node: &ComputedNode, scroll: &mut ScrollPosition, delta: f32) {
    let visible = node.size().y * node.inverse_scale_factor;
    let content = node.content_size().y * node.inverse_scale_factor;
    let max = (content - visible).max(0.0);
    scroll.y = (scroll.y - delta).clamp(0.0, max);
}

/// Keeps the focused control visible when tabbing through a scrolled lobby.
fn scroll_focus_into_view(focus: On<FocusGained>, mut commands: Commands) {
    commands.trigger(ScrollIntoView {
        entity: focus.entity,
    });
}

type LobbyTexts<'w, 's, T> = Query<'w, 's, &'static mut Text, With<T>>;

fn update_lobby_text(
    status: Res<SessionStatus>,
    form: Res<LobbyForm>,
    mut texts: ParamSet<(LobbyTexts<LobbyStatusText>, LobbyTexts<OnlineHintText>)>,
) {
    if !status.is_changed() && !form.is_changed() {
        return;
    }
    for mut text in &mut texts.p0() {
        text.set_if_neq(Text::new(lobby_status(&status)));
    }
    for mut text in &mut texts.p1() {
        text.set_if_neq(Text::new(online_hint(&status, &form)));
    }
}

fn update_room_panel(
    status: Res<SessionStatus>,
    mut panels: Query<&mut Node, With<RoomPanel>>,
    mut texts: ParamSet<(
        LobbyTexts<RoomCodeText>,
        LobbyTexts<RoomRoleText>,
        LobbyTexts<RoomCountText>,
    )>,
) {
    if !status.is_changed() {
        return;
    }
    let display = room_display(&status);
    for mut node in &mut panels {
        if node.display != display {
            node.display = display;
        }
    }
    for mut text in &mut texts.p0() {
        text.set_if_neq(Text::new(status.room_code.clone().unwrap_or_default()));
    }
    for mut text in &mut texts.p1() {
        text.set_if_neq(Text::new(room_role(&status)));
    }
    for mut text in &mut texts.p2() {
        text.set_if_neq(Text::new(room_count(&status)));
    }
}

/// Locks the fields while connecting or in a room.
fn update_field_availability(
    mut commands: Commands,
    status: Res<SessionStatus>,
    mut fields: Query<(Entity, &mut TextReadWriteMode, Has<InteractionDisabled>), With<Field>>,
) {
    let editable = fields_editable(&status);
    for (entity, mut mode, disabled) in &mut fields {
        let wanted = if editable {
            TextReadWriteMode::Editable
        } else {
            TextReadWriteMode::ReadOnly
        };
        mode.set_if_neq(wanted);
        set_disabled(&mut commands, entity, disabled, !editable);
    }
}

fn update_button_availability(
    mut commands: Commands,
    status: Res<SessionStatus>,
    form: Res<LobbyForm>,
    buttons: Query<(Entity, &Control, Has<InteractionDisabled>)>,
) {
    for (entity, control, disabled) in &buttons {
        set_disabled(
            &mut commands,
            entity,
            disabled,
            !control.enabled(&status, &form),
        );
    }
}

/// Fields are locked while connecting or in a room.
const fn fields_editable(status: &SessionStatus) -> bool {
    !status.busy && status.room_code.is_none()
}

fn set_disabled(commands: &mut Commands, entity: Entity, disabled: bool, disable: bool) {
    if disable && !disabled {
        commands.entity(entity).insert(InteractionDisabled);
    } else if !disable && disabled {
        commands.entity(entity).remove::<InteractionDisabled>();
    }
}

// ---------------------------------------------------------------------------
// Touch keypad
// ---------------------------------------------------------------------------

/// The open touch keypad, if any. Edits stay in the draft until Done.
#[derive(Resource, Default, Debug)]
struct TouchKeypad {
    open: Option<KeypadDraft>,
}

impl TouchKeypad {
    /// Closes the keypad. Returns the draft to store when the player chose Done.
    fn close(&mut self, close: KeypadClose) -> Option<KeypadDraft> {
        let draft = self.open.take();
        match close {
            KeypadClose::Done => draft,
            KeypadClose::Cancel => None,
        }
    }
}

/// Which keys a server address keypad shows.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
enum KeypadPage {
    #[default]
    Letters,
    Symbols,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct KeypadDraft {
    field: Field,
    value: String,
    /// Types capitals in server addresses. Room codes are always uppercase.
    shift: bool,
    page: KeypadPage,
}

impl KeypadDraft {
    fn new(field: Field, value: &str) -> Self {
        Self {
            field,
            value: value.into(),
            shift: false,
            page: KeypadPage::Letters,
        }
    }

    const fn max_len(&self) -> usize {
        match self.field {
            Field::Url => MAX_URL_LEN,
            Field::Code => ROOM_CODE_LEN,
        }
    }

    /// The character a key types right now.
    const fn resolve(&self, key: char) -> char {
        match self.field {
            Field::Code => key.to_ascii_uppercase(),
            Field::Url if self.shift => key.to_ascii_uppercase(),
            Field::Url => key,
        }
    }

    const fn accepts(&self, c: char) -> bool {
        match self.field {
            Field::Code => c.is_ascii_uppercase() || c.is_ascii_digit(),
            Field::Url => {
                if c.is_ascii_alphanumeric() {
                    return true;
                }
                let mut i = 0;
                while i < URL_SYMBOLS.len() {
                    if URL_SYMBOLS[i] == c {
                        return true;
                    }
                    i += 1;
                }
                false
            }
        }
    }

    fn press(&mut self, key: KeypadKey) {
        match key {
            KeypadKey::Char(key) => {
                let c = self.resolve(key);
                if self.accepts(c) && self.value.chars().count() < self.max_len() {
                    self.value.push(c);
                }
            }
            KeypadKey::Backspace => {
                self.value.pop();
            }
            KeypadKey::Clear => self.value.clear(),
            KeypadKey::Shift => {
                if self.field == Field::Url {
                    self.shift = !self.shift;
                }
            }
            KeypadKey::Page(page) => {
                if self.field == Field::Url {
                    self.page = page;
                }
            }
        }
    }

    /// Keys for one page, in reading order.
    fn keys(field: Field, page: KeypadPage) -> Vec<KeypadKey> {
        let letters = ('a'..='z').map(KeypadKey::Char);
        let digits = ('1'..='9').chain(['0']).map(KeypadKey::Char);
        match (field, page) {
            (Field::Code, _) => letters.chain(digits).collect(),
            (Field::Url, KeypadPage::Letters) => letters
                .chain(digits)
                .chain(URL_SYMBOLS[..4].iter().copied().map(KeypadKey::Char))
                .chain([KeypadKey::Shift, KeypadKey::Page(KeypadPage::Symbols)])
                .collect(),
            (Field::Url, KeypadPage::Symbols) => digits
                .chain(URL_SYMBOLS.iter().copied().map(KeypadKey::Char))
                .chain([KeypadKey::Page(KeypadPage::Letters)])
                .collect(),
        }
    }

    fn label(&self, key: KeypadKey) -> String {
        match key {
            KeypadKey::Char(c) => self.resolve(c).to_string(),
            KeypadKey::Backspace => "Delete".into(),
            KeypadKey::Clear => "Clear".into(),
            KeypadKey::Shift => "Shift".into(),
            KeypadKey::Page(KeypadPage::Symbols) => "#+=".into(),
            KeypadKey::Page(KeypadPage::Letters) => "abc".into(),
        }
    }

    fn preview(&self) -> (String, Color) {
        if self.value.is_empty() {
            let placeholder = match self.field {
                Field::Url => "ws://host:port",
                Field::Code => "Six letters or digits",
            };
            (placeholder.into(), INK_DISABLED)
        } else {
            (self.value.clone(), INK)
        }
    }

    fn count(&self) -> String {
        format!(
            "{} of {} characters",
            self.value.chars().count(),
            self.max_len()
        )
    }

    /// Stores the draft in the form and the matching text field.
    fn commit(self, form: &mut LobbyForm, text: &mut EditableText) {
        text.editor_mut().set_text(&self.value);
        text.queue_edit(TextEdit::TextEnd(false));
        match self.field {
            Field::Url => form.url = self.value,
            Field::Code => form.code = self.value,
        }
    }
}

/// A keypad key that edits the draft.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum KeypadKey {
    Char(char),
    Backspace,
    Clear,
    Shift,
    Page(KeypadPage),
}

/// A keypad key that closes the keypad.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum KeypadClose {
    Done,
    Cancel,
}

/// The keypad's full-screen layer. It blocks pointer input to the lobby below.
#[derive(Component)]
struct KeypadOverlay;

#[derive(Component)]
struct KeypadPreview;

#[derive(Component)]
struct KeypadCount;

/// Label of a keypad key, relabeled when Shift changes.
#[derive(Component)]
struct KeyLabel(KeypadKey);

/// A completed touch tap on an editable field opens the keypad. Opening on click, after the
/// finger lifts, keeps the same touch from pressing a key.
fn open_keypad_on_touch(
    mut click: On<PointerClick>,
    fields: Query<(
        &Field,
        &EditableText,
        &TextReadWriteMode,
        Has<InteractionDisabled>,
    )>,
    status: Res<SessionStatus>,
    mut keypad: ResMut<TouchKeypad>,
    mut focus: ResMut<InputFocus>,
) {
    if !matches!(click.pointer.id, PointerId::Touch(_)) || click.button != PointerButton::Primary {
        return;
    }
    let Ok((&field, text, mode, disabled)) = fields.get(click.entity) else {
        return;
    };
    click.propagate(false);
    if disabled
        || *mode != TextReadWriteMode::Editable
        || !fields_editable(&status)
        || keypad.open.is_some()
    {
        return;
    }
    keypad.open = Some(KeypadDraft::new(field, &text.value().to_string()));
    // The keypad replaces the platform keyboard, which Bevy can't open yet.
    focus.clear();
}

fn press_key(activate: On<Activate>, keys: Query<&KeypadKey>, mut keypad: ResMut<TouchKeypad>) {
    if let (Ok(&key), Some(draft)) = (keys.get(activate.entity), keypad.open.as_mut()) {
        draft.press(key);
    }
}

fn close_keypad(
    activate: On<Activate>,
    closes: Query<&KeypadClose>,
    status: Res<SessionStatus>,
    mut keypad: ResMut<TouchKeypad>,
    mut form: ResMut<LobbyForm>,
    mut fields: Query<(&Field, &mut EditableText)>,
) {
    let Ok(&close) = closes.get(activate.entity) else {
        return;
    };
    let Some(draft) = keypad.close(close) else {
        return;
    };
    if !fields_editable(&status) {
        return;
    }
    if let Some((_, mut text)) = fields.iter_mut().find(|(field, _)| **field == draft.field) {
        draft.commit(&mut form, &mut text);
    }
}

fn reset_keypad(mut keypad: ResMut<TouchKeypad>) {
    keypad.open = None;
}

type KeypadTexts<'w, 's, T> = Query<'w, 's, (&'static mut Text, &'static mut TextColor), With<T>>;
type KeypadLabels<'w, 's> = Query<'w, 's, (&'static KeyLabel, &'static mut Text)>;

/// Spawns, updates, and removes the keypad to match [`TouchKeypad`].
fn sync_keypad(
    mut commands: Commands,
    font: Res<GameFont>,
    status: Res<SessionStatus>,
    mut keypad: ResMut<TouchKeypad>,
    mut focus: ResMut<InputFocus>,
    overlays: Query<Entity, With<KeypadOverlay>>,
    field_entities: Query<(), With<Field>>,
    mut pages: Query<(&KeypadPage, &mut Node)>,
    mut styles: Query<(&KeypadKey, &mut ButtonStyle)>,
    mut texts: ParamSet<(
        KeypadTexts<KeypadPreview>,
        LobbyTexts<KeypadCount>,
        KeypadLabels,
    )>,
) {
    // A field that locks while the keypad is open can't take its edits.
    if keypad.open.is_some() && !fields_editable(&status) {
        keypad.open = None;
    }
    if !keypad.is_changed() {
        return;
    }
    let Some(draft) = &keypad.open else {
        for overlay in &overlays {
            commands.entity(overlay).despawn();
        }
        return;
    };
    if focus
        .get()
        .is_some_and(|entity| field_entities.contains(entity))
    {
        focus.clear();
    }
    if overlays.is_empty() {
        spawn_keypad(&mut commands, &font.0, draft);
        return;
    }

    let (preview, color) = draft.preview();
    for (mut text, mut ink) in &mut texts.p0() {
        text.set_if_neq(Text::new(preview.clone()));
        ink.set_if_neq(TextColor(color));
    }
    for mut text in &mut texts.p1() {
        text.set_if_neq(Text::new(draft.count()));
    }
    for (label, mut text) in &mut texts.p2() {
        text.set_if_neq(Text::new(draft.label(label.0)));
    }
    for (page, mut node) in &mut pages {
        let display = if *page == draft.page {
            Display::Grid
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    for (&key, mut style) in &mut styles {
        if key == KeypadKey::Shift {
            style.set_if_neq(shift_style(key, draft));
        }
    }
}

fn spawn_keypad(commands: &mut Commands, font: &Handle<Font>, draft: &KeypadDraft) {
    let pages: &[KeypadPage] = match draft.field {
        Field::Url => &[KeypadPage::Letters, KeypadPage::Symbols],
        Field::Code => &[KeypadPage::Letters],
    };
    commands
        .spawn((
            Name::new("Touch keypad"),
            KeypadOverlay,
            DespawnOnExit(AppState::Menu),
            GlobalZIndex(10),
            TabGroup::modal(),
            ScrollArea,
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                top: px(0),
                bottom: px(0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                overflow: Overflow::scroll_y(),
                ..default()
            },
            BackgroundColor(SCREEN),
            observe(scroll_on_touch_drag),
        ))
        .with_children(|overlay| {
            overlay
                .spawn(Node {
                    width: percent(100),
                    max_width: px(COLUMN_MAX_WIDTH),
                    flex_direction: FlexDirection::Column,
                    flex_shrink: 0.0,
                    // Six 48 px keys and five 3 px gaps fit inside 320 px.
                    padding: UiRect::axes(px(8), px(12)),
                    row_gap: px(8),
                    ..default()
                })
                .with_children(|column| {
                    column.spawn(keypad_heading(font, draft));
                    column.spawn(keypad_preview(font, draft));
                    for &page in pages {
                        spawn_key_grid(column, font, draft, page);
                    }
                    spawn_keypad_controls(column, font, draft);
                });
        });
}

/// Field name and character count.
fn keypad_heading(font: &Handle<Font>, draft: &KeypadDraft) -> impl Bundle {
    let title = match draft.field {
        Field::Url => "Server address",
        Field::Code => "Room code",
    };
    (
        Node {
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Baseline,
            column_gap: px(8),
            ..default()
        },
        children![
            (Text::new(title), text_font(font, 18.0), TextColor(INK)),
            (
                KeypadCount,
                Text::new(draft.count()),
                text_font(font, 13.0),
                TextColor(INK_MUTED),
            ),
        ],
    )
}

/// The draft value, wrapped so long addresses stay readable.
fn keypad_preview(font: &Handle<Font>, draft: &KeypadDraft) -> impl Bundle {
    let (preview, color) = draft.preview();
    (
        Node {
            min_height: px(TOUCH_TARGET),
            padding: UiRect::axes(px(12), px(13)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
            ..default()
        },
        BackgroundColor(FIELD),
        BorderColor::all(SIGNAL),
        children![(
            KeypadPreview,
            Text::new(preview),
            text_font(font, 16.0),
            TextColor(color),
            TextLayout::linebreak(LineBreak::AnyCharacter),
        )],
    )
}

fn spawn_key_grid(
    column: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    draft: &KeypadDraft,
    page: KeypadPage,
) {
    column
        .spawn((
            page,
            Node {
                display: if page == draft.page {
                    Display::Grid
                } else {
                    Display::None
                },
                grid_template_columns: RepeatedGridTrack::flex(KEYPAD_COLUMNS, 1.0),
                column_gap: px(3),
                row_gap: px(3),
                ..default()
            },
        ))
        .with_children(|grid| {
            for key in KeypadDraft::keys(draft.field, page) {
                grid.spawn((
                    key_button(
                        font,
                        shift_style(key, draft),
                        KeyLabel(key),
                        draft.label(key),
                    ),
                    key,
                    observe(press_key),
                ));
            }
        });
}

/// Clear, Delete, Cancel, and Done.
fn spawn_keypad_controls(
    column: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    draft: &KeypadDraft,
) {
    column
        .spawn(Node {
            display: Display::Grid,
            grid_template_columns: RepeatedGridTrack::flex(4, 1.0),
            column_gap: px(6),
            margin: UiRect::top(px(4)),
            ..default()
        })
        .with_children(|row| {
            for key in [KeypadKey::Clear, KeypadKey::Backspace] {
                row.spawn((
                    key_button(
                        font,
                        ButtonStyle::Secondary,
                        KeyLabel(key),
                        draft.label(key),
                    ),
                    key,
                    observe(press_key),
                ));
            }
            for (close, style, label) in [
                (KeypadClose::Cancel, ButtonStyle::Secondary, "Cancel"),
                (KeypadClose::Done, ButtonStyle::Primary, "Done"),
            ] {
                row.spawn((
                    key_button(font, style, Name::new(label), label.to_string()),
                    close,
                    observe(close_keypad),
                ));
            }
        });
}

/// Shift is filled while it is on.
fn shift_style(key: KeypadKey, draft: &KeypadDraft) -> ButtonStyle {
    if key == KeypadKey::Shift && draft.shift {
        ButtonStyle::Primary
    } else {
        ButtonStyle::Secondary
    }
}

/// A 48 px keypad button. `tag` goes on the label so Shift can relabel it.
fn key_button(
    font: &Handle<Font>,
    style: ButtonStyle,
    tag: impl Component,
    label: String,
) -> impl Bundle {
    (
        style,
        Button,
        Hovered::default(),
        Node {
            min_width: px(TOUCH_TARGET),
            min_height: px(TOUCH_TARGET),
            padding: UiRect::horizontal(px(2)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        Outline::new(px(2), px(2), Color::NONE),
        children![(
            ButtonInk,
            tag,
            Text::new(label),
            text_font(font, 17.0),
            TextLayout::no_wrap(),
        )],
    )
}

// ---------------------------------------------------------------------------
// Session bar
// ---------------------------------------------------------------------------

fn spawn_session_bar(mut commands: Commands, font: Res<GameFont>, status: Res<SessionStatus>) {
    let font = &font.0;
    commands.spawn((
        Name::new("Session bar"),
        DespawnOnExit(AppState::InGame),
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
            ..default()
        },
        BackgroundColor(BAR),
        BorderColor::all(EDGE),
        children![
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    flex_basis: px(0),
                    min_width: px(0),
                    max_height: percent(100),
                    row_gap: px(2),
                    overflow: Overflow::clip(),
                    ..default()
                },
                children![
                    (
                        BarTitleText,
                        Text::new(bar_title(&status)),
                        text_font(font, 15.0),
                        TextColor(INK),
                        TextLayout::no_wrap(),
                    ),
                    (
                        BarMessageText,
                        Text::new(status.message.clone()),
                        text_font(font, 12.0),
                        TextColor(INK_MUTED),
                        LineHeight::RelativeToFont(1.2),
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
                    bar_button(
                        font,
                        Control::SaveReplay,
                        ButtonStyle::Secondary,
                        "Save replay"
                    ),
                    bar_button(font, Control::Leave, ButtonStyle::Secondary, "Leave"),
                ],
            ),
        ],
    ));
}

/// A compact session bar button. It isn't tab-focusable, so in-game keys never activate it.
fn bar_button(
    font: &Handle<Font>,
    control: Control,
    style: ButtonStyle,
    label: &'static str,
) -> impl Bundle {
    (
        Name::new(label),
        control,
        style,
        Button,
        Hovered::default(),
        Node {
            min_width: px(TOUCH_TARGET),
            height: px(TOUCH_TARGET),
            padding: UiRect::horizontal(px(10)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        Outline::new(px(2), px(2), Color::NONE),
        observe(send_action),
        children![(
            ButtonInk,
            Text::new(label),
            text_font(font, 14.0),
            TextLayout::no_wrap(),
        )],
    )
}

fn bar_title(status: &SessionStatus) -> String {
    if status.playback {
        "Watching replay".into()
    } else if let Some(code) = &status.room_code {
        format!("Room {code}, player {}", u16::from(status.player) + 1)
    } else {
        "Solo match".into()
    }
}

fn update_bar_text(
    status: Res<SessionStatus>,
    mut texts: ParamSet<(LobbyTexts<BarTitleText>, LobbyTexts<BarMessageText>)>,
) {
    if !status.is_changed() {
        return;
    }
    for mut text in &mut texts.p0() {
        text.set_if_neq(Text::new(bar_title(&status)));
    }
    for mut text in &mut texts.p1() {
        text.set_if_neq(Text::new(status.message.clone()));
    }
}

// ---------------------------------------------------------------------------
// Looks
// ---------------------------------------------------------------------------

const fn button_look(style: ButtonStyle, hovered: bool, pressed: bool, disabled: bool) -> Look {
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

const fn field_look(focused: bool, disabled: bool) -> Look {
    if disabled {
        Look {
            fill: FIELD,
            edge: Color::srgb(0.157, 0.212, 0.286),
            ink: INK_DISABLED,
        }
    } else if focused {
        Look {
            fill: FIELD,
            edge: SIGNAL,
            ink: INK,
        }
    } else {
        Look {
            fill: FIELD,
            edge: FIELD_EDGE,
            ink: INK,
        }
    }
}

type ButtonState<'a> = (
    Entity,
    &'a ButtonStyle,
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
);

/// Colors buttons and fields from their state. Text nodes also carry a [`BackgroundColor`],
/// so labels only ever get a [`TextColor`].
fn update_looks(
    focus: Res<InputFocus>,
    focus_visible: Res<InputFocusVisible>,
    mut buttons: Query<ButtonState, Without<Field>>,
    mut fields: Query<FieldState, (With<Field>, Without<ButtonStyle>)>,
    mut inks: Query<&mut TextColor, (With<ButtonInk>, Without<Field>)>,
) {
    let focused = focus.get();
    for (entity, &style, hovered, pressed, disabled, mut fill, mut edge, mut outline, children) in
        &mut buttons
    {
        let look = button_look(style, hovered.get(), pressed, disabled);
        fill.set_if_neq(BackgroundColor(look.fill));
        edge.set_if_neq(BorderColor::all(look.edge));
        let ring = if focus_visible.0 && focused == Some(entity) {
            SIGNAL
        } else {
            Color::NONE
        };
        if outline.color != ring {
            outline.color = ring;
        }
        for &child in children {
            if let Ok(mut ink) = inks.get_mut(child) {
                ink.set_if_neq(TextColor(look.ink));
            }
        }
    }
    for (entity, disabled, mut fill, mut edge, mut ink) in &mut fields {
        let look = field_look(focused == Some(entity), disabled);
        fill.set_if_neq(BackgroundColor(look.fill));
        edge.set_if_neq(BorderColor::all(look.edge));
        ink.set_if_neq(TextColor(look.ink));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> SessionStatus {
        SessionStatus {
            message: String::new(),
            room_code: None,
            player: 0,
            joined: 0,
            capacity: 0,
            can_start: false,
            has_replay: false,
            running: false,
            playback: false,
            busy: false,
        }
    }

    fn form(code: &str) -> LobbyForm {
        LobbyForm {
            code: code.into(),
            ..default()
        }
    }

    #[test]
    fn idle_lobby_allows_solo_host_and_load() {
        let status = status();
        let form = form("");
        assert!(Control::Solo.enabled(&status, &form));
        assert!(Control::Host.enabled(&status, &form));
        assert!(Control::LoadReplay.enabled(&status, &form));
        for control in [
            Control::Join,
            Control::Start,
            Control::Leave,
            Control::ReplayLast,
            Control::SaveReplay,
        ] {
            assert!(!control.enabled(&status, &form), "{control:?}");
        }
    }

    #[test]
    fn join_needs_a_full_code_and_a_websocket_address() {
        let status = status();
        assert!(Control::Join.enabled(&status, &form("K7Q2PX")));
        assert!(!Control::Join.enabled(&status, &form("K7Q2P")));
        assert!(!Control::Join.enabled(&status, &form("k7q2px")));
        let mut http = form("K7Q2PX");
        http.url = "http://127.0.0.1:9001".into();
        assert!(!Control::Join.enabled(&status, &http));
        assert!(!Control::Host.enabled(&status, &http));
        let mut bare = form("K7Q2PX");
        bare.url = " ws:// ".into();
        assert!(!Control::Host.enabled(&status, &bare));
        bare.url = " wss://relay.example ".into();
        assert!(Control::Host.enabled(&status, &bare));
    }

    #[test]
    fn busy_or_in_room_blocks_new_connections() {
        let form = form("K7Q2PX");
        let mut busy = status();
        busy.busy = true;
        for control in [
            Control::Solo,
            Control::Host,
            Control::Join,
            Control::LoadReplay,
        ] {
            assert!(!control.enabled(&busy, &form), "{control:?}");
        }
        assert!(Control::Leave.enabled(&busy, &form));

        let mut room = status();
        room.room_code = Some("K7Q2PX".into());
        assert!(!Control::Host.enabled(&room, &form));
        assert!(!Control::Join.enabled(&room, &form));
        assert!(Control::Solo.enabled(&room, &form));
        assert!(Control::Leave.enabled(&room, &form));
        assert!(!Control::Start.enabled(&room, &form));
        room.can_start = true;
        assert!(Control::Start.enabled(&room, &form));
    }

    #[test]
    fn leave_and_replays_follow_session_state() {
        let form = form("");
        let mut running = status();
        running.running = true;
        assert!(Control::Leave.enabled(&running, &form));
        running.has_replay = true;
        assert!(Control::ReplayLast.enabled(&running, &form));
        assert!(Control::SaveReplay.enabled(&running, &form));
    }

    #[test]
    fn actions_carry_trimmed_address_and_code() {
        let mut form = form("K7Q2PX");
        form.url = "  ws://10.0.0.2:9001 ".into();
        assert!(matches!(
            Control::Host.action(&form),
            SessionAction::Host { url } if url == "ws://10.0.0.2:9001"
        ));
        assert!(matches!(
            Control::Join.action(&form),
            SessionAction::Join { url, code } if url == "ws://10.0.0.2:9001" && code == "K7Q2PX"
        ));
    }

    #[test]
    fn labels_name_the_host_and_player_numbers() {
        let mut status = status();
        status.room_code = Some("K7Q2PX".into());
        status.joined = 1;
        status.capacity = 2;
        assert_eq!(room_role(&status), "You are the host, player 1.");
        assert_eq!(room_count(&status), "1 of 2 players joined");
        assert_eq!(bar_title(&status), "Room K7Q2PX, player 1");
        status.player = 1;
        assert_eq!(room_role(&status), "You are player 2.");
        status.playback = true;
        assert_eq!(bar_title(&status), "Watching replay");
    }

    fn press_all(draft: &mut KeypadDraft, keys: &str) {
        for c in keys.chars() {
            draft.press(KeypadKey::Char(c));
        }
    }

    #[test]
    fn code_keypad_types_six_uppercase_characters() {
        let mut draft = KeypadDraft::new(Field::Code, "");
        press_all(&mut draft, "k7q2px9");
        assert_eq!(draft.value, "K7Q2PX");
        draft.press(KeypadKey::Backspace);
        draft.press(KeypadKey::Char('/'));
        draft.press(KeypadKey::Shift);
        assert_eq!(draft.value, "K7Q2P");
        assert!(!draft.shift);
        draft.press(KeypadKey::Clear);
        assert_eq!(draft.value, "");
        draft.press(KeypadKey::Backspace);
        assert_eq!(draft.value, "");
    }

    #[test]
    fn url_keypad_shifts_case_and_types_symbols() {
        let mut draft = KeypadDraft::new(Field::Url, "");
        press_all(&mut draft, "wss://relay.example:9001/");
        draft.press(KeypadKey::Shift);
        press_all(&mut draft, "ab");
        draft.press(KeypadKey::Shift);
        press_all(&mut draft, "c?x=1&y=%2b#[] ");
        assert_eq!(draft.value, "wss://relay.example:9001/ABc?x=1&y=%2b#[]");
        draft.press(KeypadKey::Page(KeypadPage::Symbols));
        assert_eq!(draft.page, KeypadPage::Symbols);
    }

    #[test]
    fn url_keypad_stops_at_the_length_limit() {
        let mut draft = KeypadDraft::new(Field::Url, &"a".repeat(MAX_URL_LEN - 1));
        press_all(&mut draft, "bc");
        assert_eq!(draft.value.len(), MAX_URL_LEN);
        assert!(draft.value.ends_with("ab"));
    }

    #[test]
    fn every_key_on_every_page_types_an_accepted_character() {
        for field in [Field::Url, Field::Code] {
            for page in [KeypadPage::Letters, KeypadPage::Symbols] {
                let keys = KeypadDraft::keys(field, page);
                assert!(
                    keys.len().is_multiple_of(usize::from(KEYPAD_COLUMNS)),
                    "{field:?} {page:?}"
                );
                let mut draft = KeypadDraft::new(field, "");
                for key in keys {
                    if let KeypadKey::Char(c) = key {
                        assert!(draft.accepts(draft.resolve(c)), "{c}");
                    }
                }
                draft.shift = true;
                assert!(draft.accepts(draft.resolve('q')));
            }
        }
    }

    #[test]
    fn cancel_discards_and_done_commits_the_draft() {
        let mut form = form("");
        let mut text = EditableText::new("");

        let mut keypad = TouchKeypad {
            open: Some(KeypadDraft::new(Field::Code, "")),
        };
        press_all(keypad.open.as_mut().unwrap(), "abc");
        assert_eq!(keypad.close(KeypadClose::Cancel), None);
        assert!(keypad.open.is_none());
        assert_eq!(form.code, "");

        keypad.open = Some(KeypadDraft::new(Field::Code, "AB"));
        press_all(keypad.open.as_mut().unwrap(), "c12x");
        keypad
            .close(KeypadClose::Done)
            .unwrap()
            .commit(&mut form, &mut text);
        assert!(keypad.open.is_none());
        assert_eq!(form.code, "ABC12X");
        assert_eq!(text.value().to_string(), "ABC12X");
        assert!(form.code_valid());

        let mut url = EditableText::new(DEFAULT_RELAY_URL);
        let mut draft = KeypadDraft::new(Field::Url, DEFAULT_RELAY_URL);
        for _ in 0..4 {
            draft.press(KeypadKey::Backspace);
        }
        press_all(&mut draft, "7000");
        draft.commit(&mut form, &mut url);
        assert_eq!(form.url, "ws://127.0.0.1:7000");
        assert_eq!(url.value().to_string(), "ws://127.0.0.1:7000");
        assert_eq!(form.code, "ABC12X");
    }
}
