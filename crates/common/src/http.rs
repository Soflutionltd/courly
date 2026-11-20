/// Process-wide shared `reqwest::Client` singleton.
///
/// All crates that need a plain HTTP client should call this rather than
/// constructing their own `reqwest::Client`.  The singleton is lazily
/// initialized on first access and lives for the lifetime of the process.
pub fn shared_http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// Client for OAuth token endpoint POSTs (code exchange and refresh). The
/// form body carries the code or refresh token and the client secret, so
/// redirects are refused (a 307/308 would replay that body to the new
/// location), and a hung endpoint times out instead of stalling every
/// refresh behind it.
pub fn oauth_token_http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            // Same failure mode as `reqwest::Client::new`: only a TLS
            // backend that cannot initialise.
            .expect("TLS backend initialises")
    })
}

/// Check HTTP response status, returning error details on failure.
///
/// `provider` is a prefix for error messages (e.g. "Gmail API", "Graph API").
pub async fn check_response_status(
    response: reqwest::Response,
    provider: &str,
) -> Result<(), String> {
    if response.status().is_success() || response.status().as_u16() == 204 {
        return Ok(());
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(format!("{provider} error: {status} {body}"))
}

/// Parse a JSON response, handling 204 No Content.
///
/// `provider` is a prefix for error messages (e.g. "Gmail API", "Graph API").
pub async fn parse_json_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    provider: &str,
) -> Result<T, String> {
    let status = response.status();

    if !status.is_success() && status.as_u16() != 204 {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("{provider} error: {status} {body}"));
    }

    if status.as_u16() == 204 {
        // For 204 No Content, try to create a default value
        return serde_json::from_str("null")
            .map_err(|e| format!("Cannot deserialize null for 204 response: {e}"));
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse {provider} response: {e}"))
}

/// Retry configuration for HTTP requests.
pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_ms: 1000,
        }
    }
}

/// Compute retry delay from a `Retry-After` header (seconds) or exponential
/// backoff. Respects the server's `Retry-After` value when present.
pub fn compute_retry_delay(
    response: Option<&reqwest::Response>,
    attempt: u32,
    config: &RetryConfig,
) -> u64 {
    if let Some(resp) = response
        && let Some(retry_after) = resp.headers().get("Retry-After")
        && let Ok(s) = retry_after.to_str()
        && let Ok(secs) = s.parse::<u64>()
    {
        return secs * 1000;
    }
    config.initial_backoff_ms * 2u64.pow(attempt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A 307 would make reqwest replay the POST body (refresh token,
    /// client secret) to the Location; the token client must stop there.
    #[tokio::test]
    async fn oauth_token_client_does_not_follow_redirects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let reply = format!(
                    "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{addr}/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });

        // Following would loop on the 307 until reqwest's redirect limit
        // turns it into an error.
        let response = oauth_token_http_client()
            .post(format!("http://{addr}/token"))
            .form(&[("refresh_token", "secret")])
            .send()
            .await
            .expect("send");
        server.abort();
        assert_eq!(response.status().as_u16(), 307);
    }
}
