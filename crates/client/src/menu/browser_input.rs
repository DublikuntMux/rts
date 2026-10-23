//! Native browser editors over the canvas fields. The input itself receives the tap, so
//! opening a touch keyboard does not depend on focus requested in a later Bevy frame.

use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use bevy::{
    input_focus::{
        FocusCause, InputFocus, InputFocusVisible,
        tab_navigation::{NavAction, TabNavigation},
    },
    prelude::*,
    text::EditableText,
    ui::{CalculatedClip, InteractionDisabled, UiSystems},
    window::PrimaryWindow,
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Event, HtmlElement, HtmlInputElement, KeyboardEvent};

use super::{Action, DialogHost, Field, Menu, MenuForm, Settings, store_field_value};

/// Uses Bevy's text model without also installing its canvas keyboard editor.
#[derive(Component, Clone, Default)]
#[require(EditableText)]
pub struct BrowserTextInput;

pub fn install(app: &mut App) {
    app.init_non_send::<BrowserInputs>()
        .add_systems(Update, read_events.before(super::perform))
        .add_systems(PostUpdate, sync_inputs.after(UiSystems::PostLayout));
}

enum Edit {
    Value(String),
    Focus,
    Blur,
    Submit,
    Back,
    Tab(bool),
}

type Events = Rc<RefCell<Vec<(Entity, Edit)>>>;

#[derive(Default)]
struct BrowserInputs {
    inputs: BTreeMap<Entity, BrowserInput>,
    events: Events,
}

const EVENTS: [&str; 6] = [
    "input",
    "compositionstart",
    "compositionend",
    "keydown",
    "focus",
    "blur",
];

struct BrowserInput {
    element: HtmlInputElement,
    listener: Closure<dyn FnMut(Event)>,
    composing: Rc<std::cell::Cell<bool>>,
}

impl BrowserInput {
    fn new(
        entity: Entity,
        field: Field,
        text: &EditableText,
        events: Events,
    ) -> Result<Self, JsValue> {
        let document = web_sys::window().unwrap().document().unwrap();
        let element: HtmlInputElement = document.create_element("input")?.dyn_into()?;
        // type=text retains selection APIs for every field; inputmode chooses the keyboard.
        element.set_type("text");
        let (label, mode, capitalize) = match field {
            Field::Url => ("Server address", "url", "off"),
            Field::Code => ("Room code", "text", "characters"),
            Field::Name => ("Player name", "text", "words"),
        };
        for (attribute, value) in [
            ("aria-label", label),
            ("inputmode", mode),
            ("autocapitalize", capitalize),
            ("autocomplete", "off"),
            ("autocorrect", "off"),
            ("spellcheck", "false"),
            (
                "enterkeyhint",
                if field == Field::Name { "done" } else { "go" },
            ),
        ] {
            element.set_attribute(attribute, value)?;
        }
        element.set_value(&text.value().to_string());
        // HTML maxlength counts UTF-16 code units. Enforce the game's character limit
        // in normalize instead, so supplementary characters in names count once.
        let max = text.max_characters.unwrap_or(usize::MAX);
        let composing = Rc::new(std::cell::Cell::new(false));
        let input = element.clone();
        let composition = composing.clone();
        let listener = Closure::new(move |event: Event| {
            let edit = match event.type_().as_str() {
                "compositionstart" => {
                    composition.set(true);
                    return;
                }
                "compositionend" => {
                    composition.set(false);
                    normalize(&input, field, max)
                }
                "input" if !composition.get() => normalize(&input, field, max),
                "focus" => Edit::Focus,
                "blur" => {
                    composition.set(false);
                    events
                        .borrow_mut()
                        .push((entity, normalize(&input, field, max)));
                    Edit::Blur
                }
                "keydown" => {
                    let key: &KeyboardEvent = event.unchecked_ref();
                    if key.is_composing() || composition.get() || key.repeat() {
                        return;
                    }
                    match key.key().as_str() {
                        "Enter" => Edit::Submit,
                        "Escape" => Edit::Back,
                        "Tab" => Edit::Tab(key.shift_key()),
                        _ => return,
                    }
                }
                _ => return,
            };
            if event.type_() == "keydown" {
                event.prevent_default();
            }
            events.borrow_mut().push((entity, edit));
        });
        let input = Self {
            element,
            listener,
            composing,
        };
        for event in EVENTS {
            input
                .element
                .add_event_listener_with_callback(event, input.listener.as_ref().unchecked_ref())?;
        }
        document.body().unwrap().append_child(&input.element)?;
        Ok(input)
    }
}

