//! Watch the subscription behind each Exchange account.
//!
//! A mailbox whose subscription lapsed keeps syncing without any error, so
//! nothing in a sync says Microsoft is about to delete it. Graph does say
//! it, in the state of the account's Exchange plan. This asks once after
//! the Service starts, then every hour, and tells the app when an account
//! is at risk and when it no longer is.
//!
//! Which state a lapsed subscription takes was read in Microsoft's
//! documentation of `assignedPlan.capabilityStatus`, not seen on a live
//! mailbox: every check logs the state it got, so a real case confirms or
//! corrects it. The log holds the account id, the HTTP status and the
//! names and states of the plans. No address, name or token is written.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use conduit_net::TokenSource;
use service_api::{Notification, SubscriptionStateNotification};

use crate::boot_progress::NotificationSender;

const PLANS_URL: &str =
    "https://graph.microsoft.com/v1.0/me?$select=accountEnabled,assignedLicenses,assignedPlans";
/// The engine refreshes an expired token as it starts syncing; the watch
/// waits for that instead of refreshing on its own.
const START_DELAY: Duration = Duration::from_secs(45);
const TOKEN_RETRY_DELAY: Duration = Duration::from_secs(60);
/// A plan passes through a transient state while a licence is being
/// changed: an account is only reported at risk when a second check, this
/// long after the first, still says so.
const CONFIRM_DELAY: Duration = Duration::from_secs(300);
const CHECK_EVERY: Duration = Duration::from_secs(3600);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// The state of a plan that works.
const ENABLED: &str = "Enabled";
/// States of a plan whose subscription lapsed, soonest first: in its grace
/// period, then disabled, then locked.
const LAPSED: &[&str] = &["Warning", "Suspended", "LockedOut"];

type Sources = Mutex<HashMap<String, Arc<dyn TokenSource>>>;

fn sources() -> &'static Sources {
    static SOURCES: OnceLock<Sources> = OnceLock::new();
    SOURCES.get_or_init(Mutex::default)
}

fn notifier() -> &'static Mutex<Option<(NotificationSender, u32)>> {
    static NOTIFIER: OnceLock<Mutex<Option<(NotificationSender, u32)>>> = OnceLock::new();
    NOTIFIER.get_or_init(Mutex::default)
}

/// Where the watch reports what it finds. Set once by the resident engine.
pub(crate) fn set_notifier(sender: NotificationSender, service_generation: u32) {
    *notifier().lock().unwrap_or_else(PoisonError::into_inner) = Some((sender, service_generation));
}

/// Watch the account. Called each time its client is built: the first call
/// starts the watch, later ones hand it the token source now in use.
pub(crate) fn watch(account_id: String, source: Arc<dyn TokenSource>) {
    let already_watched = sources()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(account_id.clone(), source)
        .is_some();
    if already_watched {
        return;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        log::warn!("[subscription] {account_id}: no runtime, not watched");
        return;
    };
    runtime.spawn(async move {
        tokio::time::sleep(START_DELAY).await;
        let mut reported_at_risk = false;
        let mut suspected = false;
        let mut token_refused = false;
        loop {
            let wait = match check(&account_id).await {
                Check::TokenRefused if !token_refused => {
                    token_refused = true;
                    TOKEN_RETRY_DELAY
                }
                Check::TokenRefused | Check::Unknown => CHECK_EVERY,
                Check::Exchange(state) => {
                    token_refused = false;
                    let at_risk = LAPSED.contains(&state.as_str());
                    if at_risk && !suspected && !reported_at_risk {
                        suspected = true;
                        CONFIRM_DELAY
                    } else {
                        suspected = false;
                        // A lapse is reported again at every check: the
                        // app may have started, or restarted, since.
                        if at_risk || reported_at_risk {
                            report(&account_id, at_risk, &state).await;
                            reported_at_risk = at_risk;
                        }
                        CHECK_EVERY
                    }
                }
            };
            tokio::time::sleep(wait).await;
        }
    });
}

enum Check {
    /// The state of the account's Exchange plan.
    Exchange(String),
    /// The token was refused: the engine has not refreshed it yet, or the
    /// account needs signing in again, which the sync reports on its own.
    TokenRefused,
    /// No answer, or one that says nothing about Exchange.
    Unknown,
}

async fn check(account_id: &str) -> Check {
    let source = sources()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(account_id)
        .cloned();
    let Some(source) = source else {
        return Check::Unknown;
    };
    let token = match source.current().await {
        Ok(token) => token,
        Err(error) => {
            log::warn!("[subscription] {account_id}: no token: {error}");
            return Check::Unknown;
        }
    };
    let http = match reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build() {
        Ok(http) => http,
        Err(error) => {
            log::warn!("[subscription] {account_id}: no HTTP client: {error}");
            return Check::Unknown;
        }
    };
    let response = match http.get(PLANS_URL).bearer_auth(token.as_str()).send().await {
        Ok(response) => response,
        Err(error) => {
            log::warn!("[subscription] {account_id}: request failed: {error}");
            return Check::Unknown;
        }
    };
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        log::info!("[subscription] {account_id}: HTTP 401, token refused");
        return Check::TokenRefused;
    }
    let body: serde_json::Value = response.json().await.unwrap_or_default();
    log::info!(
        "[subscription] {account_id}: HTTP {} {}",
        status.as_u16(),
        summary(&body)
    );
    if !status.is_success() {
        return Check::Unknown;
    }
    exchange_state(&body).map_or(Check::Unknown, Check::Exchange)
}

