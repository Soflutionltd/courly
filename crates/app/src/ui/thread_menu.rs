//! Right-click menu on a thread list row, laid out like Outlook's. A
//! submenu replaces the main page in place, with a way back, instead of
//! cascading to the side.
use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length, Vector};

use crate::icon;
use crate::ui::layout::{
    ICON_SM, PAD_DROPDOWN, ROW_MENU_ITEM_HEIGHT, ROW_MENU_ITEM_PADDING, SPACE_XS, SPACE_XXXS,
    TEXT_MD, TEXT_SM,
};
use crate::ui::theme;
use crate::ui::thread_list::ThreadListMessage;
use crate::ui::widgets;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowMenuPage {
    Main,
    Snooze,
    Related,
    Sender,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowMenuItem {
    Open,
    Reply,
    ReplyAll,
    Forward,
    CreateEvent,
    ToggleRead,
    Snooze { until: i64 },
    SearchSameSubject,
    SearchFromSender,
    AddSenderToContacts,
    CopySenderAddress,
    ToggleFlag,
    TogglePin,
    Categorize,
    Uncategorize,
    ToggleSpam,
    Move,
    ToggleMute,
    Delete,
    Archive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowMenuEntry {
    Item {
        item: RowMenuItem,
        label: String,
        shortcut: Option<String>,
        enabled: bool,
    },
    Page {
        page: RowMenuPage,
        label: String,
        enabled: bool,
    },
    Divider,
}

/// Every page of the menu, built by the app when the menu opens, from the
/// selection and the commands available for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowMenuPages {
    pub main: Vec<RowMenuEntry>,
    pub snooze: Vec<RowMenuEntry>,
    pub related: Vec<RowMenuEntry>,
    pub sender: Vec<RowMenuEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowMenu {
    /// Row the menu floats over.
    pub index: usize,
    /// Where the click landed, from the row's top-left corner.
    pub offset: Vector,
    pub page: RowMenuPage,
    pub pages: RowMenuPages,
}

impl RowMenu {
    pub fn new(index: usize, offset: Vector) -> Self {
        Self {
            index,
            offset,
            page: RowMenuPage::Main,
            pages: RowMenuPages::default(),
        }
    }

    fn entries(&self) -> &[RowMenuEntry] {
        match self.page {
            RowMenuPage::Main => &self.pages.main,
            RowMenuPage::Snooze => &self.pages.snooze,
            RowMenuPage::Related => &self.pages.related,
            RowMenuPage::Sender => &self.pages.sender,
        }
    }

    /// Label of the main-page entry that opened the current submenu.
    fn page_title(&self) -> Option<&str> {
        self.pages.main.iter().find_map(|entry| match entry {
            RowMenuEntry::Page { page, label, .. } if *page == self.page => Some(label.as_str()),
            _ => None,
        })
    }
}

pub fn row_menu_card(menu: &RowMenu) -> Element<'_, ThreadListMessage> {
    let mut items = column![].spacing(SPACE_XXXS);
    if menu.page != RowMenuPage::Main {
        items = items
            .push(menu_button(
                row![
                    icon::chevron_left().size(ICON_SM).style(text::secondary),
                    text(menu.page_title().unwrap_or_default())
                        .size(TEXT_MD)
                        .font(crate::font::text_semibold()),
                ]
                .spacing(SPACE_XS)
                .align_y(Alignment::Center),
                Some(ThreadListMessage::RowMenuShowPage(RowMenuPage::Main)),
            ))
            .push(widgets::divider());
    }
    for entry in menu.entries() {
        items = items.push(match entry {
            RowMenuEntry::Item {
                item,
                label,
                shortcut,
                enabled,
            } => menu_button(
                row![
                    entry_label(label, *enabled),
                    Space::new().width(Length::Fill),
                    text(shortcut.as_deref().unwrap_or_default())
                        .size(TEXT_SM)
                        .style(theme::TextClass::Tertiary.style()),
                ]
                .align_y(Alignment::Center),
                enabled.then_some(ThreadListMessage::RowMenuPick(*item)),
            ),
            RowMenuEntry::Page {
                page,
                label,
                enabled,
            } => menu_button(
                row![
                    entry_label(label, *enabled),
                    Space::new().width(Length::Fill),
                    icon::chevron_right()
                        .size(ICON_SM)
                        .style(theme::TextClass::Tertiary.style()),
                ]
                .align_y(Alignment::Center),
                enabled.then_some(ThreadListMessage::RowMenuShowPage(*page)),
            ),
            RowMenuEntry::Divider => widgets::divider(),
        });
    }
    container(items)
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style())
        .into()
}

fn entry_label(label: &str, enabled: bool) -> iced::widget::Text<'_> {
    let label = text(label).size(TEXT_MD);
    if enabled {
        label.style(text::base)
    } else {
        label.style(theme::TextClass::Tertiary.style())
    }
}

fn menu_button<'a>(
    content: impl Into<Element<'a, ThreadListMessage>>,
    on_press: Option<ThreadListMessage>,
) -> Element<'a, ThreadListMessage> {
    button(container(content).height(Length::Fill).align_y(Alignment::Center))
        .on_press_maybe(on_press)
        .padding(ROW_MENU_ITEM_PADDING)
        .height(ROW_MENU_ITEM_HEIGHT)
        .width(Length::Fill)
        .style(theme::ButtonClass::Dropdown { selected: false }.style())
        .into()
}