impl Drop for BrowserInput {
    fn drop(&mut self) {
        let document = web_sys::window().and_then(|window| window.document());
        let focused = document
            .as_ref()
            .and_then(web_sys::Document::active_element)
            .is_some_and(|active| active.is_same_node(Some(&self.element)));
        for event in EVENTS {
            let _ = self
                .element
                .remove_event_listener_with_callback(event, self.listener.as_ref().unchecked_ref());
        }
        self.element.remove();
        // A form submitted from the HTML editor can replace the page without a
        // canvas click. Return keyboard events to winit when that editor disappears.
        if focused
            && let Some(document) = document
            && let Ok(Some(canvas)) = document.query_selector("canvas")
            && let Some(canvas) = canvas.dyn_ref::<HtmlElement>()
        {
            let _ = canvas.focus();
        }
    }
}

fn normalize(input: &HtmlInputElement, field: Field, max: usize) -> Edit {
    let raw = input.value();
    let value = field.sanitize(&raw, max);
    if raw != value {
        // DOM selection offsets use UTF-16; preserve the caret after filtering a paste.
        let selection = input.selection_start().ok().flatten().map(|start| {
            let prefix = String::from_utf16_lossy(
                &raw.encode_utf16().take(start as usize).collect::<Vec<_>>(),
            );
            u32::try_from(field.sanitize(&prefix, max).encode_utf16().count()).unwrap_or(u32::MAX)
        });
        input.set_value(&value);
        if let Some(caret) = selection {
            let _ = input.set_selection_range(caret, caret);
        }
    }
    Edit::Value(value)
}

fn read_events(
    inputs: NonSend<BrowserInputs>,
    mut fields: Query<(&Field, &mut EditableText)>,
    mut form: ResMut<MenuForm>,
    mut settings: ResMut<Settings>,
    mut focus: ResMut<InputFocus>,
    mut visible: ResMut<InputFocusVisible>,
    navigation: TabNavigation,
    mut actions: MessageWriter<Action>,
) {
    for (entity, edit) in inputs.events.take() {
        let Ok((&field, mut text)) = fields.get_mut(entity) else {
            continue;
        };
        match edit {
            Edit::Value(value) => {
                if text.value() != value.as_str() {
                    text.editor_mut().set_text(&value);
                    store_field_value(field, value, &mut form, &mut settings);
                }
            }
            Edit::Focus => focus.set(entity, FocusCause::Pressed),
            Edit::Blur if focus.get() == Some(entity) => focus.clear(),
            Edit::Submit => match field {
                Field::Url => {
                    actions.write(Action::Connect);
                }
                Field::Code => {
                    actions.write(Action::JoinByCode);
                }
                Field::Name => {
                    let _ = inputs.inputs[&entity].element.blur();
                }
            },
            Edit::Back => {
                actions.write(Action::Back);
                focus.clear();
            }
            Edit::Tab(reverse) => {
                let direction = if reverse {
                    NavAction::Previous
                } else {
                    NavAction::Next
                };
                if let Ok(next) = navigation.navigate(&focus, direction) {
                    focus.set(next, FocusCause::Navigated);
                    visible.0 = true;
                }
            }
            Edit::Blur => {}
        }
    }
}

type FieldLayout<'a> = (
    Entity,
    &'a Field,
    &'a EditableText,
    &'a ComputedNode,
    &'a UiGlobalTransform,
    &'a InheritedVisibility,
    Option<&'a CalculatedClip>,
    Has<InteractionDisabled>,
);

