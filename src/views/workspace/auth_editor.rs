//! How an API, a folder or the project signs in (#66): inherited, none,
//! Basic, a bearer token, an API key in a header or the query (a gRPC
//! call's as metadata), Digest (C2), or OAuth 2.0 (C4). Values are templates, offering the hints as one
//! types `{{` (M6); the secrets show as dots till shown. An API's changes go
//! to it at once, and its view is made again when its request is replaced
//! (the history, a cURL command); a folder's and the project's wait for
//! their dialog's Save. Inheriting says what it stands for, and where from.
//!
//! The authorization-code grant (C4, data model 14) is the desktop's: it
//! opens the system browser and listens on `127.0.0.1` for the callback.
//! A page keeps the same fields and says it cannot open the browser.

use crate::states::{ApiState, WorkspaceState};
use crate::views::settings::Choice;
use crate::views::template_hints::{self, FieldHints, HintKey, HintSource, Hints};
use cyberapi_core::ItemId;
use cyberapi_core::auth::{Auth, AuthKind, KeyPlace};
use cyberapi_core::inherit::AuthSource;
use cyberapi_core::model::{HttpMethod, HttpRequest};
use cyberapi_core::oauth::{ClientAuth, Grant, OAuth2};
use cyberapi_core::template::Answers;
use cyberapi_core::template_hints::Hint;
use futures::future::FutureExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, AsyncApp, Context, Entity, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, WeakEntity, Window, div,
    rems,
};
use rust_i18n::t;
use std::time::Duration;

type Choices = Entity<SelectState<Vec<Choice>>>;

const PLACES: [KeyPlace; 2] = [KeyPlace::Header, KeyPlace::Query];
const GRANTS: [Grant; 3] = [Grant::ClientCredentials, Grant::Password, Grant::AuthorizationCode];
/// How long the desktop waits for the browser to come back to the loopback.
const AUTHORIZE_WAIT: Duration = Duration::from_secs(180);
const CLIENT_AUTHS: [ClientAuth; 2] = [ClientAuth::Header, ClientAuth::Body];

/// The fields whose values are templates, in the order of their index in
/// the open hints (`FieldHints::field`).
const USERNAME: usize = 0;
const PASSWORD: usize = 1;
const TOKEN: usize = 2;
const KEY_VALUE: usize = 3;
const TOKEN_URL: usize = 4;
const CLIENT_ID: usize = 5;
const CLIENT_SECRET: usize = 6;
const SCOPE: usize = 7;
const AUDIENCE: usize = 8;
const RESOURCE: usize = 9;
const AUTHORIZATION_URL: usize = 10;

/// The fields whose values are secrets, shown as dots till shown.
fn secret(field: usize) -> bool {
    matches!(field, PASSWORD | TOKEN | KEY_VALUE | CLIENT_SECRET)
}

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
const PROJECT_KINDS: [AuthKind; 6] = [
    AuthKind::None,
    AuthKind::Basic,
    AuthKind::Bearer,
    AuthKind::ApiKey,
    AuthKind::Digest,
    AuthKind::OAuth2,
];

pub struct AuthEditor {
    owner: AuthFor,
    workspace: Entity<WorkspaceState>,
    /// The kinds its list offers, in order.
    kinds: &'static [AuthKind],
    kind: Choices,
    place: Choices,
    /// C4: OAuth's grant, and where its client signs in.
    grant: Choices,
    client_auth: Choices,
    /// Basic's user and password, the token, the key's value, OAuth's
    /// token URL, client, scope and authorization URL: templates.
    templated: [Entity<InputState>; 11],
    /// The loopback port. Empty is any free port. Not a template.
    redirect_port: Entity<InputState>,
    key_name: Entity<InputState>,
    /// The browser dance, while it runs. Dropping it cancels: the listener
    /// stops. The button stays clickable so a second click can drop it.
    authorizing: Option<Task<()>>,
    /// What the last attempt came to, under the button.
    note: Option<AuthNote>,
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
        AuthKind::Digest => t!("editor.auth_digest"),
        AuthKind::OAuth2 => t!("editor.auth_oauth2"),
    }
    .to_string()
}

fn grant_label(grant: Grant) -> String {
    match grant {
        Grant::ClientCredentials => t!("editor.auth_grant_client"),
        Grant::Password => t!("editor.auth_grant_password"),
        Grant::AuthorizationCode => t!("editor.auth_grant_code"),
    }
    .to_string()
}

/// What the last authorization attempt came to.
enum AuthNote {
    Waiting,
    Done,
    Cancelled,
    TimedOut,
    Failed(String),
}

