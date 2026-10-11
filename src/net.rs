//! `sys net` — probe domains / URLs read from a file.
//!
//! The input file is read line by line:
//! - lines starting with `#` (after leading whitespace) are skipped;
//! - each http(s) address runs from `http` to the next whitespace, with
//!   trailing punctuation trimmed; one line may hold several addresses.
//!
//! Detection types (`-t`):
//! - `domain` (default): GET each address 3 times and average the response
//!   time, then print every address sorted by that average;
//! - `tv` (old name `json`): GET each address; the body must parse as JSON,
//!   then the data is analyzed (`.sites` count, `.sites.name` 4k count,
//!   `.spider` download speed) and sorted with `--sort`;
//! - `file`: GET each address; the body must be plain text (UTF-8, not JSON,
//!   not binary) — per-address reporting as each probe finishes;
//! - `test`: GET each address and probe reachability only; unreachable
//!   addresses are reported on screen and removed from the source lines, then
//!   the cleaned source (kept lines verbatim) is written to `-o`.
//! - `info`: query domain registration info via RDAP for each `-n DOMAIN`;
//!   when a name has no registration data, its parent domain is tried next.
//!
//! `-o FILE` writes every address that passed (reachable + format matched) to
//! FILE, one per line (file modes) or url + `#` info line (tv mode), or the
//! cleaned source lines (test mode).

use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashSet;
use std::error::Error as _;
use std::path::Path;
use std::time::{Duration, Instant};

/// Detection type for `sys net -t`.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetType {
    /// Probe the address 3 times and average the response time
    #[value(name = "domain")]
    Domain,
    /// The response body must parse as JSON (old name: json)
    #[value(name = "tv", alias = "json")]
    Json,
    /// Like tv, but accepts both tv JSON (`.sites` -> "单仓" group) and
    /// aggregator JSON (`.urls` -> "多仓" group); the two groups are printed
    /// separately, multi-warehouse items sorted by url count only
    #[value(name = "tvs")]
    Tvs,
    /// The response body must be plain text (not JSON, not binary)
    #[value(name = "file")]
    File,
    /// Probe reachability; unreachable addresses are reported, removed from
    /// the source lines, and the cleaned result is written to `-o`
    #[value(name = "test")]
    Test,
    /// Query domain registration info via RDAP (-n DOMAIN); when a name has
    /// no registration data, fall back to its parent domain
    #[value(name = "info")]
    Info,
}

/// Default per-request timeout in seconds.
pub const DEFAULT_TIMEOUT_SECS: f64 = 10.0;

/// Number of probes per address in `domain` mode.
const DOMAIN_PROBES: usize = 3;

// ---------------------------------------------------------------------------
// Input parsing
// ---------------------------------------------------------------------------

/// One http(s) address found inside a source line: its byte range in the line
/// (`start..end`, including any trailing punctuation that the extractor kept)
/// and the normalized URL used for probing.
struct UrlSpan {
    start: usize,
    end: usize,
    url: String,
}

/// Extract http(s) addresses from one line, preserving the original case of
/// each address. An address starts at `http` (case-insensitive) and extends to
/// the next whitespace; trailing punctuation is trimmed. A line may contain
/// several addresses.
fn extract_address_spans(line: &str) -> Vec<UrlSpan> {
    let mut out = Vec::new();
    let lower = line.to_ascii_lowercase();
    let mut i = 0usize;
    while i < lower.len() {
        let Some(rel) = lower[i..].find("http") else { break };
        let start = i + rel;
        let after = &lower[start + 4..];
        if !(after.starts_with("://") || after.starts_with("s://")) {
            i = start + 4;
            continue;
        }
        // extend to the next whitespace (Unicode-aware)
        let end = line[start..]
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map(|(p, _)| start + p)
            .unwrap_or(line.len());
        let raw = &line[start..end];
        let trimmed = raw.trim_end_matches(is_trailing_punct);
        if trimmed.len() > 7 {
            if let Some(url) = normalize_url(trimmed) {
                out.push(UrlSpan { start, end, url });
            }
        }
        i = end;
    }
    out
}

/// Extract the normalized http(s) addresses from one line (in order).
pub fn extract_addresses(line: &str) -> Vec<String> {
    extract_address_spans(line).into_iter().map(|s| s.url).collect()
}

/// Characters that terminate an address in running text (trimmed from the end).
fn is_trailing_punct(c: char) -> bool {
    matches!(
        c,
        '.' | ','
            | ';'
            | ':'
            | '!'
            | '?'
            | '"'
            | '\''
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '<'
            | '>'
            | '\\'
            | '|'
            // full-width / CJK punctuation
            | '，'
            | '。'
            | '；'
            | '：'
            | '！'
            | '？'
            | '、'
            | '（'
            | '）'
            | '【'
            | '】'
            | '《'
            | '》'
            | '〈'
            | '〉'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '…'
    )
}

/// Normalize the scheme to lowercase and validate the address.
fn normalize_url(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let (scheme, rest_start) = if lower.starts_with("https://") {
        ("https://", 8usize)
    } else if lower.starts_with("http://") {
        ("http://", 7usize)
    } else {
        return None;
    };
    let rest = &raw[rest_start..];
    if rest.is_empty() {
        return None;
    }
    let parsed = url::Url::parse(raw).ok()?;
    if parsed.host_str().is_none() {
        return None;
    }
    Some(format!("{scheme}{rest}"))
}

/// Read the input file and return every validated http(s) address in order.
fn read_addresses(input: &Path) -> Result<Vec<String>> {
    Ok(read_entries(input)?
        .into_iter()
        .map(|(url, _)| url)
        .collect())
}

/// Read the input file and return every validated http(s) address in order,
/// each paired with the optional trailing remark found on its line: after the
/// address, once whitespace is skipped, a `#` starts a remark that runs to the
/// end of the line (e.g. `https://a/x.json  #二月红接口`). The remark is taken
/// from the last address of the line only.
fn read_entries(input: &Path) -> Result<Vec<(String, Option<String>)>> {
    let raw = std::fs::read(input).with_context(|| format!("cannot read {}", input.display()))?;
    let text = String::from_utf8_lossy(&raw);
    let text = text.strip_prefix('\u{feff}').unwrap_or(text.as_ref());
    let mut entries = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        let spans = extract_address_spans(line);
        let last = spans.len().saturating_sub(1);
        for (idx, span) in spans.into_iter().enumerate() {
            let note = if idx == last { note_after(line, &span) } else { None };
            entries.push((span.url, note));
        }
    }
    if entries.is_empty() {
        bail!("no http(s) addresses found in {}", input.display());
    }
    Ok(entries)
}

/// The remark after an address: skip whitespace, expect a `#`, take the rest
/// of the line (trimmed). `None` when there is no remark.
fn note_after(line: &str, span: &UrlSpan) -> Option<String> {
    let rest = line[span.end..].trim_start();
    let rest = rest.strip_prefix('#')?;
    let note = rest.trim();
    if note.is_empty() {
        None
    } else {
        Some(note.to_string())
    }
}

// ---------------------------------------------------------------------------
// Probing
// ---------------------------------------------------------------------------

/// A single GET request. `want_body` reads the full body and reports the time
/// until it is fully received; otherwise only the response headers are waited
/// for. Returns `(status, elapsed, body)`.
fn request(
    client: &reqwest::blocking::Client,
    url: &str,
    want_body: bool,
) -> Result<(u16, Duration, Option<Vec<u8>>), String> {
    let start = Instant::now();
    let resp = client.get(url).send().map_err(short_error)?;
    let status = resp.status().as_u16();
    if want_body {
        let body = resp.bytes().map_err(short_error)?.to_vec();
        Ok((status, start.elapsed(), Some(body)))
    } else {
        Ok((status, start.elapsed(), None))
    }
}

fn short_error(e: reqwest::Error) -> String {
    if e.is_timeout() {
        return "timeout".to_string();
    }
    // The reqwest wrapper repeats the URL (already printed in the [NO] line);
    // walk the source chain and keep the deepest cause (e.g. "Connection
    // refused (os error 10061)", DNS failures, TLS errors...).
    let mut deepest: Option<String> = None;
    let mut cur: Option<&(dyn std::error::Error + 'static)> = e.source();
    while let Some(src) = cur {
        deepest = Some(src.to_string());
        cur = src.source();
    }
    let s = deepest.unwrap_or_else(|| e.to_string());
    truncate_ellipsis(&s, 100)
}

/// Truncate `s` to at most `max_bytes` (at a UTF-8 char boundary) and append
/// "…" when it was truncated. Never panics on multi-byte characters.
fn truncate_ellipsis(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let budget = max_bytes.saturating_sub(3); // room for "…" (3 bytes)
    let mut end = budget.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut t = s[..end].to_string();
    t.push('…');
    t
}

/// Classification of a received body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    Json,
    Text,
    Binary,
}

impl BodyKind {
    fn label(self) -> &'static str {
        match self {
            BodyKind::Json => "json",
            BodyKind::Text => "text",
            BodyKind::Binary => "binary",
        }
    }
}

fn body_kind(body: &[u8]) -> BodyKind {
    if serde_json::from_slice::<serde_json::Value>(body).is_ok() {
        return BodyKind::Json;
    }
    if is_plain_text(body) {
        BodyKind::Text
    } else {
        BodyKind::Binary
    }
}

/// Plain text = valid UTF-8, no NUL bytes, and at most 2% control characters
/// (excluding tab / LF / CR).
fn is_plain_text(body: &[u8]) -> bool {
    if std::str::from_utf8(body).is_err() {
        return false;
    }
    let mut ctrl = 0usize;
    for &b in body {
        if b == 0 {
            return false;
        }
        if b < 0x20 && b != b'\t' && b != b'\n' && b != b'\r' {
            ctrl += 1;
        }
    }
    ctrl * 100 <= body.len().max(1) * 2
}

// ---------------------------------------------------------------------------
// domain mode
// ---------------------------------------------------------------------------

struct DomainResult {
    url: String,
    times: Vec<Duration>,
    last_error: Option<String>,
}

impl DomainResult {
    fn avg(&self) -> Option<Duration> {
        if self.times.is_empty() {
            None
        } else {
            Some(self.times.iter().sum::<Duration>() / self.times.len() as u32)
        }
    }

    fn reachable(&self) -> bool {
        !self.times.is_empty()
    }
}

fn probe_domains(client: &reqwest::blocking::Client, urls: &[String]) -> Vec<DomainResult> {
    let mut results = Vec::with_capacity(urls.len());
    for url in urls {
        let mut times = Vec::new();
        let mut last_error = None;
        for _ in 0..DOMAIN_PROBES {
            match request(client, url, false) {
                Ok((_, elapsed, _)) => times.push(elapsed),
                Err(e) => last_error = Some(e),
            }
        }
        results.push(DomainResult {
            url: url.clone(),
            times,
            last_error,
        });
    }
    results
}

/// Render the sorted domain report (reachable by average response time first,
/// then unreachable addresses in input order).
fn render_domain(results: &[DomainResult]) -> Vec<String> {
    let mut reachable: Vec<&DomainResult> = results.iter().filter(|r| r.reachable()).collect();
    reachable.sort_by(|a, b| a.avg().unwrap().cmp(&b.avg().unwrap()));
    let unreachable: Vec<&DomainResult> = results.iter().filter(|r| !r.reachable()).collect();

    let w = results
        .iter()
        .map(|r| r.url.chars().count())
        .max()
        .unwrap_or(8)
        .max(8);

    let mut lines = vec!["=== domain response time (3 probes, average) ===".to_string()];
    for r in &reachable {
        let mut line = format!(
            "{:<w$}  avg={:<9}  probes={}",
            r.url,
            fmt_ms(r.avg().unwrap()),
            fmt_probes(&r.times)
        );
        if r.times.len() < DOMAIN_PROBES {
            line.push_str(&format!("  ({}/3 ok)", r.times.len()));
        }
        lines.push(line);
    }
    for r in &unreachable {
        let why = r.last_error.as_deref().unwrap_or("no response");
        lines.push(format!("{:<w$}  unreachable ({why})", r.url));
    }
    lines.push(format!(
        "--- {} of {} domains reachable ---",
        reachable.len(),
        results.len()
    ));
    lines
}

