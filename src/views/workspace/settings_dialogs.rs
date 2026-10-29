//! What a folder and the project set for their APIs (#66): a folder's auth
//! and the headers every API in it sends, the project's auth — and what
//! the folder is for (F3), and the variables its APIs read (D5). Kept when
//! Save is clicked; a viewer reads them and closes.

use super::auth_editor::{AuthEditor, AuthFor};
use super::request_editor::{COMMON_HEADERS, HEADER_EXAMPLE};
use crate::states::WorkspaceState;
use crate::views::kv_editor::{KvEditor, KvOptions};
use crate::views::name_dialog::ok_button;
use crate::views::template_hints::HintSource;
use cyberapi_core::ItemId;
use cyberapi_core::model::{FolderSettings, ProjectSettings};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, px,
};
use rust_i18n::t;

/// D5: a line of a variables list as text.
pub(super) const VARIABLE_EXAMPLE: &str = "userId: 42";

/// A part of a dialog: its title, what it is for, and its fields.
fn section(title: String, about: String, body: impl IntoElement, cx: &App) -> AnyElement {
    v_flex()
        .gap_2()
        .child(
            v_flex()
                .gap_0p5()
                .child(div().text_sm().font_medium().child(title))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(about)),
        )
        .child(body)
        .into_any_element()
}

/// Cancel and Save for an editor; Close alone for a viewer.
fn footer(
    id: &'static str,
    editable: bool,
    save: impl Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static,
) -> DialogFooter {
    let leave = if editable {
        t!("common.cancel")
    } else {
        t!("common.close")
    }
    .to_string();
    let footer = DialogFooter::new().child(DialogClose::new().trigger(move |button| button.label(leave.clone())));
    if editable {
        footer.child(ok_button(id, t!("common.save").to_string(), save))
    } else {
        footer
    }
}

struct FolderSettingsView {
    description: Entity<EditorState>,
    auth: Entity<AuthEditor>,
    headers: Entity<KvEditor>,
    /// D5.
    variables: Entity<KvEditor>,
}

impl Render for FolderSettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        v_flex()
            .gap_5()
            .child(section(
                t!("docs.description").to_string(),
                t!("folder_settings.description_hint").to_string(),
                div()
                    .h(px(96.))
                    .border_1()
                    .border_color(border)
                    .rounded_md()
                    .child(Editor::new(&self.description).w_full().h_full().bordered(false)),
                cx,
            ))
            .child(section(
                t!("editor.auth").to_string(),
                t!("folder_settings.auth_hint").to_string(),
                self.auth.clone(),
                cx,
            ))
            .child(section(
                t!("editor.headers").to_string(),
                t!("folder_settings.headers_hint").to_string(),
                self.headers.clone(),
                cx,
            ))
            .child(section(
                t!("own_variables.title").to_string(),
                t!("own_variables.folder_hint").to_string(),
                self.variables.clone(),
                cx,
            ))
    }
}

/// #66: a folder's auth and headers.
pub fn open_folder_settings(workspace: &Entity<WorkspaceState>, id: ItemId, window: &mut Window, cx: &mut App) {
    let (folder, editable) = {
        let state = workspace.read(cx);
        (state.folder(&id).cloned(), state.editable())
    };
    let Some(folder) = folder else { return };
    let auth = cx.new(|cx| AuthEditor::new(AuthFor::Folder(id.clone()), workspace.clone(), window, cx));
    let hints = HintSource::new(workspace);
    let headers = cx.new(|cx| {
        let options = KvOptions {
            presets: &COMMON_HEADERS,
            example: HEADER_EXAMPLE,
            ..Default::default()
        };
        KvEditor::new(&folder.settings.headers, options, window, cx).hints(hints.clone())
    });
    let variables = cx.new(|cx| {
        let options = KvOptions {
            example: VARIABLE_EXAMPLE,
            ..Default::default()
        };
        KvEditor::new(&folder.settings.variables, options, window, cx).hints(hints)
    });
    let description = cx.new(|cx| {
        let mut editor = EditorState::new(window, cx)
            .line_number(false)
            .soft_wrap(true)
            .placeholder(t!("docs.placeholder").to_string());
        editor.set_value(folder.settings.description.clone(), window, cx);
        editor
    });
    let view = cx.new(|_| FolderSettingsView {
        description: description.clone(),
        auth: auth.clone(),
        headers: headers.clone(),
        variables: variables.clone(),
    });
    let title = t!("folder_settings.title", name = folder.name).to_string();
    let workspace = workspace.downgrade();
    window.open_dialog(cx, move |dialog, _, _| {
        let save = {
            let (auth, headers, workspace, id) = (auth.clone(), headers.clone(), workspace.clone(), id.clone());
            let (description, variables) = (description.clone(), variables.clone());
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let settings = FolderSettings {
                    auth: auth.read(cx).auth(cx),
                    headers: headers.read(cx).list(cx),
                    description: description.read(cx).value().trim_end().to_string(),
                    variables: variables.read(cx).list(cx),
                };
                if let Some(workspace) = workspace.upgrade() {
                    workspace.update(cx, |state, cx| state.set_folder_settings(id.clone(), settings, cx));
                }
                true
            }
        };
        dialog
            .title(title.clone())
            .w(px(660.))
            .child(crate::views::dialog_body(view.clone()))
            .footer(footer("folder-settings-save", editable, save))
            // Enter in a field is the field's, not Save.
            .on_ok(|_, _, _| false)
    });
}

/// #66: the project's auth, for the APIs and folders that inherit.
pub fn open_project_auth(workspace: &Entity<WorkspaceState>, window: &mut Window, cx: &mut App) {
    let editable = workspace.read(cx).editable();
    let auth = cx.new(|cx| AuthEditor::new(AuthFor::Project, workspace.clone(), window, cx));
    let workspace = workspace.downgrade();
    window.open_dialog(cx, move |dialog, _, cx| {
        let save = {
            let (auth, workspace) = (auth.clone(), workspace.clone());
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                if let Some(workspace) = workspace.upgrade() {
                    // The rest of the project's settings stay (D3).
                    let settings = ProjectSettings {
                        auth: auth.read(cx).auth(cx),
                        ..workspace.read(cx).project_settings().clone()
                    };
                    workspace.update(cx, |state, cx| state.set_project_settings(settings, cx));
                }
                true
            }
        };
        dialog
            .title(t!("project_auth.title").to_string())
            .w(px(600.))
            .child(crate::views::dialog_body(section(
                t!("editor.auth").to_string(),
                t!("project_auth.hint").to_string(),
                auth.clone(),
                cx,
            )))
            .footer(footer("project-auth-save", editable, save))
            .on_ok(|_, _, _| false)
    });
}
