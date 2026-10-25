//! Importing into a project (M10): paste or pick the text, see what it is,
//! import it under a folder or on the top level — or, from the projects
//! page, as a new project (#65).

use crate::states::WorkspaceState;
use crate::this_build;
use crate::views::name_dialog::ok_button;
use cyberapi_core::ItemId;
use cyberapi_core::import::{self, Format, ImportBundle};
use cyberapi_core::tree::TreeData;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Editor, EditorState, InputEvent, Paste};
use gpui_kit::component::{ActiveTheme as _, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, Render, SharedString, Styled as _, Subscription, Task, WeakEntity, Window, div, px,
};
use rust_i18n::t;
use std::rc::Rc;
use std::sync::Arc;

/// A text longer than this is not put in the editor whole. The editor lays
/// out and colours everything it holds at once, on the main thread: a few
/// megabytes kept the dialog blank for seconds. Such a text is held beside
/// the editor and imported as it is; the editor shows its start.
const HELD_ABOVE: usize = 256 * 1024;
/// How much of a held text the editor shows.
const HELD_SHOWN: usize = 64 * 1024;

/// B3: the text, when it is an address to fetch what to import from: one
/// http or https URL, nothing around it.
fn address(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let scheme = |prefix: &str| {
        trimmed
            .get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
    };
    ((scheme("http://") || scheme("https://")) && !trimmed.contains(char::is_whitespace)).then_some(trimmed)
}

/// F10.1: what the clipboard holds, when it looks like something to import
/// — JSON, formatted, a cURL command, an OpenAPI or Swagger document in
/// YAML (#59), or an address to fetch one from (B3), as it is; nothing
/// otherwise.
fn importable(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(address) = address(trimmed) {
        return address.to_string();
    }
    // `get`, not a slice: four bytes may end inside a character.
    if trimmed.get(..4).is_some_and(|start| start.eq_ignore_ascii_case("curl")) {
        return trimmed.to_string();
    }
    let looks_json =
        (trimmed.starts_with('{') && trimmed.ends_with('}')) || (trimmed.starts_with('[') && trimmed.ends_with(']'));
    if !looks_json {
        return match import::detect(trimmed) {
            Ok((Format::Swagger | Format::OpenApi, _)) => trimmed.to_string(),
            _ => String::new(),
        };
    }
    // Too big to show whole, so not worth indenting either: it would only
    // grow, and stays as it came.
    if trimmed.len() > HELD_ABOVE {
        return match serde_json::from_str::<serde::de::IgnoredAny>(trimmed) {
            Ok(_) => trimmed.to_string(),
            Err(_) => String::new(),
        };
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(json) => cyberapi_core::format::json(&json),
        Err(_) => String::new(),
    }
}

/// The start of a held text, for the editor.
fn shown_start(text: &str) -> &str {
    &text[..text.floor_char_boundary(HELD_SHOWN)]
}

fn format_name(format: Format) -> &'static str {
    match format {
        Format::Curl => "cURL",
        Format::Postman => "Postman",
        Format::Insomnia => "Insomnia",
        Format::Swagger => "Swagger 2.0",
        Format::OpenApi => "OpenAPI 3",
        Format::CyberApi => "CyberAPI",
    }
}

/// What a text would import, said in a line; or why it would not.
type Found = Result<String, String>;

/// B3: the project an import goes into, as it was when the dialog opened,
/// and where in it.
struct MergeInto {
    tree: TreeData,
    parent: Option<ItemId>,
}

/// F10.2 before importing: what the text is and what it holds, or why it
/// is nothing to import (`None` for no text); and, going into a project,
/// how many of its APIs the project has already (B3).
fn describe(text: &str, into: Option<&MergeInto>) -> (Option<Found>, usize) {
    if text.trim().is_empty() {
        return (None, 0);
    }
    let format = match import::detect(text) {
        Ok((format, _)) => format_name(format),
        Err(failure) => {
            return (
                Some(Err(
                    t!("import.unreadable", why = crate::i18n::import_error(&failure)).to_string()
                )),
                0,
            );
        }
    };
    let mut known = 0;
    let found = match import::parse(text) {
        Ok(bundle) => {
            if let Some(into) = into {
                known = import::merge::merge(&into.tree, into.parent.as_ref(), &bundle.items, false).matched;
            }
            let (apis, folders) = bundle.counts();
            let mut found = t!(
                "import.found",
                format = format,
                apis = crate::i18n::counted("workspace.api", apis),
                folders = crate::i18n::counted("import.folder", folders)
            )
            .to_string();
            if !bundle.environment.is_empty() {
                let variables = crate::i18n::counted("import.variable", bundle.environment.len());
                found += &t!("import.found_variables", variables = variables);
            }
            Ok(found)
        }
        Err(failure) => Err(t!("import.unreadable", why = crate::i18n::import_error(&failure)).to_string()),
    };
    (Some(found), known)
}

