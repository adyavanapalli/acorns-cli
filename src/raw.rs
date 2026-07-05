//! `acorns raw` — browse (`--list`), describe (`--describe`), or execute any
//! cataloged operation. Mutations require confirmation unless `--yes`/`--dry-run`.

use crate::{catalog, exec, output, safety, GlobalOpts, RawCmd};
use serde_json::{json, Value};

pub fn run(g: &GlobalOpts, cmd: &RawCmd) -> anyhow::Result<()> {
    let cat = catalog::load();

    if cmd.list {
        let mut rows: Vec<&catalog::Op> = cat.ops.values().collect();
        rows.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
        let mut shown = 0;
        for op in rows {
            if let Some(d) = &cmd.domain {
                if !op.category.to_lowercase().contains(&d.to_lowercase()) {
                    continue;
                }
            }
            println!("{:<3} {:<44} {}", kind_tag(&op.kind), op.name, op.category);
            shown += 1;
        }
        eprintln!("\n{shown} operations");
        return Ok(());
    }

    let name = cmd
        .operation
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("provide an operation name, or use --list to browse"))?;
    let op = cat
        .ops
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("unknown operation '{name}' (try `acorns raw --list`)"))?;

    if cmd.describe {
        describe(&cat, op);
        return Ok(());
    }

    // Execute.
    let vars = build_vars(cmd)?;
    if op.kind != "query" {
        let proceed = safety::confirm(
            safety::Tier::Money,
            &format!("run mutation '{}' with {}", op.name, vars),
            g.yes,
            g.dry_run,
        )?;
        if !proceed {
            eprintln!("aborted.");
            return Ok(());
        }
    }
    let data = exec::run(&g.ctx(), name, vars)?;
    output::emit(g, &data);
    Ok(())
}

fn build_vars(cmd: &RawCmd) -> anyhow::Result<Value> {
    let mut map: Value = match &cmd.vars_json {
        Some(j) => serde_json::from_str(j)
            .map_err(|e| anyhow::anyhow!("--vars-json is not valid JSON: {e}"))?,
        None => json!({}),
    };
    let obj = map
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("--vars-json must be a JSON object"))?;
    for kv in &cmd.vars {
        let (k, v) = kv
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--var expects name=value, got '{kv}'"))?;
        // Try JSON (numbers/bools/objects/arrays); fall back to string.
        let val = serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.to_string()));
        obj.insert(k.to_string(), val);
    }
    Ok(map)
}

fn describe(cat: &catalog::Catalog, op: &catalog::Op) {
    println!("{}  [{}]", op.name, op.kind);
    if !op.category.is_empty() {
        println!("category: {}", op.category);
    }
    let roots: Vec<&String> = op.roots.iter().filter(|r| *r != "__typename").collect();
    if !roots.is_empty() {
        println!(
            "root fields: {}",
            roots.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        );
    }
    if op.vars.is_empty() {
        println!("variables: (none)");
    } else {
        println!("variables:");
        for v in &op.vars {
            let req = if v.required { " (required)" } else { "" };
            println!("  ${}: {}{}", v.name, v.ty, req);
            if let Some(shape) = cat.input_types.get(v.base_type()) {
                println!("      {} = {}", v.base_type(), shape);
            }
        }
    }
    println!("\ndocument:\n{}", op.doc);
}

fn kind_tag(kind: &str) -> &'static str {
    match kind {
        "query" => "Q",
        "mutation" => "M",
        "subscription" => "S",
        _ => "?",
    }
}