impl AuthNote {
    fn text(&self) -> String {
        match self {
            AuthNote::Waiting => t!("editor.auth_authorize_waiting"),
            AuthNote::Done => t!("editor.auth_authorized"),
            AuthNote::Cancelled => t!("editor.auth_authorize_cancelled"),
            AuthNote::TimedOut => t!("editor.auth_authorize_timeout"),
            AuthNote::Failed(text) => return text.clone(),
        }
        .to_string()
    }
}

fn client_auth_label(place: ClientAuth) -> String {
    match place {
        ClientAuth::Header => t!("editor.auth_client_in_header"),
        ClientAuth::Body => t!("editor.auth_client_in_body"),
    }
    .to_string()
}

/// The attempt ended: the note under the button, and a toast when the
/// reader may be looking at the browser. Clearing the task is last, so
/// dropping it does not cancel the rest of this poll.
fn settle(
    this: &WeakEntity<AuthEditor>,
    cx: &mut AsyncApp,
    window: AnyWindowHandle,
    note: AuthNote,
    toast: Option<String>,
    ok: bool,
) {
    if let Some(text) = toast {
        let _ = cx.update_window(window, |_, window, cx| {
            let toast = match ok {
                true => Notification::success(text),
                false => Notification::error(text),
            };
            window.push_notification(toast, cx);
        });
    }
    let _ = this.update(cx, |this, cx| {
        this.authorizing = None;
        this.note = Some(note);
        cx.notify();
    });
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
        let mut oauth = OAuth2::default();
        match &auth {
            Auth::Basic {
                username: user,
                password: secret,
            }
            | Auth::Digest {
                username: user,
                password: secret,
            } => (username, password) = (user.clone(), secret.clone()),
            Auth::Bearer { token: own } => token = own.clone(),
            Auth::ApiKey {
                name: own,
                value: key,
                place,
            } => (name, value, place_now) = (own.clone(), key.clone(), *place),
            Auth::OAuth2(own) => {
                oauth = own.as_ref().clone();
                (username, password) = (own.username.clone(), own.password.clone());
            }
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
        let listed = |labels: Vec<String>, at: usize, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| SelectState::new(choices(labels), Some(IndexPath::new(at)), window, cx))
        };
        let grant = listed(
            GRANTS.map(grant_label).to_vec(),
            GRANTS.iter().position(|each| *each == oauth.grant).unwrap_or(0),
            window,
            cx,
        );
        let client_auth = listed(
            CLIENT_AUTHS.map(client_auth_label).to_vec(),
            CLIENT_AUTHS
                .iter()
                .position(|each| *each == oauth.client_auth)
                .unwrap_or(0),
            window,
            cx,
        );
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
            field(
                oauth.token_url,
                t!("editor.auth_token_url").to_string(),
                false,
                window,
                cx,
            ),
            field(
                oauth.client_id,
                t!("editor.auth_client_id").to_string(),
                false,
                window,
                cx,
            ),
            field(
                oauth.client_secret,
                t!("editor.auth_client_secret").to_string(),
                true,
                window,
                cx,
            ),
            field(oauth.scope, t!("editor.auth_scope").to_string(), false, window, cx),
            field(
                oauth.audience,
                t!("editor.auth_audience_placeholder").to_string(),
                false,
                window,
                cx,
            ),
            field(
                oauth.resource,
                t!("editor.auth_resource_placeholder").to_string(),
                false,
                window,
                cx,
            ),
            field(
                oauth.authorization_url,
                t!("editor.auth_authorization_url").to_string(),
                false,
                window,
                cx,
            ),
        ];
        let redirect_port = field(
            match oauth.redirect_port {
                0 => String::new(),
                port => port.to_string(),
            },
            t!("editor.auth_redirect_port_placeholder").to_string(),
            false,
            window,
            cx,
        );
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
            cx.subscribe(&grant, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(_)) = event {
                    this.write(cx);
                }
            }),
            cx.subscribe(&client_auth, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(_)) = event {
                    this.write(cx);
                }
            }),
            cx.subscribe(&key_name, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.write(cx);
                }
            }),
            cx.subscribe(&redirect_port, |this, _, event: &InputEvent, cx| {
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
            grant,
            client_auth,
            templated,
            redirect_port,
            key_name,
            hints,
            menu: None,
            hints_later: None,
            authorizing: None,
            note: None,
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
            AuthKind::Digest => Auth::Digest {
                username: text(&self.templated[USERNAME]),
                password: text(&self.templated[PASSWORD]),
            },
            AuthKind::ApiKey => Auth::ApiKey {
                name: text(&self.key_name),
                value: text(&self.templated[KEY_VALUE]),
                place: PLACES[self.place.read(cx).selected_value().copied().unwrap_or(0).min(1)],
            },
            AuthKind::OAuth2 => {
                let grant = self.grant_now(cx);
                let password = grant == Grant::Password;
                let code = grant == Grant::AuthorizationCode;
                // The user's only for the grant that sends them, and the
                // browser's URL and port only for the one that opens one.
                let user = |field: usize| match password {
                    true => text(&self.templated[field]),
                    false => String::new(),
                };
                Auth::from(OAuth2 {
                    grant,
                    token_url: text(&self.templated[TOKEN_URL]),
                    authorization_url: match code {
                        true => text(&self.templated[AUTHORIZATION_URL]),
                        false => String::new(),
                    },
                    redirect_port: match code {
                        true => self.port_now(cx),
                        false => 0,
                    },
                    client_id: text(&self.templated[CLIENT_ID]),
                    client_secret: text(&self.templated[CLIENT_SECRET]),
                    username: user(USERNAME),
                    password: user(PASSWORD),
                    scope: text(&self.templated[SCOPE]),
                    audience: text(&self.templated[AUDIENCE]),
                    resource: text(&self.templated[RESOURCE]),
                    client_auth: CLIENT_AUTHS[self.client_auth.read(cx).selected_value().copied().unwrap_or(0).min(1)],
                })
            }
        }
    }

    fn grant_now(&self, cx: &App) -> Grant {
        let index = self.grant.read(cx).selected_value().copied().unwrap_or(0);
        GRANTS[index.min(GRANTS.len() - 1)]
    }

    /// Empty is any free port. A value that is not a port is none.
    fn port_now(&self, cx: &App) -> u16 {
        let text = self.redirect_port.read(cx).value().trim().to_string();
        if text.is_empty() { 0 } else { text.parse().unwrap_or(0) }
    }

    fn write(&mut self, cx: &mut Context<Self>) {
        self.note = None;
        if let AuthFor::Api(api) = &self.owner {
            let auth = self.auth(cx);
            api.update(cx, |api, cx| api.set_auth(auth, cx));
        }
        cx.notify();
    }

    /// Open the system browser for an authorization-code sign-in, and
    /// exchange the code the loopback receives. A second click drops the
    /// task, which stops the listener.
    fn authorize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.authorizing.is_some() {
            self.authorizing = None;
            self.note = Some(AuthNote::Cancelled);
            cx.notify();
            return;
        }
        if !crate::this_build::opens_oauth_browser() {
            return;
        }
        let request = HttpRequest {
            uri: "https://127.0.0.1/".into(),
            auth: self.auth(cx),
            ..Default::default()
        };
        let workspace = self.workspace.read(cx);
        let mut sender = workspace.run_sender(cx);
        // The synthetic request carries the sign-in itself. A folder's
        // headers are not part of asking for this token.
        sender.inherited = Default::default();
        let collection = workspace.collection().id.clone();
        let answers = match &self.owner {
            AuthFor::Api(api) => api.read(cx).typed(cx),
            AuthFor::Folder(_) | AuthFor::Project => Answers::default(),
        };
        let window = window.window_handle();
        let exchanging = sender.clone();
        self.note = Some(AuthNote::Waiting);
        self.authorizing = Some(cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_spawn(async move {
                    cyberapi_service::prepare_code(&sender, &collection, &request, &answers).await
                })
                .await;
            let grant = match prepared {
                Ok(grant) => grant,
                Err(error) => {
                    settle(&this, cx, window, AuthNote::Failed(error.clone()), Some(error), false);
                    return;
                }
            };
            let listening = match this.update(cx, |_, cx| {
                crate::this_build::listen_oauth(grant.port(), grant.state.clone(), cx)
            }) {
                Ok(task) => task,
                Err(_) => return,
            };
            let listened = match listening.await {
                Ok(listened) => listened,
                Err(error) => {
                    settle(&this, cx, window, AuthNote::Failed(error.clone()), Some(error), false);
                    return;
                }
            };
            let bound = listened.port();
            let url = match grant.authorize_url(bound) {
                Ok(url) => url,
                Err(error) => {
                    drop(listened);
                    settle(&this, cx, window, AuthNote::Failed(error.clone()), Some(error), false);
                    return;
                }
            };
            cx.update(|cx| cx.open_url(&url));
            let code = match futures::future::select(
                listened.code().boxed_local(),
                cx.background_executor().timer(AUTHORIZE_WAIT).boxed_local(),
            )
            .await
            {
                futures::future::Either::Left((code, _)) => code,
                futures::future::Either::Right((_, waiting)) => {
                    drop(waiting);
                    let text = t!("editor.auth_authorize_timeout").to_string();
                    settle(&this, cx, window, AuthNote::TimedOut, Some(text), false);
                    return;
                }
            };
            let code = match code {
                Ok(code) => code,
                Err(error) => {
                    settle(&this, cx, window, AuthNote::Failed(error.clone()), Some(error), false);
                    return;
                }
            };
            let exchanged = cx
                .background_spawn(
                    async move { cyberapi_service::exchange_code(&exchanging, &grant, bound, &code).await },
                )
                .await;
            match exchanged {
                Ok(()) => {
                    let text = t!("editor.auth_authorized").to_string();
                    settle(&this, cx, window, AuthNote::Done, Some(text), true);
                }
                Err(error) => {
                    settle(&this, cx, window, AuthNote::Failed(error.clone()), Some(error), false);
                }
            }
        }));
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
        let masked = secret(field);
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
            AuthKind::Digest => t!("editor.auth_digest_hint"),
            AuthKind::Bearer => t!("editor.auth_bearer_hint"),
            AuthKind::ApiKey if grpc => t!("editor.auth_api_key_grpc_hint"),
            AuthKind::ApiKey => t!("editor.auth_api_key_hint"),
            AuthKind::OAuth2 => t!("editor.auth_oauth2_hint"),
        }
        .to_string()
        .into();
        // A list is as wide as its longest choice wants, not the line.
        let narrow = |select: Select<Vec<Choice>>| div().w(rems(12.)).child(select.small());
        let mut rows = vec![row(t!("editor.auth_kind").to_string(), narrow(Select::new(&self.kind)))];
        match kind {
            AuthKind::Basic | AuthKind::Digest => {
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
            AuthKind::OAuth2 => {
                let grant_list = div().w(rems(14.)).child(Select::new(&self.grant).small());
                rows.push(row(t!("editor.auth_grant").to_string(), grant_list));
                let grant = self.grant_now(cx);
                if grant == Grant::AuthorizationCode {
                    rows.push(row(
                        t!("editor.auth_authorization_url").to_string(),
                        self.templated_field(AUTHORIZATION_URL, cx),
                    ));
                    rows.push(row(
                        t!("editor.auth_redirect_port").to_string(),
                        NumberInput::new(&self.redirect_port)
                            .small()
                            .placeholder(t!("editor.auth_redirect_port_placeholder").to_string()),
                    ));
                    let busy = self.authorizing.is_some();
                    let mut button = Button::new("oauth-authorize")
                        .small()
                        .label(match busy {
                            true => t!("editor.auth_authorize_cancel").to_string(),
                            false => t!("editor.auth_authorize").to_string(),
                        })
                        .disabled(!crate::this_build::opens_oauth_browser())
                        .on_click(cx.listener(|this, _, window, cx| this.authorize(window, cx)));
                    button = match busy {
                        true => button.danger(),
                        false => button.primary(),
                    };
                    rows.push(row(String::new(), button));
                    if let Some(note) = &self.note {
                        let color = match note {
                            AuthNote::Done => cx.theme().green,
                            AuthNote::Failed(_) | AuthNote::TimedOut => cx.theme().danger,
                            AuthNote::Waiting | AuthNote::Cancelled => cx.theme().muted_foreground,
                        };
                        rows.push(div().text_xs().text_color(color).child(note.text()).into_any_element());
                    }
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("editor.auth_code_hint").to_string())
                            .into_any_element(),
                    );
                    if !crate::this_build::opens_oauth_browser() {
                        rows.push(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("editor.auth_code_web").to_string())
                                .into_any_element(),
                        );
                    }
                }
                rows.push(row(
                    t!("editor.auth_token_url").to_string(),
                    self.templated_field(TOKEN_URL, cx),
                ));
                rows.push(row(
                    t!("editor.auth_client_id").to_string(),
                    self.templated_field(CLIENT_ID, cx),
                ));
                rows.push(row(
                    t!("editor.auth_client_secret").to_string(),
                    self.templated_field(CLIENT_SECRET, cx),
                ));
                if grant == Grant::Password {
                    rows.push(row(t!("editor.user").to_string(), self.templated_field(USERNAME, cx)));
                    rows.push(row(
                        t!("editor.password").to_string(),
                        self.templated_field(PASSWORD, cx),
                    ));
                }
                rows.push(row(
                    t!("editor.auth_scope").to_string(),
                    self.templated_field(SCOPE, cx),
                ));
                rows.push(row(
                    t!("editor.auth_audience").to_string(),
                    self.templated_field(AUDIENCE, cx),
                ));
                rows.push(row(
                    t!("editor.auth_resource").to_string(),
                    self.templated_field(RESOURCE, cx),
                ));
                rows.push(row(
                    t!("editor.auth_client_auth").to_string(),
                    narrow(Select::new(&self.client_auth)),
                ));
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
