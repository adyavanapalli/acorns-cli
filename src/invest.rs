//! `acorns invest` — investing reads + writes/money ops.

use crate::safety::Tier;
use crate::{cmd, exec, output, GlobalOpts, InvestCmd, InvestSub, OnOff, PortfolioSub, RecurringSub, RoundupAccountsSub, RoundupsSub};
use serde_json::{json, Value};

pub fn run(g: &GlobalOpts, c: &InvestCmd) -> anyhow::Result<()> {
    match &c.sub {
        // ---- reads ----
        InvestSub::Balance => cmd::read(g, "ProductAccountValues", json!({ "product": "INVEST" })),
        InvestSub::Account => cmd::read(g, "InvestAccount", json!({})),
        InvestSub::Performance { days } => cmd::read(g, "CombinedPerformance", json!({ "days": days })),
        InvestSub::Holdings => cmd::read(g, "ProductPortfolioFundDetails", json!({ "product": "INVEST" })),
        InvestSub::History { limit, exclude_fees, .. } => {
            cmd::read(g, "recentActivityInvestQuery", json!({ "first": limit, "pending": false, "excludeFees": exclude_fees }))
        }

        // ---- money ----
        InvestSub::Deposit { amount, id } => {
            let mut v = json!({ "amount": amount });
            if let Some(i) = id { v["investmentAccountId"] = json!(i); }
            cmd::write(g, Tier::Money, &format!("deposit ${amount:.2} into Invest"), "MakeInvestment", v)
        }
        InvestSub::Withdraw { amount, to, id } => {
            let fs = match to { Some(t) => t.clone(), None => cmd::funding_source_uuid(g)? };
            let mut v = json!({ "amount": amount, "fundingSourceId": fs, "type": "debit" });
            if let Some(i) = id { v["investmentAccountId"] = json!(i); }
            cmd::write(g, Tier::Money, &format!("withdraw ${amount:.2} to {fs}"), "MakeWithdrawal", v)
        }

        // ---- writes ----
        InvestSub::Cancel { investment_id } => {
            let acct = cmd::invest_account_id(g)?;
            cmd::write(g, Tier::Write, &format!("cancel investment {investment_id}"), "CancelInvestment",
                json!({ "cancelInvestmentInput": { "investmentAccountId": acct, "investmentId": investment_id } }))
        }
        InvestSub::Recurring { sub } => match sub {
            RecurringSub::Status => cmd::read(g, "recurringInvestmentSettings", json!({ "product": "INVEST" })),
            RecurringSub::Set { amount, day, frequency, id } => {
                let acct = match id { Some(i) => i.clone(), None => cmd::invest_account_id(g)? };
                cmd::write(g, Tier::Money, &format!("set recurring ${amount:.2} {} on day {day}", frequency.as_api()),
                    "UpdateRecurringInvestment",
                    json!({ "amount": amount, "day": day, "frequency": frequency.as_api(), "investment_account_id": acct }))
            }
            RecurringSub::Stop { .. } => cmd::write(g, Tier::Write, "stop recurring investment", "StopRecurringInvestment", json!({})),
        },
        InvestSub::Portfolio { sub } => match sub {
            PortfolioSub::Status => cmd::read(g, "InvestorAllPortfolios", json!({})),
            PortfolioSub::Set { id } => cmd::write(g, Tier::Write, &format!("set portfolio {id}"), "SetPortfolioMutation", json!({ "id": id })),
            PortfolioSub::Update { theme, risk, id } => {
                let acct = match id { Some(i) => i.clone(), None => cmd::invest_account_id(g)? };
                cmd::write(g, Tier::Write, "update portfolio theme/risk", "UpdateAccountPortfolio",
                    json!({ "investmentAccountId": acct, "theme": theme, "riskLevel": risk }))
            }
        },
        InvestSub::Roundups { sub } => match sub {
            RoundupsSub::Status => cmd::read(g, "roundupProfile", json!({})),
            RoundupsSub::Automatic { state } => {
                let on = matches!(state, OnOff::On);
                cmd::write(g, Tier::Write,
                    if on { "turn Round-Ups automatic ON" } else { "turn Round-Ups automatic OFF" },
                    "UpdateRoundUpEnabledAndMultiplier", json!({ "input": { "roundUpsEnabled": on } }))
            }
            RoundupsSub::Multiplier { value } => {
                let label = if *value == 1 { "off".to_string() } else { format!("{value}x") };
                cmd::write(g, Tier::Write, &format!("set Round-Ups multiplier to {label}"),
                    "UpdateRoundUpEnabledAndMultiplier", json!({ "input": { "multiplier": value } }))
            }
            RoundupsSub::WholeDollar { amount } => {
                cmd::write(g, Tier::Write, &format!("set Whole-Dollar Round-Ups to ${amount:.2}"),
                    "UpdateWholeDollarRoundup", json!({ "input": { "wholeDollarAmount": amount } }))
            }
            RoundupsSub::Accounts { sub } => match sub {
                RoundupAccountsSub::Status => roundup_accounts_list(g),
                RoundupAccountsSub::Enable { id } => cmd::write(g, Tier::Write,
                    &format!("enable round-ups for account {id}"), "UpdateRoundUpAccount",
                    json!({ "enabled": true, "id": id })),
                RoundupAccountsSub::Disable { id } => cmd::write(g, Tier::Write,
                    &format!("disable round-ups for account {id}"), "UpdateRoundUpAccount",
                    json!({ "enabled": false, "id": id })),
            },
            RoundupsSub::History { .. } => cmd::read(g, "RoundUpsPageCacheUpdate", json!({})),
        },
    }
}

/// List round-up accounts as the app does: the currently-linked sub-accounts,
/// each joined with its round-up `enabled` flag. Drops the orphaned ghost records.
fn roundup_accounts_list(g: &GlobalOpts) -> anyhow::Result<()> {
    let data = exec::run(&g.ctx(), "LinkedAccountsForEarnOffer", json!({}))?;
    if g.dry_run {
        return Ok(());
    }
    // linkedSubaccountId -> enabled
    let mut enabled = std::collections::HashMap::new();
    if let Some(rua) = data.get("roundUpAccounts").and_then(|x| x.as_array()) {
        for a in rua {
            if let Some(id) = a.get("linkedSubaccountId").and_then(|v| v.as_str()) {
                enabled.insert(id.to_string(), a.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false));
            }
        }
    }
    let mut out = Vec::new();
    if let Some(accounts) = data.get("linkedAccounts").and_then(|x| x.as_array()) {
        for la in accounts {
            let institution = la.get("institutionName").cloned().unwrap_or(Value::Null);
            if let Some(subs) = la.get("linkedSubaccounts").and_then(|x| x.as_array()) {
                for s in subs {
                    let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
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
    output::print(&Value::Array(out));
    Ok(())
}
