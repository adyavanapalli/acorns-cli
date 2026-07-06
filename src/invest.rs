//! `acorns invest` — investing reads + writes/money ops.

use crate::safety::Tier;
use crate::{cmd, GlobalOpts, InvestCmd, InvestSub, PauseState, PortfolioSub, RecurringSub, RoundupsSub};
use serde_json::json;

pub fn run(g: &GlobalOpts, c: &InvestCmd) -> anyhow::Result<()> {
    match &c.sub {
        // ---- reads ----
        InvestSub::Balance { .. } => cmd::read(g, "ProductAccountValues", json!({ "product": "INVEST" })),
        InvestSub::Account { .. } => cmd::read(g, "InvestmentAccountsByUserId", json!({})),
        InvestSub::Performance { days, .. } => cmd::read(g, "CombinedPerformance", json!({ "days": days })),
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
            let mut v = json!({ "amount": amount, "fundingSourceId": to, "type": "debit" });
            if let Some(i) = id { v["investmentAccountId"] = json!(i); }
            cmd::write(g, Tier::Money, &format!("withdraw ${amount:.2} to {to}"), "MakeWithdrawal", v)
        }

        // ---- writes ----
        InvestSub::Cancel { investment_id } => {
            let acct = cmd::invest_account_id(g)?;
            cmd::write(g, Tier::Write, &format!("cancel investment {investment_id}"), "CancelInvestment",
                json!({ "cancelInvestmentInput": { "investmentAccountId": acct, "investmentId": investment_id } }))
        }
        InvestSub::Deposits { state } => {
            let suspended = matches!(state, PauseState::Pause);
            let summary = if suspended { "pause recurring deposits" } else { "resume recurring deposits" };
            cmd::write(g, Tier::Write, summary, "SuspendDeposits", json!({ "suspended": suspended }))
        }
        InvestSub::Recurring { sub } => match sub {
            RecurringSub::Show => cmd::read(g, "recurringInvestmentSettings", json!({ "product": "INVEST" })),
            RecurringSub::Set { amount, day, frequency, id } => {
                let acct = match id { Some(i) => i.clone(), None => cmd::invest_account_id(g)? };
                cmd::write(g, Tier::Money, &format!("set recurring ${amount:.2} {} on day {day}", frequency.as_api()),
                    "UpdateRecurringInvestment",
                    json!({ "amount": amount, "day": day, "frequency": frequency.as_api(), "investment_account_id": acct }))
            }
            RecurringSub::Stop { .. } => cmd::write(g, Tier::Write, "stop recurring investment", "StopRecurringInvestment", json!({})),
        },
        InvestSub::Portfolio { sub } => match sub {
            PortfolioSub::Show => cmd::read(g, "InvestorAllPortfolios", json!({})),
            PortfolioSub::Set { id } => cmd::write(g, Tier::Write, &format!("set portfolio {id}"), "SetPortfolioMutation", json!({ "id": id })),
            PortfolioSub::Update { theme, risk, id } => {
                let acct = match id { Some(i) => i.clone(), None => cmd::invest_account_id(g)? };
                cmd::write(g, Tier::Write, "update portfolio theme/risk", "UpdateAccountPortfolio",
                    json!({ "investmentAccountId": acct, "theme": theme, "riskLevel": risk }))
            }
        },
        InvestSub::Roundups { sub } => match sub {
            RoundupsSub::Status => cmd::read(g, "WaitingRoundUpProfile", json!({})),
            RoundupsSub::Set { enabled, multiplier } => {
                let mut input = json!({});
                if let Some(e) = enabled { input["roundUpsEnabled"] = json!(e); }
                if let Some(m) = multiplier { input["multiplier"] = json!(m); }
                cmd::write(g, Tier::Write, "update round-ups", "UpdateRoundUpEnabledAndMultiplier", json!({ "input": input }))
            }
            RoundupsSub::WholeDollar { amount } => {
                cmd::write(g, Tier::Write, &format!("set Whole-Dollar Round-Ups to ${amount:.2}"),
                    "UpdateWholeDollarRoundup", json!({ "input": { "wholeDollarAmount": amount } }))
            }
            RoundupsSub::History { .. } => cmd::read(g, "RoundUpsPageCacheUpdate", json!({})),
        },
    }
}
