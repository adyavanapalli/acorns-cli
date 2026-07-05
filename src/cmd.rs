//! Shared helpers for friendly command modules.

use crate::{exec, output, safety, GlobalOpts};
use serde_json::{json, Value};

/// Run a read op and render the result.
pub fn read(g: &GlobalOpts, op: &str, vars: Value) -> anyhow::Result<()> {
    let data = exec::run(&g.ctx(), op, vars)?;
    output::emit(g, &data);
    Ok(())
}

/// Confirm (per tier) then run a write/money/destructive op and render the result.
pub fn write(
    g: &GlobalOpts,
    tier: safety::Tier,
    summary: &str,
    op: &str,
    vars: Value,
) -> anyhow::Result<()> {
    if !safety::confirm(tier, summary, g.yes, g.dry_run)? {
        eprintln!("aborted.");
        return Ok(());
    }
    let data = exec::run(&g.ctx(), op, vars)?;
    output::emit(g, &data);
    Ok(())
}

/// Resolve the primary Invest account id (for ops that require it).
/// In dry-run mode, returns a placeholder so the request shape is still printable.
pub fn invest_account_id(g: &GlobalOpts) -> anyhow::Result<String> {
    if g.dry_run {
        return Ok("<INVEST_ACCOUNT_ID>".to_string());
    }
    let data = exec::run(&g.ctx(), "ProductAccountValues", json!({ "product": "INVEST" }))?;
    data.get("investmentAccounts")
        .and_then(|a| a.get(0))
        .and_then(|x| x.get("id"))
        .and_then(|i| i.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("could not resolve Invest account id"))
}