pub fn open_import_dialog(
    state: WeakEntity<WorkspaceState>,
    parent: Option<ItemId>,
    window: &mut Window,
    cx: &mut App,
) {
    // B3: what the project has, to import again into.
    let into = state.upgrade().map(|state| {
        Arc::new(MergeInto {
            tree: state.read(cx).tree_data().clone(),
            parent: parent.clone(),
        })
    });
    let import = move |bundle, again: Option<bool>, _: &mut Window, cx: &mut App| {
        let _ = state.update(cx, |state, cx| match again {
            Some(update) => state.import_again(parent.clone(), bundle, update, cx),
            None => state.import(parent.clone(), bundle, cx),
        });
    };
    open(t!("import.dialog_title").to_string(), Rc::new(import), into, window, cx);
}

/// #65: what is imported becomes a new project; `import` names and makes it.
pub fn open_import_project_dialog(
    import: impl Fn(ImportBundle, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let import = move |bundle, _: Option<bool>, window: &mut Window, cx: &mut App| import(bundle, window, cx);
    open(
        t!("import.project_title").to_string(),
        Rc::new(import),
        None,
        window,
        cx,
    );
}

/// What the dialog does with the bundle the text holds, and whether it is
/// imported again (B3): `Some` when the project has some of its APIs,
/// `true` to update them.
type OnImport = Rc<dyn Fn(ImportBundle, Option<bool>, &mut Window, &mut App)>;

/// The dialog's text and what it would import.
struct ImportView {
    editor: Entity<EditorState>,
    /// The whole text, when it is too big for the editor ([`HELD_ABOVE`]):
    /// the editor shows its start, read-only.
    held: Option<SharedString>,
    /// What the text would import, worked out off the main thread whenever
    /// it changes.
    found: Option<Found>,
    /// A picked file that could not be read, or an address that gave
    /// nothing.
    error: Option<String>,
    /// B3: the text is an address, to fetch what to import from.
    address: Option<String>,
    /// B3: the project imported into; how many of the text's APIs it has
    /// already; whether those are updated.
    into: Option<Arc<MergeInto>>,
    known: usize,
    update: bool,
    fetching: Option<Task<()>>,
    highlighted: &'static str,
    /// The latest `describe`; a newer one drops it.
    describing: Task<()>,
    _changes: Subscription,
}

impl ImportView {
    fn new(into: Option<Arc<MergeInto>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .tab_size(crate::views::json_tab())
                // Wrapping measures every line of the text at once.
                .soft_wrap(false)
                .placeholder(t!("import.placeholder").to_string())
        });
        let changes = cx.subscribe(&editor, |this, editor, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = editor.read(cx).value();
                this.describe(text, cx);
            }
        });
        Self {
            editor,
            held: None,
            found: None,
            error: None,
            address: None,
            into,
            known: 0,
            update: false,
            fetching: None,
            highlighted: "json",
            describing: Task::ready(()),
            _changes: changes,
        }
    }

    /// All the text, held or in the editor.
    fn text(&self, cx: &App) -> SharedString {
        self.held.clone().unwrap_or_else(|| self.editor.read(cx).value())
    }

    /// Puts `text` in place of what the dialog had: a big one is held, and
    /// the editor shows its start.
    fn fill(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let held = text.len() > HELD_ABOVE;
        let shown = if held {
            shown_start(&text).to_string()
        } else {
            text.clone()
        };
        self.held = held.then(|| text.into());
        self.editor.update(cx, |editor, cx| {
            editor.set_readonly(held, cx);
            editor.set_value(shown, window, cx);
        });
        // `set_value` says nothing of the change.
        let text = self.text(cx);
        self.describe(text, cx);
    }

    /// What `text` would import, worked out in the background: parsing a
    /// big collection twice takes long enough to be felt.
    fn describe(&mut self, text: SharedString, cx: &mut Context<Self>) {
        // A cURL command, or YAML, is no JSON to colour.
        let language = if text.trim_start().starts_with(['{', '[']) {
            "json"
        } else {
            "text"
        };
        if language != self.highlighted {
            self.highlighted = language;
            self.editor
                .update(cx, |editor, cx| editor.set_highlighter(language, cx));
        }
        if text.trim().is_empty() {
            self.found = None;
            self.known = 0;
            self.address = None;
            self.describing = Task::ready(());
            cx.notify();
            return;
        }
        // An address imports nothing itself: what it holds does, fetched.
        self.address = address(&text).map(str::to_string);
        if self.address.is_some() {
            self.found = Some(Ok(t!("import.fetch_hint").to_string()));
            self.known = 0;
            self.describing = Task::ready(());
            cx.notify();
            return;
        }
        let into = self.into.clone();
        let found = cx.background_spawn(async move { describe(&text, into.as_deref()) });
        self.describing = cx.spawn(async move |this, cx| {
            let (found, known) = found.await;
            let _ = this.update(cx, |this, cx| {
                this.found = found;
                this.known = known;
                cx.notify();
            });
        });
    }

    /// A paste the editor could not take quickly is held instead: a big
    /// text, or anything while a text is held. A small one is the editor's
    /// own paste.
    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        // The desktop answers at once.
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if self.held.is_some() || text.len() > HELD_ABOVE {
                cx.stop_propagation();
                self.fill(text, window, cx);
            }
            return;
        }
        // A page's synchronous read is always empty. Asked now, while the
        // paste still counts as the reader's, the editor's own paste could
        // not tell a big text in time, so this one does both.
        cx.stop_propagation();
        let read = cx.read_from_clipboard_async();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Some(item)) = read.await else { return };
            let Some(text) = item.text() else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.held.is_some() || text.len() > HELD_ABOVE {
                    this.fill(text, window, cx);
                } else {
                    this.editor.update(cx, |editor, cx| editor.replace(text, window, cx));
                }
            });
        })
        .detach();
    }

    /// B3: what the address holds, in its place.
    fn fetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(url) = self.address.clone() else { return };
        if self.fetching.is_some() {
            return;
        }
        let fetched = crate::states::AppStore::global(cx).fetch_text(url, cx);
        self.error = None;
        self.fetching = Some(cx.spawn_in(window, async move |this, cx| {
            let fetched = fetched.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.fetching = None;
                match fetched {
                    Ok(text) => this.fill(text, window, cx),
                    Err(why) => {
                        this.error = Some(t!("import.fetch_failed", why = why).to_string());
                        cx.notify();
                    }
                }
            });
        }));
        cx.notify();
    }

    fn pick(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let read = cx.background_spawn(async move { this_build::read_text(&path) }).await;
            let _ = this.update_in(cx, |this, window, cx| match read {
                Ok(text) => {
                    this.error = None;
                    this.fill(text, window, cx);
                }
                Err(message) => {
                    this.error = Some(message);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for ImportView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let held = self.held.as_ref().map(|text| {
            t!(
                "import.held",
                size = cyberapi_core::response::format_size(text.len() as u64)
            )
            .to_string()
        });
        v_flex()
            .gap_2()
            .child(div().text_sm().text_color(muted).child(t!("import.hint").to_string()))
            .child(
                div()
                    .h(px(320.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
                    .relative()
                    .capture_action(cx.listener(Self::paste))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .child(Editor::new(&self.editor).w_full().h_full().bordered(false)),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .gap_0p5()
                            .text_xs()
                            .when_some(held, |this, held| this.child(div().text_color(muted).child(held)))
                            .map(|this| match &self.found {
                                Some(Ok(found)) => this.child(div().text_color(muted).child(found.clone())),
                                Some(Err(why)) => this.child(div().text_color(danger).child(why.clone())),
                                None => this,
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .when(self.address.is_some(), |this| {
                                this.child(
                                    Button::new("import-fetch")
                                        .primary()
                                        .small()
                                        .loading(self.fetching.is_some())
                                        .label(t!("import.fetch").to_string())
                                        .on_click(cx.listener(|this, _, window, cx| this.fetch(window, cx))),
                                )
                            })
                            .when(self.held.is_some(), |this| {
                                this.child(
                                    Button::new("import-clear")
                                        .outline()
                                        .small()
                                        .label(t!("import.clear").to_string())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.fill(String::new(), window, cx);
                                        })),
                                )
                            })
                            .when(this_build::reads_files(), |this| {
                                this.child(
                                    Button::new("import-file")
                                        .outline()
                                        .small()
                                        .label(t!("import.choose_file").to_string())
                                        .on_click(cx.listener(Self::pick)),
                                )
                            }),
                    ),
            )
            // B3: the APIs the project has already, left or updated.
            .when(self.known > 0, |this| {
                let update = self.update;
                this.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div().text_sm().child(
                                        t!("import.known", apis = crate::i18n::counted("workspace.api", self.known))
                                            .to_string(),
                                    ),
                                )
                                .child(
                                    [t!("import.add_new"), t!("import.update_known")]
                                        .into_iter()
                                        .enumerate()
                                        .fold(
                                            ButtonGroup::new("import-again").compact().small().outline(),
                                            |group, (index, label)| {
                                                group.child(
                                                    Button::new(("import-again", index))
                                                        .label(label.to_string())
                                                        .selected((index == 1) == update),
                                                )
                                            },
                                        )
                                        .on_click(cx.listener(|this, picks: &Vec<usize>, _, cx| {
                                            this.update = picks.first() == Some(&1);
                                            cx.notify();
                                        })),
                                ),
                        )
                        .when(update, |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(t!("import.update_note").to_string()),
                            )
                        }),
                )
            })
            .when_some(self.error.clone(), |this, message| {
                this.child(div().text_sm().text_color(danger).child(message))
            })
    }
}

