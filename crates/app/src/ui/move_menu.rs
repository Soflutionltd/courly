//! The ribbon's Move menu: a search field over the folders a message can
//! go to, anchored under the Move button, the way Outlook does.

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length};
use rust_i18n::t;

use crate::icon;
use crate::message::Message;
use crate::ui::layout::*;
use crate::ui::{theme, widgets};

pub const SEARCH_INPUT_ID: &str = "move-menu-search";

/// A folder the selected messages can move to.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveDestination {
    pub id: String,
    pub name: String,
    pub depth: u16,
    /// Inbox, Archive, Trash or Junk rather than one of the account's own.
    pub system: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MoveMenuMessage {
    Query(String),
    /// Enter in the search field: the first folder shown.
    Submit,
    Pick(String),
    Close,
}

#[derive(Debug)]
pub struct MoveMenu {
    query: String,
    destinations: Vec<MoveDestination>,
}

impl MoveMenu {
    pub fn new(destinations: Vec<MoveDestination>) -> Self {
        Self {
            query: String::new(),
            destinations,
        }
    }

    pub fn set_query(&mut self, query: String) {
        self.query = query;
    }

    /// The folders whose name contains the query, ignoring case.
    fn matches(&self) -> impl Iterator<Item = &MoveDestination> {
        let needle = self.query.trim().to_lowercase();
        self.destinations
            .iter()
            .filter(move |d| needle.is_empty() || d.name.to_lowercase().contains(&needle))
    }

    pub fn first_match(&self) -> Option<&MoveDestination> {
        self.matches().next()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let search = text_input(t!("main.move_menu.search"), &self.query)
            .id(SEARCH_INPUT_ID)
            .on_input(|query| Message::MoveMenu(MoveMenuMessage::Query(query)))
            .on_submit(Message::MoveMenu(MoveMenuMessage::Submit))
            .size(TEXT_MD)
            .padding(PAD_INPUT);

        // A search shows a flat list: indenting a match under a parent that
        // is filtered out would read as a mistake.
        let indented = self.query.trim().is_empty();
        let mut list = column![].spacing(SPACE_XXXS);
        let mut any = false;
        for destination in self.matches() {
            any = true;
            list = list.push(destination_row(destination, indented));
        }
        if !any {
            list = list.push(
                container(
                    text(t!("main.move_menu.no_match"))
                        .size(TEXT_MD)
                        .style(theme::TextClass::Tertiary.style()),
                )
                .padding(PAD_NAV_ITEM),
            );
        }

        container(
            column![
                search,
                widgets::divider(),
                container(scrollable(list).height(Length::Shrink))
                    .height(Length::Fit.max(TYPEAHEAD_MAX_HEIGHT))
                    .width(Length::Fill),
            ]
            .spacing(SPACE_XS),
        )
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style())
        .into()
    }
}

fn destination_row(destination: &MoveDestination, indented: bool) -> Element<'_, Message> {
    let glyph = if destination.system {
        crate::ui::sidebar::universal_folder_icon(&destination.id)
    } else {
        icon::folder()
    };
    let indent = if indented {
        SPACE_MD * f32::from(destination.depth)
    } else {
        0.0
    };
    button(
        row![
            Space::new().width(indent),
            glyph.size(ICON_MD).style(text::secondary),
            text(&destination.name)
                .size(TEXT_MD)
                .style(text::base)
                .wrapping(text::Wrapping::None)
                .ellipsis(text::Ellipsis::End),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(Message::MoveMenu(MoveMenuMessage::Pick(
        destination.id.clone(),
    )))
    .padding(ROW_MENU_ITEM_PADDING)
    .height(ROW_MENU_ITEM_HEIGHT)
    .width(Length::Fill)
    .style(theme::ButtonClass::Dropdown { selected: false }.style())
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn destination(id: &str, name: &str) -> MoveDestination {
        MoveDestination {
            id: id.into(),
            name: name.into(),
            depth: 0,
            system: false,
        }
    }

    #[test]
    fn the_search_ignores_case_and_keeps_the_order() {
        let mut menu = MoveMenu::new(vec![
            destination("INBOX", "Boîte de réception"),
            destination("f1", "Factures"),
            destination("f2", "Projets factures"),
        ]);
        menu.set_query("FACT".into());
        let ids: Vec<&str> = menu.matches().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["f1", "f2"]);
        assert_eq!(menu.first_match().map(|d| d.id.as_str()), Some("f1"));
    }

    #[test]
    fn an_empty_search_shows_every_folder() {
        let menu = MoveMenu::new(vec![destination("a", "A"), destination("b", "B")]);
        assert_eq!(menu.matches().count(), 2);
    }
}
