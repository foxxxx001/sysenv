//! Persistence backend for PATH entries and environment variables.
//!
//! - Windows: the real registry (`HKCU\Environment` for user scope,
//!   `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment` for
//!   machine scope), plus a `WM_SETTINGCHANGE` broadcast so new processes pick
//!   up the change immediately.
//! - Unix (Ubuntu etc.): a managed config dir (`~/.config/sysenv`) holding the
//!   PATH entries and variable exports in `env.sh`, sourced from the shell
//!   startup files; `--scope machine` targets `/etc/environment`.

pub mod store_common;
pub use store_common as common;
#[cfg(windows)]
pub mod store_win;
#[cfg(not(windows))]
pub mod store_unix;

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    User,
    Machine,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Machine => "machine",
        }
    }
}

/// Read the persisted PATH entries for a scope (user/machine), if any.
pub fn get_persisted_path(scope: Scope) -> Option<String> {
    #[cfg(windows)]
    {
        store_win::get_path(scope)
    }
    #[cfg(not(windows))]
    {
        store_unix::get_path(scope)
    }
}

/// Persist a full PATH value for a scope.
pub fn set_persisted_path(scope: Scope, value: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        store_win::set_path(scope, value)?;
        store_win::broadcast();
    }
    #[cfg(not(windows))]
    {
        store_unix::set_path(scope, value)?;
    }
    Ok(())
}

/// Read a persisted environment variable for a scope.
pub fn get_persisted_var(scope: Scope, name: &str) -> Option<String> {
    #[cfg(windows)]
    {
        store_win::get_var(scope, name)
    }
    #[cfg(not(windows))]
    {
        store_unix::get_var(scope, name)
    }
}

/// Persist an environment variable for a scope.
pub fn set_persisted_var(scope: Scope, name: &str, value: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        store_win::set_var(scope, name, value)?;
        store_win::broadcast();
    }
    #[cfg(not(windows))]
    {
        store_unix::set_var(scope, name, value)?;
    }
    Ok(())
}

/// Remove a persisted environment variable for a scope.
pub fn unset_persisted_var(scope: Scope, name: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        store_win::unset_var(scope, name)?;
        store_win::broadcast();
    }
    #[cfg(not(windows))]
    {
        store_unix::unset_var(scope, name)?;
    }
    Ok(())
}

/// Effective PATH entries of the current process (what the OS merged).
pub fn current_path_entries() -> Vec<String> {
    let raw = std::env::var("PATH").unwrap_or_default();
    let mut entries = common::split_path(&raw);
    common::dedup_entries(&mut entries);
    entries
}
