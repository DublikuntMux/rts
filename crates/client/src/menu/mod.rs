//! The main menu: a title bar, a sidebar with the five sections, and one page at a time.
//!
//! Pages are Bevy scenes built from [`widgets`]. A page is a pure function of the menu model
//! ([`View`]); when the parts of the model a page shows change, the page is spawned again.
//! Smaller changes, such as messages, availability, and selection highlights, update in place.
//! The menu only reads [`SessionStatus`] and sends [`SessionAction`]s; the session does the work.

#[cfg(target_arch = "wasm32")]
mod browser_input;
mod pages;
mod previews;
pub mod widgets;

use bevy::{
    app::AppExit,
    clipboard::Clipboard,
    ecs::system::SystemParam,
    input::keyboard::{Key, KeyboardInput},
    input_focus::{
        FocusCause, FocusGained, FocusedInput, InputFocus, tab_navigation::TabNavigationPlugin,
    },
    picking::{events::PointerDrag, pointer::PointerId},
    prelude::*,
    text::{EditableText, TextEditChange},
    ui::InteractionDisabled,
    ui_widgets::{Activate, ScrollArea, ScrollIntoView},
    window::PrimaryWindow,
};
use rts_net::{MAX_NAME_CHARS, RoomInfo};

use self::widgets::Field;
use crate::{
    crash::CrashReport,
    game::{AppState, MapCatalog},
    hud::MoveTargeting,
    library::{Library, Record, Shelf},
    pause_menu::PauseMenu,
    session::{Lobby, MatchKind, Server, SessionAction, SessionActions, SessionStatus},
    settings::Settings,
};

/// Relay address shown until the player types another one.
const DEFAULT_SERVER_URL: &str = "ws://127.0.0.1:9001";
pub const ROOM_CODE_LEN: usize = 6;
pub const MAX_URL_LEN: usize = 256;
/// Below this logical width navigation moves into a row above the page.
const COMPACT_WIDTH: f32 = 760.0;

/// Menu actions finish before camera and world input consume this frame's controls.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MenuActions;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(target_arch = "wasm32")]
        browser_input::install(app);
        if !app.is_plugin_added::<TabNavigationPlugin>() {
            app.add_plugins(TabNavigationPlugin);
        }
        app.init_resource::<Menu>()
            .init_resource::<MenuForm>()
            .init_resource::<PauseMenu>()
            .init_resource::<Rendered>()
            .init_resource::<Clipboard>()
            .init_resource::<CrashReport>()
            .init_resource::<previews::PreviewCache>()
            .add_message::<Action>()
            .add_systems(
                OnEnter(AppState::Menu),
                (close_dialogs, open_crash_dialog).chain(),
            )
            .add_systems(OnEnter(AppState::InGame), close_dialogs)
            .add_systems(OnExit(AppState::Menu), forget_render)
            .add_systems(
                Update,
                (
                    (track_compact, back_on_escape)
                        .chain()
                        .run_if(in_state(AppState::Menu))
                        .before(perform),
                    perform
                        .in_set(MenuActions)
                        .run_if(not(in_state(AppState::Loading)))
                        .before(SessionActions),
                    (follow_session, render, restore_focus)
                        .chain()
                        .run_if(in_state(AppState::Menu))
                        .run_if(staying)
                        .after(SessionActions),
                    render_dialog
                        .run_if(not(in_state(AppState::Loading)))
                        .run_if(staying)
                        .after(SessionActions),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    widgets::update_bindings,
                    widgets::update_availability,
                    widgets::update_looks,
                    widgets::spin,
                    widgets::fit_previews,
                    update_notice,
                )
                    .chain()
                    .run_if(not(in_state(AppState::Loading))),
            );
    }
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Singleplayer,
    Multiplayer,
    Replays,
    Settings,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SoloPage {
    #[default]
    Home,
    NewGame,
    LoadGame,
}

/// What the multiplayer section shows, derived from the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnlinePage {
    Connect,
    Connecting,
    Rooms,
    Lobby,
}

impl OnlinePage {
    const fn of(status: &SessionStatus) -> Self {
        if status.lobby.is_some() {
            Self::Lobby
        } else if status.connected() {
            Self::Rooms
        } else if status.connecting() {
            Self::Connecting
        } else {
            Self::Connect
        }
    }
}

/// A replay browser row: the last match played, or a stored record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayPick {
    Last,
    Stored(String),
}

