use std::borrow::Cow;
use std::ops::Range;

use iced::widget::{rich_text, span, text};
use iced::{Color, Element, Font, Length, Theme};

/// Same yellow in both themes, with dark text, so a match stands out on a
/// light or a dark row alike.
const MATCH_BACKGROUND: Color = Color::from_rgb8(0xFF, 0xE0, 0x4D);
const MATCH_TEXT: Color = Color::from_rgb8(0x1F, 0x1F, 0x1F);

/// The words of a search query that matched text, without its operators.
pub fn query_terms(query: &str) -> Vec<String> {
    let parsed = smart_folder::parse_query(query);
    parsed
        .free_text
        .split_whitespace()
        .chain(parsed.from.iter().map(String::as_str))
        .map(|word| word.trim_matches('"'))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Byte ranges of `content` where a term starts a word, ignoring case:
/// the search matches the first letters of a word, so does the highlight.
fn match_ranges(content: &str, terms: &[String]) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut previous: Option<char> = None;
    for (start, current) in content.char_indices() {
        let word_start = current.is_alphanumeric() && !previous.is_some_and(char::is_alphanumeric);
        previous = Some(current);
        if !word_start {
            continue;
        }
        let rest = &content[start..];
        let longest = terms.iter().filter_map(|term| prefix_len(rest, term)).max();
        if let Some(len) = longest {
            ranges.push(start..start + len);
        }
    }
    ranges
}

/// Bytes of `haystack` matched by `term` from its start, ignoring case.
fn prefix_len(haystack: &str, term: &str) -> Option<usize> {
    let mut chars = haystack.char_indices();
    for wanted in term.chars() {
        let (_, found) = chars.next()?;
        if !found.to_lowercase().eq(wanted.to_lowercase()) {
            return None;
        }
    }
    Some(chars.next().map_or(haystack.len(), |(end, _)| end))
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

    fn matched<'a>(content: &'a str, terms: &[&str]) -> Vec<&'a str> {
        let terms: Vec<String> = terms.iter().map(|term| (*term).to_owned()).collect();
        match_ranges(content, &terms)
            .into_iter()
            .map(|range| &content[range])
            .collect()
    }

    #[test]
    fn a_term_lights_up_the_start_of_each_word_it_begins() {
        assert_eq!(
            matched("Nathalie Martin <nathalie.martin@x.fr>", &["NATH"]),
            ["Nath", "nath"]
        );
    }

    #[test]
    fn a_term_inside_a_word_is_not_a_match() {
        assert!(matched("Jonathan", &["nath"]).is_empty());
    }

    #[test]
    fn the_longest_term_wins_at_one_position() {
        assert_eq!(matched("Réunion", &["ré", "réun"]), ["Réun"]);
    }

    #[test]
    fn operators_are_not_highlighted() {
        assert_eq!(query_terms("nath has:attachment"), ["nath"]);
        assert_eq!(query_terms("from:martin facture"), ["facture", "martin"]);
    }
}
