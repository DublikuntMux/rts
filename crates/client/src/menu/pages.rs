//! Menu pages and dialogs as scenes, each a function of the menu model.

use bevy::{
    input_focus::tab_navigation::TabGroup, prelude::*, scene::EntityScene, ui_widgets::ScrollArea,
};
use rts_net::RoomInfo;

use super::{
    Action, Browse, Confirm, Dialog, DialogHost, Dropdown, MAX_URL_LEN, MenuRoot, NoticeText,
    OnlinePage, PageHost, ROOM_CODE_LEN, ReplayPick, Section, SoloPage, View, name_hint,
    previews::preview_for,
    scroll_focus_into_view, scroll_on_touch_drag,
    widgets::{
        DANGER, EDGE, FIELD, FIELD_EDGE, Field, GOOD, INK, INK_DISABLED, INK_MUTED, PANEL, RAIL,
        SCREEN, SIGNAL, ShowIf, Tone, badge, bound_text, button, column, dialog as dialog_frame,
        empty_preview, heading, hint, list_item, map_image, panel, row, spinner, split, text,
        text_field, wide_button,
    },
};
use crate::{
    game::AppState,
    library::{Record, Shelf, duration},
    session::{Lobby, Server},
    settings::{Settings, UI_SCALES},
};

type Scenes = Vec<Box<dyn SceneList>>;

/// Boxes a scene as a one-entity list so pages can mix different scenes in one list.
fn boxed(scene: impl Scene) -> Box<dyn SceneList> {
    Box::new(EntityScene(scene))
}

fn when(condition: bool, scene: impl Scene) -> Scenes {
    if condition {
        vec![boxed(scene)]
    } else {
        Vec::new()
    }
}

/// The parts of the model a page shows. The page is spawned again when this changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageKey {
    SoloHome {
        latest: Option<Record>,
    },
    NewGame {
        map: usize,
    },
    Browse {
        shelf: Shelf,
        browse: Browse,
        records: Vec<Record>,
        dropdown: Option<Dropdown>,
        has_last: bool,
    },
    Connect {
        revision: u32,
        recent: Option<String>,
    },
    Connecting {
        url: String,
    },
    Rooms {
        url: String,
        rooms: Vec<RoomInfo>,
        selected: Option<String>,
    },
    Lobby(Lobby),
    Settings {
        settings: Settings,
        revision: u32,
    },
}

