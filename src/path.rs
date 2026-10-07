//! `sys path` — CRUD for PATH entries, plus registry import/export.

use crate::store::common;
use crate::store::{Scope, current_path_entries, get_persisted_path, set_persisted_path};
use anyhow::{Context, Result, bail};

fn persisted_entries(scope: Scope) -> Vec<String> {
    get_persisted_path(scope)
        .map(|v| common::split_path(&v))
        .unwrap_or_default()
}

fn persist_entries(scope: Scope, entries: &[String]) -> Result<()> {
    let value = common::join_path(entries);
    set_persisted_path(scope, &value)
}

/// Apply a new PATH value to the current process and print a snippet so the
/// change can also be applied to the current interactive shell.
fn apply_to_current_env(new_entries: &[String]) {
    let new_value = common::join_path(new_entries);
    // Update this process so any child command spawned afterwards sees it.
    unsafe { std::env::set_var("PATH", &new_value) };
    println!("{}", common::shell_export_snippet("PATH", &new_value));
}

pub fn cmd_list() -> Result<()> {
    let entries = current_path_entries();
    if entries.is_empty() {
        println!("(empty PATH)");
        return Ok(());
    }
    for e in entries {
        println!("{e}");
    }
    Ok(())
}

pub fn cmd_add(dir: &str, scope: Scope, prepend: bool, temporary: bool) -> Result<()> {
    let abs = common::absolutize_dir(dir)?;
    let mut entries = if temporary {
        current_path_entries()
    } else {
        persisted_entries(scope)
    };
    let exists = entries.iter().any(|e| common::path_eq(e, &abs));
    if !exists {
        if prepend {
            entries.insert(0, abs.clone());
        } else {
            entries.push(abs.clone());
        }
    }
    if temporary {
        println!("[temporary] PATH updated for this session only (not persisted):");
        apply_to_current_env(&entries);
        println!("hint: run `{}` (or paste it) to apply to the current shell.", common::shell_export_snippet("PATH", &common::join_path(&entries)));
        return Ok(());
    }
    persist_entries(scope, &entries)?;
    let mut effective = current_path_entries();
    if !effective.iter().any(|e| common::path_eq(e, &abs)) {
        effective = common::merge_entries(&effective, &[abs.clone()]);
        // Recompute effective PATH including the new entry for this process.
        apply_to_current_env(&effective);
    }
    if exists {
        println!("{abs} is already in PATH (scope {}, {} entry)", scope.as_str(), "no change");
    } else {
        println!(
            "Added {abs} to PATH (scope {}).",
            scope.as_str()
        );
        println!("New processes will inherit it automatically.");
        println!(
            "To apply to the current shell, run: {}",
            common::shell_export_snippet("PATH", &common::join_path(&effective))
        );
    }
    Ok(())
}

pub fn cmd_remove(dir: &str, scope: Scope, temporary: bool) -> Result<()> {
    let abs = common::absolutize_dir(dir)?;
    let entries = if temporary {
        current_path_entries()
    } else {
        persisted_entries(scope)
    };
    let before_len = entries.len();
    let new_entries: Vec<String> = entries
        .into_iter()
        .filter(|e| !common::path_eq(e, &abs))
        .collect();
    let removed = new_entries.len() < before_len;
    if !removed {
        println!("{abs} was not found in PATH (scope {}).", scope.as_str());
        return Ok(());
    }
    if temporary {
        println!("[temporary] PATH updated for this session only:");
        apply_to_current_env(&new_entries);
        return Ok(());
    }
    persist_entries(scope, &new_entries)?;
    // Reflect in the current process too.
    let mut effective = current_path_entries();
    let before_len = effective.len();
    effective.retain(|e| !common::path_eq(e, &abs));
    if effective.len() != before_len {
        apply_to_current_env(&effective);
    }
    println!("Removed {abs} from PATH (scope {}).", scope.as_str());
    println!(
        "To apply to the current shell, run: {}",
        common::shell_export_snippet("PATH", &common::join_path(&effective))
    );
    Ok(())
}

pub fn cmd_has(dir: &str) -> Result<()> {
    let abs = common::absolutize_dir(dir)?;
    // Check the current process PATH first, then the persisted scopes so a
    // freshly added entry is reported even from a new invocation.
    let mut found = current_path_entries().iter().any(|e| common::path_eq(e, &abs));
    if !found {
        for scope in [Scope::User, Scope::Machine] {
            if let Some(entries) = get_persisted_path(scope)
                .map(|v| common::split_path(&v))
            {
                if entries.iter().any(|e| common::path_eq(e, &abs)) {
                    found = true;
                    break;
                }
            }
        }
    }
    if found {
        println!("yes: {abs} is in PATH");
    } else {
        println!("no: {abs} is not in PATH");
    }
    Ok(())
}

fn export_content(entries: &[String], ext: &str) -> Result<String> {
    match ext {
        "reg" => Ok(common::build_reg_content(
            entries,
            r"HKEY_CURRENT_USER\Environment",
            "Path",
        )),
        "json" => Ok(common::build_json_content(entries)),
        "txt" => Ok(common::build_txt_content(entries)),
        other => bail!(
            "unsupported export format `{other}` (use .reg, .json or .txt)"
        ),
    }
}

pub fn cmd_export(file: &str) -> Result<()> {
    let entries = current_path_entries();
    let ext = std::path::Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if ext.is_empty() {
        bail!("export file must have an extension: .reg, .json or .txt");
    }
    let content = export_content(&entries, &ext)?;
    std::fs::write(file, content)
        .with_context(|| format!("cannot write export file `{file}`"))?;
    println!("Exported {} PATH entries to {file} ({ext}).", entries.len());
    Ok(())
}

pub fn cmd_import(file: &str, scope: Scope, replace: bool) -> Result<()> {
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("cannot read import file `{file}`"))?;
    let ext = std::path::Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let imported = match ext.as_str() {
        "reg" => common::parse_reg_content(&content)?,
        "json" => common::parse_json_content(&content)?,
        "txt" | "conf" | "list" => common::parse_txt_content(&content),
        "env" => common::parse_txt_content(&content),
        other => {
            bail!("unsupported import format `{other}` (use .reg, .json or .txt)")
        }
    };
    if imported.is_empty() {
        println!("Import file `{file}` contained no PATH entries; nothing to do.");
        return Ok(());
    }
    let mut target = if replace {
        Vec::new()
    } else {
        persisted_entries(scope)
    };
    let before = target.len();
    let mut added = 0;
    for e in &imported {
        if !target.iter().any(|x| common::path_eq(x, e)) {
            target.push(e.clone());
            added += 1;
        }
    }
    persist_entries(scope, &target)?;
    let mut effective = current_path_entries();
    effective = common::merge_entries(&imported, &effective);
    let new_value = common::join_path(&effective);
    unsafe { std::env::set_var("PATH", &new_value) };
    println!(
        "Imported {added} new entries from {file} (scope {}); {} existing kept ({} total).",
        scope.as_str(),
        before,
        target.len()
    );
    println!(
        "To apply to the current shell, run: {}",
        common::shell_export_snippet("PATH", &new_value)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_content_formats() {
        let entries = vec!["C:\\a".to_string(), "D:\\b".to_string()];
        assert!(export_content(&entries, "reg").unwrap().contains("Path\"=\"C:\\\\a;D:\\\\b"));
        assert!(export_content(&entries, "json").unwrap().contains("\"entries\""));
        assert_eq!(export_content(&entries, "txt").unwrap(), "C:\\a\nD:\\b\n");
        assert!(export_content(&entries, "xyz").is_err());
    }
}
