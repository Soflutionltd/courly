//! Pop-out window handler methods for `App`.
//!
//! All pop-out window logic lives here. `main.rs` dispatches to these methods
//! via one-line match arms.

use std::time::Duration;

pub const DRAFT_AUTO_SAVE_INTERVAL: Duration = Duration::from_secs(30);

mod compose_clipboard;
mod compose_draft;
mod compose_send;
mod compose_signature;
mod dispatcher;
mod message_view;
mod save_as;
mod session;
mod window_lifecycle;

use iced::Task;

use crate::pop_out::PopOutWindow;
use crate::{Message, ReadyApp};

impl ReadyApp {
    /// Re-render the HTML shown in pop-out windows on a new page appearance.
    pub(crate) fn set_pop_out_appearance(
        &mut self,
        appearance: mail_html::Appearance,
    ) -> Task<Message> {
        Task::batch(
            self.pop_out_windows
                .iter_mut()
                .map(|(&window_id, window)| match window {
                    PopOutWindow::MessageView(state) => {
                        message_view::set_message_view_appearance(window_id, state, appearance)
                    }
                    PopOutWindow::Compose(state) => {
                        dispatcher::set_compose_preview_appearance(window_id, state, appearance)
                    }
                    PopOutWindow::Calendar(_) => Task::none(),
                })
                .collect::<Vec<_>>(),
        )
    }
}
