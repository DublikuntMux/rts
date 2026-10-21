//! Touch keypad for menu text fields.
//!
//! Bevy's text input cannot open the platform's on-screen keyboard yet, so a completed touch tap
//! on an editable field opens this keypad instead. Mouse and physical keyboards keep using the
//! text field directly. Edits stay in a draft until Done.

use bevy::{
    input_focus::{InputFocus, tab_navigation::TabGroup},
    picking::{
        events::PointerClick,
        hover::Hovered,
        pointer::{PointerButton, PointerId},
    },
    prelude::*,
    text::{EditableText, EditableTextFilter, TextEdit, TextReadWriteMode},
    ui::InteractionDisabled,
    ui_widgets::{Activate, Button, ScrollArea},
};

use super::{
    MAX_URL_LEN, MenuForm, ROOM_CODE_LEN, scroll_on_touch_drag, store_name,
    widgets::{
        ButtonInk, FIELD, Field, INK, INK_DISABLED, INK_MUTED, SCREEN, SIGNAL, TOUCH_TARGET, Tone,
        text,
    },
};
use crate::{game::AppState, settings::Settings};

/// Punctuation on the touch keypad for server addresses.
const URL_SYMBOLS: [char; 13] = [
    ':', '/', '.', '-', '_', '?', '&', '=', '%', '+', '[', ']', '#',
];
/// Punctuation on the touch keypad for player names.
const NAME_SYMBOLS: [char; 5] = [' ', '-', '_', '.', '\''];
/// Keypad keys per row. Six 48 px keys fit a 320 px screen.
const KEYPAD_COLUMNS: u16 = 6;
/// The keypad column never grows wider than this.
const COLUMN_MAX_WIDTH: f32 = 440.0;

/// Characters a field accepts from a physical keyboard.
pub fn filter(field: Field) -> EditableTextFilter {
    match field {
        Field::Url => EditableTextFilter::new(|c| !c.is_whitespace() && !c.is_control()),
        Field::Code => EditableTextFilter::new(|c| c.is_ascii_alphanumeric()),
        Field::Name => EditableTextFilter::new(|c| !c.is_control()),
    }
}

