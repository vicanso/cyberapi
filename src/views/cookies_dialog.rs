//! The reader's cookie jar, in a dialog (M9).

use crate::i18n;
use crate::states::{AppStore, CookieKey, CookiesEvent, CookiesState, Load, cookie_key};
use crate::views::inline_confirm::inline_confirm;
use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone as _};
use cyberapi_core::Timestamp;
use cyberapi_core::cookie::Cookie;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, ElementId, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;
use std::collections::HashMap;

/// "Never expires" (F9.6), as the old app wrote it.
const NEVER: &str = "3000-01-01 00:00:00";
const FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// An expiry in local time; empty for a session cookie.
fn show_expires(expires: Option<Timestamp>) -> String {
    expires
        .map(|at| at.get().with_timezone(&Local).format(FORMAT).to_string())
        .unwrap_or_default()
}

/// What the reader typed: a local time, a date, or nothing for a session
/// cookie. `Err` when it is none of those.
fn read_expires(text: &str) -> Result<Option<Timestamp>, ()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let local = NaiveDateTime::parse_from_str(text, FORMAT)
        .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M"))
        .or_else(|_| NaiveDate::parse_from_str(text, "%Y-%m-%d").map(|date| date.and_time(Default::default())))
        .map_err(|_| ())?;
    let at = Local.from_local_datetime(&local).earliest().ok_or(())?;
    Ok(Some(Timestamp::new(at.fixed_offset())))
}

pub fn open_cookies_dialog(window: &mut Window, cx: &mut App) {
    let storage = AppStore::global(cx).storage().clone();
    let state = cx.new(|cx| CookiesState::new(storage, cx));
    let editor = cx.new(|cx| CookiesEditor::new(state, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(t!("cookies.title").to_string())
            .w(px(860.))
            .child(editor.clone())
    });
}

struct Row {
    value: Entity<InputState>,
    expires: Entity<InputState>,
    _subscriptions: [Subscription; 2],
}

/// The fields of a cookie to add (F9.5).
struct NewCookie {
    name: Entity<InputState>,
    value: Entity<InputState>,
    domain: Entity<InputState>,
    path: Entity<InputState>,
    expires: Entity<InputState>,
}

struct CookiesEditor {
    state: Entity<CookiesState>,
    /// Narrows the jar to the cookies whose name, domain or value has it.
    search: Entity<InputState>,
    /// Asking whether to clear the jar.
    confirming_clear: bool,
    rows: HashMap<CookieKey, Row>,
    new: NewCookie,
    _subscriptions: Vec<Subscription>,
}

