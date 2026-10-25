//! How an API, a folder or the project signs in (#66): inherited, none,
//! Basic, a bearer token, or an API key in a header or the query (a gRPC
//! call's as metadata). Values are templates, offering the hints as one
//! types `{{` (M6); the secrets show as dots till shown. An API's changes go
//! to it at once, and its view is made again when its request is replaced
//! (the history, a cURL command); a folder's and the project's wait for
//! their dialog's Save. Inheriting says what it stands for, and where from.

use crate::states::{ApiState, WorkspaceState};
use crate::views::settings::Choice;
use crate::views::template_hints::{self, FieldHints, HintKey, HintSource, Hints};
use cyberapi_core::ItemId;
use cyberapi_core::auth::{Auth, AuthKind, KeyPlace};
use cyberapi_core::inherit::AuthSource;
use cyberapi_core::model::HttpMethod;
use cyberapi_core::template_hints::Hint;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme as _, IndexPath, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, Window, div, rems,
};
use rust_i18n::t;

type Choices = Entity<SelectState<Vec<Choice>>>;

const PLACES: [KeyPlace; 2] = [KeyPlace::Header, KeyPlace::Query];

/// The fields whose values are templates, in the order of their index in
/// the open hints (`FieldHints::field`).
const USERNAME: usize = 0;
const PASSWORD: usize = 1;
const TOKEN: usize = 2;
const KEY_VALUE: usize = 3;

/// Whose auth it edits.
#[derive(Clone)]
pub enum AuthFor {
    /// An API's, written to it as it changes.
    Api(Entity<ApiState>),
    /// A folder's, kept till its dialog saves.
    Folder(ItemId),
    /// The project's, kept till its dialog saves: nothing is above it.
    Project,
}

/// The project has nothing to inherit: its "none" is kept as inheriting,
/// which is the same and leaves it nothing to keep.
const PROJECT_KINDS: [AuthKind; 4] = [AuthKind::None, AuthKind::Basic, AuthKind::Bearer, AuthKind::ApiKey];

