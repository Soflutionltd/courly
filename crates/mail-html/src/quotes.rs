//! Quoted earlier messages are told apart from the reply that carries them,
//! one under another instead of each inside the one that quoted it.
//!
//! A quote is found two ways. Some mail clients mark it: a `blockquote`
//! typed as a citation, the container of the new Outlook, the attribution
//! line of Gmail. Those are read by structure, never by a class alone:
//! Outlook re-serialises a Gmail message with `gmail_quote` on every
//! paragraph, and a frame per paragraph would shred the message. Every
//! other quote is found in the text, by the lines that say an earlier
//! message starts: see [`quote_lines`].

use std::collections::{HashMap, HashSet};

use blitz_dom::{BaseDocument, LocalName, NodeId, QualName, local_name, ns};

use crate::quote_header::{self, QuoteHeader};
use crate::quote_lines::{self, Signal};

/// Attribute set on every quoted message. Its value is [`CARD`] for the ones
/// the caller shows as separate cards, empty for the ones framed in place.
pub(crate) const FRAME_ATTRIBUTE: &str = "data-courly-quote";

/// Value of [`FRAME_ATTRIBUTE`] on a quote the page is cut at.
pub(crate) const CARD: &str = "card";

/// Attribute set on what spelled out the header of a quote shown as a card:
/// the caller shows that header its own way, so the stylesheet hides these.
pub(crate) const HEADER_ATTRIBUTE: &str = "data-courly-quote-header";

/// A quoted message shown as a card of its own.
pub(crate) struct QuoteCard {
    pub node: NodeId,
    /// Who wrote it and when, when the mail says so in a form that is
    /// understood.
    pub header: Option<QuoteHeader>,
}

/// Quotes that carry their own markup and need no attribution line to be
/// told apart. Apple Mail and Thunderbird keep the "On ... wrote:" line
/// outside the `blockquote`, so only the `blockquote` is framed. Outlook
/// mobile and the new Outlook wrap the header lines and the quoted message
/// in one container.
const SELF_MARKED: &str = r#"div.yahoo_quoted, blockquote[type=cite], div[id*="mail-editor-reference-message-container"]"#;

const SPLIT_QUOTES: &str = "blockquote[type=cite]";

/// The "On ... wrote:" line Gmail writes above a quoted message.
const ATTRIBUTIONS: &str = ".gmail_attr";

