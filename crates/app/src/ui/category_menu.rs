//! The ribbon's Categorize menu: the categories with their color, a check
//! on the ones the selection carries, then "Clear all", "New category" and
//! "Manage categories", anchored under the Categorize button, the way
//! Outlook does.

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length};
use rust_i18n::t;
use types::LabelGroupId;

use crate::icon;
use crate::message::Message;
use crate::ui::label_paint::LabelPaint;
use crate::ui::layout::*;
use crate::ui::{theme, widgets};

/// One category of the menu.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryEntry {
    pub group_id: LabelGroupId,
    pub name: String,
    pub paint: LabelPaint,
    /// Every selected conversation carries it.
    pub applied: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CategoryMenuMessage {
    /// Put the category on the selection, or take it off when it is on.
    Toggle(LabelGroupId),
    ClearAll,
    New,
    Manage,
    Close,
}

#[derive(Debug)]
pub struct CategoryMenu {
    entries: Vec<CategoryEntry>,
    /// Categories at least one selected conversation carries: what
    /// "Clear all" takes off.
    carried: Vec<LabelGroupId>,
}

impl CategoryMenu {
    pub fn new(entries: Vec<CategoryEntry>, carried: Vec<LabelGroupId>) -> Self {
        Self { entries, carried }
    }

    pub fn is_applied(&self, group_id: LabelGroupId) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.group_id == group_id && entry.applied)
    }

    pub fn carried(&self) -> &[LabelGroupId] {
        &self.carried
    }

    pub fn view(&self) -> Element<'_, Message> {
        let mut list = column![].spacing(SPACE_XXXS);
        for entry in &self.entries {
            list = list.push(category_row(entry));
        }
        if self.entries.is_empty() {
            list = list.push(
                container(
                    text(t!("main.category_menu.empty"))
                        .size(TEXT_MD)
                        .style(theme::TextClass::Tertiary.style()),
                )
                .padding(PAD_NAV_ITEM),
            );
        }

        container(
            column![
                container(scrollable(list).height(Length::Shrink))
                    .height(Length::Fit.max(TYPEAHEAD_MAX_HEIGHT))
                    .width(Length::Fill),
                widgets::divider(),
                action_row(
                    icon::x(),
                    t!("main.category_menu.clear_all").into_owned(),
                    (!self.carried.is_empty()).then_some(CategoryMenuMessage::ClearAll),
                ),
                action_row(
                    icon::plus(),
                    t!("main.category_menu.new").into_owned(),
                    Some(CategoryMenuMessage::New),
                ),
                action_row(
                    icon::settings(),
                    t!("main.category_menu.manage").into_owned(),
                    Some(CategoryMenuMessage::Manage),
                ),
            ]
            .spacing(SPACE_XXXS),
        )
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style())
        .into()
    }
}

fn category_row(entry: &CategoryEntry) -> Element<'_, Message> {
    let bg = entry.paint.bg();
    let mark: Element<'_, Message> = if entry.applied {
        icon::check().size(ICON_MD).style(text::base).into()
    } else {
        Space::new().width(ICON_MD).into()
    };
    button(
        row![
            icon::tag()
                .size(ICON_MD)
                .style(move |_theme| text::Style { color: Some(bg) }),
            text(&entry.name)
                .size(TEXT_MD)
                .style(text::base)
                .width(Length::Fill)
                .wrapping(text::Wrapping::None)
                .ellipsis(text::Ellipsis::End),
            mark,
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(Message::CategoryMenu(CategoryMenuMessage::Toggle(
        entry.group_id,
    )))
    .padding(ROW_MENU_ITEM_PADDING)
    .height(ROW_MENU_ITEM_HEIGHT)
    .width(Length::Fill)
    .style(
        theme::ButtonClass::Dropdown {
            selected: entry.applied,
        }
        .style(),
    )
    .into()
}

fn action_row<'a>(
    glyph: iced::widget::Text<'a>,
    label: String,
    on_press: Option<CategoryMenuMessage>,
) -> Element<'a, Message> {
    let style: fn(&iced::Theme) -> text::Style = if on_press.is_some() {
        text::base
    } else {
        theme::TextClass::Tertiary.style()
    };
    button(
        row![
            glyph.size(ICON_MD).style(style),
            text(label).size(TEXT_MD).style(style),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press_maybe(on_press.map(Message::CategoryMenu))
    .padding(ROW_MENU_ITEM_PADDING)
    .height(ROW_MENU_ITEM_HEIGHT)
    .width(Length::Fill)
    .style(theme::ButtonClass::Dropdown { selected: false }.style())
    .into()
}

/// Which categories every selected conversation carries, and which at
/// least one does. `per_thread` holds one list of group ids per selected
/// conversation.
pub fn selection_coverage(per_thread: &[Vec<i64>]) -> (Vec<i64>, Vec<i64>) {
    let mut carried: Vec<i64> = per_thread.iter().flatten().copied().collect();
    carried.sort_unstable();
    carried.dedup();
    let on_all = carried
        .iter()
        .copied()
        .filter(|id| per_thread.iter().all(|groups| groups.contains(id)))
        .collect();
    (on_all, carried)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_category_is_checked_only_when_the_whole_selection_carries_it() {
        let (on_all, carried) = selection_coverage(&[vec![1, 2], vec![2, 3]]);
        assert_eq!(on_all, vec![2]);
        assert_eq!(carried, vec![1, 2, 3]);
    }

    #[test]
    fn a_selection_without_categories_checks_nothing() {
        let (on_all, carried) = selection_coverage(&[vec![], vec![]]);
        assert!(on_all.is_empty());
        assert!(carried.is_empty());
    }

    #[test]
    fn one_uncategorized_conversation_unchecks_the_category() {
        let (on_all, carried) = selection_coverage(&[vec![4], vec![]]);
        assert!(on_all.is_empty());
        assert_eq!(carried, vec![4]);
    }
}
