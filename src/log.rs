//! `sys log` — colored, less-like log file viewer.
//!
//! Parses common log-line prefixes (timestamp / level / app marker) with a
//! color palette — time orange, level gradient, app marker and IP addresses
//! blue — plus filters (`--level` / `--line` / `--newer`), searches
//! (`--search` with n/u navigation) and tails a file (`-f`).
//!
//! The interactive viewer is only entered when stdout is a TTY; a piped stdout
//! prints plain text (no ANSI escapes, no pager), like `less` does.

use std::fs::File;
use std::io::{self, IsTerminal, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Datelike, FixedOffset, Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};

use crossterm::{
    cursor, event,
    execute,
    style::{Attribute, Color, Print, SetAttribute, SetForegroundColor},
    terminal::{self, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use crossterm::event::{Event, KeyCode, KeyModifiers};

/// How far into a line the "time anywhere" fallback scan looks.
const TIME_SCAN_LIMIT: usize = 80;
/// Follow-mode poll interval.
const FOLLOW_POLL_MS: u64 = 150;

// ---------------------------------------------------------------------------
// Color palette
// ---------------------------------------------------------------------------

/// Timestamp — orange.
const COLOR_TIME: Color = Color::Rgb { r: 255, g: 165, b: 0 };
/// App marker (head token after the timestamp) — blue.
const COLOR_APP: Color = Color::Rgb { r: 80, g: 160, b: 255 };
/// IP address (IPv4 / IPv6) anywhere in a line — blue (same blue as the app
/// marker, per user request).
const COLOR_IP: Color = Color::Rgb { r: 80, g: 160, b: 255 };
/// Line numbers (search mode) — dim gray.
const COLOR_LINENO: Color = Color::Rgb { r: 130, g: 130, b: 130 };

/// Gradient: info and below are green (lower = lighter); warning and above are
/// red (warning = red, error = dark red, fatal = deepest red + bold).
fn level_color(level: Level) -> (Color, bool) {
    use Level::*;
    match level {
        Trace => (Color::Rgb { r: 178, g: 230, b: 178 }, false), // lightest green
        Debug => (Color::Rgb { r: 102, g: 204, b: 102 }, false), // light green
        Info => (Color::Rgb { r: 0, g: 180, b: 0 }, false),       // green
        Warn => (Color::Rgb { r: 255, g: 90, b: 90 }, false),     // red
        Error => (Color::Rgb { r: 204, g: 0, b: 0 }, false),      // dark red
        Fatal => (Color::Rgb { r: 140, g: 0, b: 0 }, true),       // deepest red, bold
    }
}

// ---------------------------------------------------------------------------
// Log levels
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

/// Case-insensitive level keyword lookup (with aliases).
fn level_from_str(s: &str) -> Option<Level> {
    let t = s.trim().to_ascii_lowercase();
    match t.as_str() {
        "trace" => Some(Level::Trace),
        "debug" => Some(Level::Debug),
        "info" | "notice" => Some(Level::Info),
        "warn" | "warning" => Some(Level::Warn),
        "error" | "err" => Some(Level::Error),
        "fatal" | "critical" | "crit" | "alert" | "emerg" | "severe" => Some(Level::Fatal),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Timestamp parsing
// ---------------------------------------------------------------------------

/// A timestamp found in a log line (byte offsets refer to the original line).
#[derive(Clone, Copy, Debug)]
struct TimeMatch {
    start: usize,
    end: usize,
    ts: DateTime<FixedOffset>,
}

/// Raw time parse result, offsets relative to the scanned string.
struct RawTime {
    end: usize,
    ts: DateTime<FixedOffset>,
}

/// Parsing context so year-less timestamps are deterministic in tests.
struct TimeCtx {
    now: DateTime<Local>,
}

impl TimeCtx {
    fn new() -> TimeCtx {
        TimeCtx { now: Local::now() }
    }
    #[cfg(test)]
    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> TimeCtx {
        TimeCtx {
            now: Local.with_ymd_and_hms(y, mo, d, h, mi, s).single().unwrap(),
        }
    }
}

/// Build a fixed-offset DateTime from a naive local time (DST-gap fallback: UTC).
fn local_datetime(date: NaiveDate, time: NaiveTime) -> DateTime<FixedOffset> {
    let naive = NaiveDateTime::new(date, time);
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt.fixed_offset(),
        LocalResult::Ambiguous(a, _) => a.fixed_offset(),
        LocalResult::None => FixedOffset::east_opt(0).unwrap().from_utc_datetime(&naive),
    }
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

fn month_from_str(s: &str) -> Option<u32> {
    let t = s.get(..3)?.to_ascii_lowercase();
    MONTHS.iter().position(|&m| m == t).map(|i| i as u32 + 1)
}

/// Leading ASCII digits → (value, bytes consumed).
fn parse_num(s: &str) -> Option<(u64, usize)> {
    let n = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    if n == 0 {
        return None;
    }
    let v = s[..n].parse().ok()?;
    Some((v, n))
}

/// `YYYY-MM-DD` → (date, bytes consumed).
fn parse_iso_date(s: &str) -> Option<(NaiveDate, usize)> {
    if s.len() < 10 {
        return None;
    }
    let b = s.as_bytes();
    if b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y: u32 = s[0..4].parse().ok()?;
    let m: u32 = s[5..7].parse().ok()?;
    let d: u32 = s[8..10].parse().ok()?;
    let date = NaiveDate::from_ymd_opt(y as i32, m, d)?;
    Some((date, 10))
}

/// `HH:MM[:SS[.fff|,fff]]` → (time, bytes consumed).
fn parse_clock(s: &str) -> Option<(NaiveTime, usize)> {
    if s.len() < 5 {
        return None;
    }
    let b = s.as_bytes();
    let h: u32 = s[0..2].parse().ok()?;
    if b[2] != b':' {
        return None;
    }
    let mi: u32 = s[3..5].parse().ok()?;
    let mut end = 5;
    let mut sec = 0u32;
    let mut nano = 0u32;
    if s.len() > end && b[end] == b':' {
        let (sv, n) = parse_num(&s[end + 1..])?;
        sec = sv as u32;
        end += 1 + n;
        if s.len() > end && (b[end] == b'.' || b[end] == b',') {
            let (fv, fn_) = parse_num(&s[end + 1..])?;
            if fn_ > 0 {
                nano = if fn_ >= 9 {
                    (fv / 10u64.pow((fn_ - 9) as u32)) as u32
                } else {
                    (fv * 10u64.pow((9 - fn_) as u32)) as u32
                };
                end += 1 + fn_;
            }
        }
    }
    let time = NaiveTime::from_hms_nano_opt(h, mi, sec, nano)?;
    Some((time, end))
}

/// Optional timezone suffix `Z | ±HH | ±HHMM | ±HH:MM` (leading space allowed).
fn parse_tz(s: &str) -> Option<(FixedOffset, usize)> {
    let t = s.trim_start();
    let lead = s.len() - t.len();
    if t.is_empty() {
        return None;
    }
    let b = t.as_bytes();
    if b[0] == b'Z' || b[0] == b'z' {
        return Some((FixedOffset::east_opt(0)?, lead + 1));
    }
    if b[0] != b'+' && b[0] != b'-' {
        return None;
    }
    let sign = if b[0] == b'-' { -1 } else { 1 };
    let rest = &t[1..];
    let (hv, n) = parse_num(rest)?;
    let mut extra = 0usize;
    let (hh, mm) = if n == 4 {
        (hv / 100, hv % 100)
    } else if n == 1 || n == 2 {
        let mut mm = 0;
        if rest.as_bytes().get(n as usize) == Some(&b':') {
            let (mv, mn) = parse_num(&rest[n as usize + 1..])?;
            if mn != 2 {
                return None;
            }
            mm = mv;
            extra = 1 + mn;
        }
        (hv, mm)
    } else {
        return None;
    };
    let secs = sign * (hh as i32 * 3600 + mm as i32 * 60);
    let off = FixedOffset::east_opt(secs)?;
    Some((off, lead + 1 + n + extra))
}

/// `YYYY-MM-DD[ T]HH:MM...` (ISO-like; `T` separator, `,` ms, timezone suffix).
fn parse_iso(s: &str, ctx: &TimeCtx) -> Option<RawTime> {
    let (date, n1) = parse_iso_date(s)?;
    let sep = *s.as_bytes().get(n1)?;
    if sep != b' ' && sep != b'T' && sep != b't' {
        return None;
    }
    let (time, n2) = parse_clock(&s[n1 + 1..])?;
    let mut end = n1 + 1 + n2;
    let mut ts = local_datetime(date, time);
    if let Some((off, n3)) = parse_tz(&s[end..]) {
        end += n3;
        ts = off.from_local_datetime(&NaiveDateTime::new(date, time)).single()?;
    }
    let _ = ctx;
    Some(RawTime { end, ts })
}

/// `YYYY/MM/DD ...`, `MM/DD/YYYY ...` (US), `DD/Mon/YYYY:...` (nginx).
fn parse_slash(s: &str, ctx: &TimeCtx) -> Option<RawTime> {
    let (p1, n1) = parse_num(s)?;
    if s.as_bytes().get(n1) != Some(&b'/') {
        return None;
    }
    let rest1 = &s[n1 + 1..];

    // Alpha month — nginx access log: `DD/Mon/YYYY:HH:MM:SS ±HHMM`.
    if rest1.bytes().next().map(|b| b.is_ascii_alphabetic()).unwrap_or(false) {
        let (mon, n2) = month_from_str(rest1).zip(Some(3))?;
        if rest1.as_bytes().get(n2) != Some(&b'/') {
            return None;
        }
        let rest2 = &rest1[n2 + 1..];
        let (y, n3) = parse_num(rest2)?;
        if n3 != 4 {
            return None;
        }
        let date = NaiveDate::from_ymd_opt(y as i32, mon, p1 as u32)?;
        let after = &rest2[n3..];
        let skip = match after.as_bytes().first() {
            Some(b':') | Some(b' ') | Some(b'T') | Some(b't') => 1,
            _ => return None,
        };
        let (time, n4) = parse_clock(&after[skip..])?;
        let mut end = n1 + 1 + n2 + 1 + n3 + skip + n4;
        let mut ts = local_datetime(date, time);
        if let Some((off, n5)) = parse_tz(&s[end..]) {
            end += n5;
            ts = off.from_local_datetime(&NaiveDateTime::new(date, time)).single()?;
        }
        let _ = ctx;
        return Some(RawTime { end, ts });
    }

    // Numeric second component.
    let (p2, n2) = parse_num(rest1)?;
    if rest1.as_bytes().get(n2) != Some(&b'/') {
        return None;
    }
    let rest2 = &rest1[n2 + 1..];
    let (p3, n3) = parse_num(rest2)?;
    let date_end = n1 + 1 + n2 + 1 + n3;
    let after = &s[date_end..];
    let (date, year_is_first) = if p1 >= 1000 {
        (NaiveDate::from_ymd_opt(p1 as i32, p2 as u32, p3 as u32)?, true)
    } else {
        // US convention: MM/DD/YYYY
        (NaiveDate::from_ymd_opt(p3 as i32, p1 as u32, p2 as u32)?, false)
    };
    let sep = after.as_bytes().first().copied()?;
    if sep != b' ' && sep != b'T' && sep != b't' && sep != b':' {
        return None;
    }
    let (time, n4) = parse_clock(&after[1..])?;
    let mut end = date_end + 1 + n4;
    let mut ts = local_datetime(date, time);
    if let Some((off, n5)) = parse_tz(&s[end..]) {
        end += n5;
        ts = off.from_local_datetime(&NaiveDateTime::new(date, time)).single()?;
    }
    let _ = (ctx, year_is_first);
    Some(RawTime { end, ts })
}

/// `Mon DD HH:MM:SS` (year-less syslog; current year, rolled back when the
/// result lands more than a day in the future).
fn parse_syslog(s: &str, ctx: &TimeCtx) -> Option<RawTime> {
    let (mon, n1) = month_from_str(s).zip(Some(3))?;
    if s.as_bytes().get(n1) != Some(&b' ') {
        return None;
    }
    let rest1 = &s[n1 + 1..];
    let (day, n2) = parse_num(rest1)?;
    if n2 > 2 {
        return None;
    }
    if rest1.as_bytes().get(n2) != Some(&b' ') {
        return None;
    }
    let (time, n3) = parse_clock(&rest1[n2 + 1..])?;
    let year = ctx.now.year();
    let date = NaiveDate::from_ymd_opt(year, mon, day as u32)?;
    let mut ts = local_datetime(date, time);
    if ts > ctx.now.fixed_offset() + chrono::Duration::days(1) {
        let prev = NaiveDate::from_ymd_opt(year - 1, mon, day as u32)?;
        ts = local_datetime(prev, time);
    }
    let end = n1 + 1 + n2 + 1 + n3;
    Some(RawTime { end, ts })
}

/// Try to parse a timestamp at `s[pos..]`.
fn parse_time_at(s: &str, pos: usize, ctx: &TimeCtx) -> Option<TimeMatch> {
    let rest = &s[pos..];
    let raw = parse_iso(rest, ctx)
        .or_else(|| parse_slash(rest, ctx))
        .or_else(|| parse_syslog(rest, ctx))?;
    Some(TimeMatch { start: pos, end: pos + raw.end, ts: raw.ts })
}

/// Locate a timestamp in a log line: preferred at the start (after leading
/// whitespace); otherwise scan the first `TIME_SCAN_LIMIT` bytes (covers
/// nginx access lines like `127.0.0.1 - - [15/Jan/2024:10:23:45 +0800] ...`).
fn find_time(line: &str, ctx: &TimeCtx) -> Option<TimeMatch> {
    let ws = line.len() - line.trim_start().len();
    if let Some(tm) = parse_time_at(line, ws, ctx) {
        return Some(tm);
    }
    let limit = line.len().min(TIME_SCAN_LIMIT);
    for i in ws..limit {
        let b = line.as_bytes()[i];
        if b.is_ascii_digit() || b.is_ascii_alphabetic() {
            if let Some(tm) = parse_time_at(line, i, ctx) {
                return Some(tm);
            }
        }
    }
    None
}

/// Parse a user-supplied `--newer` time: any log format, or
/// `YYYY-MM-DD [HH:MM[:SS]]` (default midnight).
fn parse_user_time(s: &str) -> Option<DateTime<FixedOffset>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let ctx = TimeCtx::new();
    if let Some(tm) = parse_time_at(s, 0, &ctx) {
        return Some(tm.ts);
    }
    let (date, n) = parse_iso_date(s)?;
    let mut end = n;
    let mut time = NaiveTime::from_hms_opt(0, 0, 0)?;
    if s.as_bytes().get(end) == Some(&b' ') || s.as_bytes().get(end) == Some(&b'T') {
        let (t, n2) = parse_clock(&s[end + 1..])?;
        time = t;
        end += 1 + n2;
    }
    let mut ts = local_datetime(date, time);
    if let Some((off, _n3)) = parse_tz(&s[end..]) {
        ts = off.from_local_datetime(&NaiveDateTime::new(date, time)).single()?;
    }
    Some(ts)
}

// ---------------------------------------------------------------------------
// Line analysis: colored spans + level + timestamp
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Span {
    text: String,
    color: Option<Color>,
    bold: bool,
}

#[derive(Clone, Debug)]
struct Entry {
    /// Original 1-based line number in the file.
    number: usize,
    spans: Vec<Span>,
    level: Option<Level>,
    ts: Option<DateTime<FixedOffset>>,
    /// Byte ranges of keyword hits inside the line text.
    matches: Vec<(usize, usize)>,
}

/// Painted range inside `rest` (relative offsets).
struct Paint {
    start: usize,
    end: usize,
    color: Option<Color>,
    bold: bool,
}

/// Classify the tokens right after the timestamp: an optional app marker (dark
/// yellow) and an optional level (gradient). Returns painted ranges and the
/// detected level.
fn classify_head(rest: &str) -> (Vec<Paint>, Option<Level>) {
    let mut paints: Vec<Paint> = Vec::new();
    let mut level = None;
    let mut app_set = false;
    let mut pos = 0usize;
    let bytes = rest.as_bytes();
    for _ in 0..5 {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let start = pos;
        if bytes[pos] == b'[' {
            match rest[start..].find(']') {
                Some(j) => pos = start + j + 1,
                None => break,
            }
        } else if bytes[pos] == b'"' || bytes[pos] == b'-' {
            break; // the message starts here
        } else {
            while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
        }
        let token = &rest[start..pos];
        let inner = token.trim_matches(|c| c == '[' || c == ']');
        let probe = inner.trim_matches(|c: char| matches!(c, ',' | ':' | ';' | '|' | ')' | '(' | '.' | ']' | '['));
        if let Some(lv) = level_from_str(probe) {
            let (color, bold) = level_color(lv);
            paints.push(Paint { start, end: pos, color: Some(color), bold });
            level = Some(lv);
            break;
        }
        if token.chars().all(|c| c.is_ascii_punctuation()) {
            continue; // skip stray punctuation such as `]`
        }
        if !app_set {
            paints.push(Paint { start, end: pos, color: Some(COLOR_APP), bold: false });
            app_set = true;
            continue;
        }
        break;
    }
    (paints, level)
}

/// Case-insensitive ASCII substring search → byte ranges (char-aligned).
fn find_ci(hay: &str, needle: &str) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let nchars: Vec<char> = needle.chars().collect();
    let hchars: Vec<(usize, char)> = hay.char_indices().collect();
    let mut out = Vec::new();
    if hchars.len() < nchars.len() {
        return out;
    }
    let mut i = 0usize;
    while i + nchars.len() <= hchars.len() {
        let mut ok = true;
        for (k, &nc) in nchars.iter().enumerate() {
            if !hchars[i + k].1.eq_ignore_ascii_case(&nc) {
                ok = false;
                break;
            }
        }
        if ok {
            let start = hchars[i].0;
            let last = hchars[i + nchars.len() - 1];
            let end = last.0 + last.1.len_utf8();
            out.push((start, end));
            i += nchars.len();
        } else {
            i += 1;
        }
    }
    out
}

struct Analyzer<'a> {
    ctx: TimeCtx,
    kw: Option<&'a str>,
}

// ---------------------------------------------------------------------------
// IP detection
// ---------------------------------------------------------------------------

/// Try to parse a validated IPv4 at `i` (`b[i]` is a digit); returns the byte
/// range. Every octet must be ≤ 255 and the token must be delimited by
/// non-IP characters, so `999.999.999.999`, `v1.2.3.4` and `1.2.3.4.5` never
/// match.
fn ipv4_at(s: &str, b: &[u8], i: usize) -> Option<(usize, usize)> {
    let mut j = i;
    for g in 0..4 {
        let d0 = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j == d0 || j - d0 > 3 {
            return None;
        }
        let octet: u16 = b[d0..j].iter().fold(0u16, |acc, &c| acc * 10 + (c - b'0') as u16);
        if octet > 255 {
            return None;
        }
        if g < 3 {
            if j >= b.len() || b[j] != b'.' {
                return None;
            }
            j += 1;
        }
    }
    let start_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'.');
    let end_ok = j >= b.len() || !(b[j].is_ascii_alphanumeric() || b[j] == b'.');
    if start_ok && end_ok {
        Some((i, j))
    } else {
        None
    }
}