/// The open touch keypad, if any.
#[derive(Resource, Default, Debug)]
pub struct TouchKeypad {
    pub open: Option<KeypadDraft>,
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

/// Which keys an address keypad shows.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeypadPage {
    #[default]
    Letters,
    Symbols,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeypadDraft {
    field: Field,
    value: String,
    /// Types capitals in addresses and names. Room codes are always uppercase.
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
            Field::Name => rts_net::MAX_NAME_CHARS,
        }
    }

    /// The character a key types right now.
    const fn resolve(&self, key: char) -> char {
        match self.field {
            Field::Code => key.to_ascii_uppercase(),
            Field::Url | Field::Name if self.shift => key.to_ascii_uppercase(),
            Field::Url | Field::Name => key,
        }
    }

    fn accepts(&self, c: char) -> bool {
        match self.field {
            Field::Code => c.is_ascii_uppercase() || c.is_ascii_digit(),
            Field::Url => c.is_ascii_alphanumeric() || URL_SYMBOLS.contains(&c),
            Field::Name => c.is_ascii_alphanumeric() || NAME_SYMBOLS.contains(&c),
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
                if self.field != Field::Code {
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

    /// Pages this field's keypad has.
    const fn pages(field: Field) -> &'static [KeypadPage] {
        match field {
            Field::Url => &[KeypadPage::Letters, KeypadPage::Symbols],
            Field::Code | Field::Name => &[KeypadPage::Letters],
        }
    }

    /// Keys for one page, in reading order.
    fn keys(field: Field, page: KeypadPage) -> Vec<KeypadKey> {
        let letters = ('a'..='z').map(KeypadKey::Char);
        let digits = ('1'..='9').chain(['0']).map(KeypadKey::Char);
        match (field, page) {
            (Field::Code, _) => letters.chain(digits).collect(),
            (Field::Name, _) => letters
                .chain(digits)
                .chain(NAME_SYMBOLS.iter().copied().map(KeypadKey::Char))
                .chain([KeypadKey::Shift])
                .collect(),
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
            KeypadKey::Char(' ') => "Space".into(),
            KeypadKey::Char(c) => self.resolve(c).to_string(),
            KeypadKey::Backspace => "Delete".into(),
            KeypadKey::Clear => "Clear".into(),
            KeypadKey::Shift => "Shift".into(),
            KeypadKey::Page(KeypadPage::Symbols) => "#+=".into(),
            KeypadKey::Page(KeypadPage::Letters) => "abc".into(),
        }
    }

    const fn title(&self) -> &'static str {
        match self.field {
            Field::Url => "Server address",
            Field::Code => "Invite code",
            Field::Name => "Player name",
        }
    }

    fn preview(&self) -> (String, Color) {
        if self.value.is_empty() {
            let placeholder = match self.field {
                Field::Url => "ws://host:port",
                Field::Code => "Six letters or digits",
                Field::Name => "Your name",
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

    /// Stores the draft in the form, the settings, and the matching text field.
    fn commit(self, form: &mut MenuForm, settings: &mut Settings, text: &mut EditableText) {
        text.editor_mut().set_text(&self.value);
        text.queue_edit(TextEdit::TextEnd(false));
        match self.field {
            Field::Url => form.url = self.value,
            Field::Code => form.code = self.value,
            Field::Name => store_name(self.value, form, settings),
        }
    }
}

/// A keypad key that edits the draft.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeypadKey {
    Char(char),
    Backspace,
    Clear,
    Shift,
    Page(KeypadPage),
}

/// A keypad key that closes the keypad.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeypadClose {
    Done,
    Cancel,
}

/// The keypad's full-screen layer. It blocks pointer input to the menu below.
#[derive(Component, Clone, Default)]
pub struct KeypadOverlay;

#[derive(Component, Clone, Default)]
pub struct KeypadPreview;

#[derive(Component, Clone, Default)]
pub struct KeypadCount;

/// A keypad key whose label changes with Shift.
#[derive(Component, Clone, Copy)]
pub struct KeyLabel(KeypadKey);

/// A completed touch tap on an editable field opens the keypad. Opening on click, after the
/// finger lifts, keeps the same touch from pressing a key.
pub fn open_on_touch(
    mut click: On<PointerClick>,
    fields: Query<(
        &Field,
        &EditableText,
        &TextReadWriteMode,
        Has<InteractionDisabled>,
    )>,
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
    if disabled || *mode != TextReadWriteMode::Editable || keypad.open.is_some() {
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
    mut keypad: ResMut<TouchKeypad>,
    mut form: ResMut<MenuForm>,
    mut settings: ResMut<Settings>,
    mut fields: Query<(&Field, &mut EditableText)>,
) {
    let Ok(&close) = closes.get(activate.entity) else {
        return;
    };
    let Some(draft) = keypad.close(close) else {
        return;
    };
    if let Some((_, mut text)) = fields.iter_mut().find(|(field, _)| **field == draft.field) {
        draft.commit(&mut form, &mut settings, &mut text);
    }
}

pub fn reset(mut keypad: ResMut<TouchKeypad>) {
    keypad.open = None;
}

type PreviewTexts<'w, 's> =
    Query<'w, 's, (&'static mut Text, &'static mut TextColor), With<KeypadPreview>>;
type CountTexts<'w, 's> = Query<'w, 's, &'static mut Text, With<KeypadCount>>;
type KeyLabels<'w, 's> = Query<'w, 's, &'static mut Text, With<ButtonInk>>;

/// Spawns, updates, and removes the keypad to match [`TouchKeypad`].
pub fn sync(
    mut commands: Commands,
    mut keypad: ResMut<TouchKeypad>,
    mut focus: ResMut<InputFocus>,
    overlays: Query<Entity, With<KeypadOverlay>>,
    fields: Query<&Field>,
    mut pages: Query<(&KeypadPage, &mut Node)>,
    mut tones: Query<(&KeypadKey, &mut Tone)>,
    labels: Query<(&KeyLabel, &Children)>,
    mut texts: ParamSet<(PreviewTexts, CountTexts, KeyLabels)>,
) {
    // A field that disappears while the keypad is open can't take its edits.
    if let Some(draft) = &keypad.open
        && !fields.iter().any(|field| *field == draft.field)
        && !overlays.is_empty()
    {
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
    if focus.get().is_some_and(|entity| fields.contains(entity)) {
        focus.clear();
    }
    if overlays.is_empty() {
        commands.spawn_scene(keypad_scene(draft));
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
    for (label, children) in &labels {
        for &child in children {
            if let Ok(mut text) = texts.p2().get_mut(child) {
                text.set_if_neq(Text::new(draft.label(label.0)));
            }
        }
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
    for (&key, mut tone) in &mut tones {
        if key == KeypadKey::Shift {
            tone.set_if_neq(shift_tone(draft));
        }
    }
}

fn keypad_scene(draft: &KeypadDraft) -> impl Scene {
    let (preview, color) = draft.preview();
    let grids: Vec<_> = KeypadDraft::pages(draft.field)
        .iter()
        .map(|&page| key_grid(draft, page))
        .collect();
    let modal = TabGroup::modal();
    let title = draft.title();
    let count = draft.count();
    let columns: Vec<RepeatedGridTrack> = RepeatedGridTrack::flex(4, 1.0);
    let controls = vec![
        key_button(draft, KeypadKey::Clear),
        key_button(draft, KeypadKey::Backspace),
    ];
    bsn! {
        #TouchKeypad
        KeypadOverlay
        DespawnOnExit<AppState>(AppState::Menu)
        GlobalZIndex(10)
        modal
        ScrollArea
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            overflow: Overflow::scroll_y(),
        }
        BackgroundColor(SCREEN)
        on(scroll_on_touch_drag)
        Children [
            Node {
                width: percent(100),
                max_width: px(COLUMN_MAX_WIDTH),
                flex_direction: FlexDirection::Column,
                flex_shrink: 0.0,
                // Six 48 px keys and five 3 px gaps fit inside 320 px.
                padding: UiRect::axes(px(8), px(12)),
                row_gap: px(8),
            }
            Children [
                Node {
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Baseline,
                    column_gap: px(8),
                }
                Children [
                    @text(title, 18.0, INK)
                    --
                    @text(count, 13.0, INK_MUTED) KeypadCount
                ]
                --
                Node {
                    min_height: px(TOUCH_TARGET),
                    padding: UiRect::axes(px(12), px(13)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(4)),
                }
                BackgroundColor(FIELD)
                BorderColor::all(SIGNAL)
                Children [
                    @text(preview, 16.0, color)
                    KeypadPreview
                    TextLayout { linebreak: LineBreak::AnyCharacter }
                ]
                --
                {grids}
                --
                Node {
                    display: Display::Grid,
                    grid_template_columns: columns,
                    column_gap: px(6),
                    margin: UiRect::top(px(4)),
                }
                Children [
                    {controls}
                    --
                    @close_button("Cancel", KeypadClose::Cancel, Tone::Secondary)
                    --
                    @close_button("Done", KeypadClose::Done, Tone::Primary)
                ]
            ]
        ]
    }
}

fn key_grid(draft: &KeypadDraft, page: KeypadPage) -> impl Scene {
    let display = if page == draft.page {
        Display::Grid
    } else {
        Display::None
    };
    let keys: Vec<_> = KeypadDraft::keys(draft.field, page)
        .into_iter()
        .map(|key| key_button(draft, key))
        .collect();
    let columns: Vec<RepeatedGridTrack> = RepeatedGridTrack::flex(KEYPAD_COLUMNS, 1.0);
    bsn! {
        page
        Node {
            display: display,
            grid_template_columns: columns,
            column_gap: px(3),
            row_gap: px(3),
        }
        Children [ {keys} ]
    }
}

/// Shift is filled while it is on.
const fn shift_tone(draft: &KeypadDraft) -> Tone {
    if draft.shift {
        Tone::Primary
    } else {
        Tone::Secondary
    }
}

/// A 48 px button shell for the keypad.
fn key_shell(label: String, tone: Tone) -> impl Scene {
    bsn! {
        tone
        Button
        Hovered
        Node {
            min_width: px(TOUCH_TARGET),
            min_height: px(TOUCH_TARGET),
            padding: UiRect::horizontal(px(2)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
        }
        Outline { width: px(2), offset: px(2), color: Color::NONE }
        Children [
            @text(label, 17.0, INK)
            ButtonInk
            TextLayout { linebreak: LineBreak::NoWrap }
        ]
    }
}

fn key_button(draft: &KeypadDraft, key: KeypadKey) -> impl Scene {
    let tone = if key == KeypadKey::Shift {
        shift_tone(draft)
    } else {
        Tone::Secondary
    };
    let label = draft.label(key);
    let tag = KeyLabel(key);
    bsn! {
        @key_shell(label, tone)
        key
        tag
        on(press_key)
    }
}

fn close_button(label: &'static str, close: KeypadClose, tone: Tone) -> impl Scene {
    bsn! {
        @key_shell(label.into(), tone)
        close
        on(close_keypad)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn name_keypad_types_spaces_and_stops_at_the_name_limit() {
        let mut draft = KeypadDraft::new(Field::Name, "");
        draft.press(KeypadKey::Shift);
        press_all(&mut draft, "a");
        draft.press(KeypadKey::Shift);
        press_all(&mut draft, "da l'o_v-e.:/");
        assert_eq!(draft.value, "Ada l'o_v-e.");
        press_all(&mut draft, &"x".repeat(30));
        assert_eq!(draft.value.chars().count(), rts_net::MAX_NAME_CHARS);
        assert_eq!(draft.label(KeypadKey::Char(' ')), "Space");
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
        for field in [Field::Url, Field::Code, Field::Name] {
            for &page in KeypadDraft::pages(field) {
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
        let mut form = MenuForm::default();
        let mut settings = Settings::default();
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
            .commit(&mut form, &mut settings, &mut text);
        assert_eq!(form.code, "ABC12X");
        assert_eq!(text.value().to_string(), "ABC12X");
        assert!(form.code_valid());

        let mut name = EditableText::new("Player");
        let mut draft = KeypadDraft::new(Field::Name, "Player");
        draft.press(KeypadKey::Clear);
        press_all(&mut draft, "zoe");
        draft.commit(&mut form, &mut settings, &mut name);
        assert_eq!(settings.name, "zoe");
        assert_eq!(form.name.as_deref(), Some("zoe"));

        // An invalid name stays in the form but does not replace the stored one.
        let mut draft = KeypadDraft::new(Field::Name, "");
        press_all(&mut draft, " ");
        draft.commit(&mut form, &mut settings, &mut name);
        assert_eq!(settings.name, "zoe");
        assert_eq!(form.name.as_deref(), Some(" "));
    }
}