/// Filters and the selection of a save or replay browser.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Browse {
    /// Show only records on this map.
    pub map: Option<String>,
    pub oldest_first: bool,
    pub selected: Option<ReplayPick>,
}

impl Browse {
    /// Records on the chosen map, in the chosen order.
    pub fn filter<'a>(&self, records: &'a [Record]) -> Vec<&'a Record> {
        let mut shown: Vec<_> = records
            .iter()
            .filter(|record| self.map.as_ref().is_none_or(|map| &record.meta.map == map))
            .collect();
        if self.oldest_first {
            shown.reverse();
        }
        shown
    }

    pub fn selected_id(&self) -> Option<&str> {
        match &self.selected {
            Some(ReplayPick::Stored(id)) => Some(id),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dropdown {
    Map(Shelf),
    Order(Shelf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirm {
    Delete(Shelf, String),
    LeaveRoom,
    /// Leaving a match that would lose progress.
    LeaveMatch,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    JoinByCode,
    CreateRoom,
    Confirm(Confirm),
    /// A report left by a previous run.
    CrashReport,
}

/// Menu navigation and choices. Kept across visits to the menu.
#[derive(Resource, Debug)]
pub struct Menu {
    pub section: Section,
    pub solo: SoloPage,
    /// Index into the [`MapCatalog`].
    pub map: usize,
    pub saves: Browse,
    pub replays: Browse,
    /// Selected room code in the room browser.
    pub room: Option<String>,
    pub public_room: bool,
    pub dialog: Option<Dialog>,
    pub dropdown: Option<Dropdown>,
    /// The latest session message and whether it is an error. Cleared on navigation.
    pub notice: Option<(String, bool)>,
    /// Bumped when the form changes outside its fields, so fields show the new value.
    pub form_revision: u32,
    pub compact: bool,
}

impl Default for Menu {
    fn default() -> Self {
        Self {
            section: Section::default(),
            solo: SoloPage::default(),
            map: 0,
            saves: Browse::default(),
            replays: Browse::default(),
            room: None,
            public_room: true,
            dialog: None,
            dropdown: None,
            notice: None,
            form_revision: 0,
            compact: false,
        }
    }
}

impl Menu {
    pub const fn browse(&self, kind: Shelf) -> &Browse {
        match kind {
            Shelf::Saves => &self.saves,
            Shelf::Replays => &self.replays,
        }
    }

    const fn browse_mut(&mut self, kind: Shelf) -> &mut Browse {
        match kind {
            Shelf::Saves => &mut self.saves,
            Shelf::Replays => &mut self.replays,
        }
    }

    fn navigate(&mut self) {
        self.dropdown = None;
        self.notice = None;
    }
}

/// Text typed into menu fields.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct MenuForm {
    pub url: String,
    /// Normalized invite code: uppercase letters and digits.
    pub code: String,
    /// The name as typed; [`Settings::name`] keeps the last valid one.
    pub name: Option<String>,
}

impl Default for MenuForm {
    fn default() -> Self {
        Self {
            url: DEFAULT_SERVER_URL.into(),
            code: String::new(),
            name: None,
        }
    }
}

impl MenuForm {
    pub fn url(&self) -> &str {
        self.url.trim()
    }

    pub fn url_valid(&self) -> bool {
        let url = self.url();
        ["ws://", "wss://"]
            .iter()
            .any(|scheme| url.len() > scheme.len() && url.starts_with(scheme))
    }

    pub fn code_valid(&self) -> bool {
        self.code.len() == ROOM_CODE_LEN
            && self
                .code
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    }
}

/// Everything a page or binding may show.
pub struct View<'a> {
    pub status: &'a SessionStatus,
    pub menu: &'a Menu,
    pub form: &'a MenuForm,
    pub settings: &'a Settings,
    pub library: &'a Library,
    pub catalog: &'a MapCatalog,
    pub crash: &'a CrashReport,
}

impl View<'_> {
    pub const fn online_page(&self) -> OnlinePage {
        OnlinePage::of(self.status)
    }

    pub fn selected_room(&self) -> Option<&RoomInfo> {
        let code = self.menu.room.as_deref()?;
        self.status.rooms.iter().find(|room| room.code == code)
    }

    pub fn selected_record(&self, kind: Shelf) -> Option<&Record> {
        self.library
            .get(kind, self.menu.browse(kind).selected_id()?)
    }

    pub fn server_url(&self) -> &str {
        match &self.status.server {
            Server::Connecting { url } | Server::Connected { url } | Server::Failed { url, .. } => {
                url
            }
            Server::Offline => "",
        }
    }

    /// Whether Esc or Back has somewhere to go.
    pub fn can_go_back(&self) -> bool {
        self.menu.dialog.is_some()
            || self.menu.dropdown.is_some()
            || match self.menu.section {
                Section::Singleplayer => self.menu.solo != SoloPage::Home,
                Section::Multiplayer => self.online_page() != OnlinePage::Connect,
                Section::Replays | Section::Settings => false,
            }
    }
}

#[derive(SystemParam)]
pub struct ViewParam<'w> {
    status: Res<'w, SessionStatus>,
    menu: Res<'w, Menu>,
    form: Res<'w, MenuForm>,
    settings: Res<'w, Settings>,
    library: Res<'w, Library>,
    catalog: Res<'w, MapCatalog>,
    crash: Res<'w, CrashReport>,
}

