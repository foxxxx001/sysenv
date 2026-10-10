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
//! - `json`: GET each address; the body must parse as JSON — every address is
//!   reported (url / reachability / response time) as soon as it is probed;
//! - `file`: GET each address; the body must be plain text (UTF-8, not JSON,
//!   not binary) — same per-address reporting;
//! - `test`: GET each address and probe reachability only; unreachable
//!   addresses are reported on screen and removed from the source lines, then
//!   the cleaned source (kept lines verbatim) is written to `-o`.
//!
//! `-o FILE` writes every address that passed (reachable + format matched) to
//! FILE, one per line (json / file modes), or the cleaned source lines
//! (test mode).

use anyhow::{Context, Result, bail};
use std::error::Error as _;
use std::path::Path;
use std::time::{Duration, Instant};

/// Detection type for `sys net -t`.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetType {
    /// Probe the address 3 times and average the response time
    #[value(name = "domain")]
    Domain,
    /// The response body must parse as JSON
    #[value(name = "json")]
    Json,
    /// The response body must be plain text (not JSON, not binary)
    #[value(name = "file")]
    File,
    /// Probe reachability; unreachable addresses are reported, removed from
    /// the source lines, and the cleaned result is written to `-o`
    #[value(name = "test")]
    Test,
}

impl NetType {
    fn label(self) -> &'static str {
        match self {
            NetType::Domain => "domain",
            NetType::Json => "json",
            NetType::File => "file",
            NetType::Test => "test",
        }
    }
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
    let raw = std::fs::read(input).with_context(|| format!("cannot read {}", input.display()))?;
    let text = String::from_utf8_lossy(&raw);
    let text = text.strip_prefix('\u{feff}').unwrap_or(text.as_ref());
    let mut urls = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        for raw_url in extract_addresses(line) {
            if let Some(u) = normalize_url(&raw_url) {
                urls.push(u);
            }
        }
    }
    if urls.is_empty() {
        bail!("no http(s) addresses found in {}", input.display());
    }
    Ok(urls)
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
    let results = probe_domains(client, urls);
    for line in render_domain(&results) {
        println!("{line}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// json / file modes
// ---------------------------------------------------------------------------

fn cmd_content(
    client: &reqwest::blocking::Client,
    urls: &[String],
    kind: NetType,
    output: Option<&Path>,
) -> Result<()> {
    let w = urls.iter().map(|u| u.chars().count()).max().unwrap_or(8).max(8);
    let mut passed: Vec<String> = Vec::new();
    let mut n_ok = 0usize;
    let mut n_mismatch = 0usize;
    let mut n_no = 0usize;

    for url in urls {
        match request(client, url, true) {
            Ok((status, elapsed, Some(body))) => {
                let got = body_kind(&body);
                let ok = match kind {
                    NetType::Json => got == BodyKind::Json,
                    NetType::File => got == BodyKind::Text,
                    NetType::Domain | NetType::Test => unreachable!("handled elsewhere"),
                };
                if ok {
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
                        kind.label()
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

    println!(
        "--- {} of {} URLs passed ({} unreachable, {} format mismatch) ---",
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
    let mut n_deleted_lines = 0usize;

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
                None => match request(client, &span.url, false) {
                    Ok((_, elapsed, _)) => {
                        alive.push(true);
                        parts.push(format!("[OK] {}  time={}", span.url, fmt_ms(elapsed)));
                    }
                    Err(e) => {
                        alive.push(false);
                        n_unreach += 1;
                        parts.push(format!("[NO] {}  error={e}", span.url));
                    }
                },
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

    let excl = if n_excluded > 0 {
        format!(", {n_excluded} excluded")
    } else {
        String::new()
    };
    println!(
        "--- checked {n_addr} address(es) in {total} line(s): {n_unreach} unreachable{excl}, {} line(s) kept, {n_deleted_lines} line(s) deleted ---",
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

/// `sys net -i FILE [-t domain|json|file|test] [-o FILE] [--timeout SECONDS] [-x SUFFIX ...]`
pub fn cmd_net(
    input: &Path,
    kind: NetType,
    output: Option<&Path>,
    timeout_secs: f64,
    exclude: &[String],
) -> Result<()> {
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
                    "warning: -o/--output applies to `-t json|file|test` only; ignored ({})",
                    out.display()
                );
            }
            if !excludes.is_empty() {
                eprintln!("warning: -x/--exclude applies to `-t test` only; ignored");
            }
            let urls = read_addresses(input)?;
            cmd_domain(&client, &urls)
        }
        NetType::Json | NetType::File => {
            if !excludes.is_empty() {
                eprintln!("warning: -x/--exclude applies to `-t test` only; ignored");
            }
            let urls = read_addresses(input)?;
            cmd_content(&client, &urls, kind, output)
        }
        NetType::Test => cmd_test(&client, input, output, &excludes),
    }
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
                let _ = stream.write_all(body);
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
        let url = spawn_server(JSON_BODY, "application/json", Duration::ZERO);
        let input = temp_file("in-json");
        let out = temp_file("out-json");
        std::fs::write(&input, format!("probe {url} now\n")).unwrap();
        cmd_net(&input, NetType::Json, Some(&out), 10.0, &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert_eq!(content.trim(), url);
    }

    #[test]
    fn cmd_file_writes_passed_urls_to_output() {
        let url = spawn_server(TEXT_BODY, "text/plain", Duration::ZERO);
        let input = temp_file("in-file");
        let out = temp_file("out-file");
        std::fs::write(&input, format!("{url}\n")).unwrap();
        cmd_net(&input, NetType::File, Some(&out), 10.0, &[]).unwrap();
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
        cmd_net(&input, NetType::Json, Some(&out), 10.0, &[]).unwrap();
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
        cmd_net(&input, NetType::File, Some(&out), 10.0, &[]).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert!(content.is_empty(), "JSON must not pass file mode, got {content:?}");
    }

    #[test]
    fn no_addresses_is_an_error() {
        let input = temp_file("empty");
        std::fs::write(&input, "# only comments\n").unwrap();
        let err = cmd_net(&input, NetType::Domain, None, 10.0, &[]).unwrap_err();
        let _ = std::fs::remove_file(&input);
        assert!(err.to_string().contains("no http(s) addresses"));
    }

    #[test]
    fn invalid_timeout_is_rejected() {
        let input = temp_file("tmo");
        std::fs::write(&input, "http://a.com\n").unwrap();
        let err = cmd_net(&input, NetType::Domain, None, 0.0, &[]).unwrap_err();
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
        cmd_net(&input, NetType::Test, Some(&out), 10.0, &[]).unwrap();
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
        cmd_net(&input, NetType::Test, Some(&out), 10.0, &[]).unwrap();
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
        cmd_net(&input, NetType::Test, Some(&out), 10.0, &[]).unwrap();
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
        cmd_net(&input, NetType::Test, Some(&out_keep), 10.0, &[]).unwrap();
        let kept = std::fs::read_to_string(&out_keep).unwrap();
        assert!(kept.contains(&m3u), "no -x: reachable URL kept: {kept:?}");

        // With -x m3u the line is deleted without probing.
        let out_del = temp_file("test-out-x2");
        cmd_net(&input, NetType::Test, Some(&out_del), 10.0, &["m3u".to_string()]).unwrap();
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
        )
        .unwrap();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
    }
}
