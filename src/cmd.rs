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

/// Resolve the default withdrawal funding source: preferred → primary → the only one.
/// Returns its uuid. Runs even under --dry-run (it's a read), so the printed request is real.
pub fn funding_source_uuid(g: &GlobalOpts) -> anyhow::Result<String> {
    let read_ctx = exec::Ctx { dry_run: false, verbose: g.verbose };
    let data = exec::run(&read_ctx, "FundingSourceAccountsQuery", json!({}))?;
    let accts = data
        .get("fundingSourceAccounts")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    if accts.is_empty() {
        anyhow::bail!("no funding source found — specify --to <uuid> (see `acorns transfer funding-source`)");
    }
    let pick = accts
        .iter()
        .find(|a| a.get("preferredWithdrawalAccount").and_then(|b| b.as_bool()) == Some(true))
        .or_else(|| accts.iter().find(|a| a.get("role").and_then(|r| r.as_str()) == Some("primaryFunding")))
        .or_else(|| if accts.len() == 1 { accts.first() } else { None })
        .ok_or_else(|| anyhow::anyhow!("multiple funding sources — specify --to <uuid>"))?;
    pick.get("uuid")
        .and_then(|u| u.as_str())
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("funding source is missing a uuid"))
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