fn open(title: String, import: OnImport, into: Option<Arc<MergeInto>>, window: &mut Window, cx: &mut App) {
    // F10.1: the clipboard, read because the reader asked to import, and
    // asked for now, while that click still counts as theirs: a page may
    // read it only then, through the browser, which may ask the reader
    // first. A page's synchronous read is always empty.
    let clipboard = cx.read_from_clipboard_async();
    let view = cx.new(|cx| ImportView::new(into, window, cx));
    let filling = view.downgrade();
    window
        .spawn(cx, async move |cx| {
            // Not readable, or turned down: the reader pastes or picks a file.
            let Ok(Some(item)) = clipboard.await else { return };
            let text = item.text().unwrap_or_default();
            let text = cx.background_spawn(async move { importable(&text) }).await;
            let _ = cx.update(|window, cx| {
                let _ = filling.update(cx, |view, cx| {
                    // What the reader typed, pasted or picked meanwhile stays.
                    if !text.is_empty() && view.text(cx).is_empty() {
                        view.fill(text, window, cx);
                    }
                });
            });
        })
        .detach();
    window.open_dialog(cx, move |dialog, _, _| {
        let submit = {
            let (view, import) = (view.clone(), import.clone());
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let (text, again) = {
                    let view = view.read(cx);
                    (view.text(cx), (view.known > 0).then_some(view.update))
                };
                match import::parse(&text) {
                    Ok(bundle) => {
                        import(bundle, again, window, cx);
                        true
                    }
                    // Why shows under the text already (`describe`).
                    Err(_) => false,
                }
            }
        };
        dialog
            .title(title.clone())
            .w(px(720.))
            .child(crate::views::dialog_body(view.clone()))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                    .child(ok_button("import-ok", t!("import.import").to_string(), submit.clone())),
            )
            .on_ok(submit)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AnyWindowHandle, ClipboardItem, Focusable as _, TestAppContext, size};
    use std::sync::Arc;

    /// Answers `/spec` with a collection, anything else with a 404.
    struct Specs;

    impl cyberapi_http::HttpEngine for Specs {
        fn send(&self, request: cyberapi_http::PreparedRequest) -> cyberapi_http::SendFuture {
            let found = request.url.ends_with("/spec");
            Box::pin(async move {
                Ok(cyberapi_core::response::Exchange {
                    status: if found { 200 } else { 404 },
                    body: if found {
                        big_collection(2).into_bytes()
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                })
            })
        }
    }

    /// An import view in a window, the app's state behind it; and how to
    /// paste into its editor.
    fn opened(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<ImportView>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::states::AppStore::init(
                cyberapi_store::MemoryStore::new().storage(),
                Arc::new(Specs),
                Default::default(),
                crate::Extras::default(),
                cx,
            );
        });
        let mut view = None;
        let window: AnyWindowHandle = cx
            .open_window(size(px(720.), px(480.)), |window, cx| {
                let import = cx.new(|cx| ImportView::new(None, window, cx));
                view = Some(import.clone());
                Root::new(import, window, cx)
            })
            .into();
        (window, view.expect("built"))
    }

    fn paste_into(window: AnyWindowHandle, view: &Entity<ImportView>, text: &str, cx: &mut TestAppContext) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
        cx.update_window(window, |_, window, cx| {
            view.read(cx).editor.read(cx).focus_handle(cx).focus(window, cx);
            window.render_frame(cx);
            window.press("secondary-v", cx);
        })
        .expect("the window is open");
        cx.run_until_parked();
    }

    /// B3: going into a project, the dialog counts the APIs it has already
    /// — the same method to the same path — and they are left unless the
    /// reader picks updating them.
    #[gpui_kit::test]
    async fn the_apis_a_project_has_are_counted(cx: &mut TestAppContext) {
        let (window, _) = opened(cx);
        let tree = TreeData {
            apis: vec![cyberapi_core::model::ApiSetting {
                id: "a-0".into(),
                name: "api 0".into(),
                request: cyberapi_core::model::HttpRequest {
                    uri: "{{HOST}}/0/".into(),
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        };
        let into = Arc::new(MergeInto { tree, parent: None });
        let mut view = None;
        cx.update_window(window, |_, window, cx| {
            view = Some(cx.new(|cx| ImportView::new(Some(into), window, cx)));
        })
        .expect("the window is open");
        let view = view.expect("made");
        // The collection's `api 0` goes to `https://a.test/0/x…`: another path.
        let text = big_collection(1).replace("https://a.test/0/", "https://a.test/0");
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |view, cx| view.fill(text, window, cx))
        })
        .expect("the window is open");
        cx.run_until_parked();
        let (known, update) = cx.read(|cx| (view.read(cx).known, view.read(cx).update));
        assert_eq!((known, update), (0, false), "another path");
        let text = big_collection(1).replace(&"x".repeat(80), "");
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |view, cx| view.fill(text, window, cx))
        })
        .expect("the window is open");
        cx.run_until_parked();
        let (known, update) = cx.read(|cx| (view.read(cx).known, view.read(cx).update));
        assert_eq!((known, update), (1, false), "the same path; left unless picked");
    }

    /// B3: an address says what Fetch does, and Fetch puts what it holds in
    /// its place; one that gives nothing says why.
    #[gpui_kit::test]
    async fn an_address_is_fetched(cx: &mut TestAppContext) {
        let (window, view) = opened(cx);
        paste_into(window, &view, "https://specs.test/spec", cx);
        cx.read(|cx| {
            let view = view.read(cx);
            assert_eq!(view.address.as_deref(), Some("https://specs.test/spec"));
            assert_eq!(view.found, Some(Ok(t!("import.fetch_hint").to_string())));
        });
        let fetch = |cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| {
                view.update(cx, |view, cx| view.fetch(window, cx))
            })
            .expect("the window is open");
            cx.run_until_parked();
        };
        fetch(cx);
        cx.read(|cx| {
            let view = view.read(cx);
            assert_eq!(view.text(cx).as_ref(), big_collection(2));
            assert!(view.address.is_none());
            assert!(
                matches!(&view.found, Some(Ok(found)) if found.contains("Postman")),
                "{:?}",
                view.found
            );
        });

        cx.update_window(window, |_, window, cx| {
            view.update(cx, |view, cx| view.fill(String::new(), window, cx))
        })
        .expect("the window is open");
        paste_into(window, &view, "https://specs.test/gone", cx);
        fetch(cx);
        cx.read(|cx| {
            let view = view.read(cx);
            assert_eq!(
                view.text(cx).as_ref(),
                "https://specs.test/gone",
                "what was there stays"
            );
            assert!(
                view.error.as_deref().is_some_and(|error| error.contains("404")),
                "{:?}",
                view.error
            );
        });
    }

    /// F10.1: a command or a JSON document fills the dialog; anything else
    /// leaves it empty for the reader.
    #[test]
    fn only_what_imports_fills_the_dialog() {
        assert_eq!(importable("  curl https://a.test \n"), "curl https://a.test");
        assert_eq!(importable(r#"{"item":[]}"#), "{\n    \"item\": []\n}");
        assert_eq!(importable("hello"), "");
        assert_eq!(importable("{ not json }"), "");
        // YAML only when it is an API description (#59).
        let openapi = "openapi: 3.0.1\ninfo:\n  title: Pets\npaths: {}";
        assert_eq!(importable(&format!("\n{openapi}\n")), openapi);
        assert_eq!(importable("name: tree\nage: 3"), "");
        assert_eq!(importable(""), "");
        // A character of several bytes across the fourth: no panic.
        assert_eq!(importable("新建接口"), "");
        assert_eq!(importable("cu新"), "");
        // B3: an address, to fetch from.
        assert_eq!(
            importable(" https://a.test/openapi.yaml\n"),
            "https://a.test/openapi.yaml"
        );
        assert_eq!(importable("HTTP://a.test/x"), "HTTP://a.test/x");
        assert_eq!(importable("see https://a.test"), "");
        assert_eq!(importable("https://a.test/a b"), "");
        assert_eq!(importable("ftp://a.test/x"), "");
    }

    /// A document too big to show whole is kept as it came, not indented
    /// to twice its size; one that is no JSON still fills nothing.
    #[test]
    fn a_big_document_stays_as_it_came() {
        let big = format!(r#"{{"item":[{}]}}"#, vec![r#"{"name":"a"}"#; HELD_ABOVE / 10].join(","));
        assert!(big.len() > HELD_ABOVE);
        assert_eq!(importable(&format!(" {big}\n")), big);
        let broken = format!("{}}}", &big[..big.len() - 2]);
        assert_eq!(importable(&broken), "");
    }

    /// The editor shows the start of a held text, cut on a character.
    #[test]
    fn a_held_text_shows_its_start() {
        let text = "新".repeat(HELD_SHOWN);
        let shown = shown_start(&text);
        assert!(shown.len() <= HELD_SHOWN && shown.len() > HELD_SHOWN - 3);
        assert!(text.starts_with(shown));
        assert_eq!(shown_start("short"), "short");
    }

    /// A Postman collection of `apis` requests, too big for the editor.
    fn big_collection(apis: usize) -> String {
        let items: Vec<String> = (0..apis)
            .map(|n| {
                format!(
                    r#"{{"name":"api {n}","request":{{"method":"GET","url":{{"raw":"https://a.test/{n}/{}"}}}}}}"#,
                    "x".repeat(80)
                )
            })
            .collect();
        format!(
            r#"{{"info":{{"name":"big","schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"}},"item":[{}]}}"#,
            items.join(",")
        )
    }

    /// A big paste is held: the editor shows its start and what imports is
    /// all of it. Anything pasted while a text is held takes its place, and
    /// a small paste into the editor is still the editor's own.
    #[gpui_kit::test]
    async fn a_big_paste_is_held_beside_the_editor(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut view = None;
        let window: AnyWindowHandle = cx
            .open_window(size(px(720.), px(480.)), |window, cx| {
                let import = cx.new(|cx| ImportView::new(None, window, cx));
                view = Some(import.clone());
                Root::new(import, window, cx)
            })
            .into();
        let view = view.expect("built");
        let paste = |text: &str, cx: &mut TestAppContext| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
            cx.update_window(window, |_, window, cx| {
                view.read(cx).editor.read(cx).focus_handle(cx).focus(window, cx);
                window.render_frame(cx);
                window.press("secondary-v", cx);
            })
            .expect("the window is open");
            cx.run_until_parked();
        };
        let big = big_collection(3000);
        assert!(big.len() > HELD_ABOVE);
        paste(&big, cx);
        cx.read(|cx| {
            let view = view.read(cx);
            assert_eq!(view.text(cx).as_ref(), big, "the whole text imports");
            let shown = view.editor.read(cx).value();
            assert!(shown.len() <= HELD_SHOWN && big.starts_with(shown.as_ref()));
            assert!(matches!(view.found, Some(Ok(_))), "described: {:?}", view.found);
        });

        paste("curl https://a.test", cx);
        cx.read(|cx| {
            let view = view.read(cx);
            assert!(view.held.is_none());
            assert_eq!(view.text(cx).as_ref(), "curl https://a.test");
        });

        // The caret is at the start of the text it replaced.
        paste("curl https://b.test\n", cx);
        let text = cx.read(|cx| view.read(cx).text(cx));
        assert_eq!(text.as_ref(), "curl https://b.test\ncurl https://a.test");
    }
}
