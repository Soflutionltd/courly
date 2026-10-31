//! Suggestions under the title-bar search field, the way Outlook shows
//! them while typing: people, then the best matching messages, then a row
//! that fills the list with every result. Search operators (`from:`,
//! `in:`…) list their values instead.

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};
use rust_i18n::t;

use crate::db::Thread;
use crate::icon;
use crate::message::Message;
use crate::ui::layout::*;
use crate::ui::thread_list::{TypeaheadDirection, TypeaheadItem, TypeaheadState};
use crate::ui::{theme, widgets};

/// People shown above the messages.
pub const MAX_CONTACTS: usize = 4;
/// Messages shown under the people.
pub const MAX_MESSAGES: usize = 5;

#[derive(Debug, Clone)]
pub enum SearchSuggestMessage {
    /// The typing pause: look up people and messages for the query.
    Refresh,
    Loaded {
        query: String,
        contacts: Vec<TypeaheadItem>,
        messages: Vec<Thread>,
    },
    Navigate(TypeaheadDirection),
    /// Enter in the search field: the highlighted row, or every result.
    Activate,
    PickContact(usize),
    PickMessage(usize),
    ShowAll,
    Close,
    /// The cross on the sender pill.
    ClearSender,
}

/// One line of the menu, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestRow {
    Contact(usize),
    Message(usize),
    ShowAll,
}

#[derive(Debug, Default)]
pub struct SearchSuggest {
    pub open: bool,
    /// The query the people and messages were found for.
    query: String,
    contacts: Vec<TypeaheadItem>,
    messages: Vec<Thread>,
    /// Highlighted row; none until an arrow key moves into the menu, so
    /// Enter alone searches for what was typed.
    selected: Option<usize>,
}

impl SearchSuggest {
    /// Show what was found, unless the query changed since it was asked.
    pub fn load(
        &mut self,
        query: String,
        current: &str,
        contacts: Vec<TypeaheadItem>,
        messages: Vec<Thread>,
    ) {
        if query != current.trim() {
            return;
        }
        let mut seen = std::collections::HashSet::new();
        self.contacts = contacts
            .into_iter()
            .filter(|contact| seen.insert(contact.insert_value.to_lowercase()))
            .take(MAX_CONTACTS)
            .collect();
        self.messages = messages.into_iter().take(MAX_MESSAGES).collect();
        self.query = query;
        self.selected = None;
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.selected = None;
    }

    pub fn rows(&self) -> Vec<SuggestRow> {
        (0..self.contacts.len())
            .map(SuggestRow::Contact)
            .chain((0..self.messages.len()).map(SuggestRow::Message))
            .chain(std::iter::once(SuggestRow::ShowAll))
            .collect()
    }

    pub fn navigate(&mut self, direction: TypeaheadDirection) {
        let last = self.rows().len().saturating_sub(1);
        self.selected = match (direction, self.selected) {
            (TypeaheadDirection::Down, None) => Some(0),
            (TypeaheadDirection::Down, Some(i)) => Some((i + 1).min(last)),
            (TypeaheadDirection::Up, Some(0) | None) => None,
            (TypeaheadDirection::Up, Some(i)) => Some(i - 1),
        };
    }

    pub fn selected_row(&self) -> Option<SuggestRow> {
        self.rows().get(self.selected?).copied()
    }

    pub fn contact(&self, index: usize) -> Option<&TypeaheadItem> {
        self.contacts.get(index)
    }

    pub fn message(&self, index: usize) -> Option<&Thread> {
        self.messages.get(index)
    }
}

