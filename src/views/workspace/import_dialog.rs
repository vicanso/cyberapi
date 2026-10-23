//! Importing into a project (M10): paste or pick the text, see what it is,
//! import it under a folder or on the top level.

use crate::states::WorkspaceState;
use crate::this_build;
use crate::views::name_dialog::ok_button;
use cyberapi_core::ItemId;
use cyberapi_core::import::{self, Format};
use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Entity, ParentElement as _, PathPromptOptions, Styled as _, WeakEntity, Window, div, px,
};
use rust_i18n::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// F10.1: what the clipboard holds, when it looks like something to import
/// — JSON, formatted, or a cURL command; nothing otherwise.
fn importable(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.len() >= 4 && trimmed[..4].eq_ignore_ascii_case("curl") {
        return trimmed.to_string();
    }
    let looks_json =
        (trimmed.starts_with('{') && trimmed.ends_with('}')) || (trimmed.starts_with('[') && trimmed.ends_with(']'));
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(json) if looks_json => cyberapi_core::format::json(&json),
        _ => String::new(),
    }
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

/// F10.2 before importing: what the text is and what it holds, or why it
/// is nothing to import. `None` for no text.
fn describe(text: &str) -> Option<Found> {
    if text.trim().is_empty() {
        return None;
    }
    let format = match import::detect(text) {
        Ok((format, _)) => format_name(format),
        Err(failure) => {
            return Some(Err(
                t!("import.unreadable", why = crate::i18n::import_error(&failure)).to_string()
            ));
        }
    };
    Some(match import::parse(text) {
        Ok(bundle) => {
            let (apis, folders) = bundle.counts();
            let mut found = t!("import.found", format = format, apis = apis, folders = folders).to_string();
            if !bundle.environment.is_empty() {
                found += &t!("import.found_variables", count = bundle.environment.len());
            }
            Ok(found)
        }
        Err(failure) => Err(t!("import.unreadable", why = crate::i18n::import_error(&failure)).to_string()),
    })
}

pub fn open_import_dialog(
    state: WeakEntity<WorkspaceState>,
    parent: Option<ItemId>,
    window: &mut Window,
    cx: &mut App,
) {
    // F10.1: the clipboard, read because the reader asked to import, and
    // asked for now, while that click still counts as theirs: a page may
    // read it only then, through the browser, which may ask the reader
    // first. A page's synchronous read is always empty.
    let clipboard = cx.read_from_clipboard_async();
    let editor: Entity<EditorState> = cx.new(|cx| {
        EditorState::new(window, cx)
            .language("json")
            .line_number(true)
            .tab_size(crate::views::json_tab())
            .soft_wrap(true)
            .placeholder(t!("import.placeholder").to_string())
    });
    let filling = editor.clone();
    window
        .spawn(cx, async move |cx| {
            // Not readable, or turned down: the reader pastes or picks a file.
            let Ok(Some(item)) = clipboard.await else { return };
            let text = importable(&item.text().unwrap_or_default());
            let _ = cx.update(|window, cx| {
                // What the reader typed or pasted meanwhile stays.
                if !text.is_empty() && filling.read(cx).value().is_empty() {
                    filling.update(cx, |editor, cx| editor.set_value(text, window, cx));
                }
            });
        })
        .detach();
    // A picked file that could not be read.
    let error: Rc<RefCell<Option<String>>> = Rc::default();
    // What the text would import, worked out again only when it changes.
    let preview: Rc<RefCell<(String, Option<Found>)>> = Rc::default();
    let highlighted = Rc::new(Cell::new("json"));
    let reads_files = this_build::reads_files();
    window.open_dialog(cx, move |dialog, _, cx| {
        let text = editor.read(cx).value().to_string();
        let found = {
            let mut preview = preview.borrow_mut();
            if preview.0 != text {
                *preview = (text.clone(), describe(&text));
                // A cURL command is no JSON to colour.
                let curl = text
                    .trim_start()
                    .get(..4)
                    .is_some_and(|start| start.eq_ignore_ascii_case("curl"));
                let language = if curl { "text" } else { "json" };
                if language != highlighted.get() {
                    highlighted.set(language);
                    editor.update(cx, |editor, cx| editor.set_highlighter(language, cx));
                }
            }
            preview.1.clone()
        };
        let submit = {
            let (editor, state, parent) = (editor.clone(), state.clone(), parent.clone());
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                let text = editor.read(cx).value().to_string();
                match import::parse(&text) {
                    Ok(bundle) => {
                        let _ = state.update(cx, |state, cx| state.import(parent.clone(), bundle, cx));
                        true
                    }
                    // Why shows under the text already (`describe`).
                    Err(_) => false,
                }
            }
        };
        let pick = {
            let (editor, error) = (editor.clone(), error.clone());
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                let picked = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: None,
                });
                let (editor, error) = (editor.clone(), error.clone());
                window
                    .spawn(cx, async move |cx| {
                        let Ok(Ok(Some(paths))) = picked.await else { return };
                        let Some(path) = paths.into_iter().next() else { return };
                        let read = cx.background_spawn(async move { this_build::read_text(&path) }).await;
                        let _ = cx.update(|window, cx| match read {
                            Ok(text) => {
                                editor.update(cx, |editor, cx| editor.set_value(text, window, cx));
                                *error.borrow_mut() = None;
                            }
                            Err(message) => {
                                *error.borrow_mut() = Some(message);
                                window.refresh();
                            }
                        });
                    })
                    .detach();
            }
        };
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        dialog
            .title(t!("import.dialog_title").to_string())
            .w(px(720.))
            .child(
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
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .child(Editor::new(&editor).w_full().h_full().bordered(false)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .justify_between()
                            .child(div().text_xs().map(|this| match found {
                                Some(Ok(found)) => this.text_color(muted).child(found),
                                Some(Err(why)) => this.text_color(danger).child(why),
                                None => this,
                            }))
                            .when(reads_files, |this| {
                                this.child(
                                    Button::new("import-file")
                                        .outline()
                                        .small()
                                        .label(t!("import.choose_file").to_string())
                                        .on_click(pick),
                                )
                            }),
                    )
                    .when_some(error.borrow().clone(), |this, message| {
                        this.child(div().text_sm().text_color(danger).child(message))
                    }),
            )
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

    /// F10.1: a command or a JSON document fills the dialog; anything else
    /// leaves it empty for the reader.
    #[test]
    fn only_what_imports_fills_the_dialog() {
        assert_eq!(importable("  curl https://a.test \n"), "curl https://a.test");
        assert_eq!(importable(r#"{"item":[]}"#), "{\n    \"item\": []\n}");
        assert_eq!(importable("hello"), "");
        assert_eq!(importable("{ not json }"), "");
        assert_eq!(importable(""), "");
    }
}