fn sync_inputs(
    mut inputs: NonSendMut<BrowserInputs>,
    fields: Query<FieldLayout, With<BrowserTextInput>>,
    parents: Query<&ChildOf>,
    dialogs: Query<(), With<DialogHost>>,
    menu: Res<Menu>,
    window: Single<&Window, With<PrimaryWindow>>,
    focus: Res<InputFocus>,
) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(Some(canvas)) = document.query_selector("canvas") else {
        return;
    };
    let bounds = canvas.get_bounding_client_rect();
    let scale = bounds.width() / f64::from(window.physical_width().max(1));
    let css = |physical: f32| f64::from(physical) * scale;
    let mut keep = Vec::new();
    for (entity, &field, text, node, transform, visibility, clip, disabled) in &fields {
        if !visibility.get()
            || node.is_empty()
            || clip.is_some_and(CalculatedClip::is_fully_clipped)
            || (menu.dialog.is_some()
                && !parents
                    .iter_ancestors(entity)
                    .any(|parent| dialogs.contains(parent)))
        {
            continue;
        }
        keep.push(entity);
        if !inputs.inputs.contains_key(&entity) {
            match BrowserInput::new(entity, field, text, inputs.events.clone()) {
                Ok(input) => {
                    inputs.inputs.insert(entity, input);
                }
                Err(error) => {
                    warn!(?error, "Could not create browser text input");
                    continue;
                }
            }
        }
        let input = &inputs.inputs[&entity];
        let element = &input.element;
        let border = node.border();
        let rect = Rect {
            min: transform.affine().translation - node.size() * 0.5 + border.min_inset,
            max: transform.affine().translation + node.size() * 0.5 - border.max_inset,
        };
        let mut visible_rect = rect;
        if let Some(CalculatedClip::Rects(clips)) = clip {
            for clip in clips {
                let transform = clip.world_to_clip_local.inverse();
                visible_rect = visible_rect.intersect(Rect::from_corners(
                    transform.transform_point2(clip.rect.min),
                    transform.transform_point2(clip.rect.max),
                ));
            }
        }
        let padding = node.padding;
        let style = format!(
            "position:fixed;box-sizing:border-box;border:0;outline:0;margin:0;border-radius:0;\
             background:#182225;color:{};caret-color:#dfb869;font:{}px sans-serif;\
             left:{}px;top:{}px;width:{}px;height:{}px;padding:{}px {}px {}px {}px;\
             clip-path:inset({}px {}px {}px {}px);",
            if disabled { "#708484" } else { "#edefe1" },
            css(node.size().y / 3.0).max(16.0),
            bounds.left() + css(rect.min.x),
            bounds.top() + css(rect.min.y),
            css(rect.width()),
            css(rect.height()),
            css(padding.min_inset.y),
            css(padding.max_inset.x),
            css(padding.max_inset.y),
            css(padding.min_inset.x),
            css((visible_rect.min.y - rect.min.y).max(0.0)),
            css((rect.max.x - visible_rect.max.x).max(0.0)),
            css((rect.max.y - visible_rect.max.y).max(0.0)),
            css((visible_rect.min.x - rect.min.x).max(0.0)),
        );
        let _ = element.set_attribute("style", &style);
        element.set_disabled(disabled);
        if !input.composing.get() && text.value() != element.value().as_str() {
            element.set_value(&text.value().to_string());
        }
        let active = document
            .active_element()
            .is_some_and(|active| active.is_same_node(Some(element)));
        if focus.get() == Some(entity) && !active && !disabled {
            let _ = element.focus();
        } else if focus.get() != Some(entity) && active {
            let _ = element.blur();
            if let Some(canvas) = canvas.dyn_ref::<HtmlElement>() {
                let _ = canvas.focus();
            }
        }
    }
    inputs.inputs.retain(|entity, _| keep.contains(entity));
}
