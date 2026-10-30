use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length, Task};

use crate::font;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;

use rtsk::discovery::types::{DiscoveredConfig, DiscoverySource, Protocol, ProtocolOption};

use super::state::{AddAccountEvent, AddAccountMessage, AddAccountStep, AddAccountWizard};
use super::views::{ghost_button, primary_button};

/// The option to sign in with right away, skipping the provider choice.
/// A provider API (Gmail, Microsoft Graph) only comes from a known-provider
/// match, even through the MX record: like Outlook, go straight to its
/// sign-in without offering IMAP beside it.
fn auto_option(config: &DiscoveredConfig) -> Option<usize> {
    let provider_api = config.options.iter().position(|option| {
        matches!(
            option.protocol,
            Protocol::GmailApi | Protocol::MicrosoftGraph
        )
    });
    provider_api.or_else(|| {
        (config.options.len() == 1 && config.options[0].source.is_high_confidence()).then_some(0)
    })
}

impl AddAccountWizard {
    pub(super) fn handle_discovery_result(
        &mut self,
        config: &DiscoveredConfig,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        if config.options.is_empty() {
            self.error = Some("We couldn't auto-detect your mail server.".to_string());
            self.step = AddAccountStep::ManualConfiguration;
            return (Task::none(), None);
        }

        self.discovery = Some(config.clone());

        if let Some(index) = auto_option(config) {
            self.selected_option = Some(index);
            return self.proceed_to_auth(&config.options[index]);
        }

        // Multiple options or lower confidence: show selection
        self.selected_option = Some(0);
        self.step = AddAccountStep::ProtocolSelection;
        (Task::none(), None)
    }

    pub(super) fn handle_confirm_protocol(
        &mut self,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        let config = match &self.discovery {
            Some(c) => c.clone(),
            None => return (Task::none(), None),
        };
        let idx = self.selected_option.unwrap_or(0);
        let Some(option) = config.options.get(idx) else {
            return (Task::none(), None);
        };
        self.proceed_to_auth(option)
    }

    pub(super) fn view_protocol_selection(&self) -> Element<'_, AddAccountMessage> {
        let config = match &self.discovery {
            Some(c) => c,
            None => return column![].into(),
        };

        let mut col = column![
            text("Choose your email provider")
                .size(TEXT_HEADING)
                .style(text::base)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..font::text()
                }),
            Space::new().height(SPACE_XS),
            text(&self.email).size(TEXT_LG).style(text::secondary),
        ]
        .spacing(SPACE_XS)
        .width(Length::Fill);

        col = col.push(Space::new().height(SPACE_MD));

        for (i, option) in config.options.iter().enumerate() {
            let selected = self.selected_option == Some(i);
            col = col.push(protocol_card_view(option, i, selected));
        }

        col = col.push(Space::new().height(SPACE_MD));
        col = col.push(primary_button(
            "Continue",
            AddAccountMessage::ConfirmProtocol,
        ));
        col = col.push(ghost_button("Back", AddAccountMessage::Back));

        col.into()
    }
}

fn protocol_card_view(
    option: &ProtocolOption,
    index: usize,
    selected: bool,
) -> Element<'_, AddAccountMessage> {
    let name = protocol_display_name(&option.protocol, option.provider_name.as_deref());
    let detail = protocol_detail(&option.protocol);
    let source_label = source_display(&option.source);

    let content = row![
        container(
            column![
                text(name).size(TEXT_LG).style(text::base).font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..font::text()
                }),
                text(detail).size(TEXT_SM).style(text::secondary),
            ]
            .spacing(SPACE_XXXS),
        )
        .width(Length::Fill)
        .align_y(Alignment::Center),
        container(text(source_label).size(TEXT_XS).style(text::secondary))
            .align_y(Alignment::Center),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Center);

    let style = if selected {
        theme::ButtonClass::ProtocolCardSelected
    } else {
        theme::ButtonClass::ProtocolCard
    };

    button(
        container(content)
            .padding(PAD_CARD)
            .width(Length::Fill)
            .height(PROTOCOL_CARD_HEIGHT),
    )
    .on_press(AddAccountMessage::SelectProtocol(index))
    .padding(0)
    .style(style.style())
    .width(Length::Fill)
    .into()
}

