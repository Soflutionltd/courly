//! Floating card that offers, downloads and installs app updates.

use iced::widget::{Space, button, column, container, opaque, progress_bar, row, text};
use iced::{Alignment, Element, Length};
use rust_i18n::t;

use crate::ui::layout::{
    PAD_BUTTON, PAD_CARD, SPACE_SM, SPACE_XS, TEXT_LG, TEXT_SM, UPDATE_BANNER_BOTTOM_OFFSET,
    UPDATE_BANNER_WIDTH, UPDATE_PROGRESS_HEIGHT,
};
use crate::ui::theme;
use crate::updater::{AUTO_RETRIES, Phase, UpdaterMessage, UpdaterState};

/// The card positioned bottom-center over the window, or `None` when
/// there is nothing to show.
pub fn view(state: &UpdaterState) -> Option<Element<'_, UpdaterMessage>> {
    let content: Element<'_, UpdaterMessage> = match &state.phase {
        Phase::Idle => return None,
        Phase::Available(info) => {
            let mut body = column![title(
                t!("updater.available", version = info.version).into_owned()
            )]
            .spacing(SPACE_XS);
            if !info.notes.trim().is_empty() {
                body = body.push(muted(info.notes.trim().to_string()));
            }
            body.push(actions(
                Some((t!("updater.later").into_owned(), UpdaterMessage::Later)),
                (t!("updater.update").into_owned(), UpdaterMessage::Install),
            ))
            .into()
        }
        Phase::Downloading {
            info,
            downloaded,
            total,
        } => {
            let fraction = total
                .filter(|total| *total > 0)
                .map_or(0.0, |total| *downloaded as f32 / total as f32);
            column![
                title(t!("updater.downloading", version = info.version).into_owned()),
                progress_bar(0.0..=1.0, fraction.min(1.0)).girth(UPDATE_PROGRESS_HEIGHT),
                muted(
                    match total {
                        Some(total) => t!(
                            "updater.downloaded_of_total",
                            downloaded = format!("{:.1}", *downloaded as f64 / 1_048_576.0),
                            total = format!("{:.1}", *total as f64 / 1_048_576.0)
                        ),
                        None => t!(
                            "updater.downloaded",
                            downloaded = format!("{:.1}", *downloaded as f64 / 1_048_576.0)
                        ),
                    }
                    .into_owned()
                ),
            ]
            .spacing(SPACE_XS)
            .into()
        }
        Phase::Installing(info) => column![
            title(t!("updater.installing", version = info.version).into_owned()),
            muted(t!("updater.installing_detail").into_owned()),
        ]
        .spacing(SPACE_XS)
        .into(),
        Phase::Restarting(info) => column![
            title(t!("updater.installed", version = info.version).into_owned()),
            muted(t!("updater.restarting").into_owned()),
        ]
        .spacing(SPACE_XS)
        .into(),
        Phase::Installed(info) => column![
            title(t!("updater.installed", version = info.version).into_owned()),
            muted(t!("updater.quit_and_reopen").into_owned()),
            actions(
                None,
                (t!("updater.quit").into_owned(), UpdaterMessage::Quit)
            ),
        ]
        .spacing(SPACE_XS)
        .into(),
        Phase::Failed { info, error } => {
            let mut body = column![
                title(t!("updater.install_failed", version = info.version).into_owned()),
                muted(error.clone()),
            ]
            .spacing(SPACE_XS);
            if state.attempts <= AUTO_RETRIES {
                body = body.push(muted(t!("updater.retrying_soon").into_owned()));
            } else {
                body = body.push(actions(
                    Some((t!("updater.later").into_owned(), UpdaterMessage::Later)),
                    (t!("updater.retry").into_owned(), UpdaterMessage::Retry),
                ));
            }
            body.into()
        }
    };

    let card = container(content)
        .width(Length::Fixed(UPDATE_BANNER_WIDTH))
        .padding(PAD_CARD)
        .style(theme::ContainerClass::DialogCard.style());

    Some(
        container(opaque(card))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::End)
            .padding(iced::Padding {
                bottom: UPDATE_BANNER_BOTTOM_OFFSET,
                ..iced::Padding::ZERO
            })
            .into(),
    )
}

fn title<'a>(label: String) -> Element<'a, UpdaterMessage> {
    text(label)
        .size(TEXT_LG)
        .font(crate::font::text_semibold())
        .into()
}

fn muted<'a>(label: String) -> Element<'a, UpdaterMessage> {
    text(label)
        .size(TEXT_SM)
        .style(theme::TextClass::Muted.style())
        .into()
}

fn actions<'a>(
    secondary: Option<(String, UpdaterMessage)>,
    primary: (String, UpdaterMessage),
) -> Element<'a, UpdaterMessage> {
    let mut buttons = row![Space::new().width(Length::Fill)]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center);
    if let Some((label, message)) = secondary {
        buttons = buttons.push(
            button(text(label).size(TEXT_SM))
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Secondary.style())
                .on_press(message),
        );
    }
    buttons
        .push(
            button(text(primary.0).size(TEXT_SM))
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Primary.style())
                .on_press(primary.1),
        )
        .into()
}
