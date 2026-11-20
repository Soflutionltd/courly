//! A check of the quote detection against the text of a mail.
//!
//! Every line that says an earlier message starts (see
//! [`crate::quote_lines`]) should be where the detection starts a quote. One
//! that is not is a miss. Run over a mailbox, the misses say which mail
//! clients the detection does not understand yet. A miss carries the markup
//! around it, never the text.

use blitz_dom::{BaseDocument, DocumentConfig, LocalName, NodeId, local_name};

pub use crate::quote_lines::Signal;
use crate::quote_lines::{self, Line};
use crate::quotes::{self, FRAME_ATTRIBUTE, HEADER_ATTRIBUTE};

/// A place where the text starts an earlier message and the detection did
/// not start a quote.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Miss {
    pub signal: Signal,
    /// The elements around the line, outermost first, with their classes
    /// and ids: "div#divRplyFwdMsg > p.MsoNormal".
    pub place: String,
}

/// What the detection made of one mail.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuoteAudit {
    /// Quoted messages shown as cards of their own.
    pub cards: usize,
    /// Cards whose header was read.
    pub headers: usize,
    /// Signals the detection answered with the start of a quote.
    pub handled: usize,
    pub misses: Vec<Miss>,
    /// For each card whose header was not read, what its first line is and
    /// the markup around it.
    pub headless: Vec<String>,
}

/// Lines a separator may be from the quote it announces: its header lines
/// come in between.
const SEPARATOR_REACH: usize = 8;

/// Lines a header block may be under the separator that announces it.
const SEPARATOR_TO_HEADER: usize = 3;

/// A mail after the quote detection.
struct Framed {
    document: BaseDocument,
    cards: Vec<quotes::QuoteCard>,
    lines: Vec<Line>,
    /// For each line, the nearest quote it is in.
    frames: Vec<Option<NodeId>>,
    /// For each line, whether it is part of a header the detection read.
    headers: Vec<bool>,
}

fn framed(html: &str) -> Framed {
    // The same base as a render: a relative source needs one to resolve.
    let mut document = crate::parse_mail(
        html,
        DocumentConfig {
            base_url: Some(format!("https://{}/", crate::BASE_HOST)),
            ..DocumentConfig::default()
        },
    );
    let (cards, _) = quotes::frame_quotes(&mut document, false);
    let lines = quote_lines::collect(&document, document.root_node().id);
    let frame = LocalName::from(FRAME_ATTRIBUTE);
    let header = LocalName::from(HEADER_ATTRIBUTE);
    let frames = lines
        .iter()
        .map(|line| ancestor_with(&document, line.first, &frame))
        .collect();
    let headers = lines
        .iter()
        .map(|line| ancestor_with(&document, line.first, &header).is_some())
        .collect();
    Framed {
        document,
        cards,
        lines,
        frames,
        headers,
    }
}

/// The nearest ancestor of `id` carrying `attribute`.
fn ancestor_with(document: &BaseDocument, id: NodeId, attribute: &LocalName) -> Option<NodeId> {
    let mut current = document.get_node(id)?.parent;
    while let Some(id) = current {
        let node = document.get_node(id)?;
        if node.attr(attribute.clone()).is_some() {
            return Some(id);
        }
        current = node.parent;
    }
    None
}

impl Framed {
    /// Whether the detection starts a quote where line `index` says one
    /// starts: the line is in a header it read, it opens a quote, a quote
    /// opens right after it, or it opens the mail, which is then that
    /// message itself.
    fn answered(&self, index: usize, signal: Signal) -> bool {
        if self.headers[index] {
            return true;
        }
        let frame = self.frames[index];
        let opens_frame = frame.is_some()
            && self.frames[..index]
                .last()
                .is_none_or(|before| *before != frame);
        let reach = match signal {
            Signal::Separator => SEPARATOR_REACH,
            Signal::Attribution => 1,
            Signal::HeaderBlock => 0,
        };
        let frame_follows = self.frames[index + 1..]
            .iter()
            .take(reach)
            .any(|after| after.is_some() && *after != frame);
        let opens_mail = (0..index)
            .all(|before| quote_lines::signal_at(&self.lines, before) == Some(Signal::Separator));
        // A header block under a separator is the second thing its quote
        // shows: the quote starts at the separator, and that is the answer.
        let announced = signal == Signal::HeaderBlock
            && (index.saturating_sub(SEPARATOR_TO_HEADER)..index).any(|before| {
                quote_lines::signal_at(&self.lines, before) == Some(Signal::Separator)
                    && self.answered(before, Signal::Separator)
            });
        opens_frame || frame_follows || opens_mail || announced
    }