impl ViewParam<'_> {
    pub fn view(&self) -> View<'_> {
        View {
            status: &self.status,
            menu: &self.menu,
            form: &self.form,
            settings: &self.settings,
            library: &self.library,
            catalog: &self.catalog,
            crash: &self.crash,
        }
    }

    fn changed(&self) -> bool {
        self.status.is_changed()
            || self.menu.is_changed()
            || self.form.is_changed()
            || self.settings.is_changed()
            || self.library.is_changed()
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// What a menu button does. Buttons carry one; activating it sends it as a message.
#[derive(Component, Message, Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Section(Section),
    Back,
    OpenNewGame,
    OpenLoadGame,
    SelectMap(usize),
    StartGame,
    Select(Shelf, ReplayPick),
    ToggleDropdown(Dropdown),
    FilterMap(Shelf, Option<String>),
    Order(Shelf, bool),
    LoadSave,
    Delete(Shelf),
    WatchReplay,
    SaveLastReplay,
    ImportReplay,
    ExportReplay,
    UseRecentServer,
    Connect,
    Disconnect,
    RefreshRooms,
    SelectRoom(String),
    JoinSelected,
    OpenJoinByCode,
    JoinByCode,
    OpenCreateRoom,
    SetPublic(bool),
    CreateRoom,
    ToggleReady,
    StartMatch,
    LeaveRoom,
    CopyCode,
    SetUiScale(u16),
    SetEdgeScroll(bool),
    SetFullscreen(bool),
    /// Offers the crash report to the player, or forgets it.
    ExportCrash,
    DiscardCrash,
    Quit,
    CloseDialog,
    Confirm,
    TogglePause,
    Resume,
    /// In-game pause menu actions.
    Game(SessionAction),
}

impl Action {
    pub fn enabled(&self, view: &View) -> bool {
        let status = view.status;
        let menu = view.menu;
        match self {
            Self::Back => view.can_go_back(),
            Self::StartGame => view.catalog.0.get(menu.map).is_some(),
            Self::LoadSave | Self::Delete(Shelf::Saves) => {
                view.selected_record(Shelf::Saves).is_some()
            }
            Self::Delete(Shelf::Replays) | Self::ExportReplay => {
                view.selected_record(Shelf::Replays).is_some()
            }
            Self::WatchReplay => match &menu.replays.selected {
                Some(ReplayPick::Last) => status.has_replay,
                Some(ReplayPick::Stored(_)) => view.selected_record(Shelf::Replays).is_some(),
                None => false,
            },
            Self::SaveLastReplay | Self::Game(SessionAction::SaveReplay) => status.has_replay,
            Self::ImportReplay => !status.importing,
            Self::UseRecentServer => view.settings.recent_server.is_some(),
            Self::Connect => view.form.url_valid() && !status.connecting(),
            Self::Disconnect => status.connected() || status.connecting(),
            Self::RefreshRooms | Self::OpenJoinByCode | Self::OpenCreateRoom => {
                status.connected() && status.lobby.is_none()
            }
            Self::JoinSelected => view.selected_room().is_some() && !status.joining,
            Self::JoinByCode => view.form.code_valid() && status.connected() && !status.joining,
            Self::CreateRoom => status.connected() && !status.joining,
            Self::ToggleReady | Self::LeaveRoom | Self::CopyCode => status.lobby.is_some(),
            Self::StartMatch => status.lobby.as_ref().is_some_and(Lobby::can_start),
            Self::Game(SessionAction::SaveGame) => status.can_save_game(),
            Self::ExportCrash | Self::DiscardCrash => view.crash.pending().is_some(),
            _ => true,
        }
    }

