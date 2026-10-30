use iced::Element;
use iced::widget::{rich_text, span, text};
use linkify::{LinkFinder, LinkKind};

/// The URL a detected link opens. Bare addresses become `mailto:` links.
fn link_target(kind: &LinkKind, matched: &str) -> Option<String> {
    match kind {
        LinkKind::Url => Some(matched.to_string()),
        LinkKind::Email => Some(format!("mailto:{matched}")),
        _ => None,
    }
}

/// A plain-text email body with its URLs and email addresses clickable.
pub fn linked_text<'a, M: Clone + 'a>(
    body: &'a str,
    size: f32,
    on_link: impl Fn(String) -> M + 'a,
) -> Element<'a, M> {
    let spans: Vec<text::Span<'a, String>> = LinkFinder::new()
        .spans(body)
        .map(|part| {
            let fragment = span(part.as_str());
            match part
                .kind()
                .and_then(|kind| link_target(kind, part.as_str()))
            {
                Some(target) => fragment.link(target).underline(true),
                None => fragment,
            }
        })
        .collect();
    rich_text(spans).size(size).on_link_click(on_link).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(body: &str) -> Vec<String> {
        LinkFinder::new()
            .spans(body)
            .filter_map(|part| {
                part.kind()
                    .and_then(|kind| link_target(kind, part.as_str()))
            })
            .collect()
    }

    #[test]
    fn urls_and_addresses_become_links() {
        assert_eq!(
            targets("See https://example.com/a?b=1, or write to jo@example.com."),
            vec!["https://example.com/a?b=1", "mailto:jo@example.com"]
        );
    }

    #[test]
    fn text_without_scheme_stays_plain() {
        assert!(targets("example.com and javascript:alert(1)").is_empty());
    }
}
