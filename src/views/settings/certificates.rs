//! C5, on the desktop: the settings' client certificates, by host, and the
//! authorities trusted besides the system's. A certificate is added or
//! edited in a dialog — its host, PEM or PKCS#12, its files, a PKCS#12's
//! password — and an authority is a PEM file picked. Only the paths are
//! kept: the next request reads the files.

use super::{Choice, SettingsView, setting_row};
use crate::states::AppStore;
use crate::views::name_dialog::ok_button;
use cyberapi_core::certificates::{CertificateFormat, CertificatePrefs, ClientCertificate};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme as _, IndexPath, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, Focusable as _, IntoElement, ParentElement as _,
    PathPromptOptions, Render, SharedString, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;
use std::path::Path;

/// The settings' certificates changed by `change`, read by the next request.
fn set_certificates(cx: &mut App, change: impl FnOnce(&mut CertificatePrefs)) {
    let app = AppStore::global(cx).state().clone();
    let mut sending = app.read(cx).prefs().sending.clone();
    change(&mut sending.certificates);
    app.update(cx, |app, cx| app.set_sending(sending, cx));
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn format_label(format: CertificateFormat) -> &'static str {
    match format {
        CertificateFormat::Pem => "PEM",
        CertificateFormat::Pkcs12 => "PKCS#12 (.p12, .pfx)",
    }
}

/// An entry under its list's row.
fn entry(label: String, description: String, control: impl IntoElement, cx: &App) -> AnyElement {
    div()
        .pl_4()
        .child(setting_row(label, Some(description), control, cx))
        .into_any_element()
}

fn remove_button(id: (&'static str, usize), remove: impl Fn(&mut App) + 'static) -> Button {
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(IconName::Trash)
        .tooltip(SharedString::from(t!("settings.certificate_remove").to_string()))
        .on_click(move |_, _, cx| remove(cx))
}

/// The rows of the Request section: each list's, then its entries.
pub(super) fn rows(prefs: &CertificatePrefs, cx: &mut Context<SettingsView>) -> Vec<AnyElement> {
    let add = |id: &'static str| {
        Button::new(id)
            .small()
            .icon(IconName::Plus)
            .label(t!("settings.certificate_add").to_string())
    };
    let mut rows = vec![setting_row(
        t!("settings.client_certificates").to_string(),
        Some(t!("settings.client_certificates_desc").to_string()),
        add("certificate-add").on_click(|_, window, cx| open_client_dialog(None, window, cx)),
        cx,
    )];
    for (index, client) in prefs.clients.iter().enumerate() {
        let what = format!("{} · {}", format_label(client.format), file_name(&client.certificate));
        let control = h_flex()
            .gap_1()
            .child(
                Button::new(("certificate-edit", index))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Pencil)
                    .tooltip(SharedString::from(t!("settings.certificate_edit").to_string()))
                    .on_click(move |_, window, cx| open_client_dialog(Some(index), window, cx)),
            )
            .child(remove_button(("certificate-remove", index), move |cx| {
                set_certificates(cx, |prefs| {
                    if index < prefs.clients.len() {
                        prefs.clients.remove(index);
                    }
                })
            }));
        rows.push(entry(client.host.clone(), what, control, cx));
    }
    rows.push(setting_row(
        t!("settings.authorities").to_string(),
        Some(t!("settings.authorities_desc").to_string()),
        add("authority-add").on_click(|_, _, cx| pick_authorities(cx)),
        cx,
    ));
    for (index, path) in prefs.authorities.iter().enumerate() {
        let control = remove_button(("authority-remove", index), move |cx| {
            set_certificates(cx, |prefs| {
                if index < prefs.authorities.len() {
                    prefs.authorities.remove(index);
                }
            })
        });
        rows.push(entry(file_name(path), path.clone(), control, cx));
    }
    rows
}

/// PEM files of authorities, added to the list.
fn pick_authorities(cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: None,
    });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = picked.await else {
            return;
        };
        cx.update(|cx| {
            set_certificates(cx, |prefs| {
                for path in paths {
                    let path = path.display().to_string();
                    if !prefs.authorities.contains(&path) {
                        prefs.authorities.push(path);
                    }
                }
            })
        });
    })
    .detach();
}

/// The dialog for the certificate at `index` of the list, or a new one.
fn open_client_dialog(index: Option<usize>, window: &mut Window, cx: &mut App) {
    let client = index
        .and_then(|index| {
            let app = AppStore::global(cx).state().read(cx);
            app.prefs().sending.certificates.clients.get(index).cloned()
        })
        .unwrap_or_default();
    let form = cx.new(|cx| ClientForm::new(index, client, window, cx));
    let title = match index {
        Some(_) => t!("settings.certificate_edit_title"),
        None => t!("settings.certificate_add_title"),
    }
    .to_string();
    let host = form.read(cx).host.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let submit = {
            let form = form.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| form.update(cx, |form, cx| form.submit(cx))
        };
        dialog
            .title(title.clone())
            .w(px(480.))
            .child(crate::views::dialog_body(div().child(form.clone())))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                    .child(ok_button(
                        "certificate-save",
                        t!("common.save").to_string(),
                        submit.clone(),
                    )),
            )
            .on_ok(submit)
    });
    // After the dialog took focus for itself, so typing starts in the field.
    host.focus_handle(cx).focus(window, cx);
}

