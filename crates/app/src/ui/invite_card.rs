//! Outlook-style meeting invite card, shown above an invite's body.

use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Element, Length};
use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

use crate::db::{CalendarEvent, ThreadMessage};
use crate::font;
use crate::icon;
use crate::ui::animated_toggler::animated_toggler;
use crate::ui::calendar_time_grid::TimeGridEvent;
use crate::ui::layout::*;
use crate::ui::theme;

use service_api::{RsvpResponse, WireRsvpReply};

/// Online meeting links recognised in an invite's location or body.
const MEETING_LINK_MARKERS: [&str; 5] = [
    "teams.microsoft.com/l/meetup-join",
    "teams.live.com/meet",
    "zoom.us/j/",
    "meet.google.com/",
    "webex.com/",
];

/// Calendar data an invite resolved to.
#[derive(Debug, Clone)]
pub struct InviteDetails {
    pub event: CalendarEvent,
    /// Every synced event on the invite's local day, all-day first, then by start.
    pub day_events: Vec<TimeGridEvent>,
}

#[derive(Debug, Clone)]
pub enum InviteLookup {
    Loading,
    /// No single synced calendar event carries this invite's UID.
    Unsynced,
    Found(Box<InviteDetails>),
    Failed(String),
}

/// What an invite message is, from its iMIP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteKind {
    Request,
    Reply,
    Cancel,
}

impl InviteKind {
    pub fn of(msg: &ThreadMessage) -> Option<Self> {
        if !msg.has_meeting_invite {
            return None;
        }
        Some(match msg.meeting_invite_method.as_deref() {
            Some("REPLY" | "COUNTER" | "DECLINECOUNTER") => Self::Reply,
            Some("CANCEL") => Self::Cancel,
            _ => Self::Request,
        })
    }
}

#[derive(Debug, Clone)]
pub enum InviteMessage {
    CommentChanged(String),
    SendResponseToggled(bool),
    ProposeToggled,
    ProposeDateChanged(String),
    ProposeStartChanged(String),
    ProposeEndChanged(String),
    Respond(RsvpResponse),
    Join(String),
}

/// What the reading pane does after an invite edit.
#[derive(Debug, Clone)]
pub enum InviteAction {
    Respond {
        event_id: String,
        response: RsvpResponse,
        reply: WireRsvpReply,
    },
    Open(String),
}

#[derive(Debug, Clone)]
pub struct InviteCard {
    pub lookup: InviteLookup,
    /// Whether the account's provider takes a comment, a notification
    /// choice and a proposed time with the response.
    pub rich_reply: bool,
    pub join_url: Option<String>,
    pub comment: String,
    pub send_response: bool,
    pub proposing: bool,
    pub propose_date: String,
    pub propose_start: String,
    pub propose_end: String,
    /// Response in flight, until the service answers.
    pub pending: Option<RsvpResponse>,
    pub error: Option<String>,
}

impl InviteCard {
    pub fn new(msg: &ThreadMessage) -> Self {
        let lookup = if msg.meeting_invite_uid.is_some() {
            InviteLookup::Loading
        } else {
            InviteLookup::Unsynced
        };
        Self {
            lookup,
            rich_reply: false,
            join_url: join_url(&[msg.body_text.as_deref(), msg.body_html.as_deref()]),
            comment: String::new(),
            send_response: true,
            proposing: false,
            propose_date: String::new(),
            propose_start: String::new(),
            propose_end: String::new(),
            pending: None,
            error: None,
        }
    }

    pub fn details(&self) -> Option<&InviteDetails> {
        match &self.lookup {
            InviteLookup::Found(details) => Some(details),
            _ => None,
        }
    }

    pub fn set_lookup(&mut self, lookup: InviteLookup) {
        if let InviteLookup::Found(details) = &lookup
            && self.join_url.is_none()
        {
            self.join_url = join_url(&[
                details.event.location.as_deref(),
                details.event.description.as_deref(),
            ]);
        }
        self.lookup = lookup;
    }

