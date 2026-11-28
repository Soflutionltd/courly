//! The dialog that writes a commitment down from a mail: what is owed or
//! awaited, by when. It opens filled in from the selected sentence and the
//! mail's sender, so writing one down is a choice of day and a click.

use iced::widget::{button, column, row, text, text_input};
use iced::{Alignment, Element};
use jiff::Span;
use jiff::civil::{Date, Weekday};
use rust_i18n::t;

use crate::ui::dialog::{DialogAction, form_dialog};
use crate::ui::layout::{PAD_BUTTON, PAD_INPUT, SPACE_SM, SPACE_XS, TEXT_LG, TEXT_MD, TEXT_SM};
use crate::ui::theme;

pub const TEXT_INPUT_ID: &str = "commitment-text";

/// Which way a commitment runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// The user owes it.
    Owed,
    /// The user is waiting for it.
    Awaited,
}

impl Direction {
    /// The value stored in `commitments.direction`.
    pub fn as_stored(self) -> &'static str {
        match self {
            Self::Owed => "owed",
            Self::Awaited => "awaited",
        }
    }

    pub fn from_stored(stored: &str) -> Option<Self> {
        match stored {
            "owed" => Some(Self::Owed),
            "awaited" => Some(Self::Awaited),
            _ => None,
        }
    }
}

/// The day a commitment is due, as the dialog offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    Unset,
    Today,
    Tomorrow,
    /// The coming Friday, today included.
    Friday,
    /// Monday of next week.
    NextWeek,
    /// The day typed in the date field.
    Typed,
}

impl Due {
    const QUICK: [Self; 5] = [
        Self::Unset,
        Self::Today,
        Self::Tomorrow,
        Self::Friday,
        Self::NextWeek,
    ];

    fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Unset => t!("main.commitment.due_unset"),
            Self::Today => t!("main.commitment.due_today"),
            Self::Tomorrow => t!("main.commitment.due_tomorrow"),
            Self::Friday => t!("main.commitment.due_friday"),
            Self::NextWeek => t!("main.commitment.due_next_week"),
            Self::Typed => t!("main.commitment.due_typed"),
        }
    }
}

/// `after` whole days past `today`, or `today` itself on overflow.
fn days_after(today: Date, after: i64) -> Date {
    today.checked_add(Span::new().days(after)).unwrap_or(today)
}

/// The day `due` names, counted from `today`. `typed` is the date field.
pub fn due_day(due: Due, today: Date, typed: &str) -> Option<Date> {
    match due {
        Due::Unset => None,
        Due::Today => Some(today),
        Due::Tomorrow => Some(days_after(today, 1)),
        Due::Friday => {
            let ahead = (Weekday::Friday.to_monday_zero_offset()
                - today.weekday().to_monday_zero_offset())
            .rem_euclid(7);
            Some(days_after(today, i64::from(ahead)))
        }
        Due::NextWeek => {
            let ahead = 7 - today.weekday().to_monday_zero_offset();
            Some(days_after(today, i64::from(ahead)))
        }
        Due::Typed => typed_day(typed),
    }
}

/// A day typed as `DD/MM/YYYY`, the separators being `/`, `.` or `-`.
fn typed_day(typed: &str) -> Option<Date> {
    let mut parts = typed
        .trim()
        .split(['/', '.', '-'])
        .map(|part| part.trim().parse::<i16>());
    let (day, month, year) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    if parts.next().is_some() {
        return None;
    }
    Date::new(year, i8::try_from(month).ok()?, i8::try_from(day).ok()?).ok()
}

/// What the dialog is filled in with when it opens.
#[derive(Debug, Clone)]
pub struct CommitmentDraft {
    /// The commitment being changed, or `None` for a new one.
    pub id: Option<String>,
    pub account_id: String,
    pub direction: Direction,
    pub text: String,
    pub person_name: Option<String>,
    pub person_address: Option<String>,
    pub due_date: Option<Date>,
    pub thread_id: Option<String>,
    pub message_id: Option<String>,
    pub done: bool,
}

#[derive(Debug, Clone)]
pub struct CommitmentDialog {
    draft: CommitmentDraft,
    due: Due,
    typed: String,
    today: Date,
    /// A service call is running; the dialog can't be submitted or closed.
    pub pending: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum CommitmentDialogMessage {
    TextChanged(String),
    DirectionPicked(Direction),
    DuePicked(Due),
    DateTyped(String),
    Submit,
    Cancel,
    Done(Result<(), String>),
}

impl CommitmentDialog {
    pub fn new(draft: CommitmentDraft, today: Date) -> Self {
        let (due, typed) = match draft.due_date {
            None => (Due::Unset, String::new()),
            Some(day) => (
                Due::Typed,
                format!("{:02}/{:02}/{}", day.day(), day.month(), day.year()),
            ),
        };
        Self {
            draft,
            due,
            typed,
            today,
            pending: false,
            error: None,
        }
    }

