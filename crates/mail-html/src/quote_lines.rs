//! The text of a mail as the lines a reader sees, and the lines that say an
//! earlier message starts.
//!
//! Mail clients agree on very little markup for a quoted message, and what
//! they write is rewritten by every client that quotes it again: classes are
//! prefixed, styles are re-serialised, wrappers are dropped. What survives
//! is the text. A block of "From / Sent / To" lines, an "Original Message"
//! rule and an "On ... wrote:" line are written for the reader, and the
//! reader is who they are read like here.

use blitz_dom::{BaseDocument, NodeId, local_name};

/// What in the text says an earlier message starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Signal {
    /// Lines labelled From, then Sent, To or Subject.
    HeaderBlock,
    /// "-----Original Message-----" and its kin.
    Separator,
    /// "On Monday at 10:02, Ann wrote:".
    Attribution,
}

/// One line of text.
pub(crate) struct Line {
    pub text: String,
    /// The text node the line starts with.
    pub first: NodeId,
}

/// How the line naming the sender is labelled, lowercase.
pub(crate) const FROM: [&str; 9] = [
    "from",
    "de",
    "von",
    "da",
    "van",
    "från",
    "fra",
    "expéditeur",
    "sender",
];

/// Labels of the lines that follow it in a header block.
const FOLLOWING: [&str; 21] = [
    "date",
    "sent",
    "envoyé",
    "gesendet",
    "datum",
    "inviato",
    "data",
    "verzonden",
    "enviado",
    "to",
    "à",
    "a",
    "an",
    "aan",
    "para",
    "subject",
    "objet",
    "betreff",
    "oggetto",
    "asunto",
    "onderwerp",
];

/// What a separator line says between its dashes.
const SEPARATORS: [&str; 10] = [
    "original message",
    "message d'origine",
    "message d\u{2019}origine",
    "forwarded message",
    "message transféré",
    "message transmis",
    "ursprüngliche nachricht",
    "weitergeleitete nachricht",
    "mensaje original",
    "messaggio originale",
];

/// Separators Apple Mail writes without dashes, before a colon.
const SPOKEN_SEPARATORS: [&str; 3] = [
    "begin forwarded message",
    "début du message réexpédié",
    "début du message transféré",
];

/// How an attribution line ends.
const WROTE: [&str; 7] = [
    "wrote:",
    "a écrit :",
    "a écrit:",
    "schrieb:",
    "escribió:",
    "ha scritto:",
    "schreef:",
];

/// Lines a header block may leave between its sender and its next label.
const HEADER_REACH: usize = 5;

/// Elements whose text is not on the page.
pub(crate) fn is_unseen(document: &BaseDocument, id: NodeId) -> bool {
    let Some(node) = document.get_node(id) else {
        return true;
    };
    [
        local_name!("head"),
        local_name!("style"),
        local_name!("script"),
        local_name!("title"),
    ]
    .iter()
    .any(|tag| node.data.is_element_with_tag_name(tag))
}

/// Elements that start and end a line.
pub(crate) fn is_block(document: &BaseDocument, id: NodeId) -> bool {
    let Some(node) = document.get_node(id) else {
        return false;
    };
    [
        local_name!("div"),
        local_name!("p"),
        local_name!("tr"),
        local_name!("li"),
        local_name!("blockquote"),
        local_name!("table"),
        local_name!("pre"),
        local_name!("hr"),
        local_name!("center"),
        local_name!("body"),
    ]
    .iter()
    .any(|tag| node.data.is_element_with_tag_name(tag))
}

/// Every line of text under `root`, in reading order.
pub(crate) fn collect(document: &BaseDocument, root: NodeId) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut open = false;
    walk(document, root, &mut lines, &mut open);
    lines
}

fn walk(document: &BaseDocument, id: NodeId, lines: &mut Vec<Line>, open: &mut bool) {
    let Some(node) = document.get_node(id) else {
        return;
    };
    if node.is_text_node() {
        let text = node.text_content();
        if text.trim().is_empty() {
            return;
        }
        if !*open {
            lines.push(Line {
                text: String::new(),
                first: id,
            });
            *open = true;
        }
        if let Some(line) = lines.last_mut() {
            line.text.push_str(&text);
            line.text.push(' ');
        }
        return;
    }
    if is_unseen(document, id) {
        return;
    }
    if node.data.is_element_with_tag_name(&local_name!("br")) {
        *open = false;
        return;
    }
    let block = is_block(document, id);
    if block {
        *open = false;
    }
    for &child in &node.children {
        walk(document, child, lines, open);
    }
    if block {
        *open = false;
    }
}

