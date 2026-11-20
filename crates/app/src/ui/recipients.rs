//! Who a message was sent to, on one line however many they are, and the
//! dialog that lists them all.
//!
//! A message to a mailing list of thousands would otherwise open with
//! screens of addresses before its first word. The card names the first
//! recipients and counts the rest; a click on the line opens the full list,
//! To then Cc, with a field to find someone in it.

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length};
use rust_i18n::t;

use crate::ui::dialog::{DialogAction, form_dialog};
use crate::ui::layout::{
    DROPDOWN_ITEM_HEIGHT, PAD_INPUT, SCROLLBAR_SPACING, SPACE_SM, SPACE_XS, SPACE_XXS, TEXT_MD,
    TEXT_SM, TYPEAHEAD_MAX_HEIGHT,
};
use crate::ui::theme;

pub const SEARCH_INPUT_ID: &str = "recipients-dialog-search";

/// Characters of names a card's line spends before it counts the rest.
const LINE_BUDGET: usize = 60;
/// Rows the dialog draws at most: past it, the search narrows the list.
const MAX_ROWS: usize = 300;

/// One recipient, as an address header names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    /// The address, when the header gave a name as well.
    pub address: Option<String>,
}

/// The entries of an address header: "Ann <a@b>, c@d", or the
/// "Example, Ann; Other, Bob" Outlook writes in the header of a quote.
fn entries(raw: &str) -> impl Iterator<Item = &str> {
    // A header that uses semicolons keeps its commas for "Last, First".
    let by_semicolon = raw.contains(';');
    let mut quoted = false;
    let mut in_address = false;
    raw.split(move |c: char| match c {
        '"' => {
            quoted = !quoted;
            false
        }
        '<' if !quoted => {
            in_address = true;
            false
        }
        '>' if !quoted => {
            in_address = false;
            false
        }
        ';' => !quoted && !in_address,
        ',' => !by_semicolon && !quoted && !in_address,
        _ => false,
    })
    .map(str::trim)
    .filter(|entry| !entry.is_empty())
}

fn person(entry: &str) -> Person {
    let Some((name, address)) = entry.split_once('<') else {
        return Person {
            name: entry.trim_matches('"').trim().to_owned(),
            address: None,
        };
    };
    let name = name.trim().trim_matches('"').trim();
    let address = address.trim_end_matches('>').trim();
    if name.is_empty() || name.eq_ignore_ascii_case(address) {
        Person {
            name: address.to_owned(),
            address: None,
        }
    } else {
        Person {
            name: name.to_owned(),
            address: Some(address.to_owned()),
        }
    }
}

/// Everyone an address header names.
pub fn people(raw: Option<&str>) -> Vec<Person> {
    raw.map(|raw| entries(raw).map(person).collect())
        .unwrap_or_default()
}

/// The recipients a card names, and how many more it does not.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    pub to: String,
    pub cc: String,
    pub hidden: usize,
}

impl Summary {
    pub fn is_empty(&self) -> bool {
        self.to.is_empty() && self.cc.is_empty() && self.hidden == 0
    }
}

/// The first recipients that fit a line, To before Cc, and the count of
/// the others. A name is never cut: at least the first one is shown.
pub fn summary(to: Option<&str>, cc: Option<&str>) -> Summary {
    let mut summary = Summary::default();
    let mut spent = 0;
    let mut full = false;
    for (raw, shown) in [(to, &mut summary.to), (cc, &mut summary.cc)] {
        for entry in raw.into_iter().flat_map(entries) {
            if full {
                summary.hidden += 1;
                continue;
            }
            let name = person(entry).name;
            let cost = name.chars().count() + 2;
            if spent > 0 && spent + cost > LINE_BUDGET {
                full = true;
                summary.hidden += 1;
                continue;
            }
            spent += cost;
            if !shown.is_empty() {
                shown.push_str(", ");
            }
            shown.push_str(&name);
        }
    }
    summary
}

