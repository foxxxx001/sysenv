//! Platform-independent helpers shared by the Windows registry backend and the
//! Unix config-file backend. Everything here is pure and unit-testable on any OS.

use anyhow::{Context, Result};

/// Normalize a user-provided directory argument to an absolute path.
pub fn absolutize_dir(input: &str) -> Result<String> {
    let p = std::path::Path::new(input);
    if p.is_absolute() {
        return Ok(p.to_string_lossy().into_owned());
    }
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let joined = cwd.join(p);
    // Canonicalize when the path exists so `..` / `.` components are resolved.
    if joined.exists() {
        std::fs::canonicalize(&joined)
            .map(|c| c.to_string_lossy().into_owned())
            .with_context(|| format!("cannot resolve path `{input}`"))
    } else {
        Ok(joined.to_string_lossy().into_owned())
    }
}

/// Split a PATH-style value into entries using the OS path separator.
pub fn split_path(value: &str) -> Vec<String> {
    std::env::split_paths(value)
        .filter_map(|p| {
            let s = p.to_string_lossy().into_owned();
            let t = s.trim().to_string();
            if t.is_empty() { None } else { Some(t) }
        })
        .collect()
}

/// Join entries into a PATH-style value using the OS path separator.
pub fn join_path(entries: &[String]) -> String {
    std::env::join_paths(entries.iter().map(|e| std::path::Path::new(e)))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| entries.join(path_separator().to_string().as_str()))
}

/// The OS PATH separator as a char (`;` on Windows, `:` elsewhere).
pub fn path_separator() -> char {
    if cfg!(windows) { ';' } else { ':' }
}

/// Case sensitivity of PATH comparison for the current OS.
pub fn path_eq(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// Case sensitivity of environment variable names for the current OS.
#[cfg(any(not(windows), test))]
pub fn var_eq(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// Deduplicate entries in place (case-insensitive on Windows).
pub fn dedup_entries(entries: &mut Vec<String>) {
    let mut seen: Vec<String> = Vec::new();
    entries.retain(|e| {
        let dup = seen.iter().any(|s| path_eq(s, e));
        if dup {
            false
        } else {
            seen.push(e.clone());
            true
        }
    });
}

/// Merge `entries` with `candidates`, preserving order and deduplicating.
pub fn merge_entries(entries: &[String], candidates: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in entries {
        if !out.iter().any(|s| path_eq(s, e)) {
            out.push(e.clone());
        }
    }
    for c in candidates {
        if !out.iter().any(|s| path_eq(s, c)) {
            out.push(c.clone());
        }
    }
    out
}

/// Build a `.reg`-format document from a list of PATH entries.
/// `#[HKEY_CURRENT_USER\\Environment]` is the standard location for the user PATH.
pub fn build_reg_content(entries: &[String], hive: &str, value_name: &str) -> String {
    let joined = entries.join(";");
    let escaped = joined.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "Windows Registry Editor Version 5.00\n\n[{hive}]\n\"{value_name}\"=\"{escaped}\"\n"
    )
}

/// Parse a `.reg` document and return the value of the first `"Path"`-like string
/// value it contains.
pub fn parse_reg_content(content: &str) -> Result<Vec<String>> {
    let mut entries: Vec<String> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('"') && line.contains('=') {
            // "Name"="value"  -> split on first '=' then unescape.
            let (name_part, value_part) = line.split_once('=').context("malformed reg line")?;
            let name = unquote(name_part.trim().trim_end());
            // Only pick values that look like a PATH: "Path" or "PATH".
            if !name.eq_ignore_ascii_case("Path") {
                continue;
            }
            let value = unquote(value_part.trim().trim_start_matches("REG_EXPAND_SZ"));
            for e in split_path(&value) {
                if !entries.iter().any(|s| path_eq(s, &e)) {
                    entries.push(e);
                }
            }
        }
    }
    Ok(entries)
}

fn unquote(s: &str) -> String {
    let inner = s.trim().trim_start_matches('"').trim_end_matches('"');
    inner
        .replace("\\\\", "\\")
        .replace("\\\"", "\"")
        .replace("\\n", "\n")
}

/// JSON import/export document.
pub fn build_json_content(entries: &[String]) -> String {
    serde_json::json!({ "entries": entries, "separator": path_separator().to_string() })
        .to_string()
}

pub fn parse_json_content(content: &str) -> Result<Vec<String>> {
    let v: serde_json::Value = serde_json::from_str(content).context("invalid JSON file")?;
    let arr = if let Some(a) = v.get("entries").and_then(|x| x.as_array()) {
        a.clone()
    } else if let Some(a) = v.as_array() {
        a.clone()
    } else {
        anyhow::bail!("JSON must be an array or an object with an \"entries\" array");
    };
    let mut out: Vec<String> = Vec::new();
    for item in arr {
        let s = item
            .as_str()
            .context("JSON entries must all be strings")?
            .to_string();
        if !s.trim().is_empty() && !out.iter().any(|x| path_eq(x, &s)) {
            out.push(s);
        }
    }
    Ok(out)
}

/// Plain text export/import: one entry per line.
pub fn build_txt_content(entries: &[String]) -> String {
    entries.join("\n") + if entries.is_empty() { "" } else { "\n" }
}

pub fn parse_txt_content(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if !t.is_empty() && !out.iter().any(|x| path_eq(x, t)) {
            out.push(t.to_string());
        }
    }
    out
}

