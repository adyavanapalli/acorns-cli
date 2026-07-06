//! `acorns tax` — tax statements & refunds (read-only).

use crate::{cmd, exec, net, GlobalOpts, TaxCmd, TaxSub};
use serde_json::{json, Value};
use std::path::Path;

pub fn run(g: &GlobalOpts, c: &TaxCmd) -> anyhow::Result<()> {
    match &c.sub {
        // PageInput.first is required by the query; a year never has many forms.
        TaxSub::Statements { year } => cmd::read(
            g,
            "TaxCenterTaxStatementsByTaxYear",
            json!({ "taxYear": year, "input": { "first": 100 } }),
        ),
        TaxSub::Download { year, id, out } => download(g, *year, id.clone(), out.clone()),
    }
}

fn download(g: &GlobalOpts, year: i64, id: Option<String>, out: Option<String>) -> anyhow::Result<()> {
    // Fetch fresh statement links (presigned URLs expire in ~5 min).
    let data = exec::run(
        &g.ctx(),
        "TaxCenterTaxStatementsByTaxYear",
        json!({ "taxYear": year, "input": { "first": 100 } }),
    )?;
    if g.dry_run {
        return Ok(());
    }

    let edges = data
        .get("taxStatementsByTaxYear")
        .and_then(|x| x.get("edges"))
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();
    let nodes: Vec<&Value> = edges.iter().filter_map(|e| e.get("node")).collect();
    if nodes.is_empty() {
        anyhow::bail!("no tax statements for {year}");
    }

    let chosen: &Value = if let Some(want) = &id {
        nodes
            .iter()
            .find(|n| n.get("id").and_then(|i| i.as_str()) == Some(want.as_str()))
            .copied()
            .ok_or_else(|| anyhow::anyhow!("no statement with id {want} in {year}"))?
    } else if nodes.len() == 1 {
        nodes[0]
    } else {
        eprintln!("{year} has multiple statements — re-run with --id <id>:");
        for n in &nodes {
            eprintln!(
                "  {}  {}",
                n.get("id").and_then(|i| i.as_str()).unwrap_or("?"),
                n.get("taxFormType").and_then(|t| t.as_str()).unwrap_or("")
            );
        }
        anyhow::bail!("specify --id");
    };

    let url = chosen
        .get("statementLink")
        .and_then(|u| u.as_str())
        .ok_or_else(|| anyhow::anyhow!("statement has no download link"))?;
    let form = chosen.get("taxFormType").and_then(|t| t.as_str()).unwrap_or("statement");
    let sid = chosen.get("id").and_then(|i| i.as_str()).unwrap_or("doc");
    let path = out.unwrap_or_else(|| format!("{year}-{form}-{sid}.pdf"));

    let n = net::download_to(url, Path::new(&path))?;
    println!("saved {path} ({n} bytes)");
    Ok(())
}