/// Mark every quoted message, then move each quote that closes the quote
/// around it to just after that quote: a reply quoting a reply quoting a
/// message becomes three blocks in a column.
///
/// Returns, in page order, the quotes that end the message: the caller cuts
/// the page above each of them and shows the pieces as separate cards. A
/// quote with text after it is framed in place instead: the sender answered
/// between the lines, and moving it would reorder what they wrote. The
/// stored message is untouched; this only changes the document being laid
/// out.
///
/// With `quote_only` the document is itself a quoted message, the one a
/// reply is being written to: the header it opens with is read, left off
/// the page and returned as the second value, for the caller to show above
/// it like the header of any other card.
pub(crate) fn frame_quotes(
    document: &mut BaseDocument,
    quote_only: bool,
) -> (Vec<QuoteCard>, Option<QuoteHeader>) {
    join_split_quotes(document);
    wrap_gmail_quotes(document);
    let marked = marked_quotes(document);
    let (wrapped, quoted_text) = wrap_text_quotes(document, &marked);
    let quotes = find_quotes(document, marked, wrapped, quoted_text);
    let lead = quotes
        .leading
        .filter(|_| quote_only)
        .and_then(|frame| quote_header::find(document, frame));
    let frames: HashSet<NodeId> = quotes
        .whole
        .iter()
        .chain(&quotes.text_only)
        .copied()
        .collect();
    let mut moves: Vec<(usize, NodeId, NodeId)> = quotes
        .whole
        .iter()
        .filter_map(|&quote| {
            let around = frame_around(document, quote, &frames)?;
            closes(document, quote, around).then(|| (depth(document, quote), quote, around))
        })
        .collect();
    // Outermost first: an inner quote is placed after one already in place.
    moves.sort_by_key(|&(depth, _, _)| depth);

    // The last frame placed after each frame, so two moves to after the
    // same one keep their order.
    let mut placed_after: HashMap<NodeId, NodeId> = HashMap::new();
    let mut mutator = document.mutate();
    for (_, quote, around) in moves {
        let anchor = placed_after.get(&around).copied().unwrap_or(around);
        mutator.insert_nodes_after(anchor, &[quote]);
        placed_after.insert(around, quote);
    }
    drop(mutator);

    let cards = closing_frames(document, &frames);
    let card_set: HashSet<NodeId> = cards.iter().copied().collect();
    let headers: Vec<Option<quote_header::Found>> = cards
        .iter()
        .map(|&card| {
            quote_header::find(document, card).or_else(|| quote_header::find_above(document, card))
        })
        .collect();

    // What spelled out a header is taken off the page. Loose text cannot
    // carry the attribute that hides it, so a header written as loose text
    // and line breaks is first gathered in a block.
    let mut header_nodes: Vec<NodeId> = Vec::new();
    for found in headers.iter().flatten().chain(&lead) {
        let is_element = |node: &NodeId| {
            document
                .get_node(*node)
                .is_some_and(blitz_dom::Node::is_element)
        };
        let loose_text = found
            .nodes
            .iter()
            .any(|node| !is_element(node) && shows_something(document, *node));
        if loose_text {
            header_nodes.extend(wrap(document, &found.nodes));
        } else {
            header_nodes.extend(found.nodes.iter().copied().filter(is_element));
        }
    }

    let attribute = QualName::new(None, ns!(), LocalName::from(FRAME_ATTRIBUTE));
    let header_attribute = QualName::new(None, ns!(), LocalName::from(HEADER_ATTRIBUTE));
    let mut mutator = document.mutate();
    for &frame in &frames {
        let value = if card_set.contains(&frame) { CARD } else { "" };
        mutator.set_attribute(frame, attribute.clone(), value);
    }
    for node in header_nodes {
        mutator.set_attribute(node, header_attribute.clone(), "");
    }
    let cards = cards
        .into_iter()
        .zip(headers)
        .map(|(node, found)| QuoteCard {
            node,
            header: found.map(|found| found.header),
        })
        .collect();
    (cards, lead.map(|found| found.header))
}

/// Apple Mail writes one quoted message as several `blockquote`s in a row.
/// Their content is gathered in the first, so the message gets one frame
/// instead of one per piece.
fn join_split_quotes(document: &mut BaseDocument) {
    let Ok(pieces) = document.query_selector_all(SPLIT_QUOTES) else {
        return;
    };
    let piece_set: HashSet<NodeId> = pieces.iter().copied().collect();
    // Each later piece, with the first piece of its run.
    let mut first_of: HashMap<NodeId, NodeId> = HashMap::new();
    let mut joins: Vec<(NodeId, Vec<NodeId>)> = Vec::new();
    for &piece in &pieces {
        let Some(next) = next_shown_sibling(document, piece) else {
            continue;
        };
        if !piece_set.contains(&next) {
            continue;
        }
        let first = first_of.get(&piece).copied().unwrap_or(piece);
        first_of.insert(next, first);
        joins.push((first, children_of(document, next)));
    }
    let mut mutator = document.mutate();
    for (first, content) in joins {
        mutator.append_children(first, &content);
    }
}

/// Put `nodes`, siblings in a row, inside a new `div` standing where the
/// first of them stood.
fn wrap(document: &mut BaseDocument, nodes: &[NodeId]) -> Option<NodeId> {
    let first = *nodes.first()?;
    let mut mutator = document.mutate();
    let wrapper = mutator.create_element(
        QualName::new(None, ns!(html), local_name!("div")),
        Vec::new(),
    );
    mutator.insert_nodes_before(first, &[wrapper]);
    mutator.append_children(wrapper, nodes);
    Some(wrapper)
}

/// Gmail's attribution line and the quoted text under it normally share a
/// block of their own. Where a mail client flattened that block into the
/// text around it, it is rebuilt, so the quote is a message that can be told
/// apart: attribution line and text together.
fn wrap_gmail_quotes(document: &mut BaseDocument) {
    let Ok(attributions) = document.query_selector_all(ATTRIBUTIONS) else {
        return;
    };
    let runs: Vec<Vec<NodeId>> = attributions
        .iter()
        .filter_map(|&attribution| {
            let parent = document.get_node(attribution)?.parent?;
            if opens(document, attribution, parent) {
                return None;
            }
            let quoted = next_shown_sibling(document, attribution)?;
            if !is_tag(document, quoted, &local_name!("blockquote")) {
                return None;
            }
            let siblings = children_of(document, parent);
            let from = siblings.iter().position(|&child| child == attribution)?;
            let to = siblings.iter().position(|&child| child == quoted)?;
            Some(siblings[from..=to].to_vec())
        })
        .collect();
    for run in runs {
        wrap(document, &run);
    }
}

