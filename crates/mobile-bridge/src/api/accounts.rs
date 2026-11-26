//! What the phone needs to add accounts and show them: which provider an
//! address belongs to, and the accounts already on the device.

use rtsk::discovery::types::{AuthMethod, Protocol, Security, ServerConfig, UsernameFormat};
use service_api::{
    AccountCreateAck, AccountCreateParams, AccountCredentials, RedactedString, RequestParams,
    SyncStartAccountParams, VerifyAccountAck, VerifyAccountParams, VerifyFailureReason,
};

use super::engine::{block_on, call};
use super::microsoft::AddedAccount;

/// Why a password account could not be added, for the phone to word.
pub enum AddFailure {
    /// The server answered and refused the address or password.
    WrongPassword,
    /// The server did not answer; the same details may work later.
    Unreachable,
    /// Anything else; `detail` carries the engine's words.
    Other,
}

/// What adding a password account came to: the account, or why not.
pub struct AddPasswordResult {
    pub added: Option<AddedAccount>,
    pub failure: Option<AddFailure>,
    /// The server's or the engine's own words, for a failure.
    pub detail: String,
}

impl AddPasswordResult {
    fn failed(failure: AddFailure, detail: String) -> Self {
        Self {
            added: None,
            failure: Some(failure),
            detail,
        }
    }
}

/// Add an IMAP and SMTP account with its password: the servers are tried
/// first, as on the desktop, so a typo is caught before anything is saved;
/// then the account is created and its first sync started.
pub fn add_password_account(
    email: String,
    password: String,
    servers: ImapServers,
) -> AddPasswordResult {
    match try_add_password_account(email, password, servers) {
        Ok(added) => AddPasswordResult {
            added: Some(added),
            failure: None,
            detail: String::new(),
        },
        Err((failure, detail)) => AddPasswordResult::failed(failure, detail),
    }
}

fn try_add_password_account(
    email: String,
    password: String,
    servers: ImapServers,
) -> Result<AddedAccount, (AddFailure, String)> {
    let other = |detail: String| (AddFailure::Other, detail);
    let email = email.trim().to_lowercase();
    let verify: VerifyAccountAck = call(RequestParams::AccountVerify {
        params: Box::new(VerifyAccountParams {
            provider: "imap".to_owned(),
            email: email.clone(),
            imap_host: Some(servers.imap_host.clone()),
            imap_port: Some(servers.imap_port),
            imap_security: Some(servers.imap_security.clone()),
            username: Some(servers.username.clone()),
            imap_password: Some(RedactedString::new(password.clone())),
            accept_invalid_certs: false,
            access_token: None,
            jmap_url: None,
        }),
    })
    .map_err(other)?;
    if !verify.ok {
        let message = verify.message.unwrap_or_default();
        return Err(match verify.reason {
            Some(VerifyFailureReason::Authentication) => (AddFailure::WrongPassword, message),
            Some(VerifyFailureReason::Unreachable) => (AddFailure::Unreachable, message),
            None => (AddFailure::Other, message),
        });
    }

    let created: AccountCreateAck = call(RequestParams::AccountCreate {
        params: Box::new(AccountCreateParams {
            email: email.clone(),
            provider: "imap".to_owned(),
            display_name: None,
            account_name: super::microsoft::account_name_for(&email),
            account_color: super::microsoft::default_account_color().map_err(other)?,
            auth_method: "password".to_owned(),
            credentials: AccountCredentials::Plaintext {
                access_token: None,
                refresh_token: None,
                imap_password: Some(password),
                smtp_password: None,
            },
            token_expires_at: None,
            oauth_provider: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            oauth_extra_scopes: None,
            imap_host: Some(servers.imap_host),
            imap_port: Some(i64::from(servers.imap_port)),
            imap_security: Some(servers.imap_security),
            imap_username: Some(servers.username),
            smtp_host: Some(servers.smtp_host),
            smtp_port: Some(i64::from(servers.smtp_port)),
            smtp_security: Some(servers.smtp_security),
            smtp_username: None,
            jmap_url: None,
            accept_invalid_certs: false,
        }),
    })
    .map_err(other)?;

    let _: serde_json::Value = call(RequestParams::SyncStartAccount {
        params: SyncStartAccountParams {
            account_id: created.id.clone(),
        },
    })
    .map_err(other)?;

    Ok(AddedAccount {
        account_id: created.id,
        email,
        display_name: None,
    })
}

/// How an address is signed in, as the desktop's discovery found it.
pub enum SignInMethod {
    /// Microsoft 365 or Outlook.com: Microsoft's sign-in page.
    Microsoft,
    /// A mailbox reached over IMAP and SMTP with a password; the servers
    /// discovery found are filled in, the user can still change them.
    /// Gmail and Google Workspace come here too, with an app password.
    Imap,
    /// Nothing was found: the user types the servers in.
    Manual,
}