impl PageKey {
    pub fn of(view: &View) -> Self {
        let menu = view.menu;
        let browse = |shelf: Shelf| Self::Browse {
            shelf,
            browse: menu.browse(shelf).clone(),
            records: view.library.records(shelf).to_vec(),
            dropdown: menu.dropdown,
            has_last: shelf == Shelf::Replays && view.status.has_replay,
        };
        match menu.section {
            Section::Singleplayer => match menu.solo {
                SoloPage::Home => Self::SoloHome {
                    latest: view.library.records(Shelf::Saves).first().cloned(),
                },
                SoloPage::NewGame => Self::NewGame { map: menu.map },
                SoloPage::LoadGame => browse(Shelf::Saves),
            },
            Section::Multiplayer => match view.online_page() {
                OnlinePage::Connect => Self::Connect {
                    revision: menu.form_revision,
                    recent: view.settings.recent_server.clone(),
                },
                OnlinePage::Connecting => Self::Connecting {
                    url: view.server_url().into(),
                },
                OnlinePage::Rooms => Self::Rooms {
                    url: view.server_url().into(),
                    rooms: view.status.rooms.clone(),
                    selected: menu.room.clone(),
                },
                OnlinePage::Lobby => {
                    Self::Lobby(view.status.lobby.clone().expect("lobby page has a lobby"))
                }
            },
            Section::Replays => browse(Shelf::Replays),
            Section::Settings => Self::Settings {
                settings: Settings {
                    name: String::new(),
                    recent_server: None,
                    ..view.settings.clone()
                },
                revision: menu.form_revision,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Frame
// ---------------------------------------------------------------------------

/// The whole menu screen with empty page and dialog hosts.
#[expect(
    clippy::too_many_lines,
    reason = "the screen frame reads best as one scene"
)]
pub fn menu_root(compact: bool) -> impl Scene {
    let group = TabGroup::new(0);
    let dialog_shown = ShowIf(|view| view.menu.dialog.is_some());
    let rail_width = if compact { percent(100) } else { px(216) };
    let notice_shown = ShowIf(|view| view.menu.notice.is_some());
    let nav: Scenes = [
        ("Singleplayer", "Solo", Section::Singleplayer),
        ("Multiplayer", "Online", Section::Multiplayer),
        ("Replays", "Replays", Section::Replays),
        ("Settings", "Settings", Section::Settings),
    ]
    .into_iter()
    .map(|(long, short, section)| {
        boxed(nav_button(
            if compact { short } else { long },
            Action::Section(section),
            compact,
        ))
    })
    .chain(when(
        !cfg!(target_arch = "wasm32"),
        bsn! {
            @nav_button("Quit", Action::Quit, compact)
            Node { margin: {if compact { UiRect::ZERO } else { UiRect::top(auto()) }} }
        },
    ))
    .collect();
    bsn! {
        #Menu
        MenuRoot
        DespawnOnExit<AppState>(AppState::Menu)
        group
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
        }
        BackgroundColor(SCREEN)
        Children [
            @title_bar(compact)
            --
            Node {
                flex_direction: {if compact { FlexDirection::Column } else { FlexDirection::Row }},
                flex_grow: 1.0,
                min_height: px(0),
            }
            Children [
                Node {
                    flex_direction: {if compact { FlexDirection::Row } else { FlexDirection::Column }},
                    width: rail_width,
                    flex_wrap: {if compact { FlexWrap::Wrap } else { FlexWrap::NoWrap }},
                    flex_shrink: 0.0,
                    padding: UiRect::all(px(if compact { 6.0 } else { 16.0 })),
                    column_gap: px(4),
                    row_gap: px(6),
                    border: {if compact { UiRect::bottom(px(1)) } else { UiRect::right(px(1)) }},
                }
                BackgroundColor(RAIL)
                BorderColor::all(EDGE)
                Children [ {nav} ]
                --
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    min_width: px(0),
                    min_height: px(0),
                }
                Children [
                    ScrollArea
                    Node {
                        flex_direction: FlexDirection::Column,
                        flex_grow: 1.0,
                        min_height: px(0),
                        overflow: Overflow::scroll_y(),
                    }
                    on(scroll_on_touch_drag)
                    on(scroll_focus_into_view)
                    Children [
                        PageHost
                        Node {
                            flex_direction: FlexDirection::Column,
                            flex_shrink: 0.0,
                            width: percent(100),
                            max_width: px(1160),
                            align_self: AlignSelf::Center,
                            padding: UiRect::all(px(if compact { 16.0 } else { 40.0 })),
                            row_gap: px(24),
                        }
                    ]
                    --
                    notice_shown
                    Node {
                        min_height: px(40),
                        padding: UiRect::axes(px(20), px(6)),
                        border: UiRect::top(px(1)),
                        align_items: AlignItems::Center,
                    }
                    BorderColor::all(EDGE)
                    Children [ @text("", 13.0, INK_MUTED) NoticeText ]
                ]
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
                bottom: px(0),
            }
        ]
    }
}

#[expect(
    clippy::redundant_closure_for_method_calls,
    reason = "the method has a narrower lifetime than the binding's function pointer"
)]
fn title_bar(compact: bool) -> impl Scene {
    let can_go_back = ShowIf(|view| view.can_go_back());
    let back: Scenes = when(
        compact,
        bsn! {
            @button("Back", Action::Back, Tone::Secondary)
            can_go_back
        },
    );
    bsn! {
        Node {
            min_height: px(if compact { 56.0 } else { 72.0 }),
            padding: UiRect::axes(px(if compact { 16.0 } else { 24.0 }), px(8)),
            column_gap: px(12),
            align_items: AlignItems::Center,
            border: UiRect::bottom(px(1)),
        }
        BackgroundColor(PANEL)
        BorderColor::all(EDGE)
        Children [
            {back}
            --
            @command_mark()
            --
            @text("RTS", 30.0, INK)
            --
            Node { flex_grow: 1.0, min_width: px(0), justify_content: JustifyContent::FlexEnd }
            Children [ @bound_text(breadcrumb, 12.0, INK_MUTED) ]
        ]
    }
}