/// A line without the non-breaking spaces and edge blanks mail clients add,
/// in lowercase.
pub(crate) fn clean(text: &str) -> String {
    text.replace('\u{a0}', " ").trim().to_lowercase()
}

/// The label of a "Label: value" line that has a value.
pub(crate) fn label(text: &str) -> Option<String> {
    let text = clean(text);
    let (label, value) = text.split_once(':')?;
    let label = label.trim();
    (!value.trim().is_empty() && !label.is_empty() && label.chars().count() <= 12)
        .then(|| label.to_owned())
}

/// Whether a line is a rule between two messages.
pub(crate) fn is_separator(text: &str) -> bool {
    let text = clean(text);
    let bare = text.trim_matches(|c: char| "-_=*: ".contains(c));
    (bare != text && SEPARATORS.contains(&bare))
        || SPOKEN_SEPARATORS.contains(&text.trim_end_matches([':', ' ']))
}

/// An attribution line without the words that end it: "On Monday, Ann" for
/// "On Monday, Ann wrote:". `None` for a line that ends another way.
pub(crate) fn before_wrote(text: &str) -> Option<String> {
    let line = text.replace('\u{a0}', " ");
    let line = line.trim();
    let lower = line.to_lowercase();
    let wrote = WROTE.iter().find(|wrote| lower.ends_with(**wrote))?;
    let kept = line.chars().count().checked_sub(wrote.chars().count())?;
    Some(
        line.chars()
            .take(kept)
            .collect::<String>()
            .trim()
            .to_owned(),
    )
}

/// What line `index` says, if it says an earlier message starts.
pub(crate) fn signal_at(lines: &[Line], index: usize) -> Option<Signal> {
    let text = &lines[index].text;
    if is_separator(text) {
        return Some(Signal::Separator);
    }
    if label(text).is_some_and(|label| FROM.contains(&label.as_str())) {
        let followed = lines[index + 1..]
            .iter()
            .take(HEADER_REACH)
            .filter_map(|line| label(&line.text))
            .any(|label| FOLLOWING.contains(&label.as_str()));
        return followed.then_some(Signal::HeaderBlock);
    }
    let text = clean(text);
    let attribution = text.chars().count() < 300
        && WROTE.iter().any(|wrote| text.ends_with(wrote))
        && text.chars().any(|c| c.is_ascii_digit() || c == '@');
    attribution.then_some(Signal::Attribution)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines of a mail holding `texts`, one per line.
    fn lines(texts: &[&str]) -> Vec<Line> {
        let html = texts
            .iter()
            .map(|text| text.replace('<', "&lt;"))
            .collect::<Vec<_>>()
            .join("<br>");
        let document = crate::parse_mail(&html, blitz_dom::DocumentConfig::default());
        collect(&document, document.root_node().id)
    }

    #[test]
    fn a_header_block_needs_a_sender_and_another_label() {
        let header = lines(&[
            "De\u{a0}: Ann Example <ann@example.com>",
            "Envoyé : lundi",
            "À : Bob",
        ]);
        assert_eq!(signal_at(&header, 0), Some(Signal::HeaderBlock));
        assert_eq!(signal_at(&header, 1), None);
        let sentence = lines(&["From: here to there", "and back again."]);
        assert_eq!(signal_at(&sentence, 0), None);
        let empty = lines(&["From:", "Sent: Monday"]);
        assert_eq!(signal_at(&empty, 0), None);
    }

    #[test]
    fn separators_are_read_with_and_without_dashes() {
        for text in [
            "-----Original Message-----",
            "---------- Forwarded message ---------",
            "-------- Message d'origine --------",
            "Begin forwarded message:",
            "Début du message réexpédié :",
        ] {
            assert!(is_separator(text), "{text}");
        }
        assert!(!is_separator("Original message"));
        assert!(!is_separator("See the forwarded message below."));
    }

    #[test]
    fn an_attribution_ends_the_line_and_carries_a_date_or_an_address() {
        let found = lines(&[
            "Le lun. 26 oct. 2026 à 22:48, Ann Example a écrit :",
            "On Monday, Ann <ann@example.com> wrote:",
            "Ann wrote:",
            "As she wrote: nothing changes on the 3rd.",
        ]);
        assert_eq!(signal_at(&found, 0), Some(Signal::Attribution));
        assert_eq!(signal_at(&found, 1), Some(Signal::Attribution));
        assert_eq!(signal_at(&found, 2), None);
        assert_eq!(signal_at(&found, 3), None);
    }
}
