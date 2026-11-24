//! Diagnostic: what Microsoft reports about the licence of an Exchange
//! account.
//!
//! A mailbox whose subscription lapsed keeps syncing without any error, so
//! nothing in a sync says it is about to be deleted. This probe asks Graph,
//! once per account and per run of the Service, for the state of the
//! account's licence, and writes the answer to the log. It reads nothing
//! else and changes nothing: it exists to learn which field carries the
//! warning Outlook shows, before an alert is built on it.
//!
//! The log holds the account id, HTTP statuses, licence and plan names and
//! their states. No address, name or token is written.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use conduit_net::TokenSource;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";
/// The engine refreshes an expired token as it starts syncing; the probe
/// waits for that instead of refreshing on its own.
const START_DELAY: Duration = Duration::from_secs(45);
const RETRY_DELAY: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Probe the account in the background, the first time it is seen.
pub(crate) fn spawn(account_id: String, source: Arc<dyn TokenSource>) {
    static PROBED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let first_time = PROBED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(account_id.clone());
    if !first_time {
        return;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        log::warn!("[licence probe] {account_id}: no runtime, not probed");
        return;
    };
    runtime.spawn(async move {
        tokio::time::sleep(START_DELAY).await;
        if !probe(&account_id, source.as_ref()).await {
            tokio::time::sleep(RETRY_DELAY).await;
            probe(&account_id, source.as_ref()).await;
        }
    });
}

/// Ask Graph and log the answers. `false` when the token was refused, so
/// the caller tries again once the engine has refreshed it.
async fn probe(account_id: &str, source: &dyn TokenSource) -> bool {
    let token = match source.current().await {
        Ok(token) => token,
        Err(error) => {
            log::warn!("[licence probe] {account_id}: no token: {error}");
            return true;
        }
    };
    let http = match reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build() {
        Ok(http) => http,
        Err(error) => {
            log::warn!("[licence probe] {account_id}: no HTTP client: {error}");
            return true;
        }
    };
    let questions = [
        (
            "plans",
            "me?$select=accountEnabled,assignedLicenses,assignedPlans",
        ),
        ("licences", "me/licenseDetails"),
        ("mailbox", "me/mailboxSettings?$select=userPurpose"),
        ("inbox", "me/mailFolders/inbox?$select=id"),
    ];
    for (name, path) in questions {
        let response = http
            .get(format!("{GRAPH}/{path}"))
            .bearer_auth(token.as_str())
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                log::warn!("[licence probe] {account_id} {name}: request failed: {error}");
                continue;
            }
        };
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            log::info!("[licence probe] {account_id} {name}: HTTP 401, token not fresh yet");
            return false;
        }
        let body: serde_json::Value = response.json().await.unwrap_or_default();
        log::info!(
            "[licence probe] {account_id} {name}: HTTP {} {}",
            status.as_u16(),
            summary(name, &body)
        );
    }
    true
}

/// The fields of an answer that say something about the licence, and the
/// error code when Graph refused.
fn summary(name: &str, body: &serde_json::Value) -> String {
    if let Some(error) = body.get("error") {
        return format!(
            "error code={} inner={}",
            error["code"].as_str().unwrap_or("?"),
            error["innerError"]["code"].as_str().unwrap_or("-")
        );
    }
    match name {
        "plans" => {
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
        "licences" => {
            let licences: Vec<String> = body["value"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|licence| {
                    let plans: Vec<String> = licence["servicePlans"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|plan| {
                            format!(
                                "{}={}",
                                plan["servicePlanName"].as_str().unwrap_or("?"),
                                plan["provisioningStatus"].as_str().unwrap_or("?")
                            )
                        })
                        .collect();
                    format!(
                        "{} [{}]",
                        licence["skuPartNumber"].as_str().unwrap_or("?"),
                        plans.join(", ")
                    )
                })
                .collect();
            format!("licences={} {}", licences.len(), licences.join(" ; "))
        }
        "mailbox" => format!("userPurpose={}", body["userPurpose"]),
        _ => "ok".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::summary;

    #[test]
    fn the_state_of_each_plan_is_reported() {
        let body = serde_json::json!({
            "accountEnabled": true,
            "assignedLicenses": [{}],
            "assignedPlans": [
                {"service": "exchange", "capabilityStatus": "Warning"},
                {"service": "SharePoint", "capabilityStatus": "Enabled"}
            ]
        });
        assert_eq!(
            summary("plans", &body),
            "accountEnabled=true assignedLicenses=1 \
             assignedPlans=[exchange=Warning, SharePoint=Enabled]"
        );
    }

    #[test]
    fn a_refusal_reports_its_error_code() {
        let body = serde_json::json!({
            "error": {"code": "Forbidden", "innerError": {"code": "MailboxNotEnabled"}}
        });
        assert_eq!(
            summary("mailbox", &body),
            "error code=Forbidden inner=MailboxNotEnabled"
        );
    }
}