/// The state of the account's Exchange service: `Enabled` when one of its
/// Exchange plans works, else the lapsed state the plans are in. `None`
/// when the account lists no Exchange plan, or only ones that were removed.
///
/// An account keeps the plans of licences it once had, marked `Deleted`,
/// next to the ones it has now, and a shared mailbox has none at all: so a
/// removed plan alone is not a lapse, and several plans answer by the best
/// of them.
fn exchange_state(body: &serde_json::Value) -> Option<String> {
    let states: Vec<&str> = body["assignedPlans"]
        .as_array()?
        .iter()
        .filter(|plan| {
            plan["service"]
                .as_str()
                .is_some_and(|service| service.eq_ignore_ascii_case("exchange"))
        })
        .filter_map(|plan| plan["capabilityStatus"].as_str())
        .collect();
    if states.contains(&ENABLED) {
        return Some(ENABLED.to_owned());
    }
    LAPSED
        .iter()
        .find(|lapsed| states.contains(lapsed))
        .map(|lapsed| (*lapsed).to_owned())
}

/// What an answer says about the licence, and the error code when Graph
/// refused.
fn summary(body: &serde_json::Value) -> String {
    if let Some(error) = body.get("error") {
        return format!("error code={}", error["code"].as_str().unwrap_or("?"));
    }
    let plans: Vec<String> = body["assignedPlans"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|plan| {
            format!(
                "{}={}",
                plan["service"].as_str().unwrap_or("?"),
                plan["capabilityStatus"].as_str().unwrap_or("?")
            )
        })
        .collect();
    format!(
        "accountEnabled={} assignedLicenses={} assignedPlans=[{}]",
        body["accountEnabled"],
        body["assignedLicenses"].as_array().map_or(0, Vec::len),
        plans.join(", ")
    )
}

async fn report(account_id: &str, at_risk: bool, state: &str) {
    log::warn!("[subscription] {account_id}: at_risk={at_risk} exchange={state}");
    let target = notifier()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let Some((sender, service_generation)) = target else {
        log::warn!("[subscription] {account_id}: no notifier, the app was not told");
        return;
    };
    let notification = Notification::SubscriptionState(SubscriptionStateNotification {
        account_id: account_id.to_owned(),
        at_risk,
        state: state.to_owned(),
        service_generation,
    });
    if let Err(error) = sender.send(notification).await {
        log::debug!("[subscription] {account_id}: notification not sent: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::{exchange_state, summary};

    fn plans(plans: &[(&str, &str)]) -> serde_json::Value {
        let plans: Vec<serde_json::Value> = plans
            .iter()
            .map(|(service, status)| {
                serde_json::json!({"service": service, "capabilityStatus": status})
            })
            .collect();
        serde_json::json!({
            "accountEnabled": true,
            "assignedLicenses": [{}],
            "assignedPlans": plans
        })
    }

    #[test]
    fn a_working_exchange_plan_is_enabled_whatever_the_others_say() {
        let body = plans(&[
            ("exchange", "Deleted"),
            ("SharePoint", "Suspended"),
            ("Exchange", "Enabled"),
            ("exchange", "Warning"),
        ]);
        assert_eq!(exchange_state(&body).as_deref(), Some("Enabled"));
    }

    #[test]
    fn a_lapsed_subscription_reports_its_soonest_state() {
        let body = plans(&[("exchange", "Suspended"), ("exchange", "Warning")]);
        assert_eq!(exchange_state(&body).as_deref(), Some("Warning"));
        let body = plans(&[("exchange", "LockedOut"), ("To-Do", "Enabled")]);
        assert_eq!(exchange_state(&body).as_deref(), Some("LockedOut"));
    }

    #[test]
    fn removed_plans_alone_and_no_plan_at_all_say_nothing() {
        assert_eq!(exchange_state(&plans(&[("exchange", "Deleted")])), None);
        assert_eq!(exchange_state(&plans(&[("To-Do", "Warning")])), None);
        assert_eq!(exchange_state(&serde_json::json!({})), None);
    }

    #[test]
    fn the_log_line_names_each_plan_and_its_state() {
        let body = plans(&[("exchange", "Warning"), ("SharePoint", "Enabled")]);
        assert_eq!(
            summary(&body),
            "accountEnabled=true assignedLicenses=1 \
             assignedPlans=[exchange=Warning, SharePoint=Enabled]"
        );
        let refused = serde_json::json!({"error": {"code": "Forbidden"}});
        assert_eq!(summary(&refused), "error code=Forbidden");
    }
}
