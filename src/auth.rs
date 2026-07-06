//! `acorns auth` — login (email/password + MFA), refresh, status, logout, password.
//!
//! The login mutations live in the `oak` sign-in service (not the app catalog),
//! so their documents are embedded here.

use crate::{AuthCmd, AuthSub, GlobalOpts, MfaMethod, exec, net, session};
use base64::Engine;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

const VALIDATE: &str = "mutation ValidateCredentials($input: ValidateCredentialsInput!) { \
validateCredentials(input: $input) { __typename \
... on AuthSession { id identityId refreshToken token } \
... on ValidateCredentialsSuccess { mfaToken userId authenticators { id type } defaultAuthenticator { id type } } \
... on UserSuspendedException { suspendedToken } } }";

const INITIATE: &str = "mutation InitiateChallenge($input: InitiateChallengeInput!) { \
initiateChallenge(input: $input) { __typename \
... on SMSAuthChallenge { id maskedPhoneNumber alternateAuthenticators { id type } } \
... on EmailAuthChallenge { id maskedEmail alternateAuthenticators { id type } } } }";

const VEND: &str = "mutation VendAuthSession($input: VendAuthSessionInput!) { \
vendAuthSession(input: $input) { __typename \
... on AuthSession { id identityId refreshToken token } \
... on UserSuspendedException { suspendedToken } } }";

pub fn run(g: &GlobalOpts, cmd: &AuthCmd) -> anyhow::Result<()> {
    match &cmd.sub {
        AuthSub::Login { email, method } => login(g, email.clone(), *method),
        AuthSub::Status => {
            status();
            Ok(())
        }
        AuthSub::Refresh => refresh(g),
        AuthSub::Logout => logout(g),
        AuthSub::Password => password_change(g),
    }
}

fn cookie_header(jar: &BTreeMap<String, String>) -> Option<String> {
    if jar.is_empty() {
        return None;
    }
    Some(
        jar.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

fn apply(jar: &mut BTreeMap<String, String>, cookies: Vec<(String, String)>) {
    for (k, v) in cookies {
        if k.starts_with("acornsweb_") {
            jar.insert(k, v);
        }
    }
}

fn typename(v: &Value, field: &str) -> Option<String> {
    node(v, field)?
        .get("__typename")?
        .as_str()
        .map(ToString::to_string)
}

fn node<'a>(v: &'a Value, field: &str) -> Option<&'a Value> {
    v.get(0)?.get("data")?.get(field)
}

/// `--dry-run auth login`: show the first request of the flow without
/// prompting for credentials or touching the network. The MFA steps depend on
/// live server responses, so only their operation names are noted.
fn login_dry_run(email: Option<&str>) -> anyhow::Result<()> {
    let stored = session::load();
    let email = email
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            stored
                .as_ref()
                .and_then(|s| s.email.clone())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "<EMAIL>".to_string());
    let udid = stored
        .map(|s| s.udid)
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "<UDID>".to_string());
    let body = json!([{
        "operationName": "ValidateCredentials", "query": VALIDATE,
        "variables": { "input": {
            "credentialsType": "USER", "udid": udid,
            "nativeLoginInput": { "email": email, "password": "<PASSWORD>" }
        }}
    }]);
    println!("POST {}", net::ENDPOINT);
    println!("{}", serde_json::to_string_pretty(&body)?);
    eprintln!(
        "(dry-run) followed by InitiateChallenge and VendAuthSession, driven by live responses"
    );
    Ok(())
}