    /// Whether this button shows the current choice: the open section, the chosen map, and so on.
    pub fn selected(&self, view: &View) -> bool {
        let menu = view.menu;
        match self {
            Self::Section(section) => menu.section == *section,
            Self::SelectMap(map) => menu.map == *map,
            Self::Select(kind, pick) => menu.browse(*kind).selected.as_ref() == Some(pick),
            Self::SelectRoom(code) => menu.room.as_ref() == Some(code),
            Self::SetPublic(public) => menu.public_room == *public,
            Self::SetUiScale(scale) => view.settings.ui_scale == *scale,
            Self::SetEdgeScroll(on) => view.settings.edge_scroll == *on,
            Self::SetFullscreen(on) => view.settings.fullscreen == *on,
            Self::FilterMap(kind, map) => menu.browse(*kind).map == *map,
            Self::Order(kind, oldest) => menu.browse(*kind).oldest_first == *oldest,
            _ => false,
        }
    }
}

/// Sends a button's action when it is activated and enabled.
pub fn activate(
    activate: On<Activate>,
    actions: Query<&Action, Without<InteractionDisabled>>,
    mut writer: MessageWriter<Action>,
) {
    if let Ok(action) = actions.get(activate.entity) {
        writer.write(action.clone());
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one match over every menu action keeps the behavior in one place"
)]
fn perform(
    mut actions: MessageReader<Action>,
    mut menu: ResMut<Menu>,
    mut pause: ResMut<PauseMenu>,
    mut targeting: Option<ResMut<MoveTargeting>>,
    mut form: ResMut<MenuForm>,
    mut settings: ResMut<Settings>,
    status: Res<SessionStatus>,
    library: Res<Library>,
    catalog: Res<MapCatalog>,
    mut crash: ResMut<CrashReport>,
    mut session: MessageWriter<SessionAction>,
    mut clipboard: ResMut<Clipboard>,
    mut exit: MessageWriter<AppExit>,
) {
    for action in actions.read() {
        if matches!(action, Action::Game(_) | Action::Resume) && !pause.open {
            continue;
        }
        let view = View {
            status: &status,
            menu: &menu,
            form: &form,
            settings: &settings,
            library: &library,
            catalog: &catalog,
            crash: &crash,
        };
        if !action.enabled(&view) {
            continue;
        }
        let name = settings.name.clone();
        match action.clone() {
            Action::Section(section) => {
                if section == Section::Singleplayer {
                    menu.solo = SoloPage::Home;
                }
                menu.section = section;
                menu.dialog = None;
                menu.navigate();
            }
            Action::Back => {
                if let Some(next) = back(&view) {
                    apply_back(next, &mut menu, &mut session);
                }
            }
            Action::OpenNewGame => {
                menu.solo = SoloPage::NewGame;
                menu.navigate();
            }
            Action::OpenLoadGame => {
                menu.solo = SoloPage::LoadGame;
                menu.navigate();
                select_first(&mut menu.saves, library.records(Shelf::Saves), false);
            }
            Action::SelectMap(map) => menu.map = map,
            Action::StartGame => {
                session.write(SessionAction::NewGame { map: menu.map });
            }
            Action::Select(shelf, pick) => menu.browse_mut(shelf).selected = Some(pick),
            Action::ToggleDropdown(dropdown) => {
                menu.dropdown = (menu.dropdown != Some(dropdown)).then_some(dropdown);
            }
            Action::FilterMap(shelf, map) => {
                menu.dropdown = None;
                menu.browse_mut(shelf).map = map;
                let has_last = shelf == Shelf::Replays && status.has_replay;
                select_first(menu.browse_mut(shelf), library.records(shelf), has_last);
            }
            Action::Order(shelf, oldest_first) => {
                menu.dropdown = None;
                menu.browse_mut(shelf).oldest_first = oldest_first;
            }
            Action::LoadSave => {
                if let Some(id) = menu.saves.selected_id() {
                    session.write(SessionAction::LoadGame { id: id.into() });
                }
            }
            Action::Delete(shelf) => {
                if let Some(id) = menu.browse(shelf).selected_id() {
                    menu.dialog = Some(Dialog::Confirm(Confirm::Delete(shelf, id.into())));
                }
            }
            Action::WatchReplay => match &menu.replays.selected {
                Some(ReplayPick::Last) => {
                    session.write(SessionAction::WatchLast);
                }
                Some(ReplayPick::Stored(id)) => {
                    session.write(SessionAction::WatchReplay { id: id.clone() });
                }
                None => {}
            },
            Action::SaveLastReplay => {
                session.write(SessionAction::SaveReplay);
            }
            Action::ImportReplay => {
                session.write(SessionAction::ImportReplay);
            }
            Action::ExportReplay => {
                if let Some(id) = menu.replays.selected_id() {
                    session.write(SessionAction::ExportReplay { id: id.into() });
                }
            }
            Action::UseRecentServer => {
                if let Some(url) = &settings.recent_server {
                    form.url.clone_from(url);
                    menu.form_revision += 1;
                }
            }
            Action::Connect => {
                menu.navigate();
                session.write(SessionAction::Connect {
                    url: form.url().into(),
                });
            }
            Action::Disconnect => {
                menu.navigate();
                session.write(SessionAction::Disconnect);
            }
            Action::RefreshRooms => {
                session.write(SessionAction::RefreshRooms);
            }
            Action::SelectRoom(code) => menu.room = Some(code),
            Action::JoinSelected => {
                if let Some(code) = menu.room.clone() {
                    menu.notice = None;
                    session.write(SessionAction::JoinRoom { name, code });
                }
            }
            Action::OpenJoinByCode => {
                menu.notice = None;
                menu.dialog = Some(Dialog::JoinByCode);
            }
            Action::JoinByCode => {
                menu.notice = None;
                session.write(SessionAction::JoinRoom {
                    name,
                    code: form.code.clone(),
                });
            }
            Action::OpenCreateRoom => {
                menu.notice = None;
                menu.dialog = Some(Dialog::CreateRoom);
            }
            Action::SetPublic(public) => menu.public_room = public,
            Action::CreateRoom => {
                session.write(SessionAction::CreateRoom {
                    name,
                    public: menu.public_room,
                });
            }
            Action::ToggleReady => {
                if let Some(lobby) = &status.lobby {
                    session.write(SessionAction::SetReady(!lobby.ready()));
                }
            }
            Action::StartMatch => {
                session.write(SessionAction::Start);
            }
            Action::LeaveRoom => menu.dialog = Some(Dialog::Confirm(Confirm::LeaveRoom)),
            Action::CopyCode => {
                if let Some(lobby) = &status.lobby {
                    menu.notice = Some(match clipboard.set_text(lobby.code.clone()) {
                        Ok(()) => ("Invite code copied.".into(), false),
                        Err(_) => ("Cannot use the clipboard here.".into(), true),
                    });
                }
            }
            Action::SetUiScale(scale) => settings.ui_scale = scale,
            Action::SetEdgeScroll(on) => settings.edge_scroll = on,
            Action::SetFullscreen(on) => settings.fullscreen = on,
            Action::ExportCrash => match crash.export() {
                Ok(message) => {
                    menu.dialog = None;
                    menu.notice = Some((message, false));
                }
                Err(message) => menu.notice = Some((message, true)),
            },
            Action::DiscardCrash => {
                crash.discard();
                menu.dialog = None;
                menu.notice = Some(("Crash report discarded.".into(), false));
            }
            Action::Quit => {
                if status.connected() || status.connecting() {
                    menu.dialog = Some(Dialog::Confirm(Confirm::Quit));
                } else {
                    exit.write(AppExit::Success);
                }
            }
            Action::TogglePause => {
                pause.open = !pause.open;
                if pause.open
                    && let Some(targeting) = targeting.as_deref_mut()
                {
                    targeting.0 = false;
                }
            }
            Action::Resume => {
                pause.open = false;
                menu.dialog = None;
            }
            Action::CloseDialog => menu.dialog = None,
            Action::Confirm => {
                if let Some(Dialog::Confirm(confirm)) = menu.dialog.take() {
                    match confirm {
                        Confirm::Delete(shelf, id) => {
                            menu.browse_mut(shelf).selected = None;
                            session.write(SessionAction::Delete { shelf, id });
                        }
                        Confirm::LeaveRoom => {
                            session.write(SessionAction::LeaveRoom);
                        }
                        Confirm::LeaveMatch => {
                            session.write(SessionAction::Leave);
                        }
                        Confirm::Quit => {
                            session.write(SessionAction::Disconnect);
                            exit.write(AppExit::Success);
                        }
                    }
                }
            }
            Action::Game(SessionAction::Leave) if leave_needs_confirmation(&status) => {
                menu.dialog = Some(Dialog::Confirm(Confirm::LeaveMatch));
            }
            Action::Game(action) => {
                session.write(action);
            }
        }
    }
}