fn nav_button(label: &'static str, action: Action, compact: bool) -> impl Scene {
    bsn! {
        @button(label, action, Tone::Nav)
        Node {
            justify_content: JustifyContent::FlexStart,
            flex_grow: {if compact { 1.0_f32 } else { 0.0_f32 }},
            flex_basis: {if compact { px(48) } else { auto() }},
            flex_shrink: 1.0,
            min_width: px(48),
            padding: UiRect::axes(px(if compact { 2.0 } else { 10.0 }), px(8)),
            border: UiRect { left: px(3), ..UiRect::all(px(1)) },
        }
    }
}

/// Four field units around a shared objective, drawn without a font glyph or image asset.
fn command_mark() -> impl Scene {
    bsn! {
        Node {
            width: px(24),
            height: px(24),
            flex_shrink: 0.0,
            flex_wrap: FlexWrap::Wrap,
            column_gap: px(4),
            row_gap: px(4),
        }
        Children [
            Node { width: px(10), height: px(10) } BackgroundColor(SIGNAL)
            --
            Node { width: px(10), height: px(10) } BackgroundColor(INK_MUTED)
            --
            Node { width: px(10), height: px(10) } BackgroundColor(INK_MUTED)
            --
            Node { width: px(10), height: px(10) } BackgroundColor(SIGNAL)
        ]
    }
}

fn breadcrumb(view: &View) -> String {
    let menu = view.menu;
    let page = match menu.section {
        Section::Singleplayer => match menu.solo {
            SoloPage::Home => "Singleplayer",
            SoloPage::NewGame => "Singleplayer / New game",
            SoloPage::LoadGame => "Singleplayer / Load game",
        },
        Section::Multiplayer => match view.online_page() {
            OnlinePage::Connect => "Multiplayer / Server",
            OnlinePage::Connecting => "Multiplayer / Connecting",
            OnlinePage::Rooms => "Multiplayer / Rooms",
            OnlinePage::Lobby => "Multiplayer / Lobby",
        },
        Section::Replays => "Replays",
        Section::Settings => "Settings",
    };
    page.into()
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

/// The current page in a column that fills the content area.
pub fn page(view: &View) -> impl Scene {
    let contents = page_contents(view);
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(24),
        }
        Children [ {contents} ]
    }
}

fn page_contents(view: &View) -> Box<dyn SceneList> {
    match PageKey::of(view) {
        PageKey::SoloHome { latest } => Box::new(solo_home(view, latest)),
        PageKey::NewGame { map } => Box::new(new_game(view, map)),
        PageKey::Browse { shelf, .. } => Box::new(browser(view, shelf)),
        PageKey::Connect { .. } => Box::new(connect(view)),
        PageKey::Connecting { url } => Box::new(connecting(url)),
        PageKey::Rooms { url, .. } => Box::new(rooms(view, url)),
        PageKey::Lobby(lobby) => Box::new(lobby_page(lobby)),
        PageKey::Settings { .. } => Box::new(settings_page(view)),
    }
}

fn solo_home(view: &View, latest: Option<Record>) -> impl SceneList {
    let preview = latest.as_ref().map_or_else(
        || boxed(catalog_preview(view, view.menu.map)),
        |record| record_preview(view, record),
    );
    let map = &view.catalog.0[view.menu.map.min(view.catalog.0.len() - 1)];
    let (map_name, caption) = latest.map_or_else(
        || {
            (
                map.name.clone(),
                format!(
                    "{} players / {}",
                    map.setup.players,
                    map_size(&map.setup.map)
                ),
            )
        },
        |record| {
            (
                record.meta.map.clone(),
                format!(
                    "Latest save: {} / {}",
                    record.meta.name,
                    duration(record.meta.ticks)
                ),
            )
        },
    );
    bsn_list! {
        @heading("Your next move")
        --
        @hint("Read the terrain. Position your units. Take the field.")
        --
        @split(
            bsn_list! {
                @text("Singleplayer", 22.0, INK)
                --
                @hint("Choose a battlefield and command player 1. Your match can be saved and continued later.")
                --
                @wide_button("New game", Action::OpenNewGame, Tone::Primary)
                Node { margin: UiRect::top(px(16)), flex_grow: 0.0, flex_basis: auto() }
                --
                @wide_button("Load game", Action::OpenLoadGame, Tone::Secondary)
                Node { flex_grow: 0.0, flex_basis: auto() }
                --
                @hint("Continue a saved match.")
            },
            bsn_list! {
                @text(map_name, 22.0, INK)
                --
                {preview}
                --
                @hint(caption)
            },
        )
    }
}