/// Quotes the mail marks itself, before any is looked for in its text.
struct Marked {
    /// Blocks holding a quoted message with its header or attribution.
    whole: Vec<NodeId>,
    /// Blocks holding the quoted text alone.
    text_only: Vec<NodeId>,
}

fn marked_quotes(document: &BaseDocument) -> Marked {
    let mut marked = Marked {
        whole: Vec::new(),
        text_only: Vec::new(),
    };
    if let Ok(found) = document.query_selector_all(SELF_MARKED) {
        for id in found {
            if is_tag(document, id, &local_name!("blockquote")) {
                marked.text_only.push(id);
            } else {
                marked.whole.push(id);
            }
        }
    }
    if let Ok(attributions) = document.query_selector_all(ATTRIBUTIONS) {
        for attribution in attributions {
            let Some(parent) = document.get_node(attribution).and_then(|node| node.parent) else {
                continue;
            };
            if opens(document, attribution, parent) {
                marked.whole.push(parent);
            } else if let Some(quoted) = children_of(document, parent)
                .into_iter()
                .skip_while(|&child| child != attribution)
                .find(|&child| is_tag(document, child, &local_name!("blockquote")))
            {
                marked.text_only.push(quoted);
            }
        }
    }
    marked
}

/// Lines a separator may be from the header block it announces.
const SEPARATOR_TO_HEADER: usize = 3;

/// Find the quoted messages only the text tells apart, and gather each in
/// a block of its own.
///
/// A line that says an earlier message starts (see [`quote_lines`]) starts
/// a quote that runs to the end of what holds it: the header lines of
/// Outlook, whatever became of their markup, the rule of a plain-text
/// forward, an attribution line with nothing but loose paragraphs under it.
/// A line that already opens a quote the mail marks adds nothing. An
/// attribution line followed by a `blockquote` leaves the line where it is
/// and takes the `blockquote` as the quoted text.
///
/// Returns the blocks made, each a whole message, and the `blockquote`s
/// found.
fn wrap_text_quotes(document: &mut BaseDocument, marked: &Marked) -> (Vec<NodeId>, Vec<NodeId>) {
    let lines = quote_lines::collect(document, document.root_node().id);
    let mut known: HashSet<NodeId> = marked
        .whole
        .iter()
        .chain(&marked.text_only)
        .copied()
        .collect();
    // Each start with the lines the signal itself takes: a quote holds more.
    let mut starts: Vec<(NodeId, usize)> = Vec::new();
    let mut quoted_text = Vec::new();
    let mut announced: HashSet<usize> = HashSet::new();
    for index in 0..lines.len() {
        let Some(signal) = quote_lines::signal_at(&lines, index) else {
            continue;
        };
        let first = lines[index].first;
        match signal {
            Signal::Separator => {
                let header = (index + 1..lines.len())
                    .take(SEPARATOR_TO_HEADER)
                    .find(|&after| {
                        quote_lines::signal_at(&lines, after) == Some(Signal::HeaderBlock)
                    });
                announced.extend(header);
                starts.push((first, 1));
            }
            Signal::HeaderBlock if announced.contains(&index) => {}
            Signal::HeaderBlock => {
                let labelled = lines[index..]
                    .iter()
                    .take_while(|line| quote_lines::label(&line.text).is_some())
                    .count();
                starts.push((first, labelled));
            }
            Signal::Attribution if has_class_above(document, first, "gmail_attr") => {}
            Signal::Attribution => match quoted_after(document, first) {
                Some(quoted) => {
                    known.insert(quoted);
                    quoted_text.push(quoted);
                }
                None => starts.push((first, 1)),
            },
        }
    }

    let mut wrapped = Vec::new();
    // Innermost first, so an outer run gathers the blocks made inside it.
    for &(first, own_lines) in starts.iter().rev() {
        if opens_one_of(document, first, &known) {
            continue;
        }
        let Some(run) = text_quote_run(document, first) else {
            continue;
        };
        let run_lines: usize = run
            .iter()
            .map(|&node| quote_lines::collect(document, node).len())
            .sum();
        if run_lines <= own_lines {
            continue;
        }
        if let Some(block) = wrap(document, &run) {
            known.insert(block);
            wrapped.push(block);
        }
    }
    (wrapped, quoted_text)
}

