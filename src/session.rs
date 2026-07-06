//! Per-profile token store. One "token family" per profile — never share a
//! profile with a logged-in browser (refresh-token rotation will invalidate both).

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub csrf_token: String,
    #[serde(default)]
    pub udid: String,
    /// Email used at login, kept so re-login can default to it (never a password).
    #[serde(default)]
    pub email: Option<String>,
}

/// `$XDG_CONFIG_HOME/acorns-cli`, falling back to `~/.config/acorns-cli`.
/// An empty `XDG_CONFIG_HOME` counts as unset (per the XDG base-dir spec).
fn config_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map_or_else(
            || PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config"),
            PathBuf::from,
        );
    base.join("acorns-cli")
}

pub fn path() -> PathBuf {
    config_dir().join("session.json")
}

/// Load the stored session. A missing file means "not logged in"; an unreadable
/// or corrupt file warns instead of silently pretending to be logged out.
pub fn load() -> Option<Session> {
    let p = path();
    let data = match std::fs::read_to_string(&p) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            eprintln!("warning: cannot read {}: {e}", p.display());
            return None;
        }
    };
    match serde_json::from_str(&data) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!(
                "warning: {} is corrupt ({e}) — run `acorns auth login`",
                p.display()
            );
            None
        }
    }
}

/// Save atomically (temp file + rename), owner-only from the moment of
/// creation — the tokens are never world-readable, even briefly.
pub fn save(s: &Session) -> anyhow::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let tmp = dir.join("session.json.tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(s)?)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path())?;
    Ok(())
}

pub fn clear() -> anyhow::Result<()> {
    let p = path();
    if p.exists() {
        std::fs::remove_file(p)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Session, clear, load, path, save};

    #[test]
    fn save_load_clear_roundtrip_with_owner_only_perms() {
        let dir = std::env::temp_dir().join(format!("acorns-cli-test-{}", std::process::id()));
        // SAFETY: this is the only test in the crate that mutates the
        // environment, and nothing else reads XDG_CONFIG_HOME concurrently.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };

        let s = Session {
            access_token: "a".into(),
            refresh_token: "r".into(),
            csrf_token: "c".into(),
            udid: "u".into(),
            email: Some("e@x".into()),
        };
        save(&s).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file_mode = std::fs::metadata(path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(file_mode, 0o600, "session file must be owner-only");
            let dir_mode = std::fs::metadata(dir.join("acorns-cli"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(dir_mode, 0o700, "config dir must be owner-only");
        }
        assert!(
            !dir.join("acorns-cli/session.json.tmp").exists(),
            "temp file must be renamed away"
        );

        let loaded = load().expect("session should load back");
        assert_eq!(loaded.access_token, "a");
        assert_eq!(loaded.refresh_token, "r");
        assert_eq!(loaded.email.as_deref(), Some("e@x"));

        clear().unwrap();
        assert!(load().is_none(), "cleared session must not load");
        std::fs::remove_dir_all(&dir).ok();
    }
}