pub struct AuthEditor {
    owner: AuthFor,
    workspace: Entity<WorkspaceState>,
    /// The kinds its list offers, in order.
    kinds: &'static [AuthKind],
    kind: Choices,
    place: Choices,
    /// Basic's user and password, the token, the key's value: templates.
    templated: [Entity<InputState>; 4],
    key_name: Entity<InputState>,
    hints: HintSource,
    /// The hints open under one of `templated`.
    menu: Option<FieldHints>,
    /// Hints that wait for an answer a `get` path reads.
    hints_later: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

fn kind_label(kind: AuthKind) -> String {
    match kind {
        AuthKind::Inherit => t!("editor.auth_inherit"),
        AuthKind::None => t!("editor.auth_none"),
        AuthKind::Basic => t!("editor.auth_basic"),
        AuthKind::Bearer => t!("editor.auth_bearer"),
        AuthKind::ApiKey => t!("editor.auth_api_key"),
    }
    .to_string()
}

fn place_label(place: KeyPlace) -> String {
    match place {
        KeyPlace::Header => t!("editor.auth_in_header"),
        KeyPlace::Query => t!("editor.auth_in_query"),
    }
    .to_string()
}

fn choices(labels: impl IntoIterator<Item = String>) -> Vec<Choice> {
    labels
        .into_iter()
        .enumerate()
        .map(|(index, label)| Choice {
            label: label.into(),
            index,
        })
        .collect()
}

impl AuthEditor {
    pub fn new(owner: AuthFor, workspace: Entity<WorkspaceState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let auth = match &owner {
            AuthFor::Api(api) => api.read(cx).request().auth.clone(),
            AuthFor::Folder(id) => workspace
                .read(cx)
                .folder(id)
                .map(|folder| folder.settings.auth.clone())
                .unwrap_or_default(),
            AuthFor::Project => workspace.read(cx).project_settings().auth.clone(),
        };
        let kinds: &'static [AuthKind] = match owner {
            AuthFor::Project => &PROJECT_KINDS,
            _ => &Auth::KINDS,
        };
        let shown = match auth.kind() {
            AuthKind::Inherit if matches!(owner, AuthFor::Project) => AuthKind::None,
            kind => kind,
        };
        let kind_index = kinds.iter().position(|kind| *kind == shown).unwrap_or(0);
        let kind = cx.new(|cx| {
            SelectState::new(
                choices(kinds.iter().map(|kind| kind_label(*kind))),
                Some(IndexPath::new(kind_index)),
                window,
                cx,
            )
        });
        let hints = HintSource::new(&workspace);
        let (mut username, mut password, mut token, mut name, mut value, mut place_now) = Default::default();
        match &auth {
            Auth::Basic {
                username: user,
                password: secret,
            } => (username, password) = (user.clone(), secret.clone()),
            Auth::Bearer { token: own } => token = own.clone(),
            Auth::ApiKey {
                name: own,
                value: key,
                place,
            } => (name, value, place_now) = (own.clone(), key.clone(), *place),
            Auth::Inherit | Auth::None => {}
        }
        let place_index = PLACES.iter().position(|place| *place == place_now).unwrap_or(0);
        let place = cx.new(|cx| {
            SelectState::new(
                choices(PLACES.map(place_label)),
                Some(IndexPath::new(place_index)),
                window,
                cx,
            )
        });
        let field = |text: String, label: String, masked: bool, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(label)
                    .masked(masked)
                    .default_value(text)
            })
        };
        let templated = [
            field(username, t!("editor.user").to_string(), false, window, cx),
            field(password, t!("editor.password").to_string(), true, window, cx),
            field(token, t!("editor.auth_token").to_string(), true, window, cx),
            field(value, t!("editor.auth_key_value").to_string(), true, window, cx),
        ];
        let key_name = field(name, t!("editor.auth_key_name").to_string(), false, window, cx);

        let mut subscriptions = vec![
            // What it inherits changes with the folders above it.
            cx.observe(&workspace, |_, _, cx| cx.notify()),
            cx.subscribe(&kind, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(_)) = event {
                    this.menu = None;
                    this.write(cx);
                }
            }),
            cx.subscribe(&place, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(_)) = event {
                    this.write(cx);
                }
            }),
            cx.subscribe(&key_name, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.write(cx);
                }
            }),
        ];
        for input in &templated {
            subscriptions.push(cx.subscribe(input, |this, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.write(cx);
                    this.refresh_hints(&input, cx);
                }
                InputEvent::Blur => this.close_hints(&input, cx),
                _ => {}
            }));
        }
        Self {
            owner,
            workspace,
            kinds,
            kind,
            place,
            templated,
            key_name,
            hints,
            menu: None,
            hints_later: None,
            _subscriptions: subscriptions,
        }
    }

    fn kind_now(&self, cx: &App) -> AuthKind {
        let index = self.kind.read(cx).selected_value().copied().unwrap_or(0);
        self.kinds.get(index).copied().unwrap_or(self.kinds[0])
    }

    /// What the fields say, as it is kept.
    pub fn auth(&self, cx: &App) -> Auth {
        let auth = self.chosen(cx);
        match (&self.owner, auth) {
            (AuthFor::Project, Auth::None) => Auth::Inherit,
            (_, auth) => auth,
        }
    }

    fn chosen(&self, cx: &App) -> Auth {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        match self.kind_now(cx) {
            AuthKind::Inherit => Auth::Inherit,
            AuthKind::None => Auth::None,
            AuthKind::Basic => Auth::Basic {
                username: text(&self.templated[USERNAME]),
                password: text(&self.templated[PASSWORD]),
            },
            AuthKind::Bearer => Auth::Bearer {
                token: text(&self.templated[TOKEN]),
            },
            AuthKind::ApiKey => Auth::ApiKey {
                name: text(&self.key_name),
                value: text(&self.templated[KEY_VALUE]),
                place: PLACES[self.place.read(cx).selected_value().copied().unwrap_or(0).min(1)],
            },
        }
    }

    fn write(&mut self, cx: &mut Context<Self>) {
        if let AuthFor::Api(api) = &self.owner {
            let auth = self.auth(cx);
            api.update(cx, |api, cx| api.set_auth(auth, cx));
        }
        cx.notify();
    }

    /// The API it edits, if it edits one's.
    fn api_id(&self, cx: &App) -> Option<ItemId> {
        match &self.owner {
            AuthFor::Api(api) => Some(api.read(cx).api().clone()),
            AuthFor::Folder(_) | AuthFor::Project => None,
        }
    }

    /// What inheriting stands for here, and where it comes from; none for
    /// the project.
    fn inherited_note(&self, cx: &App) -> Option<String> {
        let item = match &self.owner {
            AuthFor::Api(api) => api.read(cx).api().clone(),
            AuthFor::Folder(id) => id.clone(),
            AuthFor::Project => return None,
        };
        let inherited = self.workspace.read(cx).inherited(&item);
        let kind = kind_label(inherited.auth.kind());
        Some(
            match (&inherited.from, inherited.auth.signs()) {
                (AuthSource::Folder { name, .. }, true) => t!("editor.auth_from_folder", name = name, kind = kind),
                (AuthSource::Project, true) => t!("editor.auth_from_project", kind = kind),
                (AuthSource::Folder { name, .. }, false) => t!("editor.auth_folder_sends_none", name = name),
                (AuthSource::Project, false) => t!("editor.auth_nothing_above"),
            }
            .to_string(),
        )
    }

    /// The hints for `input`'s caret, under it. Ones that wait for an answer
    /// show once it is read, if the field still reads the same.
    fn refresh_hints(&mut self, input: &Entity<InputState>, cx: &mut Context<Self>) {
        let Some(field) = self.templated.iter().position(|each| each == input) else {
            return;
        };
        let (text, caret) = {
            let state = input.read(cx);
            (state.value().to_string(), state.cursor())
        };
        self.hints_later = None;
        match self.hints.hints(&text, caret, cx) {
            Hints::Now(hints) => self.menu = FieldHints::new(field, hints),
            Hints::Later(hints) => {
                self.menu = None;
                let input = input.downgrade();
                self.hints_later = Some(cx.spawn(async move |this, cx| {
                    let hints = hints.await;
                    let _ = this.update(cx, |this, cx| {
                        let Some(input) = input.upgrade() else { return };
                        let state = input.read(cx);
                        if state.value().as_ref() != text || state.cursor() != caret {
                            return;
                        }
                        this.menu = FieldHints::new(field, hints);
                        cx.notify();
                    });
                }));
            }
        }
        cx.notify();
    }

    fn close_hints(&mut self, input: &Entity<InputState>, cx: &mut Context<Self>) {
        let open = self.menu.as_ref().map(|menu| menu.field);
        if open.is_some_and(|field| self.templated.get(field) == Some(input)) {
            self.menu = None;
            cx.notify();
        }
    }

    /// The keys the open hints take before their field does.
    fn hint_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.as_mut() else { return };
        let field = menu.field;
        match menu.key(event) {
            HintKey::Moved => cx.stop_propagation(),
            HintKey::Choose(hint) => {
                cx.stop_propagation();
                self.choose_hint(field, hint, window, cx);
            }
            HintKey::Close => {
                cx.stop_propagation();
                self.menu = None;
            }
            HintKey::Leave => self.menu = None,
            HintKey::Pass => return,
        }
        cx.notify();
    }

    fn choose_hint(&mut self, field: usize, hint: Hint, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        if let Some(input) = self.templated.get(field).cloned() {
            template_hints::apply(&input, &hint, window, cx);
        }
        cx.notify();
    }

    /// A template field, the hints under it when they are open for it.
    fn templated_field(&self, field: usize, cx: &mut Context<Self>) -> AnyElement {
        let input = &self.templated[field];
        let masked = field != USERNAME;
        let view = cx.entity().downgrade();
        div()
            .w_full()
            .relative()
            .child(Input::new(input).small().when(masked, |input| input.mask_toggle()))
            .when_some(self.menu.as_ref().filter(|menu| menu.field == field), |cell, menu| {
                cell.capture_key_down(
                    cx.listener(|this, event: &KeyDownEvent, window, cx| this.hint_key(event, window, cx)),
                )
                .child(menu.render(
                    move |hint, window, cx| {
                        let _ = view.update(cx, |this, cx| this.choose_hint(field, hint, window, cx));
                    },
                    cx,
                ))
            })
            .into_any_element()
    }
}

