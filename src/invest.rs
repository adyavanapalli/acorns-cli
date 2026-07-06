//! `acorns invest` — investing reads + writes/money ops.

use crate::safety::Tier;
use crate::{
    GlobalOpts, InvestCmd, InvestSub, OnOff, PortfolioSub, RecurringSub, RoundupAccountsSub,
    RoundupsSub, cmd, exec, output,
};
use serde_json::{Value, json};

pub fn run(g: &GlobalOpts, c: &InvestCmd) -> anyhow::Result<()> {
    match &c.sub {
        // ---- reads ----
        InvestSub::Balance => cmd::read(g, "ProductAccountValues", &json!({ "product": "INVEST" })),
        InvestSub::Account => cmd::read(g, "InvestAccount", &json!({})),
        InvestSub::Performance { days } => {
            cmd::read(g, "CombinedPerformance", &json!({ "days": days }))
        }
        InvestSub::Holdings => cmd::read(
            g,
            "ProductPortfolioFundDetails",
            &json!({ "product": "INVEST" }),
        ),
        InvestSub::History {
            limit,
            exclude_fees,
        } => cmd::read(
            g,
            "recentActivityInvestQuery",
            &json!({ "first": limit, "pending": false, "excludeFees": exclude_fees }),
        ),

        // ---- money ----
        InvestSub::Deposit { amount, id } => {
            let mut v = json!({ "amount": amount });
            if let Some(i) = id {
                v["investmentAccountId"] = json!(i);
            }
            cmd::write(
                g,
                Tier::Money,
                &format!("deposit ${amount:.2} into Invest"),
                "MakeInvestment",
                &v,
            )
        }
        InvestSub::Withdraw { amount, to, id } => {
            let fs = match to {
                Some(t) => t.clone(),
                None => cmd::funding_source_uuid(g)?,
            };
            let mut v = json!({ "amount": amount, "fundingSourceId": fs, "type": "debit" });
            if let Some(i) = id {
                v["investmentAccountId"] = json!(i);
            }
            cmd::write(
                g,
                Tier::Money,
                &format!("withdraw ${amount:.2} to {fs}"),
                "MakeWithdrawal",
                &v,
            )
        }

        // ---- writes ----
        InvestSub::Cancel { investment_id } => {
            let acct = cmd::invest_account_id(g)?;
            cmd::write(
                g,
                Tier::Write,
                &format!("cancel investment {investment_id}"),
                "CancelInvestment",
                &json!({ "cancelInvestmentInput": { "investmentAccountId": acct, "investmentId": investment_id } }),
            )
        }
        InvestSub::Recurring { sub } => recurring(g, sub),
        InvestSub::Portfolio { sub } => portfolio(g, sub),
        InvestSub::Roundups { sub } => roundups(g, sub),
    }
}

fn recurring(g: &GlobalOpts, sub: &RecurringSub) -> anyhow::Result<()> {
    match sub {
        RecurringSub::Status => cmd::read(
            g,
            "recurringInvestmentSettings",
            &json!({ "product": "INVEST" }),
        ),
        RecurringSub::Set {
            amount,
            day,
            frequency,
            id,
        } => {
            let acct = match id {
                Some(i) => i.clone(),
                None => cmd::invest_account_id(g)?,
            };
            cmd::write(
                g,
                Tier::Money,
                &format!(
                    "set recurring ${amount:.2} {} on day {day}",
                    frequency.as_api()
                ),
                "UpdateRecurringInvestment",
                // NB: `investment_account_id` really is snake_case in this op's schema.
                &json!({ "amount": amount, "day": day, "frequency": frequency.as_api(), "investment_account_id": acct }),
            )
        }
        RecurringSub::Stop => cmd::write(
            g,
            Tier::Write,
            "stop recurring investment",
            "StopRecurringInvestment",
            &json!({}),
        ),
    }
}

fn portfolio(g: &GlobalOpts, sub: &PortfolioSub) -> anyhow::Result<()> {
    match sub {
        PortfolioSub::Status => cmd::read(g, "InvestorAllPortfolios", &json!({})),
        PortfolioSub::Set { id } => cmd::write(
            g,
            Tier::Write,
            &format!("set portfolio {id}"),
            "SetPortfolioMutation",
            &json!({ "id": id }),
        ),
        PortfolioSub::Update { theme, risk, id } => {
            let acct = match id {
                Some(i) => i.clone(),
                None => cmd::invest_account_id(g)?,
            };
            cmd::write(
                g,
                Tier::Write,
                "update portfolio theme/risk",
                "UpdateAccountPortfolio",
                &json!({ "investmentAccountId": acct, "theme": theme, "riskLevel": risk }),
            )
        }
    }
}

