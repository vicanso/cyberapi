//! Completing a template expression where one is typed (M6): after `{{`,
//! the functions; inside `value(`, the custom variables; inside `get(`, the
//! APIs, then the fields of the chosen one's last answer. What is offered,
//! and what a choice writes, is `cyberapi_core::template_hints`'; this
//! shows it.
//!
//! The request body and a list edited as text are code editors, whose own
//! menu shows the hints at the caret ([`complete_in`], [`watch`]). A
//! list's values are single-line fields, which have no such menu, so
//! [`FieldHints`] draws one under the field.

use crate::states::{ApiState, WorkspaceState};
use crate::this_build;
use cyberapi_core::ItemId;
use cyberapi_core::template_hints::{self, Field, Hint, HintKind, ResolvedVar, VarRef, VarSource};
use gpui_kit::component::input::{
    CompletionProvider, EditorState, Enter, HoverProvider, InputEvent, InputState, Rope, RopeExt as _,
};
use gpui_kit::component::{ActiveTheme as _, ThemeStyled as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Entity, Focusable as _, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement as _,
    ScrollHandle, StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window, anchored,
    deferred, div, px,
};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit, Hover,
    HoverContents, MarkupContent, MarkupKind, TextEdit,
};
use rust_i18n::t;
use std::collections::HashSet;
use std::rc::Rc;

/// The hints for a caret: at once, or once the answer a `get` path reads
/// has been read.
pub enum Hints {
    Now(Vec<Hint>),
    Later(Task<Vec<Hint>>),
}

/// Where the hints come from: an open project's custom variables and APIs,
/// and its APIs' last answers. When bound to an `ApiState`, variable previews
/// follow that API's own variables and effective environment (including in an
/// apart window).
#[derive(Clone)]
pub struct HintSource {
    workspace: WeakEntity<WorkspaceState>,
    api: Option<WeakEntity<ApiState>>,
}

impl HintSource {
    pub fn new(workspace: &Entity<WorkspaceState>) -> Self {
        Self {
            workspace: workspace.downgrade(),
            api: None,
        }
    }

    pub fn for_api(workspace: &Entity<WorkspaceState>, api: &Entity<ApiState>) -> Self {
        Self {
            workspace: workspace.downgrade(),
            api: Some(api.downgrade()),
        }
    }

    /// The hints for the caret at byte `caret` of `text`; the file
    /// functions only where this build reads files.
    pub fn hints(&self, text: &str, caret: usize, cx: &mut App) -> Hints {
        let Some(workspace) = self.workspace.upgrade() else {
            return Hints::Now(Vec::new());
        };
        let mut sources = workspace.read(cx).hint_sources(this_build::reads_files());
        let Some(api) = template_hints::path_api(text, caret).map(|api| ItemId::from(api.as_str())) else {
            return Hints::Now(template_hints::hints(text, caret, &sources));
        };
        if let Some(known) = workspace.read(cx).known_answer(&api) {
            sources.response = known;
            return Hints::Now(template_hints::hints(text, caret, &sources));
        }
        let answer = workspace.update(cx, |workspace, cx| workspace.answer(api, cx));
        let text = text.to_string();
        Hints::Later(cx.foreground_executor().spawn(async move {
            sources.response = answer.await;
            template_hints::hints(&text, caret, &sources)
        }))
    }

    /// UI-9: resolves a variable reference in this source's workspace / API.
    pub fn resolve(&self, var: &VarRef, cx: &App) -> Option<ResolvedVar> {
        let workspace = self.workspace.upgrade()?;
        let ws = workspace.read(cx);
        if let Some(api) = self.api.as_ref().and_then(WeakEntity::upgrade) {
            let api = api.read(cx);
            let env = api.effective_environment(ws).map(|env| &env.id);
            Some(ws.resolve_var(
                &var.name,
                var.kind,
                Some(api.api()),
                Some(&api.request().variables),
                env,
            ))
        } else {
            let env = ws.current_environment().map(|env| &env.id);
            Some(ws.resolve_var(&var.name, var.kind, None, None, env))
        }
    }

