//! The list of commitments: what the reader owes, what they are waiting
//! for, and what was closed lately, each a click away from its mail.

use iced::widget::{button, checkbox, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length};
use jiff::civil::Date;
use rtsk::db::types::DbCommitment;
use rust_i18n::t;

use crate::icon;
use crate::ui::commitment_dialog::Direction;
use crate::ui::dialog::{DialogAction, form_dialog};
use crate::ui::layout::{
    ICON_SM, PAD_ICON_BTN, SCROLLBAR_SPACING, SPACE_SM, SPACE_XS, SPACE_XXS, TEXT_LG, TEXT_MD,
    TEXT_SM, TYPEAHEAD_MAX_HEIGHT,
};
use crate::ui::theme;

/// How long a closed commitment stays listed (seconds): a week.
pub const CLOSED_SHOWN_FOR: i64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone)]
pub struct CommitmentList {
    items: Vec<DbCommitment>,
    today: Date,
    /// The list has not been read yet.
    loading: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum CommitmentListMessage {
    Loaded(Result<Vec<DbCommitment>, String>),
    SetDone(String, bool),
    Edit(String),
    Delete(String),
    OpenMail(String),
    Written(Result<(), String>),
    Close,
}

/// The day `iso` (`YYYY-MM-DD`) as the list shows it: day and month, and
/// the year when it is not `today`'s.
fn shown_day(iso: &str, today: Date) -> String {
    match iso.parse::<Date>() {
        Ok(day) if day.year() == today.year() => format!("{:02}/{:02}", day.day(), day.month()),
        Ok(day) => format!("{:02}/{:02}/{}", day.day(), day.month(), day.year()),
        Err(_) => iso.to_owned(),
    }
}

/// Whether an open commitment due on `iso` is late on `today`.
fn is_late(iso: &str, today: Date) -> bool {
    iso.parse::<Date>().is_ok_and(|day| day < today)
}

impl CommitmentList {
    pub fn new(today: Date) -> Self {
        Self {
            items: Vec::new(),
            today,
            loading: true,
            error: None,
        }
    }

    pub fn set_items(&mut self, items: Vec<DbCommitment>) {
        self.items = items;
        self.loading = false;
        self.error = None;
    }

    pub fn find(&self, id: &str) -> Option<&DbCommitment> {
        self.items.iter().find(|item| item.id == id)
    }

    fn open(&self, direction: Direction) -> impl Iterator<Item = &DbCommitment> {
        self.items
            .iter()
            .filter(move |item| item.done_at.is_none() && item.direction == direction.as_stored())
    }

    fn row<'a>(&self, item: &'a DbCommitment) -> Element<'a, CommitmentListMessage> {
        let done = item.done_at.is_some();
        let id = item.id.clone();
        let tick = checkbox(done)
            .on_toggle(move |checked| CommitmentListMessage::SetDone(id.clone(), checked));

        let mut detail: Vec<String> = Vec::new();
        if let Some(person) = item
            .person_name
            .as_deref()
            .or(item.person_address.as_deref())
        {
            detail.push(person.to_owned());
        }
        let late = !done
            && item
                .due_date
                .as_deref()
                .is_some_and(|day| is_late(day, self.today));
        if let Some(day) = item.due_date.as_deref() {
            detail.push(shown_day(day, self.today));
        }
        let mut words = column![text(item.text.as_str()).size(TEXT_LG)].spacing(SPACE_XXS);
        if !detail.is_empty() {
            let line = text(detail.join("  ·  ")).size(TEXT_SM);
            words = words.push(if late {
                line.style(text::danger)
            } else {
                line.style(theme::TextClass::Tertiary.style())
            });
        }

        let small = |ico: iced::widget::Text<'a>, message: CommitmentListMessage| {
            button(ico.size(ICON_SM).style(text::secondary))
                .on_press(message)
                .padding(PAD_ICON_BTN)
                .style(theme::ButtonClass::Ghost.style())
        };
        let mut actions = row![].spacing(SPACE_XXS).align_y(Alignment::Center);
        if item.thread_id.is_some() {
            actions = actions.push(small(
                icon::mail(),
                CommitmentListMessage::OpenMail(item.id.clone()),
            ));
        }
        actions = actions
            .push(small(
                icon::pencil(),
                CommitmentListMessage::Edit(item.id.clone()),
            ))
            .push(small(
                icon::trash(),
                CommitmentListMessage::Delete(item.id.clone()),
            ));

        row![tick, container(words).width(Length::Fill), actions]
            .spacing(SPACE_SM)
            .align_y(Alignment::Center)
            .into()
    }

    fn section<'a>(
        &self,
        title: std::borrow::Cow<'a, str>,
        items: impl Iterator<Item = &'a DbCommitment>,
    ) -> Option<Element<'a, CommitmentListMessage>> {
        let mut rows = column![
            text(title)
                .size(TEXT_MD)
                .style(theme::TextClass::Muted.style())
        ]
        .spacing(SPACE_XS);
        let mut any = false;
        for item in items {
            rows = rows.push(self.row(item));
            any = true;
        }
        any.then(|| rows.into())
    }

    pub fn view(&self) -> Element<'_, CommitmentListMessage> {
        let sections = [
            self.section(t!("main.commitment.owed"), self.open(Direction::Owed)),
            self.section(t!("main.commitment.awaited"), self.open(Direction::Awaited)),
            self.section(
                t!("main.commitment.done_lately"),
                self.items.iter().filter(|item| item.done_at.is_some()),
            ),
        ];
        let mut list = column![].spacing(SPACE_SM);
        let mut any = false;
        for section in sections.into_iter().flatten() {
            list = list.push(section);
            any = true;
        }
        if !any && !self.loading {
            list = list.push(
                text(t!("main.commitment.none"))
                    .size(TEXT_MD)
                    .style(theme::TextClass::Muted.style()),
            );
        }
        let mut body = column![
            container(
                scrollable(list)
                    .spacing(SCROLLBAR_SPACING)
                    .height(Length::Shrink)
            )
            .height(Length::Fit.max(TYPEAHEAD_MAX_HEIGHT))
            .width(Length::Fill)
        ]
        .spacing(SPACE_SM);
        if let Some(error) = &self.error {
            body = body.push(
                text(t!("main.commitment.failed", error = error))
                    .size(TEXT_SM)
                    .style(text::danger),
            );
        }
        form_dialog(
            t!("main.commitment.list_title"),
            body,
            vec![DialogAction::default_action(
                t!("main.commitment.close"),
                CommitmentListMessage::Close,
            )],
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn a_day_shows_its_year_only_when_it_is_another_one() {
        let today = date(2026, 12, 2);
        assert_eq!(shown_day("2026-12-04", today), "04/12");
        assert_eq!(shown_day("2027-01-03", today), "03/01/2027");
        assert_eq!(shown_day("someday", today), "someday");
    }

    #[test]
    fn a_commitment_is_late_the_day_after_it_was_due() {
        let today = date(2026, 12, 2);
        assert!(is_late("2026-12-01", today));
        assert!(!is_late("2026-12-02", today));
        assert!(!is_late("2026-12-03", today));
    }
}
