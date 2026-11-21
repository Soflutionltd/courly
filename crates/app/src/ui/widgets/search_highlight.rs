use std::borrow::Cow;

use iced::widget::{rich_text, span, text};
use iced::{Color, Element, Font, Length, Theme};
use rtsk::search_match::MIN_MATCH_CHARS;
pub use rtsk::search_match::match_ranges;

/// Same yellow in both themes, with dark text, so a match stands out on a
/// light or a dark row alike.
pub const MATCH_BACKGROUND: Color = Color::from_rgb8(0xFF, 0xE0, 0x4D);
const MATCH_TEXT: Color = Color::from_rgb8(0x1F, 0x1F, 0x1F);

/// The words of a search query that matched text, without its operators.
pub fn query_terms(query: &str) -> Vec<String> {
    let parsed = smart_folder::parse_query(query);
    parsed
        .free_text
        .split_whitespace()
        .chain(parsed.from.iter().map(String::as_str))
        .chain(parsed.with.iter().map(String::as_str))
        .map(|word| word.trim_matches('"'))
        .filter(|word| word.chars().count() >= MIN_MATCH_CHARS)
        .map(str::to_owned)
        .collect()
}

/// One line of a thread row, with the query's matches on a yellow background.
pub(super) fn highlighted_line<'a, M: 'a>(
    content: Cow<'a, str>,
    terms: &[String],
    size: f32,
    font: Font,
    style: fn(&Theme) -> text::Style,
) -> Element<'a, M> {
    let ranges = match_ranges(&content, terms);
    if ranges.is_empty() {
        return text(content)
            .size(size)
            .font(font)
            .style(style)
            .wrapping(text::Wrapping::None)
            .ellipsis(text::Ellipsis::End)
            .width(Length::Fill)
            .into();
    }
    let mut spans: Vec<text::Span<'a, ()>> = Vec::with_capacity(ranges.len() * 2 + 1);
    let mut cursor = 0;
    for range in ranges {
        if range.start < cursor {
            continue;
        }
        if cursor < range.start {
            spans.push(span(content[cursor..range.start].to_owned()));
        }
        spans.push(
            span(content[range.clone()].to_owned())
                .background(MATCH_BACKGROUND)
                .color(MATCH_TEXT),
        );
        cursor = range.end;
    }
    if cursor < content.len() {
        spans.push(span(content[cursor..].to_owned()));
    }
    rich_text(spans)
        .size(size)
        .font(font)
        .style(style)
        .wrapping(text::Wrapping::None)
        .ellipsis(text::Ellipsis::End)
        .width(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operators_are_not_highlighted() {
        assert_eq!(query_terms("nath has:attachment"), ["nath"]);
        assert_eq!(query_terms("from:martin facture"), ["facture", "martin"]);
    }

    #[test]
    fn a_single_letter_is_not_highlighted() {
        assert_eq!(query_terms("philippe de r"), ["philippe", "de"]);
    }
}
