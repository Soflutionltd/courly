//! Adding a Microsoft 365 or Outlook.com account from the phone.
//!
//! The desktop catches Microsoft's answer on a loopback port; iOS has no
//! such listener, so the app opens the sign-in page in Apple's
//! authentication session and gets the answer back on the
//! `msauth.com.soflution.courly` scheme, which the Courly registration in
//! Microsoft Entra accepts for this bundle. Everything after the code is
//! the desktop path: the Service exchanges it for tokens, the account is
//! created, then synced.

use std::sync::Mutex;

use rtsk::oauth::{GenericOAuthProvider, OAuthIdentityProvider, build_authorization_url};
use service_api::{
    AccountCreateAck, AccountCreateParams, AccountCredentials, OauthExchangeCodeAck,
    OauthExchangeCodeParams, RedactedString, RequestParams, SyncStartAccountParams,
};

use super::engine::call;

/// The URL scheme Apple's authentication session watches for.
pub const MICROSOFT_CALLBACK_SCHEME: &str = "msauth.com.soflution.courly";
const REDIRECT_URI: &str = "msauth.com.soflution.courly://auth";
/// Same id as the desktop: the provider name the Service and the sync
/// engine key Microsoft Graph accounts on.
const PROVIDER_ID: &str = "microsoft_graph";

struct PendingSignIn {
    state: String,
    code_verifier: Option<String>,
}

static PENDING: Mutex<Option<PendingSignIn>> = Mutex::new(None);

/// The page to open for a Microsoft sign-in.
pub struct MicrosoftSignIn {
    /// Microsoft's authorization page, with PKCE and a fresh state.
    pub auth_url: String,
    /// The scheme the authentication session must watch.
    pub callback_scheme: String,
}

/// An account the phone just added.
pub struct AddedAccount {
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
}

/// Prepare a Microsoft sign-in. A sign-in started earlier and never
/// finished is replaced.
pub fn microsoft_sign_in_start() -> Result<MicrosoftSignIn, String> {
    let provider = GenericOAuthProvider::microsoft_graph_default();
    let request = provider.authorization_request();
    let (auth_url, code_verifier, state) = build_authorization_url(&request, REDIRECT_URI)?;
    *PENDING
        .lock()
        .map_err(|_| "sign-in state poisoned".to_owned())? = Some(PendingSignIn {
        state,
        code_verifier,
    });
    Ok(MicrosoftSignIn {
        auth_url,
        callback_scheme: MICROSOFT_CALLBACK_SCHEME.to_owned(),
    })
}

/// Finish the sign-in from the URL Microsoft redirected to: exchange the
/// code, create the account and start its first sync.
pub fn microsoft_sign_in_finish(callback_url: String) -> Result<AddedAccount, String> {
    let pending = PENDING
        .lock()
        .map_err(|_| "sign-in state poisoned".to_owned())?
        .take()
        .ok_or_else(|| "no Microsoft sign-in is in progress".to_owned())?;
    let code = code_from_callback(&callback_url, &pending.state)?;

    let provider = GenericOAuthProvider::microsoft_graph_default();
    let authorization = provider.authorization_request();
    let token_request = provider.token_request();
    let exchange: OauthExchangeCodeAck = call(RequestParams::OauthExchangeCode {
        params: Box::new(OauthExchangeCodeParams {
            provider_id: PROVIDER_ID.to_owned(),
            token_url: token_request.token_url,
            scopes: authorization.scopes,
            user_info_url: None,
            use_pkce: authorization.use_pkce,
            client_id: authorization.client_id.clone(),
            client_secret: None,
            redirect_uri: REDIRECT_URI.to_owned(),
            code: RedactedString::new(code),
            code_verifier: pending.code_verifier,
            reauth_account_id: None,
            expected_email: None,
        }),
    })?;
    let access_token = exchange
        .access_token
        .map(RedactedString::into_inner)
        .ok_or_else(|| "Microsoft returned no access token".to_owned())?;

    let created: AccountCreateAck = call(RequestParams::AccountCreate {
        params: Box::new(AccountCreateParams {
            email: exchange.email.clone(),
            provider: "graph".to_owned(),
            display_name: exchange.display_name.clone(),
            account_name: account_name_for(&exchange.email),
            account_color: default_account_color()?,
            auth_method: "oauth2".to_owned(),
            credentials: AccountCredentials::Plaintext {
                access_token: Some(access_token),
                refresh_token: exchange.refresh_token.map(RedactedString::into_inner),
                imap_password: None,
                smtp_password: None,
            },
            token_expires_at: exchange.token_expires_at,
            oauth_provider: Some(PROVIDER_ID.to_owned()),
            oauth_client_id: Some(authorization.client_id),
            oauth_client_secret: None,
            oauth_extra_scopes: None,
            imap_host: None,
            imap_port: None,
            imap_security: None,
            imap_username: None,
            smtp_host: None,
            smtp_port: None,
            smtp_security: None,
            smtp_username: None,
            jmap_url: None,
            accept_invalid_certs: false,
        }),
    })?;

    let _: serde_json::Value = call(RequestParams::SyncStartAccount {
        params: SyncStartAccountParams {
            account_id: created.id.clone(),
        },
    })?;

    Ok(AddedAccount {
        account_id: created.id,
        email: exchange.email,
        display_name: exchange.display_name,
    })
}

