//! Scope pill at the left of the title-bar search field: the active
//! folder, one added mailbox, or every mailbox. The choice is persisted
//! and never follows sidebar navigation.

use iced::widget::{Space, button, column, container, row, rule, text};
use iced::{Alignment, Element, Length};
use rust_i18n::t;
use serde::{Deserialize, Serialize};

use crate::db::Account;
use crate::icon;
use crate::message::Message;
use crate::ui::anchored_overlay::anchored_overlay;
use crate::ui::search_suggest::SearchSuggestMessage;
use crate::ui::layout::{
    ICON_MD, ICON_SM, PAD_DROPDOWN, PAD_NAV_ITEM, PAD_SEARCH_SCOPE_PILL, SEARCH_SCOPE_MENU_WIDTH,
    SPACE_XS, SPACE_XXS, SPACE_XXXS, TEXT_MD, TEXT_SM,
};
use crate::ui::theme;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SearchScopeChoice {
    /// The folder selected in the sidebar, in the account it is shown for.
    ActiveFolder,
    /// Every folder of one added account.
    Mailbox { account_id: String },
    /// Every folder of every account.
    #[default]
    AllMailboxes,
}

impl SearchScopeChoice {
    pub fn parse(json: Option<&str>) -> Self {
        let Some(json) = json else {
            return Self::default();
        };
        serde_json::from_str(json).unwrap_or_else(|e| {
            log::warn!("search_scope unreadable, searching every mailbox: {e}");
            Self::default()
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::json!(self).to_string()
    }

    /// The choice in effect: a mailbox that is no longer set up widens to
    /// every mailbox.
    pub fn effective(&self, accounts: &[Account]) -> Self {
        match self {
            Self::Mailbox { account_id }
                if !accounts.iter().any(|account| account.id == *account_id) =>
            {
                Self::AllMailboxes
            }
            other => other.clone(),
        }
    }
}

#[derive(Debug, Default)]
pub struct SearchScopeState {
    pub choice: SearchScopeChoice,
    pub menu_open: bool,
}

#[derive(Debug, Clone)]
pub enum SearchScopeMessage {
    ToggleMenu,
    CloseMenu,
    Select(SearchScopeChoice),
}

pub fn mailbox_label(account: &Account) -> &str {
    account
        .account_name
        .as_deref()
        .or(account.display_name.as_deref())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&account.email)
}

fn choice_label(choice: &SearchScopeChoice, accounts: &[Account]) -> String {
    match choice {
        SearchScopeChoice::ActiveFolder => t!("main.search.scope_active_folder").into_owned(),
        SearchScopeChoice::Mailbox { account_id } => accounts
            .iter()
            .find(|account| account.id == *account_id)
            .map_or_else(
                || t!("main.search.scope_all_mailboxes").into_owned(),
                |account| mailbox_label(account).to_owned(),
            ),
        SearchScopeChoice::AllMailboxes => t!("main.search.scope_all_mailboxes").into_owned(),
    }
}

/// The search box: scope pill, the picked sender if any, then `input`.
pub fn search_field<'a>(
    state: &SearchScopeState,
    accounts: &'a [Account],
    sender: Option<&'a str>,
    input: Element<'a, Message>,
) -> Element<'a, Message> {
    let current = state.choice.effective(accounts);
    let pill = button(
        row![
            text(choice_label(&current, accounts)).size(TEXT_SM),
            icon::chevron_down().size(ICON_SM),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .on_press(Message::SearchScope(SearchScopeMessage::ToggleMenu))
    .padding(PAD_SEARCH_SCOPE_PILL)
    .style(theme::ButtonClass::Primary.style());

    let pill: Element<'a, Message> = if state.menu_open {
        anchored_overlay(pill)
            .popup(menu(&current, accounts))
            .popup_width(SEARCH_SCOPE_MENU_WIDTH)
            .on_dismiss(Message::SearchScope(SearchScopeMessage::CloseMenu))
            .into()
    } else {
        pill.into()
    };

    // The slot is always present so the field keeps its place in the
    // widget tree, and with it the keyboard focus.
    let sender: Element<'a, Message> = match sender {
        Some(email) => container(
            button(
                row![text(email).size(TEXT_SM), icon::x().size(ICON_SM)]
                    .spacing(SPACE_XXS)
                    .align_y(Alignment::Center),
            )
            .on_press(Message::SearchSuggest(SearchSuggestMessage::ClearSender))
            .padding(PAD_SEARCH_SCOPE_PILL)
            .style(theme::ButtonClass::Chip { active: true }.style()),
        )
        .padding(iced::Padding::from([0.0, SPACE_XXS]))
        .into(),
        None => Space::new().width(SPACE_XXS).into(),
    };

    container(
        row![pill, sender, container(input).width(Length::Fill)].align_y(Alignment::Center),
    )
    .padding(iced::Padding::from([0.0, SPACE_XXS]))
    .align_y(Alignment::Center)
    .style(theme::ContainerClass::TitleSearchField.style())
    .into()
}