fn cmd_domain(client: &reqwest::blocking::Client, urls: &[String]) -> Result<()> {
    // Addresses already checked in this run are not probed again: they are
    // skipped on screen and excluded from the sorted report.
    let mut seen: HashSet<String> = HashSet::new();
    let mut unique: Vec<String> = Vec::with_capacity(urls.len());
    let mut n_dup = 0usize;
    for url in urls {
        if !seen.insert(dup_key(url)) {
            n_dup += 1;
            println!("[DUP] {url}  duplicate (already checked), skipped");
            continue;
        }
        unique.push(url.clone());
    }
    let results = probe_domains(client, &unique);
    for line in render_domain(&results) {
        println!("{line}");
    }
    if n_dup > 0 {
        println!("--- {n_dup} duplicate address(es) skipped ---");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// json / file modes
// ---------------------------------------------------------------------------

/// One analyzed json address (from `-t tv --sort`).
struct SortItem {
    url: String,
    /// number of `.sites` nodes
    count: usize,
    /// number of `.sites.name` containing "4k" (case-insensitive)
    name: usize,
    /// download speed of the `.spider` url in bytes/sec; None if unavailable
    speed: Option<f64>,
    /// human-readable note when `speed` is None (why the speed test failed)
    speed_note: Option<String>,
    /// trailing remark from the input line (`url  # remark`)
    note: Option<String>,
}

/// Parse a body as a tv JSON document: strict first, then retry after
/// stripping trailing commas (a common real-world blemish). Returns `None`
/// when the body still does not parse. A leading byte-order mark (UTF-8 /
/// UTF-16) is stripped first — raw.githubusercontent etc. often serve
/// BOM-prefixed JSON as `text/plain`.
fn parse_tv_json(body: &[u8]) -> Option<serde_json::Value> {
    let body = strip_bom(body);
    if let Ok(v) = serde_json::from_slice(body) {
        return Some(v);
    }
    let s = std::str::from_utf8(body).ok()?;
    let cleaned = strip_trailing_commas(s);
    serde_json::from_str(&cleaned).ok()
}

/// Strip a leading byte-order mark (UTF-8 `EF BB BF`, UTF-16 `FF FE` /
/// `FE FF`) from a body so BOM-prefixed JSON parses.
fn strip_bom(body: &[u8]) -> &[u8] {
    if body.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &body[3..]
    } else if body.starts_with(&[0xFF, 0xFE]) || body.starts_with(&[0xFE, 0xFF]) {
        &body[2..]
    } else {
        body
    }
}

/// Remove commas that are immediately followed (ignoring whitespace) by `}`
/// or `]` outside of string literals — i.e. trailing commas in arrays and
/// objects.
fn strip_trailing_commas(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            continue;
        }
        if c != ',' {
            out.push(c);
            continue;
        }
        // Peek the next non-whitespace char.
        let mut ahead = chars.clone();
        let mut next = ahead.next();
        while let Some(nc) = next {
            if nc.is_whitespace() {
                next = ahead.next();
            } else {
                break;
            }
        }
        if matches!(next, Some('}') | Some(']')) {
            // Trailing comma: drop it.
        } else {
            out.push(c);
        }
    }
    out
}

/// Valid tv data: `sites` is a non-empty array whose every element is an
/// object carrying a `name` key (`lives`, `parses` and `rules` are optional).
fn is_valid_tv(v: &serde_json::Value) -> bool {
    match v.get("sites").and_then(|s| s.as_array()) {
        Some(arr) if !arr.is_empty() => {
            arr.iter().all(|site| site.is_object() && site.get("name").is_some())
        }
        _ => false,
    }
}

