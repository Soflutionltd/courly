//! What the phone needs to add accounts and show them: which provider an
//! address belongs to, and the accounts already on the device.

use rtsk::discovery::types::{AuthMethod, Protocol, Security, ServerConfig, UsernameFormat};

use super::engine::block_on;

/// How an address is signed in, as the desktop's discovery found it.
pub enum AccountKind {
    /// Microsoft 365 or Outlook.com: Microsoft's sign-in page.
    Microsoft,
    /// Gmail or Google Workspace: Google's sign-in page.
    Google,
    /// A mailbox reached over IMAP and SMTP with a password; the servers
    /// discovery found are filled in, the user can still change them.
    Imap(ImapServers),
    /// Nothing was found: the user types the servers in.
    Manual,
}

/// IMAP and SMTP servers for a password account.
pub struct ImapServers {
    pub imap_host: String,
    pub imap_port: u16,
    /// `tls`, `starttls` or `none`, as the desktop stores it.
    pub imap_security: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: String,
    /// The login the servers expect: the full address or its local part.
    pub username: String,
}

/// Find how `email` signs in, through the same discovery cascade as the
/// desktop (provider registry, autoconfig, MX, JMAP, port probe), within
/// its 15 second budget.
pub fn discover_account(email: String) -> Result<AccountKind, String> {
    let email = email.trim().to_lowercase();
    let config = block_on(rtsk::discovery::discover(&email))??;
    Ok(kind_of(&email, config.options.first()))
}

/// The best option discovery ranked, as the phone can use it. JMAP is not
/// wired on the phone yet, so it falls back to manual IMAP.
fn kind_of(email: &str, best: Option<&rtsk::discovery::types::ProtocolOption>) -> AccountKind {
    let Some(option) = best else {
        return AccountKind::Manual;
    };
    match &option.protocol {
        Protocol::MicrosoftGraph => AccountKind::Microsoft,
        Protocol::GmailApi => AccountKind::Google,
        Protocol::Imap { incoming, outgoing }
            if matches!(option.auth.method, AuthMethod::Password) =>
        {
            AccountKind::Imap(ImapServers {
                imap_host: incoming.hostname.clone(),
                imap_port: incoming.port,
                imap_security: security_name(incoming.security).to_owned(),
                smtp_host: outgoing.hostname.clone(),
                smtp_port: outgoing.port,
                smtp_security: security_name(outgoing.security).to_owned(),
                username: login_for(email, incoming),
            })
        }
        _ => AccountKind::Manual,
    }
}

fn security_name(security: Security) -> &'static str {
    match security {
        Security::Tls => "tls",
        Security::StartTls => "starttls",
        Security::None => "none",
    }
}

fn login_for(email: &str, server: &ServerConfig) -> String {
    match &server.username {
        UsernameFormat::LocalPart => email.split('@').next().unwrap_or(email).to_owned(),
        UsernameFormat::Custom { value } => value.clone(),
        UsernameFormat::EmailAddress => email.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtsk::discovery::types::{AuthConfig, DiscoverySource, ProtocolOption};

    fn imap_option(username: UsernameFormat) -> ProtocolOption {
        let server = |host: &str, port, security| ServerConfig {
            hostname: host.to_owned(),
            port,
            security,
            username: username.clone(),
        };
        ProtocolOption {
            protocol: Protocol::Imap {
                incoming: server("imap.example.com", 993, Security::Tls),
                outgoing: server("smtp.example.com", 587, Security::StartTls),
            },
            auth: AuthConfig {
                method: AuthMethod::Password,
                alternatives: Vec::new(),
            },
            provider_name: None,
            help_url: None,
            source: DiscoverySource::Registry,
        }
    }

    #[test]
    fn nothing_found_means_manual_setup() {
        assert!(matches!(kind_of("a@b.com", None), AccountKind::Manual));
    }

    #[test]
    fn a_password_imap_option_carries_its_servers() {
        let option = imap_option(UsernameFormat::LocalPart);
        let AccountKind::Imap(servers) = kind_of("jo@example.com", Some(&option)) else {
            panic!("an IMAP option is an IMAP account");
        };
        assert_eq!(servers.imap_host, "imap.example.com");
        assert_eq!(servers.imap_security, "tls");
        assert_eq!(servers.smtp_port, 587);
        assert_eq!(servers.smtp_security, "starttls");
        assert_eq!(servers.username, "jo");
    }
}