    pub fn update(&mut self, message: InviteMessage, tz: &TimeZone) -> Option<InviteAction> {
        match message {
            InviteMessage::CommentChanged(comment) => self.comment = comment,
            InviteMessage::SendResponseToggled(send) => {
                self.send_response = send;
                if !send {
                    self.proposing = false;
                }
            }
            InviteMessage::ProposeToggled => {
                self.proposing = !self.proposing && self.send_response;
                if self.proposing
                    && self.propose_date.is_empty()
                    && let Some(details) = self.details()
                {
                    let (date, start, end) = slot_fields(&details.event, tz);
                    self.propose_date = date;
                    self.propose_start = start;
                    self.propose_end = end;
                }
            }
            InviteMessage::ProposeDateChanged(value) => self.propose_date = value,
            InviteMessage::ProposeStartChanged(value) => self.propose_start = value,
            InviteMessage::ProposeEndChanged(value) => self.propose_end = value,
            InviteMessage::Join(url) => return Some(InviteAction::Open(url)),
            InviteMessage::Respond(response) => return self.respond(response, tz),
        }
        None
    }

    fn respond(&mut self, response: RsvpResponse, tz: &TimeZone) -> Option<InviteAction> {
        let event_id = self.details()?.event.id.clone();
        let proposing = self.rich_reply && self.proposing;
        let proposed = if proposing {
            if response == RsvpResponse::Accepted {
                self.error = Some("A new time goes with Tentative or Decline".to_string());
                return None;
            }
            match proposed_slot(
                &self.propose_date,
                &self.propose_start,
                &self.propose_end,
                tz,
            ) {
                Ok(slot) => Some(slot),
                Err(error) => {
                    self.error = Some(error);
                    return None;
                }
            }
        } else {
            None
        };
        let comment = self.comment.trim();
        let reply = WireRsvpReply {
            comment: (self.rich_reply && !comment.is_empty()).then(|| comment.to_string()),
            send_response: self.rich_reply && self.send_response,
            proposed_start: proposed.map(|(start, _)| start),
            proposed_end: proposed.map(|(_, end)| end),
        };
        self.pending = Some(response);
        self.error = None;
        Some(InviteAction::Respond {
            event_id,
            response,
            reply,
        })
    }

    /// Settle the in-flight response once the service answers.
    pub fn finish(&mut self, result: Result<(), String>) {
        let response = self.pending.take();
        match result {
            Ok(()) => {
                if let (Some(response), InviteLookup::Found(details)) = (response, &mut self.lookup)
                {
                    details.event.rsvp_status = Some(rsvp_value(response).to_string());
                }
                self.comment.clear();
                self.proposing = false;
            }
            Err(error) => self.error = Some(error),
        }
    }
}

fn rsvp_value(response: RsvpResponse) -> &'static str {
    match response {
        RsvpResponse::Accepted => "accepted",
        RsvpResponse::Declined => "declined",
        RsvpResponse::Tentative => "tentative",
    }
}

/// First online meeting link found in the given texts.
pub fn join_url(sources: &[Option<&str>]) -> Option<String> {
    sources.iter().flatten().find_map(|source| {
        source
            .split(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']')
            })
            .map(|token| token.trim_end_matches(['.', ',', ';']))
            .find(|token| {
                token.starts_with("https://")
                    && MEETING_LINK_MARKERS
                        .iter()
                        .any(|marker| token.contains(marker))
            })
            .map(|token| token.replace("&amp;", "&"))
    })
}

