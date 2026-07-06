//! `acorns tax` — tax statements & refunds (read-only).

use crate::{GlobalOpts, TaxCmd, TaxSub, cmd, exec, net};
use serde_json::{Value, json};
use std::path::Path;

pub fn run(g: &GlobalOpts, c: &TaxCmd) -> anyhow::Result<()> {
    match &c.sub {
        // PageInput.first is required by the query; a year never has many forms.
        TaxSub::Statements { year } => cmd::read(
            g,
            "TaxCenterTaxStatementsByTaxYear",
            &json!({ "taxYear": year, "input": { "first": 100 } }),
        ),
        TaxSub::Download { year, id, out } => download(g, *year, id.as_deref(), out.as_deref()),
    }
}

fn download(g: &GlobalOpts, year: i64, id: Option<&str>, out: Option<&str>) -> anyhow::Result<()> {
    // Fetch fresh statement links (presigned URLs expire in ~5 min).
    let data = exec::run(
        &g.ctx(),
        "TaxCenterTaxStatementsByTaxYear",
        &json!({ "taxYear": year, "input": { "first": 100 } }),
    )?;
    if g.dry_run {
        return Ok(());
    }

    let edges = data
        .get("taxStatementsByTaxYear")
        .and_then(|x| x.get("edges"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let nodes: Vec<&Value> = edges.iter().filter_map(|e| e.get("node")).collect();

    let chosen: &Value = if let Some(want) = id {
        nodes
            .iter()
            .find(|n| n.get("id").and_then(Value::as_str) == Some(want))
            .copied()
            .ok_or_else(|| anyhow::anyhow!("no statement with id {want} in {year}"))?
    } else if let [only] = nodes.as_slice() {
        only
    } else if nodes.is_empty() {
        anyhow::bail!("no tax statements for {year}");
    } else {
        eprintln!("{year} has multiple statements — re-run with --id <id>:");
        for n in &nodes {
            eprintln!(
                "  {}  {}",
                n.get("id").and_then(Value::as_str).unwrap_or("?"),
                n.get("taxFormType").and_then(Value::as_str).unwrap_or("")
            );
        }
        anyhow::bail!("specify --id");
    };

    let url = chosen
        .get("statementLink")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("statement has no download link"))?;
    let form = chosen
        .get("taxFormType")
        .and_then(Value::as_str)
        .unwrap_or("statement");
    let sid = chosen.get("id").and_then(Value::as_str).unwrap_or("doc");
    let path = out.map_or_else(
        || format!("{year}-{}-{}.pdf", sanitize(form), sanitize(sid)),
        ToString::to_string,
    );
    let path = Path::new(&path);
    if path.exists() {
        anyhow::bail!(
            "{} already exists — pass --out to choose another name",
            path.display()
        );
    }

    let n = net::download_to(url, path)?;
    println!("saved {} ({n} bytes)", path.display());
    Ok(())
}

/// Filenames are built from server-provided values; keep them conservative so
/// they cannot escape the target directory (`/`, `..`) or get weird (`\n`).
fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.');
    if trimmed.is_empty() {
        "doc".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn passes_normal_values() {
        assert_eq!(sanitize("1099-DIV_B"), "1099-DIV_B");
    }

    #[test]
    fn neutralizes_path_separators_and_traversal() {
        let s = sanitize("../../etc/passwd");
        assert!(!s.contains('/'), "{s}");
        assert!(!s.starts_with('.'), "{s}");
        assert!(!sanitize("a/b\\c").contains(['/', '\\']));
        assert_eq!(sanitize(".."), "doc");
        assert_eq!(sanitize(""), "doc");
    }
}
