//! Unix (Ubuntu / Linux) backend.
//!
//! User scope keeps managed state in a config directory:
//!   - `<cfg>/path`   : managed PATH entries, one per line
//!   - `<cfg>/vars`   : other exported variables, `NAME=VALUE` per line
//!   - `<cfg>/env.sh` : generated shell script sourcing both (exported to shells)
//!
//! The env.sh file is sourced from `~/.profile`, `~/.bashrc` and `~/.zshrc`
//! (whichever exist; `~/.profile` is created if missing).
//!
//! Machine scope edits `/etc/environment` (requires root).

use crate::store::common;
use crate::store::Scope;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

fn config_dir() -> Result<PathBuf> {
    if let Ok(d) = std::env::var("SYSSENV_CONFIG_DIR") {
        return Ok(PathBuf::from(d));
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("sys"));
        }
    }
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".config").join("sys"))
}

fn path_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("path"))
}

fn vars_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("vars"))
}

fn env_sh() -> Result<PathBuf> {
    Ok(config_dir()?.join("env.sh"))
}

fn read_file(path: &PathBuf) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn ensure_dir() -> Result<()> {
    std::fs::create_dir_all(config_dir()?)
        .with_context(|| format!("cannot create config dir {}", config_dir().unwrap().display()))
}

fn read_path_entries() -> Result<Vec<String>> {
    Ok(common::parse_txt_content(&read_file(&path_file()?)))
}

fn regenerate_env_sh(entries: &[String]) -> Result<()> {
    let vars = common::parse_vars_file(&read_file(&vars_file()?));
    let content = common::generate_env_sh(entries, &vars);
    std::fs::write(env_sh()?, content)
        .with_context(|| format!("cannot write {}", env_sh().unwrap().display()))
}

/// Ensure the shell startup files source our env.sh (idempotent).
fn ensure_sourced() -> Result<()> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    let source_line = format!(
        "\n# >>> sys >>>\n[ -f \"{}\" ] && . \"{}\"\n# <<< sys <<<\n",
        env_sh()?.display(),
        env_sh()?.display()
    );
    let marker = "# >>> sys >>>";

    let mut targets: Vec<PathBuf> = vec![PathBuf::from(&home).join(".profile")];
    for name in [".bashrc", ".zshrc"] {
        let p = PathBuf::from(&home).join(name);
        if p.exists() {
            targets.push(p);
        }
    }
    for target in targets {
        let existing = read_file(&target);
        if existing.contains(marker) {
            continue;
        }
        // Keep user content; create the file if missing.
        let mut content = existing;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&source_line);
        std::fs::write(&target, content)
            .with_context(|| format!("cannot update {}", target.display()))?;
    }
    Ok(())
}

pub fn get_path(scope: Scope) -> Option<String> {
    match scope {
        Scope::User => read_path_entries()
            .ok()
            .map(|e| common::join_path(&e)),
        Scope::Machine => read_etc_environment()
            .ok()
            .and_then(|m| m.get("PATH").cloned()),
    }
}

pub fn set_path(scope: Scope, value: &str) -> Result<()> {
    match scope {
        Scope::User => {
            let entries = common::split_path(value);
            ensure_dir()?;
            std::fs::write(path_file()?, common::build_txt_content(&entries))
                .with_context(|| format!("cannot write {}", path_file().unwrap().display()))?;
            regenerate_env_sh(&entries)?;
            ensure_sourced()?;
            Ok(())
        }
        Scope::Machine => {
            let mut env = read_etc_environment().unwrap_or_default();
            env.insert(
                "PATH".to_string(),
                common::join_path(&common::split_path(value)),
            );
            write_etc_environment(&env)
        }
    }
}

pub fn get_var(scope: Scope, name: &str) -> Option<String> {
    match scope {
        Scope::User => common::parse_vars_file(&read_file(&vars_file().ok()?))
            .into_iter()
            .find(|(k, _)| common::var_eq(k, name))
            .map(|(_, v)| v),
        Scope::Machine => read_etc_environment().ok().and_then(|m| m.get(name).cloned()),
    }
}