/// Leaving a live solo or online match loses progress or ends it for others.
fn leave_needs_confirmation(status: &SessionStatus) -> bool {
    status.running && !status.stopped && status.kind != MatchKind::Playback
}

/// Where Back goes from here.
#[derive(Debug, PartialEq, Eq)]
enum BackTo {
    CloseDropdown,
    CloseDialog,
    SoloHome,
    Disconnect,
    ConfirmLeaveRoom,
}

fn back(view: &View) -> Option<BackTo> {
    if view.menu.dropdown.is_some() {
        return Some(BackTo::CloseDropdown);
    }
    if view.menu.dialog.is_some() {
        return Some(BackTo::CloseDialog);
    }
    match view.menu.section {
        Section::Singleplayer if view.menu.solo != SoloPage::Home => Some(BackTo::SoloHome),
        Section::Multiplayer => match view.online_page() {
            OnlinePage::Connect => None,
            OnlinePage::Connecting | OnlinePage::Rooms => Some(BackTo::Disconnect),
            OnlinePage::Lobby => Some(BackTo::ConfirmLeaveRoom),
        },
        _ => None,
    }
}

fn apply_back(to: BackTo, menu: &mut Menu, session: &mut MessageWriter<SessionAction>) {
    match to {
        BackTo::CloseDropdown => menu.dropdown = None,
        BackTo::CloseDialog => menu.dialog = None,
        BackTo::SoloHome => {
            menu.solo = SoloPage::Home;
            menu.navigate();
        }
        BackTo::Disconnect => {
            menu.navigate();
            session.write(SessionAction::Disconnect);
        }
        BackTo::ConfirmLeaveRoom => menu.dialog = Some(Dialog::Confirm(Confirm::LeaveRoom)),
    }
}