fn login(g: &GlobalOpts, email: Option<String>, method: MfaMethod) -> anyhow::Result<()> {
    // Logging in vends tokens and sends MFA codes — never do that on --dry-run.
    if g.dry_run {
        return login_dry_run(email.as_deref());
    }
    // Resolve email: --email flag > stored session > interactive prompt. The
    // password is always prompted (never stored, never read from the environment).
    let stored = session::load();
    let email = match email.filter(|s| !s.is_empty()) {
        Some(e) => e,
        None => match stored
            .as_ref()
            .and_then(|s| s.email.clone())
            .filter(|s| !s.is_empty())
        {
            Some(e) => e,
            None => prompt("Email: ")?,
        },
    };
    let password = read_password_masked("Password: ")?;
    // Reuse the stored device id so repeat logins keep one identity; else mint one.
    let udid = stored
        .as_ref()
        .map(|s| s.udid.clone())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let c = net::client();
    let mut jar: BTreeMap<String, String> = BTreeMap::new();

    // Step 1: validate credentials. (The request body necessarily carries the
    // password; `Zeroizing` covers our copy, not serde's transient one.)
    let vbody = json!([{
        "operationName": "ValidateCredentials", "query": VALIDATE,
        "variables": { "input": {
            "credentialsType": "USER", "udid": udid,
            "nativeLoginInput": { "email": email, "password": &*password }
        }}
    }]);
    let (v1, c1) = net::post_collect(&c, cookie_header(&jar), &vbody)?;
    apply(&mut jar, c1);
    check_errors(&v1)?;
    let vc = node(&v1, "validateCredentials")
        .ok_or_else(|| anyhow::anyhow!("unexpected login response: {v1}"))?;
    let t = vc
        .get("__typename")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("unexpected login response: {v1}"))?;

    match t {
        "AuthSession" => {
            // No MFA required — tokens delivered via Set-Cookie.
            return finish(&jar, &udid, &email);
        }
        "InvalidCredentialsException" => anyhow::bail!("invalid email or password"),
        "UserSuspendedException" => anyhow::bail!("account is suspended"),
        "ValidateCredentialsSuccess" => {}
        other => anyhow::bail!("unexpected credential result: {other}"),
    }

    let mfa_token = vc.get("mfaToken").and_then(Value::as_str).unwrap_or("");
    let user_id = vc.get("userId").and_then(Value::as_str).unwrap_or("");
    let authenticator_id = choose_authenticator(vc, method)?;

    // Step 2: initiate challenge (sends the code)
    let ibody = json!([{
        "operationName": "InitiateChallenge", "query": INITIATE,
        "variables": { "input": { "userId": user_id, "authenticatorId": authenticator_id, "mfaToken": mfa_token }}
    }]);
    let (v2, c2) = net::post_collect(&c, cookie_header(&jar), &ibody)?;
    apply(&mut jar, c2);
    check_errors(&v2)?;
    let ch =
        node(&v2, "initiateChallenge").ok_or_else(|| anyhow::anyhow!("challenge failed: {v2}"))?;
    let challenge_id = ch
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let dest = ch
        .get("maskedPhoneNumber")
        .or_else(|| ch.get("maskedEmail"))
        .and_then(Value::as_str)
        .unwrap_or("your device");
    eprintln!("A verification code was sent to {dest}.");

    submit_mfa_code(&c, &mut jar, user_id, &udid, &challenge_id, &email)
}

/// Pick the authenticator matching the requested MFA method.
fn choose_authenticator(vc: &Value, method: MfaMethod) -> anyhow::Result<String> {
    let want = match method {
        MfaMethod::Sms => "PHONE",
        MfaMethod::Email => "EMAIL",
    };
    let auths = vc
        .get("authenticators")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let chosen = auths
        .iter()
        .find(|a| a.get("type").and_then(Value::as_str) == Some(want))
        .ok_or_else(|| {
            let avail: Vec<&str> = auths
                .iter()
                .filter_map(|a| a.get("type").and_then(Value::as_str))
                .collect();
            anyhow::anyhow!("no {want} authenticator (available: {avail:?})")
        })?;
    Ok(chosen
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string())
}

/// Step 3: submit the code. Let the server drive retries — it returns
/// `AuthChallengeAnswerIncorrectException` while the challenge is still open, and
/// some other result (locked / expired / suspended) when it's done. We re-prompt on
/// the former and surface anything else. (The API exposes no attempt counter to show.)
fn submit_mfa_code(
    c: &reqwest::blocking::Client,
    jar: &mut BTreeMap<String, String>,
    user_id: &str,
    udid: &str,
    challenge_id: &str,
    email: &str,
) -> anyhow::Result<()> {
    loop {
        let code = prompt("Enter the 6-digit code: ")?;
        let vend = json!([{
            "operationName": "VendAuthSession", "query": VEND,
            "variables": { "input": {
                "userId": user_id, "udid": udid, "rememberMe": true,
                "challengeAnswerInput": { "challengeAnswer": code.trim(), "challengeId": challenge_id }
            }}
        }]);
        let (v3, c3) = net::post_collect(c, cookie_header(jar), &vend)?;
        apply(jar, c3);
        check_errors(&v3)?;
        match typename(&v3, "vendAuthSession").as_deref() {
            Some("AuthSession") => return finish(jar, udid, email),
            Some("AuthChallengeAnswerIncorrectException") => {
                eprintln!("Incorrect code — try again (Ctrl-C to cancel).");
            }
            other => anyhow::bail!("MFA failed (result: {})", other.unwrap_or("unknown")),
        }
    }
}

