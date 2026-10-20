//! The reader's cookie jar, as its dialog edits it (M9).
//!
//! A cookie's value is saved 300 ms after the last key, its expiry at once
//! (F9.6); adding (F9.5), removing and clearing (F9.7) are immediate. The
//! jar is read when the dialog opens: what requests add meanwhile shows the
//! next time.

use super::{Load, run_store};
use cyberapi_core::Timestamp;
use cyberapi_core::cookie::Cookie;
use cyberapi_store::{Storage, StoreError};
use gpui_kit::{AppContext as _, Context, EventEmitter, Task};
use std::collections::HashMap;
use std::time::Duration;

const SAVE_DELAY: Duration = Duration::from_millis(300);

/// Domain, path and name.
pub type CookieKey = (String, String, String);

fn key_of(cookie: &Cookie) -> CookieKey {
    (cookie.domain.clone(), cookie.path.clone(), cookie.name.clone())
}

#[derive(Clone, Debug)]
pub enum CookiesEvent {
    Failed(StoreError),
}

pub struct CookiesState {
    storage: Storage,
    cookies: Vec<Cookie>,
    load: Load,
    saving: HashMap<CookieKey, Task<()>>,
}

impl EventEmitter<CookiesEvent> for CookiesState {}

impl CookiesState {
    pub fn new(storage: Storage, cx: &mut Context<Self>) -> Self {
        // A value still waiting when the dialog goes is saved then.
        cx.on_release(|this, cx| {
            let waiting: Vec<Cookie> = this
                .cookies
                .iter()
                .filter(|cookie| this.saving.contains_key(&key_of(cookie)))
                .cloned()
                .collect();
            if !waiting.is_empty() {
                let call = this.storage.cookies().put(waiting);
                cx.background_spawn(async move {
                    if let Err(error) = call.await {
                        tracing::warn!(%error, "a cookie could not be saved");
                    }
                })
                .detach();
            }
        })
        .detach();
        let mut state = Self {
            storage,
            cookies: Vec::new(),
            load: Load::Loading,
            saving: HashMap::new(),
        };
        state.reload(cx);
        state
    }

    pub fn cookies(&self) -> &[Cookie] {
        &self.cookies
    }

    pub fn load(&self) -> &Load {
        &self.load
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        run_store(cx, self.storage.cookies().list(), |this, result, cx| {
            match result {
                Ok(cookies) => {
                    this.cookies = cookies;
                    this.load = Load::Loaded;
                }
                Err(error) => {
                    this.load = Load::Failed(error.clone());
                    cx.emit(CookiesEvent::Failed(error));
                }
            }
            cx.notify();
        });
    }

    /// F9.5: replaces a cookie with the same domain, path and name.
    pub fn add(&mut self, cookie: Cookie, cx: &mut Context<Self>) {
        run_store(
            cx,
            self.storage.cookies().put(vec![cookie]),
            |this, result, cx| match result {
                Ok(()) => this.reload(cx),
                Err(error) => cx.emit(CookiesEvent::Failed(error)),
            },
        );
    }

    pub fn set_value(&mut self, key: &CookieKey, value: String, cx: &mut Context<Self>) {
        let Some(cookie) = self.cookies.iter_mut().find(|cookie| key_of(cookie) == *key) else {
            return;
        };
        if cookie.value == value {
            return;
        }
        cookie.value = value;
        let key = key.clone();
        let task = cx.spawn({
            let key = key.clone();
            async move |this, cx| {
                cx.background_executor().timer(SAVE_DELAY).await;
                let _ = this.update(cx, |this, cx| this.save(&key, cx));
            }
        });
        self.saving.insert(key, task);
    }

    /// F9.6: `None` keeps it for the session.
    pub fn set_expires(&mut self, key: &CookieKey, expires: Option<Timestamp>, cx: &mut Context<Self>) {
        let Some(cookie) = self.cookies.iter_mut().find(|cookie| key_of(cookie) == *key) else {
            return;
        };
        if cookie.expires != expires {
            cookie.expires = expires;
            self.save(key, cx);
            cx.notify();
        }
    }

    fn save(&mut self, key: &CookieKey, cx: &mut Context<Self>) {
        self.saving.remove(key);
        let Some(cookie) = self.cookies.iter().find(|cookie| key_of(cookie) == *key).cloned() else {
            return;
        };
        let expired = cookie
            .expires
            .is_some_and(|at| at.get() <= chrono::Utc::now().fixed_offset());
        run_store(
            cx,
            self.storage.cookies().put(vec![cookie]),
            move |this, result, cx| match result {
                // An expiry in the past removed it.
                Ok(()) if expired => this.reload(cx),
                Ok(()) => {}
                Err(error) => cx.emit(CookiesEvent::Failed(error)),
            },
        );
    }

    /// F9.7.
    pub fn delete(&mut self, key: &CookieKey, cx: &mut Context<Self>) {
        self.saving.remove(key);
        self.cookies.retain(|cookie| key_of(cookie) != *key);
        let (domain, path, name) = key.clone();
        run_store(
            cx,
            self.storage.cookies().delete(domain, path, name),
            |_, result, cx| {
                if let Err(error) = result {
                    cx.emit(CookiesEvent::Failed(error));
                }
            },
        );
        cx.notify();
    }

    /// F9.7: the view asks first.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.saving.clear();
        self.cookies.clear();
        run_store(cx, self.storage.cookies().clear(), |_, result, cx| {
            if let Err(error) = result {
                cx.emit(CookiesEvent::Failed(error));
            }
        });
        cx.notify();
    }
}

pub fn cookie_key(cookie: &Cookie) -> CookieKey {
    key_of(cookie)
}