/// Keeps a valid selection: the current one if still listed, otherwise the first row.
fn select_first(browse: &mut Browse, records: &[Record], has_last: bool) {
    // The last match has no map filter entry, so it is listed only without a filter.
    let has_last = has_last && browse.map.is_none();
    let shown = browse.filter(records);
    let listed = match &browse.selected {
        Some(ReplayPick::Last) => has_last,
        Some(ReplayPick::Stored(id)) => shown.iter().any(|record| &record.id == id),
        None => false,
    };
    if !listed {
        browse.selected = if has_last {
            Some(ReplayPick::Last)
        } else {
            shown
                .first()
                .map(|record| ReplayPick::Stored(record.id.clone()))
        };
    }
}

fn back_on_escape(keys: Res<ButtonInput<KeyCode>>, mut actions: MessageWriter<Action>) {
    if keys.just_pressed(KeyCode::Escape) {
        actions.write(Action::Back);
    }
}

/// Reacts to session changes: closes finished dialogs, keeps selections valid, remembers the
/// last server, and shows new messages.
fn follow_session(
    status: Res<SessionStatus>,
    library: Res<Library>,
    mut menu: ResMut<Menu>,
    mut settings: ResMut<Settings>,
    mut last_message: Local<String>,
) {
    if status.is_changed() {
        if *last_message != status.message {
            last_message.clone_from(&status.message);
            if !status.message.is_empty() {
                menu.notice = Some((status.message.clone(), status.error));
            }
        }
        if let Server::Connected { url } = &status.server
            && settings.recent_server.as_ref() != Some(url)
        {
            settings.recent_server = Some(url.clone());
        }
        let in_room = status.lobby.is_some();
        let dialog_done = match &menu.dialog {
            Some(Dialog::JoinByCode | Dialog::CreateRoom) => in_room || !status.connected(),
            Some(Dialog::Confirm(Confirm::LeaveRoom)) => !in_room,
            _ => false,
        };
        if dialog_done {
            menu.dialog = None;
        }
        if menu
            .room
            .as_ref()
            .is_some_and(|code| !status.rooms.iter().any(|room| &room.code == code))
            || menu.room.is_none()
        {
            let first = status.rooms.first().map(|room| room.code.clone());
            if menu.room != first {
                menu.room = first;
            }
        }
    }
    if status.is_changed() || library.is_changed() {
        let has_last = status.has_replay;
        let mut saves = menu.saves.clone();
        select_first(&mut saves, library.records(Shelf::Saves), false);
        let mut replays = menu.replays.clone();
        select_first(&mut replays, library.records(Shelf::Replays), has_last);
        if saves != menu.saves {
            menu.saves = saves;
        }
        if replays != menu.replays {
            menu.replays = replays;
        }
    }
}

