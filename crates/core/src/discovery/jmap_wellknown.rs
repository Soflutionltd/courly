use crate::discovery::types::{AuthConfig, AuthMethod, DiscoverySource, Protocol, ProtocolOption};

/// Stage 4: Probe `.well-known/jmap` for JMAP support (RFC 8620 §2.2).
pub async fn probe(domain: &str) -> Option<ProtocolOption> {
    let url = format!("https://{domain}/.well-known/jmap");

    let client = super::discovery_client()?;

    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }

    // Validate it looks like a JMAP session resource
    let body = super::read_capped_text(resp).await?;
    if !body.contains("capabilities") {
        return None;
    }

    Some(ProtocolOption {
        protocol: Protocol::Jmap { session_url: url },
        auth: AuthConfig {
            method: AuthMethod::Password,
            alternatives: Vec::new(),
        },
        provider_name: None,
        help_url: None,
        source: DiscoverySource::JmapWellKnown,
    })
}
