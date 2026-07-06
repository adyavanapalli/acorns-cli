//! Shared helpers for friendly command modules.

use crate::{GlobalOpts, exec, output, safety};
use serde_json::{Value, json};

/// Placeholders used when `--dry-run` needs an id that would normally require
/// a live lookup: dry-run performs no network I/O, ever.
pub const DRY_RUN_FUNDING_SOURCE: &str = "<FUNDING_SOURCE_ID>";
pub const DRY_RUN_INVEST_ACCOUNT: &str = "<INVEST_ACCOUNT_ID>";

/// Run a read op and render the result.
pub fn read(g: &GlobalOpts, op: &str, vars: &Value) -> anyhow::Result<()> {
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
    vars: &Value,
) -> anyhow::Result<()> {
    if !safety::confirm(tier, summary, g.yes, g.dry_run)? {
        eprintln!("aborted.");
        return Ok(());
    }
    let data = exec::run(&g.ctx(), op, vars)?;
    output::emit(g, &data);
    Ok(())
}

/// Resolve the default withdrawal funding source: preferred → primary → the
/// only one. Returns its uuid. Under `--dry-run` this returns a placeholder
/// instead of performing a live read.
pub fn funding_source_uuid(g: &GlobalOpts) -> anyhow::Result<String> {
    if g.dry_run {
        return Ok(DRY_RUN_FUNDING_SOURCE.to_string());
    }
    let data = exec::run(&g.ctx(), "FundingSourceAccountsQuery", &json!({}))?;
    let accts = data
        .get("fundingSourceAccounts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    pick_funding_source(&accts)?
        .get("uuid")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("funding source is missing a uuid"))
}

/// Selection order: the preferred withdrawal account, else the primary funding
/// account, else the only account there is — otherwise ask the user to choose.
fn pick_funding_source(accts: &[Value]) -> anyhow::Result<&Value> {
    if accts.is_empty() {
        anyhow::bail!(
            "no funding source found — specify --to <uuid> (see `acorns funding status`)"
        );
    }
    accts
        .iter()
        .find(|a| a.get("preferredWithdrawalAccount").and_then(Value::as_bool) == Some(true))
        .or_else(|| {
            accts
                .iter()
                .find(|a| a.get("role").and_then(Value::as_str) == Some("primaryFunding"))
        })
        .or_else(|| {
            if accts.len() == 1 {
                accts.first()
            } else {
                None
            }
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "multiple funding sources — specify --to <uuid> (see `acorns funding status`)"
            )
        })
}

/// Resolve the primary Invest account id (for ops that require it).
/// Under `--dry-run` this returns a placeholder instead of a live read.
pub fn invest_account_id(g: &GlobalOpts) -> anyhow::Result<String> {
    if g.dry_run {
        return Ok(DRY_RUN_INVEST_ACCOUNT.to_string());
    }
    let data = exec::run(
        &g.ctx(),
        "ProductAccountValues",
        &json!({ "product": "INVEST" }),
    )?;
    data.get("investmentAccounts")
        .and_then(|a| a.get(0))
        .and_then(|x| x.get("id"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| anyhow::anyhow!("could not resolve Invest account id"))
}

#[cfg(test)]
mod tests {
    use super::pick_funding_source;
    use serde_json::{Value, json};

    fn uuid_of(v: &Value) -> &str {
        v.get("uuid").and_then(Value::as_str).unwrap()
    }

    #[test]
    fn prefers_preferred_withdrawal_account() {
        let accts = vec![
            json!({ "uuid": "a", "role": "primaryFunding" }),
            json!({ "uuid": "b", "preferredWithdrawalAccount": true }),
        ];
        assert_eq!(uuid_of(pick_funding_source(&accts).unwrap()), "b");
    }

    #[test]
    fn falls_back_to_primary_funding_role() {
        let accts = vec![
            json!({ "uuid": "a", "role": "spending" }),
            json!({ "uuid": "b", "role": "primaryFunding" }),
        ];
        assert_eq!(uuid_of(pick_funding_source(&accts).unwrap()), "b");
    }

    #[test]
    fn falls_back_to_the_only_account() {
        let accts = vec![json!({ "uuid": "only" })];
        assert_eq!(uuid_of(pick_funding_source(&accts).unwrap()), "only");
    }

    #[test]
    fn ambiguous_multiple_accounts_is_an_error() {
        let accts = vec![json!({ "uuid": "a" }), json!({ "uuid": "b" })];
        let err = pick_funding_source(&accts).unwrap_err().to_string();
        assert!(err.contains("multiple funding sources"), "{err}");
        assert!(err.contains("acorns funding status"), "{err}");
    }

    #[test]
    fn empty_list_is_an_error() {
        let err = pick_funding_source(&[]).unwrap_err().to_string();
        assert!(err.contains("no funding source"), "{err}");
    }
}
