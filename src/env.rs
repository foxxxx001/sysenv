//! `sys env` — read/set/list environment variables.

use crate::store::common;
use crate::store::{Scope, get_persisted_var, set_persisted_var, unset_persisted_var};
use anyhow::{Result, bail};

pub fn cmd_get(name: &str, scope: Scope) -> Result<()> {
    if let Ok(v) = std::env::var(&name) {
        println!("{v}");
        return Ok(());
    }
    if let Some(v) = get_persisted_var(scope, &name) {
        println!("{v}");
        return Ok(());
    }
    bail!("environment variable `{name}` is not set (scope {})", scope.as_str())
}

pub fn cmd_set(name: &str, value: &str, scope: Scope, temporary: bool) -> Result<()> {
    if name.is_empty() {
        bail!("variable name must not be empty");
    }
    if name.contains('=') || name.chars().any(|c| c.is_whitespace()) {
        bail!("invalid variable name `{name}`");
    }
    if temporary {
        println!(
            "[temporary] Not persisted. Run the following in your current shell:"
        );
        println!("{}", common::shell_export_snippet(name, value));
        return Ok(());
    }
    set_persisted_var(scope, &name, value)?;
    unsafe { std::env::set_var(&name, value) };
    println!(
        "Set {name}={value} (scope {}). New processes will inherit it.",
        scope.as_str()
    );
    println!(
        "To apply to the current shell, run: {}",
        common::shell_export_snippet(name, value)
    );
    Ok(())
}

pub fn cmd_unset(name: &str, scope: Scope, temporary: bool) -> Result<()> {
    if temporary {
        println!(
            "[temporary] Not persisted. Run the following in your current shell:"
        );
        println!("{}", common::shell_unset_snippet(&name));
        return Ok(());
    }
    unset_persisted_var(scope, &name)?;
    unsafe { std::env::remove_var(&name) };
    println!("Unset {name} (scope {}).", scope.as_str());
    Ok(())
}

pub fn cmd_list() -> Result<()> {
    let mut vars: Vec<(String, String)> = std::env::vars().collect();
    vars.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in vars {
        println!("{k}={v}");
    }
    Ok(())
}