/// Try to parse an IPv6 token starting at `i` (`b[i]` is a hex digit, `:` or
/// `.`). Conservative on purpose, so ordinary text is not over-painted:
///   - the token is a run of `[0-9A-Fa-f:.]` with at least one `:` and one hex
///     digit, and its first `:` within the first 5 chars;
///   - without `::` compression it must be a full 8-group address (7 colons) —
///     this excludes HH:MM:SS timestamps (2 colons) and MAC addresses (5
///     colons, no `::`);
///   - with `::` compression, group count is free (e.g. `::1`, `fe80::1`,
///     `2001:db8::1`);
///   - a dotted quad is allowed only as the trailing part (IPv4-mapped forms
///     like `::ffff:192.168.1.1`) and must itself be a valid IPv4;
///   - the token must not extend into a wider alphanumeric token.
fn ipv6_at(s: &str, b: &[u8], i: usize) -> Option<(usize, usize)> {
    let mut j = i;
    while j < b.len() && (b[j].is_ascii_hexdigit() || b[j] == b':' || b[j] == b'.') {
        j += 1;
    }
    let tok = &s[i..j];
    let colon_count = tok.bytes().filter(|&c| c == b':').count();
    if colon_count == 0 || tok.bytes().filter(|c| c.is_ascii_hexdigit()).count() == 0 {
        return None;
    }
    let compressed = tok.contains("::");
    if !compressed && colon_count != 7 {
        return None;
    }
    let first_colon = tok.find(':').unwrap();
    if first_colon > 4 {
        return None;
    }
    if let Some(dot) = tok.find('.') {
        // Dotted part must be the trailing IPv4 of an IPv4-mapped address.
        let tail = tok.rsplit_once(':').map(|(_, t)| t).unwrap_or(tok);
        if tail.matches('.').count() != 3 {
            return None;
        }
        let tail_start = tok.len() - tail.len();
        let tb = tok.as_bytes();
        let mut k = tail_start;
        let mut groups = 0usize;
        while k < tok.len() {
            let d0 = k;
            while k < tok.len() && tb[k].is_ascii_digit() {
                k += 1;
            }
            if k == d0 || k - d0 > 3 {
                return None;
            }
            let octet: u16 = tb[d0..k].iter().fold(0u16, |acc, &c| acc * 10 + (c - b'0') as u16);
            if octet > 255 {
                return None;
            }
            groups += 1;
            if k < tok.len() {
                if tb[k] != b'.' {
                    return None;
                }
                k += 1;
            }
        }
        if groups != 4 || tok[..tail_start].bytes().any(|c| c == b'.') {
            return None;
        }
        let _ = dot;
    }
    let start_ok = i == 0 || !(b[i - 1].is_ascii_hexdigit() || b[i - 1] == b':' || b[i - 1] == b'.');
    let end_ok = j >= b.len() || !(b[j].is_ascii_hexdigit() || b[j] == b':' || b[j] == b'.');
    if start_ok && end_ok {
        Some((i, j))
    } else {
        None
    }
}