// ---------------------------------------------------------------------------
// Fields
// ---------------------------------------------------------------------------

/// Stores field edits in [`MenuForm`] and the player name in [`Settings`].
fn on_field_edit(
    change: On<TextEditChange>,
    mut fields: Query<(&Field, &mut EditableText)>,
    mut form: ResMut<MenuForm>,
    mut settings: ResMut<Settings>,
) {
    let Ok((field, mut text)) = fields.get_mut(change.event_target()) else {
        return;
    };
    let value = text.value().to_string();
    let value = if *field == Field::Code {
        let code = value.to_ascii_uppercase();
        // ASCII case changes keep byte offsets, so the cursor and selection stay valid.
        if code != value && !text.is_composing() {
            text.editor_mut().set_text(&code);
        }
        code
    } else {
        value
    };
    store_field_value(*field, value, &mut form, &mut settings);
}

/// Shared by Bevy's native editor and the browser's HTML input.
fn store_field_value(field: Field, value: String, form: &mut MenuForm, settings: &mut Settings) {
    match field {
        Field::Url if form.url != value => form.url = value,
        Field::Code if form.code != value => form.code = value,
        Field::Name => store_name(value, form, settings),
        Field::Url | Field::Code => {}
    }
}

fn store_name(value: String, form: &mut MenuForm, settings: &mut Settings) {
    if rts_net::validate_name(&value).is_ok() && settings.name != value {
        settings.name.clone_from(&value);
    }
    if form.name.as_ref() != Some(&value) {
        form.name = Some(value);
    }
}

/// Whether the typed name is usable online.
pub fn name_hint(view: &View) -> String {
    let typed = view.form.name.as_deref().unwrap_or(&view.settings.name);
    if rts_net::validate_name(typed).is_ok() {
        format!("Shown to other players in online rooms. Up to {MAX_NAME_CHARS} characters.")
    } else {
        format!(
            "Names need 1 to {MAX_NAME_CHARS} characters without spaces at either end. \
             Rooms use \"{}\" until then.",
            view.settings.name
        )
    }
}

