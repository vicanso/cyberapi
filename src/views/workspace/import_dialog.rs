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
use std::cell::RefCell;
use std::rc::Rc;

/// F10.1: what the clipboard holds, when it looks like something to import
/// — JSON, formatted, or a cURL command.
fn from_clipboard(cx: &App) -> String {
    let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
        return String::new();
    };
    let trimmed = text.trim();
    if trimmed.len() >= 4 && trimmed[..4].eq_ignore_ascii_case("curl") {
        return trimmed.to_string();
    }
    let looks_json =
        (trimmed.starts_with('{') && trimmed.ends_with('}')) || (trimmed.starts_with('[') && trimmed.ends_with(']'));
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(json) if looks_json => serde_json::to_string_pretty(&json).unwrap_or_else(|_| trimmed.to_string()),
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

pub fn open_import_dialog(
    state: WeakEntity<WorkspaceState>,
    parent: Option<ItemId>,
    window: &mut Window,
    cx: &mut App,
) {
    let text = from_clipboard(cx);
    let editor: Entity<EditorState> = cx.new(|cx| {
        EditorState::new(window, cx)
            .language("json")
            .line_number(true)
            .soft_wrap(true)
            .placeholder(t!("import.placeholder").to_string())
            .default_value(text)
    });
    let error: Rc<RefCell<Option<String>>> = Rc::default();
    let reads_files = this_build::reads_files();
    window.open_dialog(cx, move |dialog, _, cx| {
        let text = editor.read(cx).value().to_string();
        let detected = import::detect(&text).ok().map(|(format, _)| format_name(format));
        let submit = {
            let (editor, error, state, parent) = (editor.clone(), error.clone(), state.clone(), parent.clone());
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                let text = editor.read(cx).value().to_string();
                match import::parse(&text) {
                    Ok(bundle) => {
                        let _ = state.update(cx, |state, cx| state.import(parent.clone(), bundle, cx));
                        true
                    }
                    Err(failure) => {
                        *error.borrow_mut() = Some(failure.to_string());
                        window.refresh();
                        false
                    }
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
            .title(t!("import.title").to_string())
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
                            .child(div().text_xs().text_color(muted).when_some(detected, |this, format| {
                                this.child(t!("import.detected", format = format).to_string())
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
