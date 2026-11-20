//! Pre-persist mailbox connection verification.

use std::sync::Arc;

use conduit_types::{
    AccountError, AccountErrorKind, AccountId, ServerErrorKind, TransportErrorKind,
};
use serde_json::Value;
use service_api::{ServiceError, VerifyAccountAck, VerifyAccountParams, VerifyFailureReason};
use types::MailProviderKind;

use crate::conduit::error_map::account_error_to_action_error;
use crate::conduit::factory::{
    ConduitBuildError, DecryptedAccountCredentials, TokenMode, factory_from_decrypted,
};
use crate::boot::BootSharedState;

pub(crate) fn build_error_message(error: &ConduitBuildError) -> &'static str {
    match error {
        ConduitBuildError::UnknownProvider(_) => "This account type is not supported.",
        ConduitBuildError::MissingCredential { .. }
        | ConduitBuildError::MissingEndpoint { .. }
        | ConduitBuildError::InvalidConfig { .. } => "Missing or invalid connection setting.",
        ConduitBuildError::Decrypt { .. } | ConduitBuildError::Db(_) => {
            "Could not prepare the account connection."
        }
    }
}

pub(crate) async fn handle_verify_account(
    _boot_state: &Arc<BootSharedState>,
    params: Box<VerifyAccountParams>,
) -> Result<Value, ServiceError> {
    let params = *params;
    let token = params
        .access_token
        .as_ref()
        .map(|token| token.expose().to_string());
    let decrypted = DecryptedAccountCredentials::from_verify_params(params);
    let provider = match MailProviderKind::parse(&decrypted.row_provider()) {
        Ok(provider) => provider,
        Err(_) => {
            return ack(
                false,
                Some("This account type is not supported.".to_string()),
                None,
            );
        }
    };
    let token_mode = TokenMode::Static(token.unwrap_or_default());
    let factory = match factory_from_decrypted(&decrypted, provider, &token_mode) {
        Ok(factory) => factory,
        Err(error) => return ack(false, Some(build_error_message(&error).to_string()), None),
    };
    let synthetic_id = AccountId(format!("verify-{}", uuid::Uuid::new_v4()));
    match factory.open(synthetic_id).await {
        Ok(opened) => {
            let _ = opened.account.close().await;
            ack(true, None, None)
        }
        Err(error) => ack(
            false,
            Some(account_error_to_action_error(&error).user_message()),
            failure_reason(&error),
        ),
    }
}

fn failure_reason(error: &AccountError) -> Option<VerifyFailureReason> {
    match error.kind() {
        AccountErrorKind::Transport(TransportErrorKind::Network | TransportErrorKind::Timeout)
        | AccountErrorKind::Server(ServerErrorKind::Unavailable | ServerErrorKind::RateLimited) => {
            Some(VerifyFailureReason::Unreachable)
        }
        AccountErrorKind::Authentication(_) => Some(VerifyFailureReason::Authentication),
        _ => None,
    }
}

fn ack(
    ok: bool,
    message: Option<String>,
    reason: Option<VerifyFailureReason>,
) -> Result<Value, ServiceError> {
    serde_json::to_value(VerifyAccountAck {
        ok,
        message,
        reason,
    })
    .map_err(|error| ServiceError::Internal(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use conduit_types::{
        AccountErrorBuilder, AuthCause, AuthErrorKind, Cause, TransportCause, TransportKind,
    };

    #[test]
    fn verify_account_build_error_messages_are_stable_and_safe() {
        // Every build-error variant maps to a fixed, user-safe string that
        // never carries the internal account id or config detail. This is
        // the § 4.4 (R2-9) no-leak guarantee: `ConduitBuildError::to_string()`
        // must not reach the wire.
        let cases = [
            ConduitBuildError::UnknownProvider("oidc:https://evil.example".into()),
            ConduitBuildError::MissingCredential {
                account_id: "verify-secret-id".into(),
                field: "access_token",
            },
            ConduitBuildError::MissingEndpoint {
                account_id: "verify-secret-id".into(),
                provider: MailProviderKind::Jmap,
            },
            ConduitBuildError::InvalidConfig {
                account_id: "verify-secret-id".into(),
                detail: "unknown IMAP security mode banana".into(),
            },
            ConduitBuildError::Decrypt {
                account_id: "verify-secret-id".into(),
                field: "imap_password",
                error: "aead tag mismatch".into(),
            },
            ConduitBuildError::Db("no such table: accounts".into()),
        ];
        for case in &cases {
            let message = build_error_message(case);
            assert!(!message.is_empty(), "empty message for {case:?}");
            assert!(
                !message.contains("verify-secret-id"),
                "leaked account id for {case:?}: {message}",
            );
            assert!(
                !message.contains("banana") && !message.contains("aead"),
                "leaked internal detail for {case:?}: {message}",
            );
        }
    }

    #[test]
    fn verify_account_maps_auth_error_to_nonempty_message() {
        // The handler routes an `open` failure through `error_map` and
        // surfaces `user_message()`. An auth-failure kind must produce a
        // non-empty, user-facing string (the `ok: false` verify result).
        let error = AccountErrorBuilder::new(
            AccountErrorKind::Authentication(AuthErrorKind::Expired),
            Cause::Auth(AuthCause::Expired),
        )
        .try_build()
        .expect("valid auth error");
        let message = account_error_to_action_error(&error).user_message();
        assert!(!message.is_empty(), "auth failure produced empty message");
    }

    #[test]
    fn verify_reason_tells_an_unanswering_server_from_refused_credentials() {
        let timeout = AccountErrorBuilder::new(
            AccountErrorKind::Transport(TransportErrorKind::Timeout),
            Cause::Transport(TransportCause::new(TransportKind::Timeout, None)),
        )
        .try_build()
        .expect("valid timeout error");
        assert_eq!(
            failure_reason(&timeout),
            Some(VerifyFailureReason::Unreachable)
        );

        let refused = AccountErrorBuilder::new(
            AccountErrorKind::Authentication(AuthErrorKind::Expired),
            Cause::Auth(AuthCause::Expired),
        )
        .try_build()
        .expect("valid auth error");
        assert_eq!(
            failure_reason(&refused),
            Some(VerifyFailureReason::Authentication)
        );
    }
}