fn new_game(view: &View, selected: usize) -> impl SceneList {
    let maps: Vec<_> = view
        .catalog
        .0
        .iter()
        .enumerate()
        .map(|(index, map)| {
            let size = map_size(&map.setup.map);
            list_item(
                map.name.clone(),
                format!("{} players · {size}", map.setup.players),
                Action::SelectMap(index),
            )
        })
        .collect();
    let map = &view.catalog.0[selected.min(view.catalog.0.len() - 1)];
    let detail = format!(
        "{} players · {}. You control player 1.",
        map.setup.players,
        map_size(&map.setup.map)
    );
    bsn_list! {
        @heading("Select map")
        --
        @hint("Choose your ground. Colored markers show each player’s starting position.")
        --
        @split(
            maps,
            bsn_list! {
                @catalog_preview(view, selected)
                --
                @text({format!("Selected: {}", map.name)}, 16.0, INK)
                --
                @hint(detail)
                --
                @row(bsn_list! { @wide_button("Start game", Action::StartGame, Tone::Primary) })
            },
        )
    }
}

/// The save browser or the replay browser.
fn browser(view: &View, shelf: Shelf) -> impl SceneList {
    let browse = view.menu.browse(shelf);
    let records = view.library.records(shelf);
    let has_last = shelf == Shelf::Replays && view.status.has_replay;
    let mut items: Scenes = Vec::new();
    if has_last && browse.map.is_none() {
        items.push(boxed(list_item(
            "Last match",
            "Not in the library yet",
            Action::Select(shelf, ReplayPick::Last),
        )));
    }
    items.extend(browse.filter(records).into_iter().map(|record| {
        boxed(list_item(
            record.meta.name.clone(),
            format!("{} · {}", record.meta.map, duration(record.meta.ticks)),
            Action::Select(shelf, ReplayPick::Stored(record.id.clone())),
        ))
    }));
    if items.is_empty() {
        items.push(boxed(empty_preview(match (shelf, records.is_empty()) {
            (Shelf::Saves, true) => "No saved games yet. Save a solo match from its top bar.",
            (Shelf::Replays, true) => "No replays yet. Save one after a match.",
            (_, false) => "Nothing on this map. Choose All maps.",
        })));
    }
    let title = match shelf {
        Shelf::Saves => "Load game",
        Shelf::Replays => "Replay browser",
    };
    let footer: Scenes = match shelf {
        Shelf::Saves => vec![boxed(hint("Map filtering is optional."))],
        Shelf::Replays => when(
            cfg!(target_arch = "wasm32"),
            row(bsn_list! {
                @button("Import replay file", Action::ImportReplay, Tone::Secondary)
                --
                @hint("Replays are kept in this browser's storage.")
            }),
        ),
    };
    bsn_list! {
        @heading(title)
        --
        @filters(view, shelf)
        --
        @split(items, browser_detail(view, shelf))
        --
        {footer}
    }
}

fn filters(view: &View, shelf: Shelf) -> impl Scene {
    let browse = view.menu.browse(shelf);
    let mut maps: Vec<String> = view
        .library
        .records(shelf)
        .iter()
        .map(|record| record.meta.map.clone())
        .collect();
    maps.sort();
    maps.dedup();
    let map_options: Vec<(String, Action)> =
        std::iter::once(("All maps".to_owned(), Action::FilterMap(shelf, None)))
            .chain(
                maps.into_iter()
                    .map(|map| (map.clone(), Action::FilterMap(shelf, Some(map)))),
            )
            .collect();
    let order_options = vec![
        ("Most recent".to_owned(), Action::Order(shelf, false)),
        ("Oldest first".to_owned(), Action::Order(shelf, true)),
    ];
    let map_label = browse.map.clone().unwrap_or_else(|| "All maps".into());
    let order_label = if browse.oldest_first {
        "Oldest first"
    } else {
        "Most recent"
    };
    let open = view.menu.dropdown;
    row(bsn_list! {
        @dropdown(map_label, Dropdown::Map(shelf), open == Some(Dropdown::Map(shelf)), map_options)
        --
        @dropdown(order_label.into(), Dropdown::Order(shelf), open == Some(Dropdown::Order(shelf)), order_options)
    })
}