/// A name, and its control taking the rest of the line.
fn row(label: String, control: impl IntoElement) -> AnyElement {
    h_flex()
        .w_full()
        .gap_3()
        .items_center()
        .child(div().w(rems(7.)).flex_none().text_sm().child(label))
        .child(div().flex_1().min_w_0().child(control))
        .into_any_element()
}

impl Render for AuthEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let request = match &self.owner {
            AuthFor::Api(api) => Some(api.read(cx).request().clone()),
            AuthFor::Folder(_) | AuthFor::Project => None,
        };
        let grpc = request
            .as_ref()
            .is_some_and(|request| request.method == HttpMethod::Grpc);
        // #3: an entry of the request's own of the same name is what goes.
        let shadowed = request
            .as_ref()
            .and_then(|request| {
                // What it signs in with, its own or inherited.
                let inherited = self.workspace.read(cx).inherited(&self.api_id(cx)?);
                inherited
                    .auth(&request.auth)
                    .shadowed(&request.headers, &request.query, !grpc)
            })
            .map(|(header, name)| {
                let list = if header {
                    t!("editor.headers")
                } else {
                    t!("editor.query")
                };
                t!("editor.auth_shadowed", list = list, name = name).to_string()
            });
        let kind = self.kind_now(cx);
        let about: SharedString = match kind {
            AuthKind::Inherit => self.inherited_note(cx).unwrap_or_default().into(),
            AuthKind::None if matches!(self.owner, AuthFor::Project) => t!("editor.auth_none_project_hint"),
            AuthKind::None => t!("editor.auth_none_hint"),
            AuthKind::Basic => t!("editor.auth_basic_hint"),
            AuthKind::Bearer => t!("editor.auth_bearer_hint"),
            AuthKind::ApiKey if grpc => t!("editor.auth_api_key_grpc_hint"),
            AuthKind::ApiKey => t!("editor.auth_api_key_hint"),
        }
        .to_string()
        .into();
        // A list is as wide as its longest choice wants, not the line.
        let narrow = |select: Select<Vec<Choice>>| div().w(rems(12.)).child(select.small());
        let mut rows = vec![row(t!("editor.auth_kind").to_string(), narrow(Select::new(&self.kind)))];
        match kind {
            AuthKind::Basic => {
                rows.push(row(t!("editor.user").to_string(), self.templated_field(USERNAME, cx)));
                rows.push(row(
                    t!("editor.password").to_string(),
                    self.templated_field(PASSWORD, cx),
                ));
            }
            AuthKind::Bearer => rows.push(row(
                t!("editor.auth_token").to_string(),
                self.templated_field(TOKEN, cx),
            )),
            AuthKind::ApiKey => {
                rows.push(row(
                    t!("editor.auth_key_name").to_string(),
                    Input::new(&self.key_name).small(),
                ));
                rows.push(row(
                    t!("editor.auth_key_value").to_string(),
                    self.templated_field(KEY_VALUE, cx),
                ));
                // A gRPC call has no query: its key is metadata.
                if !grpc {
                    rows.push(row(
                        t!("editor.auth_place").to_string(),
                        narrow(Select::new(&self.place)),
                    ));
                }
            }
            AuthKind::Inherit | AuthKind::None => {}
        }
        let (muted, warning) = (cx.theme().muted_foreground, cx.theme().warning);
        let signs = !matches!(kind, AuthKind::Inherit | AuthKind::None);
        v_flex()
            .gap_3()
            .child(rows.remove(0))
            .child(div().text_xs().text_color(muted).child(about))
            .children(rows)
            .children(shadowed.map(|text| div().text_xs().text_color(warning).child(text)))
            .when(signs, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("editor.auth_hint").to_string()),
                )
            })
    }
}
