//! Add Account wizard - multi-step state machine and views.
//!
//! Phases 2-3 of the accounts implementation spec. The wizard handles
//! first-launch onboarding and subsequent account additions.

mod discovery;
mod email_input;
mod identity;
mod manual_config;
mod oauth;
mod password_auth;
mod state;
mod views;

pub use state::{AddAccountEvent, AddAccountMessage, AddAccountWizard};

/// The text shown when `account.verify` could not open the mailbox.
fn verify_failure_text(ack: service_api::VerifyAccountAck) -> String {
    match ack.reason {
        Some(service_api::VerifyFailureReason::Unreachable) => {
            rust_i18n::t!("account_setup.error.server_unreachable").into_owned()
        }
        Some(service_api::VerifyFailureReason::Authentication) => {
            rust_i18n::t!("account_setup.error.authentication_failed").into_owned()
        }
        None => ack
            .message
            .unwrap_or_else(|| rust_i18n::t!("account_setup.error.verify_failed").into_owned()),
    }
}
