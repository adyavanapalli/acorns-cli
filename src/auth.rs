//! `acorns auth` — login (email/password + MFA), refresh, status, logout, password.
//!
//! The login mutations live in the `oak` sign-in service (not the app catalog),
//! so their documents are embedded here.

use crate::{exec, net, session, AuthCmd, AuthSub, GlobalOpts, MfaMethod};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::BTreeMap;

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
        AuthSub::Login { email, method } => login(email.clone(), *method),
        AuthSub::Status => status(),
        AuthSub::Refresh => refresh(),
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
    v.get(0)?
        .get("data")?
        .get(field)?
        .get("__typename")?
        .as_str()
        .map(|s| s.to_string())
}

fn node<'a>(v: &'a Value, field: &str) -> Option<&'a Value> {
    v.get(0)?.get("data")?.get(field)
}

fn login(email: Option<String>, method: MfaMethod) -> anyhow::Result<()> {
    // Resolve credentials: flag > env (ACORNS_USERNAME/ACORNS_PASSWORD) > prompt.
    let email = match email.filter(|s| !s.is_empty()) {
        Some(e) => e,
        None => match std::env::var("ACORNS_USERNAME") {
            Ok(e) if !e.is_empty() => e,
            _ => prompt("Email: ")?,
        },
    };
    let password = match std::env::var("ACORNS_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => read_password_masked("Password: ")?,
    };
    let udid = uuid::Uuid::new_v4().to_string();
    let c = net::client();
    let mut jar: BTreeMap<String, String> = BTreeMap::new();

    // Step 1: validate credentials
    let vbody = json!([{
        "operationName": "ValidateCredentials", "query": VALIDATE,
        "variables": { "input": {
            "credentialsType": "USER", "udid": udid,
            "nativeLoginInput": { "email": email, "password": password }
        }}
    }]);
    let (v1, c1) = net::post_collect(&c, cookie_header(&jar), &vbody)?;
    apply(&mut jar, c1);
    check_errors(&v1)?;
    let t = typename(&v1, "validateCredentials")
        .ok_or_else(|| anyhow::anyhow!("unexpected login response: {v1}"))?;

    let vc = node(&v1, "validateCredentials").unwrap();
    match t.as_str() {
        "AuthSession" => {
            // No MFA required — tokens delivered via Set-Cookie.
            return finish(jar, &udid);
        }
        "UserSuspendedException" => anyhow::bail!("account is suspended"),
        "ValidateCredentialsSuccess" => {}
        other => anyhow::bail!("unexpected credential result: {other}"),
    }

    let mfa_token = vc.get("mfaToken").and_then(|x| x.as_str()).unwrap_or("");
    let user_id = vc.get("userId").and_then(|x| x.as_str()).unwrap_or("");
    let want = match method {
        MfaMethod::Sms => "PHONE",
        MfaMethod::Email => "EMAIL",
    };
    let auths = vc
        .get("authenticators")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    let chosen = auths
        .iter()
        .find(|a| a.get("type").and_then(|t| t.as_str()) == Some(want))
        .ok_or_else(|| {
            let avail: Vec<&str> = auths
                .iter()
                .filter_map(|a| a.get("type").and_then(|t| t.as_str()))
                .collect();
            anyhow::anyhow!("no {want} authenticator (available: {avail:?})")
        })?;
    let authenticator_id = chosen.get("id").and_then(|x| x.as_str()).unwrap_or("");

    // Step 2: initiate challenge (sends the code)
    let ibody = json!([{
        "operationName": "InitiateChallenge", "query": INITIATE,
        "variables": { "input": { "userId": user_id, "authenticatorId": authenticator_id, "mfaToken": mfa_token }}
    }]);
    let (v2, c2) = net::post_collect(&c, cookie_header(&jar), &ibody)?;
    apply(&mut jar, c2);
    check_errors(&v2)?;
    let ch = node(&v2, "initiateChallenge")
        .ok_or_else(|| anyhow::anyhow!("challenge failed: {v2}"))?;
    let challenge_id = ch.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let dest = ch
        .get("maskedPhoneNumber")
        .or_else(|| ch.get("maskedEmail"))
        .and_then(|x| x.as_str())
        .unwrap_or("your device");
    eprintln!("A verification code was sent to {dest}.");

    // Step 3: submit the code. Let the server drive retries — it returns
    // AuthChallengeAnswerIncorrectException while the challenge is still open, and some
    // other result (locked / expired / suspended) when it's done. We re-prompt on the
    // former and surface anything else. (The API exposes no attempt counter to show.)
    loop {
        let code = prompt("Enter the 6-digit code: ")?;
        let vend = json!([{
            "operationName": "VendAuthSession", "query": VEND,
            "variables": { "input": {
                "userId": user_id, "udid": &udid, "rememberMe": true,
                "challengeAnswerInput": { "challengeAnswer": code.trim(), "challengeId": &challenge_id }
            }}
        }]);
        let (v3, c3) = net::post_collect(&c, cookie_header(&jar), &vend)?;
        apply(&mut jar, c3);
        check_errors(&v3)?;
        match typename(&v3, "vendAuthSession").as_deref() {
            Some("AuthSession") => return finish(jar, &udid),
            Some("AuthChallengeAnswerIncorrectException") => {
                eprintln!("Incorrect code — try again (Ctrl-C to cancel).");
            }
            other => anyhow::bail!("MFA failed (result: {})", other.unwrap_or("unknown")),
        }
    }
}