    pub fn set_text(&mut self, text: String) {
        self.draft.text = text;
    }

    pub fn set_direction(&mut self, direction: Direction) {
        self.draft.direction = direction;
    }

    pub fn set_due(&mut self, due: Due) {
        self.due = due;
    }

    /// Typing a date picks it as the due day.
    pub fn type_date(&mut self, typed: String) {
        self.typed = typed;
        self.due = Due::Typed;
    }

    fn due_day(&self) -> Option<Date> {
        due_day(self.due, self.today, &self.typed)
    }

    pub fn can_submit(&self) -> bool {
        !self.pending
            && !self.draft.text.trim().is_empty()
            && (self.due != Due::Typed || self.due_day().is_some())
    }

    /// The request that stores the dialog's content. A new commitment gets
    /// its id here.
    pub fn save_params(&self) -> service_api::CommitmentSaveParams {
        service_api::CommitmentSaveParams {
            id: self
                .draft
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            account_id: self.draft.account_id.clone(),
            direction: self.draft.direction.as_stored().to_owned(),
            text: self.draft.text.trim().to_owned(),
            person_name: self.draft.person_name.clone(),
            person_address: self.draft.person_address.clone(),
            due_date: self.due_day().map(|day| day.to_string()),
            thread_id: self.draft.thread_id.clone(),
            message_id: self.draft.message_id.clone(),
            done: self.draft.done,
        }
    }

    pub fn view(&self) -> Element<'_, CommitmentDialogMessage> {
        let chip = |label: std::borrow::Cow<'static, str>,
                    active: bool,
                    message: CommitmentDialogMessage| {
            let mut chip = button(text(label).size(TEXT_MD))
                .style(theme::ButtonClass::Chip { active }.style())
                .padding(PAD_BUTTON);
            if !self.pending {
                chip = chip.on_press(message);
            }
            chip
        };

        let direction = row![
            chip(
                t!("main.commitment.owed"),
                self.draft.direction == Direction::Owed,
                CommitmentDialogMessage::DirectionPicked(Direction::Owed),
            ),
            chip(
                t!("main.commitment.awaited"),
                self.draft.direction == Direction::Awaited,
                CommitmentDialogMessage::DirectionPicked(Direction::Awaited),
            ),
        ]
        .spacing(SPACE_XS);

        let mut what = text_input(t!("main.commitment.text_placeholder"), &self.draft.text)
            .id(TEXT_INPUT_ID)
            .size(TEXT_LG)
            .padding(PAD_INPUT);
        let mut date = text_input(t!("main.commitment.date_placeholder"), &self.typed)
            .size(TEXT_MD)
            .padding(PAD_INPUT);
        if !self.pending {
            what = what
                .on_input(CommitmentDialogMessage::TextChanged)
                .on_submit(CommitmentDialogMessage::Submit);
            date = date
                .on_input(CommitmentDialogMessage::DateTyped)
                .on_submit(CommitmentDialogMessage::Submit);
        }

        let mut body = column![direction, what].spacing(SPACE_SM);
        if let Some(person) = self
            .draft
            .person_name
            .as_deref()
            .or(self.draft.person_address.as_deref())
        {
            let line = match self.draft.direction {
                Direction::Owed => t!("main.commitment.owed_to", person = person),
                Direction::Awaited => t!("main.commitment.awaited_from", person = person),
            };
            body = body.push(
                text(line)
                    .size(TEXT_MD)
                    .style(theme::TextClass::Muted.style()),
            );
        }
        let mut days = row![].spacing(SPACE_XS).align_y(Alignment::Center);
        for due in Due::QUICK {
            days = days.push(chip(
                due.label(),
                self.due == due,
                CommitmentDialogMessage::DuePicked(due),
            ));
        }
        body = body.push(days.wrap()).push(date);
        if self.due == Due::Typed && !self.typed.trim().is_empty() && self.due_day().is_none() {
            body = body.push(
                text(t!("main.commitment.date_invalid"))
                    .size(TEXT_SM)
                    .style(text::danger),
            );
        }
        if let Some(error) = &self.error {
            body = body.push(
                text(t!("main.commitment.failed", error = error))
                    .size(TEXT_SM)
                    .style(text::danger),
            );
        }