/// Display names from an address header ("Name <a@b>, c@d").
pub fn display_names(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in raw.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            ',' | ';' if !quoted => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
        .iter()
        .filter_map(|part| {
            let part = part.trim();
            let name = match part.split_once('<') {
                Some((name, address)) => {
                    let name = name.trim().trim_matches('"').trim();
                    if name.is_empty() {
                        address.trim_end_matches('>').trim()
                    } else {
                        name
                    }
                }
                None => part,
            };
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

/// Local-day window `[start, end)` in Unix seconds around `at`.
pub fn local_day_window(at: i64, tz: &TimeZone) -> Result<(i64, i64), String> {
    let day = local(at, tz)?.date();
    let start = day
        .to_zoned(tz.clone())
        .map_err(|e| format!("day start: {e}"))?;
    let end = day
        .tomorrow()
        .and_then(|next| next.to_zoned(tz.clone()))
        .map_err(|e| format!("day end: {e}"))?;
    Ok((start.timestamp().as_second(), end.timestamp().as_second()))
}

fn local(at: i64, tz: &TimeZone) -> Result<Zoned, String> {
    Timestamp::from_second(at)
        .map(|ts| ts.to_zoned(tz.clone()))
        .map_err(|e| format!("timestamp {at}: {e}"))
}

/// Whether each day event overlaps the invite (the invite itself and
/// free or all-day events never conflict).
pub fn conflicts(details: &InviteDetails) -> Vec<bool> {
    let invite = &details.event;
    details
        .day_events
        .iter()
        .map(|ev| {
            !is_invite_event(ev, invite)
                && !ev.all_day
                && ev.availability.as_deref() != Some("free")
                && ev.start_time < invite.end_time
                && invite.start_time < ev.end_time
        })
        .collect()
}

fn is_invite_event(ev: &TimeGridEvent, invite: &CalendarEvent) -> bool {
    ev.id == invite.id
        || (ev.start_time == invite.start_time
            && ev.end_time == invite.end_time
            && invite.summary.as_deref() == Some(ev.title.as_str()))
}

/// "Tuesday 3 November 2026, 14:00 - 15:30".
pub fn when_label(event: &CalendarEvent, tz: &TimeZone) -> String {
    let (Ok(start), Ok(end)) = (local(event.start_time, tz), local(event.end_time, tz)) else {
        return String::new();
    };
    let day = |at: &Zoned| at.strftime("%A %-d %B %Y").to_string();
    if event.is_all_day {
        return format!("{} (all day)", day(&start));
    }
    if start.date() == end.date() {
        format!(
            "{}, {} - {}",
            day(&start),
            start.strftime("%H:%M"),
            end.strftime("%H:%M")
        )
    } else {
        format!(
            "{} {} - {} {}",
            day(&start),
            start.strftime("%H:%M"),
            day(&end),
            end.strftime("%H:%M")
        )
    }
}

/// Propose-new-time fields prefilled from the invite's own slot.
fn slot_fields(event: &CalendarEvent, tz: &TimeZone) -> (String, String, String) {
    let (Ok(start), Ok(end)) = (local(event.start_time, tz), local(event.end_time, tz)) else {
        return (String::new(), String::new(), String::new());
    };
    (
        start.strftime("%Y-%m-%d").to_string(),
        start.strftime("%H:%M").to_string(),
        end.strftime("%H:%M").to_string(),
    )
}

/// Parse the propose-new-time fields into a Unix-second slot.
pub fn proposed_slot(
    date: &str,
    start: &str,
    end: &str,
    tz: &TimeZone,
) -> Result<(i64, i64), String> {
    let date: Date = date
        .trim()
        .parse()
        .map_err(|_| "Enter the date as YYYY-MM-DD".to_string())?;
    let at = |value: &str| -> Result<i64, String> {
        let time: Time = value
            .trim()
            .parse()
            .map_err(|_| "Enter times as HH:MM".to_string())?;
        date.to_datetime(time)
            .to_zoned(tz.clone())
            .map(|zoned| zoned.timestamp().as_second())
            .map_err(|e| format!("Invalid time: {e}"))
    };
    let (start, end) = (at(start)?, at(end)?);
    if start >= end {
        return Err("The new time must end after it starts".to_string());
    }
    Ok((start, end))
}

// ── View ────────────────────────────────────────────────

/// Invite banner for a message, or `None` when it carries no invite.
pub fn view<'a, M: Clone + 'a>(
    msg: &'a ThreadMessage,
    card: Option<&'a InviteCard>,
    on: impl Fn(InviteMessage) -> M + Clone + 'a,
) -> Option<Element<'a, M>> {
    let sender = msg
        .from_name
        .as_deref()
        .or(msg.from_address.as_deref())
        .unwrap_or("The organizer");
    match InviteKind::of(msg)? {
        InviteKind::Reply => Some(notice(
            icon::calendar(),
            format!("{sender} responded to this meeting"),
        )),
        InviteKind::Cancel => Some(notice(
            icon::x(),
            format!("{sender} cancelled this meeting"),
        )),
        InviteKind::Request => card.map(|card| request_card(msg, card, sender, on)),
    }
}

fn notice<'a, M: 'a>(ico: iced::widget::Text<'a>, label: String) -> Element<'a, M> {
    container(
        row![
            ico.size(ICON_XL).style(text::secondary),
            text(label).size(TEXT_MD).style(text::secondary),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .padding(PAD_CARD)
    .width(Length::Fill)
    .style(theme::ContainerClass::SettingsSection.style())
    .into()
}

fn request_card<'a, M: Clone + 'a>(
    msg: &'a ThreadMessage,
    card: &'a InviteCard,
    sender: &'a str,
    on: impl Fn(InviteMessage) -> M + Clone + 'a,
) -> Element<'a, M> {
    let tz = TimeZone::system();
    let details = card.details();
    let title = details
        .and_then(|d| d.event.summary.as_deref())
        .or(msg.subject.as_deref())
        .unwrap_or("Meeting");

    let mut info = column![
        row![
            icon::calendar().size(ICON_XL).style(text::primary),
            text(title)
                .size(TEXT_XL)
                .font(font::text_semibold())
                .style(text::base),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center)
    ]
    .spacing(SPACE_XXS);

    if let Some(details) = details {
        info = info.push(field("When", when_label(&details.event, &tz)));
        if let Some(location) = details.event.location.as_deref()
            && !location.trim().is_empty()
        {
            info = info.push(field("Where", location.to_string()));
        }
    }
    if let Some(url) = card.join_url.as_deref() {
        info = info.push(
            row![
                label_cell("Join"),
                button(
                    row![
                        icon::link().size(ICON_MD),
                        text("Join meeting").size(TEXT_MD)
                    ]
                    .spacing(SPACE_XXS)
                    .align_y(Alignment::Center),
                )
                .on_press(on(InviteMessage::Join(url.to_string())))
                .padding(PAD_ICON_BTN)
                .style(theme::ButtonClass::Primary.style()),
            ]
            .align_y(Alignment::Center),
        );
    }
    let organizer = details
        .and_then(|d| {
            d.event
                .organizer_name
                .as_deref()
                .or(d.event.organizer_email.as_deref())
        })
        .unwrap_or(sender);
    info = info.push(field("Organizer", organizer.to_string()));
    let required = display_names(msg.to_addresses.as_deref());
    if !required.is_empty() {
        info = info.push(field("Required", required.join(", ")));
    }
    let optional = display_names(msg.cc_addresses.as_deref());
    if !optional.is_empty() {
        info = info.push(field("Optional", optional.join(", ")));
    }

    match &card.lookup {
        InviteLookup::Loading => info = info.push(muted("Loading meeting details...")),
        InviteLookup::Unsynced => {
            info = info.push(muted(
                "This meeting is not in your synced calendar yet. \
                 Responding unlocks once the calendar syncs.",
            ));
        }
        InviteLookup::Failed(error) => {
            info = info.push(muted(format!("Could not load meeting details: {error}")));
        }
        InviteLookup::Found(_) => {}
    }

    let current = details.and_then(|d| d.event.rsvp_status.as_deref());
    if let Some(status) = current.and_then(status_label) {
        info = info.push(
            text(status)
                .size(TEXT_MD)
                .font(font::text_semibold())
                .style(text::success),
        );
    }

    if !msg.is_own_message {
        info = info.push(response_controls(card, current, on.clone()));
    }

    let mut body = row![container(info).width(Length::Fill)].spacing(SPACE_MD);
    if let Some(details) = details {
        body = body.push(day_agenda(details, &tz));
    }

    container(body)
        .padding(PAD_CARD)
        .width(Length::Fill)
        .style(theme::ContainerClass::SettingsSection.style())
        .into()
}

fn response_controls<'a, M: Clone + 'a>(
    card: &'a InviteCard,
    current: Option<&str>,
    on: impl Fn(InviteMessage) -> M + Clone + 'a,
) -> Element<'a, M> {
    let ready = card.details().is_some() && card.pending.is_none();
    let respond = |ico: iced::widget::Text<'a>, label: &'a str, response: RsvpResponse| {
        let chosen = current == Some(rsvp_value(response)) || card.pending == Some(response);
        let allowed = ready && !(card.proposing && response == RsvpResponse::Accepted);
        button(
            row![ico.size(ICON_LG), text(label).size(TEXT_LG)]
                .spacing(SPACE_XS)
                .align_y(Alignment::Center),
        )
        .on_press_maybe(allowed.then(|| on(InviteMessage::Respond(response))))
        .padding(PAD_BUTTON)
        .width(INVITE_RESPONSE_BUTTON_WIDTH)
        .style(if chosen {
            theme::ButtonClass::Primary.style()
        } else {
            theme::ButtonClass::Secondary.style()
        })
    };
    let mut controls = column![
        row![
            respond(icon::check(), "Accept", RsvpResponse::Accepted),
            respond(icon::help_circle(), "Tentative", RsvpResponse::Tentative),
            respond(icon::x(), "Decline", RsvpResponse::Declined),
        ]
        .spacing(SPACE_XS)
    ]
    .spacing(SPACE_XS)
    .padding(iced::Padding {
        top: SPACE_XS,
        ..iced::Padding::ZERO
    });

    if card.rich_reply && ready {
        let on_comment = on.clone();
        controls = controls.push(
            text_input("Add a message to the organizer (optional)", &card.comment)
                .on_input(move |value| on_comment(InviteMessage::CommentChanged(value)))
                .size(TEXT_MD)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style()),
        );
        let on_send = on.clone();
        controls = controls.push(
            row![
                animated_toggler(card.send_response)
                    .size(TEXT_HEADING)
                    .on_toggle(move |send| on_send(InviteMessage::SendResponseToggled(send)))
                    .style(theme::TogglerClass::Settings.style()),
                text("Send response to organizer")
                    .size(TEXT_MD)
                    .style(text::base),
                Space::new().width(Length::Fill),
                button(text("Propose new time").size(TEXT_MD))
                    .on_press_maybe(
                        card.send_response
                            .then(|| on(InviteMessage::ProposeToggled))
                    )
                    .padding(PAD_ICON_BTN)
                    .style(
                        theme::ButtonClass::Chip {
                            active: card.proposing
                        }
                        .style()
                    ),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center),
        );
        if card.proposing {
            controls = controls.push(propose_row(card, on.clone()));
        }
    }
    if let Some(error) = card.error.as_deref() {
        controls = controls.push(
            text(error)
                .size(TEXT_SM)
                .style(theme::TextClass::Warning.style()),
        );
    }
    controls.into()
}

fn propose_row<'a, M: Clone + 'a>(
    card: &'a InviteCard,
    on: impl Fn(InviteMessage) -> M + Clone + 'a,
) -> Element<'a, M> {
    let input =
        |placeholder: &'a str, value: &'a str, width: f32, make: fn(String) -> InviteMessage| {
            let on = on.clone();
            text_input(placeholder, value)
                .on_input(move |value| on(make(value)))
                .size(TEXT_MD)
                .padding(PAD_INPUT)
                .width(width)
                .style(theme::TextInputClass::Settings.style())
        };
    row![
        input(
            "YYYY-MM-DD",
            &card.propose_date,
            INVITE_DATE_INPUT_WIDTH,
            InviteMessage::ProposeDateChanged,
        ),
        input(
            "HH:MM",
            &card.propose_start,
            INVITE_TIME_INPUT_WIDTH,
            InviteMessage::ProposeStartChanged,
        ),
        text("-").size(TEXT_MD).style(text::secondary),
        input(
            "HH:MM",
            &card.propose_end,
            INVITE_TIME_INPUT_WIDTH,
            InviteMessage::ProposeEndChanged,
        ),
        muted("Sent with Tentative or Decline"),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center)
    .into()
}

fn day_agenda<'a, M: 'a>(details: &'a InviteDetails, tz: &TimeZone) -> Element<'a, M> {
    let flags = conflicts(details);
    let count = flags.iter().filter(|&&conflict| conflict).count();
    let day = local(details.event.start_time, tz)
        .map(|at| at.strftime("%A %-d %B").to_string())
        .unwrap_or_default();
    let summary = match count {
        0 => "No conflicts".to_string(),
        1 => "1 conflict".to_string(),
        n => format!("{n} conflicts"),
    };
    let mut agenda = column![
        text(day)
            .size(TEXT_MD)
            .font(font::text_semibold())
            .style(text::base),
        text(summary).size(TEXT_SM).style(if count == 0 {
            theme::TextClass::Tertiary.style()
        } else {
            theme::TextClass::Warning.style()
        }),
    ]
    .spacing(SPACE_XXS);
    for (ev, conflict) in details.day_events.iter().zip(flags) {
        let time = if ev.all_day {
            "All day".to_string()
        } else {
            match (local(ev.start_time, tz), local(ev.end_time, tz)) {
                (Ok(start), Ok(end)) => {
                    format!("{} - {}", start.strftime("%H:%M"), end.strftime("%H:%M"))
                }
                _ => String::new(),
            }
        };
        let style: fn(&iced::Theme) -> text::Style = if is_invite_event(ev, &details.event) {
            text::primary
        } else if conflict {
            theme::TextClass::Warning.style()
        } else {
            text::secondary
        };
        agenda = agenda.push(
            row![
                text(time)
                    .size(TEXT_SM)
                    .style(style)
                    .width(INVITE_AGENDA_TIME_WIDTH),
                text(ev.title.as_str())
                    .size(TEXT_SM)
                    .style(style)
                    .wrapping(text::Wrapping::None),
            ]
            .spacing(SPACE_XS),
        );
    }
    container(agenda)
        .padding(PAD_CARD)
        .width(INVITE_AGENDA_WIDTH)
        .style(theme::ContainerClass::Surface.style())
        .into()
}

fn status_label(status: &str) -> Option<&'static str> {
    match status {
        "accepted" => Some("You accepted this meeting"),
        "tentative" => Some("You tentatively accepted this meeting"),
        "declined" => Some("You declined this meeting"),
        _ => None,
    }
}

fn label_cell<'a, M: 'a>(label: &'a str) -> Element<'a, M> {
    container(
        text(label)
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    )
    .width(INVITE_LABEL_WIDTH)
    .into()
}

fn field<'a, M: 'a>(label: &'a str, value: String) -> Element<'a, M> {
    row![
        label_cell(label),
        text(value)
            .size(TEXT_MD)
            .style(text::base)
            .width(Length::Fill),
    ]
    .align_y(Alignment::Start)
    .into()
}

fn muted<'a, M: 'a>(label: impl text::IntoFragment<'a>) -> Element<'a, M> {
    text(label)
        .size(TEXT_SM)
        .style(theme::TextClass::Muted.style())
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paris() -> TimeZone {
        TimeZone::get("Europe/Paris").expect("tz database")
    }

    fn event(start: i64, end: i64) -> CalendarEvent {
        CalendarEvent {
            id: "invite".to_string(),
            summary: Some("Demo".to_string()),
            description: None,
            location: None,
            start_time: start,
            end_time: end,
            is_all_day: false,
            calendar_id: None,
            account_id: "acct".to_string(),
            timezone: None,
            recurrence_rule: None,
            organizer_name: None,
            organizer_email: None,
            rsvp_status: None,
            availability: None,
            visibility: None,
        }
    }

    fn day_event(id: &str, start: i64, end: i64, availability: Option<&str>) -> TimeGridEvent {
        TimeGridEvent {
            id: id.to_string(),
            title: id.to_string(),
            start_time: start,
            end_time: end,
            all_day: false,
            color: String::new(),
            calendar_name: None,
            location: None,
            recurrence_rule: None,
            calendar_id: None,
            account_id: "acct".to_string(),
            organizer_name: None,
            organizer_email: None,
            rsvp_status: None,
            description: None,
            availability: availability.map(str::to_string),
            visibility: None,
            timezone: None,
        }
    }

    #[test]
    fn join_url_finds_teams_link_in_html() {
        let html = r#"<a href="https://teams.microsoft.com/l/meetup-join/19%3ameeting?context=%7b%7d&amp;x=1">Join</a>"#;
        assert_eq!(
            join_url(&[None, Some(html)]).as_deref(),
            Some("https://teams.microsoft.com/l/meetup-join/19%3ameeting?context=%7b%7d&x=1")
        );
        assert_eq!(
            join_url(&[Some("Call: https://us02web.zoom.us/j/123456789.")]).as_deref(),
            Some("https://us02web.zoom.us/j/123456789")
        );
        assert_eq!(join_url(&[Some("https://example.com/agenda")]), None);
    }

    #[test]
    fn display_names_keep_quoted_commas() {
        assert_eq!(
            display_names(Some(r#""Pinelli, Antoine" <a@b.fr>, <c@d.fr>; e@f.fr"#)),
            vec!["Pinelli, Antoine", "c@d.fr", "e@f.fr"]
        );
        assert!(display_names(None).is_empty());
    }

    #[test]
    fn conflicts_skip_the_invite_free_and_disjoint_events() {
        let details = InviteDetails {
            event: event(1_000, 2_000),
            day_events: vec![
                day_event("invite", 1_000, 2_000, None),
                day_event("overlap", 1_500, 2_500, Some("busy")),
                day_event("free", 1_500, 2_500, Some("free")),
                day_event("after", 2_000, 3_000, None),
            ],
        };
        assert_eq!(conflicts(&details), vec![false, true, false, false]);
    }

    #[test]
    fn proposed_slot_reads_local_time() {
        let (start, end) = proposed_slot("2026-11-02", "14:00", "15:30", &paris()).unwrap();
        assert_eq!(start, 1_793_624_400);
        assert_eq!(end - start, 5_400);
        assert!(proposed_slot("2026-11-02", "15:00", "14:00", &paris()).is_err());
        assert!(proposed_slot("02/11/2026", "14:00", "15:00", &paris()).is_err());
    }

    #[test]
    fn when_label_formats_same_day_slot() {
        let label = when_label(&event(1_793_624_400, 1_793_629_800), &paris());
        assert_eq!(label, "Monday 2 November 2026, 14:00 - 15:30");
    }

    #[test]
    fn local_day_window_spans_the_local_day() {
        let (start, end) = local_day_window(1_793_624_400, &paris()).unwrap();
        assert_eq!(end - start, 86_400);
        assert!(start <= 1_793_624_400 && 1_793_624_400 < end);
    }

    #[test]
    fn respond_builds_reply_and_blocks_accept_with_proposal() {
        let mut card = InviteCard {
            lookup: InviteLookup::Found(Box::new(InviteDetails {
                event: event(1_793_624_400, 1_793_628_000),
                day_events: Vec::new(),
            })),
            rich_reply: true,
            join_url: None,
            comment: "  See you  ".to_string(),
            send_response: true,
            proposing: false,
            propose_date: String::new(),
            propose_start: String::new(),
            propose_end: String::new(),
            pending: None,
            error: None,
        };
        let tz = paris();
        card.update(InviteMessage::ProposeToggled, &tz);
        assert_eq!(card.propose_date, "2026-11-02");
        assert_eq!(card.propose_start, "14:00");
        assert!(
            card.update(InviteMessage::Respond(RsvpResponse::Accepted), &tz)
                .is_none()
        );
        assert!(card.error.is_some());
        match card.update(InviteMessage::Respond(RsvpResponse::Tentative), &tz) {
            Some(InviteAction::Respond {
                event_id, reply, ..
            }) => {
                assert_eq!(event_id, "invite");
                assert_eq!(reply.comment.as_deref(), Some("See you"));
                assert!(reply.send_response);
                assert_eq!(reply.proposed_start, Some(1_793_624_400));
            }
            other => panic!("expected a response, got {other:?}"),
        }
        card.finish(Ok(()));
        assert_eq!(
            card.details().and_then(|d| d.event.rsvp_status.as_deref()),
            Some("tentative")
        );
        assert!(card.comment.is_empty());
    }
}