fn browser_detail(view: &View, shelf: Shelf) -> Scenes {
    let browse = view.menu.browse(shelf);
    match (&browse.selected, view.selected_record(shelf)) {
        (Some(ReplayPick::Last), _) if shelf == Shelf::Replays => vec![
            boxed(empty_preview(
                "The most recent match, kept until the next one starts.",
            )),
            boxed(text("Last match", 16.0, INK)),
            boxed(row(bsn_list! {
                @wide_button("Watch replay", Action::WatchReplay, Tone::Primary)
                --
                @button("Save to library", Action::SaveLastReplay, Tone::Secondary)
            })),
        ],
        (_, Some(record)) => {
            let meta = &record.meta;
            let title = format!("{} / {}", meta.name, meta.map);
            let detail = format!(
                "{} {} · {} players",
                if shelf == Shelf::Saves {
                    "Played"
                } else {
                    "Duration"
                },
                duration(meta.ticks),
                meta.players
            );
            let actions: Box<dyn SceneList> = match shelf {
                Shelf::Saves => boxed(row(bsn_list! {
                    @wide_button("Load save", Action::LoadSave, Tone::Primary)
                    --
                    @button("Delete…", Action::Delete(Shelf::Saves), Tone::Danger)
                })),
                Shelf::Replays => boxed(row(bsn_list! {
                    @wide_button("Watch replay", Action::WatchReplay, Tone::Primary)
                    --
                    {when(cfg!(target_arch = "wasm32"), button("Export", Action::ExportReplay, Tone::Secondary))}
                    --
                    @button("Delete…", Action::Delete(Shelf::Replays), Tone::Danger)
                })),
            };
            vec![
                record_preview(view, record),
                boxed(text(title, 16.0, INK)),
                boxed(hint(detail)),
                actions,
            ]
        }
        _ => vec![boxed(empty_preview(match shelf {
            Shelf::Saves => "Select a save to see its details.",
            Shelf::Replays => "Select a replay to see its details.",
        }))],
    }
}

fn connect(view: &View) -> impl SceneList {
    let recent: Scenes = view
        .settings
        .recent_server
        .as_ref()
        .map_or_else(Vec::new, |url| {
            vec![boxed(button(
                format!("Recent: {url}"),
                Action::UseRecentServer,
                Tone::Secondary,
            ))]
        });
    let field = text_field(Field::Url, &view.form.url, MAX_URL_LEN);
    let failed = ShowIf(|view| matches!(view.status.server, Server::Failed { .. }));
    bsn_list! {
        @heading("Connect to a server")
        --
        @panel(bsn_list! {
            @hint("Server address")
            --
            Node { column_gap: px(8) } Children [ @{field} ]
            --
            @hint("ws:// or wss:// followed by host:port.")
            --
            @row(bsn_list! {
                {recent}
                --
                @wide_button("Connect", Action::Connect, Tone::Primary)
            })
        })
        --
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(14)),
            row_gap: px(10),
            border: UiRect::left(px(3)),
        }
        BackgroundColor(PANEL)
        BorderColor::all(DANGER)
        failed
        Children [
            @bound_text(connect_failure, 15.0, INK)
            --
            @row(bsn_list! { @button("Retry", Action::Connect, Tone::Secondary) })
            --
            @hint("Your address is kept; edit it above or retry.")
        ]
        --
        @hint("Join by code is available after connecting. Room codes belong to the server that made them.")
        --
        @hint("Players can host their own servers with rts-server.")
    }
}

fn connect_failure(view: &View) -> String {
    match &view.status.server {
        Server::Failed { url, reason } => format!("Could not connect to {url}. {reason}"),
        _ => String::new(),
    }
}

fn connecting(url: String) -> impl SceneList {
    bsn_list! {
        @heading("Connection state")
        --
        @panel(bsn_list! {
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(14),
                padding: UiRect::all(px(16)),
            }
            Children [
                @text("Connecting…", 18.0, INK)
                --
                @hint(url)
                --
                @spinner()
            ]
            --
            @row(bsn_list! { @wide_button("Cancel", Action::Disconnect, Tone::Secondary) })
        })
        --
        @hint("If the connection fails you can retry or edit the address.")
    }
}