/// The mailboxes, then below a rule the two choices that stay put however
/// many mailboxes there are.
fn menu<'a>(current: &SearchScopeChoice, accounts: &'a [Account]) -> Element<'a, Message> {
    let mut entries: Vec<Element<'a, Message>> = accounts
        .iter()
        .filter(|account| !account.is_deleting)
        .map(|account| {
            entry(
                mailbox_label(account).to_owned(),
                SearchScopeChoice::Mailbox {
                    account_id: account.id.clone(),
                },
                current,
            )
        })
        .collect();
    if !entries.is_empty() {
        entries.push(
            container(rule::horizontal(1).style(theme::RuleClass::Subtle.style()))
                .padding(iced::Padding::from([SPACE_XXS, 0.0]))
                .into(),
        );
    }
    entries.push(entry(
        t!("main.search.scope_active_folder").into_owned(),
        SearchScopeChoice::ActiveFolder,
        current,
    ));
    entries.push(entry(
        t!("main.search.scope_all_mailboxes").into_owned(),
        SearchScopeChoice::AllMailboxes,
        current,
    ));
    container(column(entries).spacing(SPACE_XXXS))
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style())
        .into()
}

fn entry<'a>(
    label: String,
    choice: SearchScopeChoice,
    current: &SearchScopeChoice,
) -> Element<'a, Message> {
    let selected = choice == *current;
    let mark: Element<'a, Message> = if selected {
        icon::check().size(ICON_MD).into()
    } else {
        Space::new().width(ICON_MD).into()
    };
    button(
        row![mark, text(label).size(TEXT_MD)]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center),
    )
    .on_press(Message::SearchScope(SearchScopeMessage::Select(choice)))
    .padding(PAD_NAV_ITEM)
    .width(Length::Fill)
    .style(theme::ButtonClass::Dropdown { selected }.style())
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str) -> Account {
        Account {
            id: id.to_owned(),
            email: format!("{id}@example.com"),
            display_name: None,
            provider: "imap".to_owned(),
            account_name: None,
            account_color: None,
            last_sync_at: None,
            token_expires_at: None,
            is_active: true,
            sort_order: 0,
            is_deleting: false,
        }
    }

    #[test]
    fn every_mailbox_is_the_default() {
        assert_eq!(SearchScopeChoice::parse(None), SearchScopeChoice::AllMailboxes);
        assert_eq!(
            SearchScopeChoice::parse(Some("not json")),
            SearchScopeChoice::AllMailboxes
        );
    }

    #[test]
    fn every_choice_survives_a_restart() {
        for choice in [
            SearchScopeChoice::ActiveFolder,
            SearchScopeChoice::Mailbox {
                account_id: "acc1".to_owned(),
            },
            SearchScopeChoice::AllMailboxes,
        ] {
            assert_eq!(SearchScopeChoice::parse(Some(&choice.to_json())), choice);
        }
    }

    #[test]
    fn a_removed_mailbox_widens_to_every_mailbox() {
        let choice = SearchScopeChoice::Mailbox {
            account_id: "gone".to_owned(),
        };
        assert_eq!(
            choice.effective(&[account("acc1")]),
            SearchScopeChoice::AllMailboxes
        );
        let kept = SearchScopeChoice::Mailbox {
            account_id: "acc1".to_owned(),
        };
        assert_eq!(kept.effective(&[account("acc1")]), kept);
    }
}
