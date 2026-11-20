//! Who wrote a quoted message, when and to whom, read from what its mail
//! client put above it: Gmail's "On ... wrote:" line, or the block of
//! labelled lines Outlook and forwards write.

use blitz_dom::{BaseDocument, NodeId, local_name};

use crate::quote_lines;

/// The header of a quoted message, as far as the mail that quotes it tells.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuoteHeader {
    pub name: Option<String>,
    pub address: Option<String>,
    /// The date as written, in the language of the mail client that wrote it.
    pub date: Option<String>,
    pub to: Option<String>,
    pub cc: Option<String>,
}

/// A header that was read, with the nodes that spelled it out: the caller
/// shows the header its own way and takes those nodes off the page.
pub(crate) struct Found {
    pub header: QuoteHeader,
    pub nodes: Vec<NodeId>,
}

use quote_lines::FROM;
const DATE: [&str; 8] = [
    "date",
    "sent",
    "envoyé",
    "gesendet",
    "datum",
    "inviato",
    "data",
    "verzonden",
];
const TO: [&str; 5] = ["to", "à", "a", "an", "aan"];
const CC: [&str; 1] = ["cc"];

/// Words a mail client puts before the date of an attribution line.
const DATE_LEADS: [&str; 6] = ["Le ", "On ", "Am ", "El ", "Op ", "Il giorno "];

/// Labels of lines a header block holds besides the ones that are read.
const OTHER: [&str; 16] = [
    "subject",
    "objet",
    "betreff",
    "oggetto",
    "asunto",
    "onderwerp",
    "bcc",
    "cci",
    "reply-to",
    "répondre à",
    "importance",
    "priority",
    "priorité",
    "attachments",
    "pièces jointes",
    "anlagen",
];

/// Lines read at the top of a quote when looking for its header.
const HEADER_SEARCH: usize = 60;

/// Read the header that opens the quoted message `card`, if it has one this
/// module understands. Anything else is left on the page as it was written.
///
/// The nodes returned are the first children of `card`, in a row, up to the
/// end of the header: the rule above it, the separator line, the lines.
pub(crate) fn find(document: &BaseDocument, card: NodeId) -> Option<Found> {
    let children = &document.get_node(card)?.children;
    attributed(document, children).or_else(|| labelled_block(document, children, WRAPPERS))
}

/// Read the attribution line written just above the quoted text `card`,
/// as Apple Mail and Thunderbird do: "On Monday at 10:02, Ann wrote:" and
/// then the `blockquote`. The line must be all that the element before the
/// quote holds, since that element is taken off the page.
pub(crate) fn find_above(document: &BaseDocument, card: NodeId) -> Option<Found> {
    let parent = document.get_node(document.get_node(card)?.parent?)?;
    let position = parent.children.iter().position(|&child| child == card)?;
    let (above, lines) = parent.children[..position]
        .iter()
        .rev()
        .map(|&child| (child, lines_of(document, child)))
        .find(|(_, lines)| lines.iter().any(|line| !line.trim().is_empty()))?;
    let mut shown = lines.iter().filter(|line| !line.trim().is_empty());
    let line = shown.next()?;
    if shown.next().is_some() || !document.get_node(above)?.is_element() {
        return None;
    }
    Some(Found {
        header: attribution(line.trim())?,
        nodes: vec![above],
    })
}

/// Wrappers a header is looked for under, one inside another.
const WRAPPERS: usize = 6;

/// Gmail's attribution line, the first thing a quote shows.
fn attributed(document: &BaseDocument, children: &[NodeId]) -> Option<Found> {
    for (index, &child) in children.iter().enumerate() {
        let node = document.get_node(child)?;
        let lines = lines_of(document, child);
        if lines.iter().all(|line| line.trim().is_empty()) {
            continue;
        }
        let marked = node
            .attr(local_name!("class"))
            .is_some_and(|class| class.split_whitespace().any(|name| name == "gmail_attr"));
        // Unmarked, the line must be all the child holds: the child is
        // taken off the page.
        let mut shown = lines.iter().filter(|line| !line.trim().is_empty());
        let alone = shown.next().is_some() && shown.next().is_none();
        let header = (marked || alone)
            .then(|| attribution(lines.join(" ").trim()))
            .flatten()?;
        return Some(Found {
            header,
            nodes: children[..=index].to_vec(),
        });
    }
    None
}

