//! Hardened outbound HTTP for fetching sender-controlled URLs.
//!
//! Email content is attacker-controlled, so any URL Skim fetches on the user's
//! behalf is an SSRF boundary: the host must resolve only to public addresses,
//! and those addresses are pinned so a second DNS answer can't swap in a
//! private one after the check. Redirects are followed manually, re-vetting
//! every hop. Reused by the RFC 8058 one-click unsubscribe (`mail::sync`) and
//! by the AI `fetch_url` tool (`ai::agent`).

use crate::error::{Result, SkimError};
use crate::mail::parse::html_to_text;
use futures::StreamExt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

/// How long a single request may take before we give up.
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
/// Redirect hops we follow before refusing to chase further.
const MAX_REDIRECTS: usize = 3;

/// Vet a sender-controlled URL before fetching it. When `require_https`, plain
/// `http:` is rejected outright; otherwise both http and https are allowed. The
/// host must resolve, and every resolved address must be public — otherwise a
/// crafted URL would turn the fetch into a probe of the user's LAN or a cloud
/// metadata endpoint. Returns the parsed URL plus the vetted addresses so the
/// caller can pin them. `code` is the error category surfaced to the frontend.
pub async fn vet_public_url(
    url: &str,
    require_https: bool,
    code: &'static str,
) -> Result<(reqwest::Url, Vec<SocketAddr>)> {
    let parsed =
        reqwest::Url::parse(url).map_err(|e| SkimError::other(code, format!("bad url: {e}")))?;
    let scheme = parsed.scheme();
    if require_https {
        if scheme != "https" {
            return Err(SkimError::other(code, "url is not https"));
        }
    } else if scheme != "https" && scheme != "http" {
        return Err(SkimError::other(code, "url is not http(s)"));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| SkimError::other(code, "url has no host"))?
        .to_string();
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| SkimError::other(code, "url has no port"))?;
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| SkimError::other(code, format!("cannot resolve {host}: {e}")))?
        .collect();
    if addrs.is_empty() {
        return Err(SkimError::other(code, format!("{host} did not resolve")));
    }
    if addrs.iter().any(|a| !is_public_ip(a.ip())) {
        return Err(SkimError::other(
            code,
            format!("{host} resolves to a non-public address"),
        ));
    }
    Ok((parsed, addrs))
}

/// Fetch a web page and return it as readable text, capped at `max_bytes` of
/// downloaded body. https-only, every hop vetted through [`vet_public_url`] and
/// its addresses pinned; redirects are followed manually (up to [`MAX_REDIRECTS`])
/// so a redirect can't hop to an internal host. HTML is reduced to text with
/// [`html_to_text`]; other text types are returned as-is; binary/unknown types
/// are refused. The returned text is untrusted web content — callers must never
/// treat it as instructions.
pub async fn fetch_page_text(url: &str, max_bytes: usize) -> Result<String> {
    let mut current = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let (target, addrs) = vet_public_url(&current, true, "fetch_url").await?;
        let host = target.host_str().unwrap_or_default().to_string();
        let client = reqwest::Client::builder()
            // Don't let reqwest auto-follow: each hop must be re-vetted, so we
            // handle 3xx ourselves.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(FETCH_TIMEOUT)
            .resolve_to_addrs(&host, &addrs)
            .build()
            .map_err(|e| SkimError::other("fetch_url", e.to_string()))?;
        let resp = client
            .get(target.clone())
            .header("Accept", "text/html,text/plain")
            .send()
            .await
            .map_err(|e| SkimError::other("fetch_url", e.to_string()))?;

        let status = resp.status();
        if status.is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| SkimError::other("fetch_url", "redirect without a location"))?;
            // Resolve relative redirects against the URL we just fetched.
            current = target
                .join(location)
                .map_err(|e| SkimError::other("fetch_url", format!("bad redirect: {e}")))?
                .to_string();
            continue;
        }
        if !status.is_success() {
            return Err(SkimError::other(
                "fetch_url",
                format!("the page returned {status}"),
            ));
        }

        // Only fetch text; refuse binaries so we never pull down a large file
        // or something the model can't use.
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_html =
            content_type.contains("text/html") || content_type.contains("application/xhtml");
        let is_text = is_html || content_type.starts_with("text/") || content_type.is_empty();
        if !is_text {
            return Err(SkimError::other(
                "fetch_url",
                format!("the page is not text ({content_type})"),
            ));
        }

        // Read the body with a hard cap so a huge page can't blow up memory.
        let mut buf: Vec<u8> = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| SkimError::other("fetch_url", e.to_string()))?;
            let room = max_bytes.saturating_sub(buf.len());
            if room == 0 {
                break;
            }
            let take = room.min(chunk.len());
            buf.extend_from_slice(&chunk[..take]);
            if buf.len() >= max_bytes {
                break;
            }
        }
        let raw = String::from_utf8_lossy(&buf);
        let text = if is_html {
            html_to_text(&raw)
        } else {
            raw.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        return Ok(text);
    }
    Err(SkimError::other("fetch_url", "too many redirects"))
}

/// True when the address is routable on the public internet — not loopback,
/// LAN, link-local (cloud metadata), CGNAT, or another reserved range.
/// A hand-rolled check because `IpAddr::is_global` is still unstable.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_public_v4(mapped);
            }
            let seg0 = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (seg0 & 0xfe00) == 0xfc00 // unique-local fc00::/7
                || (seg0 & 0xffc0) == 0xfe80) // link-local fe80::/10
        }
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local() // 169.254/16 — cloud metadata lives here
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || o[0] == 0 // "this network" 0.0.0.0/8
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // CGNAT 100.64/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF protocol 192.0.0.0/24
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // benchmarking 198.18/15
        || o[0] >= 240) // reserved 240/4
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn rejects_non_public_addresses() {
        for bad in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254", // cloud metadata
            "0.0.0.0",
            "100.64.0.1", // CGNAT
            "198.18.0.1", // benchmarking
            "255.255.255.255",
            "240.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12::1",
            "ff02::1",
            "::ffff:192.168.1.1", // v4-mapped private
        ] {
            assert!(!is_public_ip(ip(bad)), "{bad} should be rejected");
        }
    }

    #[test]
    fn accepts_public_addresses() {
        for good in [
            "93.184.216.34",
            "8.8.8.8",
            "1.1.1.1",
            "2606:2800:220:1::1",
            "::ffff:8.8.8.8", // public v4 mapped into v6 stays public
        ] {
            assert!(is_public_ip(ip(good)), "{good} should be accepted");
        }
    }

    #[tokio::test]
    async fn vet_rejects_http_when_https_required() {
        let err = vet_public_url("http://example.com/", true, "fetch_url")
            .await
            .unwrap_err();
        assert_eq!(err.code(), "fetch_url");
    }

    #[tokio::test]
    async fn vet_rejects_loopback_host() {
        // Resolves to 127.0.0.1 without any network.
        let err = vet_public_url("https://localhost/", true, "fetch_url")
            .await
            .unwrap_err();
        assert_eq!(err.code(), "fetch_url");
    }
}
