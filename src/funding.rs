//! `acorns funding` — funding source & linked bank(s).

use crate::safety::Tier;
use crate::{cmd, exec, output, FundingCmd, FundingSub, GlobalOpts, OnOff};
use serde_json::{json, Value};

pub fn run(g: &GlobalOpts, c: &FundingCmd) -> anyhow::Result<()> {
    match &c.sub {
        FundingSub::Status => status(g),
        FundingSub::SetPrimary { id } => cmd::write(
            g,
            Tier::Write,
            &format!("set primary funding source {id}"),
            "SetPrimaryFundingSource",
            json!({ "id": id }),
        ),
        FundingSub::Allow { state } => {
            // "Allow transfers" ON => not suspended; OFF => suspended (global setting).
            let suspended = matches!(state, OnOff::Off);
            exec::run(&g.ctx(), "SuspendDeposits", json!({ "suspended": suspended }))?;
            if g.dry_run {
                return Ok(()); // dry-run already printed the mutation request
            }
            cmd::read(g, "RoundUpsDepositsSuspended", json!({}))
        }
        FundingSub::Unlink { linked_account_id } => cmd::write(
            g,
            Tier::Destructive,
            &format!("unlink bank connection {linked_account_id} — re-linking requires the Acorns app"),
            "UnlinkLinkedAccount",
            json!({ "linkedAccountId": linked_account_id }),
        ),
    }
}

/// Funding source(s) + linked bank connection(s), showing the `linkedAccountId`
/// (for `unlink`) and each sub-account's role (the `primaryFunding` one funds transfers).
fn status(g: &GlobalOpts) -> anyhow::Result<()> {
    let data = exec::run(&g.ctx(), "LinkedAccountsIndex", json!({}))?;
    if g.dry_run {
        return Ok(());
    }
    let mut out = Vec::new();
    if let Some(accounts) = data.get("linkedAccounts").and_then(|x| x.as_array()) {
        for la in accounts {
            let subs: Vec<Value> = la
                .get("linkedSubaccounts")
                .and_then(|x| x.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|s| {
                            json!({
                                "name": s.get("name"),
                                "last4": s.get("accountNumberLastFour"),
                                "role": s.get("role"),
                                "id": s.get("id"),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.push(json!({
                "institution": la.get("institutionName"),
                "linkedAccountId": la.get("id"),
                "status": la.get("status"),
                "accounts": subs,
            }));
        }
    }
    output::print(&Value::Array(out));
    Ok(())
}