/// Byte ranges of IP addresses (IPv4 or IPv6) inside `s`, in order.
fn find_ips(s: &str) -> Vec<(usize, usize)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            if let Some((st, en)) = ipv4_at(s, b, i) {
                out.push((st, en));
                i = en;
                continue;
            }
        }
        if b[i].is_ascii_hexdigit() || b[i] == b':' {
            if let Some((st, en)) = ipv6_at(s, b, i) {
                out.push((st, en));
                i = en;
                continue;
            }
        }
        i += 1;
    }
    out
}

impl Analyzer<'_> {
    fn entry(&self, number: usize, line: &str) -> Entry {
        let (spans, level, ts) = if let Some(tm) = find_time(line, &self.ctx) {
            let mut spans = Vec::new();
            if tm.start > 0 {
                spans.push(Span {
                    text: line[..tm.start].to_string(),
                    color: None,
                    bold: false,
                });
            }
            spans.push(Span {
                text: line[tm.start..tm.end].to_string(),
                color: Some(COLOR_TIME),
                bold: false,
            });
            let rest = &line[tm.end..];
            let (paints, lv) = classify_head(rest);
            let mut cursor = 0usize;
            for p in paints {
                if p.start > cursor {
                    spans.push(Span {
                        text: rest[cursor..p.start].to_string(),
                        color: None,
                        bold: false,
                    });
                }
                spans.push(Span {
                    text: rest[p.start..p.end].to_string(),
                    color: p.color,
                    bold: p.bold,
                });
                cursor = p.end;
            }
            if cursor < rest.len() {
                spans.push(Span {
                    text: rest[cursor..].to_string(),
                    color: None,
                    bold: false,
                });
            }
            (spans, lv, Some(tm.ts))
        } else {
            // No timestamp: still detect a leading level (e.g. "ERROR: ...") so
            // `--level` filters bare lines too; no coloring (except IPs below).
            let (_, lv) = classify_head(line.trim_start());
            (
                vec![Span {
                    text: line.to_string(),
                    color: None,
                    bold: false,
                }],
                lv,
                None,
            )
        };
        // Highlight IP addresses (IPv4 / IPv6) in every span — head and message
        // alike, including lines without a timestamp. Each IP sub-range is
        // painted blue; the surrounding text keeps the span's original color.
        let mut spans = spans;
        let mut ip_spans = Vec::with_capacity(spans.len());
        for s in spans {
            let hits = find_ips(&s.text);
            if hits.is_empty() {
                ip_spans.push(s);
                continue;
            }
            let mut cur = 0usize;
            for (hs, he) in hits {
                if hs > cur {
                    ip_spans.push(Span {
                        text: s.text[cur..hs].to_string(),
                        color: s.color,
                        bold: s.bold,
                    });
                }
                ip_spans.push(Span {
                    text: s.text[hs..he].to_string(),
                    color: Some(COLOR_IP),
                    bold: false,
                });
                cur = he;
            }
            if cur < s.text.len() {
                ip_spans.push(Span {
                    text: s.text[cur..].to_string(),
                    color: s.color,
                    bold: s.bold,
                });
            }
        }
        let spans = ip_spans;
        let matches = match self.kw {
            Some(k) if !k.is_empty() => find_ci(line, k),
            _ => Vec::new(),
        };
        Entry { number, spans, level, ts, matches }
    }
}