    /// UI-9: compact multi-line tooltip for single-line fields (KV values, URI)
    /// containing `{{value(...)}}` references; `None` when there are none.
    pub fn preview_tooltip(&self, text: &str, cx: &App) -> Option<String> {
        let vars = template_hints::referenced_vars(text);
        if vars.is_empty() {
            return None;
        }
        let mut seen = HashSet::new();
        let mut lines = Vec::new();
        for var in vars {
            if !seen.insert((var.kind, var.name.clone())) {
                continue;
            }
            if let Some(resolved) = self.resolve(&var, cx) {
                lines.push(format_preview_line(&resolved));
            }
        }
        (!lines.is_empty()).then(|| lines.join("\n"))
    }
}

/// UI-9: human-readable source layer label (e.g. `Staging`, `Shared`, `Folder: Auth`, `API`).
pub fn format_source(source: &VarSource, secret: bool) -> String {
    let base = match source {
        VarSource::ApiOwn => t!("hints.source_api").to_string(),
        VarSource::Folder(name) => t!("hints.source_folder", name = name).to_string(),
        VarSource::NamedEnv(name) => name.clone(),
        VarSource::Shared => t!("hints.source_shared").to_string(),
        VarSource::Undefined => t!("hints.source_undefined").to_string(),
    };
    if secret && !matches!(source, VarSource::Undefined) {
        format!("{base} · {}", t!("hints.source_secret"))
    } else {
        base
    }
}

/// UI-9: single-line summary for tooltips: `NAME = value (source)` or `NAME — Undefined`.
pub fn format_preview_line(resolved: &ResolvedVar) -> String {
    let source = format_source(&resolved.source, resolved.secret);
    match &resolved.value {
        Some(value) if value.is_empty() => {
            format!("{} = {} ({source})", resolved.name, t!("hints.value_empty"))
        }
        Some(value) => format!("{} = {value} ({source})", resolved.name),
        None => format!("{} — {source}", resolved.name),
    }
}

/// UI-9: Markdown content for `HoverProvider` popovers in code editors.
fn format_hover_markdown(resolved: &ResolvedVar) -> String {
    let source = format_source(&resolved.source, resolved.secret);
    match &resolved.value {
        Some(value) if value.is_empty() => {
            format!("**`{}`** · *{source}*\n\n*{}*", resolved.name, t!("hints.value_empty"))
        }
        Some(value) => format!(
            "**`{}`** · *{source}*\n\n`{}`",
            resolved.name,
            value.replace('`', "\\`")
        ),
        None => format!("**`{}`** · *{source}*", resolved.name),
    }
}

/// What a hint says beside its name: what a function does (and its short
/// name), a variable's value, the folders an API is in, what a field holds.
fn detail(hint: &Hint) -> String {
    match &hint.kind {
        HintKind::Function(function) => {
            let key = format!("hints.{}", function.name);
            let about = t!(key.as_str());
            match function.alias {
                Some(alias) => format!("{alias} · {about}"),
                None => about.to_string(),
            }
        }
        HintKind::DynamicVar(var) => {
            let key = format!("hints.dyn_{}", var.category.key());
            format!("{} · {}", t!(key.as_str()), var.example)
        }
        HintKind::Custom(value) => value.clone(),
        HintKind::Api(api) => api.place.clone(),
        HintKind::Field(Field::Object(count)) => crate::i18n::counted("hints.object", *count),
        HintKind::Field(Field::Array(count)) => crate::i18n::counted("hints.array", *count),
        HintKind::Field(Field::Value(value)) => value.clone(),
    }
}

/// The line `offset` is on, and where it starts.
fn caret_line(text: &Rope, offset: usize) -> (String, usize) {
    let row = text.offset_to_point(offset).row;
    (text.slice_line(row).to_string(), text.line_start_offset(row))
}

/// `hint` as an item of a code editor's menu, its line starting at byte
/// `start` of `text`.
fn menu_item(text: &Rope, start: usize, hint: Hint) -> CompletionItem {
    // The menu paints as much of the name as `filter_text` is long.
    let painted = hint.label[..hint.matched].to_string();
    let range = lsp_types::Range {
        start: text.offset_to_position(start + hint.replace.start),
        end: text.offset_to_position(start + hint.replace.end),
    };
    CompletionItem {
        label: hint.label.clone(),
        detail: Some(detail(&hint)),
        kind: Some(match hint.kind {
            HintKind::Function(_) => CompletionItemKind::FUNCTION,
            HintKind::DynamicVar(_) => CompletionItemKind::CONSTANT,
            HintKind::Custom(_) => CompletionItemKind::VARIABLE,
            HintKind::Api(_) => CompletionItemKind::REFERENCE,
            HintKind::Field(_) => CompletionItemKind::FIELD,
        }),
        filter_text: Some(painted),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: hint.insert,
        })),
        ..Default::default()
    }
}

