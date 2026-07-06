//! `acorns raw` — browse (`--list`), describe (`--describe`), or execute any
//! cataloged operation. Mutations require confirmation unless `--yes`/`--dry-run`.

use crate::{GlobalOpts, RawCmd, catalog, exec, output, safety};
use serde_json::{Value, json};

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
        describe(cat, op);
        return Ok(());
    }

    // Execute.
    if op.kind == "subscription" {
        anyhow::bail!(
            "'{name}' is a subscription and can't be executed over single-shot HTTP \
             (use --describe to inspect it)"
        );
    }
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
    let data = exec::run(&g.ctx(), name, &vars)?;
    output::emit(g, &data);
    Ok(())
}

/// Merge variable sources; later sources win on key conflicts:
/// `--vars-json` < `--var` (JSON-parsed when possible) < `--var-str` (verbatim).
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
        let (k, v) = split_kv(kv, "--var")?;
        // Try JSON (numbers/bools/objects/arrays); fall back to string.
        let val = serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.to_string()));
        obj.insert(k.to_string(), val);
    }
    for kv in &cmd.vars_str {
        let (k, v) = split_kv(kv, "--var-str")?;
        obj.insert(k.to_string(), Value::String(v.to_string()));
    }
    Ok(map)
}

fn split_kv<'a>(kv: &'a str, flag: &str) -> anyhow::Result<(&'a str, &'a str)> {
    kv.split_once('=')
        .ok_or_else(|| anyhow::anyhow!("{flag} expects name=value, got '{kv}'"))
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
            roots
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
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

#[cfg(test)]
mod tests {
    use super::build_vars;
    use crate::RawCmd;
    use serde_json::json;

    fn cmd() -> RawCmd {
        RawCmd {
            operation: None,
            vars: vec![],
            vars_str: vec![],
            vars_json: None,
            list: false,
            domain: None,
            describe: false,
        }
    }

    #[test]
    fn var_values_parse_as_json_when_possible() {
        let mut c = cmd();
        c.vars = vec![
            "n=5".into(),
            "b=true".into(),
            "s=hello".into(),
            "o={\"a\":1}".into(),
            "leading_zero=05".into(),
        ];
        let v = build_vars(&c).unwrap();
        assert_eq!(v["n"], json!(5));
        assert_eq!(v["b"], json!(true));
        assert_eq!(v["s"], json!("hello"));
        assert_eq!(v["o"], json!({"a": 1}));
        // Not valid JSON -> falls back to a string.
        assert_eq!(v["leading_zero"], json!("05"));
    }

    #[test]
    fn var_str_is_always_a_string_and_wins() {
        let mut c = cmd();
        c.vars = vec!["note=true".into()];
        c.vars_str = vec!["note=true".into(), "id=00123".into()];
        let v = build_vars(&c).unwrap();
        assert_eq!(v["note"], json!("true"));
        assert_eq!(v["id"], json!("00123"));
    }

    #[test]
    fn vars_json_is_the_base_layer() {
        let mut c = cmd();
        c.vars_json = Some(r#"{"a":1,"b":2}"#.into());
        c.vars = vec!["b=3".into()];
        let v = build_vars(&c).unwrap();
        assert_eq!(v["a"], json!(1));
        assert_eq!(v["b"], json!(3));
    }

    #[test]
    fn rejects_malformed_inputs() {
        let mut c = cmd();
        c.vars = vec!["novalue".into()];
        assert!(build_vars(&c).is_err());

        let mut c = cmd();
        c.vars_json = Some("[1,2]".into());
        assert!(build_vars(&c).is_err(), "--vars-json must be an object");

        let mut c = cmd();
        c.vars_json = Some("{not json".into());
        assert!(build_vars(&c).is_err());
    }
}