/// The menu, or `None` when nothing should show.
pub fn view<'a>(
    suggest: &'a SearchSuggest,
    operators: &'a TypeaheadState,
) -> Option<Element<'a, Message>> {
    if operators.visible && !operators.items.is_empty() {
        return Some(operator_menu(operators));
    }
    if !suggest.open {
        return None;
    }

    let selected = suggest.selected_row();
    let mut list = column![].spacing(SPACE_XXXS);
    if !suggest.contacts.is_empty() {
        list = list.push(section_title(t!("main.search.suggest.people")));
        for (i, contact) in suggest.contacts.iter().enumerate() {
            list = list.push(contact_row(
                contact,
                i,
                selected == Some(SuggestRow::Contact(i)),
            ));
        }
    }
    if !suggest.messages.is_empty() {
        if !suggest.contacts.is_empty() {
            list = list.push(widgets::divider());
        }
        list = list.push(section_title(t!("main.search.suggest.messages")));
        for (i, thread) in suggest.messages.iter().enumerate() {
            list = list.push(message_row(
                thread,
                i,
                selected == Some(SuggestRow::Message(i)),
            ));
        }
    }
    if !suggest.contacts.is_empty() || !suggest.messages.is_empty() {
        list = list.push(widgets::divider());
    }
    list = list.push(show_all_row(
        &suggest.query,
        selected == Some(SuggestRow::ShowAll),
    ));

    Some(
        container(list)
            .padding(PAD_DROPDOWN)
            .width(Length::Fill)
            .style(theme::ContainerClass::SelectMenu.style())
            .into(),
    )
}

fn section_title<'a>(label: impl Into<String>) -> Element<'a, Message> {
    container(
        text(label.into())
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    )
    .padding(ROW_MENU_ITEM_PADDING)
    .into()
}

fn suggestion_button<'a>(
    content: impl Into<Element<'a, Message>>,
    message: SearchSuggestMessage,
    selected: bool,
) -> Element<'a, Message> {
    button(content)
        .on_press(Message::SearchSuggest(message))
        .padding(ROW_MENU_ITEM_PADDING)
        .width(Length::Fill)
        .style(theme::ButtonClass::Dropdown { selected }.style())
        .into()
}

fn contact_row(contact: &TypeaheadItem, index: usize, selected: bool) -> Element<'_, Message> {
    let mut line = row![
        widgets::avatar_circle(&contact.label, AVATAR_DROPDOWN_TRIGGER),
        text(&contact.label)
            .size(TEXT_MD)
            .style(text::base)
            .wrapping(text::Wrapping::None)
            .ellipsis(text::Ellipsis::End),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);
    if contact.label != contact.insert_value {
        line = line.push(
            text(&contact.insert_value)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
                .wrapping(text::Wrapping::None)
                .ellipsis(text::Ellipsis::End),
        );
    }
    suggestion_button(line, SearchSuggestMessage::PickContact(index), selected)
}

fn message_row(thread: &Thread, index: usize, selected: bool) -> Element<'_, Message> {
    let subject = thread
        .subject
        .as_deref()
        .filter(|subject| !subject.trim().is_empty())
        .map_or_else(|| t!("main.no_subject"), std::borrow::Cow::Borrowed);
    let sender = thread
        .from_name
        .as_deref()
        .or(thread.from_address.as_deref())
        .map_or_else(|| t!("main.unknown_sender"), std::borrow::Cow::Borrowed);
    let date = thread
        .last_message_at
        .and_then(widgets::local_dt)
        .map(|dt| widgets::short_date(&dt));
    let detail = match date {
        Some(date) => format!("{sender} · {date}"),
        None => sender.into_owned(),
    };
    let line = row![
        container(icon::mail().size(ICON_MD).style(text::secondary))
            .width(AVATAR_DROPDOWN_TRIGGER)
            .align_x(Alignment::Center),
        column![
            text(subject)
                .size(TEXT_MD)
                .style(text::base)
                .wrapping(text::Wrapping::None)
                .ellipsis(text::Ellipsis::End),
            text(detail)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
                .wrapping(text::Wrapping::None)
                .ellipsis(text::Ellipsis::End),
        ]
        .width(Length::Fill),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);
    suggestion_button(line, SearchSuggestMessage::PickMessage(index), selected)
}