/// A client certificate as the dialog edits it.
struct ClientForm {
    /// Its place in the list; a new one goes last.
    index: Option<usize>,
    host: Entity<InputState>,
    format: Entity<SelectState<Vec<Choice>>>,
    certificate: String,
    key: String,
    password: Entity<InputState>,
    /// Saving was asked for without what it needs.
    missing: bool,
    _subscriptions: Vec<Subscription>,
}

impl ClientForm {
    fn new(index: Option<usize>, client: ClientCertificate, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("settings.certificate_host_hint").to_string())
                .default_value(client.host.clone())
        });
        let choices = CertificateFormat::ALL
            .iter()
            .enumerate()
            .map(|(index, format)| Choice {
                label: format_label(*format).into(),
                index,
            })
            .collect();
        let chosen = CertificateFormat::ALL
            .iter()
            .position(|format| *format == client.format)
            .unwrap_or(0);
        let format = cx.new(|cx| SelectState::new(choices, Some(IndexPath::new(chosen)), window, cx));
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .default_value(client.password.clone())
        });
        let subscriptions = vec![cx.subscribe(&format, |_, _, _: &SelectEvent<Vec<Choice>>, cx| cx.notify())];
        Self {
            index,
            host,
            format,
            certificate: client.certificate,
            key: client.key,
            password,
            missing: false,
            _subscriptions: subscriptions,
        }
    }

    fn format_now(&self, cx: &App) -> CertificateFormat {
        let index = self.format.read(cx).selected_value().copied().unwrap_or(0);
        CertificateFormat::ALL[index.min(CertificateFormat::ALL.len() - 1)]
    }

    fn client(&self, cx: &App) -> ClientCertificate {
        let format = self.format_now(cx);
        ClientCertificate {
            host: self.host.read(cx).value().trim().to_string(),
            format,
            certificate: self.certificate.clone(),
            key: match format {
                CertificateFormat::Pem => self.key.clone(),
                CertificateFormat::Pkcs12 => String::new(),
            },
            password: match format {
                CertificateFormat::Pem => String::new(),
                CertificateFormat::Pkcs12 => self.password.read(cx).value().to_string(),
            },
        }
    }

    /// The certificate's file, or the key's.
    fn pick(&mut self, key: bool, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let path = path.display().to_string();
                if key {
                    this.key = path;
                } else {
                    this.certificate = path;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Saves it, when it names a host and its files.
    fn submit(&mut self, cx: &mut Context<Self>) -> bool {
        let client = self.client(cx);
        if !client.is_complete() {
            self.missing = true;
            cx.notify();
            return false;
        }
        let index = self.index;
        set_certificates(cx, |prefs| match index.filter(|index| *index < prefs.clients.len()) {
            Some(index) => prefs.clients[index] = client,
            None => prefs.clients.push(client),
        });
        true
    }

    fn file_row(&self, label: String, path: &str, key: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let shown = if path.is_empty() {
            div()
                .text_color(muted)
                .child(t!("settings.certificate_none").to_string())
        } else {
            div().child(file_name(path))
        };
        v_flex()
            .gap_1p5()
            .child(div().text_sm().font_medium().child(label))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(shown),
                    )
                    .child(
                        Button::new(if key {
                            "certificate-pick-key"
                        } else {
                            "certificate-pick"
                        })
                        .small()
                        .label(t!("settings.certificate_choose").to_string())
                        .on_click(cx.listener(move |this, _, window, cx| this.pick(key, window, cx))),
                    ),
            )
            .when(!path.is_empty(), |this| {
                this.child(div().text_xs().text_color(muted).child(path.to_string()))
            })
    }
}

impl Render for ClientForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let field = |label: String, control: AnyElement| {
            v_flex()
                .gap_1p5()
                .child(div().text_sm().font_medium().child(label))
                .child(control)
        };
        let format = self.format_now(cx);
        let certificate_label = match format {
            CertificateFormat::Pem => t!("settings.certificate_file"),
            CertificateFormat::Pkcs12 => t!("settings.certificate_p12_file"),
        }
        .to_string();
        let certificate = self.certificate.clone();
        let key = self.key.clone();
        v_flex()
            .gap_3()
            .child(field(
                t!("settings.certificate_host").to_string(),
                Input::new(&self.host).into_any_element(),
            ))
            .child(field(
                t!("settings.certificate_format").to_string(),
                Select::new(&self.format).small().into_any_element(),
            ))
            .child(self.file_row(certificate_label, &certificate, false, cx))
            .when(format == CertificateFormat::Pem, |this| {
                this.child(self.file_row(t!("settings.certificate_key").to_string(), &key, true, cx))
            })
            .when(format == CertificateFormat::Pkcs12, |this| {
                this.child(field(
                    t!("settings.certificate_password").to_string(),
                    Input::new(&self.password).into_any_element(),
                ))
            })
            .when(self.missing && !self.client(cx).is_complete(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(t!("settings.certificate_missing").to_string()),
                )
            })
    }
}
