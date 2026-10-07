//! `sysenv file` — fd/sd-style text search, in-place replacement and file search.
//!
//! Usage:
//!   sysenv file PATTERN                 search PATTERN in stdin (piped input)
//!   sysenv file PATTERN PATH            search PATTERN in PATH and its subtree
//!   sysenv file OLD NEW PATH            replace OLD with NEW in PATH (in place)
//!   sysenv file NAME.EXT                display a file's content (e.g. me.txt)
//!   sysenv file -S 2m [PATH]            list files >= 2 MiB under PATH (default .)
//!   sysenv file --newer TIME [PATH]     list files modified at/after TIME
//!   sysenv file --older TIME [PATH]     list files modified before TIME
//!
//! A single argument is a stdin search unless it is quoted ("me.txt" -> search
//! the string) or carries an extension (me.txt -> display that file).
//!
//! Options:
//!   -e EXT       filter by file extension (repeatable, dot optional)
//!   -i           case-insensitive matching
//!   -t           only search plain-text files (txt/md/log/...); by default
//!                source-code files are searched too
//!   -w           match whole words only
//!   -c NUM       show NUM lines of context around every match
//!   -S SIZE      only files at least SIZE bytes (plain number = bytes;
//!                2k/2m/2g/2t = 1024-based KiB/MiB/GiB/TiB)
//!   --newer TIME only files modified at/after TIME (YYYY-MM-DD [HH:MM[:SS]])
//!   --older TIME only files modified before TIME
//!   -d NUM       descend at most NUM levels of subdirectories (0 = current dir)
//!
//! All matching is done on the Unicode char level (case folding per char), so
//! `-i` and `-w` behave correctly for non-ASCII text.

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use chrono::{Local, NaiveDateTime, TimeZone};
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Plain-text extensions (searched when `-t` is set, and always).
const TEXT_EXTS: &[&str] = &[
    "txt", "md", "markdown", "log", "csv", "tsv", "json", "yaml", "yml", "xml", "html", "htm",
    "css", "ini", "conf", "cfg", "toml", "env", "rst", "tex", "org", "diff", "patch", "srt",
    "vtt", "text", "nfo", "asc",
];

/// Source-code extensions (added to the search when `-t` is NOT set).
const CODE_EXTS: &[&str] = &[
    "c", "h", "cpp", "hpp", "cc", "cxx", "hh", "java", "py", "js", "mjs", "cjs", "ts", "tsx",
    "jsx", "go", "rs", "rb", "php", "swift", "kt", "kts", "scala", "sh", "bash", "zsh", "ps1",
    "bat", "cmd", "sql", "vue", "svelte", "lua", "pl", "pm", "r", "m", "dart", "cs", "fs", "hs",
    "clj", "ex", "exs", "erl", "zig", "nim", "asm", "s", "f", "f90", "cu", "proto", "groovy",
    "gradle", "cmake", "mk", "dockerfile", "tf", "sol", "cob", "pas", "ada", "ahk", "tcl", "jl",
];

/// Directories never descended into (compared case-insensitively).
const SKIP_DIRS: &[&str] = &[
    ".git", ".svn", ".hg", "node_modules", "target", "dist", "build", "__pycache__", ".venv",
    "venv", ".idea", ".vscode", ".settings", ".mypy_cache", ".pytest_cache", "vendor",
];

/// Parsed options shared by every mode.
#[derive(Clone, Debug)]
pub struct FileOpts {
    /// `-e` extensions (lowercased, dot stripped); empty = no extension filter.
    pub exts: Vec<String>,
    /// `-i`
    pub ignore_case: bool,
    /// `-t`
    pub text_only: bool,
    /// `-w`
    pub word: bool,
    /// `-c NUM`
    pub context: usize,
    /// `-S SIZE` in bytes; files smaller than this are skipped.
    pub min_size: Option<u64>,
    /// `--newer TIME` as a unix timestamp (local timezone); older files skipped.
    pub newer: Option<i64>,
    /// `--older TIME` as a unix timestamp (local timezone); newer files skipped.
    pub older: Option<i64>,
    /// `-d NUM` maximum subdirectory depth (0 = current directory only).
    pub max_depth: Option<usize>,
}