/// Whether `node` or an element around it carries `class`.
fn has_class_above(document: &BaseDocument, node: NodeId, class: &str) -> bool {
    let mut current = Some(node);
    while let Some(node) = current.and_then(|id| document.get_node(id)) {
        if node
            .attr(local_name!("class"))
            .is_some_and(|names| names.split_whitespace().any(|name| name == class))
        {
            return true;
        }
        current = node.parent;
    }
    false
}

/// The `blockquote` that follows the line starting at `first`: the next
/// thing shown after the block the line is in, or after the blocks that
/// block ends.
fn quoted_after(document: &BaseDocument, first: NodeId) -> Option<NodeId> {
    let mut current = container_around(document, first)?;
    for _ in 0..3 {
        if let Some(next) = next_shown_sibling(document, current) {
            return is_tag(document, next, &local_name!("blockquote")).then_some(next);
        }
        current = document.get_node(current)?.parent?;
        if is_tag(document, current, &local_name!("body")) {
            break;
        }
    }
    None
}

/// Whether the line starting at `first` is the first thing shown by one of
/// `quotes`: that quote already starts where the line says one does.
fn opens_one_of(document: &BaseDocument, first: NodeId, quotes: &HashSet<NodeId>) -> bool {
    let mut current = first;
    while let Some(parent) = document.get_node(current).and_then(|node| node.parent) {
        let preceded = children_of(document, parent)
            .into_iter()
            .take_while(|&child| child != current)
            .any(|sibling| shows_something(document, sibling));
        if preceded {
            return false;
        }
        if quotes.contains(&parent) {
            return true;
        }
        current = parent;
    }
    false
}

/// The nearest element around `node` whose children are laid out one under
/// another, so that a run of them can be gathered in a block.
fn container_around(document: &BaseDocument, node: NodeId) -> Option<NodeId> {
    let containers = [
        local_name!("div"),
        local_name!("p"),
        local_name!("center"),
        local_name!("td"),
        local_name!("th"),
        local_name!("li"),
        local_name!("blockquote"),
        local_name!("pre"),
        local_name!("body"),
    ];
    let mut current = document.get_node(node)?.parent?;
    loop {
        if containers.iter().any(|tag| is_tag(document, current, tag)) {
            return Some(current);
        }
        current = document.get_node(current)?.parent?;
    }
}

/// The nodes of the quote that starts with the line at `first`: from that
/// line to the end of what holds it.
///
/// When the line opens a block that can be moved, the quote starts with
/// that block, followed up through the wrappers that hold nothing else, so
/// that the quoted message, a sibling of the outermost one, is taken with
/// it. Otherwise the line starts in the middle of its block, after a line
/// break, and the quote is the rest of that block.
fn text_quote_run(document: &BaseDocument, first: NodeId) -> Option<Vec<NodeId>> {
    let container = container_around(document, first)?;
    let movable = [local_name!("div"), local_name!("p"), local_name!("center")]
        .iter()
        .any(|tag| is_tag(document, container, tag));
    let opens_container = {
        let mut current = first;
        let mut preceded = false;
        while current != container {
            let parent = document.get_node(current)?.parent?;
            preceded |= children_of(document, parent)
                .into_iter()
                .take_while(|&child| child != current)
                .any(|sibling| shows_something(document, sibling));
            current = parent;
        }
        !preceded
    };

    if !opens_container && let Some(run) = inline_run(document, first, container) {
        return Some(run);
    }

    let mut start = if movable && opens_container {
        container
    } else {
        let mut current = first;
        while document.get_node(current)?.parent? != container {
            current = document.get_node(current)?.parent?;
        }
        current
    };
    if start == container {
        loop {
            let parent = document.get_node(start)?.parent?;
            let alone = children_of(document, parent)
                .into_iter()
                .all(|child| child == start || !shows_something(document, child));
            if !alone || !is_tag(document, parent, &local_name!("div")) {
                break;
            }
            start = parent;
        }
    }
    let parent = document.get_node(start)?.parent?;
    let siblings = children_of(document, parent);
    let mut from = siblings.iter().position(|&child| child == start)?;
    // The rule Outlook draws above the header block belongs to the quote.
    if let Some(before) = siblings[..from].iter().rposition(|&child| {
        document
            .get_node(child)
            .is_some_and(blitz_dom::Node::is_element)
    }) && is_tag(document, siblings[before], &local_name!("hr"))
    {
        from = before;
    }
    Some(siblings[from..].to_vec())
}