        let title = if self.draft.id.is_some() {
            t!("main.commitment.edit_title")
        } else {
            t!("main.commitment.new_title")
        };
        form_dialog(
            title,
            body,
            vec![
                DialogAction::default_action(
                    t!("main.commitment.cancel"),
                    CommitmentDialogMessage::Cancel,
                )
                .disabled_when(self.pending),
                DialogAction::suggested(
                    t!("main.commitment.save"),
                    CommitmentDialogMessage::Submit,
                )
                .disabled_when(!self.can_submit()),
            ],
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn draft() -> CommitmentDraft {
        CommitmentDraft {
            id: None,
            account_id: "acc".to_owned(),
            direction: Direction::Awaited,
            text: " the signed contract ".to_owned(),
            person_name: Some("Ada".to_owned()),
            person_address: Some("ada@example.com".to_owned()),
            due_date: None,
            thread_id: Some("thread".to_owned()),
            message_id: Some("message".to_owned()),
            done: false,
        }
    }

    #[test]
    fn quick_days_count_from_today() {
        // A Wednesday.
        let today = date(2026, 12, 2);
        assert_eq!(due_day(Due::Unset, today, ""), None);
        assert_eq!(due_day(Due::Today, today, ""), Some(today));
        assert_eq!(due_day(Due::Tomorrow, today, ""), Some(date(2026, 12, 3)));
        assert_eq!(due_day(Due::Friday, today, ""), Some(date(2026, 12, 4)));
        assert_eq!(due_day(Due::NextWeek, today, ""), Some(date(2026, 12, 7)));
    }

    #[test]
    fn friday_is_today_on_a_friday_and_the_next_one_on_a_weekend() {
        assert_eq!(
            due_day(Due::Friday, date(2026, 12, 4), ""),
            Some(date(2026, 12, 4))
        );
        assert_eq!(
            due_day(Due::Friday, date(2026, 12, 5), ""),
            Some(date(2026, 12, 11))
        );
        // Next week from a Sunday is the day after.
        assert_eq!(
            due_day(Due::NextWeek, date(2026, 12, 6), ""),
            Some(date(2026, 12, 7))
        );
    }

    #[test]
    fn a_typed_day_reads_day_month_year_and_refuses_the_rest() {
        let today = date(2026, 12, 2);
        assert_eq!(
            due_day(Due::Typed, today, " 24/12/2026 "),
            Some(date(2026, 12, 24))
        );
        assert_eq!(
            due_day(Due::Typed, today, "3.1.2027"),
            Some(date(2027, 1, 3))
        );
        for bad in [
            "",
            "31/02/2026",
            "24/12",
            "12/24/2026",
            "24/12/2026/1",
            "demain",
        ] {
            assert_eq!(due_day(Due::Typed, today, bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_new_commitment_is_saved_trimmed_with_its_day_and_a_fresh_id() {
        let mut dialog = CommitmentDialog::new(draft(), date(2026, 12, 2));
        dialog.set_due(Due::Friday);
        let params = dialog.save_params();
        assert_eq!(params.text, "the signed contract");
        assert_eq!(params.direction, "awaited");
        assert_eq!(params.due_date.as_deref(), Some("2026-12-04"));
        assert!(!params.id.is_empty());
        assert_ne!(params.id, dialog.save_params().id);
    }

    #[test]
    fn an_edited_commitment_keeps_its_id_and_shows_its_day_as_typed() {
        let mut existing = draft();
        existing.id = Some("c-1".to_owned());
        existing.due_date = Some(date(2026, 12, 24));
        let dialog = CommitmentDialog::new(existing, date(2026, 12, 2));
        assert_eq!(dialog.typed, "24/12/2026");
        let params = dialog.save_params();
        assert_eq!(params.id, "c-1");
        assert_eq!(params.due_date.as_deref(), Some("2026-12-24"));
    }

    #[test]
    fn an_empty_text_or_an_unreadable_typed_day_cannot_be_saved() {
        let mut dialog = CommitmentDialog::new(draft(), date(2026, 12, 2));
        assert!(dialog.can_submit());
        dialog.type_date("someday".to_owned());
        assert!(!dialog.can_submit());
        dialog.set_due(Due::Today);
        assert!(dialog.can_submit());
        dialog.set_text("   ".to_owned());
        assert!(!dialog.can_submit());
    }
}