    fn place_of(&self, index: usize) -> String {
        let element = self
            .document
            .get_node(self.lines[index].first)
            .and_then(|node| node.parent);
        element.map_or_else(String::new, |element| place(&self.document, element))
    }
}

/// Run the quote detection on `html` and compare it with what the text says.
pub fn audit(html: &str) -> QuoteAudit {
    let framed = framed(html);
    let mut audit = QuoteAudit {
        cards: framed.cards.len(),
        headers: framed
            .cards
            .iter()
            .filter(|card| card.header.is_some())
            .count(),
        ..QuoteAudit::default()
    };
    for card in framed.cards.iter().filter(|card| card.header.is_none()) {
        let lines = quote_lines::collect(&framed.document, card.node);
        let opening = if lines.is_empty() {
            "nothing".to_owned()
        } else {
            quote_lines::signal_at(&lines, 0)
                .map_or_else(|| "text".to_owned(), |signal| format!("{signal:?}"))
        };
        let element = lines
            .first()
            .and_then(|line| framed.document.get_node(line.first))
            .and_then(|node| node.parent);
        let around = element.map_or_else(String::new, |element| place(&framed.document, element));
        audit.headless.push(format!("{opening}  {around}"));
    }
    for index in 0..framed.lines.len() {
        let Some(signal) = quote_lines::signal_at(&framed.lines, index) else {
            continue;
        };
        if framed.answered(index, signal) {
            audit.handled += 1;
        } else {
            audit.misses.push(Miss {
                signal,
                place: framed.place_of(index),
            });
        }
    }
    audit
}

/// Every signal of one mail on a line of its own, answered or not, with
/// what the detection made of the place: for looking into one miss.
pub fn explain(html: &str) -> Vec<String> {
    let framed = framed(html);
    (0..framed.lines.len())
        .filter_map(|index| {
            let signal = quote_lines::signal_at(&framed.lines, index)?;
            Some(format!(
                "line {index}: {signal:?}, answered {}, in a quote {}, in a read header {}, at {}",
                framed.answered(index, signal),
                framed.frames[index].is_some(),
                framed.headers[index],
                framed.place_of(index),
            ))
        })
        .collect()
}

/// The four elements around a line, outermost first.
fn place(document: &BaseDocument, element: NodeId) -> String {
    let mut chain = Vec::new();
    let mut current = Some(element);
    while let Some(node) = current.and_then(|id| document.get_node(id)) {
        if chain.len() == 4 {
            break;
        }
        if let Some(data) = node.element_data() {
            let mut name = data.name.local.to_string();
            if let Some(id) = node.attr(local_name!("id")) {
                name.push('#');
                // Ids often end in a counter or a message number.
                name.extend(id.chars().filter(|c| !c.is_ascii_digit()).take(40));
            }
            if let Some(class) = node.attr(local_name!("class")) {
                for class in class.split_whitespace().take(3) {
                    name.push('.');
                    name.extend(class.chars().take(30));
                }
            }
            if node
                .attr(local_name!("style"))
                .is_some_and(|style| style.contains("border"))
            {
                name.push_str("[border]");
            }
            chain.push(name);
        }
        current = node.parent;
    }
    chain.reverse();
    chain.join(" > ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_the_detection_frames_is_not_a_miss() {
        let found = audit(
            r#"<div>Thanks.</div>
            <div class="gmail_quote"><div class="gmail_attr">On Tue, 13 Oct 2025 at 10:02, Bob Example &lt;bob@example.com&gt; wrote:</div>
            <blockquote class="gmail_quote">See you Thursday.</blockquote></div>"#,
        );
        assert_eq!(found.cards, 1);
        assert_eq!(found.headers, 1);
        assert_eq!(found.handled, 1);
        assert_eq!(found.misses, Vec::new());
    }

    #[test]
    fn a_forward_sent_without_a_word_is_the_message_not_a_miss() {
        let found = audit(
            "<div>---------- Forwarded message ---------<br>From: Bob Example &lt;bob@example.com&gt;<br>Date: Monday<br>Subject: Plan</div><div>The plan.</div>",
        );
        assert_eq!(found.misses, Vec::new());
    }

    #[test]
    fn a_sentence_with_a_colon_is_no_signal() {
        let found = audit("<p>Hello,</p><p>Note: call me back.</p><p>From: here to there</p>");
        assert_eq!(found.handled, 0);
        assert_eq!(found.misses, Vec::new());
    }
}