// ---------------------------------------------------------------------------
// Filtering
// ---------------------------------------------------------------------------

struct Filters {
    level: Option<Level>,
    line_from: Option<usize>,
    newer: Option<DateTime<FixedOffset>>,
}

fn passes(f: &Filters, e: &Entry) -> bool {
    if let Some(lf) = f.line_from {
        if e.number < lf {
            return false;
        }
    }
    if let Some(lv) = f.level {
        if e.level != Some(lv) {
            return false;
        }
    }
    if let Some(nt) = f.newer {
        if let Some(ts) = e.ts {
            if ts < nt {
                return false;
            }
        }
        // untimestamped continuation lines are kept
    }
    true
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Split bytes into lines (CRLF tolerated, UTF-8 lossy, leading BOM stripped).
fn split_lines(data: &[u8]) -> Vec<String> {
    if data.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for chunk in data.split(|&b| b == b'\n') {
        let chunk = if chunk.ends_with(b"\r") { &chunk[..chunk.len() - 1] } else { chunk };
        let mut s = String::from_utf8_lossy(chunk).into_owned();
        if out.is_empty() {
            s = s.trim_start_matches('\u{feff}').to_string();
        }
        out.push(s);
    }
    if data.ends_with(b"\n") {
        out.pop();
    }
    out
}

/// Split complete newline-terminated lines out of `buf`, keeping the trailing
/// partial line in `buf` (for follow mode). CRLF tolerated.
fn drain_lines(buf: &mut Vec<u8>) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for i in 0..buf.len() {
        if buf[i] == b'\n' {
            let mut end = i;
            if end > start && buf[end - 1] == b'\r' {
                end -= 1;
            }
            let mut s = String::from_utf8_lossy(&buf[start..end]).into_owned();
            if out.is_empty() {
                s = s.trim_start_matches('\u{feff}').to_string();
            }
            out.push(s);
            start = i + 1;
        }
    }
    buf.drain(..start);
    out
}

/// Append the freshly read bytes of a growing/rotated file to `buf`.
/// Returns the number of new bytes (0 = nothing new).
fn read_new(file: &mut File, pos: &mut u64, buf: &mut Vec<u8>) -> io::Result<usize> {
    let len = file.metadata()?.len();
    if len < *pos {
        // truncated / rotated — restart from the beginning
        *pos = 0;
        buf.clear();
        file.seek(SeekFrom::Start(0))?;
    }
    if len == *pos {
        return Ok(0);
    }
    file.seek(SeekFrom::Start(*pos))?;
    let want = (len - *pos).min(1 << 16) as usize;
    let mut chunk = vec![0u8; want];
    let n = file.read(&mut chunk)?;
    *pos += n as u64;
    buf.extend_from_slice(&chunk[..n]);
    Ok(n)
}

// ---------------------------------------------------------------------------
// Store + session
// ---------------------------------------------------------------------------

/// The analyzed, filtered entries plus the search match list.
struct Store {
    entries: Vec<Entry>,
    matches: Vec<usize>,
    next_number: usize,
}

impl Store {
    fn new() -> Store {
        Store { entries: Vec::new(), matches: Vec::new(), next_number: 1 }
    }
    fn push_raw(&mut self, lines: Vec<String>, analyzer: &Analyzer, filters: &Filters) {
        for l in lines {
            let e = analyzer.entry(self.next_number, &l);
            self.next_number += 1;
            if passes(filters, &e) {
                if analyzer.kw.is_some() && !e.matches.is_empty() {
                    self.matches.push(self.entries.len());
                }
                self.entries.push(e);
            }
        }
    }
}

struct FollowSource {
    file: File,
    pos: u64,
}

struct Session {
    top: usize,
    pinned: bool,
    match_pos: Option<usize>,
    follow: bool,
}

// ---------------------------------------------------------------------------
// Pure view helpers (unit-testable)
// ---------------------------------------------------------------------------

fn visible_height(total_rows: usize) -> usize {
    total_rows.saturating_sub(1).max(1)
}