fn rooms(view: &View, url: String) -> impl SceneList {
    let host = url
        .split_once("://")
        .map_or(url.as_str(), |(_, rest)| rest)
        .to_owned();
    let mut items: Scenes = view
        .status
        .rooms
        .iter()
        .map(|room| {
            boxed(list_item(
                format!("{}'s room", room.host),
                format!("{}/{} players · {}", room.players, room.capacity, room.map),
                Action::SelectRoom(room.code.clone()),
            ))
        })
        .collect();
    if items.is_empty() {
        items.push(boxed(empty_preview(
            "No open public rooms. Create one, or join a private room with its code.",
        )));
    }
    let detail: Scenes = view.selected_room().map_or_else(
        || vec![boxed(hint("Select a room to see its details."))],
        |room| {
            vec![
                boxed(text(
                    format!("Selected room: {}'s room", room.host),
                    16.0,
                    INK,
                )),
                boxed(hint(format!("Host: {}", room.host))),
                boxed(hint(format!(
                    "{} · {}/{} players",
                    room.map, room.players, room.capacity
                ))),
                boxed(row(
                    bsn_list! { @wide_button("Join room", Action::JoinSelected, Tone::Primary) },
                )),
            ]
        },
    );
    bsn_list! {
        @heading({format!("Server: {host}")})
        --
        @row(bsn_list! {
            @badge("Connected", GOOD)
            --
            @button("Refresh", Action::RefreshRooms, Tone::Secondary)
            --
            @button("Disconnect", Action::Disconnect, Tone::Secondary)
        })
        --
        @split(items, detail)
        --
        @row(bsn_list! {
            @wide_button("Join by code", Action::OpenJoinByCode, Tone::Secondary)
            --
            @wide_button("+ Create room", Action::OpenCreateRoom, Tone::Secondary)
        })
    }
}

fn lobby_page(lobby: Lobby) -> impl SceneList {
    let slots: Vec<_> = lobby
        .slots
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            let you = index == usize::from(lobby.player);
            slot_row(index, slot.as_ref(), you)
        })
        .collect();
    let copy: Scenes = when(
        !cfg!(target_os = "android"),
        button("Copy code", Action::CopyCode, Tone::Secondary),
    );
    let ready_label = if lobby.ready() { "Not ready" } else { "Ready" };
    let ready_tone = if lobby.ready() {
        Tone::Secondary
    } else {
        Tone::Primary
    };
    let start_label = if lobby.is_host() {
        "Start match"
    } else {
        "Start (host only)"
    };
    let status = if lobby.joined() < lobby.slots.len() {
        format!(
            "Waiting for players: {} of {} joined. Share the invite code.",
            lobby.joined(),
            lobby.slots.len()
        )
    } else if !lobby.all_ready() {
        "The host starts once every player is ready.".to_owned()
    } else if lobby.is_host() {
        "Everyone is ready. Start the match.".to_owned()
    } else {
        "Everyone is ready. Waiting for the host to start.".to_owned()
    };
    let visibility = if lobby.public {
        "Public: listed in the room browser"
    } else {
        "Private: invite code only"
    };
    bsn_list! {
        @heading({format!("Room: {}'s room", lobby.host())})
        --
        @row(bsn_list! {
            @badge({format!("Invite {}", lobby.code)}, SIGNAL)
            --
            {copy}
            --
            @hint({format!("{} · Host: {}", visibility, lobby.host())})
        })
        --
        @column(8.0, slots)
        --
        @row(bsn_list! {
            @wide_button(ready_label, Action::ToggleReady, ready_tone)
            --
            @wide_button(start_label, Action::StartMatch, Tone::Primary)
            --
            @button("Leave room", Action::LeaveRoom, Tone::Secondary)
        })
        --
        @hint(status)
    }
}

fn slot_row(index: usize, slot: Option<&rts_net::LobbySlot>, you: bool) -> impl Scene {
    let (name, state, color) = slot.map_or_else(
        || (format!("Player {}", index + 1), "Open slot", INK_DISABLED),
        |slot| {
            let mut name = slot.name.clone();
            if index == 0 {
                name.push_str(" (host)");
            }
            if you {
                name.push_str(" (you)");
            }
            if slot.ready {
                (name, "Ready", GOOD)
            } else {
                (name, "Not ready", INK_MUTED)
            }
        },
    );
    let name_color = if slot.is_some() { INK } else { INK_MUTED };
    bsn! {
        Node {
            min_height: px(48),
            padding: UiRect::axes(px(14), px(6)),
            column_gap: px(12),
            align_items: AlignItems::Center,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
        }
        BackgroundColor(FIELD)
        BorderColor::all(EDGE)
        Children [
            @text(name, 15.0, name_color) Node { flex_grow: 1.0 }
            --
            @badge(state, color)
        ]
    }
}

