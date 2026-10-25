//! Completing a GraphQL query from its endpoint's schema: the fields of the
//! type a selection set selects from, a field's arguments, an argument's
//! enum values, in the query editor's own menu. What is offered is
//! `cyberapi_core::graphql_schema::complete`'s; the schema is the one the
//! project keeps for the API's URI (`ApiState::ask_schema`). Inside a
//! template expression the template hints are offered instead.

use super::template_hints::{self, HintSource};
use crate::states::ApiState;
use cyberapi_core::graphql_schema::{self, Suggestion, SuggestionKind};
use gpui_kit::component::input::{CompletionProvider, EditorState, Rope, RopeExt as _};
use gpui_kit::{App, Entity, Task, WeakEntity, Window};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit, Documentation,
    TextEdit,
};
use std::rc::Rc;

/// `suggestion` as an item of the menu, replacing the bytes `from..caret`.
fn menu_item(text: &Rope, from: usize, caret: usize, suggestion: Suggestion) -> CompletionItem {
    let range = lsp_types::Range {
        start: text.offset_to_position(from),
        end: text.offset_to_position(caret),
    };
    // The menu paints as much of the name as `filter_text` is long: what
    // was typed of it.
    let painted = suggestion.label.get(..caret - from).unwrap_or_default().to_string();
    CompletionItem {
        label: suggestion.label,
        detail: Some(suggestion.detail),
        kind: Some(match suggestion.kind {
            SuggestionKind::Field => CompletionItemKind::FIELD,
            SuggestionKind::Argument => CompletionItemKind::PROPERTY,
            SuggestionKind::EnumValue => CompletionItemKind::ENUM_MEMBER,
            SuggestionKind::Keyword => CompletionItemKind::KEYWORD,
            SuggestionKind::Type => CompletionItemKind::CLASS,
        }),
        documentation: (!suggestion.description.is_empty()).then_some(Documentation::String(suggestion.description)),
        filter_text: Some(painted),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: suggestion.insert,
        })),
        ..Default::default()
    }
}

struct QueryHints {
    api: WeakEntity<ApiState>,
    templates: Rc<dyn CompletionProvider>,
}

impl CompletionProvider for QueryHints {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        context: CompletionContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<gpui_kit::Result<CompletionResponse>> {
        let row = text.offset_to_point(offset).row;
        let start = text.line_start_offset(row);
        let line = text.slice_line(row).to_string();
        if cyberapi_core::template_hints::in_expression(&line, offset - start) {
            return self.templates.completions(text, offset, context, window, cx);
        }
        let none = || Task::ready(Ok(CompletionResponse::Array(Vec::new())));
        // Typing opens the menu in a name, not after a brace or a space —
        // where Enter, which the menu takes, is the reader's line break.
        let typed_name = context.trigger_character.is_none()
            || line[..offset - start]
                .chars()
                .next_back()
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_');
        let schema = self.api.upgrade().and_then(|api| api.read(cx).schema(cx));
        let Some(schema) = schema.filter(|_| typed_name) else {
            return none();
        };
        let completions = graphql_schema::complete(&schema, &text.to_string(), offset);
        let items = completions
            .items
            .into_iter()
            .map(|suggestion| menu_item(text, completions.from, offset, suggestion))
            .collect();
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        !new_text.is_empty()
    }
}

/// Gives the query editor `editor` the schema's completions of `api`'s
/// endpoint, and the template hints in an expression. Called as it is built.
pub fn complete_in(editor: &mut EditorState, api: &Entity<ApiState>, templates: &HintSource) {
    let hints = QueryHints {
        api: api.downgrade(),
        templates: template_hints::provider(templates),
    };
    template_hints::complete_with(editor, Rc::new(hints));
}
