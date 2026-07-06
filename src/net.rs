//! HTTP transport to the Acorns GraphQL endpoint + token refresh.

use crate::session::Session;
use reqwest::blocking::{Client, RequestBuilder, Response};
use serde_json::{Value, json};
use std::time::Duration;

pub const ENDPOINT: &str = "https://graphql.acorns.com/graphql";
pub const REFRESH_HASH: &str = "ebdecb9e33a90bcce01717ae5e77f7409dcf727cb9c0230bb725f7e9cd4ef9a2";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36";

/// Transient statuses worth one short retry: rate limit / bad gateway / unavailable.
const RETRYABLE: [u16; 3] = [429, 502, 503];
const MAX_RETRIES: u32 = 2;

pub fn client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build HTTP client")
}

/// Common headers shared by every request (auth routing + client identity).
fn base(c: &Client) -> RequestBuilder {
    c.post(ENDPOINT)
        .header("content-type", "application/json")
        .header("auth-strategy", "jwt")
        .header("x-client-auth-method", "cookies")
        .header("x-client-app", "web-app")
        .header("x-client-platform", "web")
        .header("x-client-build", "4.179.0")
        .header("user-agent", UA)
        .header("origin", "https://app.acorns.com")
        .header("referer", "https://app.acorns.com/")
}

/// POST an authenticated GraphQL batch body (`[{...}]`), returning parsed JSON.
/// Briefly retries on 429/502/503 (respecting `Retry-After` when present).
pub fn post_authed(c: &Client, s: &Session, body: &Value) -> anyhow::Result<Value> {
    let payload = serde_json::to_vec(body)?;
    let cookie = format!(
        "acornsweb_access_token={}; acornsweb_csrf_token={}",
        s.access_token, s.csrf_token
    );
    let mut attempt: u32 = 0;
    loop {
        let resp = base(c)
            .header("cookie", &cookie)
            .header("x-csrf-token", &s.csrf_token)
            .body(payload.clone())
            .send()?;
        let status = resp.status();
        if attempt < MAX_RETRIES && RETRYABLE.contains(&status.as_u16()) {
            let wait = retry_after_secs(resp.headers()).unwrap_or(1u64 << attempt);
            eprintln!("(net) HTTP {status}; retrying in {wait}s…");
            std::thread::sleep(Duration::from_secs(wait));
            attempt += 1;
            continue;
        }
        return json_body(resp);
    }
}

fn retry_after_secs(h: &reqwest::header::HeaderMap) -> Option<u64> {
    h.get("retry-after")?.to_str().ok()?.trim().parse().ok()
}

/// Parse a response body as JSON, or fail with the HTTP status and a body
/// snippet. (An HTML error page from a proxy would otherwise surface as an
/// unhelpful "error decoding response body".)
fn json_body(resp: Response) -> anyhow::Result<Value> {
    let status = resp.status();
    let bytes = resp.bytes()?;
    serde_json::from_slice(&bytes).map_err(|_| {
        let snippet: String = String::from_utf8_lossy(&bytes).chars().take(200).collect();
        anyhow::anyhow!("HTTP {status} — non-JSON response: {snippet}")
    })
}

/// Exchange the refresh-token cookie for fresh tokens (rotates the family).
pub fn refresh(c: &Client, s: &Session) -> anyhow::Result<Session> {
    let body = json!([{
        "operationName": "RefreshAuthTokens",
        "variables": { "refreshToken": "" },
        "extensions": { "persistedQuery": { "version": 1, "sha256Hash": REFRESH_HASH } }
    }]);
    let cookie = format!("acornsweb_refresh_token={}", s.refresh_token);
    let resp = base(c)
        .header("cookie", cookie)
        .body(serde_json::to_vec(&body)?)
        .send()?;
    let status = resp.status();

    // Capture rotated cookies before consuming the body.
    let mut ns = s.clone();
    let mut got_access = false;
    for hv in resp.headers().get_all("set-cookie") {
        if let Ok(sc) = hv.to_str() {
            if let Some((k, v)) = parse_cookie(sc) {
                match k.as_str() {
                    "acornsweb_access_token" => {
                        ns.access_token = v;
                        got_access = true;
                    }
                    "acornsweb_refresh_token" => ns.refresh_token = v,
                    "acornsweb_csrf_token" => ns.csrf_token = v,
                    _ => {}
                }
            }
        }
    }

    // Once rotated cookies have arrived, the OLD refresh token is already dead
    // server-side — a malformed body must not lose the new tokens.
    let j: Value = match json_body(resp) {
        Ok(j) => j,
        Err(e) => {
            if got_access {
                return Ok(ns);
            }
            return Err(e.context("token refresh failed"));
        }
    };
    if let Some(t) = j
        .get(0)
        .and_then(|x| x.get("data"))
        .and_then(|d| d.get("refreshAuthToken"))
        .and_then(|r| r.get("__typename"))
        .and_then(Value::as_str)
    {
        if t.contains("Expired") || t.contains("Invalid") {
            anyhow::bail!("session expired/invalid — run `acorns auth login`");
        }
    }
    if !got_access {
        anyhow::bail!("token refresh failed (HTTP {status}) — run `acorns auth login`");
    }
    Ok(ns)
}

/// POST with an optional raw Cookie header, returning the JSON body plus any
/// `Set-Cookie` pairs. Used by the multi-step login flow (which threads cookies
/// across `ValidateCredentials` -> `InitiateChallenge` -> `VendAuthSession`).
pub fn post_collect(
    c: &Client,
    cookie_header: Option<String>,
    body: &Value,
) -> anyhow::Result<(Value, Vec<(String, String)>)> {
    let mut rb = base(c);
    if let Some(ch) = cookie_header {
        if !ch.is_empty() {
            rb = rb.header("cookie", ch);
        }
    }
    let resp = rb.body(serde_json::to_vec(body)?).send()?;
    let cookies: Vec<(String, String)> = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|h| h.to_str().ok())
        .filter_map(parse_cookie)
        .collect();
    let j = json_body(resp)?;
    Ok((j, cookies))
}

/// Plain HTTP GET to a URL, saving the body to `path`. Returns bytes written.
/// (Used for presigned tax-statement PDF links — no auth needed.)
pub fn download_to(url: &str, path: &std::path::Path) -> anyhow::Result<u64> {
    let resp = client().get(url).send()?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("download failed: HTTP {status}");
    }
    let bytes = resp.bytes()?;
    std::fs::write(path, &bytes)?;
    Ok(bytes.len() as u64)
}

fn parse_cookie(sc: &str) -> Option<(String, String)> {
    let first = sc.split(';').next()?;
    let (k, v) = first.split_once('=')?;
    Some((k.trim().to_string(), v.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::parse_cookie;

    #[test]
    fn cookie_with_attributes() {
        assert_eq!(
            parse_cookie("acornsweb_access_token=abc123; Path=/; HttpOnly; Secure"),
            Some(("acornsweb_access_token".into(), "abc123".into()))
        );
    }

    #[test]
    fn cookie_bare_pair_and_whitespace() {
        assert_eq!(parse_cookie(" k = v "), Some(("k".into(), "v".into())));
    }

    #[test]
    fn cookie_value_containing_equals() {
        assert_eq!(
            parse_cookie("k=v=w; Path=/"),
            Some(("k".into(), "v=w".into()))
        );
    }

    #[test]
    fn cookie_without_equals_is_none() {
        assert_eq!(parse_cookie("garbage"), None);
    }
}