pub fn set_var(scope: Scope, name: &str, value: &str) -> Result<()> {
    if name.is_empty() {
        bail!("variable name must not be empty");
    }
    match scope {
        Scope::User => {
            ensure_dir()?;
            let mut vars = common::parse_vars_file(&read_file(&vars_file()?));
            if let Some(slot) = vars.iter_mut().find(|(k, _)| common::var_eq(k, name)) {
                slot.1 = value.to_string();
            } else {
                vars.push((name.to_string(), value.to_string()));
            }
            write_vars_file(&vars)?;
            let entries = read_path_entries()?;
            regenerate_env_sh(&entries)?;
            ensure_sourced()?;
            Ok(())
        }
        Scope::Machine => {
            let mut env = read_etc_environment().unwrap_or_default();
            env.insert(name.to_string(), value.to_string());
            write_etc_environment(&env)
        }
    }
}

pub fn unset_var(scope: Scope, name: &str) -> Result<()> {
    match scope {
        Scope::User => {
            ensure_dir()?;
            let mut vars = common::parse_vars_file(&read_file(&vars_file()?));
            vars.retain(|(k, _)| !common::var_eq(k, name));
            write_vars_file(&vars)?;
            let entries = read_path_entries()?;
            regenerate_env_sh(&entries)?;
            Ok(())
        }
        Scope::Machine => {
            let mut env = read_etc_environment().unwrap_or_default();
            env.remove(name);
            write_etc_environment(&env)
        }
    }
}

fn write_vars_file(vars: &[(String, String)]) -> Result<()> {
    let mut content = String::new();
    for (k, v) in vars {
        let esc = v.replace('\\', "\\\\").replace('\n', "\\n");
        content.push_str(&format!("{k}={esc}\n"));
    }
    std::fs::write(vars_file()?, content)
        .with_context(|| format!("cannot write {}", vars_file().unwrap().display()))
}

/// Read /etc/environment as a map of NAME -> value (quotes stripped).
fn read_etc_environment() -> Result<std::collections::HashMap<String, String>> {
    let content = std::fs::read_to_string("/etc/environment")
        .with_context(|| "cannot read /etc/environment".to_string())?;
    let mut map = std::collections::HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.trim().to_string(), unquote_value(v.trim()));
        }
    }
    Ok(map)
}

fn unquote_value(v: &str) -> String {
    let t = v.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        t[1..t.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
    } else {
        t.to_string()
    }
}

fn quote_value(v: &str) -> String {
    let esc = v.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{esc}\"")
}

fn write_etc_environment(map: &std::collections::HashMap<String, String>) -> Result<()> {
    // Preserve original comments/order where possible.
    let mut keys_in_order: Vec<String> = Vec::new();
    let original = std::fs::read_to_string("/etc/environment").unwrap_or_default();
    for line in original.lines() {
        let t = line.trim();
        if !t.is_empty() && !t.starts_with('#') {
            if let Some((k, _)) = t.split_once('=') {
                keys_in_order.push(k.trim().to_string());
            }
        }
    }
    for k in map.keys() {
        if !keys_in_order.iter().any(|x| x == k) {
            keys_in_order.push(k.clone());
        }
    }

    let mut content = String::new();
    for line in original.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            content.push_str(line);
            content.push('\n');
            continue;
        }
        if let Some((k, _)) = t.split_once('=') {
            let k = k.trim().to_string();
            if map.contains_key(&k) {
                content.push_str(&format!("{k}={}\n", quote_value(&map[&k])));
            }
        }
    }
    // Append any keys that were not in the original file.
    for k in &keys_in_order {
        if !original.lines().any(|l| {
            l.trim().split_once('=').map(|(k2, _)| k2.trim() == k).unwrap_or(false)
        }) {
            content.push_str(&format!("{k}={}\n", quote_value(&map[k])));
        }
    }

    std::fs::write("/etc/environment", content).with_context(|| {
        "cannot write /etc/environment (machine scope)\nhint: run with `sudo sys ... --scope machine` or as root"
            .to_string()
    })
}