impl CookiesEditor {
    fn new(state: Entity<CookiesState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: String, value: &str, window: &mut Window, cx: &mut Context<Self>| {
            let value = value.to_string();
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            })
        };
        let new = NewCookie {
            name: input(t!("cookies.name").to_string(), "", window, cx),
            value: input(t!("cookies.value").to_string(), "", window, cx),
            domain: input(t!("cookies.domain").to_string(), "", window, cx),
            path: input(t!("cookies.path").to_string(), "/", window, cx),
            expires: input(t!("cookies.expires_placeholder").to_string(), "", window, cx),
        };
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("cookies.search").to_string())
                .clean_on_escape()
        });
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| this.sync_rows(window, cx)),
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&state, window, |_, _, event, window, cx| match event {
                CookiesEvent::Failed(error) => {
                    window.push_notification(Notification::error(i18n::store_error(error)), cx);
                }
            }),
        ];
        Self {
            state,
            search,
            confirming_clear: false,
            rows: HashMap::new(),
            new,
            _subscriptions: subscriptions,
        }
    }

    fn sync_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cookies = self.state.read(cx).cookies().to_vec();
        self.rows
            .retain(|key, _| cookies.iter().any(|cookie| cookie_key(cookie) == *key));
        for cookie in cookies {
            let key = cookie_key(&cookie);
            if self.rows.contains_key(&key) {
                continue;
            }
            let value = cx.new(|cx| InputState::new(window, cx).default_value(cookie.value.clone()));
            let expires = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(t!("cookies.session").to_string())
                    .default_value(show_expires(cookie.expires))
            });
            let (for_value, for_expires) = (key.clone(), key.clone());
            let subscriptions = [
                cx.subscribe(&value, move |this, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let text = input.read(cx).value().to_string();
                        this.state.update(cx, |state, cx| state.set_value(&for_value, text, cx));
                    }
                }),
                cx.subscribe_in(&expires, window, move |this, input, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                        let text = input.read(cx).value().to_string();
                        match read_expires(&text) {
                            Ok(at) => this
                                .state
                                .update(cx, |state, cx| state.set_expires(&for_expires, at, cx)),
                            Err(()) => window
                                .push_notification(Notification::warning(t!("cookies.bad_expires").to_string()), cx),
                        }
                    }
                }),
            ];
            self.rows.insert(
                key,
                Row {
                    value,
                    expires,
                    _subscriptions: subscriptions,
                },
            );
        }
        cx.notify();
    }

    /// F9.6: a cookie kept until the year 3000.
    fn never_expires(&mut self, key: &CookieKey, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(key) else {
            return;
        };
        row.expires.update(cx, |input, cx| input.set_value(NEVER, window, cx));
        let at = read_expires(NEVER).ok().flatten();
        self.state.update(cx, |state, cx| state.set_expires(key, at, cx));
    }

    /// F9.5: a name and a domain at least; the path defaults to `/`.
    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let read = |input: &Entity<InputState>, cx: &App| input.read(cx).value().trim().to_string();
        let (name, value, domain, path, expires) = (
            read(&self.new.name, cx),
            read(&self.new.value, cx),
            read(&self.new.domain, cx),
            read(&self.new.path, cx),
            read(&self.new.expires, cx),
        );
        if name.is_empty() || domain.is_empty() {
            window.push_notification(Notification::warning(t!("cookies.missing").to_string()), cx);
            return;
        }
        let Ok(expires) = read_expires(&expires) else {
            window.push_notification(Notification::warning(t!("cookies.bad_expires").to_string()), cx);
            return;
        };
        let cookie = Cookie {
            name,
            value,
            domain: domain.trim_start_matches('.').to_ascii_lowercase(),
            path: if path.starts_with('/') { path } else { "/".into() },
            expires,
            ..Default::default()
        };
        self.state.update(cx, |state, cx| state.add(cookie, cx));
        for input in [&self.new.name, &self.new.value, &self.new.expires] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    /// F9.7, asked inside the dialog: the browser build draws no dialog
    /// stacked on this one.
    fn render_clear(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.confirming_clear {
            return None;
        }
        let (cancelled, confirmed) = (cx.entity().downgrade(), cx.entity().downgrade());
        Some(
            inline_confirm(
                t!("cookies.clear_title").to_string(),
                t!("cookies.clear_description").to_string(),
                None,
                t!("cookies.clear").to_string(),
                move |_, cx| {
                    let _ = cancelled.update(cx, |this, cx| {
                        this.confirming_clear = false;
                        cx.notify();
                    });
                },
                move |_, cx| {
                    let _ = confirmed.update(cx, |this, cx| {
                        this.confirming_clear = false;
                        this.state.update(cx, |state, cx| state.clear(cx));
                    });
                },
                cx,
            )
            .into_any_element(),
        )
    }
}

