use crate::discovery::autoconfig;
use crate::discovery::registry;
use crate::discovery::types::{DiscoverySource, Protocol, ProtocolOption};
use hickory_resolver::TokioResolver;
use std::time::Duration;

/// Stage timeout for the entire MX lookup stage.
const MX_TIMEOUT: Duration = Duration::from_secs(5);

/// Stage 3: DNS MX lookup → extract base domain → re-check registry + autoconfig.
pub async fn lookup(domain: &str, email: &str) -> (Vec<ProtocolOption>, Option<String>) {
    match tokio::time::timeout(MX_TIMEOUT, lookup_inner(domain, email)).await {
        Ok(result) => result,
        Err(_) => {
            log::debug!("MX lookup for {domain} timed out after 5s");
            (Vec::new(), None)
        }
    }
}

async fn lookup_inner(domain: &str, email: &str) -> (Vec<ProtocolOption>, Option<String>) {
    let resolver =
        match TokioResolver::builder_tokio().and_then(hickory_resolver::ResolverBuilder::build) {
            Ok(r) => r,
            Err(e) => {
                log::debug!("MX resolver creation failed: {e}");
                return (Vec::new(), None);
            }
        };

    let mx_records = match resolver.mx_lookup(domain).await {
        Ok(records) => records,
        Err(e) => {
            log::debug!("MX lookup for {domain} failed: {e}");
            return (Vec::new(), None);
        }
    };

    // Sort by preference (lower = higher priority)
    let mut exchanges: Vec<String> = mx_records
        .answers()
        .iter()
        .filter_map(|rec| {
            if let hickory_resolver::proto::rr::RData::MX(mx) = &rec.data {
                Some(mx.exchange.to_utf8().trim_end_matches('.').to_string())
            } else {
                None
            }
        })
        .collect();
    exchanges.sort();
    exchanges.dedup();

    for mx_host in &exchanges {
        // Progressive suffix stripping: try each suffix against the registry
        if let Some(result) = try_registry_match(mx_host) {
            return result;
        }

        // Try autoconfig on the MX base domain (eTLD+1 approximation)
        if let Some(base) = extract_base_domain(mx_host)
            && base != domain
        {
            let results = autoconfig::fetch(&base, email).await;
            if !results.is_empty() {
                let retagged = retag_results(results, mx_host);
                return (retagged, Some(base));
            }
        }
    }

    (Vec::new(), None)
}

fn try_registry_match(mx_host: &str) -> Option<(Vec<ProtocolOption>, Option<String>)> {
    let segments: Vec<&str> = mx_host.split('.').collect();
    for start in 0..segments.len().saturating_sub(1) {
        let candidate = segments[start..].join(".");
        let mut results = registry::lookup(&candidate);
        if is_microsoft_365_mx(mx_host) {
            // The outlook.com entry also offers consumer IMAP, whose
            // `consumers` endpoint rejects work and school accounts.
            results.retain(|opt| matches!(opt.protocol, Protocol::MicrosoftGraph));
        }
        if !results.is_empty() {
            let retagged = retag_results(results, mx_host);
            return Some((retagged, Some(candidate)));
        }
    }
    None
}

/// Exchange Online routes every Microsoft 365 tenant domain through
/// `<tenant>.mail.protection.outlook.com`.
fn is_microsoft_365_mx(mx_host: &str) -> bool {
    mx_host
        .to_ascii_lowercase()
        .ends_with(".mail.protection.outlook.com")
}

fn retag_results(results: Vec<ProtocolOption>, mx_host: &str) -> Vec<ProtocolOption> {
    results
        .into_iter()
        .map(|mut opt| {
            opt.source = DiscoverySource::MxLookup {
                mx_domain: mx_host.to_string(),
            };
            opt
        })
        .collect()
}

/// Simple base domain extraction: take last 2 segments, with exceptions
/// for known multi-part TLDs.
fn extract_base_domain(hostname: &str) -> Option<String> {
    let segments: Vec<&str> = hostname.split('.').collect();
    if segments.len() < 2 {
        return None;
    }

    // Known multi-part TLDs where we need 3 segments
    let multi_part_tlds = [
        "co.uk", "co.jp", "com.au", "com.br", "co.in", "co.nz", "co.za", "com.mx", "com.ar",
        "com.sg",
    ];

    let last_two = format!(
        "{}.{}",
        segments[segments.len() - 2],
        segments[segments.len() - 1]
    );

    if multi_part_tlds.contains(&last_two.as_str()) && segments.len() >= 3 {
        Some(format!("{}.{last_two}", segments[segments.len() - 3],))
    } else {
        Some(last_two)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocols(mx_host: &str) -> Vec<Protocol> {
        try_registry_match(mx_host)
            .expect("registry match")
            .0
            .into_iter()
            .map(|opt| opt.protocol)
            .collect()
    }

    #[test]
    fn microsoft_365_mx_offers_only_graph() {
        let found = protocols("pinelli-fr.mail.protection.outlook.com");
        assert_eq!(found.len(), 1);
        assert!(matches!(found[0], Protocol::MicrosoftGraph));
    }

    #[test]
    fn consumer_outlook_mx_keeps_imap() {
        let found = protocols("outlook-com.olc.protection.outlook.com");
        assert!(found.iter().any(|p| matches!(p, Protocol::Imap { .. })));
    }
}
