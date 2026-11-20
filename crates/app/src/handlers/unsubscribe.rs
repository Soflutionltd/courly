//! Leaving a mailing list: the reading pane says how, this carries it out.

use iced::Task;
use rust_i18n::t;

use crate::app::ReadyApp;
use crate::message::Message;
use crate::unsubscribe::{UnsubscribeMethod, UnsubscribeStatus};

impl ReadyApp {
    /// Carry out leaving a list the way its message offers.
    pub(crate) fn unsubscribe(
        &mut self,
        message_id: String,
        account_id: &str,
        method: UnsubscribeMethod,
    ) -> Task<Message> {
        match method {
            UnsubscribeMethod::OneClick { url } => {
                Task::perform(crate::unsubscribe::post_one_click(url), move |result| {
                    Message::UnsubscribeFinished {
                        message_id: message_id.clone(),
                        result,
                    }
                })
            }
            UnsubscribeMethod::Mail { to, subject, body } => {
                self.send_unsubscribe_mail(message_id, account_id, to, subject, body)
            }
            UnsubscribeMethod::Page { url } => {
                let email = self
                    .sidebar
                    .accounts
                    .iter()
                    .find(|account| account.id == account_id)
                    .map(|account| account.email.clone());
                Task::perform(crate::unsubscribe::visit_page(url, email), move |result| {
                    Message::UnsubscribeFinished {
                        message_id: message_id.clone(),
                        result,
                    }
                })
            }
        }
    }

    /// The one-click POST or the background page visit answered, or failed to.
    pub(crate) fn handle_unsubscribe_finished(
        &mut self,
        message_id: &str,
        result: Result<UnsubscribeStatus, String>,
    ) -> Task<Message> {
        let status = match result {
            Ok(status) => status,
            Err(error) => {
                log::warn!("unsubscribe failed: {error}");
                UnsubscribeStatus::Failed(error)
            }
        };
        self.finish_unsubscribe(message_id, status);
        Task::none()
    }

    /// Settle leaving a list in the pane and the status bar.
    pub(crate) fn finish_unsubscribe(&mut self, message_id: &str, status: UnsubscribeStatus) {
        let toast = match &status {
            UnsubscribeStatus::Done => Some(t!("main.action.unsubscribed")),
            UnsubscribeStatus::Requested => Some(t!("main.reading_pane.unsubscribe_requested")),
            UnsubscribeStatus::Failed(error) => {
                Some(t!("main.reading_pane.unsubscribe_failed", error = error))
            }
            UnsubscribeStatus::Ready | UnsubscribeStatus::Working => None,
        };
        if let Some(toast) = toast {
            self.status_bar.show_confirmation(toast.into_owned());
        }
        self.reading_pane.finish_unsubscribe(message_id, status);
    }

    /// Send the RFC 2369 `mailto:` message from the account that received
    /// the newsletter. It goes out right away, without the undo delay.
    fn send_unsubscribe_mail(
        &mut self,
        message_id: String,
        account_id: &str,
        to: String,
        subject: String,
        body: String,
    ) -> Task<Message> {
        let Some(account) = self
            .sidebar
            .accounts
            .iter()
            .find(|account| account.id == account_id)
        else {
            self.finish_unsubscribe(
                &message_id,
                UnsubscribeStatus::Failed(
                    t!("main.reading_pane.unsubscribe_no_account").into_owned(),
                ),
            );
            return Task::none();
        };
        let from = match account.display_name.as_deref() {
            Some(name) => format!("{name} <{}>", account.email),
            None => account.email.clone(),
        };
        let request = service_api::actions::SendRequest {
            draft_id: uuid::Uuid::new_v4().to_string(),
            account_id: account.id.clone(),
            from,
            to: vec![to],
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: Some(subject),
            body_html: format!("<p>{}</p>", escape_html(&body)),
            body_text: body,
            attachments: Vec::new(),
            in_reply_to: None,
            references: None,
            thread_id: None,
            source_message_id: None,
            intent: service_api::actions::SendIntent::New,
            request_read_receipt: false,
        };
        self.dispatch_unsubscribe_send(message_id, request)
    }
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