/// The quote that starts with the line at `first` when that line starts
/// inside an inline element of `container`, after a line break: a message
/// written as text, its lines parted by `br`, all in one `span`.
///
/// The quote is then the rest of that inline element, not the element: what
/// it shows before the line is an earlier part of the mail. That holds only
/// when the element is the last thing its block shows, so that the rest of
/// it is the rest of the block. `None` when the line starts at a child of
/// `container` itself, or when something follows the inline element.
fn inline_run(document: &BaseDocument, first: NodeId, container: NodeId) -> Option<Vec<NodeId>> {
    // The level at which something shown comes before the line.
    let mut split = first;
    loop {
        let parent = document.get_node(split)?.parent?;
        let preceded = children_of(document, parent)
            .into_iter()
            .take_while(|&child| child != split)
            .any(|sibling| shows_something(document, sibling));
        if preceded {
            break;
        }
        if parent == container {
            return None;
        }
        split = parent;
    }
    let holder = document.get_node(split)?.parent?;
    if holder == container {
        return None;
    }
    let mut current = holder;
    while current != container {
        if next_shown_sibling(document, current).is_some() {
            return None;
        }
        current = document.get_node(current)?.parent?;
    }
    let siblings = children_of(document, holder);
    let from = siblings.iter().position(|&child| child == split)?;
    Some(siblings[from..].to_vec())
}

fn children_of(document: &BaseDocument, id: NodeId) -> Vec<NodeId> {
    document
        .get_node(id)
        .map(|node| node.children.to_vec())
        .unwrap_or_default()
}

/// The next sibling of `id` that puts something on the page.
fn next_shown_sibling(document: &BaseDocument, id: NodeId) -> Option<NodeId> {
    let parent = document.get_node(id)?.parent?;
    children_of(document, parent)
        .into_iter()
        .skip_while(|&child| child != id)
        .skip(1)
        .find(|&sibling| shows_something(document, sibling))
}

/// Whether `child` is the first thing its parent, a `div`, shows.
fn opens(document: &BaseDocument, child: NodeId, parent: NodeId) -> bool {
    is_tag(document, parent, &local_name!("div"))
        && !children_of(document, parent)
            .into_iter()
            .take_while(|&sibling| sibling != child)
            .any(|sibling| shows_something(document, sibling))
}

struct Quotes {
    /// Blocks holding a quoted message together with its header or
    /// attribution line: moving one moves a whole message.
    whole: Vec<NodeId>,
    /// Blocks holding the quoted text alone; the attribution line sits
    /// outside, so they stay where they are.
    text_only: Vec<NodeId>,
    /// A whole quote the page shows nothing before: it is not a quote of
    /// the message but the message, so it is in neither list above.
    leading: Option<NodeId>,
}

fn find_quotes(
    document: &BaseDocument,
    marked: Marked,
    wrapped: Vec<NodeId>,
    quoted_text: Vec<NodeId>,
) -> Quotes {
    let mut whole = wrapped;
    whole.extend(marked.whole);
    let mut text_only = quoted_text;
    text_only.extend(marked.text_only);
    let leading = whole
        .iter()
        .copied()
        .find(|&id| shows_something(document, id) && !follows_something(document, id));
    let mut seen = HashSet::new();
    // An empty quote would be an empty block, and a message that is all
    // quote (a forward sent without a word) is the message itself.
    let mut keep = |id: &NodeId| {
        seen.insert(*id) && shows_something(document, *id) && follows_something(document, *id)
    };
    whole.retain(&mut keep);
    text_only.retain(&mut keep);
    Quotes {
        whole,
        text_only,
        leading,
    }
}