fn finish(jar: &BTreeMap<String, String>, udid: &str, email: &str) -> anyhow::Result<()> {
    let get = |k: &str| jar.get(k).cloned().unwrap_or_default();
    let s = session::Session {
        access_token: get("acornsweb_access_token"),
        refresh_token: get("acornsweb_refresh_token"),
        csrf_token: get("acornsweb_csrf_token"),
        udid: udid.to_string(),
        email: Some(email.to_string()),
    };
    if s.access_token.is_empty() || s.refresh_token.is_empty() {
        anyhow::bail!("login did not yield tokens (no Set-Cookie)");
    }
    session::save(&s)?;
    println!("Logged in.");
    Ok(())
}

fn refresh(g: &GlobalOpts) -> anyhow::Result<()> {
    // A real refresh ROTATES the token family (state-changing), so --dry-run
    // only prints the request it would send.
    if g.dry_run {
        let body = json!([{
            "operationName": "RefreshAuthTokens",
            "variables": { "refreshToken": "" },
            "extensions": { "persistedQuery": { "version": 1, "sha256Hash": net::REFRESH_HASH } }
        }]);
        println!("POST {}", net::ENDPOINT);
        println!("{}", serde_json::to_string_pretty(&body)?);
        eprintln!("(dry-run) sent with cookie: acornsweb_refresh_token=<redacted>");
        return Ok(());
    }
    let c = net::client();
    let s = session::load()
        .ok_or_else(|| anyhow::anyhow!("not logged in — run `acorns auth login`"))?;
    let ns = net::refresh(&c, &s)?;
    session::save(&ns)?;
    println!("Access token refreshed.");
    Ok(())
}

fn status() {
    match session::load() {
        None => println!("not logged in"),
        Some(s) => {
            println!("logged in");
            if let Some(email) = s.email.as_deref().filter(|e| !e.is_empty()) {
                println!("  email: {email}");
            }
            println!("  udid: {}", s.udid);
            match jwt_exp(&s.access_token) {
                Some(exp) => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
                    let rem = exp - now;
                    if rem > 0 {
                        println!("  access token: valid, expires in {rem}s");
                    } else {
                        println!(
                            "  access token: expired {}s ago (auto-refresh on next call)",
                            -rem
                        );
                    }
                }
                None => println!("  access token: present"),
            }
            println!("  refresh token: present (~90-day TTL)");
        }
    }
}

fn logout(g: &GlobalOpts) -> anyhow::Result<()> {
    // Print the request only; a dry run must NOT clear the local session.
    if g.dry_run {
        exec::run(&g.ctx(), "Logout", &json!({}))?;
        return Ok(());
    }
    // Best-effort server-side logout, then clear local session.
    if session::load().is_some() {
        let _ = exec::run(&g.ctx(), "Logout", &json!({}));
    }
    session::clear()?;
    println!("Logged out.");
    Ok(())
}

fn password_change(g: &GlobalOpts) -> anyhow::Result<()> {
    // Placeholders instead of prompts on --dry-run: the printed request must
    // never contain real credentials.
    if g.dry_run {
        let vars = json!({ "input": { "oldPassword": "<OLD_PASSWORD>", "newPassword": "<NEW_PASSWORD>" } });
        exec::run(&g.ctx(), "ChangePasswordV2", &vars)?;
        return Ok(());
    }
    let old = read_password_masked("Current password: ")?;
    let new = read_password_masked("New password: ")?;
    let confirm = read_password_masked("Confirm new password: ")?;
    if *new != *confirm {
        anyhow::bail!("new passwords do not match");
    }
    if !crate::safety::confirm(
        crate::safety::Tier::Destructive,
        "change your account password",
        g.yes,
        g.dry_run,
    )? {
        eprintln!("aborted.");
        return Ok(());
    }
    let vars = json!({ "input": { "oldPassword": &*old, "newPassword": &*new } });
    let data = exec::run(&g.ctx(), "ChangePasswordV2", &vars)?;
    println!("{}", serde_json::to_string_pretty(&data)?);
    Ok(())
}

