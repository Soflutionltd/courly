//! Where the words of a search sit in a text.
//!
//! The search matches the first letters of a word, so everything that shows
//! a match to the user follows the same rule: the highlight of a row and of
//! an open mail, and the excerpt a result is previewed with.

use std::ops::Range;

/// Shorter words still filter the results but would match nearly every word
/// of a text.
pub const MIN_MATCH_CHARS: usize = 2;

/// Characters of a mail shown before the matched word in an excerpt: enough
/// to read the word in its sentence, few enough that the word stays inside
/// a narrow list.
const EXCERPT_LEAD_CHARS: usize = 24;
/// Characters of an excerpt: a preview line never shows more.
const EXCERPT_CHARS: usize = 200;

/// The words of the free text of a query that are long enough to be shown
/// as matches.
pub fn free_text_words(free_text: &str) -> Vec<String> {
    free_text
        .split_whitespace()
        .map(|word| word.trim_matches('"'))
        .filter(|word| word.chars().count() >= MIN_MATCH_CHARS)
        .map(str::to_owned)
        .collect()
}

/// Byte ranges of `content` where a term starts a word, ignoring case.
pub fn match_ranges(content: &str, terms: &[String]) -> Vec<Range<usize>> {
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

/// Whether a term starts a word of `content`.
pub fn holds_match(content: &str, terms: &[String]) -> bool {
    !terms.is_empty() && !match_ranges(content, terms).is_empty()
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

/// One line of `text` around the first word a term starts, `None` when no
/// term does. The line opens a few words before the match, on a word, with
/// an ellipsis when the text began earlier; line breaks and runs of spaces
/// read as one space.
pub fn excerpt(text: &str, terms: &[String]) -> Option<String> {
    let found = match_ranges(text, terms).into_iter().next()?;
    let lead_start = text[..found.start]
        .char_indices()
        .rev()
        .take(EXCERPT_LEAD_CHARS)
        .last()
        .map_or(found.start, |(at, _)| at);
    // Open on a whole word unless the lead reaches the start of the text.
    let start = if lead_start == 0 {
        0
    } else {
        text[lead_start..found.start]
            .find(char::is_whitespace)
            .map_or(found.start, |gap| lead_start + gap)
    };
    let mut line = String::with_capacity(EXCERPT_CHARS + 4);
    if !text[..start].trim().is_empty() {
        line.push('\u{2026}');
    }
    let mut written = 0;
    for word in text[start..].split_whitespace() {
        if written >= EXCERPT_CHARS {
            break;
        }
        if written > 0 {
            line.push(' ');
            written += 1;
        }
        if written + word.chars().count() > EXCERPT_CHARS {
            line.extend(word.chars().take(EXCERPT_CHARS - written));
            break;
        }
        line.push_str(word);
        written += word.chars().count();
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    fn matched<'a>(content: &'a str, words: &[&str]) -> Vec<&'a str> {
        match_ranges(content, &terms(words))
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
    fn a_single_letter_is_not_a_word_to_show() {
        assert_eq!(free_text_words("philippe de r"), ["philippe", "de"]);
    }

    #[test]
    fn an_excerpt_opens_a_few_words_before_the_match() {
        let text = "AWS Health Event\r\n\r\nCongratulations! Your setup for the domain is \
complete.\r\n\r\nMessage   metadata\r\nAffected account";
        let line = excerpt(text, &terms(&["meta"])).expect("a match");
        assert!(line.starts_with('\u{2026}'), "{line:?}");
        assert!(
            line.contains("Message metadata Affected account"),
            "{line:?}"
        );
        assert!(!line.contains("Health"), "{line:?}");
        assert!(!line.contains('\n'), "{line:?}");
    }

    #[test]
    fn an_excerpt_of_a_match_near_the_start_opens_the_text() {
        let line = excerpt("Your Metal plan renews soon", &terms(&["meta"])).expect("a match");
        assert_eq!(line, "Your Metal plan renews soon");
    }

    #[test]
    fn an_excerpt_is_cut_to_a_preview_line() {
        let text = format!("Été à la mer {}", "mot ".repeat(200));
        let line = excerpt(&text, &terms(&["été"])).expect("a match");
        assert_eq!(line.chars().count(), EXCERPT_CHARS);
    }

    #[test]
    fn a_text_without_the_word_has_no_excerpt() {
        assert!(excerpt("Jonathan wrote", &terms(&["nath"])).is_none());
    }
}
