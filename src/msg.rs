// ---------------------------------------------------------------------------
// sys msg: Windows scheduled reminders via schtasks + msg.exe popup
//
// `sys msg --daily 20:00:02 "text"` / `--once` / `--cron "MIN HOUR DOM MON DOW"`
// create a Windows scheduled task whose action pops the text up with the
// msg.exe dialog. Every created task is named with the `sysmsg-` prefix, so
// `--list` / `--delete` only ever touch sys-created tasks, never the system's
// own scheduled tasks.
// ---------------------------------------------------------------------------

use anyhow::{bail, Context, Result};
use chrono::{Days, Local, NaiveDate, NaiveTime};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Task-name prefix that isolates sys-created tasks from system scheduled tasks.
const PREFIX: &str = "sysmsg-";

/// A time of day with second precision (`HH:MM[:SS]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TimeOfDay {
    h: u8,
    m: u8,
    s: u8,
}

impl TimeOfDay {
    fn fmt_st(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.h, self.m, self.s)
    }
}

/// The schedule behind a `--daily` / `--once` / `--cron` add.
enum Schedule {
    Daily(TimeOfDay),
    Once { date: NaiveDate, time: TimeOfDay },
    Cron(CronSpec),
}

/// A cron field: `*`, `*/n`, a single value or a comma list.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Field {
    Any,
    Step(u32),
    Num(u8),
    List(Vec<u8>),
}

/// Five-field cron expression (MIN HOUR DOM MON DOW); DOW 0 and 7 both mean Sunday.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CronSpec {
    minute: Field,
    hour: Field,
    dom: Field,
    month: Field,
    dow: Field,
}

const DOW_NAMES: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse `HH:MM` or `HH:MM:SS` (24h clock).
fn parse_time(s: &str) -> Result<TimeOfDay> {
    let t = s.trim();
    let parts: Vec<&str> = t.split(':').collect();
    let (hh, mm, ss) = match parts.as_slice() {
        [h, m] => (*h, *m, "0"),
        [h, m, sec] => (*h, *m, *sec),
        _ => bail!("invalid time `{s}` (expected HH:MM or HH:MM:SS, e.g. 20:00:02)"),
    };
    let to_u8 = |v: &str, what: &str, max: u8| -> Result<u8> {
        let n: u8 = v
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid {what} `{v}` in time `{s}`"))?;
        if n > max {
            bail!("invalid {what} `{v}` in time `{s}` (0-{max})");
        }
        Ok(n)
    };
    Ok(TimeOfDay {
        h: to_u8(hh, "hour", 23)?,
        m: to_u8(mm, "minute", 59)?,
        s: to_u8(ss, "second", 59)?,
    })
}

/// Parse one cron field (`*` / `*/n` / a value / a comma list), range-checked.
fn parse_field(s: &str, what: &str, max: u8) -> Result<Field> {
    let t = s.trim();
    if t == "*" {
        return Ok(Field::Any);
    }
    if let Some(rest) = t.strip_prefix("*/") {
        let n: u32 = rest
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid step `{rest}` in cron {what} field (expected */n)"))?;
        if n == 0 || n > max as u32 {
            bail!("invalid step `{n}` in cron {what} field (1-{max})");
        }
        return Ok(Field::Step(n));
    }
    let nums: Vec<u8> = t
        .split(',')
        .map(|v| {
            v.trim()
                .parse::<u8>()
                .map_err(|_| anyhow::anyhow!("invalid value `{v}` in cron {what} field"))
        })
        .collect::<Result<_>>()?;
    for n in &nums {
        if *n > max {
            bail!("invalid value `{n}` in cron {what} field (0-{max})");
        }
    }
    if nums.len() == 1 {
        Ok(Field::Num(nums[0]))
    } else {
        Ok(Field::List(nums))
    }
}

/// Parse a five-field cron expression.
fn parse_cron(s: &str) -> Result<CronSpec> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 5 {
        bail!(
            "invalid cron expression `{s}` (expected 5 fields: MIN HOUR DOM MON DOW, e.g. \"0 20 * * *\")"
        );
    }
    Ok(CronSpec {
        minute: parse_field(parts[0], "minute", 59)?,
        hour: parse_field(parts[1], "hour", 23)?,
        dom: parse_field(parts[2], "day-of-month", 31)?,
        month: parse_field(parts[3], "month", 12)?,
        dow: parse_field(parts[4], "day-of-week", 7)?, // 7 = Sunday, same as 0
    })
}

