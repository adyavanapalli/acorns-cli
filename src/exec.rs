//! Execution engine shared by friendly commands and `raw`:
//! op-name + variables -> GraphQL request -> data (with one auto-refresh retry).

use crate::{catalog, net, session};
use serde_json::{Value, json};

pub struct Ctx {
    pub dry_run: bool,
    pub verbose: bool,
}

/// Run a cataloged operation by name.
pub fn run(ctx: &Ctx, op_name: &str, variables: &Value) -> anyhow::Result<Value> {
    let cat = catalog::load();
    let op = cat.ops.get(op_name).ok_or_else(|| {
        anyhow::anyhow!("unknown operation '{op_name}' (try `acorns raw --list`)")
    })?;
    run_doc(ctx, op_name, &op.doc, variables)
}

/// Remove Apollo client-only directives the server rejects (`@connection(...)`,
/// `@client`). The web app strips these before sending; we do the same.
///
/// Boundary-aware: a hypothetical `@clientFoo` is a different directive and is
/// kept. The argument scanner respects string literals, so something like
/// `@connection(key: "a)b")` is removed in full.
fn strip_client_directives(doc: &str) -> String {
    let mut out = String::with_capacity(doc.len());
    let mut rest = doc;
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        let after = &rest[at..];
        if let Some(r) = strip_directive(after, "@connection") {
            rest = r;
        } else if let Some(r) = strip_directive(after, "@client") {
            rest = r;
        } else {
            // Keep server directives (@include, @skip, @defer, …).
            out.push('@');
            rest = &after[1..];
        }
    }
    out.push_str(rest);
    out
}

/// If `s` starts with the directive `name` at a word boundary, return the rest
/// of the string with the directive (and any parenthesized args) removed.
fn strip_directive<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let r = s.strip_prefix(name)?;
    // Word boundary: `@clientX` must not match `@client`.
    if r.chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        return None;
    }
    let trimmed = r.trim_start();
    trimmed
        .strip_prefix('(')
        .map_or(Some(r), |args| Some(skip_paren_args(args)))
}

/// Skip a balanced, quote-aware `(...)` argument list; `s` starts just past the
/// opening paren. Returns the remainder after the matching close paren, or ""
/// when unbalanced (better to drop a tail than send a client-only directive).
fn skip_paren_args(s: &str) -> &str {
    let mut depth = 1usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, ch) in s.char_indices() {
        if in_str {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => in_str = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &s[i + 1..];
                }
            }
            _ => {}
        }
    }
    ""
}

/// Deep copy of `v` with values under sensitive-looking keys replaced, so
/// `--verbose` never echoes credentials (e.g. `ChangePasswordV2` variables).
fn redact(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, val)| {
                    if is_sensitive_key(k) {
                        (k.clone(), Value::String("<redacted>".into()))
                    } else {
                        (k.clone(), redact(val))
                    }
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(redact).collect()),
        other => other.clone(),
    }
}

fn is_sensitive_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    ["password", "token", "secret", "challengeanswer"]
        .iter()
        .any(|s| k.contains(s))
}

/// Run an explicit document (used for auth ops that aren't in the app catalog).
pub fn run_doc(ctx: &Ctx, op_name: &str, doc: &str, variables: &Value) -> anyhow::Result<Value> {
    let doc = strip_client_directives(doc);
    let body = json!([{ "operationName": op_name, "query": doc, "variables": variables }]);

    if ctx.dry_run {
        println!("POST {}", net::ENDPOINT);
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(Value::Null);
    }
    if ctx.verbose {
        eprintln!("(exec) {op_name} vars={}", redact(variables));
    }

    let started = std::time::Instant::now();
    let c = net::client();
    let mut sess = session::load()
        .ok_or_else(|| anyhow::anyhow!("not logged in — run `acorns auth login`"))?;

    let mut resp = net::post_authed(&c, &sess, &body)?;
    if is_not_authorized(&resp) {
        if ctx.verbose {
            eprintln!("(auth) access token rejected; refreshing…");
        }
        sess = net::refresh(&c, &sess)?;
        session::save(&sess)?;
        resp = net::post_authed(&c, &sess, &body)?;
    }
    if ctx.verbose {
        eprintln!("(exec) {op_name} took {}ms", started.elapsed().as_millis());
    }
    extract(&resp)
}

fn is_not_authorized(resp: &Value) -> bool {
    resp.get(0)
        .and_then(|x| x.get("errors"))
        .and_then(Value::as_array)
        .is_some_and(|a| {
            a.iter()
                .any(|e| e.get("code").and_then(Value::as_str) == Some("not_authorized"))
        })
}

fn extract(resp: &Value) -> anyhow::Result<Value> {
    let first = resp
        .get(0)
        .ok_or_else(|| anyhow::anyhow!("empty response from server"))?;
    if let Some(errs) = first.get("errors").and_then(Value::as_array) {
        if !errs.is_empty() {
            let msg = errs
                .iter()
                .filter_map(|e| e.get("message").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::bail!("GraphQL error: {msg}");
        }
    }
    Ok(first.get("data").cloned().unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::{redact, strip_client_directives};
    use serde_json::json;

    #[test]
    fn strips_connection_with_args() {
        assert_eq!(
            strip_client_directives(r#"roundUps @connection(key: "roundUps") { edges }"#),
            "roundUps  { edges }"
        );
    }

    #[test]
    fn strips_bare_client() {
        assert_eq!(
            strip_client_directives("field @client other"),
            "field  other"
        );
    }

    #[test]
    fn keeps_server_directives() {
        let doc = "field @include(if: $x) @skip(if: $y)";
        assert_eq!(strip_client_directives(doc), doc);
    }

    #[test]
    fn keeps_prefix_colliding_directives() {
        let doc = "field @clientX(a: 1) @connectionish";
        assert_eq!(strip_client_directives(doc), doc);
    }

    #[test]
    fn paren_inside_string_argument() {
        assert_eq!(
            strip_client_directives(r#"a @connection(key: "a)b", filter: ["x"]) b"#),
            "a  b"
        );
    }

    #[test]
    fn nested_parens_and_escapes() {
        assert_eq!(
            strip_client_directives(r#"a @connection(key: "q\"(", f: (1, (2))) b"#),
            "a  b"
        );
    }

    #[test]
    fn multibyte_text_before_directive() {
        assert_eq!(
            strip_client_directives("émoji \u{1f331} @client x"),
            "émoji \u{1f331}  x"
        );
    }

    #[test]
    fn unbalanced_args_drop_tail() {
        assert_eq!(strip_client_directives("a @connection(key: b"), "a ");
    }

    #[test]
    fn redacts_sensitive_keys_recursively() {
        let v = json!({
            "input": { "oldPassword": "hunter2", "newPassword": "hunter3" },
            "refreshToken": "tok",
            "challengeAnswerInput": { "challengeAnswer": "123456", "challengeId": "id" },
            "amount": 5.0,
        });
        let r = redact(&v);
        assert_eq!(r["input"]["oldPassword"], "<redacted>");
        assert_eq!(r["input"]["newPassword"], "<redacted>");
        assert_eq!(r["refreshToken"], "<redacted>");
        // A sensitive key redacts its whole subtree (over-redaction is safe).
        assert_eq!(r["challengeAnswerInput"], "<redacted>");
        assert_eq!(r["amount"], 5.0);
    }
}
