//! Per-profile token store. One "token family" per profile — never share a
//! profile with a logged-in browser (refresh-token rotation will invalidate both).

use serde::{Deserialize, Serialize};
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

fn config_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config")
        });
    base.join("acorns-cli")
}

pub fn path() -> PathBuf {
    config_dir().join("session.json")
}

pub fn load() -> Option<Session> {
    let data = std::fs::read_to_string(path()).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn save(s: &Session) -> anyhow::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let p = path();
    std::fs::write(&p, serde_json::to_vec_pretty(s)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn clear() -> anyhow::Result<()> {
    let p = path();
    if p.exists() {
        std::fs::remove_file(p)?;
    }
    Ok(())
}