/// A block of labelled lines, whatever holds them: one element, a paragraph
/// per line, or loose text between line breaks.
///
/// The header is given as children of one element, in a row. When the
/// child that holds it also holds the start of the message, the header is
/// looked for among the children of that child, `wrappers` levels down.
fn labelled_block(document: &BaseDocument, children: &[NodeId], wrappers: usize) -> Option<Found> {
    let mut lines = vec![String::new()];
    // For each line, the last child that put text on it.
    let mut last_child: Vec<usize> = vec![0];
    // For each line, the first child that put text on it.
    let mut first_child: Vec<usize> = vec![usize::MAX];
    for (index, &child) in children.iter().enumerate() {
        let open = lines.len() - 1;
        let open_text = lines[open].trim().len();
        collect_lines(document, child, &mut lines);
        last_child.resize(lines.len(), 0);
        first_child.resize(lines.len(), usize::MAX);
        for line in open..lines.len() {
            let written = if line == open {
                lines[line].trim().len() > open_text
            } else {
                !lines[line].trim().is_empty()
            };
            if written {
                first_child[line] = first_child[line].min(index);
                last_child[line] = index;
            }
        }
        if lines.len() > HEADER_SEARCH {
            break;
        }
    }

    let shown = |line: &String| !line.trim().is_empty();
    let first = lines
        .iter()
        .position(|line| shown(line) && !quote_lines::is_separator(line))?;
    let label_of = |line: &String| quote_lines::label(line);
    if !label_of(&lines[first]).is_some_and(|label| FROM.contains(&label.as_str())) {
        return None;
    }
    let mut last = first;
    for (index, line) in lines.iter().enumerate().skip(first + 1) {
        if !shown(line) {
            continue;
        }
        let known = label_of(line).is_some_and(|label| {
            let label = label.as_str();
            FROM.contains(&label)
                || DATE.contains(&label)
                || TO.contains(&label)
                || CC.contains(&label)
                || OTHER.contains(&label)
        });
        if !known {
            break;
        }
        last = index;
    }
    let header = labelled(&lines[first..=last])?;
    let end = last_child[last];
    let shared =
        (last + 1..lines.len()).any(|line| shown(&lines[line]) && first_child[line] <= end);
    if !shared {
        return Some(Found {
            header,
            nodes: children[..=end].to_vec(),
        });
    }
    // The child that holds the header also holds the start of the message:
    // taking it off the page would take the message with it. When the
    // header is all in that child, it is one of its children instead.
    let inside = document.get_node(children[end])?;
    (first_child[first] == end && wrappers > 0 && inside.is_element())
        .then(|| labelled_block(document, &inside.children, wrappers - 1))
        .flatten()
}

/// The text under a node, one string per line as a reader sees them.
fn lines_of(document: &BaseDocument, id: NodeId) -> Vec<String> {
    let mut lines = vec![String::new()];
    collect_lines(document, id, &mut lines);
    lines
}

fn collect_lines(document: &BaseDocument, id: NodeId, lines: &mut Vec<String>) {
    let Some(node) = document.get_node(id) else {
        return;
    };
    if node.is_text_node() {
        if let Some(line) = lines.last_mut() {
            line.push_str(&node.text_content());
        }
        return;
    }
    if node.data.is_element_with_tag_name(&local_name!("br")) {
        lines.push(String::new());
        return;
    }
    let block = [
        local_name!("div"),
        local_name!("p"),
        local_name!("tr"),
        local_name!("li"),
    ]
    .iter()
    .any(|tag| node.data.is_element_with_tag_name(tag));
    if block {
        lines.push(String::new());
    }
    for &child in &node.children {
        collect_lines(document, child, lines);
    }
    if block {
        lines.push(String::new());
    }
}