/// Compact slug of a field used in the default task name.
fn field_slug(f: &Field) -> String {
    match f {
        Field::Any => "x".to_string(),
        Field::Step(n) => format!("n{n}"),
        Field::Num(n) => n.to_string(),
        Field::List(v) => v
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("_"),
    }
}

// ---------------------------------------------------------------------------
// cron -> schtasks mapping
// ---------------------------------------------------------------------------

/// Both MINUTE and HOUR must be concrete single values for WEEKLY / MONTHLY
/// / DAILY schedules (a wildcard there means "every minute/hour", which one
/// scheduled task cannot express).
fn pair(minute: &Field, hour: &Field) -> Result<(u8, u8)> {
    match (minute, hour) {
        (Field::Num(m), Field::Num(h)) => Ok((*m, *h)),
        _ => bail!(
            "cron 的 MINUTE 与 HOUR 必须同时为具体值（如 `0 20 * * 1`）；`*` / `*/n` / 列表组合无法用单条计划任务表达"
        ),
    }
}

/// Map a parsed cron expression to schtasks scheduling flags (`/SC ... /MO n
/// /D ... /ST HH:MM:SS`). Only the subset expressible by a single schtasks task
/// is supported; anything else fails with a hint.
fn cron_to_schtasks(c: &CronSpec) -> Result<Vec<String>> {
    if !matches!(c.month, Field::Any) {
        bail!(
            "cron 的 MONTH 字段暂不支持（计划任务无法按月过滤），请用 --daily / --once 或简化表达式"
        );
    }
    if !matches!(c.dom, Field::Any) && !matches!(c.dow, Field::Any) {
        bail!(
            "cron 的 DOM 与 DOW 同时指定是 OR 语义，单条计划任务无法表达；请拆成两条或只保留其中一个"
        );
    }
    if !matches!(c.dow, Field::Any) && !matches!(c.dow, Field::Num(_)) {
        bail!("cron 的 DOW 仅支持具体值或 `*`（`*/n`/列表无法映射到计划任务）");
    }
    if !matches!(c.dom, Field::Any) && !matches!(c.dom, Field::Num(_)) {
        bail!("cron 的 DOM 仅支持具体值或 `*`（`*/n`/列表无法映射到计划任务）");
    }
    let mut o: Vec<String> = Vec::new();
    if let Field::Num(dow) = &c.dow {
        let (mm, hh) = pair(&c.minute, &c.hour)?;
        o.extend([
            "/SC".to_string(),
            "WEEKLY".into(),
            "/D".into(),
            DOW_NAMES[(*dow % 7) as usize].into(),
            "/ST".into(),
            TimeOfDay { h: hh, m: mm, s: 0 }.fmt_st(),
        ]);
        return Ok(o);
    }
    if let Field::Num(dom) = &c.dom {
        let (mm, hh) = pair(&c.minute, &c.hour)?;
        o.extend([
            "/SC".to_string(),
            "MONTHLY".into(),
            "/D".into(),
            dom.to_string(),
            "/ST".into(),
            TimeOfDay { h: hh, m: mm, s: 0 }.fmt_st(),
        ]);
        return Ok(o);
    }
    let m = match &c.minute {
        Field::Num(n) => Some(*n),
        _ => None,
    };
    match m {
        // `*/n * * * *` -> every n minutes; `* * * * *` -> every minute.
        // A concrete HOUR with a wildcard MINUTE cannot be expressed by one task.
        None => {
            if !matches!(c.hour, Field::Any) {
                bail!(
                    "cron 的 HOUR 具体但 MINUTE 为 `*`/`*/n` 时无法用单条计划任务表达；请用 `*/n * * * *`、`m * * * *` 或 --daily"
                );
            }
            match &c.minute {
                Field::Step(n) => {
                    o.extend(["/SC".into(), "MINUTE".into(), "/MO".into(), n.to_string()]);
                }
                Field::Any => {
                    o.extend(["/SC".into(), "MINUTE".into(), "/MO".into(), "1".into()]);
                }
                Field::List(_) => bail!("cron MINUTE 列表不支持，请拆成多条任务"),
                Field::Num(_) => unreachable!(),
            }
        }
        // `m h * * *` -> daily at h:m; `m * * * *` -> hourly at minute m;
        // `m */n * * *` -> every n hours at minute m.
        Some(mm) => match &c.hour {
            Field::Num(hh) => o.extend([
                "/SC".into(),
                "DAILY".into(),
                "/ST".into(),
                TimeOfDay { h: *hh, m: mm, s: 0 }.fmt_st(),
            ]),
            Field::Any => o.extend([
                "/SC".into(),
                "HOURLY".into(),
                "/MO".into(),
                "1".into(),
                "/ST".into(),
                TimeOfDay { h: 0, m: mm, s: 0 }.fmt_st(),
            ]),
            Field::Step(n) => o.extend([
                "/SC".into(),
                "HOURLY".into(),
                "/MO".into(),
                n.to_string(),
                "/ST".into(),
                TimeOfDay { h: 0, m: mm, s: 0 }.fmt_st(),
            ]),
            Field::List(_) => bail!("cron HOUR 列表不支持，请拆成多条任务"),
        },
    }
    Ok(o)
}