fn show_all_row(query: &str, selected: bool) -> Element<'_, Message> {
    let line = row![
        container(icon::search().size(ICON_MD).style(text::secondary))
            .width(AVATAR_DROPDOWN_TRIGGER)
            .align_x(Alignment::Center),
        text(t!("main.search.suggest.show_all", query = query))
            .size(TEXT_MD)
            .style(text::base)
            .wrapping(text::Wrapping::None)
            .ellipsis(text::Ellipsis::End),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);
    suggestion_button(line, SearchSuggestMessage::ShowAll, selected)
}

fn operator_menu(operators: &TypeaheadState) -> Element<'_, Message> {
    let mut list = column![].spacing(SPACE_XXXS);
    for (i, item) in operators.items.iter().enumerate() {
        let mut line = row![
            text(&item.label)
                .size(TEXT_MD)
                .style(text::base)
                .wrapping(text::Wrapping::None)
                .ellipsis(text::Ellipsis::End),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center);
        if let Some(detail) = &item.detail {
            line = line.push(Space::new().width(Length::Fill)).push(
                text(detail)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style())
                    .wrapping(text::Wrapping::None)
                    .ellipsis(text::Ellipsis::End),
            );
        }
        list = list.push(
            button(line)
                .on_press(Message::ThreadList(
                    crate::ui::thread_list::ThreadListMessage::TypeaheadSelect(i),
                ))
                .padding(ROW_MENU_ITEM_PADDING)
                .width(Length::Fill)
                .style(
                    theme::ButtonClass::Dropdown {
                        selected: i == operators.selected,
                    }
                    .style(),
                ),
        );
    }
    container(list)
        .padding(PAD_DROPDOWN)
        .width(Length::Fill)
        .style(theme::ContainerClass::SelectMenu.style())
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(email: &str) -> TypeaheadItem {
        TypeaheadItem {
            label: email.to_owned(),
            detail: None,
            insert_value: email.to_owned(),
        }
    }

    #[test]
    fn results_for_an_older_query_are_dropped() {
        let mut suggest = SearchSuggest::default();
        suggest.load("jul".into(), "julie", vec![contact("a@x.fr")], Vec::new());
        assert!(!suggest.open);
        suggest.load(
            "julie".into(),
            " julie ",
            vec![contact("a@x.fr")],
            Vec::new(),
        );
        assert!(suggest.open);
        assert_eq!(
            suggest.rows(),
            [SuggestRow::Contact(0), SuggestRow::ShowAll]
        );
    }

    #[test]
    fn a_person_shows_once_and_the_list_is_capped() {
        let mut suggest = SearchSuggest::default();
        let people = ["a@x.fr", "A@x.fr", "b@x.fr", "c@x.fr", "d@x.fr", "e@x.fr"]
            .map(contact)
            .to_vec();
        suggest.load("x".into(), "x", people, Vec::new());
        let emails: Vec<&str> = suggest
            .contacts
            .iter()
            .map(|c| c.insert_value.as_str())
            .collect();
        assert_eq!(emails, ["a@x.fr", "b@x.fr", "c@x.fr", "d@x.fr"]);
    }

    #[test]
    fn arrows_walk_the_rows_and_up_from_the_top_leaves_the_menu() {
        let mut suggest = SearchSuggest::default();
        suggest.load("x".into(), "x", vec![contact("a@x.fr")], Vec::new());
        assert_eq!(suggest.selected_row(), None);
        suggest.navigate(TypeaheadDirection::Down);
        assert_eq!(suggest.selected_row(), Some(SuggestRow::Contact(0)));
        suggest.navigate(TypeaheadDirection::Down);
        suggest.navigate(TypeaheadDirection::Down);
        assert_eq!(suggest.selected_row(), Some(SuggestRow::ShowAll));
        suggest.navigate(TypeaheadDirection::Up);
        suggest.navigate(TypeaheadDirection::Up);
        assert_eq!(suggest.selected_row(), None);
    }
}