/// The hints in a code editor's completion menu.
struct EditorHints(HintSource);

impl CompletionProvider for EditorHints {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<gpui_kit::Result<CompletionResponse>> {
        // An expression is one line: only the caret's is read.
        let (line, start) = caret_line(text, offset);
        let caret = offset - start;
        let items = move |text: &Rope, hints: Vec<Hint>| -> Vec<CompletionItem> {
            hints.into_iter().map(|hint| menu_item(text, start, hint)).collect()
        };
        match self.0.hints(&line, caret, cx) {
            Hints::Now(hints) => Task::ready(Ok(CompletionResponse::Array(items(text, hints)))),
            Hints::Later(hints) => {
                let text = text.clone();
                cx.foreground_executor().spawn(async move {
                    let hints = hints.await;
                    Ok(CompletionResponse::Array(items(&text, hints)))
                })
            }
        }
    }

    /// Every keystroke: the list follows what is typed, and closes where no
    /// expression is.
    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        !new_text.is_empty()
    }
}

/// UI-9: hover preview for `{{value(...)}}` expressions in code editors.
struct EditorHover(HintSource);

impl HoverProvider for EditorHover {
    fn hover(&self, text: &Rope, offset: usize, _: &mut Window, cx: &mut App) -> Task<gpui_kit::Result<Option<Hover>>> {
        let (line, start) = caret_line(text, offset);
        let col = offset.saturating_sub(start);
        let Some(var_ref) = template_hints::expression_at(&line, col) else {
            return Task::ready(Ok(None));
        };
        let Some(resolved) = self.0.resolve(&var_ref, cx) else {
            return Task::ready(Ok(None));
        };
        let range = lsp_types::Range {
            start: text.offset_to_position(start + var_ref.range.start),
            end: text.offset_to_position(start + var_ref.range.end),
        };
        let hover = Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format_hover_markdown(&resolved),
            }),
            range: Some(range),
        };
        Task::ready(Ok(Some(hover)))
    }
}

/// The hints as a code editor's completion provider, for one that offers
/// its own around the expressions (a GraphQL query's fields).
pub fn provider(source: &HintSource) -> Rc<dyn CompletionProvider> {
    Rc::new(EditorHints(source.clone()))
}

/// UI-9: registers variable hover preview on `editor`.
pub fn hover_in(editor: &mut EditorState, source: &HintSource) {
    editor.lsp_mut().hover_provider = Some(Rc::new(EditorHover(source.clone())));
}

/// Gives the code editor `editor` the hints, in its own menu, and hover
/// previews for `{{value(...)}}` variables. Called as it is built.
pub fn complete_in(editor: &mut EditorState, source: &HintSource) {
    complete_with(editor, provider(source));
    hover_in(editor, source);
}

/// Gives the code editor `editor` `provider`'s menu.
pub fn complete_with(editor: &mut EditorState, provider: Rc<dyn CompletionProvider>) {
    let lsp = editor.lsp_mut();
    lsp.completion_provider = Some(provider);
    // Room for what a function does beside its name.
    lsp.completion_menu.max_width = px(480.);
}

/// Keeps `editor`'s menu in step with the hints: it opens again where a
/// choice leaves the caret at the start of the next part (`value(`, `get(`,
/// a path's `.`), as a choice from the menu is not typing, which is what
/// opens it; and Enter goes to it before the editor's own handling (the
/// kit's menu chooses on Enter but lets the key go on, which typed a line
/// break before the choice went in).
pub fn watch(editor: &Entity<EditorState>, window: &mut Window, cx: &mut App) -> [Subscription; 2] {
    let follow = window.subscribe(editor, cx, move |editor, event: &InputEvent, window, cx| {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let opens = {
            let state = editor.read(cx);
            let offset = state.cursor();
            let (line, start) = caret_line(state.text(), offset);
            template_hints::starts_part(&line, offset - start)
        };
        if opens {
            editor.update(cx, |editor, cx| editor.show_completions(window, cx));
        }
    });
    // Before any binding: once the editor's Enter has run, the key has
    // been typed.
    let weak = editor.downgrade();
    let enter = cx.intercept_keystrokes(move |event, window, cx| {
        let keystroke = &event.keystroke;
        if keystroke.key != "enter" || keystroke.modifiers.modified() {
            return;
        }
        let Some(editor) = weak.upgrade() else { return };
        let state = editor.read(cx);
        if !state.completion_menu_state().open || !state.focus_handle(cx).is_focused(window) {
            return;
        }
        editor.update(cx, |editor, cx| {
            let enter = Enter {
                secondary: false,
                shift: false,
            };
            editor.route_overlay_action(Box::new(enter), window, cx);
        });
        cx.stop_propagation();
    });
    [follow, enter]
}