fn more_label(hidden: usize) -> String {
    if hidden == 1 {
        t!("main.recipients.more_one")
    } else {
        t!("main.recipients.more_other", count = hidden)
    }
    .into_owned()
}

/// The recipients of a message on one line. With `on_open` the line is a
/// button that opens the full list; nothing when there is no recipient.
pub fn line<'a, M: Clone + 'a>(
    to: Option<&str>,
    cc: Option<&str>,
    on_open: Option<M>,
) -> Option<Element<'a, M>> {
    let summary = summary(to, cc);
    if summary.is_empty() {
        return None;
    }
    let mut parts = row![].spacing(SPACE_XXS).align_y(Alignment::Center);
    if !summary.to.is_empty() {
        parts = parts.push(text(summary.to).size(TEXT_SM).style(text::base));
    }
    if !summary.cc.is_empty() {
        parts = parts.push(
            text(t!("main.recipients.cc"))
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        );
        parts = parts.push(text(summary.cc).size(TEXT_SM).style(text::base));
    }
    if summary.hidden > 0 {
        parts = parts.push(
            text(more_label(summary.hidden))
                .size(TEXT_SM)
                .style(theme::TextClass::Accent.style()),
        );
    }
    Some(match on_open {
        Some(on_open) => button(parts)
            .on_press(on_open)
            .padding(0)
            .style(theme::ButtonClass::BareTransparent.style())
            .into(),
        None => parts.into(),
    })
}

/// The dialog listing everyone a message was sent to.
#[derive(Debug, Clone)]
pub struct RecipientsDialog {
    to: Vec<Person>,
    cc: Vec<Person>,
    query: String,
}

#[derive(Debug, Clone)]
pub enum RecipientsMessage {
    Query(String),
    Close,
}

impl RecipientsDialog {
    pub fn new(to: Option<&str>, cc: Option<&str>) -> Self {
        Self {
            to: people(to),
            cc: people(cc),
            query: String::new(),
        }
    }

    pub fn set_query(&mut self, query: String) {
        self.query = query;
    }

    /// The people of `list` the search keeps.
    fn matching<'a>(&'a self, list: &'a [Person]) -> impl Iterator<Item = &'a Person> {
        let needle = self.query.trim().to_lowercase();
        list.iter().filter(move |person| {
            needle.is_empty()
                || person.name.to_lowercase().contains(&needle)
                || person
                    .address
                    .as_deref()
                    .is_some_and(|address| address.to_lowercase().contains(&needle))
        })
    }

    pub fn view(&self) -> Element<'_, RecipientsMessage> {
        let total = self.to.len() + self.cc.len();
        let search = text_input(t!("main.recipients.search"), &self.query)
            .id(SEARCH_INPUT_ID)
            .on_input(RecipientsMessage::Query)
            .size(TEXT_MD)
            .padding(PAD_INPUT);

        let mut list = column![].spacing(SPACE_XXS);
        let mut drawn = 0;
        let mut left_out = 0;
        for (label, people) in [
            (t!("main.reading_pane.to"), &self.to),
            (t!("main.recipients.cc"), &self.cc),
        ] {
            let mut labelled = false;
            for person in self.matching(people) {
                if drawn == MAX_ROWS {
                    left_out += 1;
                    continue;
                }
                if !labelled {
                    labelled = true;
                    list = list.push(
                        text(label.clone())
                            .size(TEXT_SM)
                            .style(theme::TextClass::Tertiary.style()),
                    );
                }
                drawn += 1;
                list = list.push(person_row(person));
            }
        }
        if drawn == 0 {
            list = list.push(
                text(t!("main.recipients.none"))
                    .size(TEXT_MD)
                    .style(text::secondary),
            );
        }
        if left_out > 0 {
            list = list.push(
                text(t!("main.recipients.truncated", count = left_out))
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            );
        }