pub(super) fn view_discovering<'a>() -> Element<'a, AddAccountMessage> {
    column![
        text("Looking up your email provider...")
            .size(TEXT_LG)
            .style(text::secondary),
        Space::new().height(SPACE_MD),
        widgets::spinner(24.0),
        Space::new().height(SPACE_LG),
        ghost_button("Cancel", AddAccountMessage::Cancel),
    ]
    .spacing(SPACE_XS)
    .align_x(Alignment::Center)
    .width(Length::Fill)
    .into()
}

pub(super) fn protocol_to_db_provider(protocol: &Protocol) -> String {
    match protocol {
        Protocol::GmailApi => "gmail_api".to_string(),
        Protocol::MicrosoftGraph => "graph".to_string(),
        Protocol::Jmap { .. } => "jmap".to_string(),
        Protocol::Imap { .. } => "imap".to_string(),
    }
}

fn protocol_display_name(protocol: &Protocol, provider_name: Option<&str>) -> String {
    match (protocol, provider_name) {
        (_, Some(name)) => name.to_string(),
        (Protocol::GmailApi, _) => "Gmail".to_string(),
        (Protocol::MicrosoftGraph, _) => "Microsoft 365".to_string(),
        (Protocol::Jmap { .. }, _) => "JMAP".to_string(),
        (Protocol::Imap { .. }, _) => "IMAP".to_string(),
    }
}

fn protocol_detail(protocol: &Protocol) -> String {
    match protocol {
        Protocol::GmailApi => "Gmail API (recommended)".to_string(),
        Protocol::MicrosoftGraph => "Microsoft Graph API".to_string(),
        Protocol::Jmap { session_url } => format!("JMAP: {session_url}"),
        Protocol::Imap { incoming, outgoing } => {
            format!(
                "IMAP: {}:{} / SMTP: {}:{}",
                incoming.hostname, incoming.port, outgoing.hostname, outgoing.port
            )
        }
    }
}

fn source_display(source: &DiscoverySource) -> &str {
    match source {
        DiscoverySource::Registry => "Known provider",
        DiscoverySource::AutoconfigXml { .. } => "Auto-detected",
        DiscoverySource::MxLookup { .. } => "MX lookup",
        DiscoverySource::JmapWellKnown => "JMAP discovery",
        DiscoverySource::OidcWellKnown => "OIDC discovery",
        DiscoverySource::PortProbe => "Port scan",
    }
}

trait HighConfidence {
    fn is_high_confidence(&self) -> bool;
}

impl HighConfidence for DiscoverySource {
    fn is_high_confidence(&self) -> bool {
        matches!(
            self,
            DiscoverySource::Registry
                | DiscoverySource::JmapWellKnown
                | DiscoverySource::OidcWellKnown
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtsk::discovery::types::{AuthConfig, AuthMethod, Security, ServerConfig, UsernameFormat};

    fn option(protocol: Protocol, source: DiscoverySource) -> ProtocolOption {
        ProtocolOption {
            protocol,
            auth: AuthConfig {
                method: AuthMethod::Password,
                alternatives: Vec::new(),
            },
            provider_name: None,
            help_url: None,
            source,
        }
    }

    fn imap() -> Protocol {
        let server = |hostname: &str, port| ServerConfig {
            hostname: hostname.to_string(),
            port,
            security: Security::Tls,
            username: UsernameFormat::EmailAddress,
        };
        Protocol::Imap {
            incoming: server("lavillart.com", 993),
            outgoing: server("lavillart.com", 465),
        }
    }

    fn config(options: Vec<ProtocolOption>) -> DiscoveredConfig {
        DiscoveredConfig {
            email: "contact@lavillart.com".to_string(),
            domain: "lavillart.com".to_string(),
            options,
            resolved_domain: None,
            oidc_endpoints: None,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn microsoft_365_domain_goes_straight_to_microsoft_sign_in() {
        let mx = DiscoverySource::MxLookup {
            mx_domain: "lavillart-com.mail.protection.outlook.com".to_string(),
        };
        let autoconfig = DiscoverySource::AutoconfigXml {
            url: "https://autoconfig.lavillart.com".to_string(),
        };
        let found = config(vec![
            option(imap(), autoconfig.clone()),
            option(Protocol::MicrosoftGraph, mx),
        ]);
        assert_eq!(auto_option(&found), Some(1));
        let imap_only = config(vec![
            option(imap(), autoconfig.clone()),
            option(imap(), autoconfig),
        ]);
        assert_eq!(auto_option(&imap_only), None);
    }
}
