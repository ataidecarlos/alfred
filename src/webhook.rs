//! Allow-listed outbound webhooks for the agent.
//!
//! `alfred webhook send` is the only way the agent can make an outbound HTTP
//! request. The allow-list lives in `[webhook] allowed_hosts`; an empty list
//! denies every host, the default, because the agent runs unattended on a 24x7
//! host.
//!
//! The check happens in Alfred before any socket is opened, so a host outside
//! the list is refused here and never reaches Pi's raw `bash`.

use serde_json::Value;

use crate::error::AlfredError;

/// A completed webhook request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body, as returned by the server.
    pub body: String,
}

/// The host component of `url`, lowercased and without a port or user info.
///
/// A URL with no host — a relative path, or a `file:` URL — is rejected.
pub fn host_of(url: &str) -> Result<String, AlfredError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| AlfredError::Usage(format!("invalid webhook url '{url}': {error}")))?;
    match parsed.host_str() {
        Some(host) => Ok(host.trim_end_matches('.').to_ascii_lowercase()),
        None => Err(AlfredError::Usage(format!(
            "invalid webhook url '{url}': no host"
        ))),
    }
}

/// Refuse `host` unless it is on the allow-list.
///
/// Entries are compared as bare host names, case-insensitively, ignoring a
/// trailing dot. These are refused by Alfred, never by Pi.
pub fn check_allowed(host: &str, allowed_hosts: &[String]) -> Result<(), AlfredError> {
    let allowed = allowed_hosts.iter().any(|entry| {
        let entry = entry.trim().trim_end_matches('.').to_ascii_lowercase();
        !entry.is_empty() && entry == host
    });
    if allowed {
        Ok(())
    } else {
        Err(AlfredError::HostNotAllowed(host.to_string()))
    }
}

/// POST `body` as JSON to `url`, after enforcing the allow-list.
///
/// Redirects are not followed: an allow-listed host must not be able to bounce
/// the request to a host that is not on the list.
pub async fn send(
    url: &str,
    body: &Value,
    allowed_hosts: &[String],
) -> Result<WebhookResponse, AlfredError> {
    let host = host_of(url)?;
    check_allowed(&host, allowed_hosts)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = client.post(url).json(body).send().await?;
    let status = response.status().as_u16();
    let body = response.text().await?;
    Ok(WebhookResponse { status, body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn hosts(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|entry| entry.to_string()).collect()
    }

    #[test]
    fn host_is_extracted_lowercased_without_port() {
        assert_eq!(
            host_of("https://EXAMPLE.com:8443/x").unwrap(),
            "example.com"
        );
        assert_eq!(host_of("http://example.com./x").unwrap(), "example.com");
        assert_eq!(host_of("http://127.0.0.1:8080/x").unwrap(), "127.0.0.1");
    }

    #[test]
    fn hostless_urls_are_rejected() {
        assert!(host_of("not a url").is_err());
        assert!(host_of("file:///etc/hosts").is_err());
    }

    #[test]
    fn empty_allow_list_denies_every_host() {
        let error = check_allowed("example.com", &[]).expect_err("must deny");
        assert_eq!(error.to_string(), "host not allowed: example.com");
    }

    #[test]
    fn allow_list_matches_case_insensitively() {
        check_allowed("example.com", &hosts(&["Example.COM"])).expect("allowed");
        assert!(check_allowed("evil.com", &hosts(&["example.com"])).is_err());
        // A blank entry never matches, even for a blank host.
        assert!(check_allowed("", &hosts(&["", "  "])).is_err());
    }

    #[tokio::test]
    async fn send_refuses_a_host_off_the_allow_list_before_connecting() {
        let error = send("https://example.com/x", &serde_json::json!({}), &[])
            .await
            .expect_err("must refuse");
        assert_eq!(error.to_string(), "host not allowed: example.com");
    }

    #[tokio::test]
    async fn send_posts_json_to_an_allowed_host() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buffer = [0u8; 4096];
            // Read (at least part of) the request before replying.
            let _ = socket.read(&mut buffer).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .expect("write");
        });

        let url = format!("http://{addr}/hook");
        let response = send(
            &url,
            &serde_json::json!({"ping": true}),
            &hosts(&["127.0.0.1"]),
        )
        .await
        .expect("allowed host is sent");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, "ok");
        server.await.expect("server task");
    }

    #[tokio::test]
    async fn send_does_not_follow_redirects_to_an_unlisted_host() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buffer = [0u8; 4096];
            let _ = socket.read(&mut buffer).await;
            // A redirect the client must not follow.
            let response =
                "HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            socket.write_all(response.as_bytes()).await.expect("write");
        });

        let url = format!("http://{addr}/hook");
        let response = send(&url, &serde_json::json!({}), &hosts(&["127.0.0.1"]))
            .await
            .expect("request is sent");
        assert_eq!(response.status, 302, "the redirect must not be followed");
        server.await.expect("server task");
    }
}
