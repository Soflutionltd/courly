pub use service_api::actions::{ActionError, ActionOutcome, RemoteFailureKind};

use conduit_types::{AccountError, EngineDirective, RecoveryClass};

/// The action error for a provider failure, classed by its recovery: a
/// failure a later attempt can fix is `Transient`, one it never can is
/// `Permanent`. Lives here, below both the Service and `cal`, so that mail,
/// contact and calendar writes read a failure the same way.
pub fn account_error_to_action_error(err: &AccountError) -> ActionError {
    let kind = recovery_to_failure_kind(err.recovery());
    ActionError::remote_with_kind(kind, err.message_key())
}

fn recovery_to_failure_kind(recovery: &RecoveryClass) -> RemoteFailureKind {
    match recovery {
        RecoveryClass::Retry(_) | RecoveryClass::Reconcile(_) => RemoteFailureKind::Transient,
        // Engine-class failures split by whether the engine can self-heal.
        // Auto-recoverable directives (scope/account restart, strategy or
        // capability downgrade, scope disable) leave a later pending-ops
        // re-drive a path to success once the engine resumes -> retryable.
        // SchemaIncompatible / OperatorOverrideRequired cannot clear without
        // a migration or operator action, so an action retry can NEVER
        // succeed -> terminal (the AuthLost analogue).
        RecoveryClass::Engine(directive) => match directive {
            EngineDirective::SchemaIncompatible
            | EngineDirective::OperatorOverrideRequired { .. } => RemoteFailureKind::Permanent,
            EngineDirective::RestartScope(_)
            | EngineDirective::RestartAccount
            | EngineDirective::DowngradeStrategy(_)
            | EngineDirective::DowngradeCapabilityForScope(_)
            | EngineDirective::DisableScope(_) => RemoteFailureKind::Transient,
            // `EngineDirective` is `#[non_exhaustive]`: a new upstream
            // directive defaults to the conservative auto-recoverable
            // (retryable) classification - engine directives are recovery
            // work, and the bounded per-op retry budget caps any wasted
            // re-drive. Re-audited on every conduit bump.
            _ => RemoteFailureKind::Transient,
        },
        RecoveryClass::Unsupported(_) => RemoteFailureKind::NotImplemented,
        RecoveryClass::AuthLost
        | RecoveryClass::NeedsAdminConsent { .. }
        | RecoveryClass::NeedsPolicyChange
        | RecoveryClass::NoPermission { .. }
        | RecoveryClass::ClientBug
        | RecoveryClass::ProviderContractViolation
        | RecoveryClass::ProviderRefused
        | RecoveryClass::UnknownPermanent => RemoteFailureKind::Permanent,
        // `RecoveryClass` is `#[non_exhaustive]` upstream, so this wildcard is
        // REQUIRED to compile. It is the documented conservative-terminal
        // default: a new upstream class is treated as terminal (never enqueues
        // a doomed retry) until a conduit-bump revalidation names it above. Do
        // NOT delete this arm.
        _ => RemoteFailureKind::Permanent,
    }
}