fn settings_page(view: &View) -> impl SceneList {
    let name = view
        .form
        .name
        .clone()
        .unwrap_or_else(|| view.settings.name.clone());
    let field = text_field(Field::Name, &name, rts_net::MAX_NAME_CHARS);
    let scales: Vec<_> = UI_SCALES
        .iter()
        .map(|&scale| button(format!("{scale}%"), Action::SetUiScale(scale), Tone::Item))
        .collect();
    let fullscreen: Scenes = when(
        Settings::fullscreen_supported(),
        setting(
            "Fullscreen",
            "Fill the screen without window borders.",
            toggle(Action::SetFullscreen(true), Action::SetFullscreen(false)),
        ),
    );
    bsn_list! {
        @heading("Settings")
        --
        @panel(bsn_list! {
            @text("Player name", 16.0, INK)
            --
            Node { column_gap: px(8) } Children [ @{field} ]
            --
            @bound_text(name_hint, 13.0, INK_MUTED)
        })
        --
        @setting("Interface scale", "Size of text and controls.", row(scales))
        --
        @setting(
            "Edge scrolling",
            "Move the camera when the pointer touches a screen edge.",
            toggle(Action::SetEdgeScroll(true), Action::SetEdgeScroll(false)),
        )
        --
        {fullscreen}
    }
}

fn setting(title: &'static str, detail: &'static str, control: impl Scene) -> impl Scene {
    panel(bsn_list! {
        @text(title, 16.0, INK)
        --
        @hint(detail)
        --
        @{control}
    })
}

fn toggle(on: Action, off: Action) -> impl Scene {
    row(bsn_list! {
        @button("On", on, Tone::Item)
        --
        @button("Off", off, Tone::Item)
    })
}

// ---------------------------------------------------------------------------
// Pieces
// ---------------------------------------------------------------------------

/// A button that opens a list of choices below it.
fn dropdown(
    label: String,
    which: Dropdown,
    open: bool,
    options: Vec<(String, Action)>,
) -> impl Scene {
    let list: Scenes = if open {
        let items: Vec<_> = options
            .into_iter()
            .map(|(label, action)| {
                bsn! {
                    @button(label, action, Tone::Item)
                    Node { justify_content: JustifyContent::FlexStart }
                }
            })
            .collect();
        vec![boxed(bsn! {
            Node {
                position_type: PositionType::Absolute,
                top: percent(100),
                left: px(0),
                min_width: percent(100),
                margin: UiRect::top(px(4)),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(6)),
                row_gap: px(4),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
            }
            GlobalZIndex(4)
            BackgroundColor(PANEL)
            BorderColor::all(FIELD_EDGE)
            Children [ {items} ]
        })]
    } else {
        Vec::new()
    };
    bsn! {
        Node { flex_direction: FlexDirection::Column }
        Children [
            @button(label, Action::ToggleDropdown(which), Tone::Secondary)
            Children [
                Node {
                    width: px(7),
                    height: px(7),
                    margin: UiRect::bottom(px(3)),
                    border: {UiRect { right: px(2), bottom: px(2), ..default() }},
                }
                UiTransform { rotation: Rot2::degrees(45.0) }
                BorderColor::all(INK_MUTED)
            ]
            --
            {list}
        ]
    }
}

fn catalog_preview(view: &View, index: usize) -> impl Scene {
    let map = &view.catalog.0[index.min(view.catalog.0.len() - 1)];
    map_image(map.preview.clone(), map_aspect(&map.setup.map))
}

/// A record's map preview, from the catalog when the map is bundled.
fn record_preview(view: &View, record: &Record) -> Box<dyn SceneList> {
    let aspect = map_aspect(&record.meta.setup.map);
    view.catalog
        .0
        .iter()
        .find(|map| map.setup.map == record.meta.setup.map)
        .map_or_else(
            || boxed(preview_for(record, aspect)),
            |map| boxed(map_image(map.preview.clone(), aspect)),
        )
}

fn map_size(map: &str) -> String {
    rts_base::MapData::from_ron(map).map_or_else(
        |_| "unknown size".into(),
        |map| {
            let width = map.tiles.first().map_or(0, |row| row.chars().count());
            format!("{width}×{} tiles", map.tiles.len())
        },
    )
}