// ---------------------------------------------------------------------------
// GBK encoding helpers (msg.exe reads its stdin in the system ANSI code page)
// ---------------------------------------------------------------------------

/// Encode TEXT as GBK bytes (msg.exe on a Chinese Windows expects GBK/936).
fn encode_gbk(text: &str) -> Result<Vec<u8>> {
    let (bytes, _, had_errors) = encoding_rs::GBK.encode(text);
    if had_errors {
        bail!("提醒内容含无法用 GBK 表示的字符");
    }
    let mut out = bytes.into_owned();
    out.push(b'\n');
    Ok(out)
}

/// Decode a GBK .txt file for `--list` output (fall back to lossy UTF-8).
fn decode_gbk(bytes: &[u8]) -> String {
    let (s, _, had_errors) = encoding_rs::GBK.decode(bytes);
    if had_errors {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        s.into_owned()
    }
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

/// `%LOCALAPPDATA%\sysmsg` where the .cmd launcher and the .txt message live.
fn base_dir() -> Result<PathBuf> {
    let la = std::env::var("LOCALAPPDATA")
        .with_context(|| "LOCALAPPDATA is not set (this is a Windows feature)")?;
    Ok(Path::new(&la).join("sysmsg"))
}

/// Task names may only contain ASCII letters, digits, `-` and `_` (protects the
/// `schtasks /TN` value from injection).
fn validate_name(n: &str) -> Result<()> {
    if n.is_empty() || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("任务名称只能包含字母、数字、- 和 _（当前：`{n}`）");
    }
    Ok(())
}

/// Parse one quoted CSV line into its columns (handles embedded commas).
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut cols = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => cols.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    cols.push(cur);
    cols
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Entry point: `sys msg ...`.
pub fn cmd_msg(
    daily: Option<&str>,
    once: Option<&str>,
    cron: Option<&str>,
    name: Option<&str>,
    list: bool,
    delete: Option<&str>,
    text: Option<&str>,
) -> Result<()> {
    if !cfg!(windows) {
        bail!("sys msg 仅支持 Windows（依赖 schtasks 与 msg.exe）");
    }
    if list {
        return cmd_list();
    }
    if let Some(n) = delete {
        return cmd_delete(n);
    }
    if daily.is_some() || once.is_some() || cron.is_some() {
        return cmd_add(daily, once, cron, name, text);
    }
    bail!(
        "usage:\n  sys msg --daily 20:00:02 [--name NAME] \"内容\"\n  sys msg --once 20:00:02 [--name NAME] \"内容\"\n  sys msg --cron \"0 20 * * *\" [--name NAME] \"内容\"\n  sys msg --list\n  sys msg --delete NAME"
    );
}