impl FileOpts {
    /// Normalized extension set for quick lookup (lowercased, leading dot stripped).
    fn ext_set(&self) -> Option<HashSet<String>> {
        if self.exts.is_empty() {
            return None;
        }
        Some(
            self.exts
                .iter()
                .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
                .filter(|e| !e.is_empty())
                .collect(),
        )
    }
}

// ---------------------------------------------------------------------------
// Attribute filters (size / time / depth)
// ---------------------------------------------------------------------------

/// Parse a size argument: a plain number is bytes; a trailing unit k/kb, m/mb,
/// g/gb, t/tb scales by 1024 (case-insensitive). Decimals like 1.5m are allowed.
fn parse_size(s: &str) -> Result<u64> {
    let t = s.trim();
    if t.is_empty() {
        bail!("-S requires a size");
    }
    if let Ok(n) = t.parse::<u64>() {
        return Ok(n);
    }
    let split = t
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .ok_or_else(|| anyhow::anyhow!("cannot parse size `{t}`"))?;
    let num: f64 = t[..split]
        .parse()
        .map_err(|_| anyhow::anyhow!("cannot parse size `{t}`"))?;
    let unit = t[split..].to_ascii_lowercase();
    let mult: u64 = match unit.as_str() {
        "k" | "kb" => 1 << 10,
        "m" | "mb" => 1 << 20,
        "g" | "gb" => 1 << 30,
        "t" | "tb" => 1 << 40,
        other => bail!("unknown size unit `{other}` (expected k/m/g or kb/mb/gb)"),
    };
    Ok((num * mult as f64) as u64)
}

/// Parse a local date/time argument into a unix timestamp (seconds).
/// Accepts YYYY-MM-DD [HH:MM[:SS]] with ` ` or `T` separators, and slashes.
fn parse_datetime(s: &str) -> Result<i64> {
    let t = s.trim();
    if t.is_empty() {
        bail!("time argument is empty (expected YYYY-MM-DD [HH:MM[:SS]])");
    }
    const FORMATS: &[&str] = &[
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d",
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d %H:%M",
        "%Y/%m/%d",
    ];
    for f in FORMATS {
        if let Ok(dt) = NaiveDateTime::parse_from_str(t, f) {
            return local_timestamp(dt);
        }
        if let Ok(d) = NaiveDate::parse_from_str(t, f) {
            if let Some(dt) = d.and_hms_opt(0, 0, 0) {
                return local_timestamp(dt);
            }
        }
    }
    bail!("cannot parse time `{t}` (expected YYYY-MM-DD [HH:MM[:SS]])")
}

/// Interpret a naive local datetime in the local timezone and return unix seconds.
fn local_timestamp(dt: NaiveDateTime) -> Result<i64> {
    let local = Local
        .from_local_datetime(&dt)
        .single()
        .ok_or_else(|| anyhow::anyhow!("invalid or ambiguous local time `{dt}`"))?;
    Ok(local.timestamp())
}

