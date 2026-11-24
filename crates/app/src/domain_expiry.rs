//! When the domain name of an account expires.
//!
//! An address at one's own domain stops working the day the domain is not
//! renewed, at any host, and nothing in a sync says so beforehand. The
//! expiry date of a domain is public: its registry publishes it over RDAP.
//! Once a day the app reads it for the domain of each account and warns
//! when it comes close.
//!
//! The registry of a top-level domain is found in IANA's bootstrap file,
//! then asked directly: the only thing sent out is the domain name, to the
//! registry that already lists it. Registries without RDAP, and mailbox
//! providers such as gmail.com, are skipped: nothing is said about them.

use std::time::Duration;

use serde_json::Value;

const BOOTSTRAP_URL: &str = "https://data.iana.org/rdap/dns.json";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long before the expiry of its domain an account is warned about. A
/// registrar renews most domains by itself a few days before, so a longer
/// notice would mostly warn for nothing.
pub const WARN_DAYS: i64 = 30;
/// How often the registries are asked.
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);

/// A domain and the whole days left before it expires, negative once it
/// has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainExpiry {
    pub domain: String,
    pub days_left: i64,
}

/// The registrable domain an address depends on, when its owner may be the
/// one who has to renew it. `None` for a mailbox provider and for an
/// address without a usable domain.
pub fn owned_domain(address: &str) -> Option<String> {
    rtsk::sender_logo::logo_domains(address).pop()
}

/// Days left for each domain whose registry answers. A domain that is
/// missing from the result is one nothing could be learnt about.
pub async fn check(domains: Vec<String>) -> Vec<DomainExpiry> {
    if domains.is_empty() {
        return Vec::new();
    }
    let Ok(http) = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build() else {
        return Vec::new();
    };
    let Some(bootstrap) = fetch_json(&http, BOOTSTRAP_URL).await else {
        log::warn!("domain expiry: the RDAP bootstrap file could not be read");
        return Vec::new();
    };
    let now = jiff::Timestamp::now();
    let mut found = Vec::new();
    for domain in domains {
        let Some(base) = rdap_base(&bootstrap, &domain) else {
            log::info!("domain expiry: no RDAP registry for {domain}");
            continue;
        };
        let url = format!("{}/domain/{domain}", base.trim_end_matches('/'));
        let Some(expires) = fetch_json(&http, &url).await.as_ref().and_then(expiration) else {
            log::info!("domain expiry: no expiry date for {domain}");
            continue;
        };
        let days_left = days_between(now, expires);
        log::info!("domain expiry: {domain} expires in {days_left} days");
        found.push(DomainExpiry { domain, days_left });
    }
    found
}

async fn fetch_json(http: &reqwest::Client, url: &str) -> Option<Value> {
    let response = http
        .get(url)
        .header(
            reqwest::header::ACCEPT,
            "application/rdap+json, application/json",
        )
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

/// The RDAP server of the registry of `domain`, from IANA's bootstrap
/// file: a list of `[[top-level domains], [server URLs]]` entries.
fn rdap_base(bootstrap: &Value, domain: &str) -> Option<String> {
    let top_level = domain.rsplit('.').next()?;
    bootstrap["services"].as_array()?.iter().find(|service| {
        service[0]
            .as_array()
            .is_some_and(|tlds| tlds.iter().any(|tld| tld.as_str() == Some(top_level)))
    })?[1]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find(|url| url.starts_with("https://"))
        .map(str::to_owned)
}

/// The expiry date in a registry's answer about a domain.
fn expiration(answer: &Value) -> Option<jiff::Timestamp> {
    answer["events"]
        .as_array()?
        .iter()
        .find(|event| event["eventAction"].as_str() == Some("expiration"))?["eventDate"]
        .as_str()?
        .parse()
        .ok()
}

/// Whole days from `now` to `then`, rounded down: a domain that expires
/// in 36 hours has one day left, one that expired an hour ago has -1.
fn days_between(now: jiff::Timestamp, then: jiff::Timestamp) -> i64 {
    (then.as_second() - now.as_second()).div_euclid(86_400)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bootstrap() -> Value {
        serde_json::json!({
            "services": [
                [["com", "net"], ["http://old.example/", "https://rdap.verisign.example/com/v1/"]],
                [["fr"], ["https://rdap.nic.example/"]]
            ]
        })
    }

    #[test]
    fn the_registry_of_a_domain_is_found_by_its_top_level_domain() {
        assert_eq!(
            rdap_base(&bootstrap(), "monapp.com").as_deref(),
            Some("https://rdap.verisign.example/com/v1/")
        );
        assert_eq!(
            rdap_base(&bootstrap(), "monapp.fr").as_deref(),
            Some("https://rdap.nic.example/")
        );
        assert_eq!(rdap_base(&bootstrap(), "monapp.io"), None);
    }

    #[test]
    fn the_expiry_date_is_the_expiration_event_of_the_answer() {
        let answer = serde_json::json!({
            "events": [
                {"eventAction": "registration", "eventDate": "2001-01-13T00:12:14Z"},
                {"eventAction": "expiration", "eventDate": "2027-01-13T00:12:14Z"}
            ]
        });
        let expected: jiff::Timestamp = "2027-01-13T00:12:14Z".parse().expect("a date");
        assert_eq!(expiration(&answer), Some(expected));
        assert_eq!(expiration(&serde_json::json!({"events": []})), None);
        assert_eq!(expiration(&serde_json::json!({})), None);
    }

    #[test]
    fn days_left_round_down_and_go_negative_once_expired() {
        let now: jiff::Timestamp = "2026-11-24T12:00:00Z".parse().expect("a date");
        let in_36_hours: jiff::Timestamp = "2026-11-26T00:00:00Z".parse().expect("a date");
        let an_hour_ago: jiff::Timestamp = "2026-11-24T11:00:00Z".parse().expect("a date");
        assert_eq!(days_between(now, in_36_hours), 1);
        assert_eq!(days_between(now, an_hour_ago), -1);
    }

    #[tokio::test]
    #[ignore = "asks the public registries: run by hand"]
    async fn the_registries_answer_for_well_known_domains() {
        let found = check(vec!["wikipedia.org".to_owned(), "afnic.fr".to_owned()]).await;
        let domains: Vec<&str> = found.iter().map(|expiry| expiry.domain.as_str()).collect();
        assert_eq!(domains, ["wikipedia.org", "afnic.fr"]);
        assert!(found.iter().all(|expiry| expiry.days_left > 0), "{found:?}");
    }

    #[test]
    fn a_mailbox_provider_is_not_a_domain_to_renew() {
        assert_eq!(owned_domain("ann@gmail.com"), None);
        assert_eq!(
            owned_domain("ann@mail.monapp.com").as_deref(),
            Some("monapp.com")
        );
    }
}