/// What a key did to the list under a field.
pub enum HintKey {
    /// Up or down: another hint is marked.
    Moved,
    /// Enter or Tab: this one goes in.
    Choose(Hint),
    /// Escape: the list closes.
    Close,
    /// The caret moved: the list closes, and the field still takes the key.
    Leave,
    /// Not the list's.
    Pass,
}

/// The hints under a single-line field, which has no completion menu of its
/// own: open while the caret is in an expression it can complete, steered
/// with the arrow keys, Enter or Tab to choose, Escape to close.
pub struct FieldHints {
    /// Which of the host's fields it is under.
    pub field: usize,
    hints: Vec<Hint>,
    selected: usize,
    scroll: ScrollHandle,
}

impl FieldHints {
    /// `hints` under the host's field `field`; `None` for none.
    pub fn new(field: usize, hints: Vec<Hint>) -> Option<Self> {
        (!hints.is_empty()).then(|| Self {
            field,
            hints,
            selected: 0,
            scroll: ScrollHandle::new(),
        })
    }

    pub fn key(&mut self, event: &KeyDownEvent) -> HintKey {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return HintKey::Pass;
        }
        let count = self.hints.len();
        match keystroke.key.as_str() {
            "up" | "down" => {
                let step = if keystroke.key == "up" { count - 1 } else { 1 };
                self.selected = (self.selected + step) % count;
                self.scroll.scroll_to_item(self.selected);
                HintKey::Moved
            }
            "enter" | "tab" => HintKey::Choose(self.hints[self.selected].clone()),
            "escape" => HintKey::Close,
            "left" | "right" | "home" | "end" => HintKey::Leave,
            _ => HintKey::Pass,
        }
    }

    /// The list, to be a child of the field's box (which must be
    /// `relative`), just under it. `choose` writes the hint clicked.
    pub fn render(&self, choose: impl Fn(Hint, &mut Window, &mut App) + 'static, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (radius, marked, muted) = (theme.radius, theme.accent, theme.muted_foreground);
        let choose = Rc::new(choose);
        let rows = self.hints.iter().enumerate().map(|(index, hint)| {
            let (chosen, choose) = (hint.clone(), choose.clone());
            h_flex()
                .id(("field-hint", index))
                .flex_none()
                .h(px(26.))
                .px_2()
                .gap_3()
                .rounded(radius)
                .text_sm()
                .cursor_pointer()
                .when(index == self.selected, |row| row.bg(marked))
                .hover(|style| style.bg(marked))
                // Down, not up: the field keeps its focus, and the list
                // goes before the click ends.
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    choose(chosen.clone(), window, cx);
                })
                // The name first; a long one is cut, then what it says.
                .child(div().min_w_0().truncate().child(hint.label.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(muted)
                        .child(detail(hint)),
                )
        });
        div().absolute().top_full().left_0().child(
            deferred(
                anchored().snap_to_window_with_margin(px(8.)).child(
                    v_flex()
                        .id("field-hints")
                        .occlude()
                        .popover_style(cx)
                        .mt_1()
                        .p_1()
                        .w(px(380.))
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .children(rows),
                ),
            )
            .with_priority(1),
        )
    }
}

/// Writes `hint` into `input` in place of what it replaces, the caret after
/// it.
pub fn apply(input: &Entity<InputState>, hint: &Hint, window: &mut Window, cx: &mut App) {
    input.update(cx, |input, cx| {
        input.set_selected_range(hint.replace.clone(), cx);
        input.replace(hint.insert.clone(), window, cx);
        input.focus(window, cx);
    });
}