fn map_aspect(map: &str) -> f32 {
    rts_base::MapData::from_ron(map).map_or(1.5, |map| {
        let width = map
            .tiles
            .first()
            .map_or(1, |row| row.chars().count())
            .max(1);
        let height = map.tiles.len().max(1);
        #[expect(
            clippy::cast_precision_loss,
            reason = "map sides are at most a few thousand"
        )]
        let aspect = width as f32 / height as f32;
        aspect
    })
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

/// The open dialog over a dimmed screen.
pub fn dialog(view: &View) -> impl Scene {
    let contents: Box<dyn SceneList> = match &view.menu.dialog {
        Some(Dialog::JoinByCode) => boxed(join_by_code(view)),
        Some(Dialog::CreateRoom) => boxed(create_room()),
        Some(Dialog::Confirm(confirm)) => boxed(confirm_dialog(view, confirm)),
        None => Box::new(bsn_list! {}),
    };
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
        }
        Children [ {contents} ]
    }
}

fn join_by_code(view: &View) -> impl Scene {
    let field = text_field(Field::Code, &view.form.code, ROOM_CODE_LEN);
    dialog_frame(
        "Join room with invite code",
        bsn_list! {
            @hint({format!("Server: {}", view.server_url())})
            --
            Node { column_gap: px(8) } Children [ @{field} ]
            --
            @hint("6 characters · letters and digits · not case-sensitive")
            --
            @bound_text(dialog_error, 13.0, DANGER)
            --
            @row(bsn_list! {
                @wide_button("Cancel", Action::CloseDialog, Tone::Secondary)
                --
                @wide_button("Join room", Action::JoinByCode, Tone::Primary)
            })
        },
    )
}

fn create_room() -> impl Scene {
    dialog_frame(
        "Create room",
        bsn_list! {
            @hint("Who can find this room?")
            --
            @row(bsn_list! {
                @wide_button("Public", Action::SetPublic(true), Tone::Item)
                --
                @wide_button("Private", Action::SetPublic(false), Tone::Item)
            })
            --
            @bound_text(visibility_hint, 13.0, INK_MUTED)
            --
            @bound_text(dialog_error, 13.0, DANGER)
            --
            @row(bsn_list! {
                @wide_button("Cancel", Action::CloseDialog, Tone::Secondary)
                --
                @wide_button("Create room", Action::CreateRoom, Tone::Primary)
            })
        },
    )
}

fn visibility_hint(view: &View) -> String {
    if view.menu.public_room {
        "Listed in the room browser; anyone on this server can join.".into()
    } else {
        "Hidden from the room browser; players join with the invite code.".into()
    }
}

/// The latest error, shown inside dialogs so a failed join keeps the typed code visible.
fn dialog_error(view: &View) -> String {
    match &view.menu.notice {
        Some((message, true)) => message.clone(),
        _ => String::new(),
    }
}

fn confirm_dialog(view: &View, confirm: &Confirm) -> impl Scene {
    let (title, body, label) = match confirm {
        Confirm::Delete(shelf, id) => {
            let name = view
                .library
                .get(*shelf, id)
                .map_or_else(|| "this file".to_owned(), |record| record.meta.name.clone());
            (
                format!("Delete {name}?"),
                "This cannot be undone.".to_owned(),
                "Delete",
            )
        }
        Confirm::LeaveRoom => (
            "Leave this room?".to_owned(),
            if view.status.lobby.as_ref().is_some_and(Lobby::is_host) {
                "You are the host. Leaving closes the room for everyone.".to_owned()
            } else {
                "You can rejoin while the room is open.".to_owned()
            },
            "Leave room",
        ),
        Confirm::LeaveMatch => (
            "Leave the match?".to_owned(),
            if view.status.kind == crate::session::MatchKind::Online {
                "Leaving ends the match for every player.".to_owned()
            } else {
                "Progress since your last save is lost.".to_owned()
            },
            "Leave",
        ),
        Confirm::Quit => (
            "Quit the game?".to_owned(),
            "You are connected to a server. Quitting leaves it.".to_owned(),
            "Quit",
        ),
    };
    dialog_frame(
        title,
        bsn_list! {
            @hint(body)
            --
            @row(bsn_list! {
                @wide_button("Cancel", Action::CloseDialog, Tone::Secondary)
                --
                @wide_button(label, Action::Confirm, Tone::Danger)
            })
        },
    )
}
