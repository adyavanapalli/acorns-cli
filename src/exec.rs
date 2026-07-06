//! Execution engine shared by friendly commands and `raw`:
//! op-name + variables -> GraphQL request -> data (with one auto-refresh retry).

use crate::{catalog, net, session};
use serde_json::{json, Value};

pub struct Ctx {
    pub dry_run: bool,
    pub verbose: bool,
}

/// Run a cataloged operation by name.
pub fn run(ctx: &Ctx, op_name: &str, variables: Value) -> anyhow::Result<Value> {
    let cat = catalog::load();
    let op = cat
        .ops
        .get(op_name)
        .ok_or_else(|| anyhow::anyhow!("unknown operation '{op_name}' (try `acorns raw --list`)"))?;
    run_doc(ctx, op_name, &op.doc, variables)
}

/// Remove Apollo client-only directives the server rejects (`@connection(...)`,
/// `@client`). The web app strips these before sending; we do the same.
fn strip_client_directives(doc: &str) -> String {
    let mut out = String::with_capacity(doc.len());
    let mut rest = doc;
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        let after = &rest[at..];
        if let Some(r) = after.strip_prefix("@connection") {
            let r = r.trim_start();
            if let Some(args) = r.strip_prefix('(') {
                match args.find(')') {
                    Some(close) => rest = &args[close + 1..],
                    None => rest = "",
                }
            } else {
                rest = r;
            }
        } else if let Some(r) = after.strip_prefix("@client") {
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

/// Run an explicit document (used for auth ops that aren't in the app catalog).
pub fn run_doc(ctx: &Ctx, op_name: &str, doc: &str, variables: Value) -> anyhow::Result<Value> {
    let doc = strip_client_directives(doc);
    let body = json!([{ "operationName": op_name, "query": doc, "variables": variables }]);

    if ctx.dry_run {
        println!("POST {}", net::ENDPOINT);
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(Value::Null);
    }
    if ctx.verbose {
        eprintln!("(exec) {op_name} vars={variables}");
    }

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
    extract(&resp)
}

fn is_not_authorized(resp: &Value) -> bool {
    resp.get(0)
        .and_then(|x| x.get("errors"))
        .and_then(|e| e.as_array())
        .map(|a| {
            a.iter().any(|e| {
                e.get("code").and_then(|c| c.as_str()) == Some("not_authorized")
            })
        })
        .unwrap_or(false)
}

fn extract(resp: &Value) -> anyhow::Result<Value> {
    let first = resp
        .get(0)
        .ok_or_else(|| anyhow::anyhow!("empty response from server"))?;
    if let Some(errs) = first.get("errors").and_then(|e| e.as_array()) {
        if !errs.is_empty() {
            let msg = errs
                .iter()
                .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::bail!("GraphQL error: {msg}");
        }
    }
    Ok(first.get("data").cloned().unwrap_or(Value::Null))
}