        let body = column![
            search,
            container(
                scrollable(list)
                    .spacing(SCROLLBAR_SPACING)
                    .height(Length::Shrink)
            )
            .height(Length::Fit.max(TYPEAHEAD_MAX_HEIGHT))
            .width(Length::Fill),
        ]
        .spacing(SPACE_SM);

        form_dialog(
            t!("main.recipients.title", count = total),
            body,
            vec![DialogAction::default_action(
                t!("main.recipients.close"),
                RecipientsMessage::Close,
            )],
            None,
        )
    }
}

fn person_row(person: &Person) -> Element<'_, RecipientsMessage> {
    let address = person.address.as_deref().map(|address| {
        text(address)
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style())
    });
    container(
        row![
            text(person.name.as_str()).size(TEXT_MD).style(text::base),
            Space::new().width(Length::Fill),
            address,
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .height(DROPDOWN_ITEM_HEIGHT)
    .align_y(Alignment::Center)
    .width(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str, address: Option<&str>) -> Person {
        Person {
            name: name.to_owned(),
            address: address.map(str::to_owned),
        }
    }

    #[test]
    fn a_header_is_read_name_by_name() {
        assert_eq!(
            people(Some(
                "Ann Example <ann@example.com>, bob@example.com, \"Example, Carl\" <carl@example.com>"
            )),
            vec![
                named("Ann Example", Some("ann@example.com")),
                named("bob@example.com", None),
                named("Example, Carl", Some("carl@example.com")),
            ]
        );
        assert_eq!(people(Some("  ")), Vec::new());
        assert_eq!(people(None), Vec::new());
    }

    #[test]
    fn semicolons_keep_the_commas_of_last_name_first() {
        assert_eq!(
            people(Some("Example, Ann; Other, Bob <bob@example.com>")),
            vec![
                named("Example, Ann", None),
                named("Other, Bob", Some("bob@example.com")),
            ]
        );
    }

    #[test]
    fn a_name_that_is_its_address_is_shown_once() {
        assert_eq!(
            people(Some("ann@example.com <ann@example.com>, <bob@example.com>")),
            vec![
                named("ann@example.com", None),
                named("bob@example.com", None)
            ]
        );
    }

    #[test]
    fn a_short_list_is_shown_whole() {
        let shown = summary(
            Some("Ann Example <ann@example.com>, bob@example.com"),
            Some("Carl <carl@example.com>"),
        );
        assert_eq!(shown.to, "Ann Example, bob@example.com");
        assert_eq!(shown.cc, "Carl");
        assert_eq!(shown.hidden, 0);
    }

    #[test]
    fn a_long_list_names_the_first_and_counts_the_rest() {
        let to = (0..3000)
            .map(|index| format!("Person Number {index} <p{index}@example.com>"))
            .collect::<Vec<_>>()
            .join(", ");
        let shown = summary(Some(&to), Some("Carl <carl@example.com>"));
        assert_eq!(
            shown.to,
            "Person Number 0, Person Number 1, Person Number 2"
        );
        assert_eq!(shown.cc, "");
        // The 2 997 others and the one in copy.
        assert_eq!(shown.hidden, 2998);
    }

    #[test]
    fn the_first_name_is_shown_however_long() {
        let long = "A".repeat(200);
        let shown = summary(Some(&format!("{long}, bob@example.com")), None);
        assert_eq!(shown.to, long);
        assert_eq!(shown.hidden, 1);
    }

    #[test]
    fn no_recipient_is_no_line() {
        assert!(summary(None, Some(" ")).is_empty());
    }

    #[test]
    fn the_search_reads_names_and_addresses() {
        let mut dialog = RecipientsDialog::new(
            Some("Ann Example <ann@example.com>, Bob <bob@other.org>"),
            None,
        );
        dialog.set_query("OTHER".into());
        let found: Vec<_> = dialog.matching(&dialog.to).collect();
        assert_eq!(found, vec![&named("Bob", Some("bob@other.org"))]);
        dialog.set_query("ann".into());
        assert_eq!(dialog.matching(&dialog.to).count(), 1);
    }
}