/// What discovery found for an address.
pub struct DiscoveredAccount {
    pub method: SignInMethod,
    /// The servers found, for `Imap`.
    pub servers: Option<ImapServers>,
    /// For Gmail: the page where the user makes the app password Google
    /// requires for IMAP, which the phone links to.
    pub app_password_url: Option<String>,
}

/// Google's page for making an app password; IMAP refuses the account
/// password once two-step verification is on, which Google requires for
/// app passwords.
const GOOGLE_APP_PASSWORDS: &str = "https://myaccount.google.com/apppasswords";

/// Gmail and Google Workspace over IMAP: Google's published servers, with
/// the full address as the login.
fn gmail_servers(email: &str) -> ImapServers {
    ImapServers {
        imap_host: "imap.gmail.com".to_owned(),
        imap_port: 993,
        imap_security: "tls".to_owned(),
        smtp_host: "smtp.gmail.com".to_owned(),
        smtp_port: 465,
        smtp_security: "tls".to_owned(),
        username: email.to_owned(),
    }
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
pub fn discover_account(email: String) -> Result<DiscoveredAccount, String> {
    let email = email.trim().to_lowercase();
    let config = block_on(rtsk::discovery::discover(&email))??;
    Ok(kind_of(&email, config.options.first()))
}

/// The best option discovery ranked, as the phone can use it. JMAP is not
/// wired on the phone yet, so it falls back to manual IMAP. Gmail goes over
/// IMAP with an app password: Courly has no verified Google client for
/// Gmail's restricted scopes.
fn kind_of(
    email: &str,
    best: Option<&rtsk::discovery::types::ProtocolOption>,
) -> DiscoveredAccount {
    let only = |method| DiscoveredAccount {
        method,
        servers: None,
        app_password_url: None,
    };
    let Some(option) = best else {
        return only(SignInMethod::Manual);
    };
    match &option.protocol {
        Protocol::MicrosoftGraph => only(SignInMethod::Microsoft),
        Protocol::GmailApi => DiscoveredAccount {
            method: SignInMethod::Imap,
            servers: Some(gmail_servers(email)),
            app_password_url: Some(GOOGLE_APP_PASSWORDS.to_owned()),
        },
        Protocol::Imap { incoming, outgoing }
            if matches!(option.auth.method, AuthMethod::Password) =>
        {
            DiscoveredAccount {
                method: SignInMethod::Imap,
                app_password_url: None,
                servers: Some(ImapServers {
                    imap_host: incoming.hostname.clone(),
                    imap_port: incoming.port,
                    imap_security: security_name(incoming.security).to_owned(),
                    smtp_host: outgoing.hostname.clone(),
                    smtp_port: outgoing.port,
                    smtp_security: security_name(outgoing.security).to_owned(),
                    username: login_for(email, incoming),
                }),
            }
        }
        _ => only(SignInMethod::Manual),
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
        let found = kind_of("a@b.com", None);
        assert!(matches!(found.method, SignInMethod::Manual));
        assert!(found.servers.is_none());
    }

    #[test]
    fn gmail_goes_over_imap_with_an_app_password() {
        let option = ProtocolOption {
            protocol: Protocol::GmailApi,
            auth: AuthConfig {
                method: AuthMethod::Password,
                alternatives: Vec::new(),
            },
            provider_name: None,
            help_url: None,
            source: DiscoverySource::Registry,
        };
        let found = kind_of("jo@gmail.com", Some(&option));
        assert!(matches!(found.method, SignInMethod::Imap));
        let servers = found.servers.expect("Gmail's servers");
        assert_eq!(servers.imap_host, "imap.gmail.com");
        assert_eq!(servers.username, "jo@gmail.com");
        assert_eq!(
            found.app_password_url.as_deref(),
            Some(GOOGLE_APP_PASSWORDS)
        );
    }

    #[test]
    fn a_password_imap_option_carries_its_servers() {
        let option = imap_option(UsernameFormat::LocalPart);
        let found = kind_of("jo@example.com", Some(&option));
        assert!(matches!(found.method, SignInMethod::Imap));
        let servers = found.servers.expect("an IMAP option carries its servers");
        assert_eq!(servers.imap_host, "imap.example.com");
        assert_eq!(servers.imap_security, "tls");
        assert_eq!(servers.smtp_port, 587);
        assert_eq!(servers.smtp_security, "starttls");
        assert_eq!(servers.username, "jo");
    }
}
