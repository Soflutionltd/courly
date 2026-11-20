//! Reply and forward marks in a subject line.
//!
//! Every mail client prepends its own, in its own language, to the ones
//! already there: a long exchange ends up titled "RE: TR: Re: Re: Invoice".
//! The reader is shown the subject itself with one arrow saying what the
//! message is; a reply or a forward written here carries a single mark.

/// What the marks in front of a subject say the message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectKind {
    Plain,
    Reply,
    Forward,
}

/// Marks that mean "reply", lowercase, in the languages mail clients write.
const REPLY_MARKS: [&str; 5] = ["re", "aw", "sv", "antw", "rif"];
/// Marks that mean "forward".
const FORWARD_MARKS: [&str; 6] = ["fwd", "fw", "tr", "wg", "vs", "doorst"];

/// A subject without the reply and forward marks in front of it, and what
/// the first of them, the one the last sender added, says the message is.
pub fn split(subject: &str) -> (SubjectKind, &str) {
    let mut kind = SubjectKind::Plain;
    let mut rest = subject.trim_start();
    while let Some((mark, after)) = leading_mark(rest) {
        if kind == SubjectKind::Plain {
            kind = mark;
        }
        rest = after.trim_start();
    }
    (kind, rest)
}

/// The subject of a reply to, or a forward of, a message titled `original`:
/// that subject with one mark, whatever it carried.
pub fn marked(kind: SubjectKind, original: &str) -> String {
    let (_, bare) = split(original);
    match kind {
        SubjectKind::Plain => bare.to_owned(),
        SubjectKind::Reply => format!("Re: {bare}"),
        SubjectKind::Forward => format!("Fwd: {bare}"),
    }
}

/// One mark at the start of `text`: "Re:", "RE :", "Fwd:", "Re[2]:".
fn leading_mark(text: &str) -> Option<(SubjectKind, &str)> {
    let colon = text.find(':')?;
    let word = text[..colon].trim_end();
    // "Re[2]" and "Re(2)", as some clients count the replies.
    let word = match word.find(['[', '(']) {
        Some(open)
            if word.ends_with([']', ')'])
                && word[open + 1..word.len() - 1]
                    .chars()
                    .all(|c| c.is_ascii_digit()) =>
        {
            &word[..open]
        }
        _ => word,
    };
    let word = word.to_lowercase();
    let kind = if REPLY_MARKS.contains(&word.as_str()) {
        SubjectKind::Reply
    } else if FORWARD_MARKS.contains(&word.as_str()) {
        SubjectKind::Forward
    } else {
        return None;
    };
    Some((kind, &text[colon + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pile_of_marks_leaves_the_subject_and_the_outermost_kind() {
        assert_eq!(
            split("RE: TR: Re: Re: Invoice 42"),
            (SubjectKind::Reply, "Invoice 42")
        );
        assert_eq!(
            split("TR : RE : Invoice 42"),
            (SubjectKind::Forward, "Invoice 42")
        );
        assert_eq!(
            split("Fw: AW: Re[2]: Invoice"),
            (SubjectKind::Forward, "Invoice")
        );
    }

    #[test]
    fn a_subject_that_only_looks_marked_is_left_whole() {
        assert_eq!(split("Invoice 42"), (SubjectKind::Plain, "Invoice 42"));
        assert_eq!(
            split("Review: the plan"),
            (SubjectKind::Plain, "Review: the plan")
        );
        assert_eq!(split("Rent due"), (SubjectKind::Plain, "Rent due"));
        assert_eq!(split("Re[x]: odd"), (SubjectKind::Plain, "Re[x]: odd"));
        assert_eq!(split(""), (SubjectKind::Plain, ""));
    }

    #[test]
    fn a_reply_or_a_forward_carries_one_mark() {
        assert_eq!(marked(SubjectKind::Reply, "Invoice"), "Re: Invoice");
        assert_eq!(
            marked(SubjectKind::Reply, "RE: TR: Re: Invoice"),
            "Re: Invoice"
        );
        assert_eq!(marked(SubjectKind::Forward, "Re: Invoice"), "Fwd: Invoice");
        assert_eq!(
            marked(SubjectKind::Forward, "Fwd: Fwd: Invoice"),
            "Fwd: Invoice"
        );
    }
}