/// The color of a new account: the first preset, as the desktop wizard
/// uses when the user picks none.
fn default_account_color() -> Result<String, String> {
    label_colors::preset_colors::all_presets()
        .first()
        .map(|(_, background, _)| (*background).to_owned())
        .ok_or_else(|| "no preset account color is defined".to_owned())
}

/// The account's name in the list: its domain, capitalised, as the
/// desktop wizard pre-fills it.
fn account_name_for(email: &str) -> String {
    let domain = email.split('@').nth(1).unwrap_or(email);
    let first = domain.split('.').next().unwrap_or(domain);
    let mut chars = first.chars();
    match chars.next() {
        Some(head) => head.to_uppercase().chain(chars).collect(),
        None => email.to_owned(),
    }
}

/// Pull the authorization code out of Microsoft's redirect, refusing a
/// redirect whose state is not the one this sign-in sent and surfacing
/// Microsoft's own error text when the user cancelled or was refused.
fn code_from_callback(callback_url: &str, expected_state: &str) -> Result<String, String> {
    let url = url::Url::parse(callback_url).map_err(|error| format!("callback URL: {error}"))?;
    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut error_description = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "error_description" => error_description = Some(value.into_owned()),
            _ => {}
        }
    }
    if let Some(error) = error {
        return Err(match error_description {
            Some(description) => format!("Microsoft refused the sign-in: {description}"),
            None => format!("Microsoft refused the sign-in: {error}"),
        });
    }
    if state.as_deref() != Some(expected_state) {
        return Err("the sign-in answer does not match the request; try again".to_owned());
    }
    code.ok_or_else(|| "Microsoft's answer carries no code".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_is_read_when_the_state_matches() {
        let code = code_from_callback("msauth.com.soflution.courly://auth?code=abc&state=s1", "s1");
        assert_eq!(code, Ok("abc".to_owned()));
    }

    #[test]
    fn a_foreign_state_is_refused() {
        let result = code_from_callback(
            "msauth.com.soflution.courly://auth?code=abc&state=other",
            "s1",
        );
        assert!(result.is_err());
    }

    #[test]
    fn microsofts_refusal_is_shown_in_its_own_words() {
        let result = code_from_callback(
            "msauth.com.soflution.courly://auth?error=access_denied&error_description=User%20cancelled&state=s1",
            "s1",
        );
        assert_eq!(
            result,
            Err("Microsoft refused the sign-in: User cancelled".to_owned())
        );
    }

    #[test]
    fn the_start_url_targets_microsoft_with_the_phone_redirect_and_pkce() {
        let start = microsoft_sign_in_start().expect("start");
        assert_eq!(start.callback_scheme, MICROSOFT_CALLBACK_SCHEME);
        let url = url::Url::parse(&start.auth_url).expect("url");
        assert_eq!(url.host_str(), Some("login.microsoftonline.com"));
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(
            pairs.get("redirect_uri").map(String::as_str),
            Some(REDIRECT_URI)
        );
        assert_eq!(
            pairs.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
    }

    #[test]
    fn the_account_is_named_after_its_domain() {
        assert_eq!(account_name_for("jo@contoso.com"), "Contoso");
    }
}