fn finish(jar: BTreeMap<String, String>, udid: &str) -> anyhow::Result<()> {
    let get = |k: &str| jar.get(k).cloned().unwrap_or_default();
    let s = session::Session {
        access_token: get("acornsweb_access_token"),
        refresh_token: get("acornsweb_refresh_token"),
        csrf_token: get("acornsweb_csrf_token"),
        udid: udid.to_string(),
    };
    if s.access_token.is_empty() || s.refresh_token.is_empty() {
        anyhow::bail!("login did not yield tokens (no Set-Cookie)");
    }
    session::save(&s)?;
    println!("Logged in.");
    Ok(())
}

fn refresh() -> anyhow::Result<()> {
    let c = net::client();
    let s = session::load()
        .ok_or_else(|| anyhow::anyhow!("not logged in — run `acorns auth login`"))?;
    let ns = net::refresh(&c, &s)?;
    session::save(&ns)?;
    println!("Access token refreshed.");
    Ok(())
}

fn status() -> anyhow::Result<()> {
    match session::load() {
        None => {
            println!("not logged in");
        }
        Some(s) => {
            println!("logged in");
            println!("  udid: {}", s.udid);
            match jwt_exp(&s.access_token) {
                Some(exp) => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    let rem = exp - now;
                    if rem > 0 {
                        println!("  access token: valid, expires in {rem}s");
                    } else {
                        println!("  access token: expired {}s ago (auto-refresh on next call)", -rem);
                    }
                }
                None => println!("  access token: present"),
            }
            println!("  refresh token: present (~90-day TTL)");
        }
    }
    Ok(())
}

fn logout(g: &GlobalOpts) -> anyhow::Result<()> {
    // Best-effort server-side logout, then clear local session.
    if session::load().is_some() {
        let _ = exec::run(&g.ctx(), "Logout", json!({}));
    }
    session::clear()?;
    println!("Logged out.");
    Ok(())
}

fn password_change(g: &GlobalOpts) -> anyhow::Result<()> {
    let old = read_password_masked("Current password: ")?;
    let new = read_password_masked("New password: ")?;
    let confirm = read_password_masked("Confirm new password: ")?;
    if new != confirm {
        anyhow::bail!("new passwords do not match");
    }
    if !crate::safety::confirm(crate::safety::Tier::Destructive, "change your account password", g.yes, g.dry_run)? {
        eprintln!("aborted.");
        return Ok(());
    }
    let vars = json!({ "input": { "oldPassword": old, "newPassword": new } });
    let data = exec::run(&g.ctx(), "ChangePasswordV2", vars)?;
    if !g.dry_run {
        println!("{}", serde_json::to_string_pretty(&data)?);
    }
    Ok(())
}

fn check_errors(v: &Value) -> anyhow::Result<()> {
    if let Some(errs) = v.get(0).and_then(|x| x.get("errors")).and_then(|e| e.as_array()) {
        if !errs.is_empty() {
            let msg = errs
                .iter()
                .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
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
    v.get("exp").and_then(|e| e.as_i64())
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
fn read_password_masked(msg: &str) -> anyhow::Result<String> {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
        return Ok(s.trim_end_matches(['\r', '\n']).to_string());
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

fn masked_loop(err: &mut std::io::Stderr) -> anyhow::Result<String> {
    use crossterm::event::{read, Event, KeyCode, KeyEventKind, KeyModifiers};
    use std::io::Write;
    let mut pw = String::new();
    loop {
        if let Event::Key(k) = read()? {
            if k.kind == KeyEventKind::Release {
                continue;
            }
            match (k.code, k.modifiers) {
                (KeyCode::Enter, _) => break,
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => anyhow::bail!("cancelled"),
                (KeyCode::Char('d'), KeyModifiers::CONTROL) if pw.is_empty() => {
                    anyhow::bail!("cancelled")
                }
                (KeyCode::Char(c), _) => {
                    pw.push(c);
                    write!(err, "*")?;
                    err.flush().ok();
                }
                (KeyCode::Backspace, _) => {
                    if pw.pop().is_some() {
                        write!(err, "\x08 \x08")?; // erase the last '*'
                        err.flush().ok();
                    }
                }
                _ => {}
            }
        }
    }
    Ok(pw)
}
