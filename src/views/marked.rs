//! A name with what a search found in it marked (F15.1): the text, or the
//! characters whose pinyin the keyword spelt.

use cyberapi_core::search::match_range;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, HighlightStyle, StyledText};

pub fn marked(text: &str, keyword: &str, cx: &App) -> StyledText {
    let styled = StyledText::new(text.to_string());
    let Some(range) = match_range(text, keyword.trim()) else {
        return styled;
    };
    let byte = |at: usize| text.char_indices().nth(at).map_or(text.len(), |(byte, _)| byte);
    let highlight = HighlightStyle {
        background_color: Some(cx.theme().yellow.opacity(0.35)),
        ..Default::default()
    };
    styled.with_highlights(vec![(byte(range.start)..byte(range.end), highlight)])
}
