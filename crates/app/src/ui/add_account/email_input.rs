use std::sync::Arc;

use iced::widget::{Space, column, container, svg, text, text_input};
use iced::{Alignment, Element, Length, Task};
use rust_i18n::t;

use crate::font;
use crate::ui::layout::*;
use crate::ui::theme;

use rtsk::db::queries_extra::account_exists_by_email_sync;

use super::state::{AddAccountEvent, AddAccountMessage, AddAccountStep, AddAccountWizard};
use super::views::{ghost_button, primary_button};

impl AddAccountWizard {
    pub(super) fn handle_submit_email(
        &mut self,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        let email = self.email.trim().to_lowercase();
        if email.is_empty() || !email.contains('@') {
            self.error = Some(t!("account_setup.error.invalid_email").into_owned());
            return (Task::none(), None);
        }
        self.email = email.clone();
        self.step = AddAccountStep::Discovering;
        self.error = None;
        let generation = self.generation.next();
        let db = Arc::clone(&self.db);

        let task = Task::perform(
            async move {
                // Duplicate check - run synchronously inside spawn_blocking
                let email_for_dup = email.clone();
                let dup = db
                    .with_read(move |conn| account_exists_by_email_sync(conn, &email_for_dup))
                    .await;
                match dup {
                    Ok(true) => {
                        return (
                            generation,
                            Err(t!("account_setup.error.already_configured").into_owned()),
                        );
                    }
                    Err(e) => {
                        return (
                            generation,
                            Err(t!("account_setup.error.database", error = e).into_owned()),
                        );
                    }
                    Ok(false) => {}
                }

                // Run real discovery with 15s timeout
                let result = rtsk::discovery::discover(&email).await;
                (generation, result)
            },
            |(g, result)| AddAccountMessage::DiscoveryComplete(g, result),
        );
        (task, None)
    }

    pub(super) fn prefill_from_email(&mut self) {
        if self.auth_state.username.is_empty() {
            self.auth_state.username = self.email.clone();
        }
        let domain = self.email.split('@').nth(1).unwrap_or("");
        if self.auth_state.imap_host.is_empty() {
            self.auth_state.imap_host = format!("imap.{domain}");
        }
        if self.auth_state.smtp_host.is_empty() {
            self.auth_state.smtp_host = format!("smtp.{domain}");
        }
    }

    pub(super) fn view_email_input(&self) -> Element<'_, AddAccountMessage> {
        let mut col = column![]
            .spacing(SPACE_LG)
            .align_x(Alignment::Center)
            .width(Length::Fill);

        let logo_handle =
            svg::Handle::from_memory(include_bytes!("../../../../../assets/icon.svg"));
        col = col.push(
            container(
                svg(logo_handle)
                    .width(Length::Fixed(WELCOME_ICON_SIZE))
                    .height(Length::Fixed(WELCOME_ICON_SIZE))
                    .content_fit(iced::ContentFit::Contain),
            )
            .width(Length::Fixed(WELCOME_ICON_SIZE))
            .height(Length::Fixed(WELCOME_ICON_SIZE)),
        );
        col = col.push(Space::new().height(SPACE_SM));

        if self.is_first_launch {
            col = col.push(
                text(t!("account_setup.welcome_title"))
                    .size(TEXT_HEADING)
                    .style(text::base)
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..font::text()
                    }),
            );
            col = col.push(
                text(t!("account_setup.welcome_subtitle"))
                    .size(TEXT_LG)
                    .style(text::secondary),
            );
        } else {
            col = col.push(
                text(t!("account_setup.add_account_title"))
                    .size(TEXT_HEADING)
                    .style(text::base)
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..font::text()
                    }),
            );
        }

        col = col.push(Space::new().height(SPACE_SM));

        col = col.push(
            text_input("alice@example.com", &self.email)
                .on_input(AddAccountMessage::EmailChanged)
                .on_submit(AddAccountMessage::SubmitEmail)
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style())
                .width(Length::Fill),
        );

        if let Some(ref err) = self.error {
            col = col.push(text(err.as_str()).size(TEXT_SM).style(text::danger));
        }

        col = col.push(primary_button(
            t!("account_setup.continue"),
            AddAccountMessage::SubmitEmail,
        ));

        if !self.is_first_launch {
            col = col.push(ghost_button(
                t!("account_setup.cancel"),
                AddAccountMessage::Cancel,
            ));
        }

        col.width(Length::Fill).into()
    }
}
