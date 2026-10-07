//! `sysenv file` — fd/sd-style text search and in-place replacement.
//!
//! Usage:
//!   sysenv file PATTERN                 search PATTERN in stdin (piped input)
//!   sysenv file PATTERN PATH            search PATTERN in PATH and its subtree
//!   sysenv file OLD NEW PATH            replace OLD with NEW in PATH (in place)
//!
//! Options:
//!   -e EXT       filter by file extension (repeatable, dot optional)
//!   -i           case-insensitive matching
//!   -t           only search plain-text files (txt/md/log/...); by default
//!                source-code files are searched too
//!   -w           match whole words only
//!   -c NUM       show NUM lines of context around every match
//!
//! All matching is done on the Unicode char level (case folding per char), so
//! `-i` and `-w` behave correctly for non-ASCII text.

use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};

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

/// Recursively collect target files under `dir` (sorted, deterministic).
fn walk(dir: &Path, exts: &Option<HashSet<String>>, text_only: bool, out: &mut Vec<PathBuf>) {
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
            dirs.push(path);
        } else if ft.is_file() && is_target_file(&path, exts, text_only) {
            out.push(path);
        }
    }
    dirs.sort();
    for d in dirs {
        walk(&d, exts, text_only, out);
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
        walk(p, &exts, opts.text_only, &mut files);
    } else if p.is_file() {
        files.push(p.to_path_buf());
    } else {
        bail!("path `{path}` does not exist");
    }
    Ok(files)
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
pub fn cmd_file(args: &[String], exts: &[String], ignore_case: bool, text_only: bool, word: bool, context: Option<usize>) -> Result<()> {
    let opts = FileOpts {
        exts: exts.to_vec(),
        ignore_case,
        text_only,
        word,
        context: context.unwrap_or(0),
    };
    match args.len() {
        0 => bail!(
            "usage:\n  sysenv file PATTERN             search PATTERN in stdin (piped)\n  sysenv file PATTERN PATH        search PATTERN in PATH\n  sysenv file OLD NEW PATH        replace OLD with NEW in PATH\noptions: -e EXT (repeatable) -i -t -w -c NUM"
        ),
        1 => search_stdin(&args[0], &opts),
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
}
