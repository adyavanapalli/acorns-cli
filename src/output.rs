//! Output: pretty-printed JSON to stdout — composable, pipe straight into `jq`.

use serde_json::Value;

/// Print a value as pretty JSON.
pub fn print(v: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
    );
}

/// Print unless in dry-run mode (where the request was already shown).
pub fn emit(g: &crate::GlobalOpts, v: &Value) {
    if !g.dry_run {
        print(v);
    }
}
