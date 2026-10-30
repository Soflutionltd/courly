//! The side panel: open, close and switch modules on the user's word only,
//! keep My Day's events loaded while it shows, and manage templates.

use std::sync::Arc;

use iced::Task;
use jiff::Zoned;
use rust_i18n::t;

use crate::app::ReadyApp;
use crate::message::Message;
use crate::pop_out::compose::ComposeMessage;
use crate::pop_out::{PopOutMessage, PopOutWindow};
use crate::ui::calendar::{CalendarEventData, CalendarMessage};
use crate::ui::side_panel::SidePanelMessage;
use crate::ui::side_panel::my_day::MyDayMessage;
use crate::ui::side_panel::templates::{Template, TemplatesMessage};

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
            SidePanelMessage::Templates(message) => self.handle_templates(message),
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
        Task::batch([self.reload_my_day(), self.reload_templates()])
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

    /// Loads the templates when their list is on screen; nothing otherwise.
    fn reload_templates(&self) -> Task<Message> {
        if !self.side_panel.shows_templates() {
            return Task::none();
        }
        let db = Arc::clone(&self.db);
        Task::perform(
            async move {
                let rows =
                    rtsk::db::queries_extra::db_get_all_templates(&db.read_db_state()).await?;
                Ok(rows
                    .into_iter()
                    .map(|row| Template::new(row.id, row.name, row.subject, row.body_html))
                    .collect())
            },
            |result| templates_message(TemplatesMessage::Loaded(result)),
        )
    }

    fn handle_templates(&mut self, message: TemplatesMessage) -> Task<Message> {
        let templates = &mut self.side_panel.templates;
        match message {
            TemplatesMessage::Loaded(Ok(list)) => {
                templates.templates = list;
                Task::none()
            }
            TemplatesMessage::Loaded(Err(error)) => {
                log::error!("Templates: loading failed: {error}");
                self.status =
                    t!("main.side_panel.templates.load_failed", error = error).into_owned();
                Task::none()
            }
            TemplatesMessage::Query(query) => {
                templates.query = query;
                Task::none()
            }
            TemplatesMessage::New => {
                templates.start_new();
                Task::none()
            }
            TemplatesMessage::Edit(id) => {
                templates.start_edit(&id);
                Task::none()
            }
            TemplatesMessage::Name(name) => {
                if let Some(editor) = templates.editor.as_mut() {
                    editor.name = name;
                }
                Task::none()
            }
            TemplatesMessage::Subject(subject) => {
                if let Some(editor) = templates.editor.as_mut() {
                    editor.subject = subject;
                }
                Task::none()
            }
            TemplatesMessage::Body(action) => {
                if let Some(editor) = templates.editor.as_mut() {
                    editor.body.perform(action);
                }
                Task::none()
            }
            TemplatesMessage::Cancel => {
                templates.editor = None;
                Task::none()
            }
            TemplatesMessage::Save => self.save_template(),
            TemplatesMessage::Saved(Ok(_)) => {
                templates.editor = None;
                self.reload_templates()
            }
            TemplatesMessage::Saved(Err(error)) => {
                if let Some(editor) = templates.editor.as_mut() {
                    editor.saving = false;
                }
                log::error!("Templates: saving failed: {error}");
                self.status =
                    t!("main.side_panel.templates.save_failed", error = error).into_owned();
                Task::none()
            }
            TemplatesMessage::Delete(id) => {
                templates.confirm_delete = Some(id);
                Task::none()
            }
            TemplatesMessage::CancelDelete => {
                templates.confirm_delete = None;
                Task::none()
            }
            TemplatesMessage::ConfirmDelete(id) => {
                templates.confirm_delete = None;
                self.delete_template(id)
            }
            TemplatesMessage::Deleted(Ok(())) => self.reload_templates(),
            TemplatesMessage::Deleted(Err(error)) => {
                log::error!("Templates: deleting failed: {error}");
                self.status =
                    t!("main.side_panel.templates.delete_failed", error = error).into_owned();
                self.reload_templates()
            }
            TemplatesMessage::Insert(id) => self.insert_template(&id),
        }
    }

    fn save_template(&mut self) -> Task<Message> {
        let Some(editor) = self.side_panel.templates.editor.as_mut() else {
            return Task::none();
        };
        let Some(client) = self.service_client.clone() else {
            return Task::done(templates_message(TemplatesMessage::Saved(Err(t!(
                "main.side_panel.templates.service_unavailable"
            )
            .into_owned()))));
        };
        editor.saving = true;
        let params = service_api::TemplateSaveParams {
            id: editor.id.clone(),
            name: editor.name.clone(),
            subject: Some(editor.subject.clone()),
            body_html: editor.body.to_html(),
        };
        Task::perform(
            async move {
                client
                    .save_template(params)
                    .await
                    .map_err(|e| e.to_string())
            },
            |result| templates_message(TemplatesMessage::Saved(result)),
        )
    }

    fn delete_template(&self, id: String) -> Task<Message> {
        let Some(client) = self.service_client.clone() else {
            return Task::done(templates_message(TemplatesMessage::Deleted(Err(t!(
                "main.side_panel.templates.service_unavailable"
            )
            .into_owned()))));
        };
        Task::perform(
            async move { client.delete_template(id).await.map_err(|e| e.to_string()) },
            |result| templates_message(TemplatesMessage::Deleted(result)),
        )
    }

    /// Puts a template into the message being written: the reply open
    /// beside the mail, else the compose window used last, else a new
    /// message. Its subject only fills an empty subject.
    fn insert_template(&mut self, id: &str) -> Task<Message> {
        let Some(template) = self.side_panel.templates.find(id).cloned() else {
            return Task::none();
        };
        let Some(window_id) = self.template_target() else {
            return self
                .open_compose_from_template(template.subject.as_deref(), &template.body_html);
        };
        let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) else {
            return Task::none();
        };
        state.body.insert_html(&template.body_html);
        state.draft_dirty = true;
        if state.subject.trim().is_empty()
            && let Some(subject) = template.subject
        {
            state.subject = subject;
        }
        let body_focus = Task::done(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::BodyChanged(rte::Action::Focus)),
        ));
        if self.inline_compose == Some(window_id) {
            body_focus
        } else {
            Task::batch([iced::window::gain_focus(window_id), body_focus])
        }
    }

    fn template_target(&self) -> Option<iced::window::Id> {
        if let Some((id, _)) = self.inline_compose_for_selected_thread() {
            return Some(id);
        }
        self.last_compose_window
            .filter(|id| matches!(self.pop_out_windows.get(id), Some(PopOutWindow::Compose(_))))
    }
}

fn templates_message(message: TemplatesMessage) -> Message {
    Message::SidePanel(SidePanelMessage::Templates(message))
}
