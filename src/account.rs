//! `acorns account` — read-only overview commands.

use crate::{exec, output, AccountCmd, AccountSub, GlobalOpts};
use serde_json::json;

pub fn run(g: &GlobalOpts, cmd: &AccountCmd) -> anyhow::Result<()> {
    let ctx = g.ctx();
    let data = match &cmd.sub {
        AccountSub::Value => exec::run(&ctx, "ProductAccountValues", json!({ "product": "INVEST" }))?,
        AccountSub::List => exec::run(&ctx, "AllInvestmentAccounts", json!({}))?,
        AccountSub::Billing => exec::run(&ctx, "nextBillingDate", json!({}))?,
    };
    output::emit(g, &data);
    Ok(())
}
