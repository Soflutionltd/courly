use iced::Task;
use service_api::SettingValue;

use crate::app::ReadyApp;
use crate::message::Message;
use crate::ui::toolbar::{ToolbarEvent, ToolbarMessage};

impl ReadyApp {
    pub(crate) fn handle_toolbar(&mut self, message: ToolbarMessage) -> Task<Message> {
        match self.toolbar.update(message) {
            Some(ToolbarEvent::Execute(id)) => self.handle_execute_command(id),
            Some(ToolbarEvent::LayoutChanged(json)) => self.persist_toolbar_layout(json),
            None => Task::none(),
        }
    }

    fn persist_toolbar_layout(&self, json: String) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            log::warn!("settings.set: no ServiceClient yet; toolbar layout not persisted");
            return Task::none();
        };
        Task::perform(
            async move {
                client
                    .set_settings(vec![SettingValue::ToolbarLayout(json)])
                    .await
            },
            |result| {
                if let Err(e) = result {
                    log::warn!("settings.set toolbar_layout failed: {e}");
                }
                Message::Noop
            },
        )
    }
}