/// A block of "Label: value" lines, as Outlook and forwarded messages write
/// it. `None` unless one of them names the sender.
fn labelled(lines: &[String]) -> Option<QuoteHeader> {
    let mut header = QuoteHeader::default();
    let mut has_sender = false;
    for line in lines {
        let line = single_spaced(line);
        let Some((label, value)) = line.split_once(':') else {
            continue;
        };
        let label = label.trim().to_lowercase();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if FROM.contains(&label.as_str()) {
            (header.name, header.address) = person(value);
            has_sender = true;
        } else if DATE.contains(&label.as_str()) {
            header.date = Some(value.to_owned());
        } else if TO.contains(&label.as_str()) {
            header.to = Some(value.to_owned());
        } else if CC.contains(&label.as_str()) {
            header.cc = Some(value.to_owned());
        }
    }
    (has_sender && (header.name.is_some() || header.address.is_some())).then_some(header)
}

/// An attribution line: "On Mon, Oct 26, 2026 at 10:48 PM Ann Example
/// <ann@x> wrote:", with or without the address. The date ends at its time
/// of day and the name follows; a line without a time, or that names
/// nobody, is not understood.
fn attribution(line: &str) -> Option<QuoteHeader> {
    let line = quote_lines::before_wrote(&single_spaced(line))?;
    let address = line.find('<').and_then(|open| {
        let close = open + line[open..].find('>')?;
        let address = line[open + 1..close].trim();
        address.contains('@').then(|| (open, address.to_owned()))
    });
    let before = match &address {
        Some((open, _)) => line[..*open].trim_end(),
        None if line.contains('<') => return None,
        None => line.as_str(),
    };
    let time_end = end_of_time(before)?;
    let date = before[..time_end].trim();
    let date = DATE_LEADS
        .iter()
        .find_map(|lead| date.strip_prefix(lead))
        .unwrap_or(date);
    let name = before[time_end..]
        .trim_matches(|c: char| c == ',' || c == '"' || c.is_whitespace())
        .to_owned();
    let address = address.map(|(_, address)| address);
    (!name.is_empty() || address.is_some()).then(|| QuoteHeader {
        name: (!name.is_empty()).then_some(name),
        address,
        date: Some(date.to_owned()),
        to: None,
        cc: None,
    })
}