fn check_errors(v: &Value) -> anyhow::Result<()> {
    if let Some(errs) = v
        .get(0)
        .and_then(|x| x.get("errors"))
        .and_then(Value::as_array)
    {
        if !errs.is_empty() {
            let msg = errs
                .iter()
                .filter_map(|e| e.get("message").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::bail!("{msg}");
        }
    }
    Ok(())
}

fn jwt_exp(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    v.get("exp").and_then(Value::as_i64)
}

fn prompt(msg: &str) -> anyhow::Result<String> {
    use std::io::Write;
    eprint!("{msg}");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

/// Read a password, echoing `*` per keystroke (with backspace). Falls back to a
/// plain line read when stdin isn't a TTY (e.g. piped input for automation).
/// The buffer is zeroed on drop; copies serialized into request bodies are not.
fn read_password_masked(msg: &str) -> anyhow::Result<Zeroizing<String>> {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
        while s.ends_with(['\r', '\n']) {
            s.pop();
        }
        return Ok(Zeroizing::new(s));
    }
    let mut err = std::io::stderr();
    write!(err, "{msg}")?;
    err.flush().ok();
    crossterm::terminal::enable_raw_mode()?;
    let outcome = masked_loop(&mut err);
    let _ = crossterm::terminal::disable_raw_mode();
    writeln!(err).ok();
    outcome
}

fn masked_loop(err: &mut std::io::Stderr) -> anyhow::Result<Zeroizing<String>> {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, read};
    use std::io::Write;
    const CHORD: KeyModifiers = KeyModifiers::CONTROL.union(KeyModifiers::ALT);
    let mut pw = Zeroizing::new(String::new());
    loop {
        if let Event::Key(k) = read()? {
            if k.kind == KeyEventKind::Release {
                continue;
            }
            match (k.code, k.modifiers) {
                (KeyCode::Enter, _) => break,
                (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
                    anyhow::bail!("cancelled")
                }
                (KeyCode::Char('d'), m) if m.contains(KeyModifiers::CONTROL) && pw.is_empty() => {
                    anyhow::bail!("cancelled")
                }
                // Ignore other Ctrl-/Alt- chords: they are commands, not
                // characters — silently inserting `a` for Ctrl-A corrupts the
                // password with no visual difference.
                (KeyCode::Char(c), m) if !m.intersects(CHORD) => {
                    pw.push(c);
                    write!(err, "*")?;
                    err.flush().ok();
                }
                (KeyCode::Backspace, _) => {
                    if pw.pop().is_none() {
                        continue;
                    }
                    write!(err, "\x08 \x08")?; // erase the last '*'
                    err.flush().ok();
                }
                _ => {}
            }
        }
    }
    Ok(pw)
}

#[cfg(test)]
mod tests {
    use super::jwt_exp;
    use base64::Engine;

    fn token_with_payload(payload: &str) -> String {
        let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        format!(
            "{}.{}.{}",
            b64(r#"{"alg":"HS256"}"#),
            b64(payload),
            b64("sig")
        )
    }

    #[test]
    fn extracts_exp_claim() {
        assert_eq!(
            jwt_exp(&token_with_payload(r#"{"exp":1234567890}"#)),
            Some(1_234_567_890)
        );
    }

    #[test]
    fn missing_exp_is_none() {
        assert_eq!(jwt_exp(&token_with_payload(r#"{"sub":"x"}"#)), None);
    }

    #[test]
    fn garbage_tokens_are_none() {
        assert_eq!(jwt_exp(""), None);
        assert_eq!(jwt_exp("not-a-jwt"), None);
        assert_eq!(jwt_exp("a.!!!not-base64!!!.c"), None);
    }
}