fn roundups(g: &GlobalOpts, sub: &RoundupsSub) -> anyhow::Result<()> {
    match sub {
        RoundupsSub::Status => cmd::read(g, "roundupProfile", &json!({})),
        RoundupsSub::Automatic { state } => {
            let on = matches!(state, OnOff::On);
            cmd::write(
                g,
                Tier::Write,
                if on {
                    "turn Round-Ups automatic ON"
                } else {
                    "turn Round-Ups automatic OFF"
                },
                "UpdateRoundUpEnabledAndMultiplier",
                &json!({ "input": { "roundUpsEnabled": on } }),
            )
        }
        RoundupsSub::Multiplier { value } => {
            let label = if *value == 1 {
                "off".to_string()
            } else {
                format!("{value}x")
            };
            cmd::write(
                g,
                Tier::Write,
                &format!("set Round-Ups multiplier to {label}"),
                "UpdateRoundUpEnabledAndMultiplier",
                &json!({ "input": { "multiplier": value } }),
            )
        }
        RoundupsSub::WholeDollar { amount } => cmd::write(
            g,
            Tier::Write,
            &format!("set Whole-Dollar Round-Ups to ${amount:.2}"),
            "UpdateWholeDollarRoundup",
            &json!({ "input": { "wholeDollarAmount": amount } }),
        ),
        RoundupsSub::Accounts { sub } => match sub {
            RoundupAccountsSub::Status => roundup_accounts_list(g),
            RoundupAccountsSub::Enable { id } => cmd::write(
                g,
                Tier::Write,
                &format!("enable round-ups for account {id}"),
                "UpdateRoundUpAccount",
                &json!({ "enabled": true, "id": id }),
            ),
            RoundupAccountsSub::Disable { id } => cmd::write(
                g,
                Tier::Write,
                &format!("disable round-ups for account {id}"),
                "UpdateRoundUpAccount",
                &json!({ "enabled": false, "id": id }),
            ),
        },
        RoundupsSub::History => cmd::read(g, "RoundUpsPageCacheUpdate", &json!({})),
    }
}

/// List round-up accounts as the app does: the currently-linked sub-accounts,
/// each joined with its round-up `enabled` flag. Drops the orphaned ghost records.
fn roundup_accounts_list(g: &GlobalOpts) -> anyhow::Result<()> {
    let data = exec::run(&g.ctx(), "LinkedAccountsForEarnOffer", &json!({}))?;
    if g.dry_run {
        return Ok(());
    }
    output::print(&Value::Array(join_roundup_accounts(&data)));
    Ok(())
}

/// Join linked sub-accounts with their round-up `enabled` flag (defaulting to
/// false), keyed by `linkedSubaccountId`.
fn join_roundup_accounts(data: &Value) -> Vec<Value> {
    // linkedSubaccountId -> enabled
    let mut enabled = std::collections::HashMap::new();
    if let Some(rua) = data.get("roundUpAccounts").and_then(Value::as_array) {
        for a in rua {
            if let Some(id) = a.get("linkedSubaccountId").and_then(Value::as_str) {
                enabled.insert(
                    id.to_string(),
                    a.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                );
            }
        }
    }
    let mut out = Vec::new();
    if let Some(accounts) = data.get("linkedAccounts").and_then(Value::as_array) {
        for la in accounts {
            let institution = la.get("institutionName").cloned().unwrap_or(Value::Null);
            if let Some(subs) = la.get("linkedSubaccounts").and_then(Value::as_array) {
                for s in subs {
                    let id = s
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    out.push(json!({
                        "enabled": enabled.get(&id).copied().unwrap_or(false),
                        "institution": institution,
                        "name": s.get("name"),
                        "last4": s.get("accountNumberLastFour"),
                        "id": id,
                    }));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::join_roundup_accounts;
    use serde_json::json;

    #[test]
    fn joins_enabled_flags_and_drops_orphans() {
        let data = json!({
            "roundUpAccounts": [
                { "linkedSubaccountId": "sub-1", "enabled": true },
                { "linkedSubaccountId": "ghost", "enabled": true },
            ],
            "linkedAccounts": [{
                "institutionName": "Big Bank",
                "linkedSubaccounts": [
                    { "id": "sub-1", "name": "Checking", "accountNumberLastFour": "1234" },
                    { "id": "sub-2", "name": "Savings", "accountNumberLastFour": "5678" },
                ],
            }],
        });
        let out = join_roundup_accounts(&data);
        assert_eq!(out.len(), 2, "ghost record must not appear: {out:?}");
        assert_eq!(out[0]["id"], "sub-1");
        assert_eq!(out[0]["enabled"], true);
        assert_eq!(out[0]["institution"], "Big Bank");
        assert_eq!(out[1]["id"], "sub-2");
        assert_eq!(
            out[1]["enabled"], false,
            "unlisted accounts default to disabled"
        );
    }

    #[test]
    fn tolerates_missing_sections() {
        assert!(join_roundup_accounts(&json!({})).is_empty());
        assert!(join_roundup_accounts(&json!({ "linkedAccounts": [] })).is_empty());
    }
}