/// File size in bytes (0 when metadata is unavailable).
fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// File modification time in unix seconds (0 when unavailable).
fn file_mtime(path: &Path) -> i64 {
    fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// True when the file passes the -S / --newer / --older attribute filters.
fn passes_attrs(path: &Path, opts: &FileOpts) -> bool {
    if let Some(min) = opts.min_size {
        if file_size(path) < min {
            return false;
        }
    }
    if let Some(t) = opts.newer {
        if file_mtime(path) < t {
            return false;
        }
    }
    if let Some(t) = opts.older {
        if file_mtime(path) >= t {
            return false;
        }
    }
    true
}

/// Strip a wrapping `"..."` / `'...'` pair; None when not quoted.
fn strip_quotes(s: &str) -> Option<&str> {
    let t = s.trim();
    if t.len() >= 2 {
        let b = t.as_bytes();
        let same = (b[0] == b'"' && b[t.len() - 1] == b'"') || (b[0] == b'\'' && b[t.len() - 1] == b'\'');
        if same {
            return Some(&t[1..t.len() - 1]);
        }
    }
    None
}

/// True when a single-arg token carries a file extension (`me.txt`, `a.b.c`,
/// `D:\x\y.log`); hidden names like `.env` have none and stay a search pattern.
fn looks_like_file(s: &str) -> bool {
    Path::new(s.trim()).extension().is_some()
}

/// Read a file for display (binary-safe, lossy UTF-8).
fn read_showable(name: &str) -> Result<String> {
    let p = Path::new(name);
    if !p.is_file() {
        bail!("`{name}` is not a file (no such file?)");
    }
    let bytes = fs::read(p).with_context(|| format!("cannot read {}", p.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

// ---------------------------------------------------------------------------
// Matching core (char-level)
// ---------------------------------------------------------------------------

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// First occurrence of `needle` in `hay` starting at char index `start`
/// (None when absent). `ignore_case` folds both sides per char.
fn find_chars(hay: &[char], needle: &[char], start: usize, ignore_case: bool) -> Option<usize> {
    if needle.is_empty() {
        return Some(start.min(hay.len()));
    }
    let mut i = start;
    while i + needle.len() <= hay.len() {
        let mut ok = true;
        for j in 0..needle.len() {
            let a = if ignore_case {
                hay[i + j].to_lowercase().next()
            } else {
                Some(hay[i + j])
            };
            let b = if ignore_case {
                needle[j].to_lowercase().next()
            } else {
                Some(needle[j])
            };
            if a != b {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// True when the match starting at char index `start` is word-bounded.
fn is_word_match_chars(hay: &[char], needle_len: usize, start: usize) -> bool {
    let before = if start > 0 { Some(hay[start - 1]) } else { None };
    let after = if start + needle_len < hay.len() {
        Some(hay[start + needle_len])
    } else {
        None
    };
    !before.map(is_word_char).unwrap_or(false) && !after.map(is_word_char).unwrap_or(false)
}

/// Every match start (char index) in `hay`.
fn match_positions(hay: &[char], needle: &[char], ignore_case: bool, word: bool) -> Vec<usize> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(pos) = find_chars(hay, needle, start, ignore_case) {
        if !word || is_word_match_chars(hay, needle.len(), pos) {
            out.push(pos);
        }
        start = pos + needle.len().max(1);
    }
    out
}

/// Replace every occurrence of `needle` in `hay` with `replacement`; returns
/// the number of replacements.
fn replace_all(
    hay: &mut Vec<char>,
    needle: &[char],
    replacement: &[char],
    ignore_case: bool,
    word: bool,
) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        let mut ok = true;
        for j in 0..needle.len() {
            let a = if ignore_case {
                hay[i + j].to_lowercase().next()
            } else {
                Some(hay[i + j])
            };
            let b = if ignore_case {
                needle[j].to_lowercase().next()
            } else {
                Some(needle[j])
            };
            if a != b {
                ok = false;
                break;
            }
        }
        if ok && (!word || is_word_match_chars(hay, needle.len(), i)) {
            hay.splice(i..i + needle.len(), replacement.iter().cloned());
            i += replacement.len().max(1);
            count += 1;
        } else {
            i += 1;
        }
    }
    count
}

/// True when `line` contains at least one match.
fn line_matches(line: &str, needle: &[char], ignore_case: bool, word: bool) -> bool {
    let chars: Vec<char> = line.chars().collect();
    !match_positions(&chars, needle, ignore_case, word).is_empty()
}

// ---------------------------------------------------------------------------
// Output building (pure, unit-testable)
// ---------------------------------------------------------------------------

/// Build the grep-style output lines for `lines` (0-based indexes).
/// `prefix` is the file path shown as `prefix:line:content`; None prints the
/// bare line (stdin mode). Context groups are separated by `--`.
fn build_match_output(
    lines: &[String],
    prefix: Option<&Path>,
    needle: &[char],
    ignore_case: bool,
    word: bool,
    context: usize,
) -> Vec<String> {
    let matches: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| line_matches(l, needle, ignore_case, word))
        .map(|(i, _)| i)
        .collect();
    let mut out = Vec::new();
    if matches.is_empty() {
        return out;
    }
    let mut i = 0;
    let mut first_group = true;
    while i < matches.len() {
        let gs = matches[i];
        let mut ge = matches[i];
        i += 1;
        while i < matches.len() && matches[i] - ge <= 2 * context + 1 {
            ge = matches[i];
            i += 1;
        }
        let start = gs.saturating_sub(context);
        let end = (ge + context).min(lines.len() - 1);
        if !first_group {
            out.push("--".to_string());
        }
        first_group = false;
        for ln in start..=end {
            let line = &lines[ln];
            match prefix {
                Some(p) => out.push(format!("{}:{}:{}", p.display(), ln + 1, line)),
                None => out.push(line.clone()),
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// File discovery / reading
// ---------------------------------------------------------------------------

/// Decide whether a file should be searched, based on its extension.
fn is_target_file(path: &Path, exts: &Option<HashSet<String>>, text_only: bool) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if name.starts_with('.') {
        return false; // hidden files are skipped (fd convention)
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match exts {
        Some(es) => es.contains(&ext) || es.contains(&name),
        None if text_only => TEXT_EXTS.contains(&ext.as_str()),
        None => TEXT_EXTS.contains(&ext.as_str()) || CODE_EXTS.contains(&ext.as_str()),
    }
}

/// Recursively collect files under `dir` (sorted, deterministic).
///
/// `ext_filter` applies the text/source extension filter (search/replace modes);
/// when false every file is collected (list mode, filters applied later).
/// `max_depth` bounds subdirectory descent (root = depth 0).
fn walk(
    dir: &Path,
    exts: &Option<HashSet<String>>,
    text_only: bool,
    ext_filter: bool,
    max_depth: Option<usize>,
    depth: usize,
    out: &mut Vec<PathBuf>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let name = entry
            .file_name()
            .to_string_lossy()
            .to_ascii_lowercase();
        if ft.is_dir() {
            if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            if max_depth.is_some_and(|m| depth + 1 > m) {
                continue;
            }
            dirs.push(path);
        } else if ft.is_file() && (!ext_filter || is_target_file(&path, exts, text_only)) {
            out.push(path);
        }
    }
    dirs.sort();
    for d in dirs {
        walk(&d, exts, text_only, ext_filter, max_depth, depth + 1, out);
    }
    out.sort();
}

/// Read a file as UTF-8 text; Ok(None) for binary or non-UTF-8 files.
fn read_text_file(path: &Path) -> Result<Option<String>> {
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return Ok(None); // NUL byte -> binary
    }
    match String::from_utf8(bytes) {
        Ok(s) => Ok(Some(s)),
        Err(_) => Ok(None),
    }
}

/// Collect the target files for a PATH argument (file or directory tree).
fn collect_files(path: &str, opts: &FileOpts) -> Result<Vec<PathBuf>> {
    let p = Path::new(path);
    let exts = opts.ext_set();
    let mut files = Vec::new();
    if p.is_dir() {
        walk(p, &exts, opts.text_only, true, opts.max_depth, 0, &mut files);
    } else if p.is_file() {
        files.push(p.to_path_buf());
    } else {
        bail!("path `{path}` does not exist");
    }
    Ok(files)
}

/// List mode: collect every file (extension filter applied later), then print
/// the ones passing -e / -t / -S / --newer / --older.
fn list_files(path: &str, opts: &FileOpts) -> Result<()> {
    let p = Path::new(path);
    let mut files = Vec::new();
    if p.is_dir() {
        let exts = opts.ext_set();
        walk(p, &exts, opts.text_only, false, opts.max_depth, 0, &mut files);
    } else if p.is_file() {
        files.push(p.to_path_buf());
    } else {
        bail!("path `{path}` does not exist");
    }
    let es = opts.ext_set();
    for f in &files {
        if !is_visible(&f, &es, opts.text_only) {
            continue;
        }
        if !passes_attrs(f, opts) {
            continue;
        }
        println!("{}", f.display());
    }
    Ok(())
}

/// Visibility filter for list mode (hidden files skipped; `-e` / `-t` apply).
fn is_visible(path: &Path, exts: &Option<HashSet<String>>, text_only: bool) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if name.starts_with('.') {
        return false;
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match exts {
        Some(es) => es.contains(&ext) || es.contains(&name),
        None if text_only => TEXT_EXTS.contains(&ext.as_str()),
        None => true,
    }
}

// ---------------------------------------------------------------------------
// Modes
// ---------------------------------------------------------------------------

/// `sysenv file PATTERN` — read stdin and print matching lines.
fn search_stdin(needle: &str, opts: &FileOpts) -> Result<()> {
    let stdin = io::stdin();
    let mut lines: Vec<String> = Vec::new();
    for line in stdin.lock().lines() {
        lines.push(line.context("stdin read error")?);
    }
    let n: Vec<char> = needle.chars().collect();
    let out = build_match_output(&lines, None, &n, opts.ignore_case, opts.word, opts.context);
    for l in &out {
        println!("{l}");
    }
    Ok(())
}

/// `sysenv file PATTERN PATH` — search PATH (file or directory tree).
fn search_path(needle: &str, path: &str, opts: &FileOpts) -> Result<()> {
    let files = collect_files(path, opts)?;
    let n: Vec<char> = needle.chars().collect();
    let mut matched_files = 0usize;
    for f in &files {
        if !passes_attrs(f, opts) {
            continue;
        }
        let Some(content) = read_text_file(f)? else { continue };
        let lines: Vec<String> = content.lines().map(str::to_string).collect();
        let out = build_match_output(&lines, Some(f), &n, opts.ignore_case, opts.word, opts.context);
        if !out.is_empty() {
            matched_files += 1;
            for l in &out {
                println!("{l}");
            }
        }
    }
    if matched_files == 0 {
        println!("(no matches in {} files)", files.len());
    }
    Ok(())
}

/// `sysenv file OLD NEW PATH` — replace OLD with NEW in place.
fn replace_path(old: &str, new: &str, path: &str, opts: &FileOpts) -> Result<()> {
    let files = collect_files(path, opts)?;
    let needle: Vec<char> = old.chars().collect();
    let replacement: Vec<char> = new.chars().collect();
    let mut modified = 0usize;
    let mut total = 0usize;
    for f in &files {
        if !passes_attrs(f, opts) {
            continue;
        }
        let Some(content) = read_text_file(f)? else { continue };
        let mut chars: Vec<char> = content.chars().collect();
        let n = replace_all(&mut chars, &needle, &replacement, opts.ignore_case, opts.word);
        if n > 0 {
            let out: String = chars.into_iter().collect();
            fs::write(f, out).with_context(|| format!("cannot write {}", f.display()))?;
            modified += 1;
            total += n;
            println!("{}: {} 处替换", f.display(), n);
        }
    }
    if modified == 0 {
        println!("未找到匹配，无文件被修改。");
    } else {
        println!("已修改 {modified} 个文件，共替换 {total} 处。");
    }
    Ok(())
}

/// Entry point: `sysenv file ...`
#[allow(clippy::too_many_arguments)]
pub fn cmd_file(
    args: &[String],
    exts: &[String],
    ignore_case: bool,
    text_only: bool,
    word: bool,
    context: Option<usize>,
    size: Option<String>,
    newer: Option<String>,
    older: Option<String>,
    max_depth: Option<usize>,
) -> Result<()> {
    let opts = FileOpts {
        exts: exts.to_vec(),
        ignore_case,
        text_only,
        word,
        context: context.unwrap_or(0),
        min_size: match size {
            Some(s) => Some(parse_size(&s)?),
            None => None,
        },
        newer: match newer {
            Some(t) => Some(parse_datetime(&t)?),
            None => None,
        },
        older: match older {
            Some(t) => Some(parse_datetime(&t)?),
            None => None,
        },
        max_depth,
    };
    let has_filter = opts.min_size.is_some()
        || opts.newer.is_some()
        || opts.older.is_some()
        || opts.max_depth.is_some();
    match args.len() {
        0 if has_filter => list_files(".", &opts),
        1 if has_filter => list_files(&args[0], &opts),
        0 => bail!(
            "usage:\n  sysenv file PATTERN             search PATTERN in stdin (piped)\n  sysenv file PATTERN PATH        search PATTERN in PATH\n  sysenv file OLD NEW PATH        replace OLD with NEW in PATH\n  sysenv file NAME.EXT            display a file (e.g. file me.txt)\n  sysenv file -S SIZE [PATH]      list files by size (e.g. -S 2m)\n  sysenv file --newer TIME [PATH] list files by mtime\noptions: -e EXT (repeatable) -i -t -w -c NUM -S SIZE --newer TIME --older TIME -d NUM"
        ),
        1 => {
            if let Some(inner) = strip_quotes(&args[0]) {
                // Quoted string: force the search meaning ("me.txt" -> search).
                search_stdin(inner, &opts)
            } else if looks_like_file(&args[0]) {
                // Token with an extension (me.txt) -> display the file.
                let text = read_showable(&args[0])?;
                print!("{text}");
                if !text.ends_with('\n') {
                    println!();
                }
                Ok(())
            } else {
                search_stdin(&args[0], &opts)
            }
        }
        2 => search_path(&args[0], &args[1], &opts),
        3 => replace_path(&args[0], &args[1], &args[2], &opts),
        _ => bail!("too many arguments (expected PATTERN [REPLACEMENT] [PATH])"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn find_chars_basic_and_ignore_case() {
        assert_eq!(find_chars(&chars("hello world"), &chars("world"), 0, false), Some(6));
        assert_eq!(find_chars(&chars("hello world"), &chars("nope"), 0, false), None);
        assert_eq!(find_chars(&chars("Hello World"), &chars("world"), 0, true), Some(6));
        assert_eq!(find_chars(&chars("Hello World"), &chars("world"), 0, false), None);
        // 中文大小写不敏感（无大小写，但逐字比较）
        assert_eq!(find_chars(&chars("你好世界"), &chars("世界"), 0, false), Some(2));
    }

    #[test]
    fn match_positions_respects_whole_word() {
        let hay = chars("foo foobar bar foo");
        assert_eq!(match_positions(&hay, &chars("foo"), false, false), vec![0, 4, 15]);
        assert_eq!(match_positions(&hay, &chars("foo"), false, true), vec![0, 15]);
        let zh = chars("abc_foo foo-bar");
        assert_eq!(match_positions(&zh, &chars("foo"), false, true), vec![8]);
    }

    #[test]
    fn replace_all_literal_and_case() {
        let mut v = chars("foo FOo bar foo");
        assert_eq!(replace_all(&mut v, &chars("foo"), &chars("X"), true, false), 3);
        assert_eq!(v.iter().collect::<String>(), "X X bar X");

        let mut w = chars("the cat and the dog");
        assert_eq!(replace_all(&mut w, &chars("the"), &chars("a"), false, true), 2);
        assert_eq!(w.iter().collect::<String>(), "a cat and a dog");

        let mut zh = chars("苹果苹果香蕉");
        assert_eq!(replace_all(&mut zh, &chars("苹果"), &chars("梨"), false, false), 2);
        assert_eq!(zh.iter().collect::<String>(), "梨梨香蕉");
    }

    #[test]
    fn replace_all_word_boundary() {
        let mut v = chars("cat category cat");
        assert_eq!(replace_all(&mut v, &chars("cat"), &chars("dog"), false, true), 2);
        assert_eq!(v.iter().collect::<String>(), "dog category dog");
    }

    #[test]
    fn build_output_groups_context() {
        let lines: Vec<String> = (0..10).map(|i| format!("line{i}")).collect();
        let needle = chars("line2");
        let out = build_match_output(&lines, None, &needle, false, false, 1);
        assert_eq!(out, vec!["line1".to_string(), "line2".to_string(), "line3".to_string()]);
    }

    #[test]
    fn build_output_two_groups() {
        let lines: Vec<String> = (0..10).map(|i| format!("line{i}")).collect();
        let needle = chars("line2");
        // 需要两处匹配：构造匹配 line2 和 line8
        let mut lines2 = lines.clone();
        lines2[8] = "line2".to_string();
        let out = build_match_output(&lines2, None, &needle, false, false, 1);
        assert_eq!(
            out,
            vec![
                "line1".to_string(),
                "line2".to_string(),
                "line3".to_string(),
                "--".to_string(),
                "line7".to_string(),
                "line2".to_string(),
                "line9".to_string(),
            ]
        );
    }

    #[test]
    fn build_output_with_prefix() {
        let lines = vec!["hello world".to_string(), "world".to_string()];
        let out = build_match_output(&lines, Some(Path::new("a/b.txt")), &chars("world"), false, false, 0);
        assert_eq!(out, vec!["a/b.txt:1:hello world".to_string(), "a/b.txt:2:world".to_string()]);
    }

    #[test]
    fn target_file_extension_filter() {
        let text = Path::new("note.md");
        let code = Path::new("main.rs");
        let data = Path::new("data.json");
        let bin = Path::new("app.exe");
        assert!(is_target_file(text, &None, false));
        assert!(is_target_file(code, &None, false));
        assert!(!is_target_file(code, &None, true)); // -t 排除源码
        assert!(is_target_file(data, &None, true));
        assert!(!is_target_file(bin, &None, false));
        let es = Some(HashSet::from(["py".to_string(), "md".to_string()]));
        assert!(is_target_file(text, &es, false));
        assert!(is_target_file(Path::new("s.py"), &es, false));
        assert!(!is_target_file(code, &es, false));
        assert!(!is_target_file(Path::new(".env"), &es, false)); // 隐藏文件跳过
    }

    #[test]
    fn empty_needle_safe() {
        assert_eq!(find_chars(&chars("abc"), &chars(""), 0, false), Some(0));
        let mut v = chars("abc");
        assert_eq!(replace_all(&mut v, &chars(""), &chars("x"), false, false), 0);
    }

    #[test]
    fn parse_size_units() {
        assert_eq!(parse_size("100").unwrap(), 100);
        assert_eq!(parse_size("2k").unwrap(), 2 * 1024);
        assert_eq!(parse_size("2K").unwrap(), 2 * 1024);
        assert_eq!(parse_size("2kb").unwrap(), 2 * 1024);
        assert_eq!(parse_size("2m").unwrap(), 2 * 1024 * 1024);
        assert_eq!(parse_size("2MB").unwrap(), 2 * 1024 * 1024);
        assert_eq!(parse_size("2g").unwrap(), 2 * 1024 * 1024 * 1024);
        assert_eq!(parse_size("1.5k").unwrap(), 1536);
        assert!(parse_size("").is_err());
        assert!(parse_size("2x").is_err());
        assert!(parse_size("abc").is_err());
    }

    #[test]
    fn parse_datetime_formats() {
        let d0 = parse_datetime("2026-10-01").unwrap();
        let d1 = parse_datetime("2026-10-01 00:00").unwrap();
        let d2 = parse_datetime("2026-10-01 00:00:00").unwrap();
        let d3 = parse_datetime("2026-10-01T00:00:00").unwrap();
        let d4 = parse_datetime("2026/10/01").unwrap();
        assert_eq!(d0, d1);
        assert_eq!(d1, d2);
        assert_eq!(d2, d3);
        assert_eq!(d3, d4);
        // 同一本地时区下，+1 分钟 = 60 秒
        let later = parse_datetime("2026-10-01 00:01").unwrap();
        assert_eq!(later - d0, 60);
        assert!(parse_datetime("not a date").is_err());
        assert!(parse_datetime("2026-13-45").is_err());
    }

    #[test]
    fn attrs_size_filter() {
        // 以本文件自身为样本（肯定远大于 1 KiB）
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/file.rs");
        let small = FileOpts { min_size: Some(1024), ..opts_base() };
        assert!(passes_attrs(&p, &small));
        let huge = FileOpts { min_size: Some(1 << 40), ..opts_base() };
        assert!(!passes_attrs(&p, &huge));
    }

    #[test]
    fn walk_respects_max_depth() {
        let base = std::env::temp_dir().join(format!("sysenv_filedepth_{}", std::process::id()));
        let sub = base.join("a").join("b");
        fs::create_dir_all(&sub).unwrap();
        fs::write(base.join("r.txt"), "x").unwrap();
        fs::write(base.join("a").join("l1.txt"), "x").unwrap();
        fs::write(sub.join("l2.txt"), "x").unwrap();
        let mut files = Vec::new();
        walk(&base, &None, false, false, Some(0), 0, &mut files);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name().unwrap().to_str().unwrap(), "r.txt");
        let mut files = Vec::new();
        walk(&base, &None, false, false, Some(1), 0, &mut files);
        let names: Vec<String> = files.iter().map(|f| f.file_name().unwrap().to_str().unwrap().to_string()).collect();
        assert!(names.contains(&"r.txt".to_string()));
        assert!(names.contains(&"l1.txt".to_string()));
        assert!(!names.contains(&"l2.txt".to_string()));
        let mut files = Vec::new();
        walk(&base, &None, false, false, None, 0, &mut files);
        assert_eq!(files.len(), 3);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn single_arg_dispatch_helpers() {
        // 引号包裹 → 剥引号
        assert_eq!(strip_quotes("\"hello\""), Some("hello"));
        assert_eq!(strip_quotes("'me.txt'"), Some("me.txt"));
        assert_eq!(strip_quotes("  \"x\"  "), Some("x"));
        assert_eq!(strip_quotes("hello"), None);
        assert_eq!(strip_quotes("\"unbalanced"), None);
        assert_eq!(strip_quotes(""), None);
        // 后缀判断：有扩展名 → 文件显示；无扩展名 / 隐藏名 → 搜索
        assert!(looks_like_file("me.txt"));
        assert!(looks_like_file("a.b.c"));
        assert!(looks_like_file(r"D:\x\y.log"));
        assert!(looks_like_file("note.md"));
        assert!(!looks_like_file("hello"));
        assert!(!looks_like_file("hello world"));
        assert!(!looks_like_file(".env"));
        assert!(!looks_like_file("README"));
    }

    #[test]
    fn show_file_reads_content() {
        let base = std::env::temp_dir().join(format!("sysenv_show_{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let f = base.join("t.txt");
        fs::write(&f, "line1\nline2").unwrap();
        assert_eq!(read_showable(f.to_str().unwrap()).unwrap(), "line1\nline2");
        // 不存在的文件报错
        assert!(read_showable(base.join("nope.txt").to_str().unwrap()).is_err());
        fs::remove_dir_all(&base).unwrap();
    }

    fn opts_base() -> FileOpts {
        FileOpts {
            exts: Vec::new(),
            ignore_case: false,
            text_only: false,
            word: false,
            context: 0,
            min_size: None,
            newer: None,
            older: None,
            max_depth: None,
        }
    }
}
