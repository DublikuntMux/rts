//! Menu pages and dialogs as scenes, each a function of the menu model.

use bevy::{
    input_focus::tab_navigation::TabGroup, prelude::*, scene::EntityScene, ui_widgets::ScrollArea,
};
use rts_ai::Difficulty;
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
    locale::Localization,
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
        difficulty: Difficulty,
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
                SoloPage::NewGame => Self::NewGame {
                    map: menu.map,
                    difficulty: menu.difficulty,
                },
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
pub fn menu_root(compact: bool, loc: &Localization) -> impl Scene {
    let group = TabGroup::new(0);
    let dialog_shown = ShowIf(|view| view.menu.dialog.is_some());
    let rail_width = if compact { percent(100) } else { px(216) };
    let notice_shown = ShowIf(|view| view.menu.notice.is_some());
    let nav: Scenes = [
        (
            "nav.singleplayer",
            "nav.singleplayer_short",
            Section::Singleplayer,
        ),
        (
            "nav.multiplayer",
            "nav.multiplayer_short",
            Section::Multiplayer,
        ),
        ("nav.replays", "nav.replays", Section::Replays),
        ("nav.settings", "nav.settings", Section::Settings),
    ]
    .into_iter()
    .map(|(long, short, section)| {
        boxed(nav_button(
            loc.t(if compact { short } else { long }).to_owned(),
            Action::Section(section),
            compact,
        ))
    })
    .chain(std::iter::once(boxed(bsn! {
        @nav_button({loc.t("nav.quit").to_owned()}, Action::Quit, compact)
        Node { margin: {if compact { UiRect::ZERO } else { UiRect::top(auto()) }} }
    })))
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
            @title_bar(compact, loc)
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
fn title_bar(compact: bool, loc: &Localization) -> impl Scene {
    let can_go_back = ShowIf(|view| view.can_go_back());
    let back_label = loc.t("nav.back").to_owned();
    let back: Scenes = when(
        compact,
        bsn! {
            @button(back_label, Action::Back, Tone::Secondary)
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

fn nav_button(label: String, action: Action, compact: bool) -> impl Scene {
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
    let key = match menu.section {
        Section::Singleplayer => match menu.solo {
            SoloPage::Home => "breadcrumb.solo_home",
            SoloPage::NewGame => "breadcrumb.solo_new",
            SoloPage::LoadGame => "breadcrumb.solo_load",
        },
        Section::Multiplayer => match view.online_page() {
            OnlinePage::Connect => "breadcrumb.online_connect",
            OnlinePage::Connecting => "breadcrumb.online_connecting",
            OnlinePage::Rooms => "breadcrumb.online_rooms",
            OnlinePage::Lobby => "breadcrumb.online_lobby",
        },
        Section::Replays => "breadcrumb.replays",
        Section::Settings => "breadcrumb.settings",
    };
    view.loc.t(key).to_owned()
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
        PageKey::NewGame { map, .. } => Box::new(new_game(view, map)),
        PageKey::Browse { shelf, .. } => Box::new(browser(view, shelf)),
        PageKey::Connect { .. } => Box::new(connect(view)),
        PageKey::Connecting { url } => Box::new(connecting(view, url)),
        PageKey::Rooms { url, .. } => Box::new(rooms(view, url)),
        PageKey::Lobby(lobby) => Box::new(lobby_page(view, lobby)),
        PageKey::Settings { .. } => Box::new(settings_page(view)),
    }
}

fn solo_home(view: &View, latest: Option<Record>) -> impl SceneList {
    let loc = view.loc;
    let preferred = view.catalog.preferred();
    let preview = latest.as_ref().map_or_else(
        || boxed(catalog_preview(view, preferred)),
        |record| record_preview(view, record),
    );
    let map = &view.catalog.0[preferred];
    let (map_name, caption) = latest.map_or_else(
        || {
            let players = map.setup.players.to_string();
            let size = map_size(loc, &map.setup.map);
            (
                map.name.clone(),
                loc.fmt(
                    "solo.players_size",
                    &[("players", &players), ("size", &size)],
                ),
            )
        },
        |record| {
            let dur = duration(record.meta.ticks);
            (
                record.meta.map.clone(),
                loc.fmt(
                    "solo.latest_save",
                    &[("name", &record.meta.name), ("duration", &dur)],
                ),
            )
        },
    );
    bsn_list! {
        @heading({loc.t("solo.heading").to_owned()})
        --
        @hint({loc.t("solo.tagline").to_owned()})
        --
        @split(
            bsn_list! {
                @text({loc.t("solo.card_title").to_owned()}, 22.0, INK)
                --
                @hint({loc.t("solo.card_hint").to_owned()})
                --
                @wide_button({loc.t("solo.new_game").to_owned()}, Action::OpenNewGame, Tone::Primary)
                Node { margin: UiRect::top(px(16)), flex_grow: 0.0, flex_basis: auto() }
                --
                @wide_button({loc.t("solo.load_game").to_owned()}, Action::OpenLoadGame, Tone::Secondary)
                Node { flex_grow: 0.0, flex_basis: auto() }
                --
                @hint({loc.t("solo.load_hint").to_owned()})
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
    let loc = view.loc;
    let mut maps: Vec<_> = view.catalog.0.iter().enumerate().collect();
    maps.sort_by_key(|(_, map)| !map.skirmish);
    let maps: Vec<_> = maps
        .into_iter()
        .map(|(index, map)| {
            let size = map_size(loc, &map.setup.map);
            let mode = loc.t(if map.skirmish {
                "solo.mode_skirmish"
            } else {
                "solo.mode_sandbox"
            });
            list_item(
                map.name.clone(),
                format!("{mode} · {size}"),
                Action::SelectMap(index),
            )
        })
        .collect();
    let map = &view.catalog.0[selected.min(view.catalog.0.len() - 1)];
    let detail = loc
        .t(if map.skirmish {
            "solo.detail_skirmish"
        } else {
            "solo.detail_sandbox"
        })
        .to_owned();
    let start = loc
        .t(if map.skirmish {
            "solo.start_skirmish"
        } else {
            "solo.start_sandbox"
        })
        .to_owned();
    let difficulty: Scenes = when(map.skirmish, difficulty_choice(loc, view.menu.difficulty));
    let selected_label = loc.fmt("solo.selected_map", &[("map", &map.name)]);
    bsn_list! {
        @heading({loc.t("solo.select_map").to_owned()})
        --
        @hint({loc.t("solo.preview_hint").to_owned()})
        --
        @split(
            maps,
            bsn_list! {
                @catalog_preview(view, selected)
                --
                @text(selected_label, 16.0, INK)
                --
                @hint(detail)
                --
                {difficulty}
                --
                @row(bsn_list! { @wide_button(start, Action::StartGame, Tone::Primary) })
            },
        )
    }
}

/// Opponent difficulty buttons and what the chosen level changes.
fn difficulty_choice(loc: &Localization, chosen: Difficulty) -> impl Scene {
    let levels: Vec<_> = Difficulty::ALL
        .into_iter()
        .map(|level| {
            let label_key = match level {
                Difficulty::Easy => "difficulty.easy",
                Difficulty::Normal => "difficulty.normal",
                Difficulty::Hard => "difficulty.hard",
            };
            button(
                loc.t(label_key).to_owned(),
                Action::SelectDifficulty(level),
                Tone::Item,
            )
        })
        .collect();
    let desc_key = match chosen {
        Difficulty::Easy => "difficulty.easy_desc",
        Difficulty::Normal => "difficulty.normal_desc",
        Difficulty::Hard => "difficulty.hard_desc",
    };
    column(
        6.0,
        bsn_list! {
            @text({loc.t("solo.opponent").to_owned()}, 14.0, INK)
            --
            @row(levels)
            --
            @hint({loc.t(desc_key).to_owned()})
        },
    )
}

/// The save browser or the replay browser.
fn browser(view: &View, shelf: Shelf) -> impl SceneList {
    let loc = view.loc;
    let browse = view.menu.browse(shelf);
    let records = view.library.records(shelf);
    let has_last = shelf == Shelf::Replays && view.status.has_replay;
    let mut items: Scenes = Vec::new();
    if has_last && browse.map.is_none() {
        items.push(boxed(list_item(
            loc.t("browser.last_match").to_owned(),
            loc.t("browser.not_in_library").to_owned(),
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
        let key = match (shelf, records.is_empty()) {
            (Shelf::Saves, true) => "browser.empty_saves",
            (Shelf::Replays, true) => "browser.empty_replays",
            (_, false) => "browser.empty_filtered",
        };
        items.push(boxed(empty_preview(loc.t(key).to_owned())));
    }
    let title = loc
        .t(match shelf {
            Shelf::Saves => "browser.saves_title",
            Shelf::Replays => "browser.replays_title",
        })
        .to_owned();
    let footer: Scenes = match shelf {
        Shelf::Saves => vec![boxed(hint(loc.t("browser.filter_optional").to_owned()))],
        Shelf::Replays => Vec::new(),
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
    let loc = view.loc;
    let browse = view.menu.browse(shelf);
    let mut maps: Vec<String> = view
        .library
        .records(shelf)
        .iter()
        .map(|record| record.meta.map.clone())
        .collect();
    maps.sort();
    maps.dedup();
    let all_maps = loc.t("browser.all_maps").to_owned();
    let map_options: Vec<(String, Action)> =
        std::iter::once((all_maps.clone(), Action::FilterMap(shelf, None)))
            .chain(
                maps.into_iter()
                    .map(|map| (map.clone(), Action::FilterMap(shelf, Some(map)))),
            )
            .collect();
    let most_recent = loc.t("browser.most_recent").to_owned();
    let oldest_first = loc.t("browser.oldest_first").to_owned();
    let order_options = vec![
        (most_recent.clone(), Action::Order(shelf, false)),
        (oldest_first.clone(), Action::Order(shelf, true)),
    ];
    let map_label = browse.map.clone().unwrap_or(all_maps);
    let order_label = if browse.oldest_first {
        oldest_first
    } else {
        most_recent
    };
    let open = view.menu.dropdown;
    row(bsn_list! {
        @dropdown(map_label, Dropdown::Map(shelf), open == Some(Dropdown::Map(shelf)), map_options)
        --
        @dropdown(order_label, Dropdown::Order(shelf), open == Some(Dropdown::Order(shelf)), order_options)
    })
}

fn browser_detail(view: &View, shelf: Shelf) -> Scenes {
    let loc = view.loc;
    let browse = view.menu.browse(shelf);
    match (&browse.selected, view.selected_record(shelf)) {
        (Some(ReplayPick::Last), _) if shelf == Shelf::Replays => vec![
            boxed(empty_preview(loc.t("browser.last_match_detail").to_owned())),
            boxed(text(loc.t("browser.last_match").to_owned(), 16.0, INK)),
            boxed(row(bsn_list! {
                @wide_button({loc.t("browser.watch_replay").to_owned()}, Action::WatchReplay, Tone::Primary)
                --
                @button({loc.t("browser.save_to_library").to_owned()}, Action::SaveLastReplay, Tone::Secondary)
            })),
        ],
        (_, Some(record)) => {
            let meta = &record.meta;
            let title = format!("{} / {}", meta.name, meta.map);
            let dur = duration(meta.ticks);
            let players = meta.players.to_string();
            let detail = loc.fmt(
                if shelf == Shelf::Saves {
                    "browser.played_detail"
                } else {
                    "browser.duration_detail"
                },
                &[("duration", &dur), ("players", &players)],
            );
            let actions: Box<dyn SceneList> = match shelf {
                Shelf::Saves => boxed(row(bsn_list! {
                    @wide_button({loc.t("browser.load_save").to_owned()}, Action::LoadSave, Tone::Primary)
                    --
                    @button({loc.t("browser.delete_ellipsis").to_owned()}, Action::Delete(Shelf::Saves), Tone::Danger)
                })),
                Shelf::Replays => boxed(row(bsn_list! {
                    @wide_button({loc.t("browser.watch_replay").to_owned()}, Action::WatchReplay, Tone::Primary)
                    --
                    @button({loc.t("browser.delete_ellipsis").to_owned()}, Action::Delete(Shelf::Replays), Tone::Danger)
                })),
            };
            vec![
                record_preview(view, record),
                boxed(text(title, 16.0, INK)),
                boxed(hint(detail)),
                actions,
            ]
        }
        _ => vec![boxed(empty_preview(
            loc.t(match shelf {
                Shelf::Saves => "browser.select_save",
                Shelf::Replays => "browser.select_replay",
            })
            .to_owned(),
        ))],
    }
}

fn connect(view: &View) -> impl SceneList {
    let loc = view.loc;
    let recent: Scenes = view
        .settings
        .recent_server
        .as_ref()
        .map_or_else(Vec::new, |url| {
            vec![boxed(button(
                loc.fmt("online.recent", &[("url", url)]),
                Action::UseRecentServer,
                Tone::Secondary,
            ))]
        });
    let field = text_field(Field::Url, &view.form.url, MAX_URL_LEN);
    let failed = ShowIf(|view| matches!(view.status.server, Server::Failed { .. }));
    bsn_list! {
        @heading({loc.t("online.connect_heading").to_owned()})
        --
        @panel(bsn_list! {
            @hint({loc.t("online.server_address").to_owned()})
            --
            Node { column_gap: px(8) } Children [ @{field} ]
            --
            @hint({loc.t("online.address_hint").to_owned()})
            --
            @row(bsn_list! {
                {recent}
                --
                @wide_button({loc.t("online.connect").to_owned()}, Action::Connect, Tone::Primary)
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
            @row(bsn_list! { @button({loc.t("online.retry").to_owned()}, Action::Connect, Tone::Secondary) })
            --
            @hint({loc.t("online.retry_hint").to_owned()})
        ]
        --
        @hint({loc.t("online.join_code_hint").to_owned()})
        --
        @hint({loc.t("online.self_host_hint").to_owned()})
    }
}

fn connect_failure(view: &View) -> String {
    match &view.status.server {
        Server::Failed { url, reason } => view
            .loc
            .fmt("online.connect_failed", &[("url", url), ("reason", reason)]),
        _ => String::new(),
    }
}

fn connecting(view: &View, url: String) -> impl SceneList {
    let loc = view.loc;
    bsn_list! {
        @heading({loc.t("online.connecting_heading").to_owned()})
        --
        @panel(bsn_list! {
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(14),
                padding: UiRect::all(px(16)),
            }
            Children [
                @text({loc.t("online.connecting").to_owned()}, 18.0, INK)
                --
                @hint(url)
                --
                @spinner()
            ]
            --
            @row(bsn_list! { @wide_button({loc.t("online.cancel").to_owned()}, Action::Disconnect, Tone::Secondary) })
        })
        --
        @hint({loc.t("online.connecting_hint").to_owned()})
    }
}

fn rooms(view: &View, url: String) -> impl SceneList {
    let loc = view.loc;
    let host = url
        .split_once("://")
        .map_or(url.as_str(), |(_, rest)| rest)
        .to_owned();
    let mut items: Scenes = view
        .status
        .rooms
        .iter()
        .map(|room| {
            let players = room.players.to_string();
            let capacity = room.capacity.to_string();
            boxed(list_item(
                loc.fmt("online.room_name", &[("host", &room.host)]),
                loc.fmt(
                    "online.room_summary",
                    &[
                        ("players", &players),
                        ("capacity", &capacity),
                        ("map", &room.map),
                    ],
                ),
                Action::SelectRoom(room.code.clone()),
            ))
        })
        .collect();
    if items.is_empty() {
        items.push(boxed(empty_preview(loc.t("online.no_rooms").to_owned())));
    }
    let detail: Scenes = view.selected_room().map_or_else(
        || vec![boxed(hint(loc.t("online.select_room").to_owned()))],
        |room| {
            let players = room.players.to_string();
            let capacity = room.capacity.to_string();
            vec![
                boxed(text(
                    loc.fmt("online.selected_room", &[("host", &room.host)]),
                    16.0,
                    INK,
                )),
                boxed(hint(loc.fmt("online.host_label", &[("host", &room.host)]))),
                boxed(hint(loc.fmt(
                    "online.room_detail",
                    &[
                        ("map", &room.map),
                        ("players", &players),
                        ("capacity", &capacity),
                    ],
                ))),
                boxed(row(bsn_list! {
                    @wide_button({loc.t("online.join_room").to_owned()}, Action::JoinSelected, Tone::Primary)
                })),
            ]
        },
    );
    bsn_list! {
        @heading({loc.fmt("online.server_heading", &[("host", &host)])})
        --
        @row(bsn_list! {
            @badge({loc.t("online.connected").to_owned()}, GOOD)
            --
            @button({loc.t("online.refresh").to_owned()}, Action::RefreshRooms, Tone::Secondary)
            --
            @button({loc.t("online.disconnect").to_owned()}, Action::Disconnect, Tone::Secondary)
        })
        --
        @split(items, detail)
        --
        @row(bsn_list! {
            @wide_button({loc.t("online.join_by_code").to_owned()}, Action::OpenJoinByCode, Tone::Secondary)
            --
            @wide_button({loc.t("online.create_room_btn").to_owned()}, Action::OpenCreateRoom, Tone::Secondary)
        })
    }
}

fn lobby_page(view: &View, lobby: Lobby) -> impl SceneList {
    let loc = view.loc;
    let slots: Vec<_> = lobby
        .slots
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            let you = index == usize::from(lobby.player);
            slot_row(loc, index, slot.as_ref(), you)
        })
        .collect();
    let copy: Scenes = when(
        !cfg!(target_os = "android"),
        button(
            loc.t("lobby.copy_code").to_owned(),
            Action::CopyCode,
            Tone::Secondary,
        ),
    );
    let ready_label = loc
        .t(if lobby.ready() {
            "lobby.not_ready"
        } else {
            "lobby.ready"
        })
        .to_owned();
    let ready_tone = if lobby.ready() {
        Tone::Secondary
    } else {
        Tone::Primary
    };
    let start_label = loc
        .t(if lobby.is_host() {
            "lobby.start_match"
        } else {
            "lobby.start_host_only"
        })
        .to_owned();
    let status = if lobby.joined() < lobby.slots.len() {
        let joined = lobby.joined().to_string();
        let total = lobby.slots.len().to_string();
        loc.fmt(
            "lobby.waiting_players",
            &[("joined", &joined), ("total", &total)],
        )
    } else if !lobby.all_ready() {
        loc.t("lobby.waiting_ready").to_owned()
    } else if lobby.is_host() {
        loc.t("lobby.all_ready_host").to_owned()
    } else {
        loc.t("lobby.all_ready_guest").to_owned()
    };
    let visibility = loc.t(if lobby.public {
        "lobby.public_desc"
    } else {
        "lobby.private_desc"
    });
    let host = lobby.host();
    bsn_list! {
        @heading({loc.fmt("lobby.heading", &[("host", host)])})
        --
        @row(bsn_list! {
            @badge({loc.fmt("lobby.invite_badge", &[("code", &lobby.code)])}, SIGNAL)
            --
            {copy}
            --
            @hint({loc.fmt("lobby.meta", &[("visibility", visibility), ("host", host)])})
        })
        --
        @column(8.0, slots)
        --
        @row(bsn_list! {
            @wide_button(ready_label, Action::ToggleReady, ready_tone)
            --
            @wide_button(start_label, Action::StartMatch, Tone::Primary)
            --
            @button({loc.t("lobby.leave_room").to_owned()}, Action::LeaveRoom, Tone::Secondary)
        })
        --
        @hint(status)
    }
}

fn slot_row(
    loc: &Localization,
    index: usize,
    slot: Option<&rts_net::LobbySlot>,
    you: bool,
) -> impl Scene {
    let (name, state, color) = slot.map_or_else(
        || {
            let n = (index + 1).to_string();
            (
                loc.fmt("lobby.player_slot", &[("n", &n)]),
                loc.t("lobby.open_slot").to_owned(),
                INK_DISABLED,
            )
        },
        |slot| {
            let mut name = slot.name.clone();
            if index == 0 {
                name.push_str(loc.t("lobby.tag_host"));
            }
            if you {
                name.push_str(loc.t("lobby.tag_you"));
            }
            if slot.ready {
                (name, loc.t("lobby.ready").to_owned(), GOOD)
            } else {
                (name, loc.t("lobby.not_ready").to_owned(), INK_MUTED)
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
    let loc = view.loc;
    let name = view
        .form
        .name
        .clone()
        .unwrap_or_else(|| view.settings.name.clone());
    let field = text_field(Field::Name, &name, rts_net::MAX_NAME_CHARS);
    let languages: Vec<_> = loc
        .languages()
        .into_iter()
        .map(|(code, label)| button(label, Action::SetLanguage(code), Tone::Item))
        .collect();
    let scales: Vec<_> = UI_SCALES
        .iter()
        .map(|&scale| button(format!("{scale}%"), Action::SetUiScale(scale), Tone::Item))
        .collect();
    let fullscreen: Scenes = when(
        Settings::fullscreen_supported(),
        setting(
            loc.t("settings.fullscreen").to_owned(),
            loc.t("settings.fullscreen_detail").to_owned(),
            toggle(
                loc,
                Action::SetFullscreen(true),
                Action::SetFullscreen(false),
            ),
        ),
    );
    bsn_list! {
        @heading({loc.t("settings.heading").to_owned()})
        --
        @panel(bsn_list! {
            @text({loc.t("settings.player_name").to_owned()}, 16.0, INK)
            --
            Node { column_gap: px(8) } Children [ @{field} ]
            --
            @bound_text(name_hint, 13.0, INK_MUTED)
        })
        --
        @setting(
            {loc.t("settings.language").to_owned()},
            {loc.t("settings.language_detail").to_owned()},
            row(languages),
        )
        --
        @setting(
            {loc.t("settings.ui_scale").to_owned()},
            {loc.t("settings.ui_scale_detail").to_owned()},
            row(scales),
        )
        --
        @setting(
            {loc.t("settings.edge_scroll").to_owned()},
            {loc.t("settings.edge_scroll_detail").to_owned()},
            toggle(loc, Action::SetEdgeScroll(true), Action::SetEdgeScroll(false)),
        )
        --
        {fullscreen}
    }
}

fn setting(title: String, detail: String, control: impl Scene) -> impl Scene {
    panel(bsn_list! {
        @text(title, 16.0, INK)
        --
        @hint(detail)
        --
        @{control}
    })
}

fn toggle(loc: &Localization, on: Action, off: Action) -> impl Scene {
    row(bsn_list! {
        @button({loc.t("settings.on").to_owned()}, on, Tone::Item)
        --
        @button({loc.t("settings.off").to_owned()}, off, Tone::Item)
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
    if record.meta.setup.map.is_empty() {
        return boxed(empty_preview(
            view.loc.t("browser.private_replay_preview").to_owned(),
        ));
    }
    let aspect = map_aspect(&record.meta.setup.map);
    view.catalog
        .0
        .iter()
        .find(|map| map.setup == record.meta.setup)
        .map_or_else(
            || boxed(preview_for(record, aspect)),
            |map| boxed(map_image(map.preview.clone(), aspect)),
        )
}

fn map_size(loc: &Localization, map: &str) -> String {
    rts_base::MapData::from_ron(map).map_or_else(
        |_| loc.t("solo.unknown_size").to_owned(),
        |map| {
            let width = map
                .tiles
                .first()
                .map_or(0, |row| row.chars().count())
                .to_string();
            let height = map.tiles.len().to_string();
            loc.fmt("solo.map_tiles", &[("width", &width), ("height", &height)])
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
        Some(Dialog::CreateRoom) => boxed(create_room(view)),
        Some(Dialog::Confirm(confirm)) => boxed(confirm_dialog(view, confirm)),
        Some(Dialog::CrashReport) => boxed(crash_dialog(view)),
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
    let loc = view.loc;
    let field = text_field(Field::Code, &view.form.code, ROOM_CODE_LEN);
    dialog_frame(
        loc.t("dialog.join_code_title").to_owned(),
        bsn_list! {
            @hint({loc.fmt("online.server_heading", &[("host", view.server_url())])})
            --
            Node { column_gap: px(8) } Children [ @{field} ]
            --
            @hint({loc.t("dialog.join_code_hint").to_owned()})
            --
            @bound_text(dialog_error, 13.0, DANGER)
            --
            @row(bsn_list! {
                @wide_button({loc.t("online.cancel").to_owned()}, Action::CloseDialog, Tone::Secondary)
                --
                @wide_button({loc.t("online.join_room").to_owned()}, Action::JoinByCode, Tone::Primary)
            })
        },
    )
}

fn create_room(view: &View) -> impl Scene {
    let loc = view.loc;
    dialog_frame(
        loc.t("dialog.create_room_title").to_owned(),
        bsn_list! {
            @hint({loc.t("dialog.create_room_who").to_owned()})
            --
            @row(bsn_list! {
                @wide_button({loc.t("dialog.public").to_owned()}, Action::SetPublic(true), Tone::Item)
                --
                @wide_button({loc.t("dialog.private").to_owned()}, Action::SetPublic(false), Tone::Item)
            })
            --
            @bound_text(visibility_hint, 13.0, INK_MUTED)
            --
            @bound_text(dialog_error, 13.0, DANGER)
            --
            @row(bsn_list! {
                @wide_button({loc.t("online.cancel").to_owned()}, Action::CloseDialog, Tone::Secondary)
                --
                @wide_button({loc.t("dialog.create_room_title").to_owned()}, Action::CreateRoom, Tone::Primary)
            })
        },
    )
}

fn visibility_hint(view: &View) -> String {
    view.loc
        .t(if view.menu.public_room {
            "dialog.public_hint"
        } else {
            "dialog.private_hint"
        })
        .to_owned()
}

/// A report left by a previous run, offered for export.
fn crash_dialog(view: &View) -> impl Scene {
    let loc = view.loc;
    let summary = view.crash.pending().map_or_else(String::new, crash_summary);
    dialog_frame(
        loc.t("dialog.crash_title").to_owned(),
        bsn_list! {
            @hint({loc.t("dialog.crash_intro").to_owned()})
            --
            @hint(summary)
            --
            @bound_text(dialog_error, 13.0, DANGER)
            --
            @row(bsn_list! {
                @wide_button({loc.t("dialog.crash_export").to_owned()}, Action::ExportCrash, Tone::Primary)
                --
                @wide_button({loc.t("dialog.crash_discard").to_owned()}, Action::DiscardCrash, Tone::Secondary)
            })
            --
            @hint({loc.t("dialog.crash_footer").to_owned()})
        },
    )
}

/// When and why a stored report was written.
fn crash_summary(report: &str) -> String {
    let failures: Vec<&str> = report
        .lines()
        .filter(|line| line.starts_with("Time: ") || line.starts_with("Panic: "))
        .collect();
    if !failures.is_empty() {
        return failures.join("\n");
    }
    report
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// The latest error, shown inside dialogs so a failed join keeps the typed code visible.
fn dialog_error(view: &View) -> String {
    match &view.menu.notice {
        Some((message, true)) => message.clone(),
        _ => String::new(),
    }
}

fn confirm_dialog(view: &View, confirm: &Confirm) -> impl Scene {
    let loc = view.loc;
    let (title, body, label) = match confirm {
        Confirm::Delete(shelf, id) => {
            let name = view.library.get(*shelf, id).map_or_else(
                || loc.t("dialog.delete_fallback").to_owned(),
                |record| record.meta.name.clone(),
            );
            (
                loc.fmt("dialog.delete_title", &[("name", &name)]),
                loc.t("dialog.delete_body").to_owned(),
                loc.t("dialog.delete_confirm").to_owned(),
            )
        }
        Confirm::LeaveRoom => (
            loc.t("dialog.leave_room_title").to_owned(),
            loc.t(if view.status.lobby.as_ref().is_some_and(Lobby::is_host) {
                "dialog.leave_room_host"
            } else {
                "dialog.leave_room_guest"
            })
            .to_owned(),
            loc.t("lobby.leave_room").to_owned(),
        ),
        Confirm::LeaveMatch => (
            loc.t("dialog.leave_match_title").to_owned(),
            loc.t(if view.status.kind == crate::session::MatchKind::Online {
                "dialog.leave_match_online"
            } else {
                "dialog.leave_match_solo"
            })
            .to_owned(),
            loc.t("dialog.leave_confirm").to_owned(),
        ),
        Confirm::Quit => (
            loc.t("dialog.quit_title").to_owned(),
            loc.t("dialog.quit_body").to_owned(),
            loc.t("nav.quit").to_owned(),
        ),
    };
    dialog_frame(
        title,
        bsn_list! {
            @hint(body)
            --
            @row(bsn_list! {
                @wide_button({loc.t("online.cancel").to_owned()}, Action::CloseDialog, Tone::Secondary)
                --
                @wide_button(label, Action::Confirm, Tone::Danger)
            })
        },
    )
}