/// Count `.sites` nodes in the parsed body.
fn count_sites(v: &serde_json::Value) -> usize {
    v.get("sites")
        .and_then(|s| s.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
}

/// Count `.sites.name` entries containing `word` (case-insensitive).
fn count_name_containing(v: &serde_json::Value, word: &str) -> usize {
    let needle = word.to_ascii_lowercase();
    v.get("sites")
        .and_then(|s| s.as_array())
        .map(|a| {
            a.iter()
                .filter(|site| {
                    site.get("name")
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_lowercase().contains(&needle))
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

/// Extract the download url from `.spider`: a string of the form
/// `url;md5;hash` yields the part before the first `;`; an array yields its
/// first string element.
fn spider_url(v: &serde_json::Value) -> Option<String> {
    let sp = v.get("spider")?;
    if let Some(s) = sp.as_str() {
        let first = s.split(';').next().unwrap_or(s).trim();
        if first.is_empty() {
            None
        } else {
            Some(first.to_string())
        }
    } else if let Some(arr) = sp.as_array() {
        arr.iter()
            .find_map(|e| e.as_str().map(str::trim).filter(|s| !s.is_empty()).map(|s| s.to_string()))
    } else {
        None
    }
}

/// Download the `.spider` url and measure the speed (bytes / elapsed).
fn probe_speed(
    client: &reqwest::blocking::Client,
    v: &serde_json::Value,
) -> (Option<f64>, Option<String>) {
    match spider_url(v) {
        Some(u) => match request(client, &u, true) {
            Ok((_, elapsed, Some(body))) => {
                let secs = elapsed.as_secs_f64();
                if secs > 0.0 {
                    (Some(body.len() as f64 / secs), None)
                } else {
                    (None, Some("no elapsed time".to_string()))
                }
            }
            Ok(_) => (None, Some("empty body".to_string())),
            Err(e) => (None, Some(truncate_ellipsis(&e, 40))),
        },
        None => (None, Some("no spider url".to_string())),
    }
}

fn fmt_speed(bps: f64) -> String {
    if bps >= 1_000_000.0 {
        format!("{:.1}MB/s", bps / 1_000_000.0)
    } else if bps >= 1_000.0 {
        format!("{:.0}KB/s", bps / 1_000.0)
    } else {
        format!("{:.0}B/s", bps)
    }
}

fn fmt_speed_line(it: &SortItem) -> String {
    match it.speed {
        Some(bps) => fmt_speed(bps),
        None => it.speed_note.clone().unwrap_or_else(|| "n/a".to_string()),
    }
}

fn cmp_speed(a: Option<f64>, b: Option<f64>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

/// Default parallelism for `-t tv|tvs`: the number of CPU cores.
fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .max(1)
}

/// Run `probe` over every unique address with at most `threads` concurrent
/// workers. Each probe prints its own result line as soon as it finishes;
/// the collected values come back in input order (used for the sorted
/// report, whose order is independent of completion order).
fn probe_parallel<T: Send>(
    unique: &[(String, Option<String>)],
    threads: usize,
    probe: impl Fn(&str, Option<&str>) -> T + Send + Sync,
) -> Vec<T> {
    let threads = threads.max(1).min(unique.len().max(1));
    let next = std::sync::Arc::new(std::sync::Mutex::new(0usize));
    let (tx, rx) = std::sync::mpsc::channel::<(usize, T)>();
    std::thread::scope(|scope| {
        let probe = &probe;
        for _ in 0..threads {
            let tx = tx.clone();
            let next = next.clone();
            scope.spawn(move || loop {
                let idx = {
                    let mut n = next.lock().unwrap();
                    let i = *n;
                    *n += 1;
                    i
                };
                if idx >= unique.len() {
                    break;
                }
                let (url, note) = &unique[idx];
                let r = probe(url, note.as_deref());
                let _ = tx.send((idx, r));
            });
        }
    });
    drop(tx);
    let mut by_idx: Vec<Option<T>> = (0..unique.len()).map(|_| None).collect();
    for (idx, r) in rx.iter() {
        by_idx[idx] = Some(r);
    }
    by_idx.into_iter().map(|o| o.unwrap()).collect()
}

/// `-t tv --sort KEY ...`: probe each address (in parallel, `threads`
/// concurrent, one result line per address as soon as it finishes), parse the
/// body as JSON, compute the count / name / speed statistics, sort by the
/// given keys (in order; default `count, name`), then print each url followed
/// by a `#` test-info line — on screen and in `-o` alike. Failed addresses
/// are reported as `[NO]`/`[FMT]` and are not part of the sorted output.
fn cmd_json_sorted(
    client: &reqwest::blocking::Client,
    entries: &[(String, Option<String>)],
    output: Option<&Path>,
    keys: &[SortKey],
    threads: usize,
    word: &str,
) -> Result<()> {
    // Deduplicate first (parallel probing would otherwise race the seen set).
    let mut seen: HashSet<String> = HashSet::new();
    let mut unique: Vec<(String, Option<String>)> = Vec::with_capacity(entries.len());
    let mut n_dup = 0usize;
    for (url, note) in entries {
        if !seen.insert(dup_key(url)) {
            n_dup += 1;
            println!("[DUP] {url}  duplicate (already checked), skipped");
            continue;
        }
        unique.push((url.clone(), note.clone()));
    }
    let w = unique.iter().map(|(u, _)| u.chars().count()).max().unwrap_or(8).max(8);

    enum ProbeOut {
        Ok(SortItem),
        No,
        Fmt,
    }
    let results = probe_parallel(&unique, threads, |url, note| {
        match request(client, url, true) {
            Ok((status, elapsed, Some(body))) => match parse_tv_json(&body) {
                Some(v) if is_valid_tv(&v) => {
                    let (speed, speed_note) = probe_speed(client, &v);
                    let note_part = note.map(|n| format!("  note={n}")).unwrap_or_default();
                    println!(
                        "[OK]  {:<w$}  status={status}  time={:<9}  format=json{note_part}",
                        url,
                        fmt_ms(elapsed)
                    );
                    ProbeOut::Ok(SortItem {
                        url: url.to_string(),
                        count: count_sites(&v),
                        name: count_name_containing(&v, word),
                        speed,
                        speed_note,
                        note: note.map(|n| n.to_string()),
                    })
                }
                Some(_) => {
                    let note_part = note.map(|n| format!("  note={n}")).unwrap_or_default();
                    println!(
                        "[FMT] {:<w$}  status={status}  time={:<9}  body lacks tv nodes (sites with name){note_part}",
                        url,
                        fmt_ms(elapsed)
                    );
                    ProbeOut::Fmt
                }
                None => {
                    let note_part = note.map(|n| format!("  note={n}")).unwrap_or_default();
                    println!(
                        "[FMT] {:<w$}  status={status}  time={:<9}  body is not valid JSON{note_part}",
                        url,
                        fmt_ms(elapsed)
                    );
                    ProbeOut::Fmt
                }
            },
            Ok((status, _, None)) => {
                let note_part = note.map(|n| format!("  note={n}")).unwrap_or_default();
                println!("[NO]  {:<w$}  status={status}  unexpected empty body{note_part}", url);
                ProbeOut::No
            }
            Err(e) => {
                let note_part = note.map(|n| format!("  note={n}")).unwrap_or_default();
                println!("[NO]  {:<w$}  error={e}{note_part}", url);
                ProbeOut::No
            }
        }
    });
    let mut n_ok = 0usize;
    let mut n_no = 0usize;
    let mut n_mismatch = 0usize;
    let mut items: Vec<SortItem> = Vec::new();
    for r in results {
        match r {
            ProbeOut::Ok(it) => {
                n_ok += 1;
                items.push(it);
            }
            ProbeOut::No => n_no += 1,
            ProbeOut::Fmt => n_mismatch += 1,
        }
    }

    let mut sort_keys = keys.to_vec();
    if sort_keys.is_empty() {
        sort_keys.push(SortKey::Count);
        sort_keys.push(SortKey::Name);
    }
    items.sort_by(|a, b| {
        for k in &sort_keys {
            let ord = match k {
                SortKey::Count => b.count.cmp(&a.count),
                SortKey::Name => b.name.cmp(&a.name),
                SortKey::Speed => cmp_speed(a.speed, b.speed),
            };
            if ord != std::cmp::Ordering::Equal {
                return ord;
            }
        }
        std::cmp::Ordering::Equal
    });

    for it in &items {
        println!("{}", it.url);
        let note_part = it.note.as_ref().map(|n| format!("note={n}  ")).unwrap_or_default();
        println!("# {note_part}count={} {word}={} speed={}", it.count, it.name, fmt_speed_line(it));
    }
    let dup_part = if n_dup > 0 {
        format!(", {n_dup} duplicate")
    } else {
        String::new()
    };
    println!(
        "--- {} of {} URLs passed ({} unreachable, {} format mismatch{dup_part}) ---",
        n_ok,
        entries.len(),
        n_no,
        n_mismatch
    );

    if let Some(out) = output {
        let mut s = String::new();
        for it in &items {
            s.push_str(&it.url);
            s.push('\n');
            let note_part = it.note.as_ref().map(|n| format!("note={n}  ")).unwrap_or_default();
            s.push_str(&format!(
                "# {note_part}count={} {word}={} speed={}\n",
                it.count,
                it.name,
                fmt_speed_line(it)
            ));
        }
        std::fs::write(out, s).with_context(|| format!("cannot write {}", out.display()))?;
        println!("wrote {} URL(s) to {}", items.len(), out.display());
    }
    Ok(())
}

/// One aggregator (`.urls`) address from `-t tvs`.
struct TvsMultiItem {
    url: String,
    /// number of `.urls` nodes
    count: usize,
    /// trailing remark from the input line (`url  # remark`)
    note: Option<String>,
}

/// `-t tvs`: like tv, but accepts both tv JSON (`.sites` -> "单仓" group) and
/// aggregator JSON (`.urls` -> "多仓" group). Addresses are probed in
/// parallel (`threads` concurrent), one result line printed as soon as each
/// probe finishes; the two groups are then printed separately (single first,
/// then multi), each with its own `#` test-info line, on screen and in `-o`
/// alike. The single group uses the tv `--sort KEY ...` rules (default
/// `count, name`); the multi group is sorted by url count descending only.
fn cmd_tvs(
    client: &reqwest::blocking::Client,
    entries: &[(String, Option<String>)],
    output: Option<&Path>,
    keys: &[SortKey],
    threads: usize,
    word: &str,
) -> Result<()> {
    // Deduplicate first.
    let mut seen: HashSet<String> = HashSet::new();
    let mut unique: Vec<(String, Option<String>)> = Vec::with_capacity(entries.len());
    let mut n_dup = 0usize;
    for (url, note) in entries {
        if !seen.insert(dup_key(url)) {
            n_dup += 1;
            println!("[DUP] {url}  duplicate (already checked), skipped");
            continue;
        }
        unique.push((url.clone(), note.clone()));
    }
    let w = unique.iter().map(|(u, _)| u.chars().count()).max().unwrap_or(8).max(8);

    enum ProbeOut {
        Single(SortItem),
        Multi(TvsMultiItem),
        No,
        Fmt,
    }
    let results = probe_parallel(&unique, threads, |url, note| {
        match request(client, url, true) {
            Ok((status, elapsed, Some(body))) => match parse_tv_json(&body) {
                Some(v) if is_valid_tv(&v) => {
                    let (speed, speed_note) = probe_speed(client, &v);
                    let note_part = note.map(|n| format!("  note={n}")).unwrap_or_default();
                    println!(
                        "[OK]  {:<w$}  status={status}  time={:<9}  format=json(tv){note_part}",
                        url,
                        fmt_ms(elapsed)
                    );
                    ProbeOut::Single(SortItem {
                        url: url.to_string(),
                        count: count_sites(&v),
                        name: count_name_containing(&v, word),
                        speed,
                        speed_note,
                        note: note.map(|n| n.to_string()),
                    })
                }
                Some(v) if v.get("urls").and_then(|u| u.as_array()).is_some() => {
                    let count = v.get("urls").and_then(|u| u.as_array()).map(|a| a.len()).unwrap_or(0);
                    let note_part = note.map(|n| format!("  note={n}")).unwrap_or_default();
                    println!(
                        "[OK]  {:<w$}  status={status}  time={:<9}  format=json(urls){note_part}",
                        url,
                        fmt_ms(elapsed)
                    );
                    ProbeOut::Multi(TvsMultiItem {
                        url: url.to_string(),
                        count,
                        note: note.map(|n| n.to_string()),
                    })
                }
                Some(_) => {
                    let note_part = note.map(|n| format!("  note={n}")).unwrap_or_default();
                    println!(
                        "[FMT] {:<w$}  status={status}  time={:<9}  body lacks tv nodes (sites with name) or urls{note_part}",
                        url,
                        fmt_ms(elapsed)
                    );
                    ProbeOut::Fmt
                }
                None => {
                    let note_part = note.map(|n| format!("  note={n}")).unwrap_or_default();
                    println!(
                        "[FMT] {:<w$}  status={status}  time={:<9}  body is not valid JSON{note_part}",
                        url,
                        fmt_ms(elapsed)
                    );
                    ProbeOut::Fmt
                }
            },
            Ok((status, _, None)) => {
                let note_part = note.map(|n| format!("  note={n}")).unwrap_or_default();
                println!("[NO]  {:<w$}  status={status}  unexpected empty body{note_part}", url);
                ProbeOut::No
            }
            Err(e) => {
                let note_part = note.map(|n| format!("  note={n}")).unwrap_or_default();
                println!("[NO]  {:<w$}  error={e}{note_part}", url);
                ProbeOut::No
            }
        }
    });
    let mut n_ok = 0usize;
    let mut n_no = 0usize;
    let mut n_mismatch = 0usize;
    let mut singles: Vec<SortItem> = Vec::new();
    let mut multis: Vec<TvsMultiItem> = Vec::new();
    for r in results {
        match r {
            ProbeOut::Single(it) => {
                n_ok += 1;
                singles.push(it);
            }
            ProbeOut::Multi(it) => {
                n_ok += 1;
                multis.push(it);
            }
            ProbeOut::No => n_no += 1,
            ProbeOut::Fmt => n_mismatch += 1,
        }
    }

    // Single group: tv sort rules (default count, name).
    let mut sort_keys = keys.to_vec();
    if sort_keys.is_empty() {
        sort_keys.push(SortKey::Count);
        sort_keys.push(SortKey::Name);
    }
    singles.sort_by(|a, b| {
        for k in &sort_keys {
            let ord = match k {
                SortKey::Count => b.count.cmp(&a.count),
                SortKey::Name => b.name.cmp(&a.name),
                SortKey::Speed => cmp_speed(a.speed, b.speed),
            };
            if ord != std::cmp::Ordering::Equal {
                return ord;
            }
        }
        std::cmp::Ordering::Equal
    });
    // Multi group: url count descending only.
    multis.sort_by(|a, b| b.count.cmp(&a.count));

    let mut file = String::new();
    if !singles.is_empty() {
        let title = format!("--- 单仓 ({}) ---", singles.len());
        println!("{title}");
        file.push_str(&title);
        file.push('\n');
        for it in &singles {
            println!("{}", it.url);
            let note_part = it.note.as_ref().map(|n| format!("note={n}  ")).unwrap_or_default();
            println!("# {note_part}count={} {word}={} speed={}", it.count, it.name, fmt_speed_line(it));
            file.push_str(&it.url);
            file.push('\n');
            let note_part = it.note.as_ref().map(|n| format!("note={n}  ")).unwrap_or_default();
            file.push_str(&format!(
                "# {note_part}count={} {word}={} speed={}\n",
                it.count,
                it.name,
                fmt_speed_line(it)
            ));
        }
    }
    if !multis.is_empty() {
        let title = format!("--- 多仓 ({}) ---", multis.len());
        println!("{title}");
        file.push_str(&title);
        file.push('\n');
        for it in &multis {
            println!("{}", it.url);
            let note_part = it.note.as_ref().map(|n| format!("note={n}  ")).unwrap_or_default();
            println!("# {note_part}count={}", it.count);
            file.push_str(&it.url);
            file.push('\n');
            let note_part = it.note.as_ref().map(|n| format!("note={n}  ")).unwrap_or_default();
            file.push_str(&format!("# {note_part}count={}\n", it.count));
        }
    }

    let dup_part = if n_dup > 0 {
        format!(", {n_dup} duplicate")
    } else {
        String::new()
    };
    println!(
        "--- {} of {} URLs passed ({} unreachable, {} format mismatch{dup_part}) ---",
        n_ok,
        entries.len(),
        n_no,
        n_mismatch
    );

    if let Some(out) = output {
        std::fs::write(out, file).with_context(|| format!("cannot write {}", out.display()))?;
        println!(
            "wrote {} URL(s) to {}",
            singles.len() + multis.len(),
            out.display()
        );
    }
    Ok(())
}

/// `-t file` (and json before --sort): report each probe as it finishes and
/// write the passed (reachable + format-matched) urls to `-o`, one per line.
fn cmd_file_probe(
    client: &reqwest::blocking::Client,
    urls: &[String],
    output: Option<&Path>,
) -> Result<()> {
    let w = urls.iter().map(|u| u.chars().count()).max().unwrap_or(8).max(8);
    let mut passed: Vec<String> = Vec::new();
    let mut n_ok = 0usize;
    let mut n_mismatch = 0usize;
    let mut n_no = 0usize;
    let mut n_dup = 0usize;
    let mut seen: HashSet<String> = HashSet::new();

    for url in urls {
        if !seen.insert(dup_key(url)) {
            n_dup += 1;
            println!("[DUP] {:<w$}  duplicate (already checked), skipped", url);
            continue;
        }
        match request(client, url, true) {
            Ok((status, elapsed, Some(body))) => {
                let got = body_kind(&body);
                if got == BodyKind::Text {
                    n_ok += 1;
                    passed.push(url.clone());
                    println!(
                        "[OK]  {:<w$}  status={status}  time={:<9}  format={}",
                        url,
                        fmt_ms(elapsed),
                        got.label()
                    );
                } else {
                    n_mismatch += 1;
                    println!(
                        "[FMT] {:<w$}  status={status}  time={:<9}  got={} (expected {})",
                        url,
                        fmt_ms(elapsed),
                        got.label(),
                        "file"
                    );
                }
            }
            Ok((status, _, None)) => {
                n_no += 1;
                println!("[NO]  {:<w$}  status={status}  unexpected empty body", url);
            }
            Err(e) => {
                n_no += 1;
                println!("[NO]  {:<w$}  error={e}", url);
            }
        }
    }

    let dup_part = if n_dup > 0 {
        format!(", {n_dup} duplicate")
    } else {
        String::new()
    };
    println!(
        "--- {} of {} URLs passed ({} unreachable, {} format mismatch{dup_part}) ---",
        n_ok,
        urls.len(),
        n_no,
        n_mismatch
    );

    if let Some(out) = output {
        let mut s = String::new();
        for u in &passed {
            s.push_str(u);
            s.push('\n');
        }
        std::fs::write(out, s).with_context(|| format!("cannot write {}", out.display()))?;
        println!("wrote {} URL(s) to {}", passed.len(), out.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// test mode
// ---------------------------------------------------------------------------

/// Rebuild a source line without the unreachable addresses (`alive[i]` says
/// whether `spans[i]` is reachable). Reachable addresses keep their original
/// text; unreachable ones (including the trailing punctuation the extractor
/// kept) are dropped.
fn rebuild_line(line: &str, spans: &[UrlSpan], alive: &[bool]) -> String {
    let mut out = String::with_capacity(line.len());
    let mut pos = 0usize;
    for (span, ok) in spans.iter().zip(alive) {
        out.push_str(&line[pos..span.start]);
        if *ok {
            out.push_str(&line[span.start..span.end]);
        }
        pos = span.end;
    }
    out.push_str(&line[pos..]);
    out
}

/// `sys net -t test -i FILE [-o FILE] [-x SUFFIX ...]` — probe every address
/// for reachability; every line is reported on screen with a `[line/total]`
/// progress counter; unreachable addresses are removed from the source lines;
/// addresses whose file-name suffix is listed in `excludes` are not probed and
/// are removed directly; the cleaned result is written to `-o` (lines kept
/// verbatim, empty / `#` comment / non-URL lines are always kept).
fn cmd_test(
    client: &reqwest::blocking::Client,
    input: &Path,
    output: Option<&Path>,
    excludes: &[String],
) -> Result<()> {
    let raw = std::fs::read(input).with_context(|| format!("cannot read {}", input.display()))?;
    let text = String::from_utf8_lossy(&raw);
    let text = text.strip_prefix('\u{feff}').unwrap_or(text.as_ref());
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();

    let mut kept: Vec<String> = Vec::new();
    let mut n_addr = 0usize;
    let mut n_unreach = 0usize;
    let mut n_excluded = 0usize;
    let mut n_dup = 0usize;
    let mut n_deleted_lines = 0usize;
    let mut seen: HashSet<String> = HashSet::new();

    for (idx, line) in lines.iter().enumerate() {
        let cur = idx + 1;
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            kept.push(line.to_string());
            let tag = if trimmed.is_empty() { "(empty)" } else { "(comment)" };
            println!("[{cur}/{total}]  [skip] {tag}");
            continue;
        }
        let spans = extract_address_spans(line);
        if spans.is_empty() {
            kept.push(line.to_string());
            println!("[{cur}/{total}]  [skip] (no address)");
            continue;
        }

        let mut parts: Vec<String> = Vec::new();
        let mut alive = Vec::with_capacity(spans.len());
        for span in &spans {
            n_addr += 1;
            match url_ext(&span.url).filter(|ext| excludes.iter().any(|e| e == ext)) {
                Some(ext) => {
                    // Not probed: the suffix is excluded, remove directly.
                    alive.push(false);
                    n_excluded += 1;
                    parts.push(format!("[X] {}  excluded (-x {ext})", span.url));
                }
                None => {
                    let key = dup_key(&span.url);
                    if !seen.insert(key) {
                        // Already checked in this run: do not probe again,
                        // remove directly.
                        alive.push(false);
                        n_dup += 1;
                        parts.push(format!("[DUP] {}  duplicate (already checked)", span.url));
                    } else {
                        match request(client, &span.url, false) {
                            Ok((_, elapsed, _)) => {
                                alive.push(true);
                                parts.push(format!("[OK] {}  time={}", span.url, fmt_ms(elapsed)));
                            }
                            Err(e) => {
                                alive.push(false);
                                n_unreach += 1;
                                parts.push(format!("[NO] {}  error={e}", span.url));
                            }
                        }
                    }
                }
            }
        }

        let action = if alive.iter().all(|a| *a) {
            kept.push(line.to_string());
            "kept"
        } else if alive.iter().any(|a| *a) {
            kept.push(rebuild_line(line, &spans, &alive));
            "kept (bad address removed)"
        } else {
            n_deleted_lines += 1;
            "line deleted"
        };
        println!("[{cur}/{total}]  {}  -> {action}", parts.join(" | "));
    }

    let n_reachable = n_addr - n_unreach - n_excluded - n_dup;
    let extra = {
        let mut v = Vec::new();
        if n_excluded > 0 {
            v.push(format!("{n_excluded} excluded"));
        }
        if n_dup > 0 {
            v.push(format!("{n_dup} duplicate"));
        }
        if v.is_empty() {
            String::new()
        } else {
            format!(", {}", v.join(", "))
        }
    };
    println!(
        "--- checked {n_addr} address(es) in {total} line(s): {n_reachable} reachable, {n_unreach} unreachable{extra}, {} line(s) kept, {n_deleted_lines} line(s) deleted ---",
        kept.len()
    );
    if let Some(out) = output {
        let mut s = String::new();
        for l in &kept {
            s.push_str(l);
            s.push('\n');
        }
        std::fs::write(out, s).with_context(|| format!("cannot write {}", out.display()))?;
        println!("wrote {} line(s) to {}", kept.len(), out.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// `sys net -i FILE [-t domain|tv|tvs|file|test] [-o FILE] [--timeout SECONDS] [-x SUFFIX ...] [--sort KEY ...]` or `sys net -t info -n DOMAIN ...`
pub fn cmd_net(
    input: &Path,
    kind: NetType,
    output: Option<&Path>,
    timeout_secs: f64,
    exclude: &[String],
    sort: &[String],
) -> Result<()> {
    cmd_net_t(input, kind, output, timeout_secs, exclude, sort, None, "4k")
}

/// Like `cmd_net`, with an explicit parallelism hint for `-t tv|tvs`
/// (`None` = number of CPU cores) and the `--words` keyword used for the
/// `name` statistic (default `"4k"`).
pub fn cmd_net_t(
    input: &Path,
    kind: NetType,
    output: Option<&Path>,
    timeout_secs: f64,
    exclude: &[String],
    sort: &[String],
    threads: Option<usize>,
    word: &str,
) -> Result<()> {
    let threads = threads.unwrap_or_else(default_threads).max(1);
    if !(timeout_secs > 0.0) || !timeout_secs.is_finite() {
        bail!("--timeout must be a positive number of seconds");
    }
    // Normalize -x suffixes: leading dot optional, case-insensitive, empty
    // entries dropped.
    let excludes: Vec<String> = exclude
        .iter()
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .collect();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs_f64(timeout_secs))
        .build()
        .context("cannot build HTTP client")?;

    match kind {
        NetType::Domain => {
            if let Some(out) = output {
                eprintln!(
                    "warning: -o/--output applies to `-t tv|file|test` only; ignored ({})",
                    out.display()
                );
            }
            if !excludes.is_empty() {
                eprintln!("warning: -x/--exclude applies to `-t test` only; ignored");
            }
            if !sort.is_empty() {
                eprintln!("warning: --sort applies to `-t tv` only; ignored");
            }
            let urls = read_addresses(input)?;
            cmd_domain(&client, &urls)
        }
        NetType::Json | NetType::File | NetType::Tvs => {
            if !excludes.is_empty() {
                eprintln!("warning: -x/--exclude applies to `-t test` only; ignored");
            }
            if kind == NetType::File && !sort.is_empty() {
                eprintln!("warning: --sort applies to `-t tv|tvs` only; ignored");
            }
            match kind {
                NetType::Json | NetType::Tvs => {
                    let entries = read_entries(input)?;
                    match kind {
                        NetType::Json => cmd_json_sorted(&client, &entries, output, &parse_sort_keys(sort)?, threads, word),
                        NetType::Tvs => cmd_tvs(&client, &entries, output, &parse_sort_keys(sort)?, threads, word),
                        _ => unreachable!(),
                    }
                }
                _ => {
                    let urls = read_addresses(input)?;
                    cmd_file_probe(&client, &urls, output)
                }
            }
        }
        NetType::Test => {
            if !sort.is_empty() {
                eprintln!("warning: --sort applies to `-t tv` only; ignored");
            }
            cmd_test(&client, input, output, &excludes)
        }
        NetType::Info => {
            bail!("`-t info` reads -n DOMAIN, not -i; use `sys net -t info -n example.com`")
        }
    }
}

/// `sys net -t info -n DOMAIN ...`: query RDAP registration info for each
/// domain; when a name has no registration data, fall back to its parent
/// domain (a.b.example.com → b.example.com → example.com).
pub fn cmd_net_info(names: &[String], timeout_secs: f64) -> Result<()> {
    if !(timeout_secs > 0.0) || !timeout_secs.is_finite() {
        bail!("--timeout must be a positive number of seconds");
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs_f64(timeout_secs))
        .user_agent(RDAP_USER_AGENT)
        .build()
        .context("cannot build HTTP client")?;
    let out = cmd_info_with_base(&client, names, RDAP_BASE)?;
    print!("{out}");
    Ok(())
}

/// RDAP bootstrap service (redirects to the registry for each TLD).
const RDAP_BASE: &str = "https://rdap.org/domain/";

/// Some registries (e.g. Verisign) reject non-browser user agents; use a
/// generic browser UA for RDAP lookups.
const RDAP_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

/// Registration info extracted from an RDAP domain object.
struct RdapInfo {
    registrar: Option<String>,
    registrant: Option<String>,
    org: Option<String>,
    email: Option<String>,
    address: Option<String>,
    country: Option<String>,
    created: Option<String>,
    expires: Option<String>,
    status: Vec<String>,
    nameservers: Vec<String>,
}

/// Extract the parent domain (leftmost label dropped): `a.b.example.com` →
/// `b.example.com` → `example.com` → `com` → None.
fn parent_domain(d: &str) -> Option<String> {
    let idx = d.find('.')?;
    let parent = d[idx + 1..].trim();
    if parent.is_empty() {
        None
    } else {
        Some(parent.to_string())
    }
}

/// Look up one domain via RDAP. `Ok(Some(info))` = registration data found;
/// `Ok(None)` = the registry answered but has no data for this name (parent
/// should be tried); `Err(e)` = the lookup itself failed.
/// Find the registrar-level RDAP url advertised by a registry response: the
/// `links` entry with `rel == "related"` and an `application/rdap+json` type
/// (ICANN profile — registries often hide the registrant, the registrar's own
/// RDAP server carries the full entity).
fn related_rdap_url(v: &serde_json::Value) -> Option<String> {
    v.get("links")?
        .as_array()?
        .iter()
        .find(|l| {
            l.get("rel").and_then(|r| r.as_str()) == Some("related")
                && l.get("type")
                    .and_then(|t| t.as_str())
                    .map(|t| t.contains("rdap+json"))
                    .unwrap_or(false)
        })
        .and_then(|l| l.get("href").and_then(|h| h.as_str()))
        .map(|s| s.to_string())
}

fn rdap_lookup(
    client: &reqwest::blocking::Client,
    base: &str,
    domain: &str,
) -> Result<Option<RdapInfo>> {
    let url = format!("{base}{domain}");
    match request(client, &url, true) {
        Ok((status, _, Some(body))) => {
            if status == 404 {
                return Ok(None);
            }
            match serde_json::from_slice::<serde_json::Value>(&body) {
                Ok(v) => {
                    let info = parse_rdap(&v);
                    // The registry response often hides the registrant; when
                    // no registrant entity is present, follow the advertised
                    // registrar-level RDAP link and prefer its (fuller) data.
                    if info.registrant.is_none() && info.org.is_none() {
                        if let Some(rel) = related_rdap_url(&v) {
                            if let Ok((s2, _, Some(b2))) = request(client, &rel, true) {
                                if s2 == 200 {
                                    if let Ok(v2) = serde_json::from_slice::<serde_json::Value>(&b2)
                                    {
                                        let info2 = parse_rdap(&v2);
                                        if info2.registrant.is_some() || info2.org.is_some() {
                                            return Ok(Some(info2));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(Some(info))
                }
                Err(e) => {
                    let preview = String::from_utf8_lossy(&body[..body.len().min(200)]);
                    Err(anyhow!(
                        "rdap response for {domain} is not valid JSON (status {status}): {preview} (parse error: {e})"
                    ))
                }
            }
        }
        Ok((status, _, None)) => Err(anyhow!("rdap response for {domain} has no body (status {status})")),
        Err(e) => Err(anyhow!("rdap lookup for {domain} failed: {e}")),
    }
}

/// Extract one vCard field (`fn` / `org` / `email` / `adr` ...) from the
/// entity whose `roles` contain `role` (e.g. "registrant", "registrar").
/// Returns the raw value (string, or the adr array) when present.
fn entity_field(v: &serde_json::Value, role: &str, field: &str) -> Option<serde_json::Value> {
    entity_row(v, role, field).and_then(|row| row.get(3).cloned())
}

/// Locate the vCard row for `field` inside the entity whose `roles` contain
/// `role`; returns the whole row array (field name, params object, type,
/// value) so callers can read both the value and its parameters.
fn entity_row(v: &serde_json::Value, role: &str, field: &str) -> Option<serde_json::Value> {
    v.get("entities")?
        .as_array()?
        .iter()
        .find(|e| {
            e.get("roles")
                .and_then(|r| r.as_array())
                .map(|rs| rs.iter().any(|x| x.as_str() == Some(role)))
                .unwrap_or(false)
        })?
        .get("vcardArray")
        .and_then(|va| va.as_array())
        .and_then(|va| va.get(1))
        .and_then(|arr| arr.as_array())
        .and_then(|arr| {
            arr.iter().find(|f| {
                f.as_array()
                    .and_then(|fa| fa.first())
                    .and_then(|x| x.as_str())
                    == Some(field)
            })
        })
        .cloned()
}

/// Format a vCard `adr` value (an array of [po box, extended, street,
/// locality, region, postal code, country]) into a readable one-liner.
fn format_address(adr: &serde_json::Value) -> Option<String> {
    let parts = adr.as_array()?;
    let comps: Vec<&str> = parts
        .iter()
        .filter_map(|p| p.as_str())
        .filter(|s| !s.trim().is_empty())
        .collect();
    if comps.is_empty() {
        None
    } else {
        Some(comps.join(", "))
    }
}

/// Extract the country from a vCard `adr` value: the 7th component (index 6)
/// per RFC 6350; falls back to the last non-empty component.
fn adr_country(adr: &serde_json::Value) -> Option<String> {
    let parts = adr.as_array()?;
    if let Some(c) = parts.get(6).and_then(|p| p.as_str()) {
        let c = c.trim();
        if !c.is_empty() {
            return Some(c.to_string());
        }
    }
    parts
        .iter()
        .rev()
        .find_map(|p| p.as_str().map(str::trim).filter(|s| !s.is_empty()).map(|s| s.to_string()))
}

/// Extract the useful fields from an RDAP domain object.
fn parse_rdap(v: &serde_json::Value) -> RdapInfo {
    let event_date = |action: &str| -> Option<String> {
        v.get("events")
            .and_then(|e| e.as_array())
            .and_then(|arr| {
                arr.iter().find(|e| {
                    e.get("eventAction")
                        .and_then(|a| a.as_str())
                        .map(|a| a == action)
                        .unwrap_or(false)
                })
            })
            .and_then(|e| e.get("eventDate"))
            .and_then(|d| d.as_str())
            .map(|s| s.to_string())
    };
    let text_field = |role: &str, field: &str| -> Option<String> {
        entity_field(v, role, field).and_then(|x| x.as_str().map(|s| s.to_string()))
    };
    // The adr vCard row is `["adr", {params}, "text", [value...]]`; the
    // country is often carried as the `cc` parameter (e.g. `{"cc":"CN"}`)
    // rather than in the value array.
    let adr_row = entity_row(v, "registrant", "adr");
    let adr_cc = adr_row
        .as_ref()
        .and_then(|row| row.get(1))
        .and_then(|p| p.get("cc"))
        .and_then(|c| c.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let adr_val = adr_row.as_ref().and_then(|row| row.get(3));
    let nameservers = v
        .get("nameservers")
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|ns| ns.get("ldhName").and_then(|n| n.as_str()))
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let status = v
        .get("status")
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    RdapInfo {
        registrar: text_field("registrar", "fn"),
        registrant: text_field("registrant", "fn"),
        org: text_field("registrant", "org"),
        email: text_field("registrant", "email"),
        address: adr_val.and_then(|a| format_address(a)),
        country: adr_cc.or_else(|| adr_val.and_then(|a| adr_country(a))),
        created: event_date("registration"),
        expires: event_date("expiration"),
        status,
        nameservers,
    }
}

/// Format one domain's registration info, noting the fallback source when the
/// requested name itself had no data (or its lookup failed).
fn format_info(queried: &str, resolved: &str, fallback: Option<&str>, info: &RdapInfo) -> String {
    let mut s = String::new();
    if let Some(reason) = fallback {
        s.push_str(&format!("{queried}: {reason}, using parent {resolved}\n"));
    } else {
        s.push_str(resolved);
        s.push('\n');
    }
    if let Some(r) = &info.registrar {
        s.push_str(&format!("  Registrar:   {r}\n"));
    }
    if let Some(r) = &info.registrant {
        s.push_str(&format!("  Registrant:  {r}\n"));
    }
    if let Some(o) = &info.org {
        s.push_str(&format!("  Org:         {o}\n"));
    }
    if let Some(e) = &info.email {
        s.push_str(&format!("  Email:       {e}\n"));
    }
    if let Some(a) = &info.address {
        s.push_str(&format!("  Address:     {a}\n"));
    }
    if let Some(c) = &info.country {
        s.push_str(&format!("  Country:     {c}\n"));
    }
    if let Some(c) = &info.created {
        s.push_str(&format!("  Created:     {c}\n"));
    }
    if let Some(e) = &info.expires {
        s.push_str(&format!("  Expires:     {e}\n"));
    }
    if !info.status.is_empty() {
        s.push_str(&format!("  Status:      {}\n", info.status.join(", ")));
    }
    if info.nameservers.is_empty() {
        s.push_str("  Nameservers: (none)\n");
    } else {
        s.push_str(&format!("  Nameservers: {}\n", info.nameservers.join(", ")));
    }
    s
}

/// `-t info`: for each name, walk down the parent chain until registration
/// data is found or the chain is exhausted. A 404 (no data) *and* a failed
/// lookup both fall back to the parent. Returns the screen text.
fn cmd_info_with_base(
    client: &reqwest::blocking::Client,
    names: &[String],
    base: &str,
) -> Result<String> {
    if names.is_empty() {
        bail!("`-t info` requires at least one `-n DOMAIN` (e.g. `sys net -t info -n example.com`)");
    }
    let mut out = String::new();
    for raw in names {
        let mut current = raw.trim().trim_end_matches('.').to_ascii_lowercase();
        if current.is_empty() {
            out.push_str("(empty domain name)\n");
            continue;
        }
        let mut tried: Vec<String> = Vec::new();
        let mut last_err: Option<String> = None;
        loop {
            tried.push(current.clone());
            match rdap_lookup(client, base, &current) {
                Ok(Some(info)) => {
                    let fallback = match last_err.take() {
                        Some(e) => Some(format!("lookup on {0} failed ({e})", tried[0])),
                        None if tried.len() > 1 => Some("no registration data".to_string()),
                        None => None,
                    };
                    out.push_str(&format_info(&tried[0], &current, fallback.as_deref(), &info));
                    break;
                }
                Ok(None) => {}
                Err(e) => last_err = Some(e.to_string()),
            }
            match parent_domain(&current) {
                Some(parent) => current = parent,
                None => {
                    match last_err {
                        Some(e) => out.push_str(&format!(
                            "{}: lookup failed (checked: {}), error: {e}\n",
                            tried[0],
                            tried.join(", ")
                        )),
                        None => out.push_str(&format!(
                            "{}: no registration info found (checked: {})\n",
                            tried[0],
                            tried.join(", ")
                        )),
                    }
                    break;
                }
            }
        }
    }
    Ok(out)
}

/// Sort keys for `-t json --sort KEY ...`, applied in the given order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {    /// Total number of `.sites` nodes, descending
    Count,
    /// Number of `.sites.name` entries containing "4k" (case-insensitive), descending
    Name,
    /// Download speed of the `.spider` url, ascending
    Speed,
}

/// Parse `--sort` values: count | name | speed.
fn parse_sort_keys(raw: &[String]) -> Result<Vec<SortKey>> {
    let mut keys = Vec::new();
    for r in raw {
        let k = match r.trim().to_ascii_lowercase().as_str() {
            "count" => SortKey::Count,
            "name" => SortKey::Name,
            "speed" => SortKey::Speed,
            other => bail!(
                "invalid --sort key {other:?} (expected count | name | speed)"
            ),
        };
        keys.push(k);
    }
    Ok(keys)
}

/// Lowercase file-name suffix of a URL's last path segment, if any.
/// `https://www.main.com/123.m3u` -> `Some("m3u")`; `/a.tar.gz` -> `Some("gz")`;
/// `/abc`, `/abc.` or a bare host have no suffix. The query string is ignored.
fn url_ext(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let last = parsed.path_segments()?.next_back()?;
    let (_, ext) = last.rsplit_once('.')?;
    if ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// Canonical key used to detect addresses that were already checked in this
/// run: the `url` crate normalizes the host to lowercase, drops default ports
/// and normalizes the path, so `HTTP://EXAMPLE.com/a` and
/// `http://example.com/a` count as the same address.
fn dup_key(url: &str) -> String {
    url::Url::parse(url)
        .map(|u| u.as_str().to_string())
        .unwrap_or_else(|_| url.to_string())
}

fn fmt_ms(d: Duration) -> String {
    format!("{:.1}ms", d.as_secs_f64() * 1000.0)
}

fn fmt_probes(times: &[Duration]) -> String {
    times
        .iter()
        .map(|t| fmt_ms(*t))
        .collect::<Vec<_>>()
        .join("/")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    const JSON_BODY: &[u8] = br#"{"ok":true,"n":42}"#;
    /// A structurally valid tv document (sites non-empty with name, plus
    /// lives / parses / rules).
    const TV_BODY: &[u8] = br#"{"sites":[{"name":"a"}],"lives":[{"name":"l1"}],"parses":[{"name":"p1"}],"rules":[{"name":"r1"}]}"#;
    const TEXT_BODY: &[u8] = b"hello plain text file\nsecond line\n";
    const BIN_BODY: &[u8] = &[0x89, b'P', b'N', b'G', 0, 1, 2, 3, 0xFF, 0xFE, 0xFD];

    fn test_client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("build test client")
    }

    /// Serve `body` with `content_type` on an ephemeral 127.0.0.1 port and
    /// return its URL. Each connection reads the request head first (like a
    /// real server), then sleeps `delay` (used to make one server measurably
    /// slower), then responds and closes cleanly: FIN first via
    /// shutdown(Write), then the socket is drained until the client closes, so
    /// Windows never sends a RST that would make reqwest fail the exchange.
    fn spawn_server(body: &'static [u8], content_type: &'static str, delay: Duration) -> String {
        spawn_server_owned(body.to_vec(), content_type, delay)
    }

    /// Like `spawn_server` but with an owned body (for dynamic JSON fixtures).
    fn spawn_server_owned(body: Vec<u8>, content_type: &'static str, delay: Duration) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let url = format!("http://127.0.0.1:{}/test", addr.port());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                // 1) read the request head (GET has no body) so nothing is
                //    left unread in the receive buffer when we close
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                let mut buf = [0u8; 8192];
                let mut got = 0usize;
                while got < buf.len() {
                    match stream.read(&mut buf[got..]) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            got += n;
                            if buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
                // 2) respond
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                // 3) FIN first, then drain until the client closes so the
                //    final drop never triggers a Windows RST
                let _ = stream.shutdown(std::net::Shutdown::Write);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut rest = [0u8; 4096];
                loop {
                    match stream.read(&mut rest) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => continue,
                    }
                }
            }
        });
        url
    }

    fn temp_file(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("sys-net-{tag}-{}.txt", std::process::id()))
    }

    // --- parsing ---

    #[test]
    fn extracts_multiple_addresses_per_line() {
        let urls = extract_addresses("see http://a.com/x, then https://b.com/y! and more");
        assert_eq!(urls, vec!["http://a.com/x", "https://b.com/y"]);
    }

    #[test]
    fn skips_comments_and_empty_lines() {
        let path = temp_file("comments");
        std::fs::write(
            &path,
            "# comment\n\n  # indented comment\nhttp://a.com\nhttp://b.com and https://c.com\n",
        )
        .unwrap();
        let urls = read_addresses(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(urls, vec!["http://a.com", "http://b.com", "https://c.com"]);
    }

    #[test]
    fn trims_trailing_punctuation() {
        assert_eq!(extract_addresses("(http://a.com/path)."), vec!["http://a.com/path"]);
        assert_eq!(
            extract_addresses("http://a.com/x?a=1&b=2;"),
            vec!["http://a.com/x?a=1&b=2"]
        );
    }

    #[test]
    fn keeps_query_and_fragment() {
        let urls = extract_addresses("go http://a.com/api?x=1&y=two#frag now");
        assert_eq!(urls, vec!["http://a.com/api?x=1&y=two#frag"]);
    }

    #[test]
    fn ignores_http_like_words() {
        assert!(extract_addresses("the http client is nice").is_empty());
        assert!(extract_addresses("no scheme: https-not-a-url").is_empty());
    }

    #[test]
    fn normalize_url_validates() {
        assert_eq!(
            normalize_url("HTTP://Example.COM/x").unwrap(),
            "http://Example.COM/x"
        );
        assert_eq!(normalize_url("https://a.com").unwrap(), "https://a.com");
        assert!(normalize_url("http://").is_none());
        assert!(normalize_url("ftp://a.com").is_none());
    }

    #[test]
    fn body_kind_detection() {
        assert_eq!(body_kind(JSON_BODY), BodyKind::Json);
        assert_eq!(body_kind(TEXT_BODY), BodyKind::Text);
        assert_eq!(body_kind(BIN_BODY), BodyKind::Binary);
        assert_eq!(body_kind(b"not json at all"), BodyKind::Text);
    }

    #[test]
    fn truncate_ellipsis_handles_multibyte_chars() {
        // Short strings pass through untouched.
        assert_eq!(truncate_ellipsis("abc", 100), "abc");
        // 3-byte CJK chars: a byte budget can land mid-char; the result must
        // stay within budget, end with "…" and hold only whole characters.
        let long = "由于目标计算机积极拒绝，无法连接。".repeat(8); // > 100 bytes, all 3-byte chars
        let t = truncate_ellipsis(&long, 100);
        assert!(t.len() <= 100, "len {} <= 100", t.len());
        assert!(t.ends_with('…'));
        let prefix = &t[..t.len() - 3];
        assert_eq!(prefix.len() % 3, 0, "CJK prefix keeps whole chars: {prefix:?}");
        // Mixed ASCII + CJK crossing the budget: no panic, ends on a boundary.
        let mixed = format!("{} 中文说明中文说明中文", "x".repeat(80)); // > 100 bytes
        let m = truncate_ellipsis(&mixed, 100);
        assert!(m.len() <= 100, "len {} <= 100", m.len());
        assert!(m.ends_with('…'));
        let mprefix = &m[..m.len() - 3];
        assert!(mprefix.is_char_boundary(mprefix.len()));
    }

    // --- probing ---

    #[test]
    fn json_probe_reports_ok() {
        let url = spawn_server(JSON_BODY, "application/json", Duration::ZERO);
        let (status, _, body) = request(&test_client(), &url, true).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body_kind(&body.expect("body expected")), BodyKind::Json);
    }

    #[test]
    fn file_probe_reports_ok() {
        let url = spawn_server(TEXT_BODY, "text/plain", Duration::ZERO);
        let (status, _, body) = request(&test_client(), &url, true).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body_kind(&body.expect("body expected")), BodyKind::Text);
    }

    #[test]
    fn unreachable_probe_fails() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l); // port is now closed
        let url = format!("http://127.0.0.1:{port}/x");
        assert!(request(&test_client(), &url, true).is_err());
    }

    #[test]
    fn domain_mode_probes_three_times_and_averages() {
        let fast = spawn_server(TEXT_BODY, "text/plain", Duration::ZERO);
        let slow = spawn_server(TEXT_BODY, "text/plain", Duration::from_millis(120));
        let client = test_client();
        let results = probe_domains(&client, &[slow, fast]);
        assert_eq!(results.len(), 2);
        for r in &results {
            assert!(r.reachable());
            assert_eq!(r.times.len(), 3, "three probes for {}", r.url);
            assert!(r.last_error.is_none());
        }
        assert!(results[0].avg().unwrap() > results[1].avg().unwrap());
    }

    #[test]
    fn render_domain_sorts_reachable_first() {
        let r1 = DomainResult {
            url: "http://fast.com".into(),
            times: vec![Duration::from_millis(10); 3],
            last_error: None,
        };
        let r2 = DomainResult {
            url: "http://slow.com".into(),
            times: vec![Duration::from_millis(800); 3],
            last_error: None,
        };
        let dead = DomainResult {
            url: "http://dead.com".into(),
            times: vec![],
            last_error: Some("timeout".into()),
        };
        let lines = render_domain(&[r2, dead, r1]);
        let pos_fast = lines.iter().position(|l| l.contains("http://fast.com")).unwrap();
        let pos_slow = lines.iter().position(|l| l.contains("http://slow.com")).unwrap();
        let pos_dead = lines.iter().position(|l| l.contains("http://dead.com")).unwrap();
        assert!(pos_fast < pos_slow, "fast before slow: {lines:?}");
        assert!(pos_slow < pos_dead, "reachable before unreachable: {lines:?}");
        assert!(lines.last().unwrap().contains("2 of 3 domains reachable"));
    }

    // --- end-to-end via cmd_net ---

    #[test]
    fn cmd_json_writes_passed_urls_to_output() {
        let url = spawn_server(TV_BODY, "application/json", Duration::ZERO);
        let input = temp_file("in-json");
        let out = temp_file("out-json");
        std::fs::write(&input, format!("probe {url} now\n")).unwrap();
        cmd_net(&input, NetType::Json, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "url + # info line: {content:?}");
        assert_eq!(lines[0], url.as_str());
        assert!(
            lines[1].starts_with("# count=1 4k=0 speed="),
            "info line: {content:?}"
        );
    }

    #[test]
    fn cmd_file_writes_passed_urls_to_output() {
        let url = spawn_server(TEXT_BODY, "text/plain", Duration::ZERO);
        let input = temp_file("in-file");
        let out = temp_file("out-file");
        std::fs::write(&input, format!("{url}\n")).unwrap();
        cmd_net(&input, NetType::File, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert_eq!(content.trim(), url);
    }

    #[test]
    fn cmd_json_rejects_text_and_unreachable() {
        let text_url = spawn_server(TEXT_BODY, "text/plain", Duration::ZERO);
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_port = l.local_addr().unwrap().port();
        drop(l);
        let dead_url = format!("http://127.0.0.1:{dead_port}/x");
        let input = temp_file("in-mix");
        let out = temp_file("out-mix");
        std::fs::write(&input, format!("{text_url}\n{dead_url}\n")).unwrap();
        cmd_net(&input, NetType::Json, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert!(content.is_empty(), "no URL should pass, got {content:?}");
    }

    #[test]
    fn cmd_file_rejects_json_body() {
        let json_url = spawn_server(JSON_BODY, "application/json", Duration::ZERO);
        let input = temp_file("in-json2file");
        let out = temp_file("out-json2file");
        std::fs::write(&input, format!("{json_url}\n")).unwrap();
        cmd_net(&input, NetType::File, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert!(content.is_empty(), "JSON must not pass file mode, got {content:?}");
    }

    #[test]
    fn no_addresses_is_an_error() {
        let input = temp_file("empty");
        std::fs::write(&input, "# only comments\n").unwrap();
        let err = cmd_net(&input, NetType::Domain, None, 10.0, &[], &[]).unwrap_err();
        let _ = std::fs::remove_file(&input);
        assert!(err.to_string().contains("no http(s) addresses"));
    }

    #[test]
    fn invalid_timeout_is_rejected() {
        let input = temp_file("tmo");
        std::fs::write(&input, "http://a.com\n").unwrap();
        let err = cmd_net(&input, NetType::Domain, None, 0.0, &[], &[]).unwrap_err();
        let _ = std::fs::remove_file(&input);
        assert!(err.to_string().contains("--timeout"));
    }

    // --- test mode ---

    fn dead_url() -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        format!("http://127.0.0.1:{port}/x")
    }

    #[test]
    fn test_mode_deletes_unreachable_lines_and_keeps_others() {
        let good = spawn_server(TEXT_BODY, "text/plain", Duration::ZERO);
        let bad = dead_url();
        let input = temp_file("test-in1");
        let out = temp_file("test-out1");
        std::fs::write(
            &input,
            format!("# comment stays\n\n{good}\n{bad}\nplain text line without url\n"),
        )
        .unwrap();
        cmd_net(&input, NetType::Test, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(
            lines,
            vec!["# comment stays", "", good.as_str(), "plain text line without url"],
            "bad line must be deleted, everything else kept: {content:?}"
        );
    }

    #[test]
    fn test_mode_multi_address_line_keeps_only_alive_ones() {
        let good = spawn_server(TEXT_BODY, "text/plain", Duration::ZERO);
        let bad = dead_url();
        let input = temp_file("test-in2");
        let out = temp_file("test-out2");
        std::fs::write(&input, format!("see {bad}, and {good} now\n")).unwrap();
        cmd_net(&input, NetType::Test, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert!(content.contains(&good), "alive address kept: {content:?}");
        assert!(!content.contains(&bad), "dead address removed: {content:?}");
        assert_eq!(content.lines().count(), 1, "line kept: {content:?}");
    }

    #[test]
    fn test_mode_all_dead_line_is_deleted() {
        let bad1 = dead_url();
        let bad2 = dead_url();
        let input = temp_file("test-in3");
        let out = temp_file("test-out3");
        std::fs::write(&input, format!("{bad1} and {bad2}\n")).unwrap();
        cmd_net(&input, NetType::Test, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert!(content.is_empty(), "all-dead line must be deleted: {content:?}");
    }

    #[test]
    fn test_mode_rebuild_keeps_surrounding_text() {
        let good = spawn_server(TEXT_BODY, "text/plain", Duration::ZERO);
        let bad = dead_url();
        let line = rebuild_line(
            &format!("prefix {bad} middle {good} suffix"),
            &extract_address_spans(&format!("prefix {bad} middle {good} suffix")),
            &[false, true],
        );
        assert!(line.contains("prefix "));
        assert!(line.contains(" middle "));
        assert!(line.contains(&good));
        assert!(line.contains(" suffix"));
        assert!(!line.contains(&bad));
    }

    #[test]
    fn url_ext_detects_suffix() {
        assert_eq!(
            url_ext("https://www.main.com/123.m3u"),
            Some("m3u".to_string())
        );
        // query strings do not affect the suffix
        assert_eq!(
            url_ext("https://www.main.com/123.M3U?token=abc"),
            Some("m3u".to_string())
        );
        assert_eq!(url_ext("http://a.com/a.tar.gz"), Some("gz".to_string()));
        assert_eq!(url_ext("http://a.com/abc"), None);
        assert_eq!(url_ext("http://a.com/abc."), None);
        assert_eq!(url_ext("http://a.com/"), None);
        assert_eq!(url_ext("http://a.com"), None);
        assert_eq!(url_ext("not a url"), None);
    }

    /// A reachable URL whose suffix is excluded must be removed WITHOUT being
    /// probed (the server stays untouched: had it been probed, the line would
    /// have been kept).
    #[test]
    fn test_mode_excluded_suffix_is_removed_without_probing() {
        let good = spawn_server(TEXT_BODY, "text/plain", Duration::ZERO);
        let port = url::Url::parse(&good).unwrap().port().unwrap();
        let m3u = format!("http://127.0.0.1:{port}/123.m3u");

        // Without -x the same reachable URL is kept.
        let input = temp_file("test-in-x1");
        let out_keep = temp_file("test-out-x1");
        std::fs::write(&input, format!("{m3u}\n")).unwrap();
        cmd_net(&input, NetType::Test, Some(&out_keep), 10.0, &[], &[]).unwrap();
        let kept = std::fs::read_to_string(&out_keep).unwrap();
        assert!(kept.contains(&m3u), "no -x: reachable URL kept: {kept:?}");

        // With -x m3u the line is deleted without probing.
        let out_del = temp_file("test-out-x2");
        cmd_net(&input, NetType::Test, Some(&out_del), 10.0, &["m3u".to_string()], &[]).unwrap();
        let deleted = std::fs::read_to_string(&out_del).unwrap();
        assert!(
            deleted.trim().is_empty(),
            "-x m3u: line must be deleted: {deleted:?}"
        );
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out_keep);
        let _ = std::fs::remove_file(&out_del);
    }

    #[test]
    fn test_mode_multiple_excludes_and_case_insensitive() {
        let good = spawn_server(TEXT_BODY, "text/plain", Duration::ZERO);
        let port = url::Url::parse(&good).unwrap().port().unwrap();
        let m3u = format!("http://127.0.0.1:{port}/a.m3u");
        let zip = format!("http://127.0.0.1:{port}/b.zip");
        let txt = format!("http://127.0.0.1:{port}/c.txt");
        let upper = format!("http://127.0.0.1:{port}/d.M3U");

        let input = temp_file("test-in-x3");
        let out = temp_file("test-out-x3");
        std::fs::write(
            &input,
            format!("see {m3u} and {txt} now\n{upper}\n{zip}\n"),
        )
        .unwrap();
        cmd_net(
            &input,
            NetType::Test,
            Some(&out),
            10.0,
            &["m3u".to_string(), "zip".to_string()],
            &[],
        )
        .unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(
            lines,
            vec![format!("see  and {txt} now").as_str()],
            "m3u removed from multi-address line, txt kept: {content:?}"
        );
        assert!(!content.contains(&m3u), "m3u removed: {content:?}");
        assert!(!content.contains(&upper), "case-insensitive .M3U removed: {content:?}");
        assert!(!content.contains(&zip), "zip line deleted: {content:?}");
        assert_eq!(content.lines().count(), 1, "only txt line kept: {content:?}");
    }

    #[test]
    fn exclude_ignored_outside_test_mode() {
        let input = temp_file("test-in-x4");
        std::fs::write(&input, "https://example.com/\n").unwrap();
        // -x is ignored in json mode (a warning is printed, no error).
        let out = temp_file("test-out-x4");
        cmd_net(
            &input,
            NetType::Json,
            Some(&out),
            10.0,
            &["m3u".to_string()],
            &[],
        )
        .unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn dup_key_normalizes_host_case() {
        assert_eq!(
            dup_key("http://EXAMPLE.com/a"),
            dup_key("http://example.com/a")
        );
        assert_eq!(dup_key("http://example.com:80/a"), dup_key("http://example.com/a"));
        assert_ne!(dup_key("http://example.com/a"), dup_key("http://example.com/b"));
    }

    /// The same reachable URL appearing twice: the first occurrence is kept,
    /// the second is removed without probing.
    #[test]
    fn test_mode_duplicate_line_is_deleted() {
        let good = spawn_server(TEXT_BODY, "text/plain", Duration::ZERO);
        let input = temp_file("test-in-d1");
        let out = temp_file("test-out-d1");
        std::fs::write(&input, format!("{good}\n{good}\n")).unwrap();
        cmd_net(&input, NetType::Test, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines, vec![good.as_str()], "only first occurrence kept: {content:?}");
    }

    #[test]
    fn test_mode_duplicate_address_in_one_line_removed() {
        let good = spawn_server(TEXT_BODY, "text/plain", Duration::ZERO);
        let input = temp_file("test-in-d2");
        let out = temp_file("test-out-d2");
        std::fs::write(&input, format!("{good} {good}\n")).unwrap();
        cmd_net(&input, NetType::Test, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert_eq!(content.lines().count(), 1, "line kept: {content:?}");
        assert_eq!(content.trim().matches(&good).count(), 1, "one copy left: {content:?}");
    }

    /// json mode: a duplicated URL is not probed and not written to -o.
    #[test]
    fn json_mode_duplicate_not_passed() {
        let good = spawn_server(TV_BODY, "application/json", Duration::ZERO);
        let input = temp_file("test-in-d3");
        let out = temp_file("test-out-d3");
        std::fs::write(&input, format!("{good}\n{good}\n")).unwrap();
        cmd_net(&input, NetType::Json, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert_eq!(content.lines().count(), 2, "one url + one # line: {content:?}");
        assert_eq!(content.lines().next().unwrap(), good.as_str());
        assert!(content.lines().nth(1).unwrap().starts_with("# count="));
    }

    // --- -t info ---

    /// A tiny RDAP mock: serves `(status, body)` per request path under
    /// `/domain/<name>`, answering `n` connections then shutting down.
    fn spawn_rdap_mock(routes: &[(&str, u16, &[u8])], n: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let routes: Vec<(String, u16, Vec<u8>)> = routes
            .iter()
            .map(|(p, s, b)| (p.to_string(), *s, b.to_vec()))
            .collect();
        std::thread::spawn(move || {
            for _ in 0..n {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let req = String::from_utf8_lossy(&buf);
                let path = req.split_whitespace().nth(1).unwrap_or("/");
                if let Some((_, status, body)) = routes.iter().find(|(p, _, _)| p == path) {
                    let reason = if *status == 200 { "OK" } else { "Not Found" };
                    let head = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/rdap+json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(body);
                } else {
                    let body = b"not found";
                    let head = format!(
                        "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(body);
                }
                let _ = stream.flush();
            }
        });
        format!("http://{addr}/domain/")
    }

    const RDAP_JSON: &[u8] = br#"{
      "ldhName": "EXAMPLE.COM",
      "status": ["client delete prohibited", "client transfer prohibited"],
      "events": [
        {"eventAction":"registration","eventDate":"1995-08-14T04:00:00Z"},
        {"eventAction":"expiration","eventDate":"2026-08-13T04:00:00Z"}
      ],
      "entities": [
        {"roles":["registrar"],"vcardArray":["vcard",[["version",{},"text","4.0"],["fn",{},"text","RESERVED-Internet Assigned Numbers Authority"]]]},
        {"roles":["registrant"],"vcardArray":["vcard",[["version",{},"text","4.0"],["fn",{},"text","Domain Administrator"],["org",{},"text","Example Org Co."],["email",{},"text","admin@example.com"],["adr",{},"text",["","","123 Main St","Springfield","IL","62701","US"]]]]}
      ],
      "nameservers": [{"ldhName":"a.iana-servers.net"},{"ldhName":"b.iana-servers.net"}]
    }"#;

    #[test]
    fn parent_domain_strips_leftmost_label() {
        assert_eq!(
            parent_domain("www.example.com"),
            Some("example.com".to_string())
        );
        assert_eq!(parent_domain("example.com"), Some("com".to_string()));
        assert_eq!(parent_domain("com"), None);
    }

    #[test]
    fn parse_rdap_extracts_fields() {
        let v: serde_json::Value = serde_json::from_slice(RDAP_JSON).unwrap();
        let info = parse_rdap(&v);
        assert_eq!(
            info.registrar.as_deref(),
            Some("RESERVED-Internet Assigned Numbers Authority")
        );
        assert_eq!(info.registrant.as_deref(), Some("Domain Administrator"));
        assert_eq!(info.org.as_deref(), Some("Example Org Co."));
        assert_eq!(info.email.as_deref(), Some("admin@example.com"));
        assert_eq!(
            info.address.as_deref(),
            Some("123 Main St, Springfield, IL, 62701, US")
        );
        assert_eq!(info.country.as_deref(), Some("US"));
        assert_eq!(info.created.as_deref(), Some("1995-08-14T04:00:00Z"));
        assert_eq!(info.expires.as_deref(), Some("2026-08-13T04:00:00Z"));
        assert_eq!(info.status.len(), 2);
        assert_eq!(
            info.nameservers,
            vec!["a.iana-servers.net".to_string(), "b.iana-servers.net".to_string()]
        );
    }

    #[test]
    fn parse_rdap_handles_missing_registrant_fields() {
        // Some registries (e.g. Verisign for example.com) expose no
        // registrant entity — those fields must stay None, not panic.
        let v: serde_json::Value = serde_json::from_slice(RDAP_JSON).unwrap();
        let mut v = v;
        let entities = v.get_mut("entities").unwrap().as_array_mut().unwrap();
        entities.retain(|e| {
            e.get("roles")
                .and_then(|r| r.as_array())
                .map(|rs| !rs.iter().any(|x| x.as_str() == Some("registrant")))
                .unwrap_or(true)
        });
        let info = parse_rdap(&v);
        assert_eq!(info.registrar.as_deref(), Some("RESERVED-Internet Assigned Numbers Authority"));
        assert!(info.registrant.is_none());
        assert!(info.org.is_none());
        assert!(info.email.is_none());
        assert!(info.address.is_none());
        assert!(info.country.is_none());
        assert_eq!(info.created.as_deref(), Some("1995-08-14T04:00:00Z"));
    }

    #[test]
    fn parse_rdap_uses_adr_cc_param_for_country() {
        // Some registrars (e.g. MarkMonitor) carry the country as the `cc`
        // parameter of the adr row instead of the 7th value component.
        let body = r#"{
              "entities":[
                {"roles":["registrant"],"vcardArray":["vcard",[
                  ["version",{},"text","4.0"],
                  ["fn",{},"text","REDACTED REGISTRANT"],
                  ["org",{"type":"work"},"text","广州网易计算机系统有限公司"],
                  ["adr",{"cc":"CN"},"text",["","","REDACTED","REDACTED","","REDACTED",""]]
                ]]}
              ]
            }"#;
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        let info = parse_rdap(&v);
        assert_eq!(info.org.as_deref(), Some("广州网易计算机系统有限公司"));
        assert_eq!(info.country.as_deref(), Some("CN"));
        // The 7th value component is empty, so cc must be the source.
        assert_eq!(info.address.as_deref(), Some("REDACTED, REDACTED, REDACTED"));
    }

    #[test]
    fn info_mode_follows_related_registrar_rdap_link() {
        // A registry response without a registrant entity advertises a
        // `related` application/rdap+json link; the registrar-level response
        // carries the full registrant/org and must be used instead.
        let registrar = spawn_rdap_mock(&[("/domain/registrar-data", 200, RDAP_JSON)], 1);
        let registry_json = format!(
            r#"{{"links":[{{"rel":"self","href":"http://reg/domain/example.com","type":"application/rdap+json"}},
                {{"rel":"related","href":"{registrar}registrar-data","type":"application/rdap+json"}}],
              "status":[],"events":[],"nameservers":[]}}"#
        );
        let registry = spawn_rdap_mock(
            &[("/domain/example.com", 200, registry_json.as_bytes())],
            1,
        );
        let client = test_client();
        let out = cmd_info_with_base(&client, &["example.com".to_string()], &registry).unwrap();
        assert!(
            out.contains("Registrant:  Domain Administrator"),
            "{out}"
        );
        assert!(out.contains("Org:         Example Org Co."), "{out}");
        assert!(out.contains("Country:     US"), "{out}");
    }

    #[test]
    fn info_mode_falls_back_to_parent() {
        let base = spawn_rdap_mock(
            &[
                ("/domain/www.example.com", 404, b"not found"),
                ("/domain/example.com", 200, RDAP_JSON),
            ],
            2,
        );
        let client = test_client();
        let out = cmd_info_with_base(&client, &["www.example.com".to_string()], &base).unwrap();
        assert!(
            out.contains("www.example.com: no registration data, using parent example.com"),
            "{out}"
        );
        assert!(out.contains("Registrar:   RESERVED-Internet Assigned Numbers Authority"));
        assert!(out.contains("Created:     1995-08-14T04:00:00Z"));
    }

    #[test]
    fn info_mode_reports_no_data_when_chain_exhausted() {
        let base = spawn_rdap_mock(&[("/domain/example.com", 404, b"nf")], 2);
        let client = test_client();
        let out = cmd_info_with_base(&client, &["example.com".to_string()], &base).unwrap();
        assert!(
            out.contains("no registration info found (checked: example.com, com)"),
            "{out}"
        );
    }

    #[test]
    fn info_mode_falls_back_on_lookup_error() {
        // A failed lookup (500 + non-JSON body) on the child also falls back
        // to the parent, and the fallback reason is annotated.
        let base = spawn_rdap_mock(
            &[
                ("/domain/www.example.com", 500, b"oops"),
                ("/domain/example.com", 200, RDAP_JSON),
            ],
            2,
        );
        let client = test_client();
        let out = cmd_info_with_base(&client, &["www.example.com".to_string()], &base).unwrap();
        assert!(
            out.contains("www.example.com: lookup on www.example.com failed"),
            "{out}"
        );
        assert!(out.contains("using parent example.com"), "{out}");
        assert!(out.contains("Registrar:   RESERVED-Internet Assigned Numbers Authority"));
    }

    #[test]
    fn info_mode_direct_hit_and_multiple_names() {
        let base = spawn_rdap_mock(
            &[
                ("/domain/example.com", 200, RDAP_JSON),
                ("/domain/example.org", 200, RDAP_JSON),
            ],
            2,
        );
        let client = test_client();
        let out = cmd_info_with_base(
            &client,
            &["example.com".to_string(), "example.org".to_string()],
            &base,
        )
        .unwrap();
        assert!(out.contains("example.com"), "{out}");
        assert!(out.contains("example.org"), "{out}");
        assert!(!out.contains("using parent"), "{out}");
    }

    // --- -t json --sort ---

    /// tv validity: node completeness rules.
    #[test]
    fn tv_validity_checks_nodes() {
        let good = serde_json::json!({
            "sites": [{"name": "a"}],
            "lives": [],
            "parses": [],
            "rules": []
        });
        assert!(is_valid_tv(&good));
        // lives is optional
        let mut v = good.clone();
        v.as_object_mut().unwrap().remove("lives");
        assert!(is_valid_tv(&v));
        // parses is optional
        let mut v = good.clone();
        v.as_object_mut().unwrap().remove("parses");
        assert!(is_valid_tv(&v));
        // rules is optional
        let mut v = good.clone();
        v.as_object_mut().unwrap().remove("rules");
        assert!(is_valid_tv(&v));
        // empty sites
        let mut v = good.clone();
        v["sites"] = serde_json::json!([]);
        assert!(!is_valid_tv(&v));
        // site without name
        let mut v = good.clone();
        v["sites"] = serde_json::json!([{"key": "x"}]);
        assert!(!is_valid_tv(&v));
        // sites not an array
        let mut v = good.clone();
        v["sites"] = serde_json::json!("nope");
        assert!(!is_valid_tv(&v));
        // name key present (value type irrelevant) still counts
        let mut v = good.clone();
        v["sites"] = serde_json::json!([{"name": 42}]);
        assert!(is_valid_tv(&v));
    }

    /// strip_trailing_commas removes commas before } / ] outside strings only.
    #[test]
    fn strip_trailing_commas_removes_only_real_trailing_commas() {
        let input = r#"{"a":[1,2,],"b":"x,y,","c":{"d":3,},"e":"},"}"#;
        let out = strip_trailing_commas(input);
        assert_eq!(out, r#"{"a":[1,2],"b":"x,y,","c":{"d":3},"e":"},"}"#);
        // no trailing commas -> unchanged
        assert_eq!(strip_trailing_commas(r#"{"a":[1,2]}"#), r#"{"a":[1,2]}"#);
    }

    /// parse_tv_json falls back to trailing-comma-stripped parsing.
    #[test]
    fn parse_tv_json_tolerates_trailing_commas() {
        let body = br#"{"sites":[{"name":"a"},],"lives":[],"parses":[],"rules":[],}"#;
        let v = parse_tv_json(body).expect("trailing commas tolerated");
        assert!(is_valid_tv(&v));
        // garbage still fails
        assert!(parse_tv_json(b"not json").is_none());
    }

    /// End-to-end: a body with trailing commas still passes tv mode.
    #[test]
    fn cmd_json_accepts_trailing_comma_tv_body() {
        let body =
            br#"{"sites":[{"name":"a"},{"name":"b"},],"lives":[],"parses":[],"rules":[],}"#.to_vec();
        let url = spawn_server_owned(body, "application/json", Duration::ZERO);
        let input = temp_file("in-tv-lenient");
        let out = temp_file("out-tv-lenient");
        std::fs::write(&input, format!("{url}\n")).unwrap();
        cmd_net(&input, NetType::Json, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert_eq!(content.lines().next().unwrap(), url.as_str());
        assert!(content.lines().nth(1).unwrap().starts_with("# count=2 4k=0"));
    }

    /// A body that parses as JSON but lacks tv nodes is a format mismatch.
    #[test]
    fn cmd_json_rejects_non_tv_json() {
        let url = spawn_server(JSON_BODY, "application/json", Duration::ZERO);
        let input = temp_file("in-tv-notv");
        let out = temp_file("out-tv-notv");
        std::fs::write(&input, format!("{url}\n")).unwrap();
        cmd_net(&input, NetType::Json, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert!(content.is_empty(), "non-tv JSON must not pass: {content:?}");
    }

    const SORT_JSON_3: &[u8] = br#"{"spider":"http://127.0.0.1:1/x.jpg;md5;abc","lives":[],"parses":[],"rules":[],"sites":[
      {"name":"A 4K"},
      {"name":"B 4k"},
      {"name":"C"}
    ]}"#;
    const SORT_JSON_1: &[u8] = br#"{"spider":"not a url","lives":[],"parses":[],"rules":[],"sites":[{"name":"D"}]}"#;

    #[test]
    fn json_stats_are_computed() {
        let v3: serde_json::Value = serde_json::from_slice(SORT_JSON_3).unwrap();
        let v1: serde_json::Value = serde_json::from_slice(SORT_JSON_1).unwrap();
        assert_eq!(count_sites(&v3), 3);
        assert_eq!(count_name_containing(&v3, "4k"), 2, "4K and 4k both match");
        assert_eq!(count_sites(&v1), 1);
        assert_eq!(count_name_containing(&v1, "4k"), 0);
        // --words keyword is configurable: other words count their own matches.
        assert_eq!(count_name_containing(&v3, "c"), 1, "only \"C\" contains c");
        assert_eq!(count_name_containing(&v3, "zzz"), 0);
        assert_eq!(
            spider_url(&v3),
            Some("http://127.0.0.1:1/x.jpg".to_string()),
            "semicolon-separated spider string keeps the part before ';'"
        );
        assert_eq!(spider_url(&v1), Some("not a url".to_string()));
        assert_eq!(spider_url(&serde_json::json!({"spider": 42})), None);
        assert_eq!(spider_url(&serde_json::json!({"sites": []})), None);
    }

    #[test]
    fn parse_sort_keys_accepts_aliases_and_rejects_unknown() {
        assert_eq!(
            parse_sort_keys(&["count".into(), "name".into(), "speed".into()]).unwrap(),
            vec![SortKey::Count, SortKey::Name, SortKey::Speed]
        );
        assert_eq!(parse_sort_keys(&[]).unwrap(), vec![]);
        let err = parse_sort_keys(&["count".into(), "foo".into()]).unwrap_err();
        assert!(err.to_string().contains("invalid --sort key"));
        // `speen` is no longer accepted (only `speed` is).
        let err = parse_sort_keys(&["speen".into()]).unwrap_err();
        assert!(err.to_string().contains("invalid --sort key"), "{err}");
    }

    /// `--sort count`: more sites first. Input order is reversed to prove the
    /// sort actually runs.
    #[test]
    fn json_sort_by_count_desc() {
        let one = spawn_server(SORT_JSON_1, "application/json", Duration::ZERO);
        let three = spawn_server(SORT_JSON_3, "application/json", Duration::ZERO);
        let input = temp_file("in-sort-c");
        let out = temp_file("out-sort-c");
        std::fs::write(&input, format!("{one}\n{three}\n")).unwrap();
        cmd_net(
            &input,
            NetType::Json,
            Some(&out),
            10.0,
            &[],
            &["count".to_string()],
        )
        .unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], three.as_str(), "count=3 first: {content:?}");
        assert!(lines[1].starts_with("# count=3 4k=2"));
        assert_eq!(lines[2], one.as_str(), "count=1 second: {content:?}");
        assert!(lines[3].starts_with("# count=1 4k=0"));
    }

    /// Default (no --sort): sort by count desc, then name (4k count) desc.
    /// A higher count beats a higher name count.
    #[test]
    fn json_sort_defaults_to_count_then_name() {
        // A: count=2 name=0 ; B: count=1 name=1
        let a = spawn_server_owned(
            br#"{"sites":[{"name":"a"},{"name":"b"}],"lives":[],"parses":[],"rules":[]}"#.to_vec(),
            "application/json",
            Duration::ZERO,
        );
        let b = spawn_server_owned(
            br#"{"sites":[{"name":"x 4K"}],"lives":[],"parses":[],"rules":[]}"#.to_vec(),
            "application/json",
            Duration::ZERO,
        );
        let input = temp_file("in-sort-n");
        let out = temp_file("out-sort-n");
        // input order: b (name=1) first, a (count=2) second
        std::fs::write(&input, format!("{b}\n{a}\n")).unwrap();
        cmd_net(&input, NetType::Json, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], a.as_str(), "count=2 first (default): {content:?}");
        assert_eq!(lines[2], b.as_str(), "name=1 second: {content:?}");
    }

    /// Multi-key: count equal -> tie broken by name (order of the keys).
    #[test]
    fn json_sort_multi_key_uses_key_order() {
        // count=2 name=2 ; count=2 name=0 ; count=1 name=1
        let j1 = spawn_server_owned(
            br#"{"sites":[{"name":"a 4K"},{"name":"b 4K"}],"lives":[],"parses":[],"rules":[]}"#.to_vec(),
            "application/json",
            Duration::ZERO,
        );
        let j2 = spawn_server_owned(
            br#"{"sites":[{"name":"a"},{"name":"b"}],"lives":[],"parses":[],"rules":[]}"#.to_vec(),
            "application/json",
            Duration::ZERO,
        );
        let j3 = spawn_server_owned(
            br#"{"sites":[{"name":"x 4K"}],"lives":[],"parses":[],"rules":[]}"#.to_vec(),
            "application/json",
            Duration::ZERO,
        );
        let input = temp_file("in-sort-m");
        let out = temp_file("out-sort-m");
        // input order: j2, j3, j1
        std::fs::write(&input, format!("{j2}\n{j3}\n{j1}\n")).unwrap();
        cmd_net(
            &input,
            NetType::Json,
            Some(&out),
            10.0,
            &[],
            &["count".to_string(), "name".to_string()],
        )
        .unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        let urls: Vec<&str> = content.lines().step_by(2).collect();
        // count desc: j1(2),j2(2),j3(1) -> name desc among count=2: j1(name=2) then j2(name=0);
        // a count-only sort would keep j2 before j1 (stable), so this proves the tie-break.
        assert_eq!(urls, vec![j1.as_str(), j2.as_str(), j3.as_str()], "{content:?}");
    }

    /// `--sort speed`: faster spider download first (ascending).
    #[test]
    fn json_sort_by_speed_asc() {
        // A slow downloadable body (small + 400ms delay) and a fast one (large, no delay).
        let slow = spawn_server_owned(vec![b'x'; 1024], "application/octet-stream", Duration::from_millis(400));
        let fast = spawn_server_owned(vec![b'y'; 128 * 1024], "application/octet-stream", Duration::ZERO);
        let jslow = spawn_server_owned(
            format!(r#"{{"spider":"{slow}","sites":[{{"name":"s"}}],"lives":[],"parses":[],"rules":[]}}"#)
                .into_bytes(),
            "application/json",
            Duration::ZERO,
        );
        let jfast = spawn_server_owned(
            format!(r#"{{"spider":"{fast}","sites":[{{"name":"f"}}],"lives":[],"parses":[],"rules":[]}}"#)
                .into_bytes(),
            "application/json",
            Duration::ZERO,
        );
        let input = temp_file("in-sort-s");
        let out = temp_file("out-sort-s");
        std::fs::write(&input, format!("{jslow}\n{jfast}\n")).unwrap();
        cmd_net(
            &input,
            NetType::Json,
            Some(&out),
            10.0,
            &[],
            &["speed".to_string()],
        )
        .unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        let urls: Vec<&str> = content.lines().step_by(2).collect();
        assert_eq!(
            urls,
            vec![jslow.as_str(), jfast.as_str()],
            "ascending speed: slowest first: {content:?}"
        );
    }

    // --- -t tvs ---

    const TVS_BODY_2: &[u8] =
        br#"{"urls":[{"url":"a","name":"a"},{"url":"b","name":"b"}]}"#;
    const TVS_BODY_3: &[u8] = br#"{"urls":[{"url":"a","name":"a"},{"url":"b","name":"b"},{"url":"c","name":"c"}]}"#;

    #[test]
    fn tvs_groups_single_and_multi() {
        let single = spawn_server_owned(TV_BODY.to_vec(), "application/json", Duration::ZERO);
        let multi3 = spawn_server_owned(TVS_BODY_3.to_vec(), "application/json", Duration::ZERO);
        let multi2 = spawn_server_owned(TVS_BODY_2.to_vec(), "application/json", Duration::ZERO);
        let out = temp_file("out-tvs1");
        let client = test_client();
        cmd_tvs(
            &client,
            &[(single.clone(), None), (multi3.clone(), None), (multi2.clone(), None)],
            Some(&out),
            &[],
            2,
            "4k",
        )
        .unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&out);
        // Both group headers, single first.
        assert!(content.starts_with("--- 单仓 (1) ---"), "{content}");
        assert!(content.contains("--- 多仓 (2) ---"), "{content}");
        // Single item carries the tv-style info line.
        assert!(content.contains(&format!("{single}\n# count=1 4k=0")), "{content}");
        // Multi items carry only the url count, sorted descending (3 before 2).
        assert!(content.contains(&format!("{multi3}\n# count=3")), "{content}");
        assert!(content.contains(&format!("{multi2}\n# count=2")), "{content}");
        let i3 = content.find(&multi3).unwrap();
        let i2 = content.find(&multi2).unwrap();
        assert!(i3 < i2, "multi group must be count desc: {content}");
    }

    #[test]
    fn tvs_only_multi_group_omits_single_header() {
        let multi = spawn_server_owned(TVS_BODY_3.to_vec(), "application/json", Duration::ZERO);
        let out = temp_file("out-tvs2");
        let client = test_client();
        cmd_tvs(&client, &[(multi.clone(), None)], Some(&out), &[], 1, "4k").unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&out);
        assert!(content.contains("--- 多仓 (1) ---"), "{content}");
        assert!(!content.contains("单仓"), "{content}");
        assert!(content.contains(&format!("{multi}\n# count=3")), "{content}");
    }

    #[test]
    fn tvs_rejects_body_without_sites_or_urls() {
        let bad = spawn_server_owned(br#"{"foo":1}"#.to_vec(), "application/json", Duration::ZERO);
        let out = temp_file("out-tvs3");
        let client = test_client();
        cmd_tvs(&client, &[(bad, None)], Some(&out), &[], 1, "4k").unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&out);
        assert!(content.is_empty(), "{content}");
    }

    #[test]
    fn tvs_accepts_bom_prefixed_multi_json() {
        // raw.githubusercontent etc. serve BOM-prefixed JSON as text/plain;
        // the BOM must not make a valid multi (`.urls`) body fail to parse.
        let mut body = vec![0xEF, 0xBB, 0xBF];
        body.extend_from_slice(TVS_BODY_3);
        let v = parse_tv_json(&body).expect("BOM-prefixed multi JSON must parse");
        assert!(v.get("urls").and_then(|u| u.as_array()).is_some());
        assert!(!is_valid_tv(&v), "multi JSON is not a tv (sites) doc");
        // Same for a tv (sites) body.
        let mut body2 = vec![0xEF, 0xBB, 0xBF];
        body2.extend_from_slice(TV_BODY);
        let v2 = parse_tv_json(&body2).expect("BOM-prefixed tv JSON must parse");
        assert!(is_valid_tv(&v2));
    }

    // --- input remarks (`url  # remark`) ---

    #[test]
    fn read_entries_captures_trailing_note() {
        let p = temp_file("in-note1");
        // Remark separated by tabs / spaces after the address.
        std::fs::write(&p, "https://a.example/x.json\t\t#二月红接口\n").unwrap();
        let entries = read_entries(&p).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "https://a.example/x.json");
        assert_eq!(entries[0].1.as_deref(), Some("二月红接口"));
        // No remark -> None.
        std::fs::write(&p, "https://a.example/y.json\n").unwrap();
        let entries = read_entries(&p).unwrap();
        assert_eq!(entries[0].1, None);
        // Multi-address line: the remark binds to the last address only.
        std::fs::write(&p, "https://a.example/x.json https://b.example/y.json # 多仓备注\n").unwrap();
        let entries = read_entries(&p).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].1, None);
        assert_eq!(entries[1].1.as_deref(), Some("多仓备注"));
        // A line starting with # is still skipped entirely.
        std::fs::write(&p, "# https://a.example/z.json\nhttps://c.example/w.json\n").unwrap();
        let entries = read_entries(&p).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "https://c.example/w.json");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn tv_mode_outputs_input_note() {
        let tv = spawn_server_owned(SORT_JSON_3.to_vec(), "application/json", Duration::ZERO);
        let input = temp_file("in-note2");
        let out = temp_file("out-note2");
        std::fs::write(&input, format!("{tv}\t#二月红接口\n")).unwrap();
        cmd_net(&input, NetType::Json, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert!(
            content.contains(&format!("{tv}\n# note=二月红接口  count=")),
            "{content}"
        );
    }

    #[test]
    fn tvs_mode_outputs_input_note() {
        let multi = spawn_server_owned(TVS_BODY_3.to_vec(), "application/json", Duration::ZERO);
        let input = temp_file("in-note3");
        let out = temp_file("out-note3");
        std::fs::write(&input, format!("{multi}\t#接口A\n")).unwrap();
        cmd_net(&input, NetType::Tvs, Some(&out), 10.0, &[], &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert!(
            content.contains(&format!("{multi}\n# note=接口A  count=3")),
            "{content}"
        );
    }

    #[test]
    fn tv_mode_words_keyword_drives_name_label_and_count() {
        let tv = spawn_server_owned(SORT_JSON_3.to_vec(), "application/json", Duration::ZERO);
        let input = temp_file("in-words");
        let out = temp_file("out-words");
        std::fs::write(&input, format!("{tv}\n")).unwrap();
        // Default --words 4k: label is 4k=2.
        cmd_net(&input, NetType::Json, Some(&out), 10.0, &[], &[]).unwrap();
        let dflt = std::fs::read_to_string(&out).unwrap();
        assert!(dflt.contains("# count=3 4k=2"), "{dflt}");
        // Custom --words: the label shows the keyword and counts its matches.
        cmd_net_t(&input, NetType::Json, Some(&out), 10.0, &[], &[], Some(1), "hd").unwrap();
        let custom = std::fs::read_to_string(&out).unwrap();
        assert!(custom.contains("# count=3 hd=0"), "{custom}");
        cmd_net_t(&input, NetType::Json, Some(&out), 10.0, &[], &[], Some(1), "c").unwrap();
        let custom2 = std::fs::read_to_string(&out).unwrap();
        assert!(custom2.contains("# count=3 c=1"), "{custom2}");
        assert!(!custom2.contains(" 4k="), "{custom2}");
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
    }

    // --- parallelism ---

    /// tv/tvs probe addresses concurrently: with `threads` < address count
    /// the collected report still matches the serial result (group order,
    /// counts), and every address produces one immediate `[OK]`-style line.
    #[test]
    fn json_sorted_parallel_matches_serial() {
        let tv3 = spawn_server_owned(SORT_JSON_3.to_vec(), "application/json", Duration::ZERO);
        let tv1 = spawn_server_owned(SORT_JSON_1.to_vec(), "application/json", Duration::ZERO);
        let input = temp_file("in-par");
        let out = temp_file("out-par");
        std::fs::write(&input, format!("{tv3}\n{tv1}\n")).unwrap();
        // Serial (default threads) vs explicit threads=1 must agree.
        cmd_net(&input, NetType::Json, Some(&out), 10.0, &[], &[]).unwrap();
        let serial = std::fs::read_to_string(&out).unwrap();
        cmd_net_t(&input, NetType::Json, Some(&out), 10.0, &[], &[], Some(1), "4k").unwrap();
        let parallel = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert_eq!(serial, parallel, "parallel report must match serial report");
        assert!(parallel.contains("# count=3 4k=2"), "{parallel}");
        assert!(parallel.contains("# count=1 4k=0"), "{parallel}");
    }

    #[test]
    fn tvs_parallel_reports_each_address_and_groups() {
        let single = spawn_server_owned(TV_BODY.to_vec(), "application/json", Duration::ZERO);
        let multi3 = spawn_server_owned(TVS_BODY_3.to_vec(), "application/json", Duration::ZERO);
        let multi2 = spawn_server_owned(TVS_BODY_2.to_vec(), "application/json", Duration::ZERO);
        let out = temp_file("out-par-tvs");
        let client = test_client();
        cmd_tvs(
            &client,
            &[(single.clone(), None), (multi3.clone(), None), (multi2.clone(), None)],
            Some(&out),
            &[],
            2,
            "4k",
        )
        .unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&out);
        assert!(content.starts_with("--- 单仓 (1) ---"), "{content}");
        assert!(content.contains("--- 多仓 (2) ---"), "{content}");
        assert!(content.contains(&format!("{multi3}\n# count=3")), "{content}");
        assert!(content.contains(&format!("{multi2}\n# count=2")), "{content}");
        let i3 = content.find(&multi3).unwrap();
        let i2 = content.find(&multi2).unwrap();
        assert!(i3 < i2, "multi group must be count desc: {content}");
    }
}