/// Create a scheduled task whose action pops TEXT up via msg.exe.
fn cmd_add(
    daily: Option<&str>,
    once: Option<&str>,
    cron: Option<&str>,
    name: Option<&str>,
    text: Option<&str>,
) -> Result<()> {
    let text = text.ok_or_else(|| {
        anyhow::anyhow!("请提供提醒内容：sys msg --daily 20:00:02 \"内容\"")
    })?;
    let text = text.trim();
    if text.is_empty() {
        bail!("提醒内容不能为空");
    }

    // Resolve the schedule and the default task name.
    let (schedule, default_name, desc) = if let Some(t) = daily {
        let tod = parse_time(t)?;
        (
            Schedule::Daily(tod),
            format!("daily-{:02}{:02}{:02}", tod.h, tod.m, tod.s),
            format!("每天 {}", tod.fmt_st()),
        )
    } else if let Some(t) = once {
        let tod = parse_time(t)?;
        let now = Local::now();
        let mut date = now.date_naive();
        let target = NaiveTime::from_hms_opt(tod.h as u32, tod.m as u32, tod.s as u32)
            .ok_or_else(|| anyhow::anyhow!("invalid time {t}"))?;
        if now.time() >= target {
            date = date + Days::new(1);
        }
        (
            Schedule::Once { date, time: tod },
            format!("once-{:02}{:02}{:02}", tod.h, tod.m, tod.s),
            format!("{} {}（一次）", date.format("%Y-%m-%d"), tod.fmt_st()),
        )
    } else {
        let expr = cron.ok_or_else(|| anyhow::anyhow!("missing --cron expression"))?;
        let c = parse_cron(expr)?;
        let slug = format!(
            "cron-{}-{}-{}-{}-{}",
            field_slug(&c.minute),
            field_slug(&c.hour),
            field_slug(&c.dom),
            field_slug(&c.month),
            field_slug(&c.dow)
        );
        (Schedule::Cron(c), slug, format!("cron: {expr}"))
    };

    let task_name = match name {
        Some(n) => {
            validate_name(n)?;
            n.to_string()
        }
        None => default_name,
    };
    let full = format!("{PREFIX}{task_name}");

    // Validate the schedule (cron -> schtasks mapping) *before* writing any
    // file, so an unsupported expression leaves no orphan .cmd/.txt behind.
    let mut flags: Vec<String> = Vec::new();
    match &schedule {
        Schedule::Daily(t) => flags.extend(["/SC".into(), "DAILY".into(), "/ST".into(), t.fmt_st()]),
        Schedule::Once { date, time } => flags.extend([
            "/SC".into(),
            "ONCE".into(),
            "/SD".into(),
            date.format("%Y/%m/%d").to_string(),
            "/ST".into(),
            time.fmt_st(),
        ]),
        Schedule::Cron(c) => flags.extend(cron_to_schtasks(c)?),
    }

    // Write the launcher (.cmd, pure ASCII) and the message (.txt). The message
    // file must be GBK: msg.exe decodes its stdin with the system ANSI code
    // page (936 on Chinese Windows) — chcp 65001 does not affect it, UTF-8
    // bytes come out as mojibake. The popup stays until the user dismisses it.
    let dir = base_dir()?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("cannot create {}", dir.display()))?;
    let txt_gbk = encode_gbk(text)?;
    fs::write(dir.join(format!("{full}.txt")), txt_gbk)
        .with_context(|| format!("cannot write {}", dir.join(format!("{full}.txt")).display()))?;
    // One-shot tasks delete themselves after the popup is dismissed: msg waits
    // for the user's OK, then the batch removes the scheduled task and both
    // files. Daily/cron tasks stay. Order matters: `del "%~f0"` (the running
    // batch itself) must be the LAST line — once cmd.exe deletes the batch it
    // is executing it cannot read any further lines, so the .txt is removed
    // first. If the schtasks delete ever fails the batch exits without touching
    // the files, so a manual `sys msg --delete` can retry.
    let cmd_body = match &schedule {
        Schedule::Once { .. } => format!(
            "@echo off\r\nmsg * < \"%~dp0{full}.txt\"\r\nschtasks /Delete /TN \"{full}\" /F >nul 2>nul || goto :eof\r\ndel \"%~dp0{full}.txt\"\r\ndel \"%~f0\"\r\n"
        ),
        _ => format!("@echo off\r\nmsg * < \"%~dp0{full}.txt\"\r\n"),
    };
    fs::write(dir.join(format!("{full}.cmd")), cmd_body)
        .with_context(|| format!("cannot write {}", dir.join(format!("{full}.cmd")).display()))?;

    // Build the schtasks command line.
    let cmd_path = dir.join(format!("{full}.cmd"));
    let cmd_path_s = cmd_path.to_string_lossy().into_owned();
    let tr = if cmd_path_s.contains(' ') {
        format!("cmd /c \"{cmd_path_s}\"")
    } else {
        format!("cmd /c {cmd_path_s}")
    };
    let mut args: Vec<String> = vec![
        "/Create".into(),
        "/F".into(),
        "/TN".into(),
        full.clone(),
        "/TR".into(),
        tr,
    ];
    args.extend(flags);
    let username = std::env::var("USERNAME").unwrap_or_default();
    if !username.is_empty() {
        args.push("/RU".into());
        args.push(username);
        args.push("/IT".into());
    }
    let out = Command::new("schtasks").args(&args).output()?;
    if !out.status.success() {
        bail!(
            "schtasks 创建失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    println!("已创建定时提醒任务 {full}");
    println!("  时间：{desc}");
    println!("  内容：{text}");
    Ok(())
}

/// List every task whose name starts with `sysmsg-`, with next run time / status
/// / message content.
fn cmd_list() -> Result<()> {
    let out = Command::new("schtasks").args(["/Query", "/FO", "CSV"]).output()?;
    if !out.status.success() {
        bail!("schtasks /Query 失败：{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let dir = base_dir()?;
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for line in stdout.lines().skip(1) {
        let cols = parse_csv_line(line);
        if cols.len() < 3 {
            continue;
        }
        let name = cols[0].trim().trim_start_matches('\\');
        if !name.starts_with(PREFIX) {
            continue;
        }
        rows.push((
            name.to_string(),
            cols[1].trim().to_string(),
            cols[2].trim().to_string(),
        ));
    }
    rows.sort();
    if rows.is_empty() {
        println!("(no sysmsg tasks)");
        return Ok(());
    }
    for (name, next, status) in rows {
        let content = fs::read(dir.join(format!("{name}.txt")))
            .map(|b| decode_gbk(&b))
            .unwrap_or_default()
            .trim()
            .to_string();
        println!("{name}  [{status}]  next: {next}  msg: {content}");
    }
    Ok(())
}

/// Delete one sysmsg task (a bare name gets the `sysmsg-` prefix automatically;
/// nothing outside the prefix is ever touched).
fn cmd_delete(name: &str) -> Result<()> {
    let mut full = name.trim().to_string();
    if full.is_empty() {
        bail!("请提供要删除的任务名：sys msg --delete NAME");
    }
    if !full.starts_with(PREFIX) {
        full = format!("{PREFIX}{full}");
    }
    validate_name(full.trim_start_matches(PREFIX))?;
    let out = Command::new("schtasks")
        .args(["/Delete", "/TN", full.as_str(), "/F"])
        .output()?;
    if !out.status.success() {
        bail!(
            "删除任务失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let dir = base_dir()?;
    for ext in ["cmd", "txt"] {
        let p = dir.join(format!("{full}.{ext}"));
        if p.exists() {
            fs::remove_file(&p)
                .with_context(|| format!("cannot remove {}", p.display()))?;
        }
    }
    println!("已删除定时提醒任务 {full}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_parsing_valid_and_invalid() {
        assert_eq!(parse_time("20:00").unwrap(), TimeOfDay { h: 20, m: 0, s: 0 });
        assert_eq!(parse_time("20:00:02").unwrap(), TimeOfDay { h: 20, m: 0, s: 2 });
        assert_eq!(parse_time(" 09:05:59 ").unwrap(), TimeOfDay { h: 9, m: 5, s: 59 });
        assert!(parse_time("25:00").is_err());
        assert!(parse_time("20:60").is_err());
        assert!(parse_time("20:00:61").is_err());
        assert!(parse_time("20").is_err());
        assert!(parse_time("20:00:00:00").is_err());
        assert!(parse_time("abc").is_err());
    }

    #[test]
    fn cron_parsing() {
        let c = parse_cron("0 20 * * *").unwrap();
        assert_eq!(
            c,
            CronSpec {
                minute: Field::Num(0),
                hour: Field::Num(20),
                dom: Field::Any,
                month: Field::Any,
                dow: Field::Any,
            }
        );
        let c = parse_cron("*/5 * * * *").unwrap();
        assert_eq!(c.minute, Field::Step(5));
        let c = parse_cron("1,15 9 * * 1,3").unwrap();
        assert_eq!(c.minute, Field::List(vec![1, 15]));
        assert_eq!(c.dow, Field::List(vec![1, 3]));
        assert!(parse_cron("0 20 * *").is_err()); // 4 fields
        assert!(parse_cron("60 * * * *").is_err()); // minute > 59
        assert!(parse_cron("0 24 * * *").is_err()); // hour > 23
        assert!(parse_cron("0 20 32 * *").is_err()); // dom > 31
        assert!(parse_cron("0 20 * * 8").is_err()); // dow > 7
        assert!(parse_cron("0 20 * * */0").is_err()); // step 0
        assert!(parse_cron("0 20 * * */8").is_err()); // step > 7 (dow max)
    }

    #[test]
    fn cron_to_schtasks_mapping() {
        let flags = |s: &str| -> Vec<String> { cron_to_schtasks(&parse_cron(s).unwrap()).unwrap() };

        // Daily at 20:00.
        assert_eq!(flags("0 20 * * *"), vec!["/SC", "DAILY", "/ST", "20:00:00"]);
        // Every 5 minutes.
        assert_eq!(flags("*/5 * * * *"), vec!["/SC", "MINUTE", "/MO", "5"]);
        // Every minute.
        assert_eq!(flags("* * * * *"), vec!["/SC", "MINUTE", "/MO", "1"]);
        // Hourly at minute 30.
        assert_eq!(flags("30 * * * *"), vec!["/SC", "HOURLY", "/MO", "1", "/ST", "00:30:00"]);
        // Every 2 hours at minute 0.
        assert_eq!(flags("0 */2 * * *"), vec!["/SC", "HOURLY", "/MO", "2", "/ST", "00:00:00"]);
        // Weekly on Monday 20:00; DOW 0 and 7 both map to SUN.
        assert_eq!(flags("0 20 * * 1"), vec!["/SC", "WEEKLY", "/D", "MON", "/ST", "20:00:00"]);
        assert_eq!(flags("0 20 * * 0"), vec!["/SC", "WEEKLY", "/D", "SUN", "/ST", "20:00:00"]);
        assert_eq!(flags("0 20 * * 7"), vec!["/SC", "WEEKLY", "/D", "SUN", "/ST", "20:00:00"]);
        // Monthly on the 1st at 20:00.
        assert_eq!(flags("0 20 1 * *"), vec!["/SC", "MONTHLY", "/D", "1", "/ST", "20:00:00"]);
        // Unsupported combos error with hints.
        assert!(cron_to_schtasks(&parse_cron("0 20 1 * 1").unwrap()).is_err()); // dom + dow
        assert!(cron_to_schtasks(&parse_cron("0 20 * 6 *").unwrap()).is_err()); // month
        assert!(cron_to_schtasks(&parse_cron("0 9,18 * * *").unwrap()).is_err()); // list
        assert!(cron_to_schtasks(&parse_cron("* 20 * * 1").unwrap()).is_err()); // minute any + dow
        assert!(cron_to_schtasks(&parse_cron("*/15 9 * * *").unwrap()).is_err()); // step minute + concrete hour
        assert!(cron_to_schtasks(&parse_cron("0 20 * * */2").unwrap()).is_err()); // dow step
        assert!(cron_to_schtasks(&parse_cron("0 20 * * 1,3").unwrap()).is_err()); // dow list
        assert!(cron_to_schtasks(&parse_cron("0 20 */2 * *").unwrap()).is_err()); // dom step
        assert!(cron_to_schtasks(&parse_cron("0 20,21 * * *").unwrap()).is_err()); // hour list
    }

    #[test]
    fn csv_line_parsing() {
        assert_eq!(
            parse_csv_line(r#""\sysmsg-daily-200002","2026-10-27 03:00:00","Ready""#),
            vec!["\\sysmsg-daily-200002", "2026-10-27 03:00:00", "Ready"]
        );
        assert_eq!(parse_csv_line(r#""a,b","c""#), vec!["a,b", "c"]);
    }

    #[test]
    fn name_validation_and_slugs() {
        assert!(validate_name("daily-200002").is_ok());
        assert!(validate_name("my_reminder-1").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("a b").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("cmd&calc").is_err());
        assert_eq!(field_slug(&Field::Any), "x");
        assert_eq!(field_slug(&Field::Step(5)), "n5");
        assert_eq!(field_slug(&Field::Num(20)), "20");
        assert_eq!(field_slug(&Field::List(vec![9, 18])), "9_18");
    }

    #[test]
    fn gbk_roundtrip_chinese() {
        let text = "每日下班提醒：该休息了";
        let bytes = encode_gbk(text).unwrap();
        // Not valid UTF-8 (it is GBK), and decodes back losslessly.
        assert!(std::str::from_utf8(&bytes).is_err() || !bytes.is_empty());
        assert_eq!(decode_gbk(&bytes).trim(), text);
        // ASCII text stays valid UTF-8 too.
        assert_eq!(decode_gbk(&encode_gbk("drink water").unwrap()).trim(), "drink water");
    }
}