/// Enter submits the field's form: connect, or join by code.
fn submit_on_enter(
    input: On<FocusedInput<KeyboardInput>>,
    fields: Query<(&Field, &EditableText)>,
    mut actions: MessageWriter<Action>,
) {
    let event = &input.input;
    if event.logical_key != Key::Enter || !event.state.is_pressed() || event.repeat {
        return;
    }
    let Ok((field, text)) = fields.get(input.focused_entity) else {
        return;
    };
    if text.is_composing() {
        return;
    }
    match field {
        Field::Url => {
            actions.write(Action::Connect);
        }
        Field::Code => {
            actions.write(Action::JoinByCode);
        }
        Field::Name => {}
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Root of the menu screen.
#[derive(Component, Clone, Default)]
pub struct MenuRoot;

/// The content area; its children are the current page.
#[derive(Component, Clone, Default)]
pub struct PageHost;

/// Layer above the page for dialogs.
#[derive(Component, Clone, Default)]
pub struct DialogHost;

/// The message line under the page.
#[derive(Component, Clone, Default)]
pub struct NoticeText;

/// What is on screen, to spawn again only when it changes.
#[derive(Resource, Default)]
struct Rendered {
    compact: Option<bool>,
    page: Option<pages::PageKey>,
    /// Action of the focused button before a page was replaced.
    focus: Option<Action>,
}

/// No state change is queued. Scenes spawned now would outlive the screen being left.
fn staying(next: Res<NextState<AppState>>) -> bool {
    matches!(*next, NextState::Unchanged)
}

fn close_dialogs(mut menu: ResMut<Menu>) {
    menu.dialog = None;
    menu.dropdown = None;
}

/// Offers a report left by a previous run, once per launch.
fn open_crash_dialog(report: Res<CrashReport>, mut menu: ResMut<Menu>, mut offered: Local<bool>) {
    if *offered || menu.dialog.is_some() || report.pending().is_none() {
        return;
    }
    info!("offering the crash report left by the previous run");
    menu.dialog = Some(Dialog::CrashReport);
    *offered = true;
}

fn forget_render(mut rendered: ResMut<Rendered>) {
    *rendered = Rendered::default();
}

fn track_compact(
    window: Single<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    mut menu: ResMut<Menu>,
) {
    let compact = window.width() / scale.0.max(0.1) < COMPACT_WIDTH;
    if menu.compact != compact {
        menu.compact = compact;
    }
}

fn render(
    mut commands: Commands,
    view: ViewParam,
    mut rendered: ResMut<Rendered>,
    focus: Res<InputFocus>,
    actions: Query<&Action>,
    roots: Query<Entity, With<MenuRoot>>,
    page_hosts: Query<Entity, With<PageHost>>,
) {
    if !view.changed() && rendered.compact.is_some() && rendered.page.is_some() {
        return;
    }
    let view = view.view();
    if rendered.compact != Some(view.menu.compact) {
        for root in &roots {
            commands.entity(root).despawn();
        }
        commands.spawn_scene(pages::menu_root(view.menu.compact));
        rendered.compact = Some(view.menu.compact);
        rendered.page = None;
        // Hosts exist after the commands apply; fill them next frame.
        return;
    }
    let page = pages::PageKey::of(&view);
    if rendered.page.as_ref() != Some(&page) && !page_hosts.is_empty() {
        rendered.focus = focus
            .get()
            .and_then(|entity| actions.get(entity).ok())
            .cloned();
        for host in &page_hosts {
            commands.entity(host).despawn_children();
            commands
                .spawn_scene(pages::page(&view))
                .insert(ChildOf(host));
        }
        rendered.page = Some(page);
    }
}

/// Fills the dialog layer of the main menu or the pause menu.
fn render_dialog(
    mut commands: Commands,
    view: ViewParam,
    mut shown: Local<ShownDialog>,
    hosts: Query<Entity, With<DialogHost>>,
    added: Query<(), Added<DialogHost>>,
) {
    let view = view.view();
    if !added.is_empty() {
        shown.known = false;
    }
    if shown.known && shown.dialog == view.menu.dialog {
        return;
    }
    for host in &hosts {
        commands.entity(host).despawn_children();
        if view.menu.dialog.is_some() {
            commands
                .spawn_scene(pages::dialog(&view))
                .insert(ChildOf(host));
        }
    }
    *shown = ShownDialog {
        known: true,
        dialog: view.menu.dialog.clone(),
    };
}

/// The dialog on screen. Unknown after a new host appears.
#[derive(Default)]
struct ShownDialog {
    known: bool,
    dialog: Option<Dialog>,
}

/// After a page is replaced, focuses the button with the action that had focus before.
fn restore_focus(
    mut rendered: ResMut<Rendered>,
    mut focus: ResMut<InputFocus>,
    buttons: Query<(Entity, &Action), Added<Action>>,
) {
    let Some(wanted) = &rendered.focus else {
        return;
    };
    if let Some((entity, _)) = buttons.iter().find(|(_, action)| *action == wanted) {
        focus.set(entity, FocusCause::Navigated);
        rendered.focus = None;
    } else if !buttons.is_empty() {
        rendered.focus = None;
    }
}

fn update_notice(
    menu: Res<Menu>,
    status: Res<SessionStatus>,
    mut notices: Query<(&mut Text, &mut TextColor), With<NoticeText>>,
) {
    if !menu.is_changed() && !status.is_changed() {
        return;
    }
    let (message, color) = match &menu.notice {
        Some((message, true)) => (message.clone(), widgets::DANGER),
        Some((message, false)) => (message.clone(), widgets::INK_MUTED),
        None => (String::new(), widgets::INK_MUTED),
    };
    for (mut text, mut ink) in &mut notices {
        if text.0 != message {
            text.0.clone_from(&message);
        }
        ink.set_if_neq(TextColor(color));
    }
}

/// Touch scrolling for scroll areas. Mouse wheels scroll through [`ScrollArea`].
pub fn scroll_on_touch_drag(
    drag: On<PointerDrag>,
    mut areas: Query<(&ComputedNode, &mut ScrollPosition), With<ScrollArea>>,
) {
    if !matches!(drag.pointer.id, PointerId::Touch(_)) {
        return;
    }
    if let Ok((node, mut scroll)) = areas.get_mut(drag.event_target()) {
        let visible = node.size().y * node.inverse_scale_factor;
        let content = node.content_size().y * node.inverse_scale_factor;
        let max = (content - visible).max(0.0);
        scroll.y = (scroll.y - drag.delta.y).clamp(0.0, max);
    }
}

/// Keeps the focused control visible when tabbing through a scrolled page.
pub fn scroll_focus_into_view(focus: On<FocusGained>, mut commands: Commands) {
    commands.trigger(ScrollIntoView {
        entity: focus.entity,
    });
}

#[cfg(test)]
mod tests;