/// Generate the Unix `env.sh` file content managed by sysenv.
/// Exports PATH from the managed entries plus every extra variable.
#[cfg(any(not(windows), test))]
pub fn generate_env_sh(path_entries: &[String], vars: &[(String, String)]) -> String {
    let mut s = String::from("# Generated by sysenv. Do not edit manually.\n");
    let joined = path_entries.join(":");
    s.push_str(&format!(
        "export PATH=\"{joined}${{PATH:+:$PATH}}\"\n"
    ));
    for (k, v) in vars {
        let esc = v.replace('\\', "\\\\").replace('"', "\\\"");
        s.push_str(&format!("export {k}=\"{esc}\"\n"));
    }
    s
}

/// Parse a `NAME=VALUE` style vars file (one per line, `#` comments).
#[cfg(any(not(windows), test))]
pub fn parse_vars_file(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || !t.contains('=') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            let k = k.trim();
            if !k.is_empty() {
                out.push((k.to_string(), v.to_string()));
            }
        }
    }
    out
}

/// Read the current shell export snippet for applying `NAME=VALUE` to the
/// current interactive session (used by the `--temporary` option).
pub fn shell_export_snippet(name: &str, value: &str) -> String {
    match detect_shell() {
        ShellKind::PowerShell => {
            format!("$env:{} = '{}'", name, value.replace('\'', "''"))
        }
        ShellKind::Cmd => format!("set {name}={value}"),
        ShellKind::Fish => {
            format!("set -gx {name} \"{}\"", value.replace('"', "\\\""))
        }
        ShellKind::Bash => {
            let esc = value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$")
                .replace('`', "\\`");
            format!("export {name}=\"{esc}\"")
        }
    }
}

/// Snippet to unset a variable in the current session.
pub fn shell_unset_snippet(name: &str) -> String {
    match detect_shell() {
        ShellKind::PowerShell => format!("Remove-Item Env:{name} -ErrorAction SilentlyContinue"),
        ShellKind::Cmd => format!("set {name}="),
        ShellKind::Fish => format!("set -e {name}"),
        ShellKind::Bash => format!("unset {name}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ShellKind {
    PowerShell,
    Cmd,
    Fish,
    Bash,
}

fn detect_shell() -> ShellKind {
    if let Ok(shell) = std::env::var("SHELL") {
        let low = shell.to_ascii_lowercase();
        if low.ends_with("fish") {
            return ShellKind::Fish;
        }
        if low.ends_with("zsh") || low.ends_with("bash") || low.ends_with("sh") {
            return ShellKind::Bash;
        }
    }
    if cfg!(windows) {
        // PowerShell sets PSModulePath; plain cmd.exe does not.
        if std::env::var_os("PSModulePath").is_some() {
            ShellKind::PowerShell
        } else {
            ShellKind::Cmd
        }
    } else {
        ShellKind::Bash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn split_and_join_roundtrip() {
        let entries = vec!["C:\\a".to_string(), "D:\\b c".to_string()];
        let joined = join_path(&entries);
        let back = split_path(&joined);
        assert_eq!(back, entries);
    }

    #[test]
    #[cfg(windows)]
    fn dedup_works_case_insensitive_on_windows() {
        let mut v = vec!["C:\\A".to_string(), "C:\\a".to_string(), "B".to_string()];
        dedup_entries(&mut v);
        assert_eq!(v.len(), 2);
        if cfg!(windows) {
            assert!(path_eq(&v[0], "C:\\A"));
        } else {
            assert_eq!(v[0], "C:\\A");
        }
    }

    #[test]
    #[cfg(windows)]
    fn reg_content_roundtrip() {
        let entries = vec!["C:\\Program Files".to_string(), "%USERPROFILE%\\bin".to_string()];
        let content = build_reg_content(&entries, r"HKEY_CURRENT_USER\Environment", "Path");
        let parsed = parse_reg_content(&content).unwrap();
        assert_eq!(parsed, entries);
    }

    #[test]
    fn reg_escape_backslashes() {
        let content = build_reg_content(&["C:\\x\\y".to_string()], "HKEY", "Path");
        assert!(content.contains("C:\\\\x\\\\y"));
    }

    #[test]
    fn json_roundtrip() {
        let entries = vec!["/usr/bin".to_string(), "/opt/tool".to_string()];
        let content = build_json_content(&entries);
        let parsed = parse_json_content(&content).unwrap();
        assert_eq!(parsed, entries);
    }

    #[test]
    fn txt_roundtrip() {
        let entries = vec!["/a".to_string(), "/b".to_string()];
        let parsed = parse_txt_content(&build_txt_content(&entries));
        assert_eq!(parsed, entries);
    }

    #[test]
    fn env_sh_generation() {
        let sh = generate_env_sh(
            &["/opt/a".to_string(), "/opt/b".to_string()],
            &[("FOO".to_string(), "bar baz".to_string())],
        );
        assert!(sh.contains("export PATH=\"/opt/a:/opt/b"));
        assert!(sh.contains("export FOO=\"bar baz\""));
    }

    #[test]
    fn parse_vars() {
        let vars = parse_vars_file("FOO=bar baz\n# comment\nBAZ=1\n");
        assert_eq!(vars, vec![("FOO".to_string(), "bar baz".to_string()), ("BAZ".to_string(), "1".to_string())]);
        #[cfg(windows)]
        assert!(var_eq("FOO", "foo"));
        #[cfg(not(windows))]
        assert!(!var_eq("FOO", "foo"));
    }
}