/// A line of the source as one line of text: the line breaks and runs of
/// blanks the markup holds inside it, which a reader never sees, become one
/// space. A name wrapped in the source, "Ann\n Example", is "Ann Example".
fn single_spaced(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where the last time of day of `text` ends: after "22:48" or "10:48 PM".
fn end_of_time(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let colon = (1..bytes.len().saturating_sub(2)).rev().find(|&at| {
        bytes[at] == b':'
            && bytes[at - 1].is_ascii_digit()
            && bytes[at + 1].is_ascii_digit()
            && bytes[at + 2].is_ascii_digit()
    })?;
    let end = colon + 3;
    let rest = &text[end..];
    let after_space = rest.trim_start();
    let meridiem = ["AM", "PM", "am", "pm"]
        .iter()
        .find(|half| after_space.starts_with(**half))
        .map_or(0, |half| rest.len() - after_space.len() + half.len());
    Some(end + meridiem)
}

/// The name and the address of "Ann Example <ann@example.com>", either of
/// which may be missing. Outlook also writes "Ann Example [mailto:ann@x]".
fn person(value: &str) -> (Option<String>, Option<String>) {
    let bracket = value
        .find('<')
        .map(|open| (open, 1, '>'))
        .or_else(|| value.find("[mailto:").map(|open| (open, 8, ']')));
    let Some((open, skip, closing)) = bracket else {
        let value = value.trim().to_owned();
        return if value.contains('@') && !value.contains(' ') {
            (None, Some(value))
        } else {
            (Some(value), None)
        };
    };
    let inside = &value[open + skip..];
    let address = inside[..inside.find(closing).unwrap_or(inside.len())].trim();
    let name = value[..open].trim().trim_matches('"').trim();
    (
        (!name.is_empty()).then(|| name.to_owned()),
        address.contains('@').then(|| address.to_owned()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    #[test]
    fn a_gmail_line_gives_the_name_the_address_and_the_date() {
        let header =
            attribution("Le lun. 26 oct. 2026 à 22:48, Ann Example <ann@example.com> a écrit :")
                .expect("a header");
        assert_eq!(header.name.as_deref(), Some("Ann Example"));
        assert_eq!(header.address.as_deref(), Some("ann@example.com"));
        assert_eq!(header.date.as_deref(), Some("lun. 26 oct. 2026 à 22:48"));

        let header =
            attribution("On Mon, Oct 26, 2026 at 10:48 PM Ann Example <ann@example.com> wrote:")
                .expect("a header");
        assert_eq!(header.name.as_deref(), Some("Ann Example"));
        assert_eq!(
            header.date.as_deref(),
            Some("Mon, Oct 26, 2026 at 10:48 PM")
        );
    }

    #[test]
    fn a_line_without_an_address_still_gives_the_name_and_the_date() {
        let header =
            attribution("Le 12 nov. 2026 à 10:02, Ann Example a écrit\u{a0}:").expect("a header");
        assert_eq!(header.name.as_deref(), Some("Ann Example"));
        assert_eq!(header.address, None);
        assert_eq!(header.date.as_deref(), Some("12 nov. 2026 à 10:02"));
    }

    #[test]
    fn a_line_without_an_address_or_a_time_is_left_alone() {
        assert_eq!(attribution("Ann Example wrote:"), None);
        assert_eq!(attribution("On Monday at 10:48 wrote:"), None);
        assert_eq!(attribution("At 10:48 Ann said hello."), None);
        assert_eq!(attribution("Yesterday Ann <ann@example.com> wrote:"), None);
        assert_eq!(attribution("At 10:48 <not an address> wrote:"), None);
    }

    #[test]
    fn a_name_wrapped_in_the_source_is_read_on_one_line() {
        // One line as the reader sees it, broken in the markup after the
        // first name.
        let header = labelled(&[
            "De : Ann\r\n   Example".to_owned(),
            "Envoyé : lundi 2\u{a0}novembre  2026 11:32".to_owned(),
        ])
        .expect("a header");
        assert_eq!(header.name.as_deref(), Some("Ann Example"));
        assert_eq!(header.date.as_deref(), Some("lundi 2 novembre 2026 11:32"));

        let header =
            attribution("Le 12 nov. 2026 à 10:02, Ann\n Example a écrit :").expect("a header");
        assert_eq!(header.name.as_deref(), Some("Ann Example"));
    }

    #[test]
    fn labelled_lines_give_every_field_in_several_languages() {
        let header = labelled(&lines(
            "De : Ann Example <ann@example.com>\n\
             Envoyé : lundi 26 octobre 2026 22:25\n\
             À : Bob Example <bob@example.com>; Cy <cy@example.com>\n\
             Cc : Dee <dee@example.com>\n\
             Objet : Re: Hello",
        ))
        .expect("a header");
        assert_eq!(header.name.as_deref(), Some("Ann Example"));
        assert_eq!(header.address.as_deref(), Some("ann@example.com"));
        assert_eq!(header.date.as_deref(), Some("lundi 26 octobre 2026 22:25"));
        assert_eq!(
            header.to.as_deref(),
            Some("Bob Example <bob@example.com>; Cy <cy@example.com>")
        );
        assert_eq!(header.cc.as_deref(), Some("Dee <dee@example.com>"));

        let header = labelled(&lines(
            "From: Ann Example [mailto:ann@example.com]\nSent: Monday",
        ))
        .expect("a header");
        assert_eq!(header.address.as_deref(), Some("ann@example.com"));
        assert_eq!(header.date.as_deref(), Some("Monday"));
    }

    #[test]
    fn lines_that_name_no_sender_are_not_a_header() {
        assert_eq!(labelled(&lines("Note: call me\nDate: tomorrow")), None);
        assert_eq!(labelled(&lines("A plain paragraph.")), None);
    }
}
