//! `acorns transfer` — transfer & funding-source reads + writes/money ops.

use crate::safety::Tier;
use crate::{cmd, FundingSourceSub, GlobalOpts, TransferCmd, TransferRecurringSub, TransferSub};
use serde_json::json;

pub fn run(g: &GlobalOpts, c: &TransferCmd) -> anyhow::Result<()> {
    match &c.sub {
        // ---- money ----
        TransferSub::Create { from, to, amount } => {
            let nonce = uuid::Uuid::new_v4().to_string();
            cmd::write(g, Tier::Money, &format!("transfer ${amount:.2} from {from} to {to}"), "InitiateTransfer",
                json!({ "amount": amount, "fromAccountId": from, "toAccountId": to, "nonce": nonce }))
        }

        // ---- reads ----
        TransferSub::Accounts { to } => match to {
            Some(from) => cmd::read(g, "TransferableToQuery", json!({ "from": from })),
            None => cmd::read(g, "AllTransferableFromQuery", json!({})),
        },
        TransferSub::FundingSource { sub } => match sub {
            None => cmd::read(g, "FundingSourceAccountsQuery", json!({})),
            Some(FundingSourceSub::SetPrimary { id }) => {
                cmd::write(g, Tier::Write, &format!("set primary funding source {id}"), "SetPrimaryFundingSource", json!({ "id": id }))
            }
        },
        TransferSub::Estimate { created_at } => {
            let ts = created_at
                .clone()
                .ok_or_else(|| anyhow::anyhow!("provide --created-at <RFC3339 datetime>"))?;
            cmd::read(g, "EstimatedTransferDatesQuery", json!({ "transferCreatedAt": ts }))
        }
        TransferSub::Institutions { search } => {
            let mut v = json!({});
            if let Some(s) = search { v["search"] = json!(s); }
            cmd::read(g, "FinancialInstitutions", v)
        }
        TransferSub::Recurring { sub } => match sub {
            TransferRecurringSub::List => cmd::read(g, "RecurringTransfers", json!({})),
            TransferRecurringSub::Cancel { id } => {
                cmd::write(g, Tier::Write, &format!("cancel recurring transfer {id}"), "CancelRecurringTransfer", json!({ "id": id }))
            }
        },
        TransferSub::Unlink { linked_account_id } => {
            cmd::write(g, Tier::Write, &format!("unlink account {linked_account_id}"), "UnlinkLinkedAccount",
                json!({ "linkedAccountId": linked_account_id }))
        }
    }
}
