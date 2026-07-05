//! Confirmation gating for write/money/destructive operations.

use std::io::Write;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Tier {
    Write,
    Money,
    Destructive,
}

/// Returns Ok(true) to proceed, Ok(false) if the user declined.
/// Read/Write proceed silently. Money/Destructive require confirmation
/// unless `--yes` is set. `--dry-run` always proceeds (the request is only printed).
pub fn confirm(tier: Tier, summary: &str, yes: bool, dry_run: bool) -> anyhow::Result<bool> {
    if dry_run {
        return Ok(true);
    }
    match tier {
        Tier::Write => Ok(true),
        Tier::Money | Tier::Destructive => {
            if yes {
                return Ok(true);
            }
            let label = if tier == Tier::Destructive {
                "DESTRUCTIVE"
            } else {
                "MONEY MOVEMENT"
            };
            eprint!("[{label}] {summary}\nType 'yes' to confirm: ");
            std::io::stderr().flush().ok();
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            Ok(line.trim().eq_ignore_ascii_case("yes"))
        }
    }
}
