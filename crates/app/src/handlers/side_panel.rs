//! The side panel: open, close and switch modules on the user's word only,
//! and keep My Day's events loaded while it shows.

use std::sync::Arc;

use iced::Task;
use jiff::Zoned;
use rust_i18n::t;

use crate::app::ReadyApp;
use crate::message::Message;
use crate::ui::calendar::{CalendarEventData, CalendarMessage};
use crate::ui::side_panel::SidePanelMessage;
use crate::ui::side_panel::my_day::MyDayMessage;

/// Hour a new event from My Day starts at on a day other than today.
const NEW_EVENT_DEFAULT_HOUR: u32 = 9;

impl ReadyApp {
    pub(crate) fn handle_side_panel(&mut self, message: SidePanelMessage) -> Task<Message> {
        match message {
            SidePanelMessage::Toggle => self.set_side_panel(!self.side_panel.open),
            SidePanelMessage::Close => self.set_side_panel(false),
            SidePanelMessage::Show(module) => {
                self.side_panel.module = module;
                self.set_side_panel(true)
            }
            SidePanelMessage::MyDay(message) => self.handle_my_day(message),
        }
    }

    fn set_side_panel(&mut self, open: bool) -> Task<Message> {
        self.side_panel.open = open;
        self.clamp_panel_widths();
        self.window.side_panel_open = open;
        self.window.side_panel_module = self.side_panel.module;
        if let Some(data_dir) = crate::APP_DATA_DIR.get() {
            self.window.save(data_dir);
        }
        self.reload_my_day()
    }

    /// Loads My Day's events when it is on screen; nothing otherwise.
    pub(crate) fn reload_my_day(&mut self) -> Task<Message> {
        if !self.side_panel.shows_my_day() {
            return Task::none();
        }
        let (generation, from, to) = self.side_panel.my_day.begin_load(self.calendar.week_start);
        let db = Arc::clone(&self.db);
        Task::perform(
            async move { db.load_calendar_events_for_view(from, to).await },
            move |result| {
                Message::SidePanel(SidePanelMessage::MyDay(MyDayMessage::Loaded(
                    generation, result,
                )))
            },
        )
    }

    fn handle_my_day(&mut self, message: MyDayMessage) -> Task<Message> {
        let my_day = &mut self.side_panel.my_day;
        match message {
            MyDayMessage::PrevMonth => {
                my_day.shift_month(-1);
                self.reload_my_day()
            }
            MyDayMessage::NextMonth => {
                my_day.shift_month(1);
                self.reload_my_day()
            }
            MyDayMessage::SelectDate(date) => {
                my_day.select(date);
                self.reload_my_day()
            }
            MyDayMessage::Today => {
                let today = my_day.today;
                my_day.select(today);
                self.reload_my_day()
            }
            MyDayMessage::ToggleMonth => {
                my_day.unfolded = !my_day.unfolded;
                let start = my_day.start;
                my_day.select(start);
                self.reload_my_day()
            }
            MyDayMessage::NewEvent => {
                let hour = if my_day.start == my_day.today {
                    (u32::from(Zoned::now().hour().unsigned_abs()) + 1).min(23)
                } else {
                    NEW_EVENT_DEFAULT_HOUR
                };
                let event = CalendarEventData::new_at(my_day.start, hour);
                self.handle_calendar(CalendarMessage::OpenEventEditor(Some(event)))
            }
            MyDayMessage::OpenEvent(event_id) => {
                self.handle_calendar(CalendarMessage::EventClicked(event_id))
            }
            MyDayMessage::Loaded(generation, Ok(events)) => {
                my_day.finish_load(generation, events);
                Task::none()
            }
            MyDayMessage::Loaded(generation, Err(error)) => {
                if my_day.is_current(generation) {
                    log::error!("My Day: loading events failed: {error}");
                    self.status = t!("calendar.my_day.load_failed", error = error).into_owned();
                }
                Task::none()
            }
            MyDayMessage::Tick => {
                if my_day.roll_over(Zoned::now().date()) {
                    self.reload_my_day()
                } else {
                    Task::none()
                }
            }
        }
    }
}