fn clamp_top(top: usize, height: usize, total: usize) -> usize {
    if total <= height {
        return 0;
    }
    top.min(total - height)
}

fn next_match_idx(cur: Option<usize>, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match cur {
        None => 0,
        Some(i) => (i + 1) % count,
    })
}

fn prev_match_idx(cur: Option<usize>, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match cur {
        None => count - 1,
        Some(i) => (i + count - 1) % count,
    })
}

/// Approximate display width (CJK and friends count as 2 columns).
fn display_cols(s: &str) -> usize {
    s.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum()
}

/// Byte length of the longest prefix fitting `budget` display columns.
fn cols_fit(s: &str, budget: usize) -> usize {
    let mut col = 0usize;
    let mut idx = 0usize;
    for (i, c) in s.char_indices() {
        let w = if c.is_ascii() { 1 } else { 2 };
        if col + w > budget {
            break;
        }
        col += w;
        idx = i + c.len_utf8();
    }
    idx
}

fn truncate_display(s: &str, width: usize) -> String {
    let mut col = 0usize;
    let mut out = String::new();
    for c in s.chars() {
        let w = if c.is_ascii() { 1 } else { 2 };
        if col + w > width {
            break;
        }
        col += w;
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn draw_entry(stdout: &mut impl Write, e: &Entry, show_lineno: bool, width: usize) -> io::Result<()> {
    let mut col = 0usize;
    let mut styled = false;
    if show_lineno {
        let s = format!("{:>6} ", e.number);
        execute!(
            stdout,
            SetForegroundColor(COLOR_LINENO),
            Print(&s),
            SetForegroundColor(Color::Reset)
        )?;
        col += 7;
    }
    let mut line_off = 0usize;
    for span in &e.spans {
        let s0 = line_off;
        let s1 = line_off + span.text.len();
        // Split the span into normal / highlighted parts.
        let mut parts: Vec<(bool, &str)> = Vec::new();
        if e.matches.is_empty() {
            parts.push((false, span.text.as_str()));
        } else {
            let mut cur = 0usize;
            for (ms, me) in &e.matches {
                let ls = ms.saturating_sub(s0).min(span.text.len());
                let le = me.saturating_sub(s0).min(span.text.len());
                if le <= ls {
                    continue;
                }
                if ls > cur {
                    parts.push((false, &span.text[cur..ls]));
                }
                parts.push((true, &span.text[ls..le]));
                cur = le;
            }
            if cur < span.text.len() {
                parts.push((false, &span.text[cur..]));
            }
        }
        for (hl, text) in parts {
            if col >= width {
                break;
            }
            let take = cols_fit(text, width - col);
            if take == 0 {
                break;
            }
            let text = &text[..take];
            if let Some(c) = span.color {
                execute!(stdout, SetForegroundColor(c))?;
                styled = true;
            }
            if span.bold {
                execute!(stdout, SetAttribute(Attribute::Bold))?;
                styled = true;
            }
            if hl {
                execute!(stdout, SetAttribute(Attribute::Reverse))?;
                styled = true;
            }
            write!(stdout, "{text}")?;
            if styled {
                execute!(stdout, SetAttribute(Attribute::Reset), SetForegroundColor(Color::Reset))?;
                styled = false;
            }
            col += display_cols(text);
        }
        if col >= width {
            break;
        }
        line_off = s1;
    }
    Ok(())
}

fn status_line(file_name: &str, store: &Store, sess: &Session, top: usize, height: usize, total: usize, lv: Option<Level>, kw: Option<&str>) -> String {
    let mut s = format!(" {}  L{}-{} / {}  ", file_name, top + 1, (top + height).min(total), total);
    if let Some(lv) = lv {
        s.push_str(&format!("level={:?}  ", lv));
    }
    if let Some(kw) = kw {
        let mpos = sess.match_pos.map(|i| i + 1).unwrap_or(0);
        s.push_str(&format!("\"{}\" {}/{}  ", kw, mpos, store.matches.len()));
    }
    if sess.follow {
        s.push_str("following...  ");
    }
    s.push_str("q=quit PgUp/PgDn=page \u{2191}\u{2193}=line n/u=search");
    s
}

fn draw(
    stdout: &mut io::Stdout,
    store: &Store,
    sess: &Session,
    file_name: &str,
    lv: Option<Level>,
    kw: Option<&str>,
    width: usize,
    height: usize,
) -> Result<()> {
    let total = store.entries.len();
    let top = clamp_top(sess.top, height, total);
    let show_lineno = kw.is_some();
    for row in 0..height {
        let idx = top + row;
        execute!(stdout, cursor::MoveTo(0, row as u16), terminal::Clear(ClearType::CurrentLine))?;
        if idx < total {
            draw_entry(stdout, &store.entries[idx], show_lineno, width)?;
        }
    }
    execute!(stdout, cursor::MoveTo(0, height as u16), terminal::Clear(ClearType::CurrentLine))?;
    let status = status_line(file_name, store, sess, top, height, total, lv, kw);
    execute!(stdout, SetAttribute(Attribute::Reverse))?;
    write!(stdout, "{}", truncate_display(&status, width))?;
    execute!(stdout, SetAttribute(Attribute::Reset))?;
    stdout.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive / plain output
// ---------------------------------------------------------------------------

fn run_tty(
    mut store: Store,
    mut source: Option<FollowSource>,
    analyzer: &Analyzer,
    filters: &Filters,
    file_name: &str,
    lv: Option<Level>,
    kw: Option<&str>,
) -> Result<()> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    let _ = execute!(stdout, EnterAlternateScreen, cursor::Hide);
    let res = tty_loop(&mut store, &mut source, analyzer, filters, file_name, lv, kw, &mut stdout);
    let _ = execute!(stdout, LeaveAlternateScreen, cursor::Show);
    let _ = terminal::disable_raw_mode();
    res
}

fn tty_loop(
    store: &mut Store,
    source: &mut Option<FollowSource>,
    analyzer: &Analyzer,
    filters: &Filters,
    file_name: &str,
    lv: Option<Level>,
    kw: Option<&str>,
    stdout: &mut io::Stdout,
) -> Result<()> {
    let mut sess = Session { top: usize::MAX, pinned: true, match_pos: None, follow: source.is_some() };
    let mut need_draw = true;
    loop {
        let (w, h) = terminal::size().unwrap_or((80, 24));
        let height = visible_height(h as usize);
        let total = store.entries.len();
        let max_top = total.saturating_sub(height);
        let top = clamp_top(sess.top, height, total);

        // Follow: pick up appended lines.
        if let Some(src) = source.as_mut() {
            let mut buf = Vec::new();
            if read_new(&mut src.file, &mut src.pos, &mut buf)? > 0 {
                let lines = drain_lines(&mut buf);
                store.push_raw(lines, analyzer, filters);
                if sess.pinned {
                    sess.top = clamp_top(usize::MAX, height, store.entries.len());
                }
                need_draw = true;
            }
        }

        if need_draw {
            draw(stdout, store, &sess, file_name, lv, kw, w as usize, height)?;
            need_draw = false;
        }

        let timeout = if sess.follow { Duration::from_millis(FOLLOW_POLL_MS) } else { Duration::from_millis(500) };
        if !event::poll(timeout)? {
            continue;
        }
        match event::read()? {
            Event::Key(ke) => match ke.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('c') if ke.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                KeyCode::PageUp => {
                    sess.top = top.saturating_sub(height);
                    sess.pinned = false;
                    need_draw = true;
                }
                KeyCode::PageDown => {
                    sess.top = (top + height).min(max_top);
                    if sess.top >= max_top {
                        sess.pinned = true;
                    }
                    need_draw = true;
                }
                KeyCode::Up => {
                    if top > 0 {
                        sess.top = top - 1;
                        sess.pinned = false;
                    }
                    need_draw = true;
                }
                KeyCode::Down => {
                    sess.top = (top + 1).min(max_top);
                    if sess.top >= max_top {
                        sess.pinned = true;
                    }
                    need_draw = true;
                }
                KeyCode::Home | KeyCode::Char('g') => {
                    sess.top = 0;
                    sess.pinned = false;
                    need_draw = true;
                }
                KeyCode::End | KeyCode::Char('G') => {
                    sess.top = max_top;
                    sess.pinned = true;
                    need_draw = true;
                }
                KeyCode::Char('n') => {
                    if let Some(next) = next_match_idx(sess.match_pos, store.matches.len()) {
                        sess.match_pos = Some(next);
                        if let Some(idx) = store.matches.get(next).copied() {
                            sess.top = idx.saturating_sub(height / 2);
                        }
                        need_draw = true;
                    }
                }
                KeyCode::Char('u') => {
                    if let Some(prev) = prev_match_idx(sess.match_pos, store.matches.len()) {
                        sess.match_pos = Some(prev);
                        if let Some(idx) = store.matches.get(prev).copied() {
                            sess.top = idx.saturating_sub(height / 2);
                        }
                        need_draw = true;
                    }
                }
                _ => {}
            },
            Event::Resize(_, _) => need_draw = true,
            _ => {}
        }
    }
}

fn print_plain(store: &Store, show_lineno: bool) -> Result<()> {
    let out = io::stdout();
    let mut w = io::BufWriter::new(out.lock());
    for e in &store.entries {
        if show_lineno {
            write!(w, "{:>6} ", e.number)?;
        }
        for span in &e.spans {
            write!(w, "{}", span.text)?;
        }
        writeln!(w)?;
    }
    w.flush()?;
    Ok(())
}

fn run_follow_plain(
    mut store: Store,
    mut source: FollowSource,
    analyzer: &Analyzer,
    filters: &Filters,
) -> Result<()> {
    let out = io::stdout();
    let mut w = io::BufWriter::new(out.lock());
    let show_lineno = analyzer.kw.is_some();
    for e in &store.entries {
        if show_lineno {
            write!(w, "{:>6} ", e.number)?;
        }
        for span in &e.spans {
            write!(w, "{}", span.text)?;
        }
        writeln!(w)?;
    }
    w.flush()?;
    loop {
        let mut buf = Vec::new();
        if read_new(&mut source.file, &mut source.pos, &mut buf)? > 0 {
            let lines = drain_lines(&mut buf);
            let before = store.entries.len();
            store.push_raw(lines, analyzer, filters);
            for e in &store.entries[before..] {
                if show_lineno {
                    write!(w, "{:>6} ", e.number)?;
                }
                for span in &e.spans {
                    write!(w, "{}", span.text)?;
                }
                writeln!(w)?;
            }
            w.flush()?;
        } else {
            std::thread::sleep(Duration::from_millis(FOLLOW_POLL_MS));
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub(crate) fn cmd_log(
    file: Option<&Path>,
    level: Option<&str>,
    search: Option<&str>,
    line: Option<usize>,
    newer: Option<&str>,
    follow: bool,
) -> Result<()> {
    let is_stdin = file.map(|f| f == Path::new("-")).unwrap_or(true);
    if follow && is_stdin {
        bail!("-f/--follow requires a real file (stdin cannot be followed)");
    }
    let kw = search.filter(|s| !s.is_empty());
    let lv = match level {
        Some(s) => Some(level_from_str(s).ok_or_else(|| {
            anyhow::anyhow!("invalid --level {s:?} (try trace|debug|info|warn|warning|error|fatal)")
        })?),
        None => None,
    };
    let nt = match newer {
        Some(s) => Some(parse_user_time(s).ok_or_else(|| {
            anyhow::anyhow!("cannot parse --newer time {s:?} (e.g. 2024-01-15 10:23:45)")
        })?),
        None => None,
    };
    let filters = Filters { level: lv, line_from: line, newer: nt };
    let analyzer = Analyzer { ctx: TimeCtx::new(), kw };
    let tty = io::stdout().is_terminal();

    match file {
        Some(f) if !is_stdin => {
            let mut fh = File::open(f).with_context(|| format!("cannot open {}", f.display()))?;
            if follow {
                let mut pending = Vec::new();
                fh.read_to_end(&mut pending)?;
                let pos = pending.len() as u64;
                let mut store = Store::new();
                store.push_raw(drain_lines(&mut pending), &analyzer, &filters);
                let source = FollowSource { file: fh, pos };
                if tty {
                    run_tty(store, Some(source), &analyzer, &filters, &f.display().to_string(), lv, kw)
                } else {
                    run_follow_plain(store, source, &analyzer, &filters)
                }
            } else {
                let mut data = Vec::new();
                fh.read_to_end(&mut data)?;
                let mut store = Store::new();
                store.push_raw(split_lines(&data), &analyzer, &filters);
                if tty {
                    run_tty(store, None, &analyzer, &filters, &f.display().to_string(), lv, kw)
                } else {
                    print_plain(&store, kw.is_some())
                }
            }
        }
        _ => {
            let mut data = Vec::new();
            io::stdin().read_to_end(&mut data)?;
            let mut store = Store::new();
            store.push_raw(split_lines(&data), &analyzer, &filters);
            if tty {
                run_tty(store, None, &analyzer, &filters, "-", lv, kw)
            } else {
                print_plain(&store, kw.is_some())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn tm(line: &str, ctx: &TimeCtx) -> Option<TimeMatch> {
        find_time(line, ctx)
    }

    #[test]
    fn parse_iso_dash_basic() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let m = tm("2024-01-15 10:23:45 INFO msg", &ctx).unwrap();
        assert_eq!(m.start, 0);
        assert_eq!(m.end, 19);
        assert_eq!(m.ts.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-01-15 10:23:45");
    }

    #[test]
    fn parse_iso_t_with_ms_and_z() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let m = tm("2024-01-15T10:23:45.123Z done", &ctx).unwrap();
        assert_eq!(m.end, 24);
        assert_eq!(m.ts.to_rfc3339(), "2024-01-15T10:23:45.123+00:00");
    }

    #[test]
    fn parse_log4j_comma_ms() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let m = tm("2024-01-15 10:23:45,123 [main] INFO x - y", &ctx).unwrap();
        assert_eq!(m.ts.format("%Y-%m-%d %H:%M:%S%.3f").to_string(), "2024-01-15 10:23:45.123");
    }

    #[test]
    fn parse_tz_compact() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let m = tm("2024-01-15T10:23:45+0800 end", &ctx).unwrap();
        assert_eq!(m.ts.offset().local_minus_utc(), 8 * 3600);
        assert_eq!(m.ts.to_rfc3339(), "2024-01-15T10:23:45+08:00");
    }

    #[test]
    fn parse_slash_year_first() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let m = tm("2024/01/15 10:23:45 [error] boom", &ctx).unwrap();
        assert_eq!(m.ts.format("%Y-%m-%d").to_string(), "2024-01-15");
    }

    #[test]
    fn parse_nginx_access_bracket() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let line = r#"127.0.0.1 - - [15/Jan/2024:10:23:45 +0800] "GET / HTTP/1.1" 200"#;
        let m = tm(line, &ctx).unwrap();
        assert_eq!(m.start, 15);
        assert_eq!(m.ts.offset().local_minus_utc(), 8 * 3600);
        assert_eq!(m.ts.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-01-15 10:23:45");
        assert_eq!(&line[m.start..m.end], "15/Jan/2024:10:23:45 +0800");
        // the prefix before an in-line timestamp must not be dropped
        let a = Analyzer { ctx, kw: None };
        let e = a.entry(1, line);
        let joined: String = e.spans.iter().map(|s| s.text.clone()).collect();
        assert_eq!(joined, line);
        // the client IP is highlighted blue, the rest of the prefix stays plain
        assert!(e.spans.iter().any(|s| s.text == "127.0.0.1" && s.color == Some(COLOR_IP)));
        assert!(e.spans.iter().any(|s| s.text == " - - [" && s.color.is_none()));
    }

    #[test]
    fn parse_us_style() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let m = tm("01/15/2024 10:23:45 msg", &ctx).unwrap();
        assert_eq!(m.ts.format("%Y-%m-%d").to_string(), "2024-01-15");
    }

    #[test]
    fn parse_syslog_yearless_current_year() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let m = tm("Jan 15 10:23:45 myhost app: msg", &ctx).unwrap();
        assert_eq!(m.ts.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-01-15 10:23:45");
    }

    #[test]
    fn parse_syslog_yearless_rolls_back() {
        let ctx = TimeCtx::at(2026, 1, 5, 12, 0, 0);
        let m = tm("Dec 30 10:00:00 host x: y", &ctx).unwrap();
        assert_eq!(m.ts.format("%Y-%m-%d").to_string(), "2025-12-30");
    }

    #[test]
    fn parse_no_time() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        assert!(tm("hello world 42", &ctx).is_none());
        assert!(tm("  \t plain line", &ctx).is_none());
    }

    #[test]
    fn parse_invalid_date_rejected() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        assert!(tm("2024-02-30 10:23:45 x", &ctx).is_none());
        assert!(tm("2024-13-01 10:23:45 x", &ctx).is_none());
    }

    #[test]
    fn parse_user_time_variants() {
        let midnight = parse_user_time("2024-01-15").unwrap();
        assert_eq!(midnight.format("%H:%M:%S").to_string(), "00:00:00");
        let full = parse_user_time("2024-01-15 10:23:45").unwrap();
        assert_eq!(full.format("%H:%M:%S").to_string(), "10:23:45");
        let iso = parse_user_time("2024-01-15T10:23:45.123Z").unwrap();
        assert_eq!(iso.to_rfc3339(), "2024-01-15T10:23:45.123+00:00");
        assert!(parse_user_time("not-a-time").is_none());
    }

    #[test]
    fn level_after_time_span_colors() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let a = Analyzer { ctx, kw: None };
        let e = a.entry(1, "2024-01-15 10:23:45 ERROR boom");
        assert_eq!(e.level, Some(Level::Error));
        assert_eq!(e.spans[0].color, Some(COLOR_TIME));
        assert!(e.spans.iter().any(|s| s.text == "ERROR" && s.color == Some(level_color(Level::Error).0)));
        // spans cover the whole line
        let joined: String = e.spans.iter().map(|s| s.text.clone()).collect();
        assert_eq!(joined, "2024-01-15 10:23:45 ERROR boom");
    }

    #[test]
    fn level_in_brackets() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let a = Analyzer { ctx, kw: None };
        let e = a.entry(1, "2024-01-15 10:23:45 [WARN] hot");
        assert_eq!(e.level, Some(Level::Warn));
        assert!(e.spans.iter().any(|s| s.text == "[WARN]" && s.color == Some(level_color(Level::Warn).0)));
    }

    #[test]
    fn app_then_level_log4j_layout() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let a = Analyzer { ctx, kw: None };
        let line = "2024-01-15 10:23:45,123 [http-nio-8080-exec-1] INFO  com.foo.App - msg";
        let e = a.entry(1, line);
        assert_eq!(e.level, Some(Level::Info));
        let joined: String = e.spans.iter().map(|s| s.text.clone()).collect();
        assert_eq!(joined, line);
        // [thread] painted blue, INFO painted level color
        assert!(e.spans.iter().any(|s| s.text == "[http-nio-8080-exec-1]" && s.color == Some(COLOR_APP)));
        assert!(e.spans.iter().any(|s| s.text == "INFO" && s.color == Some(level_color(Level::Info).0)));
    }

    #[test]
    fn level_gradient_colors() {
        let cases = [
            (Level::Trace, (178u8, 230u8, 178u8), false),
            (Level::Debug, (102u8, 204u8, 102u8), false),
            (Level::Info, (0u8, 180u8, 0u8), false),
            (Level::Warn, (255u8, 90u8, 90u8), false),
            (Level::Error, (204u8, 0u8, 0u8), false),
            (Level::Fatal, (140u8, 0u8, 0u8), true),
        ];
        for (lv, (r, g, b), bold) in cases {
            let (color, is_bold) = level_color(lv);
            assert_eq!(color, Color::Rgb { r, g, b });
            assert_eq!(is_bold, bold);
        }
        // green lightens toward the bottom, red darkens toward the top
        assert!(level_color(Level::Trace).0 != level_color(Level::Debug).0);
        assert!(level_color(Level::Debug).0 != level_color(Level::Info).0);
    }

    #[test]
    fn no_time_level_detected_for_filter() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let a = Analyzer { ctx, kw: None };
        let e = a.entry(1, "ERROR: boom");
        assert_eq!(e.level, Some(Level::Error));
        assert_eq!(e.spans.len(), 1); // no coloring without a timestamp (unless an IP is present)
        assert!(e.ts.is_none());
    }

    #[test]
    fn syslog_app_marker() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let a = Analyzer { ctx, kw: None };
        let e = a.entry(1, "Jan 15 10:23:45 myhost app[123]: msg");
        assert!(e.spans.iter().any(|s| s.text == "myhost" && s.color == Some(COLOR_APP)));
    }

    #[test]
    fn ipv4_in_message_highlighted() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let a = Analyzer { ctx, kw: None };
        let line = "2024-01-15 10:23:45 ERROR connection from 192.168.1.100 refused";
        let e = a.entry(1, line);
        assert!(e.spans.iter().any(|s| s.text == "192.168.1.100" && s.color == Some(COLOR_IP)));
        // non-IP parts of the message keep their original (plain) color
        assert!(e.spans.iter().any(|s| s.text == " connection from " && s.color.is_none()));
        let joined: String = e.spans.iter().map(|s| s.text.clone()).collect();
        assert_eq!(joined, line);
    }

    #[test]
    fn ip_highlighted_without_timestamp() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let a = Analyzer { ctx, kw: None };
        let e = a.entry(1, "connect 10.0.0.1 timeout");
        assert_eq!(e.ts, None);
        let ip = e.spans.iter().find(|s| s.text == "10.0.0.1").expect("ip span");
        assert_eq!(ip.color, Some(COLOR_IP));
        let joined: String = e.spans.iter().map(|s| s.text.clone()).collect();
        assert_eq!(joined, "connect 10.0.0.1 timeout");
    }

    #[test]
    fn ipv6_highlighted() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let a = Analyzer { ctx, kw: None };
        for (line, ip) in [
            ("connect ::1 ok", "::1"),
            ("2001:db8::1 refused", "2001:db8::1"),
            ("fe80::aabb:ccdd link", "fe80::aabb:ccdd"),
            ("::ffff:192.168.1.1 mapped", "::ffff:192.168.1.1"),
        ] {
            let e = a.entry(1, line);
            assert!(e.spans.iter().any(|s| s.text == ip && s.color == Some(COLOR_IP)), "line: {line}");
            let joined: String = e.spans.iter().map(|s| s.text.clone()).collect();
            assert_eq!(joined, line);
        }
    }

    #[test]
    fn non_ip_tokens_not_highlighted() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let a = Analyzer { ctx, kw: None };
        let line = "2024-01-15 10:23:45 INFO v1.2.3 12:34:56 999.999.999.999 mac 00:1a:2b:3c:4d:5e 1.2.3.4.5 done";
        let e = a.entry(1, line);
        assert!(
            e.spans.iter().all(|s| s.color != Some(COLOR_IP)),
            "no IP spans expected: {:?}",
            e.spans
        );
        let joined: String = e.spans.iter().map(|s| s.text.clone()).collect();
        assert_eq!(joined, line);
    }

    #[test]
    fn filter_by_level() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let a = Analyzer { ctx, kw: None };
        let lines = vec![
            "2024-01-15 10:23:45 INFO a".to_string(),
            "2024-01-15 10:23:46 ERROR b".to_string(),
            "plain continuation".to_string(),
            "2024-01-15 10:23:47 DEBUG c".to_string(),
        ];
        let mut store = Store::new();
        store.push_raw(lines, &a, &Filters { level: Some(Level::Error), line_from: None, newer: None });
        assert_eq!(store.entries.len(), 1);
        assert_eq!(store.entries[0].number, 2);
        assert_eq!(store.entries[0].level, Some(Level::Error));
    }

    #[test]
    fn filter_by_line_from() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let a = Analyzer { ctx, kw: None };
        let lines: Vec<String> = (1..=5).map(|i| format!("2024-01-15 10:23:{i:02} line{i}")).collect();
        let mut store = Store::new();
        store.push_raw(lines, &a, &Filters { level: None, line_from: Some(3), newer: None });
        let nums: Vec<usize> = store.entries.iter().map(|e| e.number).collect();
        assert_eq!(nums, vec![3, 4, 5]);
    }

    #[test]
    fn filter_by_newer_keeps_untagged() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let a = Analyzer { ctx, kw: None };
        let newer = parse_user_time("2024-01-15 10:24:00").unwrap();
        let lines = vec![
            "2024-01-15 10:23:00 early".to_string(),
            "2024-01-15 10:24:00 exact".to_string(),
            "2024-01-15 10:25:00 late".to_string(),
            "continuation without time".to_string(),
        ];
        let mut store = Store::new();
        store.push_raw(lines, &a, &Filters { level: None, line_from: None, newer: Some(newer) });
        let nums: Vec<usize> = store.entries.iter().map(|e| e.number).collect();
        assert_eq!(nums, vec![2, 3, 4]);
    }

    #[test]
    fn find_ci_offsets() {
        let hits = find_ci("Foo BAR foo end", "foo");
        assert_eq!(hits, vec![(0, 3), (8, 11)]);
        assert!(find_ci("no hits here", "zzz").is_empty());
        assert!(find_ci("anything", "").is_empty());
    }

    #[test]
    fn search_match_list_and_lineno() {
        let ctx = TimeCtx::at(2026, 6, 1, 12, 0, 0);
        let a = Analyzer { ctx, kw: Some("timeout") };
        let lines = vec![
            "2024-01-15 10:23:00 ok".to_string(),
            "2024-01-15 10:23:01 TIMEOUT after 3s".to_string(),
            "2024-01-15 10:23:02 retry ok".to_string(),
        ];
        let mut store = Store::new();
        store.push_raw(lines, &a, &Filters { level: None, line_from: None, newer: None });
        assert_eq!(store.matches, vec![1]);
        assert!(!store.entries[1].matches.is_empty());
        // line numbers are the original file numbers
        assert_eq!(store.entries[1].number, 2);
    }

    #[test]
    fn next_prev_match_navigation() {
        assert_eq!(next_match_idx(None, 3), Some(0));
        assert_eq!(next_match_idx(Some(0), 3), Some(1));
        assert_eq!(next_match_idx(Some(2), 3), Some(0)); // wraps
        assert_eq!(prev_match_idx(None, 3), Some(2));
        assert_eq!(prev_match_idx(Some(2), 3), Some(1));
        assert_eq!(prev_match_idx(Some(0), 3), Some(2)); // wraps
        assert_eq!(next_match_idx(None, 0), None);
        assert_eq!(prev_match_idx(None, 0), None);
    }

    #[test]
    fn drain_lines_keeps_partial() {
        let mut buf = b"a\nb\r\nc".to_vec();
        let lines = drain_lines(&mut buf);
        assert_eq!(lines, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(buf, b"c".to_vec());
        let more = drain_lines(&mut buf);
        assert!(more.is_empty());
        assert_eq!(buf, b"c".to_vec());
    }

    #[test]
    fn split_lines_edges() {
        assert!(split_lines(b"").is_empty());
        assert_eq!(split_lines(b"a\nb\n"), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(split_lines(b"a"), vec!["a".to_string()]);
        assert_eq!(split_lines(b"a\r\nb\r\n"), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(split_lines(b"\n"), vec!["".to_string()]);
    }

    #[test]
    fn read_new_appends_and_handles_truncation() {
        let base = std::env::temp_dir().join(format!("sys_log_readnew_{}", std::process::id()));
        fs_create_dir_all(&base);
        let path = base.join("app.log");
        std::fs::write(&path, "line1\nline2\n").unwrap();
        let mut file = File::open(&path).unwrap();
        let mut pos = 0u64;
        let mut buf = Vec::new();
        assert_eq!(read_new(&mut file, &mut pos, &mut buf).unwrap(), 12);
        assert_eq!(drain_lines(&mut buf), vec!["line1".to_string(), "line2".to_string()]);

        std::fs::write(&path, "line3\n").unwrap();
        assert_eq!(read_new(&mut file, &mut pos, &mut buf).unwrap(), 6);
        assert_eq!(drain_lines(&mut buf), vec!["line3".to_string()]);

        // truncation: shorter file restarts from the beginning
        std::fs::write(&path, "X\n").unwrap();
        assert_eq!(read_new(&mut file, &mut pos, &mut buf).unwrap(), 2);
        assert_eq!(drain_lines(&mut buf), vec!["X".to_string()]);

        std::fs::remove_dir_all(&base).ok();
    }

    fn fs_create_dir_all(p: &std::path::Path) {
        std::fs::create_dir_all(p).unwrap();
    }

    #[test]
    fn clamp_top_bounds() {
        assert_eq!(clamp_top(0, 10, 100), 0);
        assert_eq!(clamp_top(95, 10, 100), 90);
        assert_eq!(clamp_top(usize::MAX, 10, 100), 90);
        assert_eq!(clamp_top(usize::MAX, 10, 5), 0);
        assert_eq!(clamp_top(usize::MAX, 10, 0), 0);
    }

    #[test]
    fn truncate_display_width() {
        assert_eq!(truncate_display("hello world", 5), "hello");
        assert_eq!(truncate_display("你好世界", 3), "你"); // 2 cols per CJK char
        assert_eq!(truncate_display("abc", 10), "abc");
        assert_eq!(display_cols("ab你c"), 5);
    }
}
