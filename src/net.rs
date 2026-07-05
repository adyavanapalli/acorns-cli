//! HTTP transport to the Acorns GraphQL endpoint + token refresh.

use crate::session::Session;
use reqwest::blocking::{Client, RequestBuilder};
use serde_json::{json, Value};
use std::time::Duration;

pub const ENDPOINT: &str = "https://graphql.acorns.com/graphql";
const REFRESH_HASH: &str = "ebdecb9e33a90bcce01717ae5e77f7409dcf727cb9c0230bb725f7e9cd4ef9a2";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36";

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
pub fn post_authed(c: &Client, s: &Session, body: &Value) -> anyhow::Result<Value> {
    let cookie = format!(
        "acornsweb_access_token={}; acornsweb_csrf_token={}",
        s.access_token, s.csrf_token
    );
    let resp = base(c)
        .header("cookie", cookie)
        .header("x-csrf-token", &s.csrf_token)
        .body(serde_json::to_vec(body)?)
        .send()?;
    Ok(resp.json()?)
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

    let j: Value = resp.json()?;
    if let Some(t) = j
        .get(0)
        .and_then(|x| x.get("data"))
        .and_then(|d| d.get("refreshAuthToken"))
        .and_then(|r| r.get("__typename"))
        .and_then(|t| t.as_str())
    {
        if t.contains("Expired") || t.contains("Invalid") {
            anyhow::bail!("session expired/invalid — run `acorns auth login`");
        }
    }
    if !got_access {
        anyhow::bail!("token refresh failed — run `acorns auth login`");
    }
    Ok(ns)
}

/// POST with an optional raw Cookie header, returning the JSON body plus any
/// `Set-Cookie` pairs. Used by the multi-step login flow (which threads cookies
/// across ValidateCredentials -> InitiateChallenge -> VendAuthSession).
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
    let j = resp.json()?;
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