fn is_tag(document: &BaseDocument, id: NodeId, tag: &LocalName) -> bool {
    document
        .get_node(id)
        .is_some_and(|node| node.data.is_element_with_tag_name(tag))
}

fn depth(document: &BaseDocument, id: NodeId) -> usize {
    let mut depth = 0;
    let mut current = id;
    while let Some(parent) = document.get_node(current).and_then(|node| node.parent) {
        depth += 1;
        current = parent;
    }
    depth
}

/// The nearest frame that contains `node`.
fn frame_around(document: &BaseDocument, node: NodeId, frames: &HashSet<NodeId>) -> Option<NodeId> {
    let mut current = document.get_node(node)?.parent?;
    loop {
        if frames.contains(&current) {
            return Some(current);
        }
        current = document.get_node(current)?.parent?;
    }
}

/// Whether nothing the reader can see follows `quote` inside `around`.
fn closes(document: &BaseDocument, quote: NodeId, around: NodeId) -> bool {
    let mut current = quote;
    loop {
        let Some(parent) = document.get_node(current).and_then(|node| node.parent) else {
            return false;
        };
        let followed = children_of(document, parent)
            .into_iter()
            .skip_while(|&child| child != current)
            .skip(1)
            .any(|sibling| shows_something(document, sibling));
        if followed {
            return false;
        }
        if parent == around {
            return true;
        }
        current = parent;
    }
}

/// Whether the page shows something before `id`.
fn follows_something(document: &BaseDocument, id: NodeId) -> bool {
    let mut current = id;
    while let Some(parent) = document.get_node(current).and_then(|node| node.parent) {
        let preceded = children_of(document, parent)
            .into_iter()
            .take_while(|&child| child != current)
            .any(|sibling| shows_something(document, sibling));
        if preceded {
            return true;
        }
        current = parent;
    }
    false
}

/// The frames that end the page, in page order: each is inside no other
/// frame, and everything the page shows after it is another of them.
fn closing_frames(document: &BaseDocument, frames: &HashSet<NodeId>) -> Vec<NodeId> {
    let mut outermost = Vec::new();
    let mut pending = vec![document.root_node().id];
    while let Some(id) = pending.pop() {
        if frames.contains(&id) {
            outermost.push(id);
        } else {
            pending.extend(children_of(document, id).into_iter().rev());
        }
    }
    let mut closing: HashSet<NodeId> = HashSet::new();
    let mut cards = Vec::new();
    for &frame in outermost.iter().rev() {
        if !ends_the_page(document, frame, &closing) {
            break;
        }
        closing.insert(frame);
        cards.push(frame);
    }
    cards.reverse();
    cards
}

/// Whether everything the page shows after `frame` is in `closing`.
fn ends_the_page(document: &BaseDocument, frame: NodeId, closing: &HashSet<NodeId>) -> bool {
    let mut current = frame;
    while let Some(parent) = document.get_node(current).and_then(|node| node.parent) {
        let followed = children_of(document, parent)
            .into_iter()
            .skip_while(|&child| child != current)
            .skip(1)
            .any(|sibling| shows_outside(document, sibling, closing));
        if followed {
            return false;
        }
        current = parent;
    }
    true
}

/// Whether a node puts text or an image on the page.
fn shows_something(document: &BaseDocument, id: NodeId) -> bool {
    shows_outside(document, id, &HashSet::new())
}

/// Whether a node puts text or an image on the page outside the `skipped`
/// blocks.
fn shows_outside(document: &BaseDocument, id: NodeId, skipped: &HashSet<NodeId>) -> bool {
    if skipped.contains(&id) {
        return false;
    }
    let Some(node) = document.get_node(id) else {
        return false;
    };
    if node.is_text_node() {
        return !node.text_content().trim().is_empty();
    }
    let unseen = [
        local_name!("head"),
        local_name!("style"),
        local_name!("script"),
        local_name!("title"),
    ];
    if unseen
        .iter()
        .any(|tag| node.data.is_element_with_tag_name(tag))
    {
        return false;
    }
    node.data.is_element_with_tag_name(&local_name!("img"))
        || node
            .children
            .iter()
            .any(|&child| shows_outside(document, child, skipped))
}