impl Render for CookiesEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let header = h_flex()
            .gap_2()
            .text_xs()
            .text_color(muted)
            .child(div().w(px(130.)).child(t!("cookies.name").to_string()))
            .child(div().flex_1().child(t!("cookies.value").to_string()))
            .child(div().w(px(150.)).child(t!("cookies.domain").to_string()))
            .child(div().w(px(70.)).child(t!("cookies.path").to_string()))
            .child(div().w(px(170.)).child(t!("cookies.expires").to_string()))
            .child(actions());
        let keyword = self.search.read(cx).value().trim().to_lowercase();
        let shown = |cookie: &&Cookie| {
            keyword.is_empty()
                || [&cookie.name, &cookie.domain, &cookie.value]
                    .iter()
                    .any(|text| text.to_lowercase().contains(&keyword))
        };
        let found = state.cookies().iter().filter(shown).count();
        let body: gpui_kit::AnyElement = match state.load() {
            Load::Loading => h_flex().child(Spinner::new().small()).into_any_element(),
            _ if state.cookies().is_empty() => div()
                .py_3()
                .text_sm()
                .text_color(muted)
                .child(t!("cookies.empty").to_string())
                .into_any_element(),
            _ if found == 0 => div()
                .py_3()
                .text_sm()
                .text_color(muted)
                .child(t!("cookies.no_match", keyword = keyword).to_string())
                .into_any_element(),
            _ => v_flex()
                .gap_1()
                .children(
                    state
                        .cookies()
                        .iter()
                        .enumerate()
                        .filter(|(_, cookie)| shown(cookie))
                        .filter_map(|(index, cookie)| {
                            let key = cookie_key(cookie);
                            let row = self.rows.get(&key)?;
                            let never_key = key.clone();
                            Some(
                                h_flex()
                                    .id(("cookie", index))
                                    .gap_2()
                                    .items_center()
                                    .text_sm()
                                    .child(div().w(px(130.)).truncate().child(cookie.name.clone()))
                                    .child(div().flex_1().min_w_0().child(Input::new(&row.value).small()))
                                    .child(
                                        div()
                                            .w(px(150.))
                                            .truncate()
                                            .text_color(muted)
                                            .child(cookie.domain.clone()),
                                    )
                                    .child(div().w(px(70.)).truncate().text_color(muted).child(cookie.path.clone()))
                                    .child(div().w(px(170.)).child(Input::new(&row.expires).small()))
                                    .child(
                                        actions()
                                            .child(never_button(("cookie-never", index)).on_click(cx.listener(
                                                move |this, _, window, cx| this.never_expires(&never_key, window, cx),
                                            )))
                                            .child(
                                                Button::new(("cookie-delete", index))
                                                    .ghost()
                                                    .small()
                                                    .icon(IconName::Close)
                                                    .tooltip(t!("common.delete").to_string())
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.state.update(cx, |state, cx| state.delete(&key, cx));
                                                    })),
                                            ),
                                    ),
                            )
                        }),
                )
                .into_any_element(),
        };
        let new = &self.new;
        let add_form = h_flex()
            .gap_2()
            .items_center()
            .pt_2()
            .border_t_1()
            .border_color(theme.border)
            .child(div().w(px(130.)).child(Input::new(&new.name).small()))
            .child(div().flex_1().min_w_0().child(Input::new(&new.value).small()))
            .child(div().w(px(150.)).child(Input::new(&new.domain).small()))
            .child(div().w(px(70.)).child(Input::new(&new.path).small()))
            .child(div().w(px(170.)).child(Input::new(&new.expires).small()))
            .child(
                actions()
                    .child(
                        never_button("cookie-never").on_click(cx.listener(|this, _, window, cx| {
                            this.new
                                .expires
                                .update(cx, |input, cx| input.set_value(NEVER, window, cx));
                        })),
                    )
                    .child(
                        Button::new("cookie-add")
                            .primary()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip(t!("cookies.add").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                    ),
            );
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(muted)
                            .child(t!("cookies.hint").to_string()),
                    )
                    .child(
                        Input::new(&self.search)
                            .small()
                            .w(px(220.))
                            .cleanable(true)
                            .prefix(Icon::new(IconName::Search).text_color(muted)),
                    ),
            )
            .child(header)
            .child(div().id("cookies").max_h(px(360.)).overflow_y_scrollbar().child(body))
            .child(add_form)
            .child(
                h_flex().justify_end().child(
                    Button::new("cookie-clear")
                        .outline()
                        .small()
                        .label(t!("cookies.clear").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirming_clear = true;
                            cx.notify();
                        })),
                ),
            )
            .children(self.render_clear(cx))
    }
}

/// The buttons at the end of a row, as wide in every row as the header
/// leaves for them, so the columns before them line up.
fn actions() -> Div {
    h_flex().flex_none().w(px(64.)).gap_1().justify_end()
}

/// F9.6: sets the expiry beside it to "never".
fn never_button(id: impl Into<ElementId>) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .icon(gpui_kit::assets::IconName::Infinity)
        .tooltip(t!("cookies.never").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiries_read_as_local_times() {
        assert_eq!(read_expires(""), Ok(None));
        let at = read_expires("2030-05-06 07:08:09")
            .expect("a time")
            .expect("not a session");
        assert_eq!(show_expires(Some(at)), "2030-05-06 07:08:09");
        assert!(read_expires("2030-05-06").expect("a date").is_some());
        assert!(read_expires(NEVER).expect("never").is_some());
        assert_eq!(read_expires("soon"), Err(()));
    }
}
