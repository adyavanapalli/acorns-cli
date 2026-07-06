//! `acorns account` — read-only overview commands.

use crate::{AccountCmd, AccountSub, GlobalOpts, cmd};
use serde_json::json;

pub fn run(g: &GlobalOpts, c: &AccountCmd) -> anyhow::Result<()> {
    match &c.sub {
        AccountSub::Value => cmd::read(g, "ProductAccountValues", &json!({ "product": "INVEST" })),
        AccountSub::List => cmd::read(g, "AllInvestmentAccounts", &json!({})),
        AccountSub::Billing => cmd::read(g, "nextBillingDate", &json!({})),
    }
}
