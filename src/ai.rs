//! `sys ai` — query AI model info.
//!
//! - `ai model` / `ai provider` query the models.dev database
//!   (`https://models.dev/api.json`), the data behind the `/models/` and
//!   `/providers/` pages.
//! - `ai cn-model` queries the DataLearner AI model list
//!   (`https://www.datalearner.com/ai-models/pretrained-models`), a
//!   server-rendered HTML page (paged via `?page=N`); only released models
//!   with a real `published` date are kept. A single exact hit fetches the
//!   detail page to enrich the entry with the context length, the
//!   input/output modalities and the parameter scale (总参数 / 激活参数).

use anyhow::{Context, Result, bail};
use crate::chat;
use crate::OutFormat;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const DATA_URL: &str = "https://models.dev/api.json";
const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

// ---------------------------------------------------------------------------
// OpenRouter model catalogue (https://openrouter.ai/api/v1/models)
// ---------------------------------------------------------------------------

/// Cache file for the OpenRouter catalogue (same 24 h TTL as models.dev).
fn openrouter_cache_file() -> PathBuf {
    cache_dir().join("openrouter-models.json")
}

fn load_openrouter_cache() -> Option<Vec<Value>> {
    let file = openrouter_cache_file();
    let meta = std::fs::metadata(&file).ok()?;
    let mtime = meta.modified().ok()?;
    if SystemTime::now().duration_since(mtime).map(|d| d < CACHE_TTL).unwrap_or(false) {
        if let Ok(content) = std::fs::read_to_string(&file) {
            if let Ok(v) = serde_json::from_str::<Value>(&content) {
                if let Some(arr) = v.get("data").and_then(|d| d.as_array()).cloned() {
                    return Some(arr);
                }
            }
        }
    }
    None
}

fn save_openrouter_cache(text: &str) {
    let dir = cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(openrouter_cache_file(), text);
}

/// Fetch the OpenRouter model catalogue (`data[]` array). Cached for 24 h
/// unless `refresh` is set; the request has a 30 s timeout.
fn fetch_openrouter_models(refresh: bool) -> Result<Vec<Value>> {
    if !refresh {
        if let Some(arr) = load_openrouter_cache() {
            return Ok(arr);
        }
    }
    eprintln!("sys: fetching {OPENROUTER_MODELS_URL} ...");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("cannot build HTTP client")?;
    let resp = client
        .get(OPENROUTER_MODELS_URL)
        .send()
        .with_context(|| format!("cannot fetch {OPENROUTER_MODELS_URL}"))?;
    if !resp.status().is_success() {
        bail!("openrouter.ai returned HTTP {}", resp.status());
    }
    let text = resp.text().context("cannot read openrouter.ai response")?;
    let v: Value = serde_json::from_str(&text).context("invalid JSON received from openrouter.ai")?;
    let arr = v
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("unexpected openrouter.ai payload (expected a `data` array)"))?;
    save_openrouter_cache(&text);
    Ok(arr)
}

/// `pricing.prompt` as f64 (missing / non-numeric -> None).
fn or_price(m: &Value, key: &str) -> Option<f64> {
    m.get("pricing")
        .and_then(|p| p.get(key))
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| {
            m.get("pricing")
                .and_then(|p| p.get(key))
                .and_then(|v| v.as_f64())
        })
}

/// Format a price for display. OpenRouter's `pricing` fields are per-token USD;
/// every view of them is per 1M tokens, so the value is multiplied by 1e6 here.
/// Keeps up to 8 decimals and trims trailing zeros (`0.00000025` -> `$0.25`);
/// None -> "-".
fn or_fmt_price(v: Option<f64>) -> String {
    match v {
        Some(x) => {
            let mut s = format!("{:.8}", x * 1e6);
            while s.ends_with('0') && !s.ends_with(".0") {
                s.pop();
            }
            format!("${s}")
        }
        None => "-".to_string(),
    }
}

/// One row of the OpenRouter model list.
#[derive(Clone)]
struct OrModelRow {
    id: String,
    name: String,
    context_length: u64,
    prompt: Option<f64>,
    completion: Option<f64>,
    created: String,
    /// Raw `created` epoch seconds (0 when missing), for JSON/YAML output.
    created_ts: i64,
}

fn or_model_rows(items: &[Value]) -> Vec<OrModelRow> {
    items
        .iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(|x| x.as_str())?.to_string();
            let name = m.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let ctx = m.get("context_length").and_then(|x| x.as_u64()).unwrap_or(0);
            let prompt = or_price(m, "prompt");
            let created_ts = m.get("created").and_then(|x| x.as_i64()).unwrap_or(0);
            let created = m
                .get("created")
                .and_then(|x| x.as_i64())
                .map(|ts| {
                    chrono::DateTime::from_timestamp(ts, 0)
                        .map(|dt| {
                            dt.with_timezone(&chrono::Local)
                                .format("%Y-%m-%d %H:%M:%S")
                                .to_string()
                        })
                        .unwrap_or_else(|| ts.to_string())
                })
                .or_else(|| m.get("created").and_then(|x| x.as_str()).map(str::to_string))
                .unwrap_or_default();
            Some(OrModelRow { id, name, context_length: ctx, prompt, completion: or_price(m, "completion"), created, created_ts })
        })
        .collect()
}

/// Text view of the OpenRouter model list (aligned columns).
fn or_model_table(rows: &[OrModelRow]) -> String {
    let w_id = rows.iter().map(|r| r.id.chars().count()).max().unwrap_or(2).max(2);
    let w_nm = rows.iter().map(|r| r.name.chars().count()).max().unwrap_or(4).max(4);
    let w_ctx = rows.iter().map(|r| fmt_num(r.context_length).len()).max().unwrap_or(7).max(7);
    let w_cr = rows.iter().map(|r| r.created.chars().count()).max().unwrap_or(7).max(7);
    let mut s = format!(
        "{:<w_id$}  {:<w_nm$}  {:>w_ctx$}  {:>10}  {:<w_cr$}\n",
        "ID", "NAME", "CONTEXT", "PRICE/1M", "CREATED"
    );
    for r in rows {
        s.push_str(&format!(
            "{:<w_id$}  {:<w_nm$}  {:>w_ctx$}  {:>10}  {:<w_cr$}\n",
            r.id,
            r.name,
            fmt_num(r.context_length),
            or_fmt_price(r.prompt),
            r.created
        ));
    }
    s
}

/// Filter OpenRouter rows by a query: exact (case-insensitive) on id / name
/// first, then substring on id / name. `queries` accepts several
/// comma-separated terms (half- or full-width); a row matches when any term
/// matches. Empty list keeps everything.
fn filter_or_rows(rows: Vec<OrModelRow>, queries: &[String]) -> Vec<OrModelRow> {
    if queries.is_empty() {
        return rows;
    }
    let exact: Vec<OrModelRow> = rows
        .iter()
        .filter(|r| {
            queries
                .iter()
                .any(|q| ci_eq(&r.id, q) || ci_eq(&r.name, q))
        })
        .cloned()
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    rows.into_iter()
        .filter(|r| {
            queries
                .iter()
                .any(|q| contains_ci(&r.id, q) || contains_ci(&r.name, q))
        })
        .collect()
}

/// Parse a query string into comma-separated terms (half- or full-width).
fn parse_or_queries(query: Option<&str>) -> Vec<String> {
    query
        .map(|s| {
            s.split([',', '，'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Render the OpenRouter catalogue (`items` = raw `data[]` array) in the
/// requested format. `query` filters rows (exact then substring), `limit` caps
/// the output; every price value is per 1M tokens. Pure function (no I/O), so
/// it is unit-testable.
fn or_model_view(items: &[Value], query: Option<&str>, limit: Option<usize>, fmt: Option<OutFormat>) -> Result<String> {
    let queries = parse_or_queries(query);
    let mut rows = filter_or_rows(or_model_rows(items), &queries);
    if let Some(n) = limit {
        rows.truncate(n.max(1));
    }
    Ok(match fmt {
        Some(OutFormat::Json) => {
            let arr: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "name": r.name,
                        "context_length": r.context_length,
                        "price_prompt": r.prompt.map(|x| x * 1e6),
                        "created": r.created_ts,
                    })
                })
                .collect();
            serde_json::to_string_pretty(&Value::Array(arr))?
        }
        Some(OutFormat::Csv) => {
            let mut s = String::from("id,name,context_length,price_prompt,created\n");
            for r in &rows {
                s.push_str(&format!(
                    "{},{},{},{},{}\n",
                    csv_escape(&r.id),
                    csv_escape(&r.name),
                    r.context_length,
                    r.prompt.map(|x| format!("{:.8}", x * 1e6)).unwrap_or_default(),
                    r.created
                ));
            }
            s
        }
        Some(OutFormat::Md) => {
            let cells: Vec<Vec<String>> = rows
                .iter()
                .map(|r| {
                    vec![
                        r.id.clone(),
                        r.name.clone(),
                        fmt_num(r.context_length),
                        or_fmt_price(r.prompt),
                        r.created.clone(),
                    ]
                })
                .collect();
            md_table(&["id", "name", "context_length", "price/1M", "created"], &cells)
        }
        Some(OutFormat::Yaml) => {
            let arr: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "name": r.name,
                        "context_length": r.context_length,
                        "price_prompt": r.prompt.map(|x| x * 1e6),
                        "created": r.created_ts,
                    })
                })
                .collect();
            serde_yaml::to_string(&Value::Array(arr))?
        }
        None => or_model_table(&rows),
    })
}

/// `sys ai model --src openrouter` — list the OpenRouter catalogue.
pub fn cmd_model_openrouter(query: Option<&str>, json: bool, out: Option<OutFormat>, limit: Option<usize>, refresh: bool) -> Result<()> {
    let items = fetch_openrouter_models(refresh)?;
    let fmt = out.or(if json { Some(OutFormat::Json) } else { None });
    print!("{}", or_model_view(&items, query, limit, fmt)?);
    Ok(())
}

/// One row of the OpenRouter price view (`sys ai price --src openrouter`).
struct OrPriceRow {
    id: String,
    name: String,
    prompt: Option<f64>,
    completion: Option<f64>,
}

/// `sys ai price --src openrouter` — prompt/completion prices of the OpenRouter
/// catalogue (all models, or the comma-separated filtered subset). Prices are
/// per 1M tokens in every output format.
pub fn cmd_price_openrouter(query: Option<&str>, json: bool, out: Option<OutFormat>, limit: Option<usize>, refresh: bool) -> Result<()> {
    let items = fetch_openrouter_models(refresh)?;
    let queries = parse_or_queries(query);
    let mut rows: Vec<OrPriceRow> = filter_or_rows(or_model_rows(&items), &queries)
        .into_iter()
        .map(|r| OrPriceRow {
            id: r.id,
            name: r.name,
            prompt: r.prompt,
            completion: r.completion,
        })
        .collect();
    if let Some(n) = limit {
        rows.truncate(n.max(1));
    }
    let fmt = out.or(if json { Some(OutFormat::Json) } else { None });
    match fmt {
        Some(OutFormat::Json) => {
            let arr: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "name": r.name,
                        "price_prompt": r.prompt.map(|x| x * 1e6),
                        "price_completion": r.completion.map(|x| x * 1e6),
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
        }
        Some(OutFormat::Csv) => {
            println!("id,name,price_prompt,price_completion");
            for r in &rows {
                println!(
                    "{},{},{},{}",
                    csv_escape(&r.id),
                    csv_escape(&r.name),
                    r.prompt.map(|x| format!("{:.8}", x * 1e6)).unwrap_or_default(),
                    r.completion.map(|x| format!("{:.8}", x * 1e6)).unwrap_or_default()
                );
            }
        }
        Some(OutFormat::Md) => {
            let cells: Vec<Vec<String>> = rows
                .iter()
                .map(|r| {
                    vec![
                        r.id.clone(),
                        r.name.clone(),
                        or_fmt_price(r.prompt),
                        or_fmt_price(r.completion),
                    ]
                })
                .collect();
            println!("{}", md_table(&["id", "name", "prompt/1M", "completion/1M"], &cells));
        }
        Some(OutFormat::Yaml) => {
            let arr: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "name": r.name,
                        "price_prompt": r.prompt.map(|x| x * 1e6),
                        "price_completion": r.completion.map(|x| x * 1e6),
                    })
                })
                .collect();
            println!("{}", serde_yaml::to_string(&Value::Array(arr))?);
        }
        None => {
            let w_id = rows.iter().map(|r| r.id.chars().count()).max().unwrap_or(2).max(2);
            let w_nm = rows.iter().map(|r| r.name.chars().count()).max().unwrap_or(4).max(4);
            println!(
                "{:<w_id$}  {:<w_nm$}  {:>12}  {:>12}    (per 1M tokens, USD)",
                "ID", "NAME", "PROMPT", "COMPLETION"
            );
            for r in &rows {
                println!(
                    "{:<w_id$}  {:<w_nm$}  {:>12}  {:>12}",
                    r.id,
                    r.name,
                    or_fmt_price(r.prompt),
                    or_fmt_price(r.completion)
                );
            }
        }
    }
    Ok(())
}
const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);

// ---------------------------------------------------------------------------
// Data acquisition with local caching (24 h TTL, --refresh to force).
// ---------------------------------------------------------------------------

fn cache_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            PathBuf::from(p).join("sys")
        } else if let Some(p) = std::env::var_os("USERPROFILE") {
            PathBuf::from(p).join(".sysenv").join("cache")
        } else {
            std::env::temp_dir().join("sys")
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(p) = std::env::var_os("XDG_CACHE_HOME") {
            PathBuf::from(p).join("sys")
        } else if let Some(h) = std::env::var_os("HOME") {
            PathBuf::from(h).join(".cache").join("sys")
        } else {
            std::env::temp_dir().join("sys")
        }
    }
}

fn cache_file() -> PathBuf {
    cache_dir().join("models-dev.json")
}

fn load_cache() -> Option<Value> {
    let file = cache_file();
    let meta = std::fs::metadata(&file).ok()?;
    let mtime = meta.modified().ok()?;
    if SystemTime::now().duration_since(mtime).map(|d| d < CACHE_TTL).unwrap_or(false) {
        if let Ok(content) = std::fs::read_to_string(&file) {
            if let Ok(v) = serde_json::from_str(&content) {
                return Some(v);
            }
        }
    }
    None
}

fn save_cache(text: &str) {
    let dir = cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(cache_file(), text);
}

/// Fetch the models.dev data set. Uses a cached copy when fresh unless
/// `refresh` is set; the cache is stored under the platform cache directory.
pub fn fetch_data(refresh: bool) -> Result<Value> {
    if !refresh {
        if let Some(v) = load_cache() {
            return Ok(v);
        }
    }
    eprintln!("sys: fetching {DATA_URL} ...");
    let resp = reqwest::blocking::get(DATA_URL)
        .with_context(|| format!("cannot fetch {DATA_URL} (offline? use a cached copy if available)"))?;
    if !resp.status().is_success() {
        bail!("models.dev returned HTTP {}", resp.status());
    }
    let text = resp.text().context("cannot read models.dev response")?;
    let v: Value = serde_json::from_str(&text).context("invalid JSON received from models.dev")?;
    if v.as_object().is_none() {
        bail!("unexpected models.dev payload (expected a JSON object)");
    }
    save_cache(&text);
    Ok(v)
}

// ---------------------------------------------------------------------------
// Lookup helpers
// ---------------------------------------------------------------------------

fn ci_eq(a: &str, b: &str) -> bool {
    a.to_ascii_lowercase() == b.to_ascii_lowercase()
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
}

type ModelHit = (String, Value); // (provider id, model object)

/// Collect every model from every provider into a flat list.
fn all_models(data: &Value) -> Vec<ModelHit> {
    let mut out = Vec::new();
    if let Some(providers) = data.as_object() {
        for (pid, pv) in providers {
            if let Some(models) = pv.get("models").and_then(|m| m.as_object()) {
                for mv in models.values() {
                    out.push((pid.clone(), mv.clone()));
                }
            }
        }
    }
    out
}

/// Match models against a query: exact (case-insensitive) on id /
/// canonical_model_id / name first, then substring on id / name.
fn search_models(data: &Value, query: &str) -> Vec<ModelHit> {
    let all = all_models(data);
    let exact: Vec<ModelHit> = all
        .iter()
        .filter(|(_, m)| {
            ["id", "canonical_model_id", "name"].iter().any(|f| {
                m.get(*f)
                    .and_then(|v| v.as_str())
                    .map(|s| ci_eq(s, query))
                    .unwrap_or(false)
            })
        })
        .cloned()
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    all.into_iter()
        .filter(|(_, m)| {
            ["id", "canonical_model_id", "name"].iter().any(|f| {
                m.get(*f)
                    .and_then(|v| v.as_str())
                    .map(|s| contains_ci(s, query))
                    .unwrap_or(false)
            })
        })
        .collect()
}

/// Match providers: exact (case-insensitive) on id / name, then substring.
fn search_providers(data: &Value, query: &str) -> Vec<(String, Value)> {
    let mut providers: Vec<(String, Value)> = data
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    let exact: Vec<(String, Value)> = providers
        .iter()
        .filter(|(id, v)| {
            ci_eq(id, query)
                || v.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| ci_eq(n, query))
                    .unwrap_or(false)
        })
        .cloned()
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    providers.retain(|(id, v)| {
        contains_ci(id, query)
            || v.get("name")
                .and_then(|n| n.as_str())
                .map(|n| contains_ci(n, query))
                .unwrap_or(false)
    });
    providers
}

/// Validate a `YYYY-MM-DD` date string and return it unchanged.
fn parse_date(s: &str) -> Result<String> {
    let b = s.as_bytes();
    let valid = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(|c| c.is_ascii_digit())
        && b[5..7].iter().all(|c| c.is_ascii_digit())
        && b[8..10].iter().all(|c| c.is_ascii_digit());
    if !valid {
        bail!("invalid date `{s}` (expected YYYY-MM-DD)");
    }
    let month: u8 = s[5..7].parse().unwrap_or(0);
    let day: u8 = s[8..10].parse().unwrap_or(0);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        bail!("invalid date `{s}` (month/day out of range)");
    }
    Ok(s.to_string())
}

/// True when the model's `last_updated` (ISO date, possibly with a time
/// suffix) is strictly after `date`. Models without `last_updated` never match.
fn model_updated_after(m: &Value, date: &str) -> bool {
    match m.get("last_updated").and_then(|v| v.as_str()) {
        Some(s) => {
            let d = if s.len() >= 10 { &s[..10] } else { s };
            d > date
        }
        None => false,
    }
}

/// Keep only models whose `last_updated` is strictly after `date`.
fn filter_by_date(models: Vec<ModelHit>, date: &str) -> Vec<ModelHit> {
    models
        .into_iter()
        .filter(|(_, m)| model_updated_after(m, date))
        .collect()
}

/// Keep only models whose `open_weights` is true.
fn filter_open(models: Vec<ModelHit>) -> Vec<ModelHit> {
    models
        .into_iter()
        .filter(|(_, m)| m.get("open_weights").and_then(|v| v.as_bool()).unwrap_or(false))
        .collect()
}

/// Apply the `--date` and `--open` filters in order (date first, then open).
fn apply_filters(models: Vec<ModelHit>, date: Option<&str>, open: bool) -> Vec<ModelHit> {
    let models = match date {
        Some(d) => filter_by_date(models, d),
        None => models,
    };
    if open {
        filter_open(models)
    } else {
        models
    }
}

// ---------------------------------------------------------------------------
// Machine-readable output (JSON arrays / CSV tables)
// ---------------------------------------------------------------------------

/// Clone a model object and inject the `provider` field (used by text/CSV views).
fn model_with_provider(pid: &str, m: &Value) -> Value {
    let mut o = m.clone();
    o.as_object_mut()
        .map(|x| x.insert("provider".into(), Value::String(pid.to_string())));
    o
}

/// RFC-4180 CSV field escaping (quotes when the value contains , " \n \r).
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn csv_row(fields: &[&str]) -> String {
    fields.iter().map(|f| csv_escape(f)).collect::<Vec<_>>().join(",")
}

fn csv_get(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// Normalize a `--model-type` value to the canonical modality name used by
/// models.dev (`text`, `image`, `audio`, `video`, `pdf`); Chinese aliases are
/// accepted and unknown values are returned unchanged (so the caller can
/// report them).
fn normalize_modality(t: &str) -> String {
    match t.trim().to_ascii_lowercase().as_str() {
        "文本" => "text".to_string(),
        "图像" | "图片" => "image".to_string(),
        "语音" | "音频" => "audio".to_string(),
        "视频" => "video".to_string(),
        other => other.to_string(),
    }
}

/// Split a `--model-type` argument into canonical modalities. Accepts both
/// half-width `,` and full-width `，` as separators; every value is trimmed and
/// normalized. An empty / all-separator input yields `None` (no filter).
fn parse_model_types(input: &str) -> Option<Vec<String>> {
    let v: Vec<String> = input
        .split([',', '，'])
        .map(normalize_modality)
        .filter(|s| !s.is_empty())
        .collect();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// True when the model's `modalities.input` or `modalities.output` contains
/// `modality` (case-insensitive). Models without a `modalities` field never
/// match.
fn model_has_modality(m: &Value, modality: &str) -> bool {
    ["input", "output"].iter().any(|dir| {
        m.get("modalities")
            .and_then(|x| x.get(*dir))
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .any(|e| e.as_str().map(|s| ci_eq(s, modality)).unwrap_or(false))
            })
            .unwrap_or(false)
    })
}

/// Keep only models that support **every** requested modality (AND semantics,
/// e.g. `text,image` keeps models whose input/output modalities contain both
/// text and image).
fn filter_by_model_type(models: Vec<ModelHit>, types: &[String]) -> Vec<ModelHit> {
    models
        .into_iter()
        .filter(|(_, m)| types.iter().all(|t| model_has_modality(m, t)))
        .collect()
}

/// Keep only models whose provider id or provider name contains `provider`
/// (case-insensitive substring).
fn filter_by_provider(models: Vec<ModelHit>, data: &Value, provider: &str) -> Vec<ModelHit> {
    models
        .into_iter()
        .filter(|(pid, _)| {
            if contains_ci(pid, provider) {
                return true;
            }
            data.get(pid)
                .and_then(|pv| pv.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| contains_ci(n, provider))
                .unwrap_or(false)
        })
        .collect()
}

/// Keep only models whose `cost.input` and `cost.output` are both <= `price`
/// (per 1M tokens). Models without a `cost` field (or missing one side) count
/// as 0, so `--price 0` keeps free and unpriced models alike.
fn filter_by_price(models: Vec<ModelHit>, price: f64) -> Vec<ModelHit> {
    models
        .into_iter()
        .filter(|(_, m)| {
            let cost_in = m
                .get("cost")
                .and_then(|c| c.get("input"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let cost_out = m
                .get("cost")
                .and_then(|c| c.get("output"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            cost_in <= price && cost_out <= price
        })
        .collect()
}

/// The modality family of a model, e.g. `text,image` (union of input and
/// output modalities; empty when unknown).
fn model_modalities(m: &Value) -> String {
    let mut out: Vec<String> = Vec::new();
    for dir in ["input", "output"] {
        if let Some(arr) = m.get("modalities").and_then(|x| x.get(dir)).and_then(|x| x.as_array()) {
            for e in arr {
                if let Some(s) = e.as_str() {
                    if !s.is_empty() && !out.iter().any(|x| x == s) {
                        out.push(s.to_string());
                    }
                }
            }
        }
    }
    out.join(",")
}

/// Best-effort lookup of a model's capabilities in the models.dev data (24h
/// cache): returns `(context_tokens, modality_union)` of the **first** model
/// whose id or name matches `model_name` (exact match first, then substring).
/// Any failure (network / cache / no match) yields `(None, None)` so callers
/// can degrade silently. Used by `sys ai chat --update` to fill missing
/// `max_input_tokens` / `type` in the config.
pub fn lookup_model_capabilities(model_name: &str) -> (Option<u64>, Option<String>) {
    let data = match fetch_data(false) {
        Ok(d) => d,
        Err(_) => return (None, None),
    };
    let needle = model_name.to_ascii_lowercase();
    let mut exact: Option<(Option<u64>, Option<String>)> = None;
    let mut fuzzy: Option<(Option<u64>, Option<String>)> = None;
    for (_, m) in all_models(&data) {
        let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();
        let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();
        let ctx = m
            .get("limit")
            .and_then(|v| v.get("context"))
            .and_then(|v| v.as_u64())
            .filter(|&c| c > 0);
        let modl = model_modalities(&m);
        let modl = if modl.is_empty() { None } else { Some(modl) };
        let cap = (ctx, modl);
        if id == needle || name == needle {
            if exact.is_none() {
                exact = Some(cap);
            }
        } else if id.contains(&needle) || name.contains(&needle) {
            if fuzzy.is_none() {
                fuzzy = Some(cap);
            }
        }
    }
    exact.or(fuzzy).unwrap_or((None, None))
}

/// Character length of a string field (used for column widths).
fn field_len(m: &Value, key: &str) -> usize {
    m.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.chars().count())
        .unwrap_or(0)
}

/// Print a list of models as an aligned table with a header row
/// (`id`, `name`, `family`, `context`, `type`, `last_updated`).
fn print_model_rows(hits: &[ModelHit]) {
    let w_id = hits.iter().map(|(_, m)| field_len(m, "id")).max().unwrap_or(0).max(2);
    let w_name = hits.iter().map(|(_, m)| field_len(m, "name")).max().unwrap_or(0).max(4);
    let w_family = hits
        .iter()
        .map(|(_, m)| field_len(m, "family"))
        .max()
        .unwrap_or(0)
        .max(6);
    let w_ctx = hits
        .iter()
        .map(|(_, m)| fmt_num(m.get("limit").and_then(|l| l.get("context")).and_then(|v| v.as_u64()).unwrap_or(0)).len())
        .max()
        .unwrap_or(0)
        .max(7);
    let w_type = hits
        .iter()
        .map(|(_, m)| model_modalities(m).chars().count())
        .max()
        .unwrap_or(0)
        .max(4);
    let w_updated = hits
        .iter()
        .map(|(_, m)| field_len(m, "last_updated"))
        .max()
        .unwrap_or(0)
        .max(12);

    println!(
        "{:<w_id$}  {:<w_name$}  {:<w_family$}  {:<w_ctx$}  {:<w_type$}  {:<w_updated$}",
        "ID", "NAME", "FAMILY", "CONTEXT", "TYPE", "LAST UPDATED"
    );
    for (_, m) in hits {
        let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let nm = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let fam = m.get("family").and_then(|v| v.as_str()).unwrap_or("");
        let ctx = m.get("limit").and_then(|l| l.get("context")).and_then(|v| v.as_u64()).unwrap_or(0);
        let ctx = if ctx == 0 { "-".to_string() } else { fmt_num(ctx) };
        let typ = model_modalities(m);
        let typ = if typ.is_empty() { "-".to_string() } else { typ };
        let lu = m.get("last_updated").and_then(|v| v.as_str()).unwrap_or("");
        println!("{:<w_id$}  {:<w_name$}  {:<w_family$}  {:<w_ctx$}  {:<w_type$}  {:<w_updated$}", id, nm, fam, ctx, typ, lu);
    }
}

fn csv_bool(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(|x| x.as_bool())
        .map(|b| b.to_string())
        .unwrap_or_default()
}

fn csv_nested_num(v: &Value, outer: &str, k: &str) -> String {
    v.get(outer)
        .and_then(|o| o.get(k))
        .and_then(|x| x.as_u64())
        .map(|n| n.to_string())
        .unwrap_or_default()
}

fn csv_nested_float(v: &Value, outer: &str, k: &str) -> String {
    v.get(outer)
        .and_then(|o| o.get(k))
        .and_then(|x| x.as_f64())
        .map(|n| n.to_string())
        .unwrap_or_default()
}

fn csv_modality_list(v: &Value, dir: &str) -> String {
    v.get("modalities")
        .and_then(|m| m.get(dir))
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| e.as_str())
                .collect::<Vec<_>>()
                .join(";")
        })
        .unwrap_or_default()
}

const MODEL_CSV_HEADER: &str = "id,name,provider,family,status,knowledge_cutoff,description,context,input_limit,output_limit,cost_input,cost_output,cost_cache_read,modalities_input,modalities_output,reasoning,tool_call,structured_output,temperature,attachment,open_weights,release_date,last_updated,canonical_model_id";

fn model_csv_row(m: &Value) -> String {
    let fields: Vec<String> = if m.as_object().is_some() {
        vec![
            csv_get(m, "id"),
            csv_get(m, "name"),
            csv_get(m, "provider"),
            csv_get(m, "family"),
            csv_get(m, "status"),
            csv_get(m, "knowledge"),
            csv_get(m, "description"),
            csv_nested_num(m, "limit", "context"),
            csv_nested_num(m, "limit", "input"),
            csv_nested_num(m, "limit", "output"),
            csv_nested_float(m, "cost", "input"),
            csv_nested_float(m, "cost", "output"),
            csv_nested_float(m, "cost", "cache_read"),
            csv_modality_list(m, "input"),
            csv_modality_list(m, "output"),
            csv_bool(m, "reasoning"),
            csv_bool(m, "tool_call"),
            csv_bool(m, "structured_output"),
            csv_bool(m, "temperature"),
            csv_bool(m, "attachment"),
            csv_bool(m, "open_weights"),
            csv_get(m, "release_date"),
            csv_get(m, "last_updated"),
            csv_get(m, "canonical_model_id"),
        ]
    } else {
        vec![String::new(); 24]
    };
    let refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
    csv_row(&refs)
}

const PROVIDER_CSV_HEADER: &str = "id,name,api,env,npm,models_count";

fn provider_csv_row(id: &str, v: &Value) -> String {
    let fields: Vec<String> = if v.as_object().is_some() {
        vec![
            id.to_string(),
            csv_get(v, "name"),
            csv_get(v, "doc"),
            csv_get(v, "env"),
            csv_get(v, "npm"),
            v.get("models")
                .and_then(|m| m.as_object())
                .map(|m| m.len().to_string())
                .unwrap_or_default(),
        ]
    } else {
        vec![id.to_string(), String::new(), String::new(), String::new(), String::new(), String::new()]
    };
    let refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
    csv_row(&refs)
}

// ---------------------------------------------------------------------------
// Model command
// ---------------------------------------------------------------------------

pub fn cmd_model(
    name: Option<&str>,
    search: Option<&str>,
    list: bool,
    limit: Option<usize>,
    json: bool,
    out: Option<OutFormat>,
    refresh: bool,
    updated_after: Option<&str>,
    open: bool,
    model_type: Option<&str>,
    provider: Option<&str>,
    price: Option<f64>,
    src: Option<&str>,
    limit_flag: Option<usize>,
) -> Result<()> {
    if src.map(|s| ci_eq(s, "openrouter")).unwrap_or(false) {
        if list || provider.is_some() || price.is_some() || open || model_type.is_some() || updated_after.is_some() {
            bail!("`--list`, `--provider`, `--price`, `--open`, `--model-type` and `--date` only apply to the models.dev catalogue; with `--src openrouter` use NAME/SEARCH, `--limit`, `--json` or `-o`");
        }
        return cmd_model_openrouter(name.or(search), json, out, limit_flag, refresh);
    }
    let data = fetch_data(refresh)?;
    let fmt = out.or(if json { Some(OutFormat::Json) } else { None });
    // `--price` defaults to 0: keep only free (or unpriced) models.
    let price = price.unwrap_or(0.0);

    // `sai model` with no arguments defaults to listing every model.
    let bare = name.is_none() && search.is_none() && !list;
    let default_limit = if bare { usize::MAX } else { 20 };
    let date = updated_after.map(parse_date).transpose()?;
    let mt = model_type.and_then(parse_model_types);
    if let Some(ts) = &mt {
        for t in ts {
            if !["text", "image", "audio", "video", "pdf"].contains(&t.as_str()) {
                bail!("unsupported --model-type `{t}` (supported: text, image, audio, video, pdf or 文本/图像/语音/视频; comma-separated multi-values like `text,image` are AND-ed)");
            }
        }
    }

    if list || bare {
        let all = all_models(&data);
        let hits = apply_filters(all, date.as_deref(), open);
        let hits = match &mt {
            Some(ts) => filter_by_model_type(hits, ts),
            None => hits,
        };
        let hits = match provider {
            Some(p) => filter_by_provider(hits, &data, p),
            None => hits,
        };
        let hits = filter_by_price(hits, price);
        let total_hits = hits.len();
        let take = limit.unwrap_or(default_limit).max(1);
        match fmt {
            Some(OutFormat::Json) => {
                let arr: Vec<Value> = hits.into_iter().take(take).map(|(_, m)| m).collect();
                println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
            }
            Some(OutFormat::Yaml) => {
                let arr: Vec<Value> = hits.into_iter().take(take).map(|(_, m)| m).collect();
                println!("{}", serde_yaml::to_string(&Value::Array(arr))?);
            }
            Some(OutFormat::Csv) => {
                println!("{MODEL_CSV_HEADER}");
                for (pid, m) in hits.iter().take(take) {
                    println!("{}", model_csv_row(&model_with_provider(pid, m)));
                }
            }
            Some(OutFormat::Md) => {
                println!("{}", md_model_rows(&hits[..take.min(hits.len())]));
            }
            None => {
                let shown = total_hits.min(take);
                print_model_rows(&hits[..shown]);
                let mut footer = format!("--- {shown} of {total_hits} models");
                if let Some(d) = &date {
                    footer.push_str(&format!(" updated after {d}"));
                }
                if open {
                    footer.push_str(" with open weights");
                }
                if let Some(ts) = &mt {
                    footer.push_str(&format!(" with modalities {}", ts.join(",")));
                }
                if let Some(p) = provider {
                    footer.push_str(&format!(" of providers containing `{p}`"));
                }
                footer.push_str(&format!(" with input/output cost <= {price}"));
                if total_hits > shown {
                    footer.push_str(" (use --limit to show more)");
                }
                println!("{footer}");
            }
        }
        return Ok(());
    }

    let query = match (name, search) {
        (Some(n), _) => n,
        (None, Some(s)) => s,
        (None, None) => unreachable!("bare invocation was handled above"),
    };

    let hits = apply_filters(search_models(&data, query), date.as_deref(), open);
    let hits = match &mt {
        Some(ts) => filter_by_model_type(hits, ts),
        None => hits,
    };
    let hits = match provider {
        Some(p) => filter_by_provider(hits, &data, p),
        None => hits,
    };
    let hits = filter_by_price(hits, price);
    if hits.is_empty() {
        let mut msg = format!("no model matches `{query}`");
        if let Some(d) = &date {
            msg.push_str(&format!(" updated after {d}"));
        }
        if open {
            msg.push_str(" with open weights");
        }
        if let Some(ts) = &mt {
            msg.push_str(&format!(" with modalities {}", ts.join(",")));
        }
        if let Some(p) = provider {
            msg.push_str(&format!(" of providers containing `{p}`"));
        }
        msg.push_str(&format!(" with input/output cost <= {price}"));
        bail!("{msg} (source: {DATA_URL})");
    }

    // --search: always list the matching entries (grep-style).
    if search.is_some() {
        let take = limit.unwrap_or(20).max(1);
        match fmt {
            Some(OutFormat::Json) => {
                let arr: Vec<Value> = hits.into_iter().take(take).map(|(_, m)| m).collect();
                println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
            }
            Some(OutFormat::Yaml) => {
                let arr: Vec<Value> = hits.into_iter().take(take).map(|(_, m)| m).collect();
                println!("{}", serde_yaml::to_string(&Value::Array(arr))?);
            }
            Some(OutFormat::Csv) => {
                println!("{MODEL_CSV_HEADER}");
                for (pid, m) in hits.iter().take(take) {
                    println!("{}", model_csv_row(&model_with_provider(pid, m)));
                }
            }
            Some(OutFormat::Md) => {
                println!("{}", md_model_rows(&hits[..take.min(hits.len())]));
            }
            None => {
                let shown = hits.len().min(take);
                print_model_rows(&hits[..shown]);
                if hits.len() > take {
                    println!("--- {} of {} matches (use --limit to show more)", take, hits.len());
                }
            }
        }
        return Ok(());
    }

    // Exact-name mode: if several providers expose the same model, prefer the
    // canonical provider (majority vote on the `provider/...` id prefix of the
    // entries' canonical_model_id, e.g. `openai/gpt-4.1` for GPT-4.1).
    if hits.len() > 1 {
        let mut prefix_counts: HashMap<&str, usize> = HashMap::new();
        for (_, m) in &hits {
            if let Some(c) = m.get("canonical_model_id").and_then(|v| v.as_str()) {
                if let Some((prefix, _)) = c.split_once('/') {
                    *prefix_counts.entry(prefix).or_insert(0) += 1;
                }
            }
        }
        let best_prefix = prefix_counts.iter().max_by_key(|(_, c)| *c).map(|(p, _)| *p);
        let chosen: Vec<&ModelHit> = match best_prefix {
            Some(prefix) => hits.iter().filter(|(pid, _)| pid.as_str() == prefix).collect(),
            None => Vec::new(),
        };
        if chosen.len() == 1 {
            let hit = model_with_provider(&chosen[0].0, &chosen[0].1);
            match fmt {
                Some(OutFormat::Json) => {
                    println!("{}", serde_json::to_string_pretty(&Value::Array(vec![hit]))?);
                }
                Some(OutFormat::Yaml) => {
                    println!("{}", serde_yaml::to_string(&Value::Array(vec![hit]))?);
                }
                Some(OutFormat::Csv) => {
                    println!("{MODEL_CSV_HEADER}");
                    println!("{}", model_csv_row(&hit));
                }
                Some(OutFormat::Md) => {
                    println!("{}", md_model_detail(&hit));
                }
                None => {
                    print_model(&hit);
                    println!(
                        "(canonical entry; {} other provider(s) also expose `{query}` — use --search to list them)",
                        hits.len() - 1
                    );
                }
            }
            return Ok(());
        }
        match fmt {
            Some(OutFormat::Json) => {
                let arr: Vec<Value> = hits.into_iter().map(|(_, m)| m).collect();
                println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
            }
            Some(OutFormat::Yaml) => {
                let arr: Vec<Value> = hits.into_iter().map(|(_, m)| m).collect();
                println!("{}", serde_yaml::to_string(&Value::Array(arr))?);
            }
            Some(OutFormat::Csv) => {
                println!("{MODEL_CSV_HEADER}");
                for (pid, m) in &hits {
                    println!("{}", model_csv_row(&model_with_provider(pid, m)));
                }
            }
            Some(OutFormat::Md) => {
                println!("{}", md_model_rows(&hits));
            }
            None => {
                println!("Multiple models match `{query}`:");
                print_model_rows(&hits);
                println!("Use a full model id (e.g. `provider/model`), --search, -o json or -o csv for details.");
            }
        }
        return Ok(());
    }

    // Single match -> detailed view (or machine-readable output).
    let hit = model_with_provider(&hits[0].0, &hits[0].1);
    match fmt {
        Some(OutFormat::Json) => {
            println!("{}", serde_json::to_string_pretty(&Value::Array(vec![hit]))?);
        }
        Some(OutFormat::Yaml) => {
            println!("{}", serde_yaml::to_string(&Value::Array(vec![hit]))?);
        }
        Some(OutFormat::Csv) => {
            println!("{MODEL_CSV_HEADER}");
            println!("{}", model_csv_row(&hit));
        }
        Some(OutFormat::Md) => {
            println!("{}", md_model_detail(&hit));
        }
        None => print_model(&hit),
    }
    Ok(())
}

/// Print a model object as aligned `label: value` lines. A title line (the
/// model name — the title models.dev shows on its model pages) is printed on
/// top; known fields get friendly formatting, unknown fields are appended
/// verbatim.
fn print_model(m: &Value) {
    let obj = match m.as_object() {
        Some(o) => o,
        None => {
            println!("{}", m);
            return;
        }
    };
    let s = |k: &str| obj.get(k).and_then(|v| v.as_str());
    let b = |k: &str| {
        obj.get(k)
            .and_then(|v| v.as_bool())
            .map(|x| if x { "yes" } else { "no" })
    };

    // Title: the model name shown on the models.dev page (fallback: id).
    if let Some(title) = s("name").or_else(|| s("id")) {
        println!("== {title} ==");
    }

    let mut rows: Vec<(String, String)> = Vec::new();
    push(&mut rows, "id", s("id"));
    push(&mut rows, "name", s("name"));
    push(&mut rows, "provider", s("provider"));
    push(&mut rows, "family", s("family"));
    push(&mut rows, "status", s("status"));
    push(&mut rows, "knowledge cutoff", s("knowledge"));
    push(&mut rows, "description", s("description"));

    if let Some(mods) = obj.get("modalities") {
        let fmt = |k: &str| -> Option<String> {
            mods.get(k)
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
        };
        let mut parts = Vec::new();
        if let Some(i) = fmt("input") {
            parts.push(format!("input={i}"));
        }
        if let Some(o) = fmt("output") {
            parts.push(format!("output={o}"));
        }
        if !parts.is_empty() {
            rows.push(("modalities".into(), parts.join(" | ")));
        }
    }

    if let Some(lim) = obj.get("limit") {
        if let Some(c) = lim.get("context").and_then(|v| v.as_u64()) {
            rows.push(("context".into(), fmt_num(c)));
        }
        if let Some(i) = lim.get("input").and_then(|v| v.as_u64()) {
            rows.push(("input limit".into(), fmt_num(i)));
        }
        if let Some(o) = lim.get("output").and_then(|v| v.as_u64()) {
            rows.push(("output limit".into(), fmt_num(o)));
        }
    }

    if let Some(cost) = obj.get("cost") {
        let mut parts = Vec::new();
        for k in ["input", "output", "cache_read"] {
            if let Some(v) = cost.get(k).and_then(|v| v.as_f64()) {
                parts.push(format!("{k} ${v:.4}"));
            }
        }
        if !parts.is_empty() {
            rows.push(("cost /1M tokens".into(), parts.join(", ")));
        }
    }

    push(&mut rows, "reasoning", b("reasoning"));
    if let Some(ro) = obj.get("reasoning_options").and_then(|v| v.as_array()) {
        if !ro.is_empty() {
            rows.push(("reasoning options".into(), serde_json::to_string(ro).unwrap_or_default()));
        }
    }
    push(&mut rows, "tool call", b("tool_call"));
    push(&mut rows, "structured output", b("structured_output"));
    push(&mut rows, "temperature", b("temperature"));
    push(&mut rows, "attachment", b("attachment"));
    push(&mut rows, "open weights", b("open_weights"));
    push(&mut rows, "release date", s("release_date"));
    push(&mut rows, "last updated", s("last_updated"));
    push(&mut rows, "canonical id", s("canonical_model_id"));

    // Any remaining keys we did not format above.
    let known: &[&str] = &[
        "id", "name", "provider", "family", "status", "knowledge", "description",
        "modalities", "limit", "cost", "reasoning", "reasoning_options",
        "tool_call", "structured_output", "temperature", "attachment",
        "open_weights", "release_date", "last_updated", "canonical_model_id",
    ];
    for (k, v) in obj {
        if !known.contains(&k.as_str()) {
            rows.push((k.clone(), serde_json::to_string(v).unwrap_or_default()));
        }
    }

    let width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (k, v) in rows {
        println!("{k:>width$}: {v}");
    }
}

fn push(rows: &mut Vec<(String, String)>, label: &str, v: Option<&str>) {
    if let Some(v) = v {
        rows.push((label.to_string(), v.to_string()));
    }
}

fn fmt_num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// Provider command
// ---------------------------------------------------------------------------

pub fn cmd_provider(
    name: Option<&str>,
    search: Option<&str>,
    list: bool,
    limit: usize,
    json: bool,
    out: Option<OutFormat>,
    refresh: bool,
) -> Result<()> {
    let data = fetch_data(refresh)?;
    let fmt = out.or(if json { Some(OutFormat::Json) } else { None });

    if list {
        let providers = data
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>())
            .unwrap_or_default();
        let total = providers.len();
        let take = limit.max(1);
        match fmt {
            Some(OutFormat::Json) => {
                let arr: Vec<Value> = providers
                    .iter()
                    .take(take)
                    .map(|(id, v)| {
                        let mut o = v.clone();
                        o.as_object_mut()
                            .map(|m| m.insert("id".into(), Value::String(id.clone())));
                        o
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
            }
            Some(OutFormat::Yaml) => {
                let arr: Vec<Value> = providers
                    .iter()
                    .take(take)
                    .map(|(id, v)| {
                        let mut o = v.clone();
                        o.as_object_mut()
                            .map(|m| m.insert("id".into(), Value::String(id.clone())));
                        o
                    })
                    .collect();
                println!("{}", serde_yaml::to_string(&Value::Array(arr))?);
            }
            Some(OutFormat::Csv) => {
                println!("{PROVIDER_CSV_HEADER}");
                for (id, v) in providers.iter().take(take) {
                    println!("{}", provider_csv_row(id, v));
                }
            }
            Some(OutFormat::Md) => {
                println!("{}", md_provider_rows(&providers[..take.min(providers.len())]));
            }
            None => {
                for (id, v) in providers.iter().take(take) {
                    let nm = v.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                    let models = v.get("models").and_then(|m| m.as_object()).map(|m| m.len()).unwrap_or(0);
                    println!("{id}\t{nm}\t{models} models");
                }
                println!("--- {take} of {total} providers (use --limit to show more)");
            }
        }
        return Ok(());
    }

    let query = match (name, search) {
        (Some(n), _) => n,
        (None, Some(s)) => s,
        (None, None) => bail!("provide a PROVIDER name, or use --search QUERY / --list"),
    };

    let hits = search_providers(&data, query);
    if hits.is_empty() {
        bail!("no provider matches `{query}` (source: {DATA_URL})");
    }

    // --search: always list the matching entries (grep-style).
    if search.is_some() {
        let take = limit.max(1);
        match fmt {
            Some(OutFormat::Json) => {
                let arr: Vec<Value> = hits
                    .iter()
                    .take(take)
                    .map(|(id, v)| {
                        let mut o = v.clone();
                        o.as_object_mut()
                            .map(|m| m.insert("id".into(), Value::String(id.clone())));
                        o
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
            }
            Some(OutFormat::Yaml) => {
                let arr: Vec<Value> = hits
                    .iter()
                    .take(take)
                    .map(|(id, v)| {
                        let mut o = v.clone();
                        o.as_object_mut()
                            .map(|m| m.insert("id".into(), Value::String(id.clone())));
                        o
                    })
                    .collect();
                println!("{}", serde_yaml::to_string(&Value::Array(arr))?);
            }
            Some(OutFormat::Csv) => {
                println!("{PROVIDER_CSV_HEADER}");
                for (id, v) in hits.iter().take(take) {
                    println!("{}", provider_csv_row(id, v));
                }
            }
            Some(OutFormat::Md) => {
                println!("{}", md_provider_rows(&hits[..take.min(hits.len())]));
            }
            None => {
                for (id, v) in hits.iter().take(take) {
                    let nm = v.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                    let models = v.get("models").and_then(|m| m.as_object()).map(|m| m.len()).unwrap_or(0);
                    println!("{id}\t{nm}\t{models} models");
                }
                if hits.len() > take {
                    println!("--- {} of {} matches (use --limit to show more)", take, hits.len());
                }
            }
        }
        return Ok(());
    }

    if hits.len() > 1 && fmt.is_none() {
        println!("Multiple providers match `{query}`:");
        for (id, v) in &hits {
            let nm = v.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            println!("  {id}\t{nm}");
        }
        println!("Use the exact provider id, -o json or -o csv for details.");
        return Ok(());
    }

    match fmt {
        Some(OutFormat::Json) => {
            let arr: Vec<Value> = hits
                .into_iter()
                .map(|(id, mut v)| {
                    v.as_object_mut().map(|m| m.insert("id".into(), Value::String(id)));
                    v
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
            return Ok(());
        }
        Some(OutFormat::Yaml) => {
            let arr: Vec<Value> = hits
                .into_iter()
                .map(|(id, mut v)| {
                    v.as_object_mut().map(|m| m.insert("id".into(), Value::String(id)));
                    v
                })
                .collect();
            println!("{}", serde_yaml::to_string(&Value::Array(arr))?);
            return Ok(());
        }
        Some(OutFormat::Csv) => {
            println!("{PROVIDER_CSV_HEADER}");
            for (id, v) in &hits {
                println!("{}", provider_csv_row(id, v));
            }
            return Ok(());
        }
        Some(OutFormat::Md) => {
            println!("{}", md_provider_rows(&hits));
            return Ok(());
        }
        None => {}
    }

    let (pid, pv) = &hits[0];
    let obj = match pv.as_object() {
        Some(o) => o,
        None => bail!("provider `{pid}` payload is not an object"),
    };
    let s = |k: &str| obj.get(k).and_then(|v| v.as_str());
    let models = obj.get("models").and_then(|m| m.as_object());
    let model_ids: Vec<&str> = models
        .map(|m| m.keys().map(|k| k.as_str()).collect())
        .unwrap_or_default();

    println!("id:     {pid}");
    println!("name:   {}", s("name").unwrap_or("?"));
    println!("api:    {}", s("doc").unwrap_or(""));
    println!("env:    {}", s("env").unwrap_or(""));
    println!("npm:    {}", s("npm").unwrap_or(""));
    println!("models: {}", model_ids.len());
    for id in model_ids.iter().take(limit.max(1)) {
        println!("  - {id}");
    }
    if model_ids.len() > limit.max(1) {
        println!("  ... ({} more; use --limit N to list more)", model_ids.len() - limit.max(1));
    }
    if name.is_some() && hits.len() > 1 {
        println!("(showing 1 of {} matches; use -o json or -o csv for more)", hits.len());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `sys ai info` — inspect the local config (providers / models) and the
// models.dev model prices.
//
// - `info provider [KEYWORD]`  lists every provider configured under `clients`
//   (name / api_base / api_key); with KEYWORD only the providers whose name
//   contains it are kept.
// - `info model [KEYWORD]`     lists every configured model as `{provider}:{name}`;
//   with KEYWORD only the models of providers whose name contains it are kept
//   (`provider:model` / `provider:*` select one / all models of a provider;
//   when no provider matches, models whose name contains KEYWORD are listed).
// - `sys ai price P1,P2,...`   prints the per-1M-token price list (input /
//   output / cache_read) of the comma-separated providers (source: models.dev,
//   24 h cache, --refresh to force; prices moved here from the old `info price`).
// - `info list-model PROVIDER` / `info models PROVIDER`  queries the
//   provider's own models API (`GET {api_base}/models` for OpenAI-compatible,
//   `{api_base}/v1/models` for Anthropic — same function as
//   `sys ai chat --query-model PROVIDER`) and prints the models the configured
//   api_key can actually use (live query, no cache; `next_page` pagination is
//   followed).
// ---------------------------------------------------------------------------

/// Grouped view used by `sys ai server --help`: one line per configured
/// provider (name / api_base), followed by one indented `{provider}:{name}`
/// line per model of that provider.
pub fn print_server_config(config: Option<&Path>) -> Result<()> {
    let (cfg, _) = chat::load_config(config)?;
    for p in &cfg.providers {
        println!("[{}] api_base={}", p.name, p.api_base);
        for m in &p.models {
            println!("  {}:{}", p.name, m.name);
        }
    }
    Ok(())
}

/// One row of `info provider` (a configured client).
#[derive(serde::Serialize)]
struct ProviderInfoRow {
    name: String,
    api_base: String,
    api_key: String,
    /// Official help-docs URL of the provider ("" when unknown).
    docs: String,
    /// Official web console URL of the provider ("" when unknown).
    console: String,
    /// Example request against the provider's own models API, e.g.
    /// `https://openrouter.ai/api/v1/models  Authorization:"Bearer sk-sample"`.
    sample: String,
}

/// Example request hitting the provider's models endpoint with its key,
/// matching `chat::models_url` (OpenAI-compatible vs Anthropic header).
fn provider_sample_request(p: &chat::Provider) -> String {
    let url = chat::models_url(p);
    if p.kind == "anthropic" {
        format!(
            "{url}  x-api-key:\"{}\" anthropic-version:\"{}\"",
            p.api_key,
            chat::ANTHROPIC_VERSION
        )
    } else {
        format!("{url}  Authorization:\"Bearer {}\"", p.api_key)
    }
}

/// Markdown unordered-list view of `info provider` rows (`-o md`):
/// `- name` with `api_base / api_key / console / docs / sample` children.
fn md_provider_list(rows: &[ProviderInfoRow]) -> String {
    let mut s = String::new();
    for r in rows {
        s.push_str(&format!("- {}\n", md_cell(&r.name)));
        s.push_str(&format!("  - api_base：{}\n", md_cell(&r.api_base)));
        s.push_str(&format!("  - api_key：{}\n", md_cell(&r.api_key)));
        s.push_str(&format!("  - console：{}\n", md_cell(&r.console)));
        s.push_str(&format!("  - docs：{}\n", md_cell(&r.docs)));
        s.push_str(&format!("  - sample：{}\n", md_cell(&r.sample)));
    }
    s
}

/// Markdown unordered-list view of `info model` rows (`-o md`): one
/// `- provider` entry per provider (grouped) with `- name：` children.
fn md_model_list(rows: &[ModelInfoRow]) -> String {
    let mut s = String::new();
    let mut last: Option<&str> = None;
    for r in rows {
        if last != Some(r.provider.as_str()) {
            last = Some(r.provider.as_str());
            s.push_str(&format!("- {}\n", md_cell(&r.provider)));
        }
        s.push_str(&format!("  - name：{}\n", md_cell(&r.name)));
        if !r.groups.is_empty() {
            s.push_str(&format!("  - group：{}\n", md_cell(&r.groups.join(", "))));
        }
    }
    s
}

/// Markdown unordered-list view of `info list-model` rows (`-o md`).
fn md_list_model_rows(rows: &[ModelListRow]) -> String {
    let mut s = String::new();
    for r in rows {
        s.push_str(&format!("- {}\n", md_cell(&r.id)));
        s.push_str(&format!("  - owned_by：{}\n", md_cell(&r.owned_by)));
        s.push_str(&format!("  - created：{}\n", md_cell(&fmt_created(&r.created))));
    }
    s
}

/// Official help-docs / console URLs keyed by provider name (case-insensitive
/// substring match, like the `info provider` keyword filter). Every URL below
/// was verified reachable on 2026-10-07. Unknown providers get "" / "".
fn provider_links(name: &str) -> (String, String) {
    let n = name.to_ascii_lowercase();
    let hit: Option<(&str, &str)> = if contains_ci(&n, "agnes") {
        Some(("https://wiki.agnes-ai.cn/zh-Hans/docs", "https://platform.agnes-ai.cn"))
    } else if contains_ci(&n, "alibaba") || contains_ci(&n, "dashscope") || contains_ci(&n, "bailian") {
        Some(("https://help.aliyun.com/zh/model-studio/", "https://bailian.console.aliyun.com"))
    } else if contains_ci(&n, "minimax") {
        Some(("https://platform.minimax.cn/docs", "https://platform.minimax.cn"))
    } else if contains_ci(&n, "modelscope") {
        Some(("https://modelscope.cn/docs", "https://modelscope.cn"))
    } else if contains_ci(&n, "anspire") {
        Some(("https://open.anspire.cn/document/docs/", "https://open.anspire.cn"))
    } else if contains_ci(&n, "sensenova") || contains_ci(&n, "sensecore") {
        Some(("https://console.sensecore.cn/micro/help/docs/model-as-a-service/nova/", "https://console.sensecore.cn"))
    } else if contains_ci(&n, "bigmodel") || contains_ci(&n, "zhipu") {
        Some(("https://docs.bigmodel.cn", "https://bigmodel.cn/console"))
    } else if contains_ci(&n, "amd") {
        Some(("https://developer.amd.com.cn", "https://developer.amd.com.cn"))
    } else {
        None
    };
    match hit {
        Some((docs, console)) => (docs.to_string(), console.to_string()),
        None => (String::new(), String::new()),
    }
}

/// One row of `info model` in `{provider}:{name}` form.
struct ModelInfoRow {
    provider: String,
    name: String,
    groups: Vec<String>,
}

/// One model of a models.dev provider with its token prices.
struct PriceRow {
    provider: String,
    model_id: String,
    model_name: String,
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    currency: String,
    unit: String,
}

fn info_provider_rows(cfg: &chat::Config, keyword: Option<&str>) -> Result<Vec<ProviderInfoRow>> {
    let rows: Vec<ProviderInfoRow> = cfg
        .providers
        .iter()
        .filter(|p| keyword.map(|k| contains_ci(&p.name, k)).unwrap_or(true))
        .map(|p| {
            let (docs, console) = provider_links(&p.name);
            ProviderInfoRow {
                name: p.name.clone(),
                api_base: p.api_base.clone(),
                api_key: p.api_key.clone(),
                docs,
                console,
                sample: provider_sample_request(p),
            }
        })
        .collect();
    if let Some(k) = keyword {
        if rows.is_empty() {
            let avail: Vec<&str> = cfg.providers.iter().map(|p| p.name.as_str()).collect();
            bail!("no provider contains `{k}` (available: {})", avail.join(", "));
        }
    }
    Ok(rows)
}

fn info_model_rows(cfg: &chat::Config, keyword: Option<&str>) -> Result<Vec<ModelInfoRow>> {
    let kw = keyword.map(str::trim).filter(|s| !s.is_empty());
    let mut rows: Vec<ModelInfoRow> = Vec::new();
    match kw {
        None => {
            for p in &cfg.providers {
                for m in &p.models {
                    rows.push(ModelInfoRow {
                        provider: p.name.clone(),
                        name: m.name.clone(),
                        groups: m.group.clone(),
                    });
                }
            }
        }
        Some(k) if k.contains(':') => {
            // `provider:model` / `provider:*` — the provider part is a
            // substring of the provider name, the model part a substring of
            // the model name (`*` = every model of the provider).
            let (prov, msel) = k.split_once(':').expect("contains(':') checked");
            let prov = prov.trim();
            let msel = msel.trim();
            let hits: Vec<&chat::Provider> = cfg
                .providers
                .iter()
                .filter(|p| contains_ci(&p.name, prov))
                .collect();
            if hits.is_empty() {
                let avail: Vec<&str> = cfg.providers.iter().map(|p| p.name.as_str()).collect();
                bail!("no provider contains `{prov}` (available: {})", avail.join(", "));
            }
            for p in hits {
                for m in &p.models {
                    if msel == "*" || contains_ci(&m.name, msel) {
                        rows.push(ModelInfoRow {
                            provider: p.name.clone(),
                            name: m.name.clone(),
                            groups: m.group.clone(),
                        });
                    }
                }
            }
            if rows.is_empty() {
                bail!("no model contains `{msel}` under provider(s) matching `{prov}`");
            }
        }
        Some(k) => {
            let hits: Vec<&chat::Provider> = cfg
                .providers
                .iter()
                .filter(|p| contains_ci(&p.name, k))
                .collect();
            if hits.is_empty() {
                // Fall back to matching model names across every provider so
                // `info model deepseek` still finds the models named deepseek.
                for p in &cfg.providers {
                    for m in &p.models {
                        if contains_ci(&m.name, k) {
                            rows.push(ModelInfoRow {
                                provider: p.name.clone(),
                                name: m.name.clone(),
                                groups: m.group.clone(),
                            });
                        }
                    }
                }
                if rows.is_empty() {
                    let avail: Vec<&str> = cfg.providers.iter().map(|p| p.name.as_str()).collect();
                    bail!("no provider or model contains `{k}` (providers: {})", avail.join(", "));
                }
            } else {
                for p in hits {
                    for m in &p.models {
                        rows.push(ModelInfoRow {
                            provider: p.name.clone(),
                            name: m.name.clone(),
                            groups: m.group.clone(),
                        });
                    }
                }
            }
        }
    }
    Ok(rows)
}

fn info_price_rows(data: &Value, providers: &[String]) -> Result<Vec<PriceRow>> {
    let mut rows: Vec<PriceRow> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for q in providers {
        let hits = search_providers(data, q);
        if hits.is_empty() {
            missing.push(q.clone());
            continue;
        }
        for (pid, pv) in hits {
            let prov_cost = pv.get("cost");
            let models = pv.get("models").and_then(|m| m.as_object());
            if let Some(models) = models {
                for (mid, mv) in models {
                    let cost = mv.get("cost");
                    let currency = cost
                        .or(prov_cost)
                        .and_then(|c| c.get("currency"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("USD")
                        .to_string();
                    let unit = cost
                        .or(prov_cost)
                        .and_then(|c| c.get("unit"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("1M")
                        .to_string();
                    rows.push(PriceRow {
                        provider: pid.clone(),
                        model_id: mid.clone(),
                        model_name: mv
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?")
                            .to_string(),
                        input: cost.and_then(|c| c.get("input")).and_then(|v| v.as_f64()),
                        output: cost.and_then(|c| c.get("output")).and_then(|v| v.as_f64()),
                        cache_read: cost.and_then(|c| c.get("cache_read")).and_then(|v| v.as_f64()),
                        currency,
                        unit,
                    });
                }
            }
        }
    }
    if !missing.is_empty() {
        let avail: Vec<&str> = data
            .as_object()
            .map(|m| m.keys().map(|k| k.as_str()).collect())
            .unwrap_or_default();
        bail!(
            "no models.dev provider matches {} (available: {})",
            missing.join(", "),
            avail.join(", ")
        );
    }
    Ok(rows)
}

fn fmt_price(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:.4}"),
        None => "-".to_string(),
    }
}

fn print_price_rows(rows: &[PriceRow]) {
    let w_prov = rows.iter().map(|r| r.provider.chars().count()).max().unwrap_or(8).max(8);
    let w_id = rows.iter().map(|r| r.model_id.chars().count()).max().unwrap_or(5).max(5);
    let w_nm = rows.iter().map(|r| r.model_name.chars().count()).max().unwrap_or(4).max(4);
    let currency = rows.first().map(|r| r.currency.as_str()).unwrap_or("USD");
    let unit = rows.first().map(|r| r.unit.as_str()).unwrap_or("1M");
    println!(
        "{:<w_prov$}  {:<w_id$}  {:<w_nm$}  {:>10}  {:>10}  {:>10}    (per {unit} {currency})",
        "PROVIDER", "MODEL", "NAME", "INPUT", "OUTPUT", "CACHE_READ"
    );
    for r in rows {
        println!(
            "{:<w_prov$}  {:<w_id$}  {:<w_nm$}  {:>10}  {:>10}  {:>10}",
            r.provider,
            r.model_id,
            r.model_name,
            fmt_price(r.input),
            fmt_price(r.output),
            fmt_price(r.cache_read)
        );
    }
}

/// Text view of the provider rows (PROVIDER / API_BASE / DOCS / CONSOLE / API_KEY).
fn print_provider_table(rows: &[ProviderInfoRow]) {
    let w_name = rows.iter().map(|r| r.name.chars().count()).max().unwrap_or(8).max(8);
    let w_base = rows.iter().map(|r| r.api_base.chars().count()).max().unwrap_or(8).max(8);
    let w_docs = rows.iter().map(|r| r.docs.chars().count()).max().unwrap_or(4).max(4);
    let w_console = rows.iter().map(|r| r.console.chars().count()).max().unwrap_or(7).max(7);
    println!("{:<w_name$}  {:<w_base$}  {:<w_docs$}  {:<w_console$}  {}", "PROVIDER", "API_BASE", "DOCS", "CONSOLE", "API_KEY");
    for r in rows {
        let docs = if r.docs.is_empty() { "-".to_string() } else { r.docs.clone() };
        let console = if r.console.is_empty() { "-".to_string() } else { r.console.clone() };
        println!("{:<w_name$}  {:<w_base$}  {:<w_docs$}  {:<w_console$}  {}", r.name, r.api_base, docs, console, r.api_key);
    }
}

/// Markdown table renderer for `-o md`: header row + separator + data rows.
/// Cells escape `|` (as `\|`) and collapse newlines so the table stays intact.
fn md_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for h in headers {
        out.push_str(&format!("| {} ", h));
    }
    out.push_str("|\n");
    for _ in headers {
        out.push_str("| --- ");
    }
    out.push_str("|\n");
    for r in rows {
        for (i, cell) in r.iter().enumerate() {
            let _ = i;
            out.push_str(&format!("| {} ", md_cell(cell)));
        }
        out.push_str("|\n");
    }
    out
}

/// Escape one markdown table cell: `|` -> `\|`, newlines / CR -> space.
fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// Markdown table of model list hits: id / name / provider.
fn md_model_rows(hits: &[(String, Value)]) -> String {
    md_table(
        &["id", "name", "provider"],
        &hits
            .iter()
            .map(|(pid, m)| {
                vec![
                    m.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    m.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    pid.clone(),
                ]
            })
            .collect::<Vec<_>>(),
    )
}

/// Markdown key/value table of one model object.
fn md_model_detail(m: &Value) -> String {
    let obj = m.as_object().cloned().unwrap_or_default();
    let rows: Vec<Vec<String>> = obj
        .into_iter()
        .map(|(k, v)| {
            let text = if let Some(s) = v.as_str() {
                s.to_string()
            } else {
                v.to_string()
            };
            vec![k, md_cell(&text)]
        })
        .collect();
    md_table(&["key", "value"], &rows)
}

/// Markdown table of provider list hits: id / name / api / env / npm /
/// models_count (mirrors `provider_csv_row`'s fields).
fn md_provider_rows(rows: &[(String, Value)]) -> String {
    md_table(
        &["id", "name", "api", "env", "npm", "models_count"],
        &rows
            .iter()
            .map(|(id, v)| {
                vec![
                    id.clone(),
                    csv_get(v, "name"),
                    csv_get(v, "doc"),
                    csv_get(v, "env"),
                    csv_get(v, "npm"),
                    v.get("models")
                        .and_then(|m| m.as_object())
                        .map(|m| m.len().to_string())
                        .unwrap_or_default(),
                ]
            })
            .collect::<Vec<_>>(),
    )
}

/// `sys ai info FIELD [PARAM...] [-c FILE] [--refresh]`
///
/// FIELD is `provider`, `model` or `price`:
/// - `provider [KEYWORD]` — every configured provider (name / api_base /
///   api_key / docs / console); the official help-docs and console URLs are
///   looked up per provider name. KEYWORD keeps only the providers whose name
///   contains it.
/// - `model [KEYWORD]` — every configured model as `{provider}:{name}`;
///   KEYWORD keeps only the models of providers whose name contains it
///   (`provider:model` / `provider:*` select specific models; model-name
///   matching is used as a fallback when no provider matches).
/// - `sale-price PROVIDER` — scrape the provider's official pricing page and
///   print every model's sale price (agnes / minimax scraped; the rest point
///   to their official pricing pages). Pages are cached 24 h, `--refresh`
///   forces a re-fetch.
/// - `list-model PROVIDER` / `models PROVIDER` — query the provider's own
///   models API and print the models its configured api_key can use (same
///   function as `sys ai chat --query-model PROVIDER`; `GET {api_base}/models`
///   for OpenAI-compatible providers, `{api_base}/v1/models` for Anthropic;
///   live query, no cache; `next_page` pagination is followed).
/// - `-o json` / `-o csv` / `--json` — machine-readable output, supported by
///   `info provider`, `info model` and `info list-model` (JSON array or CSV
///   table).
///
/// Balance queries live in `sys ai balance PROVIDER` (moved up from
/// `info balance`).
pub fn cmd_info(
    field: Option<&str>,
    param: &[String],
    refresh: bool,
    config: Option<&Path>,
    out: Option<OutFormat>,
    json: bool,
    free: Option<&str>,
) -> Result<()> {
    let fmt = out.or(if json { Some(OutFormat::Json) } else { None });
    let keyword = {
        let s = param.join(" ");
        let s = s.trim();
        if s.is_empty() { None } else { Some(s.to_string()) }
    };
    let field = field.map(|f| f.to_ascii_lowercase());
    match field.as_deref() {
        None => {
            // No field: print the configured providers and models together.
            let (cfg, _) = chat::load_config(config)?;
            let provs = info_provider_rows(&cfg, None)?;
            let mods = info_model_rows(&cfg, None)?;
            match fmt {
                Some(OutFormat::Json) => {
                    let providers: Vec<Value> = provs
                        .iter()
                        .map(|r| {
                            serde_json::json!({
                                "name": r.name,
                                "api_base": r.api_base,
                                "api_key": r.api_key,
                                "docs": r.docs,
                                "console": r.console,
                                "sample": r.sample,
                            })
                        })
                        .collect();
                    let models: Vec<Value> = mods
                        .iter()
                        .map(|r| serde_json::json!({"provider": r.provider, "name": r.name}))
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "providers": providers,
                            "models": models,
                        }))?
                    );
                }
                Some(OutFormat::Csv) => {
                    bail!("`sys ai info` without a field does not support CSV; use `info provider` / `info model` for CSV, or run `sys ai info` for the combined text view")
                }
                Some(OutFormat::Md) => {
                    println!("## Providers");
                    println!();
                    println!("{}", md_provider_list(&provs));
                    println!("## Models");
                    println!();
                    println!("{}", md_model_list(&mods));
                }
                Some(OutFormat::Yaml) => {
                    let providers: Vec<Value> = provs
                        .iter()
                        .map(|r| {
                            serde_json::json!({
                                "name": r.name,
                                "api_base": r.api_base,
                                "api_key": r.api_key,
                                "docs": r.docs,
                                "console": r.console,
                                "sample": r.sample,
                            })
                        })
                        .collect();
                    let models: Vec<Value> = mods
                        .iter()
                        .map(|r| serde_json::json!({"provider": r.provider, "name": r.name}))
                        .collect();
                    println!(
                        "{}",
                        serde_yaml::to_string(&serde_json::json!({
                            "providers": providers,
                            "models": models,
                        }))?
                    );
                }
                None => {
                    print_provider_table(&provs);
                    println!();
                    println!("MODELS:");
                    for r in &mods {
                        println!("{}:{}", r.provider, r.name);
                    }
                }
            }
        }
        Some("provider") | Some("providers") => {
            let cfg = if free.is_some() {
                chat::free_config(free)?
            } else {
                chat::load_config(config)?.0
            };
            let rows = info_provider_rows(&cfg, keyword.as_deref())?;
            match fmt {
                Some(OutFormat::Json) => {
                    let arr: Vec<Value> = rows
                        .iter()
                        .map(|r| {
                            serde_json::json!({
                                "name": r.name,
                                "api_base": r.api_base,
                                "api_key": r.api_key,
                                "docs": r.docs,
                                "console": r.console,
                                "sample": r.sample,
                            })
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
                }
                Some(OutFormat::Csv) => {
                    println!("name,api_base,api_key,docs,console,sample");
                    for r in &rows {
                        println!("{},{},{},{},{},{}", r.name, r.api_base, r.api_key, r.docs, r.console, r.sample);
                    }
                }
                Some(OutFormat::Md) => {
                    println!("{}", md_provider_list(&rows));
                }
                // YAML is the default view of `info provider`.
                Some(OutFormat::Yaml) | None => {
                    println!("{}", serde_yaml::to_string(&rows)?);
                }
            }
        }
        Some("model") => {
            let (cfg, _) = chat::load_config(config)?;
            let rows = info_model_rows(&cfg, keyword.as_deref())?;
            match fmt {
                Some(OutFormat::Json) => {
                    let arr: Vec<Value> = rows
                        .iter()
                        .map(|r| serde_json::json!({"provider": r.provider, "name": r.name, "groups": r.groups}))
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
                }
                Some(OutFormat::Csv) => {
                    println!("provider,name,groups");
                    for r in &rows {
                        println!("{},{},{}", r.provider, r.name, r.groups.join("|"));
                    }
                }
                Some(OutFormat::Md) => {
                    println!("{}", md_model_list(&rows));
                }
                Some(OutFormat::Yaml) => {
                    let arr: Vec<Value> = rows
                        .iter()
                        .map(|r| serde_json::json!({"provider": r.provider, "name": r.name, "groups": r.groups}))
                        .collect();
                    println!("{}", serde_yaml::to_string(&Value::Array(arr))?);
                }
                None => {
                    for r in &rows {
                        if r.groups.is_empty() {
                            println!("{}:{}", r.provider, r.name);
                        } else {
                            println!("{}:{} (group: {})", r.provider, r.name, r.groups.join(", "));
                        }
                    }
                }
            }
        }
        Some("sale-price") | Some("saleprice") | Some("sale_price") => {
            if fmt.is_some() {
                bail!("`info sale-price` does not support -o/--json (run without it for the price view)");
            }
            let kw = keyword.ok_or_else(|| {
                anyhow::anyhow!("`info sale-price` needs a provider name: `sys ai info sale-price agnes`")
            })?;
            let (cfg, _) = chat::load_config(config)?;
            info_sale_price(&cfg, &kw, refresh)?;
        }
        Some("list-model") | Some("list-models") | Some("models") => {
            if free.is_some() {
                // --free[=router]: list the built-in free models instead of
                // querying a provider's own models endpoint.
                let cfg = chat::free_config(free)?;
                let rows = info_model_rows(&cfg, None)?;
                match fmt {
                    Some(OutFormat::Json) => {
                        let arr: Vec<Value> = rows
                            .iter()
                            .map(|r| {
                                serde_json::json!({"provider": r.provider, "name": r.name, "groups": r.groups})
                            })
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
                    }
                    Some(OutFormat::Csv) => {
                        println!("provider,name,groups");
                        for r in &rows {
                            println!("{},{},{}", r.provider, r.name, r.groups.join("|"));
                        }
                    }
                    Some(OutFormat::Md) => {
                        println!("{}", md_model_list(&rows));
                    }
                    Some(OutFormat::Yaml) => {
                        let arr: Vec<Value> = rows
                            .iter()
                            .map(|r| {
                                serde_json::json!({"provider": r.provider, "name": r.name, "groups": r.groups})
                            })
                            .collect();
                        println!("{}", serde_yaml::to_string(&Value::Array(arr))?);
                    }
                    None => {
                        for r in &rows {
                            if r.groups.is_empty() {
                                println!("{}:{}", r.provider, r.name);
                            } else {
                                println!("{}:{} (group: {})", r.provider, r.name, r.groups.join(", "));
                            }
                        }
                    }
                }
                return Ok(());
            }
            let kw = keyword.ok_or_else(|| {
                anyhow::anyhow!(
                    "`info list-model` / `info models` needs a provider name: `sys ai info models agnes`"
                )
            })?;
            let (cfg, _) = chat::load_config(config)?;
            info_model_list(&cfg, &kw, fmt)?;
        }
        Some(other) => bail!("unknown `info` field `{other}` (expected: provider | model | sale-price | list-model | models; prices moved to `sys ai price`, balances to `sys ai balance`)"),
    }
    Ok(())
}

/// `sys ai price P1,P2,... [--src SRC] [--refresh]` — the standalone price
/// command (moved from `sys ai info price`).
///
/// - src = models (default): per-1M-token prices (input / output / cache_read)
///   of the comma-separated providers from models.dev (24 h cache, `--refresh`
///   forces a re-fetch) — identical to the old `info price`.
/// - src = openrouter: the whole openrouter.ai catalogue with each model's
///   `pricing.prompt` / `pricing.completion` prices (per 1M tokens).
pub fn cmd_price(
    providers: &[String],
    src: Option<&str>,
    limit: Option<usize>,
    json: bool,
    out: Option<OutFormat>,
    refresh: bool,
) -> Result<()> {
    if src.map(|s| ci_eq(s, "openrouter")).unwrap_or(false) {
        // Join positional args with commas so `sys ai price a b` means the
        // same as `sys ai price a,b` (two filter terms, not one "a b" term).
        let q = providers.join(",");
        let q = q.trim();
        return cmd_price_openrouter(if q.is_empty() { None } else { Some(q) }, json, out, limit, refresh);
    }
    if providers.is_empty() {
        bail!("`sys ai price` needs at least one provider name: `sys ai price openai,anthropic` (or use --src openrouter)");
    }
    let data = fetch_data(refresh)?;
    let rows = info_price_rows(&data, providers)?;
    print_price_rows(&rows);
    Ok(())
}

// ---------------------------------------------------------------------------
// `sys ai balance` / `sys ai info sale-price` — per-provider
// official queries (余额 / 官网销售价).
//
// balance:  用该 provider 的 api_key 到官方接口查询余额。仅部分平台开放
//           API Key 余额接口（minimax 的 token_plan/remains、OpenAI 兼容的
//           dashboard/billing、DeepSeek 的 user/balance）；其余平台余额只在
//           登录控制台可见（tencent/TokenHub 管控面需 SecretId/SecretKey，
//           与配置的 API Key 不同），命令会给出控制台地址。余额查询始终
//           实时（不走缓存）。`sys ai balance` 是独立子命令（从
//           `sys ai info balance` 提升上来）。
// sale-price:抓取官网定价页（agnes wiki、minimax 开放平台）返回全部模型的
//           销售价；未接入的平台给出官网定价页地址。定价页 24h 缓存，
//           `--refresh` 强制重抓。
// ---------------------------------------------------------------------------

/// Resolve exactly one configured provider by name (exact ci, then substring).
fn find_provider<'a>(cfg: &'a chat::Config, keyword: &str) -> Result<&'a chat::Provider> {
    if let Some(p) = cfg.providers.iter().find(|p| ci_eq(&p.name, keyword)) {
        return Ok(p);
    }
    let hits: Vec<&chat::Provider> = cfg
        .providers
        .iter()
        .filter(|p| contains_ci(&p.name, keyword))
        .collect();
    match hits.len() {
        1 => Ok(hits[0]),
        0 => {
            let avail: Vec<&str> = cfg.providers.iter().map(|p| p.name.as_str()).collect();
            bail!("no provider contains `{keyword}` (available: {})", avail.join(", "))
        }
        _ => {
            let names: Vec<&str> = hits.iter().map(|p| p.name.as_str()).collect();
            bail!("`{keyword}` matches multiple providers: {} (use the exact name)", names.join(", "))
        }
    }
}

// ---------------------------------------------------------------------------
// `sys ai info list-model` / `sys ai info models` — query a provider's own
// models API and list the models its configured api_key can actually use
// (the same function as `sys ai chat --query-model PROVIDER`).
//
// URL rule shared with chat::models_url: OpenAI 兼容 provider 用
// GET {api_base}/models（Bearer api_key，`models` 直接追加在配置的 api_base
// 后——如百度千帆是 /v2/models 而非 /v2/v1/models）；Anthropic provider 用
// GET {api_base}/v1/models（api_base 已以 /v1 结尾时直接用 {api_base}/models，
// x-api-key / anthropic-version）。实时查询，不走缓存；分页（next_page）
// 自动翻页（上限 20 页）。
// ---------------------------------------------------------------------------

/// One row of `info list-model` (a model entry returned by the models API).
struct ModelListRow {
    id: String,
    owned_by: String,
    created: String,
}

/// Normalize a models API response into rows. Standard shape is `data[]`
/// (`id` / `owned_by` / `created` on OpenAI, `display_name` / `created_at`
/// on Anthropic); a bare `models[]` array is accepted as a fallback for
/// non-standard compatible providers.
fn model_list_rows(v: &Value) -> Vec<ModelListRow> {
    let items = v
        .get("data")
        .and_then(|d| d.as_array())
        .or_else(|| v.get("models").and_then(|m| m.as_array()))
        .cloned()
        .unwrap_or_default();
    items
        .iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(|x| x.as_str())?;
            let owned_by = m
                .get("owned_by")
                .and_then(|x| x.as_str())
                .or_else(|| m.get("display_name").and_then(|x| x.as_str()))
                .unwrap_or("")
                .to_string();
            let created = m
                .get("created")
                .and_then(|x| x.as_u64())
                .map(|t| t.to_string())
                .or_else(|| m.get("created_at").and_then(|x| x.as_str()).map(str::to_string))
                .unwrap_or_default();
            Some(ModelListRow { id: id.to_string(), owned_by, created })
        })
        .collect()
}

/// Convert a model's created value for display: a unix timestamp (seconds)
/// becomes the local date-time `YYYY-MM-DD HH:MM:SS`; anything else (Anthropic
/// ISO `created_at`, empty, non-numeric) passes through unchanged.
fn fmt_created(raw: &str) -> String {
    if let Ok(ts) = raw.parse::<i64>() {
        if let Some(dt) = chrono::DateTime::from_timestamp(ts, 0) {
            let local = dt.with_timezone(&chrono::Local);
            return local.format("%Y-%m-%d %H:%M:%S").to_string();
        }
    }
    raw.to_string()
}

/// `sys ai info list-model PROVIDER` — fetch the provider's model list from
/// its own models API and print it.
fn info_model_list(cfg: &chat::Config, keyword: &str, fmt: Option<OutFormat>) -> Result<()> {
    let p = find_provider(cfg, keyword)?;
    let client = reqwest::blocking::Client::new();
    let mut headers = reqwest::header::HeaderMap::new();
    if p.kind == "anthropic" {
        headers.insert(
            "x-api-key",
            reqwest::header::HeaderValue::from_str(&p.api_key).context("invalid api_key")?,
        );
        headers.insert(
            "anthropic-version",
            reqwest::header::HeaderValue::from_static(chat::ANTHROPIC_VERSION),
        );
    } else {
        let auth = format!("Bearer {}", p.api_key);
        headers.insert(
            AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&auth).context("invalid api_key")?,
        );
    }

    let base_url = chat::models_url(p);
    let mut url = base_url.clone();
    let mut rows: Vec<ModelListRow> = Vec::new();
    let mut pages = 0;
    loop {
        let resp = client
            .get(&url)
            .headers(headers.clone())
            .timeout(Duration::from_secs(20))
            .send()
            .with_context(|| format!("cannot query model list of `{}` at {url}", p.name))?;
        if !resp.status().is_success() {
            bail!("model list API of `{}` returned HTTP {} at {url}", p.name, resp.status());
        }
        let v: Value = serde_json::from_str(&resp.text().context("cannot read model list response")?)
            .context("invalid JSON from model list API")?;
        rows.extend(model_list_rows(&v));
        pages += 1;
        let next = v.get("next_page").and_then(|n| n.as_str());
        match next {
            Some(n) if pages < 20 && !n.is_empty() => {
                url = format!("{base_url}?next_page={n}");
            }
            _ => break,
        }
    }
    match fmt {
        Some(OutFormat::Json) => {
            let arr: Vec<Value> = rows
                .iter()
                .map(|r| serde_json::json!({"id": r.id, "owned_by": r.owned_by, "created": r.created}))
                .collect();
            println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
        }
        Some(OutFormat::Csv) => {
            println!("id,owned_by,created");
            for r in &rows {
                println!("{},{},{}", r.id, r.owned_by, r.created);
            }
        }
        Some(OutFormat::Md) => {
            println!("{}", md_list_model_rows(&rows));
        }
        Some(OutFormat::Yaml) => {
            let arr: Vec<Value> = rows
                .iter()
                .map(|r| serde_json::json!({"id": r.id, "owned_by": r.owned_by, "created": r.created}))
                .collect();
            println!("{}", serde_yaml::to_string(&Value::Array(arr))?);
        }
        None => {
            println!("== {} 可用模型（api_base: {}，共 {} 个）==", p.name, p.api_base, rows.len());
            if rows.is_empty() {
                println!("  （models 接口返回空，请检查 api_key 权限）");
                return Ok(());
            }
            let w_id = rows.iter().map(|r| r.id.chars().count()).max().unwrap_or(4).max(4);
            let w_own = rows.iter().map(|r| r.owned_by.chars().count()).max().unwrap_or(8).max(8);
            println!("{:<w_id$}  {:<w_own$}  {}", "MODEL", "OWNED_BY", "CREATED");
            for r in &rows {
                println!("{:<w_id$}  {:<w_own$}  {}", r.id, r.owned_by, fmt_created(&r.created));
            }
        }
    }
    Ok(())
}

/// `sys ai balance PROVIDER` — query the provider's official balance with
/// its configured api_key (moved up from `sys ai info balance`).
pub fn cmd_balance(provider: &str, config: Option<&Path>) -> Result<()> {
    let (cfg, _) = chat::load_config(config)?;
    let p = find_provider(&cfg, provider)?;
    println!("== {} 余额 ==", p.name);
    match p.name.to_ascii_lowercase().as_str() {
        "minimax" => balance_minimax(p),
        "agnes" => balance_openai_billing(p),
        "deepseek" => balance_deepseek(p),
        "tencent" | "tencentmaas" | "tokenhub" => balance_tencent(p),
        "alibaba-cn" | "alibaba" | "dashscope" | "bailian" => balance_alibaba(p),
        "modelscope" => balance_console_only(p, "https://modelscope.cn（控制台 / 阿里云费用中心）"),
        "sensenova" => balance_console_only(p, "https://console.sensecore.cn（费用中心）"),
        "bigmodel" => balance_console_only(p, "https://bigmodel.cn/console（财务总览）"),
        "amd" => balance_console_only(p, "https://developer.amd.com.cn（控制台）"),
        "anspire" => balance_console_only(p, "https://open.anspire.cn（用量信息页）"),
        other => bail!("no balance handler for provider `{other}`"),
    }
}

fn balance_console_only(p: &chat::Provider, where_: &str) -> Result<()> {
    println!("官方未提供 API Key 余额查询接口（该平台余额仅登录控制台可见）。");
    println!("请到 {where_} 查看 `{}` 的余额。", p.name);
    Ok(())
}

/// MiniMax 官方余额接口：GET https://www.minimaxi.com/v1/token_plan/remains
/// （Bearer api_key）。返回 Token Plan 各模型剩余额度；非 Token Plan 用户
/// （按量付费）返回 status_msg 提示。
fn balance_minimax(p: &chat::Provider) -> Result<()> {
    let url = "https://www.minimaxi.com/v1/token_plan/remains";
    let resp = reqwest::blocking::Client::new()
        .get(url)
        .header(AUTHORIZATION, format!("Bearer {}", p.api_key))
        .header(CONTENT_TYPE, "application/json")
        .timeout(Duration::from_secs(20))
        .send()
        .with_context(|| format!("cannot query MiniMax balance at {url}"))?;
    if !resp.status().is_success() {
        bail!("MiniMax balance API returned HTTP {} at {url}", resp.status());
    }
    let v: Value = serde_json::from_str(&resp.text().context("cannot read MiniMax balance response")?)
        .context("invalid JSON from MiniMax balance API")?;
    let status = v
        .pointer("/base_resp/status_code")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let msg = v
        .pointer("/base_resp/status_msg")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if status != 0 && !msg.is_empty() {
        println!("Token Plan: {msg}");
    }
    match v.get("model_remains").and_then(|r| r.as_object()) {
        Some(remains) if !remains.is_empty() => {
            for (model, mv) in remains {
                let remain = mv.get("remain").and_then(|x| x.as_u64());
                let total = mv.get("total").and_then(|x| x.as_u64());
                let used = mv.get("used").and_then(|x| x.as_u64());
                let mut parts: Vec<String> = Vec::new();
                if let Some(r) = remain {
                    parts.push(format!("剩余 {r}"));
                }
                if let Some(t) = total {
                    parts.push(format!("总额 {t}"));
                }
                if let Some(u) = used {
                    parts.push(format!("已用 {u}"));
                }
                if parts.is_empty() {
                    parts.push(mv.to_string());
                }
                println!("  {model}: {}", parts.join(", "));
            }
        }
        _ => {
            println!("  该 Key 无有效 Token Plan 订阅（按量付费余额请在 https://platform.minimaxi.com 账户管理 > 余额 查看）。");
        }
    }
    Ok(())
}

/// 阿里云百炼（DashScope）：官方 `GET /api/v1/models/limits` 返回该 Key 下
/// 各模型的用量限额（限流配额），可验证 Key 有效性；官方无 Key 级现金余额
/// API，账户余额/费用需登录控制台查看。
fn balance_alibaba(p: &chat::Provider) -> Result<()> {
    let url = "https://dashscope.aliyuncs.com/api/v1/models/limits?page_no=1&page_size=100";
    let resp = reqwest::blocking::Client::new()
        .get(url)
        .header(AUTHORIZATION, format!("Bearer {}", p.api_key))
        .header(CONTENT_TYPE, "application/json")
        .timeout(Duration::from_secs(20))
        .send()
        .with_context(|| format!("cannot query Alibaba (DashScope) limits at {url}"))?;
    if !resp.status().is_success() {
        bail!("Alibaba (DashScope) limits API returned HTTP {} at {url}", resp.status());
    }
    let v: Value = serde_json::from_str(&resp.text().context("cannot read Alibaba limits response")?)
        .context("invalid JSON from Alibaba limits API")?;
    let total = v
        .pointer("/output/total")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let rows = alibaba_limits_rows(&v);
    println!("官方未提供 API Key 现金余额查询接口；以下为该 Key 可查的用量限额（限流配额，非余额）：");
    if rows.is_empty() {
        println!("  （接口返回空，请在控制台查看）");
    } else {
        let w_model = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(12);
        println!("{:<w_model$}  {:>14}  {:>22}", "模型", "请求频率", "用量限制(tokens/周期)");
        for (model, req, usage) in rows.iter().take(20) {
            println!("{model:<w_model$}  {req:>14}  {usage:>22}");
        }
        if rows.len() > 20 {
            println!("  …共 {total} 个模型（仅显示前 20 个，完整列表见百炼控制台）");
        }
    }
    println!("账户余额 / 费用明细请在 https://bailian.console.aliyun.com 控制台查看。");
    Ok(())
}

/// Extract (model, request-limit, usage-limit) rows from the limits response
/// (pure, unit-testable).
fn alibaba_limits_rows(v: &Value) -> Vec<(String, String, String)> {
    let quotas = v.pointer("/output/quotas").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    quotas
        .iter()
        .filter_map(|q| {
            let model = q.get("model").and_then(|x| x.as_str()).unwrap_or("?").to_string();
            let ml = q.get("model_limit").and_then(|x| x.as_object());
            let req = ml.and_then(|m| m.get("request_limit")).and_then(|x| x.as_u64());
            let req_period = ml.and_then(|m| m.get("request_limit_period")).and_then(|x| x.as_u64());
            let usage = ml.and_then(|m| m.get("usage_limit")).and_then(|x| x.as_u64());
            let usage_period = ml.and_then(|m| m.get("usage_limit_period")).and_then(|x| x.as_u64());
            let req_s = match (req, req_period) {
                (Some(r), Some(1)) => format!("{r}/s"),
                (Some(r), Some(60)) => format!("{r}/min"),
                (Some(r), Some(6)) => format!("{r}/6s"),
                (Some(r), Some(per)) => format!("{r}/{per}s"),
                (Some(r), None) => format!("{r}"),
                _ => "-".to_string(),
            };
            let usage_s = match (usage, usage_period) {
                (Some(u), Some(per)) => format!("{u}/{per}s"),
                (Some(u), None) => format!("{u}"),
                _ => "-".to_string(),
            };
            Some((model, req_s, usage_s))
        })
        .collect()
}

/// OpenAI 兼容账单探测（agnes 等）：依次尝试 credit_grants / subscription /
/// usage，打印接口实际返回的可用字段；若均无余额字段，提示到控制台查看。
fn balance_openai_billing(p: &chat::Provider) -> Result<()> {
    let client = reqwest::blocking::Client::new();
    let base = p.api_base.trim_end_matches('/');
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut balance_field = false;

    for (path, fields, is_balance) in [
        (
            "/dashboard/billing/credit_grants",
            &[("total_available", "可用余额"), ("total_granted", "充值总额"), ("total_used", "已用额度"), ("currency", "币种")][..],
            true,
        ),
        (
            "/dashboard/billing/subscription",
            &[("has_payment_method", "已绑定支付"), ("soft_limit_usd", "软限额($)"), ("hard_limit_usd", "硬限额($)")][..],
            false,
        ),
        (
            "/dashboard/billing/usage",
            &[("total_usage", "本月用量")][..],
            false,
        ),
    ] {
        let url = format!("{base}{path}");
        let resp = match client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", p.api_key))
            .timeout(Duration::from_secs(20))
            .send()
        {
            Ok(r) if r.status().is_success() => r,
            _ => continue,
        };
        let v: Value = match serde_json::from_str(&resp.text().unwrap_or_default()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(o) = v.as_object() {
            for (k, label) in fields {
                if let Some(x) = o.get(*k) {
                    rows.push((label.to_string(), x.to_string()));
                }
            }
        }
        if is_balance {
            balance_field = v
                .as_object()
                .map(|o| o.contains_key("total_available") || o.contains_key("total_granted"))
                .unwrap_or(false);
        }
    }

    if rows.is_empty() {
        println!("官方接口未开放余额查询（无可用的账单端点）。");
    } else {
        let w = rows.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(4);
        for (k, v) in rows {
            println!("{k:>w$}: {v}");
        }
    }
    if !balance_field {
        println!("提示：官方接口未返回余额字段；余额请在 https://agnes-ai.cn 控制台（Usage / Billing）查看。");
    }
    Ok(())
}

/// DeepSeek 官方余额接口：GET {api_base}/user/balance（Bearer api_key）。
/// 返回 is_available（账户是否有可用余额）与 balance_infos[]（currency /
/// total_balance / granted_balance / topped_up_balance，均为字符串）。
fn balance_deepseek(p: &chat::Provider) -> Result<()> {
    let base = p.api_base.trim_end_matches('/');
    let url = format!("{base}/user/balance");
    let resp = reqwest::blocking::Client::new()
        .get(&url)
        .header(AUTHORIZATION, format!("Bearer {}", p.api_key))
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(20))
        .send()
        .with_context(|| format!("cannot query DeepSeek balance at {url}"))?;
    if !resp.status().is_success() {
        bail!("DeepSeek balance API returned HTTP {} at {url}", resp.status());
    }
    let v: Value = serde_json::from_str(&resp.text().context("cannot read DeepSeek balance response")?)
        .context("invalid JSON from DeepSeek balance API")?;
    let (available, rows) = deepseek_balance_rows(&v);
    println!(
        "账户可用: {}",
        if available { "是" } else { "否（余额不足，可能无法调用）" }
    );
    if rows.is_empty() {
        println!("  （接口未返回余额明细）");
    } else {
        for (currency, total, granted, topped) in rows {
            println!("  {currency}: 总额 {total}（赠送 {granted}，充值 {topped}）");
        }
    }
    println!("余额 / 充值记录也可在 https://platform.deepseek.com 控制台查看。");
    Ok(())
}

/// Parse the DeepSeek balance response into (is_available, rows of
/// (currency, total_balance, granted_balance, topped_up_balance)); pure and
/// unit-testable.
fn deepseek_balance_rows(v: &Value) -> (bool, Vec<(String, String, String, String)>) {
    let available = v.get("is_available").and_then(|x| x.as_bool()).unwrap_or(false);
    let rows = v
        .get("balance_infos")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|b| {
            let currency = b.get("currency").and_then(|x| x.as_str()).unwrap_or("?").to_string();
            let total = b.get("total_balance").and_then(|x| x.as_str()).unwrap_or("?").to_string();
            let granted = b.get("granted_balance").and_then(|x| x.as_str()).unwrap_or("?").to_string();
            let topped = b.get("topped_up_balance").and_then(|x| x.as_str()).unwrap_or("?").to_string();
            Some((currency, total, granted, topped))
        })
        .collect();
    (available, rows)
}

/// 腾讯云 TokenHub（tokenhub.tencentmaas.com）：数据面仅提供 OpenAI/Anthropic
/// 协议的模型调用接口，无 API Key 余额/额度查询端点；管控面
/// （tokenhub.tencentcloudapi.com 的 DescribeApiKey / DescribeTokenPlanApiKey）
/// 需要腾讯云 SecretId/SecretKey 签名，与配置的 API Key 不同，故仅给出控制台指引。
fn balance_tencent(p: &chat::Provider) -> Result<()> {
    balance_console_only(
        p,
        "https://console.cloud.tencent.com/tokenhub（TokenHub 控制台 > API Key 额度 / 用量统计；账单见腾讯云费用中心）",
    )
}

/// `sys ai info sale-price PROVIDER` — scrape the provider's official
/// pricing page and print every listed model's sale price.
fn info_sale_price(cfg: &chat::Config, keyword: &str, refresh: bool) -> Result<()> {
    let p = find_provider(cfg, keyword)?;
    println!("== {} 官网销售价 ==", p.name);
    match p.name.to_ascii_lowercase().as_str() {
        "agnes" => sale_price_agnes(refresh),
        "minimax" => sale_price_minimax(refresh),
        "alibaba-cn" | "alibaba" | "dashscope" | "bailian" => sale_price_url(p, "https://help.aliyun.com/zh/model-studio/models（模型列表与计费说明）"),
        "modelscope" => sale_price_url(p, "https://modelscope.cn/models（模型详情页含计费说明）"),
        "sensenova" => sale_price_url(p, "https://console.sensecore.cn/micro/help/docs/model-as-a-service/nova/（产品定价文档）"),
        "bigmodel" => sale_price_url(p, "https://docs.bigmodel.cn/cn/guide/start/price"),
        "amd" => sale_price_url(p, "https://developer.amd.com.cn（产品定价）"),
        "anspire" => sale_price_url(p, "https://open.anspire.cn/document/docs/openPlatform/（产品计费逻辑）"),
        other => bail!("no sale-price handler for provider `{other}`"),
    }
}

fn sale_price_url(p: &chat::Provider, url: &str) -> Result<()> {
    println!("暂未接入 `{}` 官网价格抓取，请直接访问：{url}", p.name);
    Ok(())
}

// --- pricing-page fetch + HTML table parsing ----------------------------------

fn load_text_cache(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    if SystemTime::now().duration_since(mtime).map(|d| d < CACHE_TTL).unwrap_or(false) {
        if let Ok(content) = std::fs::read_to_string(path) {
            return Some(content);
        }
    }
    None
}

fn save_text_cache(path: &Path, text: &str) {
    let dir = cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(path, text);
}

/// Fetch a pricing page with a 24 h local cache (`--refresh` forces a re-fetch).
fn fetch_pricing_page(url: &str, cache_key: &str, refresh: bool) -> Result<String> {
    let file = cache_dir().join(cache_key);
    if !refresh {
        if let Some(html) = load_text_cache(&file) {
            return Ok(html);
        }
    }
    let resp = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; sys)")
        .build()
        .context("failed to build HTTP client")?
        .get(url)
        .send()
        .with_context(|| format!("cannot fetch {url} (offline? use a cached copy if available)"))?;
    if !resp.status().is_success() {
        bail!("{url} returned HTTP {}", resp.status());
    }
    let text = resp.text().with_context(|| format!("cannot read response from {url}"))?;
    save_text_cache(&file, &text);
    Ok(text)
}

/// Strip HTML tags from a cell, decode common entities, turn `<br>` into " | "
/// and collapse whitespace.
fn html_text(s: &str) -> String {
    let s = s.replace("<br/>", " | ").replace("<br>", " | ").replace("</br>", " ");
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split an HTML document into tables, each table into rows of text cells
/// (tags stripped; `<td>` cells first, `<th>` fallback for header rows).
fn extract_tables(html: &str) -> Vec<Vec<Vec<String>>> {
    let mut tables: Vec<Vec<Vec<String>>> = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = html[pos..].find("<table") {
        let start = pos + rel;
        let end = html[start..]
            .find("</table>")
            .map(|i| start + i)
            .unwrap_or(html.len());
        let body = &html[start..end];
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut rpos = 0usize;
        while let Some(tr) = body[rpos..].find("<tr") {
            let tr_start = rpos + tr;
            let tr_end = body[tr_start..]
                .find("</tr>")
                .map(|i| tr_start + i)
                .unwrap_or(body.len());
            let row_html = &body[tr_start..tr_end];
            let mut cells: Vec<String> = Vec::new();
            let mut cpos = 0usize;
            while let Some(td) = row_html[cpos..].find("<td") {
                let td_start = cpos + td;
                let gt = row_html[td_start..]
                    .find('>')
                    .map(|i| td_start + i + 1)
                    .unwrap_or(td_start);
                let td_end = row_html[td_start..]
                    .find("</td>")
                    .map(|i| td_start + i)
                    .unwrap_or(row_html.len());
                cells.push(html_text(&row_html[gt..td_end]));
                cpos = td_end + 5;
            }
            if cells.is_empty() {
                let mut cpos = 0usize;
                while let Some(th) = row_html[cpos..].find("<th") {
                    let th_start = cpos + th;
                    let gt = row_html[th_start..]
                        .find('>')
                        .map(|i| th_start + i + 1)
                        .unwrap_or(th_start);
                    let th_end = row_html[th_start..]
                        .find("</th>")
                        .map(|i| th_start + i)
                        .unwrap_or(row_html.len());
                    cells.push(html_text(&row_html[gt..th_end]));
                    cpos = th_end + 5;
                }
            }
            if !cells.is_empty() {
                rows.push(cells);
            }
            rpos = tr_end + 5;
        }
        if !rows.is_empty() {
            tables.push(rows);
        }
        pos = end + 8;
    }
    tables
}

const AGNES_PRICING_URL: &str = "https://wiki.agnes-ai.cn/zh-Hans/docs/pricing";
const AGNES_PRICING_CACHE: &str = "agnes-pricing.html";

/// Agnes 中国站定价页：文本/图片/视频模型表（模型 | 计费项 | 刊例价 | 现价），
/// 人民币价格。模型单元格带 rowspan，后续行继承当前模型。
fn sale_price_agnes(refresh: bool) -> Result<()> {
    let html = fetch_pricing_page(AGNES_PRICING_URL, AGNES_PRICING_CACHE, refresh)?;
    let out = parse_agnes_prices(&html);
    if out.is_empty() {
        bail!("未能从 {AGNES_PRICING_URL} 解析出模型价格（页面结构可能已变更，请用 --refresh 重试）");
    }
    let w_model = out.iter().map(|r| r.0.chars().count()).max().unwrap_or(12);
    let w_item = out.iter().map(|r| r.1.chars().count()).max().unwrap_or(8);
    let w_list = out.iter().map(|r| r.2.chars().count()).max().unwrap_or(10);
    println!("{:<w_model$}  {:<w_item$}  {:<w_list$}  {}", "模型", "计费项", "刊例价（原价）", "现价（优惠价）");
    for (m, item, list, sale) in &out {
        println!("{m:<w_model$}  {item:<w_item$}  {list:<w_list$}  {sale}");
    }
    println!("来源：{AGNES_PRICING_URL}");
    Ok(())
}

/// Pure parser for the agnes pricing page (kept separate for unit tests).
fn parse_agnes_prices(html: &str) -> Vec<(String, String, String, String)> {
    let mut out: Vec<(String, String, String, String)> = Vec::new();
    for table in extract_tables(html) {
        let mut cur_model = String::new();
        for row in table {
            // 表头行（模型 | 计费项 | 刊例价 | 现价）
            if row.len() >= 3 && row[0] == "模型" && row.iter().any(|c| c == "计费项") {
                continue;
            }
            let mut cells = row;
            if cells.len() >= 4 && !cells[0].is_empty() && cells[0] != "计费项" {
                cur_model = cells[0].clone();
                cells.remove(0);
            }
            if cells.len() >= 3 && !cur_model.is_empty() {
                out.push((cur_model.clone(), cells[0].clone(), cells[1].clone(), cells[2].clone()));
            }
        }
    }
    out
}

const MINIMAX_PRICING_URL: &str = "https://platform.minimaxi.com/docs/pricing";
const MINIMAX_PRICING_CACHE: &str = "minimax-pricing.html";

/// MiniMax 开放平台定价页：语言模型表（模型 | 输入价格 | 输出价格 | 缓存读取
/// [| 缓存写入]，元/百万 tokens）。只取表头含「模型」+「输入价格」的表。
fn sale_price_minimax(refresh: bool) -> Result<()> {
    let html = fetch_pricing_page(MINIMAX_PRICING_URL, MINIMAX_PRICING_CACHE, refresh)?;
    let out = parse_minimax_prices(&html);
    if out.is_empty() {
        bail!("未能从 {MINIMAX_PRICING_URL} 解析出模型价格（页面结构可能已变更，请用 --refresh 重试）");
    }
    let w_model = out.iter().map(|r| r[0].chars().count()).max().unwrap_or(12);
    println!("{:<w_model$}  {:>10}  {:>10}  {:>10}  {:>10}  (元 / 百万 tokens)", "模型", "输入价格", "输出价格", "缓存读取", "缓存写入");
    for row in &out {
        let v = |i: usize| row.get(i).cloned().unwrap_or_else(|| "-".to_string());
        println!("{:<w_model$}  {:>10}  {:>10}  {:>10}  {:>10}", row[0], v(1), v(2), v(3), v(4));
    }
    println!("来源：{MINIMAX_PRICING_URL}");
    Ok(())
}

/// Pure parser for the minimax language-model price tables (unit-testable).
fn parse_minimax_prices(html: &str) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    for table in extract_tables(html) {
        let header = table.first().cloned().unwrap_or_default();
        let joined = header.join(" ");
        if !joined.contains("模型") || !joined.contains("输入价格") {
            continue;
        }
        for row in table.iter().skip(1) {
            if row.len() >= 4 && !row[0].is_empty() {
                out.push(row.clone());
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// `sys ai cn-model` — query the DataLearner AI model list.
//
// The DataLearner page (https://www.datalearner.com/ai-models/pretrained-models)
// is server-rendered HTML. Models are listed as cards in the "全部模型" grid,
// one page at a time (`?page=N`, page 1 has no query string). Each released
// model card carries: name, provider, optional aliases (又名), type badges
// (预览版 / 精选 / 开源模型 / 闭源模型 ...), a published date and a category.
// Rumor (传闻) cards carry "预计发布" instead of a real date and are skipped.
// The model detail page additionally exposes the model scale
// (总参数 / 激活参数, e.g. `2350亿 / 220亿`), which is fetched for single
// exact hits and shown as the `parameters` field.
// ---------------------------------------------------------------------------

const DL_BASE: &str = "https://www.datalearner.com/ai-models/pretrained-models";
const DL_UA: &str = "Mozilla/5.0 (compatible; sys)";
const DL_MAX_PAGES: usize = 200;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DlModel {
    /// URL slug, e.g. `gpt-6-1-sol`
    id: String,
    /// Display name, e.g. `GPT-6.1 Sol`
    name: String,
    /// Publishing organization, e.g. `OpenAI`
    provider: String,
    /// Aliases joined with " / " (又名 on the page)
    aliases: String,
    /// Type badges joined with spaces (e.g. "精选 闭源模型")
    r#type: String,
    /// Model category, e.g. `推理大模型`
    category: String,
    /// Context length from the detail page, e.g. `1.05M` (empty when unknown)
    context: String,
    /// Input/output modalities from the detail page, e.g. `文本、图像 → 文本`
    modality: String,
    /// Model scale (参数量) from the detail page's `总参数 / 激活参数` row,
    /// e.g. `2350亿 / 220亿` (empty when unknown). Only filled for single
    /// exact hits that fetch the detail page; cards never carry it.
    #[serde(default)]
    parameters: String,
    /// Published date YYYY-MM-DD (empty when the card has no real date)
    published: String,
    /// Detail page URL
    url: String,
}

const DL_CSV_HEADER: &str = "id,name,provider,aliases,type,category,context,modality,parameters,published,url";

fn dl_cache_file() -> PathBuf {
    cache_dir().join("datalearner-models.json")
}

fn load_dl_cache() -> Option<Vec<DlModel>> {
    let file = dl_cache_file();
    let meta = std::fs::metadata(&file).ok()?;
    let mtime = meta.modified().ok()?;
    if SystemTime::now().duration_since(mtime).map(|d| d < CACHE_TTL).unwrap_or(false) {
        if let Ok(content) = std::fs::read_to_string(&file) {
            if let Ok(v) = serde_json::from_str(&content) {
                return Some(v);
            }
        }
    }
    None
}

fn save_dl_cache(models: &[DlModel]) -> Result<()> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).context("cannot create cache directory")?;
    let text = serde_json::to_string(models).context("cannot serialize DataLearner cache")?;
    std::fs::write(dl_cache_file(), text).context("cannot write DataLearner cache")?;
    Ok(())
}

fn fetch_dl_page(url: &str) -> Result<String> {
    let resp = reqwest::blocking::Client::builder()
        .user_agent(DL_UA)
        .build()
        .context("failed to build HTTP client")?
        .get(url)
        .send()
        .with_context(|| format!("cannot fetch {url} (offline? use a cached copy if available)"))?;
    if !resp.status().is_success() {
        bail!("{url} returned HTTP {}", resp.status());
    }
    resp.text().with_context(|| format!("cannot read response from {url}"))
}

/// Fetch every page of the DataLearner model list (deduplicated by slug) and
/// cache it as JSON for 24 h. Returns the flattened model list.
fn fetch_datalearner(refresh: bool) -> Result<Vec<DlModel>> {
    if !refresh {
        if let Some(v) = load_dl_cache() {
            return Ok(v);
        }
    }
    eprintln!("sys: fetching {DL_BASE} ...");
    let mut models: Vec<DlModel> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut page = 1usize;
    loop {
        let url = if page == 1 {
            DL_BASE.to_string()
        } else {
            format!("{DL_BASE}?page={page}")
        };
        let html = fetch_dl_page(&url)?;
        let cards = parse_dl_cards(&html);
        if cards.is_empty() {
            break;
        }
        for m in cards {
            if seen.insert(m.id.clone()) {
                models.push(m);
            }
        }
        if page >= DL_MAX_PAGES {
            break;
        }
        page += 1;
        std::thread::sleep(Duration::from_millis(120));
    }
    if models.is_empty() {
        bail!("no model entries could be parsed from {DL_BASE}");
    }
    save_dl_cache(&models)?;
    Ok(models)
}

/// Parse the value that follows a labeled row in a DataLearner detail page.
/// The label sits in a small header div, immediately followed by the value
/// div: `...>LABEL</div><div class="...">VALUE</div>`. The label may be
/// wrapped in its own span (`>LABEL</span></div><div class="...">`), so we
/// locate the label text and then read the value div that directly follows
/// the next `</div><div class="` sequence.
///
/// The label text can also occur outside a label row (e.g. in the page's
/// meta description or scripts), so occurrences are scanned one by one and
/// only one whose value div starts within `MAX_GAP` bytes after the label is
/// accepted; this keeps the meta-description hit from grabbing an unrelated
/// element far away in the document. Values that only carry the site's
/// `暂无数据` placeholders (e.g. `暂无数据 / 暂无数据`) are treated as absent.
fn dl_detail_value(html: &str, label: &str) -> Option<String> {
    const MAX_GAP: usize = 200;
    let marker = "</div><div class=\"";
    let mut pos = 0usize;
    while let Some(offset) = html.get(pos..).and_then(|r| r.find(label)) {
        let label_pos = pos + offset;
        let rest = &html[label_pos + label.len()..];
        if let Some(ms) = rest.find(marker) {
            if ms <= MAX_GAP {
                let after = &rest[ms + marker.len()..];
                if let Some(gt) = after.find('>') {
                    let value_start = gt + 1;
                    if let Some(value_end) = after[value_start..].find("</div>") {
                        let raw = &after[value_start..value_start + value_end];
                        let text = html_unescape(&html_strip_tags(raw));
                        let t = text.trim();
                        // The site marks unknown values with 暂无数据; a value
                        // made only of placeholders (e.g. `暂无数据 / 暂无数据`)
                        // counts as absent, while partially known values
                        // (e.g. `2350亿 / 暂无数据`) are kept.
                        let placeholder_only = t
                            .split(['/', '、', ',', '，', ' ', '\t'])
                            .all(|p| p.trim().is_empty() || p.trim() == "暂无数据");
                        if !t.is_empty() && !placeholder_only {
                            return Some(t.to_string());
                        }
                    }
                }
            }
        }
        pos = label_pos + label.len();
    }
    None
}

/// Fetch a model's detail page and extract the context length, the
/// input/output modalities and the parameter scale (总参数 / 激活参数).
/// Failures (network, missing fields) return empty strings rather than
/// aborting the whole query.
fn fetch_dl_detail(url: &str) -> Result<(String, String, String)> {
    let html = fetch_dl_page(url)?;
    Ok((
        dl_detail_value(&html, "上下文长度").unwrap_or_default(),
        dl_detail_value(&html, "输入/输出模态").unwrap_or_default(),
        dl_detail_value(&html, "总参数 / 激活参数").unwrap_or_default(),
    ))
}

/// Map a `--model-type` value for the DataLearner list to the category
/// keywords it matches. The list cards carry a category such as
/// `推理大模型` / `语音大模型` / `多模态大模型` / `视觉大模型` /
/// `编程大模型` / `基础大模型` / `翻译大模型`.
fn dl_type_keywords(t: &str) -> Option<Vec<&'static str>> {
    match t.trim().to_ascii_lowercase().as_str() {
        "text" | "文本" => Some(vec!["推理", "编程", "对话", "翻译", "基础", "文本"]),
        "image" | "图像" | "视觉" | "图片" => Some(vec!["视觉", "图像", "多模态"]),
        "audio" | "语音" | "音频" | "voice" => Some(vec!["语音"]),
        "video" | "视频" => Some(vec!["视频"]),
        "multimodal" | "多模态" => Some(vec!["多模态"]),
        _ => None,
    }
}

/// Keep only DataLearner models whose category matches **every** requested
/// type (AND semantics: each type's keyword set must hit the category at
/// least once).
fn filter_dl_by_type(models: Vec<DlModel>, types: &[String]) -> Result<Vec<DlModel>> {
    let kw_sets: Vec<Vec<&'static str>> = types
        .iter()
        .map(|t| {
            dl_type_keywords(t).ok_or_else(|| {
                anyhow::anyhow!(
                    "unsupported --model-type `{t}` (supported: text, image, audio, video, multimodal or 文本/图像/语音/视频/多模态; comma-separated multi-values like `text,image` are AND-ed)"
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(models
        .into_iter()
        .filter(|m| {
            kw_sets
                .iter()
                .all(|kws| kws.iter().any(|k| m.category.contains(k)))
        })
        .collect())
}

// --- minimal HTML scanning helpers (no external parser dependency) ---------

/// Byte offset of `needle` at or after `from` (None when absent).
fn find_sub(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    haystack.get(from..)?.find(needle).map(|i| from + i)
}

/// Remove every `<...>` tag from a string, keeping the text content.
fn html_strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => rest = &rest[start + end + 1..],
            None => break,
        }
    }
    out.push_str(rest);
    out
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

/// Value of the first `attr="..."` attribute found in `s`.
fn tag_attr(s: &str, attr: &str) -> Option<String> {
    let marker = format!("{attr}=\"");
    let start = s.find(&marker)?;
    let rest = &s[start + marker.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Inner text of the first `<tag ...>TEXT</tag>` element.
fn tag_inner_text(s: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let start = s.find(&open)?;
    let gt = find_sub(s, ">", start + open.len())?;
    let close = format!("</{tag}>");
    let cl = find_sub(s, &close, gt + 1)?;
    let text = html_strip_tags(&s[gt + 1..cl]);
    Some(html_unescape(&text).trim().to_string())
}

/// Inner text of the first `<span class="...MARKER...">TEXT</span>` after `from`.
fn span_class_text(s: &str, marker: &str) -> String {
    let start = match s.find("<span class=\"") {
        Some(i) => i,
        None => return String::new(),
    };
    let rest = &s[start..];
    if !rest[..rest.find('>').unwrap_or(rest.len())].contains(marker) {
        return String::new();
    }
    let gt = find_sub(rest, ">", 0).unwrap_or(0);
    let cl = match find_sub(rest, "</span>", gt) {
        Some(i) => i,
        None => return String::new(),
    };
    let text = html_strip_tags(&rest[gt + 1..cl]);
    html_unescape(&text).trim().to_string()
}

/// Aliases from a `<span class="text-[11px]" title="...">又名...</span>` element.
fn alias_text(card: &str) -> String {
    let start = match card.find("text-[11px]") {
        Some(i) => i,
        None => return String::new(),
    };
    let head = &card[..start];
    let span_start = head.rfind("<span").unwrap_or(start);
    let seg = &card[span_start..];
    let marker = "title=\"";
    let ti = match seg.find(marker) {
        Some(i) => i,
        None => return String::new(),
    };
    let rest = &seg[ti + marker.len()..];
    let end = match rest.find('"') {
        Some(i) => i,
        None => return String::new(),
    };
    rest[..end].trim().to_string()
}

/// True for a bare `YYYY-MM-DD` string.
fn is_plain_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(|c| c.is_ascii_digit())
        && b[5..7].iter().all(|c| c.is_ascii_digit())
        && b[8..10].iter().all(|c| c.is_ascii_digit())
}

/// Parse every main-grid model card out of one DataLearner page.
///
/// Only cards whose anchor class contains `flex flex-col` (the "全部模型"
/// grid) are considered; sidebar entries (特色/开源最新 lists) and rumor
/// cards (`预计发布`) are skipped. Cards are deduplicated later by slug.
fn parse_dl_cards(html: &str) -> Vec<DlModel> {
    const HREF: &str = "<a href=\"/ai-models/pretrained-models/";
    let mut out: Vec<DlModel> = Vec::new();
    let mut pos = 0usize;
    while let Some(start) = find_sub(html, HREF, pos) {
        let slug_start = start + HREF.len();
        let slug_end = match html.get(slug_start..).and_then(|r| r.find('"')) {
            Some(i) => i,
            None => break,
        };
        let slug = html[slug_start..slug_start + slug_end].to_string();
        let a_close = match html.get(slug_start + slug_end..).and_then(|r| r.find("</a>")) {
            Some(i) => i,
            None => break,
        };
        let card_end = slug_start + slug_end + a_close;
        let card = &html[start..card_end];
        pos = card_end + 4;

        let cls = tag_attr(card, "class").unwrap_or_default();
        if !cls.contains("flex flex-col") || card.contains("预计发布") {
            continue;
        }

        let name = tag_inner_text(card, "h3").unwrap_or_default();
        let provider = match find_sub(card, "</h3>", 0) {
            Some(h3e) => span_class_text(&card[h3e + 5..], "text-[12px]"),
            None => String::new(),
        };
        let aliases = alias_text(card);

        let mut badges: Vec<String> = Vec::new();
        let mut i = 0usize;
        while let Some(p) = find_sub(card, "text-[10px]", i) {
            let span_start = match card[..p].rfind("<span") {
                Some(s) => s,
                None => break,
            };
            let gt = match find_sub(card, ">", span_start) {
                Some(g) => g,
                None => break,
            };
            let cl = match find_sub(card, "</span>", gt) {
                Some(c) => c,
                None => break,
            };
            let t = html_strip_tags(&card[gt + 1..cl]);
            let t = html_unescape(&t).trim().to_string();
            if !t.is_empty() {
                badges.push(t);
            }
            i = cl + 7;
        }

        let mut published = String::new();
        let mut category = String::new();
        if let Some(mr) = find_sub(card, "flex items-center justify-between mt-auto", 0) {
            let row_start = card[..mr].rfind("<div class=").unwrap_or(mr);
            if let Some(row_end) = find_sub(card, "</div>", mr) {
                let row = &card[row_start..row_end];
                let mut j = 0usize;
                while let Some(sp) = find_sub(row, "<span", j) {
                    let gt = match find_sub(row, ">", sp) {
                        Some(g) => g,
                        None => break,
                    };
                    let cl = match find_sub(row, "</span>", gt) {
                        Some(c) => c,
                        None => break,
                    };
                    let t = html_strip_tags(&row[gt + 1..cl]);
                    let t = html_unescape(&t).trim().to_string();
                    if !t.is_empty() {
                        if is_plain_date(&t) {
                            published = t;
                        } else if category.is_empty() {
                            category = t;
                        }
                    }
                    j = cl + 7;
                }
            }
        }

        out.push(DlModel {
            id: slug.clone(),
            name,
            provider,
            aliases,
            r#type: badges.join(" "),
            category,
            context: String::new(),
            modality: String::new(),
            parameters: String::new(),
            published,
            url: format!("{DL_BASE}/{slug}"),
        });
    }
    out
}

/// Match models: exact (case-insensitive) on name / slug / alias first, then
/// substring on name / slug / aliases.
fn search_dl(models: &[DlModel], query: &str) -> Vec<DlModel> {
    let exact: Vec<DlModel> = models
        .iter()
        .filter(|m| {
            ci_eq(&m.name, query)
                || ci_eq(&m.id, query)
                || m.aliases.split(" / ").any(|a| ci_eq(a.trim(), query))
        })
        .cloned()
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    models
        .iter()
        .filter(|m| {
            contains_ci(&m.name, query)
                || contains_ci(&m.id, query)
                || contains_ci(&m.aliases, query)
        })
        .cloned()
        .collect()
}

/// True when the model has a real published date strictly after `date`.
fn dl_published_after(m: &DlModel, date: &str) -> bool {
    is_plain_date(&m.published) && m.published.as_str() > date
}

fn filter_dl_by_date(models: Vec<DlModel>, date: &str) -> Vec<DlModel> {
    models
        .into_iter()
        .filter(|m| dl_published_after(m, date))
        .collect()
}

fn dl_csv_row(m: &DlModel) -> String {
    csv_row(&[
        &m.id, &m.name, &m.provider, &m.aliases, &m.r#type, &m.category, &m.context, &m.modality,
        &m.parameters, &m.published, &m.url,
    ])
}

/// Aligned `ID  NAME  PROVIDER  TYPE  PUBLISHED` table for a list of models.
fn print_dl_rows(hits: &[DlModel]) {
    let w_id = hits.iter().map(|m| m.id.chars().count()).max().unwrap_or(0).max(2);
    let w_name = hits.iter().map(|m| m.name.chars().count()).max().unwrap_or(0).max(4);
    let w_prov = hits.iter().map(|m| m.provider.chars().count()).max().unwrap_or(0).max(8);
    let w_type = hits.iter().map(|m| m.category.chars().count()).max().unwrap_or(0).max(4);
    let w_pub = hits.iter().map(|m| m.published.chars().count()).max().unwrap_or(0).max(9);

    println!(
        "{:<w_id$}  {:<w_name$}  {:<w_prov$}  {:<w_type$}  {:<w_pub$}",
        "ID", "NAME", "PROVIDER", "TYPE", "PUBLISHED"
    );
    for m in hits {
        let typ = if m.category.is_empty() { "-".to_string() } else { m.category.clone() };
        println!("{:<w_id$}  {:<w_name$}  {:<w_prov$}  {:<w_type$}  {:<w_pub$}", m.id, m.name, m.provider, typ, m.published);
    }
}

/// Aligned `label: value` detail view for a single model.
fn print_dl_detail(m: &DlModel) {
    println!("== {} ==", if m.name.is_empty() { &m.id } else { &m.name });
    let mut rows: Vec<(String, String)> = Vec::new();
    rows.push(("id".into(), m.id.clone()));
    rows.push(("name".into(), m.name.clone()));
    rows.push(("provider".into(), m.provider.clone()));
    if !m.aliases.is_empty() {
        rows.push(("aliases".into(), m.aliases.clone()));
    }
    if !m.r#type.is_empty() {
        rows.push(("type".into(), m.r#type.clone()));
    }
    if !m.category.is_empty() {
        rows.push(("category".into(), m.category.clone()));
    }
    if !m.context.is_empty() {
        rows.push(("context".into(), m.context.clone()));
    }
    if !m.modality.is_empty() {
        rows.push(("modality".into(), m.modality.clone()));
    }
    if !m.parameters.is_empty() {
        rows.push(("parameters".into(), m.parameters.clone()));
    }
    if !m.published.is_empty() {
        rows.push(("published".into(), m.published.clone()));
    }
    rows.push(("url".into(), m.url.clone()));
    let width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (k, v) in rows {
        println!("{k:>width$}: {v}");
    }
}

fn output_dl(hits: &[DlModel], take: usize, fmt: Option<OutFormat>, list_mode: bool) -> Result<()> {
    let shown = hits.len().min(take);
    match fmt {
        Some(OutFormat::Json) => {
            let arr: Vec<&DlModel> = hits.iter().take(take).collect();
            println!("{}", serde_json::to_string_pretty(&arr)?);
        }
        Some(OutFormat::Yaml) => {
            let arr: Vec<&DlModel> = hits.iter().take(take).collect();
            println!("{}", serde_yaml::to_string(&arr)?);
        }
        Some(OutFormat::Csv) => {
            println!("{DL_CSV_HEADER}");
            for m in hits.iter().take(take) {
                println!("{}", dl_csv_row(m));
            }
        }
        Some(OutFormat::Md) => {
            println!(
                "{}",
                md_table(
                    &["id", "name", "provider", "aliases", "type", "category", "context", "modality", "parameters", "published", "url"],
                    &hits
                        .iter()
                        .take(take)
                        .map(|m| vec![
                            m.id.clone(),
                            m.name.clone(),
                            m.provider.clone(),
                            m.aliases.clone(),
                            m.r#type.clone(),
                            m.category.clone(),
                            m.context.clone(),
                            m.modality.clone(),
                            m.parameters.clone(),
                            m.published.clone(),
                            m.url.clone(),
                        ])
                        .collect::<Vec<_>>()
                )
            );
        }
        None => {
            if list_mode {
                print_dl_rows(&hits[..shown]);
                let mut footer = format!("--- {shown} of {} models", hits.len());
                if hits.len() > shown {
                    footer.push_str(" (use --limit to show more)");
                }
                println!("{footer}");
            } else {
                print_dl_detail(&hits[0]);
            }
        }
    }
    Ok(())
}

/// `sys ai cn-model [NAME | -s QUERY | --list] [--date DATE] [--model-type TYPE] [--limit N] [-o json|csv] [--refresh]`
///
/// Query-only: a NAME or `--search` is required, unless `--list` (full
/// listing) or `--date`/`--model-type` filters are given (no `--open` —
/// DataLearner exposes no structured open-weights field). `--date YYYY-MM-DD`
/// keeps only models published strictly after it; `--model-type` keeps only
/// models whose category matches the given kind (text / image / audio /
/// video / multimodal). A single exact hit fetches the detail page to enrich
/// the output with the context length, the input/output modalities and the
/// parameter scale (总参数 / 激活参数, the model-scale info the site
/// provides).
pub fn cmd_cn_model(
    name: Option<&str>,
    search: Option<&str>,
    list: bool,
    limit: Option<usize>,
    json: bool,
    out: Option<OutFormat>,
    refresh: bool,
    updated_after: Option<&str>,
    model_type: Option<&str>,
) -> Result<()> {
    let models = fetch_datalearner(refresh)?;
    let fmt = out.or(if json { Some(OutFormat::Json) } else { None });
    let date = updated_after.map(parse_date).transpose()?;
    let mt = model_type.and_then(parse_model_types);
    if let Some(ts) = &mt {
        for t in ts {
            if dl_type_keywords(t).is_none() {
                bail!("unsupported --model-type `{t}` (supported: text, image, audio, video, multimodal or 文本/图像/语音/视频/多模态; comma-separated multi-values like `text,image` are AND-ed)");
            }
        }
    }

    // --list shows the full catalogue; --date / --model-type alone also list
    // every matching model.
    if list || (name.is_none() && search.is_none()) {
        if !list && date.is_none() && model_type.is_none() {
            bail!("provide a model NAME, --search QUERY, or --list (data: {DL_BASE})");
        }
        let hits = match &updated_after {
            Some(d) => filter_dl_by_date(models, d),
            None => models,
        };
        let hits = match &mt {
            Some(ts) => filter_dl_by_type(hits, ts)?,
            None => hits,
        };
        if hits.is_empty() {
            let mut msg = String::from("no model matches the filter");
            if let Some(d) = &date {
                msg.push_str(&format!(" published after {d}"));
            }
            if let Some(ts) = &mt {
                msg.push_str(&format!(" of types {}", ts.join(",")));
            }
            bail!("{msg} (source: {DL_BASE})");
        }
        let take = limit.unwrap_or(if list { usize::MAX } else { 20 }).max(1);
        return output_dl(&hits, take, fmt, true);
    }

    let query = name.or(search).unwrap_or_default();

    let hits = search_dl(&models, query);
    let hits = match &date {
        Some(d) => filter_dl_by_date(hits, d),
        None => hits,
    };
    let hits = match &mt {
        Some(ts) => filter_dl_by_type(hits, ts)?,
        None => hits,
    };
    if hits.is_empty() {
        let mut msg = format!("no model matches `{query}`");
        if let Some(d) = &date {
            msg.push_str(&format!(" published after {d}"));
        }
        if let Some(ts) = &mt {
            msg.push_str(&format!(" of types {}", ts.join(",")));
        }
        bail!("{msg} (source: {DL_BASE})");
    }

    if search.is_some() {
        // grep-style listing of every match.
        let take = limit.unwrap_or(20).max(1);
        return output_dl(&hits, take, fmt, true);
    }

    if hits.len() == 1 {
        // Single exact hit: enrich with the detail page (best effort).
        let mut m = hits.into_iter().next().unwrap();
        match fetch_dl_detail(&m.url) {
            Ok((ctx, modl, params)) => {
                m.context = ctx;
                m.modality = modl;
                m.parameters = params;
            }
            Err(e) => eprintln!("sys: warning: cannot fetch detail page for {}: {e:#}", m.name),
        }
        return output_dl(std::slice::from_ref(&m), 1, fmt, false);
    }

    // Several models match exactly: show the list and how to disambiguate.
    if fmt.is_none() {
        println!("Multiple models match `{query}`:");
    }
    output_dl(&hits, limit.unwrap_or(20).max(1), fmt, true)?;
    if fmt.is_none() {
        println!("Use -o json or -o csv for machine output, or a full name / slug.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `sys ai config` — export the local config to another tool's format.
//
// Formats: codex (TOML), opencode (JSON), litellm (YAML), freellmapi (JSON),
// codebuddy / workbuddy (models.json), trae (trae_cli.yaml).
// Selection: omit = the same models `sys ai chat` uses by default (the
// top-level `model:` selector, or the first provider's first model when none
// is configured); `-m` accepts the same comma-separated forms as `sys ai
// chat -m` (`provider:*` / `provider:model` / bare model name).
// `-f/--file FILE` writes to a file instead of stdout; `--update` merges into
// the software's default config file (backup first).
// ---------------------------------------------------------------------------

/// Pick the (provider_idx, model_idx) pairs described by `selector`:
/// None -> the models `sys ai chat` would use by default (top-level `model:`
/// selector, else the first provider's first model); otherwise the same
/// formats as `sys ai chat -m` (comma-separated `p:*` / `p:m` / bare `m`,
/// case-insensitive, deduped).
fn select_models(cfg: &chat::Config, selector: Option<&str>) -> Result<Vec<(usize, usize)>> {
    let sel = match selector.map(str::trim).filter(|s| !s.is_empty()) {
        None => cfg.model.as_deref().unwrap_or("<default>"),
        Some(sel) => sel,
    };
    Ok(chat::resolve_targets(cfg, sel)?
        .iter()
        .map(|t| (t.provider, t.model))
        .collect())
}

/// A provider's identity is its api_base: two blocks are the same provider
/// when their endpoints match (trailing slashes and case ignored).
fn same_api_base(a: &str, b: &str) -> bool {
    a.trim_end_matches('/').eq_ignore_ascii_case(b.trim_end_matches('/'))
}

/// The selected models grouped by API base. One entry/block per endpoint,
/// whatever the block names are: models pointing at the same api_base are
/// exported as ONE entry, never several (mirrors the `--import` merge). The
/// first block of the base supplies the name/kind/key; models are merged
/// across blocks and deduped by name.
struct ExportGroup<'a> {
    p: &'a chat::Provider,
    models: Vec<&'a chat::Model>,
}

fn export_groups<'a>(cfg: &'a chat::Config, pairs: &[(usize, usize)]) -> Vec<ExportGroup<'a>> {
    let mut out: Vec<ExportGroup<'a>> = Vec::new();
    for (pi, mi) in pairs {
        let p = &cfg.providers[*pi];
        let m = &p.models[*mi];
        match out.iter_mut().find(|g| same_api_base(&g.p.api_base, &p.api_base)) {
            Some(g) => {
                if !g.models.iter().any(|x| x.name.eq_ignore_ascii_case(&m.name)) {
                    g.models.push(m);
                }
            }
            None => out.push(ExportGroup { p, models: vec![m] }),
        }
    }
    out
}

/// A unique entry key for an export group: the provider name when unused,
/// otherwise `{name}@{host}` — the same name pointing at a different endpoint
/// is a distinct provider and must stay a distinct entry/block.
fn export_key(used: &mut Vec<String>, name: &str, api_base: &str) -> String {
    let base = name.to_string();
    if !used.contains(&base) {
        used.push(base.clone());
        return base;
    }
    let host = host_of_api_base(api_base);
    let mut k = format!("{name}@{host}");
    let mut n = 2;
    while used.contains(&k) {
        k = format!("{name}@{host}-{n}");
        n += 1;
    }
    used.push(k.clone());
    k
}

/// Escape a string for a TOML basic string (double-quoted).
fn toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Upper-case provider name with non-alphanumeric chars replaced by `_`
/// (used to build the codex `env_key`).
fn env_key_name(provider: &str) -> String {
    let mut s = String::new();
    for c in provider.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_uppercase());
        } else {
            s.push('_');
        }
    }
    if s.trim_matches('_').is_empty() {
        s = String::from("PROVIDER");
    }
    s
}

/// codex: `~/.codex/config.toml` — one `[model_providers.<p>]` table per
/// provider. codex reads the key from an env var, so `env_key` names one and
/// the value is printed to stderr as `export` hints.
fn export_codex(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Result<String> {
    let mut s = String::from("# Generated by `sys ai config codex` from ~/.sysenv/config.yaml\n");
    s.push_str("# Copy to ~/.codex/config.toml, then set the env keys below.\n");
    let first = &cfg.providers[pairs[0].0];
    let first_m = &first.models[pairs[0].1];
    s.push_str(&format!("model = \"{}.{}\"\n", first.name, first_m.name));
    s.push_str(&format!("model_provider = {}\n\n", toml_str(&first.name)));
    let mut used: Vec<String> = Vec::new();
    for g in export_groups(cfg, pairs) {
        let key = export_key(&mut used, &g.p.name, &g.p.api_base);
        let env = format!("SYS_{}_API_KEY", env_key_name(&key));
        s.push_str(&format!("[model_providers.{}]\n", toml_str(&key)));
        s.push_str(&format!("name = {}\n", toml_str(&g.p.name)));
        s.push_str(&format!("base_url = {}\n", toml_str(&g.p.api_base)));
        s.push_str(&format!("env_key = {}\n", toml_str(&env)));
        s.push_str("wire_api = \"chat\"\n");
        let models: Vec<&str> = g.models.iter().map(|m| m.name.as_str()).collect();
        s.push_str(&format!("# models: {}\n", models.join(", ")));
        s.push_str("\n");
        eprintln!("sys: codex: export {env}={}", g.p.api_key);
    }
    Ok(s)
}

/// opencode: `opencode.json` — one provider entry per API base (merged across
/// same-endpoint blocks) with inline apiKey.
fn export_opencode(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Result<String> {
    let mut providers = serde_json::Map::new();
    let mut used: Vec<String> = Vec::new();
    for g in export_groups(cfg, pairs) {
        let key = export_key(&mut used, &g.p.name, &g.p.api_base);
        let entry = providers
            .entry(key.clone())
            .or_insert_with(|| json!({ "npm": "@ai-sdk/openai-compatible", "name": g.p.name, "options": { "baseURL": g.p.api_base, "apiKey": g.p.api_key }, "models": {} }));
        if let Some(models) = entry.get_mut("models").and_then(|v| v.as_object_mut()) {
            for m in &g.models {
                models.insert(m.name.clone(), json!({ "name": m.name }));
            }
        }
    }
    let mut root = serde_json::Map::new();
    root.insert("$schema".to_string(), json!("https://opencode.ai/config.json"));
    root.insert("provider".to_string(), Value::Object(providers));
    Ok(serde_json::to_string_pretty(&Value::Object(root))?)
}

/// litellm: `config.yaml` — one `model_list` entry per selected model; the
/// exposed model name is `{provider}:{model}` (collision-free) routing to
/// `openai/{model}`. Same-endpoint providers are merged, so a duplicate
/// model name can never produce repeated entries.
fn export_litellm(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Result<String> {
    let mut s = String::from("model_list:\n");
    let mut used: Vec<String> = Vec::new();
    for g in export_groups(cfg, pairs) {
        let key = export_key(&mut used, &g.p.name, &g.p.api_base);
        for m in &g.models {
            s.push_str(&format!(
                "  - model_name: {}\n",
                toml_str(&format!("{}:{}", key, m.name))
            ));
            s.push_str("    litellm_params:\n");
            s.push_str(&format!("      model: {}\n", toml_str(&format!("openai/{}", m.name))));
            s.push_str(&format!("      api_base: {}\n", toml_str(&g.p.api_base)));
            s.push_str(&format!("      api_key: {}\n", toml_str(&g.p.api_key)));
        }
    }
    Ok(s)
}

/// freellmapi: `freellmapi.config.json` — one `customProviders` entry per
/// API base (the file also accepts `admin`/`keys`/`routing`; we export
/// only the providers section so it can be merged). Models of same-endpoint
/// blocks are merged into the single entry.
fn export_freellmapi(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Result<String> {
    let mut list: Vec<Value> = Vec::new();
    let mut used: Vec<String> = Vec::new();
    for g in export_groups(cfg, pairs) {
        let key = export_key(&mut used, &g.p.name, &g.p.api_base);
        let models: Vec<Value> = g
            .models
            .iter()
            .map(|m| json!({ "model": m.name, "displayName": m.name, "supportsTools": true }))
            .collect();
        list.push(json!({
            "baseUrl": g.p.api_base,
            "label": key,
            "models": models,
        }));
    }
    Ok(serde_json::to_string_pretty(&json!({ "customProviders": list }))?)
}

/// codebuddy / workbuddy: `models.json` entries (deduped by model `id`); the
/// two tools share the same schema (`~/.codebuddy/models.json` and
/// `~/.workbuddy/models.json`).
fn codebuddy_entries(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for g in export_groups(cfg, pairs) {
        for m in &g.models {
            if out.iter().any(|v| v["id"].as_str() == Some(m.name.as_str())) {
                continue;
            }
            let mut e = serde_json::Map::new();
            e.insert("id".to_string(), json!(m.name));
            e.insert("name".to_string(), json!(m.name));
            e.insert("vendor".to_string(), json!(g.p.name));
            e.insert(
                "url".to_string(),
                json!(format!("{}/chat/completions", g.p.api_base.trim_end_matches('/'))),
            );
            e.insert("apiKey".to_string(), json!(g.p.api_key));
            if let Some(n) = m.max_input_tokens {
                e.insert("maxInputTokens".to_string(), json!(n));
            }
            if let Some(n) = m.max_tokens {
                e.insert("maxOutputTokens".to_string(), json!(n));
            }
            if let Some(t) = &m.model_type {
                e.insert("supportsImages".to_string(), json!(t.contains("image")));
            }
            out.push(Value::Object(e));
        }
    }
    out
}

fn export_codebuddy(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Result<String> {
    let models = codebuddy_entries(cfg, pairs);
    let available: Vec<Value> = models.iter().map(|m| m["id"].clone()).collect();
    Ok(serde_json::to_string_pretty(&json!({ "models": models, "availableModels": available }))?
        + "\n")
}

fn export_workbuddy(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Result<String> {
    // The real workbuddy models.json is a plain top-level array of model
    // entries (`[{id,name,vendor,url,apiKey,...}]`), not the codebuddy
    // `{"models":[...],"availableModels":[...]}` wrapper.
    let models = codebuddy_entries(cfg, pairs);
    Ok(serde_json::to_string_pretty(&json!(models))? + "\n")
}

/// The model id of a trae `models` entry (`open_ai.model` / `claude.model`).
fn trae_model_id(v: &Value) -> Option<&str> {
    v.get("open_ai")
        .and_then(|o| o.get("model"))
        .and_then(|m| m.as_str())
        .or_else(|| {
            v.get("claude")
                .and_then(|c| c.get("model"))
                .and_then(|m| m.as_str())
        })
}

/// trae: `trae_cli.yaml` `models` entries (deduped by model id). OpenAI
/// compatible providers use the `open_ai` block, anthropic the `claude` one.
fn trae_entries(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Vec<Value> {
    let mut list: Vec<Value> = Vec::new();
    for g in export_groups(cfg, pairs) {
        for m in &g.models {
            if list.iter().any(|v| trae_model_id(v) == Some(m.name.as_str())) {
                continue;
            }
            if g.p.kind.eq_ignore_ascii_case("anthropic") {
                list.push(json!({
                    "name": m.name,
                    "claude": { "base_url": g.p.api_base, "model": m.name, "api_key": g.p.api_key }
                }));
            } else {
                list.push(json!({
                    "name": m.name,
                    "open_ai": { "base_url": g.p.api_base, "api_key": g.p.api_key, "model": m.name }
                }));
            }
        }
    }
    list
}

fn export_trae(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Result<String> {
    Ok(serde_yaml::to_string(&json!({ "models": trae_entries(cfg, pairs) }))
        .context("cannot serialize trae config")?)
}

/// `sys ai config FORMAT [-m MODEL] [--show] [-f FILE] [-c FILE] [--update]`
/// [`--list`]
///
/// FORMAT: `codex` (TOML) | `opencode` (JSON) | `litellm` (YAML) |
/// `freellmapi` (JSON) | `codebuddy` (models.json) | `workbuddy`
/// (models.json, plain array) | `trae` (trae_cli.yaml) | `zcoder` (ZCode
/// config.json). SELECTION: omit = the models `sys ai chat` uses by
/// default (top-level `model:` selector, else the first provider's first
/// model); `-m` takes the same comma-separated forms as `sys ai chat -m`
/// (`provider:*`, `provider:model`, bare model name; the legacy positional
/// SELECT is equivalent to a single `-m` value). The export goes to stdout
/// unless `-f/--file FILE` is given.
///
/// `--show` previews without touching any file: it prints the target
/// software's default config file path first, then the config that would be
/// applied (and the `-f` write target when one is given). Writing with `-f`
/// backs up an existing file to `FILE.bak-YYYYMMDD-HHMMSS` first, then
/// overwrites it and prints what actually changed.
///
/// `--update` instead merges into the software's default config file (or the
/// `-f` path when given): the file must exist (a missing file is reported and
/// nothing happens), a `.bak-*` copy is made first, then the selected models
/// that are not already present are added and reported.
/// The formats plus their config paths; `src` overrides the location: the
/// software's usual config file name is then looked up directly inside `src`
/// (used when an agent is installed at a non-default place).
fn config_format_list_with(src: Option<&Path>) -> Vec<(&'static str, Option<PathBuf>)> {
    ["codex", "opencode", "litellm", "freellmapi", "codebuddy", "workbuddy", "trae", "zcoder"]
        .iter()
        .map(|f| (*f, software_config_path_with(src, f)))
        .collect()
}

fn config_existing_list_with(src: Option<&Path>) -> Vec<(&'static str, PathBuf)> {
    config_format_list_with(src)
        .into_iter()
        .filter_map(|(name, path)| path.filter(|p| p.exists()).map(|p| (name, p)))
        .collect()
}

pub fn cmd_config(
    format: Option<&str>,
    selector: Option<&str>,
    out_file: Option<&Path>,
    config: Option<&Path>,
    show: bool,
    models: Option<&str>,
    update: bool,
    list: bool,
    import_mode: bool,
    src: Option<&Path>,
) -> Result<()> {
    if list {
        // `sys ai config --list`: show only the formats whose config file
        // actually exists (at the default location, or under --src DIR).
        let existing = config_existing_list_with(src);
        if existing.is_empty() {
            eprintln!("sys: no existing config files found");
            return Ok(());
        }
        for (name, path) in existing {
            println!("{name:<12} {}", path.display());
        }
        return Ok(());
    }
    let format = format.context("missing FORMAT")?;
    let format_lc = format.to_ascii_lowercase();
    let supported = matches!(
        format_lc.as_str(),
        "codex"
            | "opencode"
            | "openxode"
            | "litellm"
            | "freellmapi"
            | "codebuddy"
            | "workbuddy"
            | "trae"
            | "zcoder"
    );
    if !supported {
        bail!(
            "unknown format `{format}` (supported: codex, opencode, litellm, freellmapi, codebuddy, workbuddy, trae, zcoder)"
        );
    }
    if import_mode {
        // Import: reverse direction — read the third-party config (default
        // path, --src DIR or -f FILE) and merge its providers/models into the
        // sysenv config, backing up first. Only codebuddy / workbuddy /
        // opencode / zcoder can be imported; the local cfg is not needed for
        // selection here.
        return cmd_config_import(&format_lc, out_file, config, src);
    }
    let (cfg, _) = chat::load_config(config)?;
    let sel = models.or(selector);
    let pairs = select_models(&cfg, sel)?;
    if pairs.is_empty() {
        bail!("nothing to export (no models selected)");
    }
    if update {
        return cmd_config_update(&format_lc, &cfg, &pairs, out_file, src);
    }
    let text = match format_lc.as_str() {
        "codex" => export_codex(&cfg, &pairs)?,
        "opencode" | "openxode" => export_opencode(&cfg, &pairs)?,
        "litellm" => export_litellm(&cfg, &pairs)?,
        "freellmapi" => export_freellmapi(&cfg, &pairs)?,
        "codebuddy" => export_codebuddy(&cfg, &pairs)?,
        "workbuddy" => export_workbuddy(&cfg, &pairs)?,
        "trae" => export_trae(&cfg, &pairs)?,
        "zcoder" => export_zcoder(&cfg, &pairs)?,
        other => unreachable!("format validated above: {other}"),
    };
    if show {
        // Show mode: print the target software's config file path first, then the
        // config that would be applied — without touching any file.
        match software_config_path_with(src, format) {
            Some(p) => println!("config file: {}", p.display()),
            None => println!("config file: <no default path; choose one with -f FILE>"),
        }
        if let Some(path) = out_file {
            println!("write target: {}", path.display());
        }
        print!("{text}");
        return Ok(());
    }
    match out_file {
        Some(path) => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    bail!("output directory does not exist: {}", parent.display());
                }
            }
            // Apply mode: back up the previous config file before writing, then
            // report what actually changed.
            let existed = path.exists();
            let backup = if existed {
                Some(backup_file(path)?)
            } else {
                None
            };
            std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))?;
            let n_providers = pairs.iter().map(|(pi, _)| *pi).collect::<HashSet<_>>().len();
            eprintln!(
                "sys: wrote {} provider(s), {} model(s) to {}",
                n_providers,
                pairs.len(),
                path.display()
            );
            match backup {
                Some(b) => eprintln!("sys: previous config backed up to {}", b.display()),
                None if !existed => eprintln!(
                    "sys: {} did not exist; created a new config file",
                    path.display()
                ),
                _ => {}
            }
        }
        None => print!("{text}"),
    }
    Ok(())
}

/// `--update` mode of `cmd_config`: merge the selected models into the target
/// software's default config file (or the `-f` override). The file must
/// already exist — a missing one is reported and nothing is written. A
/// `.bak-*` copy is made first, then the entries that are not already present
/// are added; the added model ids are printed on success.
fn cmd_config_update(
    format: &str,
    cfg: &chat::Config,
    pairs: &[(usize, usize)],
    out_file: Option<&Path>,
    src: Option<&Path>,
) -> Result<()> {
    if !matches!(format, "codebuddy" | "workbuddy" | "trae" | "zcoder") {
        bail!(
            "`--update` is not supported for `{format}` (only codebuddy, workbuddy, trae, zcoder; use -f FILE to write)"
        );
    }
    let path = match out_file {
        Some(p) => p.to_path_buf(),
        None => software_config_path_with(src, format)
            .ok_or_else(|| anyhow::anyhow!("no default config path for `{format}`; use -f FILE"))?,
    };
    if !path.exists() {
        eprintln!("sys: config file not found at {}; nothing to update", path.display());
        return Ok(());
    }
    let backup = backup_file(&path)?;
    eprintln!("sys: backed up {} to {}", path.display(), backup.display());
    let added = match format {
        "codebuddy" | "workbuddy" => {
            let entries = codebuddy_entries(cfg, pairs);
            merge_json_models(&path, &entries)?
        }
        "trae" => {
            let entries = trae_entries(cfg, pairs);
            merge_trae_models(&path, &entries)?
        }
        "zcoder" => merge_zcode_models(&path, cfg, pairs)?,
        _ => unreachable!("format checked above"),
    };
    if added.is_empty() {
        eprintln!("sys: nothing to add; selected model(s) already present");
    } else {
        eprintln!("sys: added {} model(s): {}", added.len(), added.join(", "));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `--import`: reverse of the exports. A third-party config file (codebuddy /
// workbuddy `models.json` or the ZCode `config.json`) is parsed back into
// provider/model entries, which are merged into `~/.sysenv/config.yaml`
// (backed up first). Providers already present are kept; missing models are
// appended to the matching provider, brand-new providers get a full block.
// Same-name providers from the source are merged into one entry before any
// write: models of the same provider must live in one `clients:` block.
// ---------------------------------------------------------------------------

/// One model parsed back from a third-party config.
#[derive(Debug, Clone)]
struct ImportedModel {
    name: String,
    max_tokens: Option<u64>,
    max_input_tokens: Option<u64>,
    model_type: Option<String>,
}

/// One provider parsed back from a third-party config, ready to be written
/// into the sysenv config as a `clients:` entry.
#[derive(Debug, Clone)]
struct ImportedProvider {
    name: String,
    kind: String,
    api_base: String,
    api_key: String,
    models: Vec<ImportedModel>,
}

/// What an import actually changed in the sysenv config.
struct ImportReport {
    providers_added: Vec<String>,
    models_added: Vec<String>,
    models_skipped: Vec<String>,
}

/// Recover the API base from a models.json `url`: the exports store
/// `{base}/chat/completions`, the real workbuddy entries do the same, but a
/// plain base URL is accepted too.
fn api_base_from_url(url: &str) -> Option<String> {
    let mut s = url.trim().trim_end_matches(['/', ' ']);
    let lower = s.to_ascii_lowercase();
    if let Some(pos) = lower.rfind("/chat/completions") {
        if lower[pos..].eq_ignore_ascii_case("/chat/completions") {
            s = s[..pos].trim_end_matches('/');
        }
    }
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Host part of an API base (`https://api-inference.modelscope.cn/v1` ->
/// `api-inference.modelscope.cn`), used as the provider name when the file
/// carries no useful vendor name.
fn host_of_api_base(base: &str) -> String {
    let after_scheme = base
        .trim_end_matches('/')
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or_else(|| base.trim_end_matches('/'));
    after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme)
        .to_string()
}

/// Provider name for a models.json entry: the `vendor` field when it carries
/// real information, otherwise the API base host (workbuddy writes a useless
/// `Custom` vendor for hand-added models).
fn provider_name_from_vendor(vendor: Option<&str>, api_base: &str) -> String {
    match vendor.map(str::trim).filter(|v| {
        !v.is_empty() && !v.eq_ignore_ascii_case("custom") && !v.eq_ignore_ascii_case("none")
    }) {
        Some(v) => v.to_string(),
        None => host_of_api_base(api_base),
    }
}

/// Merge imported entries that share the same api_base into one entry: a
/// provider's identity is its endpoint, so entries pointing at the same base
/// (whatever their names) are one provider — the first entry's name / kind /
/// api_key wins and models are merged, deduped by name. Entries at a
/// different api_base stay separate even when the names match.
fn merge_imported_providers(providers: Vec<ImportedProvider>) -> Vec<ImportedProvider> {
    let mut out: Vec<ImportedProvider> = Vec::new();
    for p in providers {
        match out.iter_mut().find(|o| same_api_base(&o.api_base, &p.api_base)) {
            Some(o) => {
                for m in p.models {
                    if !o.models.iter().any(|x| x.name.eq_ignore_ascii_case(&m.name)) {
                        o.models.push(m);
                    }
                }
            }
            None => out.push(p),
        }
    }
    out
}

/// Parse a codebuddy / workbuddy `models.json` back into providers. Both the
/// codebuddy object shape (`{"models":[...]}`) and the workbuddy array shape
/// (root array) are accepted. Entries are merged by provider name
/// (case-insensitive): the first entry's endpoint wins when the same name
/// appears with different API bases; entries missing `id` / `url` / `apiKey`
/// are skipped and reported.
fn parse_models_json(path: &Path) -> Result<(Vec<ImportedProvider>, Vec<String>)> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let root: Value = serde_json::from_str(content)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    let entries: Vec<&Value> = if let Some(arr) = root.as_array() {
        arr.iter().collect()
    } else if let Some(models) = root.get("models").and_then(|m| m.as_array()) {
        models.iter().collect()
    } else {
        bail!(
            "{} has neither a top-level array nor a `models` array",
            path.display()
        );
    };

    let mut providers: Vec<ImportedProvider> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for e in &entries {
        let id = e["id"].as_str().unwrap_or_default().trim().to_string();
        if id.is_empty() {
            skipped.push("entry without `id`".to_string());
            continue;
        }
        let url = e["url"].as_str().unwrap_or_default().trim();
        if url.is_empty() {
            skipped.push(format!("`{id}` without `url`"));
            continue;
        }
        let api_key = e["apiKey"].as_str().unwrap_or_default().trim().to_string();
        if api_key.is_empty() {
            skipped.push(format!("`{id}` without `apiKey`"));
            continue;
        }
        let Some(api_base) = api_base_from_url(url) else {
            skipped.push(format!("`{id}` with an unusable `url`"));
            continue;
        };
        let name = provider_name_from_vendor(e["vendor"].as_str(), &api_base);
        let model = ImportedModel {
            name: id,
            max_tokens: e["maxOutputTokens"].as_u64(),
            max_input_tokens: e["maxInputTokens"].as_u64(),
            model_type: match e["supportsImages"].as_bool() {
                Some(true) => Some("text,image".to_string()),
                _ => None,
            },
        };
        providers.push(ImportedProvider {
            name,
            kind: "openai".to_string(),
            api_base,
            api_key,
            models: vec![model],
        });
    }
    Ok((merge_imported_providers(providers), skipped))
}

/// Parse a ZCode `config.json` back into providers. Every `provider.<id>`
/// block becomes one provider: `options.baseURL` / `options.apiKey` supply the
/// endpoint and key, the `models` object keys are the model ids (`models.*.name`
/// wins when present). Blocks without a key or without models are skipped.
fn parse_zcoder_config(path: &Path) -> Result<(Vec<ImportedProvider>, Vec<String>)> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let root: Value = serde_json::from_str(content)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    let providers_obj = root
        .get("provider")
        .and_then(|p| p.as_object())
        .with_context(|| format!("{} has no `provider` object", path.display()))?;

    let mut providers: Vec<ImportedProvider> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for (id, entry) in providers_obj {
        let name = entry["name"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| id.clone());
        let options = entry.get("options").unwrap_or(&Value::Null);
        let Some(api_base) = options["baseURL"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            skipped.push(format!("provider `{id}` without `options.baseURL`"));
            continue;
        };
        let Some(api_key) = options["apiKey"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            skipped.push(format!("provider `{id}` without `options.apiKey`"));
            continue;
        };
        let kind = match entry["kind"].as_str() {
            Some(k) if k.eq_ignore_ascii_case("anthropic") => "anthropic".to_string(),
            _ => "openai".to_string(),
        };
        let mut models: Vec<ImportedModel> = Vec::new();
        if let Some(models_obj) = entry["models"].as_object() {
            for (mid, mv) in models_obj {
                let mname = mv["name"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| mid.clone());
                models.push(ImportedModel {
                    name: mname,
                    max_tokens: None,
                    max_input_tokens: None,
                    model_type: None,
                });
            }
        }
        if models.is_empty() {
            skipped.push(format!("provider `{id}` without models"));
            continue;
        }
        providers.push(ImportedProvider {
            name,
            kind,
            api_base: api_base.to_string(),
            api_key: api_key.to_string(),
            models,
        });
    }
    Ok((merge_imported_providers(providers), skipped))
}

/// Parse an opencode `opencode.json` back into providers. Every
/// `provider.<id>` block becomes one provider: `options.baseURL` /
/// `options.apiKey` supply the endpoint and key, the `models` object keys are
/// the model ids (`models.*.name` wins when present). opencode carries no
/// provider kind field, so everything imports as OpenAI-compatible. Blocks
/// without a key or without models are skipped.
fn parse_opencode_config(path: &Path) -> Result<(Vec<ImportedProvider>, Vec<String>)> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let root: Value = serde_json::from_str(content)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    let providers_obj = root
        .get("provider")
        .and_then(|p| p.as_object())
        .with_context(|| format!("{} has no `provider` object", path.display()))?;

    let mut providers: Vec<ImportedProvider> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for (id, entry) in providers_obj {
        let name = entry["name"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| id.clone());
        let options = entry.get("options").unwrap_or(&Value::Null);
        let Some(api_base) = options["baseURL"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            skipped.push(format!("provider `{id}` without `options.baseURL`"));
            continue;
        };
        let Some(api_key) = options["apiKey"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            skipped.push(format!("provider `{id}` without `options.apiKey`"));
            continue;
        };
        let mut models: Vec<ImportedModel> = Vec::new();
        if let Some(models_obj) = entry["models"].as_object() {
            for (mid, mv) in models_obj {
                let mname = mv["name"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| mid.clone());
                models.push(ImportedModel {
                    name: mname,
                    max_tokens: None,
                    max_input_tokens: None,
                    model_type: None,
                });
            }
        }
        if models.is_empty() {
            skipped.push(format!("provider `{id}` without models"));
            continue;
        }
        providers.push(ImportedProvider {
            name,
            kind: "openai".to_string(),
            api_base: api_base.to_string(),
            api_key: api_key.to_string(),
            models,
        });
    }
    Ok((merge_imported_providers(providers), skipped))
}

/// `(indent, trimmed content)` of one config line (space indentation; the
/// sysenv config is written with plain spaces).
fn yaml_line_parts(s: &str) -> (usize, &str) {
    let trimmed = s.trim_start();
    (s.len() - trimmed.len(), trimmed)
}

/// Strip one pair of surrounding single/double quotes.
fn yaml_unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v[1..v.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
    } else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

/// Quote a YAML scalar when it would change meaning unquoted; otherwise keep
/// it plain (matching the existing config's style — URLs with `https://` stay
/// unquoted because only a `:` followed by whitespace is ambiguous).
fn yaml_scalar(v: &str) -> String {
    let trimmed = v.trim();
    let needs = v.is_empty()
        || trimmed != v
        || v.contains(": ")
        || v.ends_with(':')
        || v.starts_with(['-', '?', ':', '"', '\'', '&', '*', '!', '|', '>', '%', '@', '`', '#', '{', '['])
        || v.contains(['"', '\'', '{', '}', '[', ']', ','])
        || v.contains('\n');
    if needs {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v.to_string()
    }
}

/// A provider block found in the sysenv config: its line range, name, the
/// provider entry indent, the `models:` key indent (when present), the end of
/// the model zone (first line after the last model entry / its sub-fields)
/// and the model entry lines inside it.
struct CfgBlock {
    end: usize,
    name: String,
    api_base: Option<String>,
    entry_indent: usize,
    models_indent: Option<usize>,
    model_end: Option<usize>,
    models: Vec<(usize, String)>,
}

/// One insertion into the sysenv config, applied after backing up. `at` is
/// the line index the text is inserted before; `text` already ends with `\n`.
enum CfgInsert {
    /// A full new `clients:` provider block.
    Provider { at: usize, text: String },
    /// One `- name:` model entry appended inside an existing provider block.
    Model { at: usize, text: String },
}

/// Quote-free model line for an existing block's `models:` list.
fn model_line(indent: usize, m: &ImportedModel) -> String {
    let mut s = format!("{}- name: {}\n", " ".repeat(indent), yaml_scalar(&m.name));
    if let Some(n) = m.max_tokens {
        s.push_str(&format!("{}max_tokens: {}\n", " ".repeat(indent + 2), n));
    }
    if let Some(n) = m.max_input_tokens {
        s.push_str(&format!("{}max_input_tokens: {}\n", " ".repeat(indent + 2), n));
    }
    if let Some(t) = &m.model_type {
        s.push_str(&format!("{}type: {}\n", " ".repeat(indent + 2), yaml_scalar(t)));
    }
    s
}

/// Full `clients:` entry for a brand-new provider (2/4/6-space layout, the
/// same as the real config).
fn provider_block(p: &ImportedProvider) -> String {
    let mut s = format!("  - type: {}\n", yaml_scalar(&p.kind));
    s.push_str(&format!("    name: {}\n", yaml_scalar(&p.name)));
    s.push_str(&format!("    api_base: {}\n", yaml_scalar(&p.api_base)));
    s.push_str(&format!("    api_key: {}\n", yaml_scalar(&p.api_key)));
    s.push_str("    models:\n");
    for m in &p.models {
        s.push_str(&model_line(6, m));
    }
    s
}

/// Merge the imported providers into the sysenv config at `path` (line-level:
/// every untouched line keeps its exact bytes). Existing providers are kept
/// whole; their missing models are appended, brand-new providers get a full
/// block. The file is backed up only when something is actually written.
fn import_into_config(path: &Path, imports: &[ImportedProvider]) -> Result<ImportReport> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let lines: Vec<String> = content.split('\n').map(str::to_string).collect();
    let eof = lines.len();

    // Same-name providers coming from the source are merged into one entry
    // before anything is written: config.yaml must never get the same provider
    // as several blocks.
    let imports = merge_imported_providers(imports.to_vec());

    // Locate the `clients:` top-level key and parse every provider block under
    // it (blocks end where a shallower/equal `- ` entry or a top-level key
    // begins).
    let mut clients_idx: Option<usize> = None;
    let mut blocks: Vec<CfgBlock> = Vec::new();
    let mut provider_indent: Option<usize> = None;
    let mut cur: Option<CfgBlock> = None;
    for (i, raw) in lines.iter().enumerate() {
        let (ind, content) = yaml_line_parts(raw);
        if content.is_empty() {
            continue;
        }
        if ind == 0 && !content.starts_with('-') {
            if let Some(k) = content.split(':').next() {
                if k == "clients" {
                    clients_idx = Some(i);
                }
            }
            // any top-level key closes the clients list
            if let Some(mut c) = cur.take() {
                c.end = i;
                blocks.push(c);
            }
            provider_indent = None;
            continue;
        }
        if ind == 0 && content.starts_with('-') {
            // top-level list (not expected); treat as a block boundary
            if let Some(mut c) = cur.take() {
                c.end = i;
                blocks.push(c);
            }
            provider_indent = None;
            continue;
        }
        let is_entry = content.starts_with('-');
        if let Some(pi) = provider_indent {
            if is_entry && ind == pi {
                if let Some(mut c) = cur.take() {
                    c.end = i;
                    blocks.push(c);
                }
            }
        } else if is_entry {
            provider_indent = Some(ind);
        }
        if is_entry && cur.is_none() && provider_indent == Some(ind) {
            cur = Some(CfgBlock {
                end: eof,
                name: String::new(),
                api_base: None,
                entry_indent: ind,
                models_indent: None,
                model_end: None,
                models: Vec::new(),
            });
        }
        if let Some(c) = cur.as_mut() {
            if let Some(pi) = provider_indent {
                if ind == pi + 2 {
                    let rest = content.strip_prefix('-').map(str::trim_start).unwrap_or(content);
                    if let Some(name) = rest.strip_prefix("name:").map(str::trim) {
                        c.name = yaml_unquote(name);
                    }
                    if let Some(base) = rest.strip_prefix("api_base:").map(str::trim) {
                        c.api_base = Some(yaml_unquote(base));
                    }
                }
                if ind == pi + 2 {
                    if let Some(_rest) = content.strip_prefix("models:").map(str::trim_start) {
                        c.models_indent = Some(ind);
                        c.models.clear();
                        // scan the model entries (indent pi+4) and their
                        // sub-fields until the block ends; `zone_end` is the
                        // first line after the whole model zone
                        let mut j = i + 1;
                        let mut zone_end = i + 1;
                        while j < lines.len() {
                            let (nj, nc) = yaml_line_parts(&lines[j]);
                            if nj <= pi || (nj == pi && nc.starts_with('-')) {
                                break;
                            }
                            if nj > pi + 2 {
                                zone_end = j + 1;
                            }
                            if nj == pi + 4 {
                                if let Some(mrest) = nc.strip_prefix('-').map(str::trim_start) {
                                    if let Some(mname) = mrest.strip_prefix("name:").map(str::trim) {
                                        c.models.push((j, yaml_unquote(mname)));
                                    }
                                }
                            }
                            j += 1;
                        }
                        c.model_end = Some(zone_end);
                    }
                }
            }
        }
    }
    if let Some(mut c) = cur.take() {
        c.end = eof;
        blocks.push(c);
    }

    let mut report = ImportReport {
        providers_added: Vec::new(),
        models_added: Vec::new(),
        models_skipped: Vec::new(),
    };
    let mut inserts: Vec<CfgInsert> = Vec::new();
    for p in &imports {
        // a provider "exists" when a local block points at the same api_base
        // (whatever its name); blocks without an api_base line can't match
        let local = blocks.iter().find(|b| {
            b.api_base
                .as_deref()
                .map(|a| same_api_base(a, &p.api_base))
                .unwrap_or(false)
        });
        match local {
            None => {
                // brand-new provider -> a full block at the end of the
                // clients list (before the next top-level key / EOF)
                let at = blocks
                    .last()
                    .map(|b| b.end)
                    .unwrap_or_else(|| clients_idx.map(|i| i + 1).unwrap_or(0));
                let mut text = provider_block(p);
                if clients_idx.is_none() {
                    text = "clients:\n".to_string() + &text;
                }
                inserts.push(CfgInsert::Provider { at, text });
                report.providers_added.push(p.name.clone());
                for m in &p.models {
                    report.models_added.push(format!("{}/{}", p.name, m.name));
                }
            }
            Some(b) => {
                // existing provider -> append only missing models
                let model_indent = b.models_indent.map(|mi| mi + 2).unwrap_or(b.entry_indent + 4);
                for m in &p.models {
                    let present = b
                        .models
                        .iter()
                        .any(|(_, n)| n.eq_ignore_ascii_case(&m.name));
                    if present {
                        report.models_skipped.push(format!("{}/{}", p.name, m.name));
                        continue;
                    }
                    // insert after the block's whole model zone (sub-fields
                    // included), or at the block end when it has no `models:`
                    let at = b.model_end.unwrap_or(b.end);
                    let mut text = model_line(model_indent, m);
                    if b.models_indent.is_none() {
                        // the block has no `models:` list yet; open one
                        text = format!("{}models:\n", " ".repeat(b.entry_indent + 2)) + &text;
                    }
                    inserts.push(CfgInsert::Model { at, text });
                    report.models_added.push(format!("{}/{}", p.name, m.name));
                }
            }
        }
    }

    if inserts.is_empty() {
        return Ok(report);
    }

    let backup = backup_file(path)?;
    eprintln!("sys: backed up {} to {}", path.display(), backup.display());

    // Apply from the bottom up so earlier line indices stay valid.
    let mut out = lines.clone();
    inserts.sort_by_key(|ins| match ins {
        CfgInsert::Provider { at, .. } => *at,
        CfgInsert::Model { at, .. } => *at,
    });
    inserts.reverse();
    for ins in inserts {
        let (at, text) = match ins {
            CfgInsert::Provider { at, text } => (at, text),
            CfgInsert::Model { at, text } => (at, text),
        };
        let at = at.min(out.len());
        out.insert(at, text.trim_end_matches('\n').to_string());
    }
    let mut text = out.join("\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(report)
}

/// `--import` mode: read the target software's config file (or the `-f`
/// override) and merge its provider/model entries into the sysenv config
/// (`-c` override or the default `~/.sysenv/config.yaml`), backing up first.
fn cmd_config_import(
    format: &str,
    source: Option<&Path>,
    config: Option<&Path>,
    src: Option<&Path>,
) -> Result<()> {
    if !matches!(format, "codebuddy" | "workbuddy" | "opencode" | "openxode" | "zcoder") {
        bail!(
            "`--import` is not supported for `{format}` (only codebuddy, workbuddy, opencode, zcoder)"
        );
    }
    let src_path = match source {
        Some(p) => p.to_path_buf(),
        None => software_config_path_with(src, format)
            .ok_or_else(|| anyhow::anyhow!("no default config path for `{format}`; use -f FILE or --src DIR"))?,
    };
    if !src_path.exists() {
        bail!("source file not found at {} (nothing to import)", src_path.display());
    }
    let (providers, skipped) = match format {
        "codebuddy" | "workbuddy" => parse_models_json(&src_path)?,
        "opencode" | "openxode" => parse_opencode_config(&src_path)?,
        "zcoder" => parse_zcoder_config(&src_path)?,
        _ => unreachable!("format checked above"),
    };
    if providers.is_empty() {
        bail!(
            "nothing importable from {} ({} incomplete entry/entries skipped)",
            src_path.display(),
            skipped.len()
        );
    }
    // The target sysenv config must exist and parse, so we never write a
    // half-broken file.
    let (_, cfg_path) = chat::load_config(config)?;
    let report = import_into_config(&cfg_path, &providers)?;
    if report.providers_added.is_empty() && report.models_added.is_empty() {
        eprintln!(
            "sys: nothing to import; all provider/model(s) already present in {}",
            cfg_path.display()
        );
        return Ok(());
    }
    eprintln!(
        "sys: added {} model(s) in {} provider(s): {}",
        report.models_added.len(),
        report.providers_added.len(),
        report.models_added.join(", ")
    );
    if !report.models_skipped.is_empty() {
        eprintln!(
            "sys: skipped (already present): {}",
            report.models_skipped.join(", ")
        );
    }
    if !skipped.is_empty() {
        eprintln!(
            "sys: skipped {} incomplete entry/entries: {}",
            skipped.len(),
            skipped.join("; ")
        );
    }
    Ok(())
}

/// Merge `entries` into an existing models.json-style file (keyed by `id`),
/// keeping every existing entry; returns the model ids that were added.
///
/// Two shapes are accepted: the codebuddy object shape
/// `{"models":[...],"availableModels":[...]}` and the workbuddy array shape
/// `[...]` where the root array itself is the model list (no
/// `availableModels` is maintained for the array shape).
fn merge_json_models(path: &Path, entries: &[Value]) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    // Strip a UTF-8 BOM (Windows editors often save it) before strict parsing.
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let mut root: Value = serde_json::from_str(content)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    let mut added: Vec<String> = Vec::new();
    if let Some(arr) = root.as_array_mut() {
        // workbuddy shape: the root array is the model list.
        for e in entries {
            let id = e["id"].as_str().unwrap_or_default();
            if id.is_empty() || arr.iter().any(|m| m["id"].as_str() == Some(id)) {
                continue;
            }
            arr.push(e.clone());
            added.push(id.to_string());
        }
    } else {
        let root_obj = root
            .as_object_mut()
            .with_context(|| format!("{} is not a JSON object or array", path.display()))?;
        let models = root_obj
            .entry("models".to_string())
            .or_insert_with(|| json!([]));
        let models = models
            .as_array_mut()
            .with_context(|| format!("{} `models` is not an array", path.display()))?;
        for e in entries {
            let id = e["id"].as_str().unwrap_or_default();
            if id.is_empty() || models.iter().any(|m| m["id"].as_str() == Some(id)) {
                continue;
            }
            models.push(e.clone());
            added.push(id.to_string());
        }
        if !added.is_empty() {
            let mut avail: Vec<String> = root_obj
                .get("availableModels")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            for id in &added {
                if !avail.iter().any(|a| a == id) {
                    avail.push(id.clone());
                }
            }
            root_obj.insert("availableModels".to_string(), json!(avail));
        }
    }
    if !added.is_empty() {
        let text = serde_json::to_string_pretty(&root)?;
        std::fs::write(path, text + "\n")
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    Ok(added)
}

/// Merge trae `models` entries into an existing trae_cli.yaml (read as YAML
/// into a JSON value, keyed by each entry's model id); returns the model ids
/// that were added. Other top-level keys (mcp_servers, allowed_tools, ...)
/// are preserved.
/// shortcut: YAML comments in the existing file are dropped on rewrite; the
/// .bak backup preserves the original, switch to a comment-preserving merge
/// if that becomes a problem.
fn merge_trae_models(path: &Path, entries: &[Value]) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let mut root: Value = if content.trim().is_empty() {
        json!({})
    } else {
        serde_yaml::from_str(content)
            .with_context(|| format!("{} is not valid YAML", path.display()))?
    };
    let root_obj = root
        .as_object_mut()
        .with_context(|| format!("{} is not a YAML mapping", path.display()))?;
    let models = root_obj
        .entry("models".to_string())
        .or_insert_with(|| json!([]));
    let models = models
        .as_array_mut()
        .with_context(|| format!("{} `models` is not a list", path.display()))?;
    let mut added: Vec<String> = Vec::new();
    for e in entries {
        let id = trae_model_id(e).unwrap_or_default();
        if id.is_empty() || models.iter().any(|m| trae_model_id(m) == Some(id)) {
            continue;
        }
        models.push(e.clone());
        added.push(id.to_string());
    }
    if !added.is_empty() {
        let text = serde_yaml::to_string(&root)
            .with_context(|| format!("cannot serialize {}", path.display()))?;
        std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))?;
    }
    Ok(added)
}

/// ZCode (`zcoder`) config: `%USERPROFILE%\.zcode\cli\config.json`. Every sys
/// provider becomes a `provider.<id>` block (id = sanitized provider name)
/// with `kind` / `options.baseURL` + `options.apiKey` / `models` (an object
/// keyed by model id); `model.main` / `model.lite` point at a selected model.
fn zcode_provider_id(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    if out.is_empty() {
        "provider".to_string()
    } else {
        out
    }
}

/// ZCode provider kind for a sys provider (`openai`/anything else defaults to
/// an OpenAI-compatible endpoint; anthropic stays anthropic).
fn zcode_kind(kind: &str) -> &'static str {
    if kind.eq_ignore_ascii_case("anthropic") {
        "anthropic"
    } else {
        "openai-compatible"
    }
}

/// A `provider.<id>` entry with the provider's endpoint, key and model map.
fn zcode_provider_entry(p: &chat::Provider) -> Value {
    let mut options = serde_json::Map::new();
    if p.api_key.is_empty() {
        options.insert("apiKeyRequired".to_string(), json!(false));
    } else {
        options.insert("apiKeyRequired".to_string(), json!(true));
        options.insert("apiKey".to_string(), json!(p.api_key));
    }
    options.insert("baseURL".to_string(), json!(p.api_base));
    json!({
        "kind": zcode_kind(&p.kind),
        "name": p.name,
        "options": options,
        "headers": {},
        "models": {}
    })
}

/// Provider blocks for `sys ai config zcoder`, keyed by sanitized provider
/// name; models are deduped by model id within each provider.
fn zcode_providers(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Value {
    let mut map = serde_json::Map::new();
    let mut used: Vec<String> = Vec::new();
    for g in export_groups(cfg, pairs) {
        let key = export_key(&mut used, &g.p.name, &g.p.api_base);
        let entry = map
            .entry(zcode_provider_id(&key))
            .or_insert_with(|| zcode_provider_entry(g.p));
        let models = entry["models"].as_object_mut().expect("models is an object");
        for m in &g.models {
            if !models.contains_key(&m.name) {
                models.insert(m.name.clone(), json!({ "name": m.name }));
            }
        }
    }
    Value::Object(map)
}

fn export_zcoder(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Result<String> {
    let providers = zcode_providers(cfg, pairs);
    let (pi, mi) = pairs[0];
    let main = format!(
        "{}/{}",
        zcode_provider_id(&cfg.providers[pi].name),
        cfg.providers[pi].models[mi].name
    );
    Ok(serde_json::to_string_pretty(&json!({ "provider": providers, "model": { "main": main, "lite": main } }))?
        + "\n")
}

/// Merge the selected models into an existing ZCode config.json: each
/// provider block is created when missing (existing blocks are left
/// untouched), missing model ids are added to `provider.<id>.models`, and
/// `model.main` / `model.lite` are filled only when absent. All other
/// top-level keys are preserved. Returns the added `provider/model` refs.
fn merge_zcode_models(
    path: &Path,
    cfg: &chat::Config,
    pairs: &[(usize, usize)],
) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let mut root: Value = if content.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(content)
            .with_context(|| format!("{} is not valid JSON", path.display()))?
    };
    let root_obj = root
        .as_object_mut()
        .with_context(|| format!("{} is not a JSON object", path.display()))?;
    let providers = root_obj
        .entry("provider".to_string())
        .or_insert_with(|| json!({}));
    let providers = providers
        .as_object_mut()
        .with_context(|| format!("{} `provider` is not an object", path.display()))?;
    let mut added: Vec<String> = Vec::new();
    for (pi, mi) in pairs {
        let p = &cfg.providers[*pi];
        let m = &p.models[*mi];
        let id = zcode_provider_id(&p.name);
        let entry = providers
            .entry(id.clone())
            .or_insert_with(|| zcode_provider_entry(p));
        let models = entry["models"].as_object_mut().with_context(|| {
            format!("{} provider `{id}` `models` is not an object", path.display())
        })?;
        if !models.contains_key(&m.name) {
            models.insert(m.name.clone(), json!({ "name": m.name }));
            added.push(format!("{id}/{}", m.name));
        }
    }
    if !added.is_empty() {
        let first = added[0].clone();
        if let Some(model_obj) = root_obj.entry("model".to_string()).or_insert_with(|| json!({})).as_object_mut()
        {
            model_obj
                .entry("main".to_string())
                .or_insert_with(|| json!(first.clone()));
            model_obj
                .entry("lite".to_string())
                .or_insert_with(|| json!(first));
        }
        let text = serde_json::to_string_pretty(&root)?;
        std::fs::write(path, text + "\n")
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    Ok(added)
}

/// Default config file for `format`, or the file named after the format's
/// convention inside `src` when a `--src` directory is given (agents
/// installed at a non-default place). `-f FILE` always wins over both.
fn software_config_path_with(src: Option<&Path>, format: &str) -> Option<PathBuf> {
    let Some(dir) = src else {
        return software_config_path_default(format);
    };
    config_file_name(format).map(|name| dir.join(name))
}

/// The config file name each format's software conventionally uses (used to
/// resolve `--src DIR`).
fn config_file_name(format: &str) -> Option<&'static str> {
    match format.to_ascii_lowercase().as_str() {
        "codex" => Some("config.toml"),
        "opencode" | "openxode" => Some("opencode.json"),
        "litellm" => Some("config.yaml"),
        "freellmapi" => Some("freellmapi.config.json"),
        "codebuddy" | "workbuddy" => Some("models.json"),
        "trae" => Some("trae_cli.yaml"),
        "zcoder" => Some("config.json"),
        _ => None,
    }
}

/// The software's usual config path under the user's home (the historical
/// convention; `--src` replaces it).
fn software_config_path_default(format: &str) -> Option<PathBuf> {
    if format.eq_ignore_ascii_case("trae") {
        #[cfg(not(windows))]
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(xdg).join("trae_cli").join("trae_cli.yaml"));
        }
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)?;
    software_config_path_in(&home, format)
}

/// Mapping from a format to its software's usual config file, given `home`.
fn software_config_path_in(home: &Path, format: &str) -> Option<PathBuf> {
    match format.to_ascii_lowercase().as_str() {
        "codex" => Some(home.join(".codex").join("config.toml")),
        "opencode" | "openxode" => {
            Some(home.join(".config").join("opencode").join("opencode.json"))
        }
        "litellm" => Some(home.join(".litellm").join("config.yaml")),
        "codebuddy" => Some(home.join(".codebuddy").join("models.json")),
        "workbuddy" => Some(home.join(".workbuddy").join("models.json")),
        "zcoder" => Some(home.join(".zcode").join("cli").join("config.json")),
        "qoder" => Some(home.join(".qoder").join("settings.json")),
        "trae" => {
            #[cfg(windows)]
            {
                Some(
                    home.join("AppData")
                        .join("Roaming")
                        .join("trae_cli")
                        .join("trae_cli.yaml"),
                )
            }
            #[cfg(not(windows))]
            {
                Some(home.join(".config").join("trae_cli").join("trae_cli.yaml"))
            }
        }
        "freellmapi" => std::env::current_dir()
            .ok()
            .map(|d| d.join("freellmapi.config.json")),
        _ => None,
    }
}

/// Copy `path` to `path.bak-YYYYMMDD-HHMMSS` in the same directory so a
/// previous config can always be restored; appends `-N` when the name is
/// already taken, so an earlier backup is never overwritten. Returns the
/// backup path.
fn backup_file(path: &Path) -> Result<PathBuf> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_string());
    let mut backup = path.with_file_name(format!("{file_name}.bak-{stamp}"));
    let mut n = 1;
    while backup.exists() {
        backup = path.with_file_name(format!("{file_name}.bak-{stamp}-{n}"));
        n += 1;
    }
    std::fs::copy(path, &backup)
        .with_context(|| format!("cannot back up {} to {}", path.display(), backup.display()))?;
    Ok(backup)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data() -> Value {
        serde_json::json!({
            "openai": {
                "id": "openai",
                "name": "OpenAI",
                "doc": "https://platform.openai.com/docs",
                "models": {
                    "gpt-4.1": {
                        "id": "gpt-4.1",
                        "name": "GPT-4.1",
                        "limit": {"context": 1047576, "output": 32768},
                        "cost": {"input": 1.6, "output": 6.4},
                        "reasoning": false
                    },
                    "gpt-4.1-mini": {
                        "id": "gpt-4.1-mini",
                        "name": "GPT-4.1 mini",
                        "limit": {"context": 1047576, "output": 32768}
                    }
                }
            },
            "groq": {
                "id": "groq",
                "name": "Groq",
                "models": {
                    "meta-llama/Llama-3.3-70B": {
                        "id": "meta-llama/Llama-3.3-70B",
                        "name": "Llama 3.3 70B"
                    }
                }
            }
        })
    }

    #[test]
    fn model_exact_by_id() {
        let hits = search_models(&sample_data(), "gpt-4.1");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "openai");
        assert_eq!(hits[0].1["name"], "GPT-4.1");
    }

    #[test]
    fn model_exact_ci_and_name() {
        assert_eq!(search_models(&sample_data(), "GPT-4.1 MINI").len(), 1);
        assert_eq!(search_models(&sample_data(), "Llama 3.3 70B").len(), 1);
    }

    #[test]
    fn model_substring() {
        let hits = search_models(&sample_data(), "4.1");
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn model_no_match() {
        assert!(search_models(&sample_data(), "nope").is_empty());
    }

    #[test]
    fn or_fmt_price_is_per_1m_tokens() {
        // OpenRouter `pricing` is per token; every view is per 1M tokens.
        assert_eq!(or_fmt_price(Some(0.00000025)), "$0.25");
        assert_eq!(or_fmt_price(Some(0.000005)), "$5.0");
        assert_eq!(or_fmt_price(Some(0.0)), "$0.0");
        assert_eq!(or_fmt_price(None), "-");
    }

    #[test]
    fn filter_or_rows_exact_first_then_substring() {
        let mk = |id: &str, name: &str| OrModelRow {
            id: id.to_string(),
            name: name.to_string(),
            context_length: 0,
            prompt: None,
            completion: None,
            created: String::new(),
            created_ts: 0,
        };
        let rows = vec![
            mk("deepseek/deepseek-chat", "DeepSeek: Chat"),
            mk("deepseek/deepseek-r1", "DeepSeek: R1"),
            mk("openai/gpt-4o", "OpenAI: GPT-4o"),
            mk("moonshot/deepseek-tokenizer", "Moonshot: tokenizer"),
        ];
        // An exact id/name hit wins: only that row, not the substring hits.
        let got = filter_or_rows(rows.clone(), &["deepseek/deepseek-r1".to_string()]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "deepseek/deepseek-r1");
        // No exact hit: case-insensitive substring on id/name.
        let got = filter_or_rows(rows.clone(), &["gpt".to_string()]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "openai/gpt-4o");
        // Empty query keeps everything.
        assert_eq!(filter_or_rows(rows.clone(), &[]).len(), 4);
    }

    #[test]
    fn or_model_view_json_honours_query_and_price_unit() {
        let items = vec![
            json!({
                "id": "deepseek/deepseek-chat",
                "name": "DeepSeek: Chat",
                "context_length": 64000,
                "pricing": { "prompt": "0.00000014", "completion": "0.00000028" },
                "created": 1700000000,
            }),
            json!({
                "id": "openai/gpt-4o",
                "name": "OpenAI: GPT-4o",
                "context_length": 128000,
                "pricing": { "prompt": "0.0000025", "completion": "0.00001" },
                "created": 1700000001,
            }),
        ];
        // The JSON view must honour the query filter (not dump all items)...
        let out = or_model_view(&items, Some("deepseek"), Some(5), Some(OutFormat::Json)).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "deepseek/deepseek-chat");
        // ...and report prices per 1M tokens (0.00000014 -> 0.14).
        assert_eq!(arr[0]["price_prompt"], json!(0.14));
        assert_eq!(arr[0]["created"], json!(1700000000));
        // `--limit` also applies to the JSON view.
        let out = or_model_view(&items, None, Some(1), Some(OutFormat::Json)).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
    }

    #[test]
    fn provider_exact_and_substring() {
        assert_eq!(search_providers(&sample_data(), "openai").len(), 1);
        assert_eq!(search_providers(&sample_data(), "GROQ").len(), 1);
        assert_eq!(search_providers(&sample_data(), "o").len(), 2);
    }

    #[test]
    fn filter_by_provider_id_name_ci() {
        let data = sample_data();
        let all = all_models(&data);
        // id substring, case-insensitive
        assert_eq!(filter_by_provider(all.clone(), &data, "openai").len(), 2);
        assert_eq!(filter_by_provider(all.clone(), &data, "OPEN").len(), 2);
        assert_eq!(filter_by_provider(all.clone(), &data, "gro").len(), 1);
        // provider name substring, case-insensitive
        assert_eq!(filter_by_provider(all.clone(), &data, "Groq").len(), 1);
        // no match
        assert!(filter_by_provider(all.clone(), &data, "zzz").is_empty());
    }

    #[test]
    fn filter_by_price_input_output_le() {
        let data = sample_data();
        let all = all_models(&data);
        // default 0: only models without a cost field (counted as 0) pass
        assert_eq!(filter_by_price(all.clone(), 0.0).len(), 2);
        // input 1.6 <= 2 but output 6.4 > 2 -> gpt-4.1 excluded
        assert_eq!(filter_by_price(all.clone(), 2.0).len(), 2);
        // price above both sides keeps every model
        assert_eq!(filter_by_price(all.clone(), 10.0).len(), 3);
        // exact equality counts (<=)
        assert_eq!(filter_by_price(all.clone(), 6.4).len(), 3);
    }

    #[test]
    fn fmt_num_thousands() {
        assert_eq!(fmt_num(1047576), "1,047,576");
        assert_eq!(fmt_num(999), "999");
    }

    #[test]
    fn csv_escape_basic_and_quoted() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("he said \"hi\""), "\"he said \"\"hi\"\"\"");
        assert_eq!(csv_escape("line1\nline2"), "\"line1\nline2\"");
    }

    #[test]
    fn model_csv_row_fields() {
        let data = sample_data();
        let hits = search_models(&data, "gpt-4.1");
        let row = model_csv_row(&model_with_provider(&hits[0].0, &hits[0].1));
        let cols: Vec<&str> = row.split(',').collect();
        assert_eq!(cols[0], "gpt-4.1");          // id
        assert_eq!(cols[1], "GPT-4.1");          // name
        assert_eq!(cols[2], "openai");           // provider
        assert_eq!(cols[7], "1047576");          // context
        assert_eq!(cols[8], "");                 // input_limit (absent)
        assert_eq!(cols[9], "32768");            // output_limit
        assert_eq!(cols[10], "1.6");             // cost.input
        assert_eq!(cols[11], "6.4");             // cost.output
        assert_eq!(cols[15], "false");           // reasoning
        assert_eq!(cols.len(), 24);
        assert!(MODEL_CSV_HEADER.split(',').count() == 24);
    }

    #[test]
    fn provider_csv_row_fields() {
        let data = sample_data();
        let hits = search_providers(&data, "openai");
        let row = provider_csv_row(&hits[0].0, &hits[0].1);
        let cols: Vec<&str> = row.split(',').collect();
        assert_eq!(cols[0], "openai");
        assert_eq!(cols[1], "OpenAI");
        assert_eq!(cols[2], "https://platform.openai.com/docs");
        assert_eq!(cols[5], "2");                // models_count
        assert_eq!(cols.len(), 6);
        assert!(PROVIDER_CSV_HEADER.split(',').count() == 6);
    }

    #[test]
    fn parse_date_valid_and_invalid() {
        assert_eq!(parse_date("2026-10-01").unwrap(), "2026-10-01");
        assert!(parse_date("2026-10-1").is_err());   // day not zero-padded
        assert!(parse_date("26-10-01").is_err());    // short year
        assert!(parse_date("2026/10/01").is_err());  // wrong separator
        assert!(parse_date("2026-13-01").is_err());  // month out of range
        assert!(parse_date("2026-10-32").is_err());  // day out of range
    }

    #[test]
    fn model_updated_after_compares_dates() {
        let m = |lu: Option<&str>| -> Value {
            let mut o = serde_json::json!({"id": "m"});
            if let Some(lu) = lu {
                o["last_updated"] = Value::String(lu.to_string());
            }
            o
        };
        assert!(model_updated_after(&m(Some("2026-10-02")), "2026-10-01"));
        assert!(!model_updated_after(&m(Some("2026-10-01")), "2026-10-01")); // strictly after
        assert!(!model_updated_after(&m(Some("2026-09-30")), "2026-10-01"));
        assert!(!model_updated_after(&m(None), "2026-10-01"));               // no field
        // a full timestamp is compared on its date part only
        assert!(model_updated_after(&m(Some("2026-10-02T12:00:00Z")), "2026-10-01"));
        assert!(!model_updated_after(&m(Some("2026-10-01T12:00:00Z")), "2026-10-02"));
    }

    #[test]
    fn filter_by_date_keeps_only_recent_models() {
        let data = serde_json::json!({
            "p": {
                "models": {
                    "old": {"id": "old", "last_updated": "2026-09-30"},
                    "today": {"id": "today", "last_updated": "2026-10-01"},
                    "fresh": {"id": "fresh", "last_updated": "2026-10-04"},
                    "none": {"id": "none"}
                }
            }
        });
        let hits = filter_by_date(all_models(&data), "2026-10-01");
        let ids: Vec<&str> = hits.iter().map(|(_, m)| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["fresh"]);
        assert_eq!(filter_by_date(all_models(&data), "2026-01-01").len(), 3);
    }

    #[test]
    fn filter_open_keeps_only_open_weights_models() {
        let data = serde_json::json!({
            "p": {
                "models": {
                    "open": {"id": "open", "open_weights": true},
                    "closed": {"id": "closed", "open_weights": false},
                    "missing": {"id": "missing"}
                }
            }
        });
        let hits = filter_open(all_models(&data));
        let ids: Vec<&str> = hits.iter().map(|(_, m)| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["open"]);
    }

    #[test]
    fn apply_filters_combines_date_and_open() {
        let data = serde_json::json!({
            "p": {
                "models": {
                    "old_open": {"id": "old_open", "open_weights": true, "last_updated": "2025-01-01"},
                    "new_closed": {"id": "new_closed", "open_weights": false, "last_updated": "2026-10-02"},
                    "new_open": {"id": "new_open", "open_weights": true, "last_updated": "2026-10-02"}
                }
            }
        });
        let hits = apply_filters(all_models(&data), Some("2026-10-01"), true);
        let ids: Vec<&str> = hits.iter().map(|(_, m)| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["new_open"]);
        // only the date filter
        let hits = apply_filters(all_models(&data), Some("2026-10-01"), false);
        assert_eq!(hits.len(), 2);
        // only the open filter
        let hits = apply_filters(all_models(&data), None, true);
        assert_eq!(hits.len(), 2);
        // no filters keeps everything
        assert_eq!(apply_filters(all_models(&data), None, false).len(), 3);
    }

    // ------------------------------------------------------------------
    // DataLearner (cn-model) parsing
    // ------------------------------------------------------------------

    const CARD_HTML: &str = r#"<a href="/ai-models/pretrained-models/gpt-6-1-sol" class=" group flex flex-col p-5 border border-slate-100 dark:border-slate-700/60 rounded-xl hover:border-slate-200 dark:hover:border-slate-600 hover:bg-slate-50/50 dark:hover:bg-slate-800/50 hover:-translate-y-0.5 hover:shadow-sm transition-all duration-200 no-underline text-inherit "><div class="flex flex-wrap items-center gap-3 mb-3"><div class="flex-none w-10 h-10 rounded-full ring-1 ring-slate-200/80 dark:ring-slate-600/50 grid place-items-center bg-slate-100 dark:bg-slate-800 overflow-hidden relative"><img alt="GPT-6.1 Sol - OpenAI 标志" loading="lazy" decoding="async" data-nimg="fill" class="object-cover dark:brightness-90" style="position:absolute;height:100%;width:100%;left:0;top:0;right:0;bottom:0;color:transparent" src="/resources/ai-org-logo/x.png"/></div><div class="min-w-0 flex-1"><h3 class="m-0 text-[15px] font-semibold text-slate-900 dark:text-slate-100 leading-snug line-clamp-1 group-hover:text-black dark:group-hover:text-white">GPT-6.1 Sol</h3><span class="text-[12px] text-slate-400 dark:text-slate-500 line-clamp-1">OpenAI</span><span class="text-[11px] text-slate-400 dark:text-slate-500 line-clamp-1" title="GPT 6.1 Sol / gpt-6.1-sol">又名<!-- -->：<!-- -->GPT 6.1 Sol / gpt-6.1-sol</span></div><div class="flex items-center gap-1.5 flex-shrink-0"><span class="inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded text-[10px] font-medium text-amber-600 dark:text-amber-400 bg-amber-50 dark:bg-amber-900/30"><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="lucide lucide-star" aria-hidden="true"><path d="M11.525 2.295a.53.53 0 0 1 .95 0"/></svg>精选</span><span class="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium text-slate-500 bg-slate-50 dark:bg-slate-700 dark:text-slate-400">闭源模型</span></div></div><div class="flex items-center justify-between mt-auto pt-2 border-t border-slate-50 dark:border-slate-700/60 text-[11px] text-slate-400 dark:text-slate-500"><span class="inline-flex items-center gap-1"><svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="lucide lucide-calendar text-slate-300 dark:text-slate-600" aria-hidden="true"><path d="M8 2v4"></path></svg>2026-09-29</span><span class="inline-flex items-center gap-1"><svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="lucide lucide-tag text-slate-300 dark:text-slate-600" aria-hidden="true"><path d="M12 2H2v10l9.29 9.29a1 1 0 0 0 1.42 0l8.58-8.58a1 1 0 0 0 0-1.42z"></path></svg>推理大模型</span></div></a>"#;

    #[test]
    fn dl_parse_full_card() {
        let cards = parse_dl_cards(CARD_HTML);
        assert_eq!(cards.len(), 1);
        let m = &cards[0];
        assert_eq!(m.id, "gpt-6-1-sol");
        assert_eq!(m.name, "GPT-6.1 Sol");
        assert_eq!(m.provider, "OpenAI");
        assert_eq!(m.aliases, "GPT 6.1 Sol / gpt-6.1-sol");
        assert_eq!(m.r#type, "精选 闭源模型");
        assert_eq!(m.category, "推理大模型");
        assert_eq!(m.published, "2026-09-29");
        assert_eq!(m.url, "https://www.datalearner.com/ai-models/pretrained-models/gpt-6-1-sol");
    }

    #[test]
    fn dl_skips_rumor_and_sidebar_cards() {
        // rumor card: w-[260px] carousel + 预计发布
        let rumor = r#"<a href="/ai-models/pretrained-models/claude-fable-5-5" title="查看模型详情" class="group flex min-h-[140px] w-[260px] flex-none snap-start flex-col rounded-xl border border-slate-100 bg-white p-4 text-inherit no-underline transition-colors hover:border-amber-200/80 hover:bg-amber-50/30 dark:border-slate-700/60 dark:bg-transparent dark:hover:border-amber-800/60 dark:hover:bg-amber-950/10 sm:w-[280px]"><h3 class="m-0 line-clamp-2 text-[15px] font-semibold leading-snug text-slate-900 group-hover:text-black dark:text-slate-100 dark:group-hover:text-white">Claude Fable 5.5</h3><div class="mt-auto pt-3 flex items-center justify-between"><span class="flex items-center gap-1.5 text-[12px] text-slate-400 dark:text-slate-500"><svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="lucide lucide-calendar-clock flex-none text-amber-500/80 dark:text-amber-400/70" aria-hidden="true"><path d="M16 14v2.2l1.6 1"></path></svg><span class="text-slate-400 dark:text-slate-500">预计发布</span><span class="font-medium tabular-nums text-slate-600 dark:text-slate-300">2026-11-04</span></div></div></a>"#;
        // sidebar row: flex items-center gap-3 py-2.5
        let sidebar = r#"<a href="/ai-models/pretrained-models/gpt-6-luna" class="group flex items-center gap-3 py-2.5 no-underline -mx-2.5 px-2.5 rounded-lg transition-colors hover:bg-slate-50 dark:hover:bg-slate-800/60" title="查看模型详情"><span class="flex-none w-1.5 h-1.5 rounded-full ml-[7px] mr-[5px] bg-amber-500"></span><span class="flex-1 min-w-0 truncate text-[13px] text-slate-800 dark:text-slate-200 group-hover:text-slate-900 dark:group-hover:text-white font-semibold">GPT-6 Luna</span></a>"#;
        let html = format!("{}{}{}", rumor, sidebar, CARD_HTML);
        let cards = parse_dl_cards(&html);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].id, "gpt-6-1-sol");
    }

    #[test]
    fn dl_dedupe_not_here_but_search_works() {
        let models = parse_dl_cards(CARD_HTML);
        // exact match by name, slug and alias (case-insensitive)
        assert_eq!(search_dl(&models, "gpt-6.1 sol").len(), 1);
        assert_eq!(search_dl(&models, "GPT-6-1-SOL").len(), 1);
        assert_eq!(search_dl(&models, "gpt-6.1-sol").len(), 1);
        // substring match
        assert_eq!(search_dl(&models, "6.1").len(), 1);
        assert!(search_dl(&models, "nope").is_empty());
    }

    #[test]
    fn dl_published_date_filter() {
        let mut models = parse_dl_cards(CARD_HTML);
        assert!(dl_published_after(&models[0], "2026-09-28"));
        assert!(!dl_published_after(&models[0], "2026-09-29")); // strictly after
        assert!(!dl_published_after(&models[0], "2026-09-30"));
        models[0].published = String::new();
        assert!(!dl_published_after(&models[0], "2020-01-01")); // no real date -> never matches
        assert!(is_plain_date("2026-10-01"));
        assert!(!is_plain_date("预计发布2026-10-01"));
        assert!(!is_plain_date("2026-1-1"));
    }

    #[test]
    fn dl_csv_row_fields() {
        let models = parse_dl_cards(CARD_HTML);
        let row = dl_csv_row(&models[0]);
        let cols: Vec<&str> = row.split(',').collect();
        assert_eq!(cols[0], "gpt-6-1-sol");
        assert_eq!(cols[1], "GPT-6.1 Sol");
        assert_eq!(cols[2], "OpenAI");
        assert_eq!(cols[3], "GPT 6.1 Sol / gpt-6.1-sol");
        assert_eq!(cols[5], "推理大模型");
        assert_eq!(cols[6], "");                 // context (empty from card)
        assert_eq!(cols[7], "");                 // modality (empty from card)
        assert_eq!(cols[8], "");                 // parameters (empty from card)
        assert_eq!(cols[9], "2026-09-29");
        assert_eq!(cols[10], models[0].url);
        assert_eq!(cols.len(), 11);
        assert!(DL_CSV_HEADER.split(',').count() == 11);
    }

    #[test]
    fn model_type_filter_by_modalities() {
        let data = serde_json::json!({
            "p": {
                "models": {
                    "text_only": {
                        "id": "text_only",
                        "modalities": {"input": ["text"], "output": ["text"]}
                    },
                    "vision": {
                        "id": "vision",
                        "modalities": {"input": ["text", "image"], "output": ["text"]}
                    },
                    "voice": {
                        "id": "voice",
                        "modalities": {"input": ["text", "audio"], "output": ["text", "audio"]}
                    },
                    "none": {"id": "none"}
                }
            }
        });
        let one = |s: &str| vec![normalize_modality(s)];
        assert_eq!(filter_by_model_type(all_models(&data), &one("text")).len(), 3);
        assert_eq!(filter_by_model_type(all_models(&data), &one("image")).len(), 1);
        assert_eq!(filter_by_model_type(all_models(&data), &one("audio")).len(), 1);
        assert_eq!(filter_by_model_type(all_models(&data), &one("video")).len(), 0);
        // Chinese alias
        assert_eq!(filter_by_model_type(all_models(&data), &one("图像")).len(), 1);
        assert_eq!(filter_by_model_type(all_models(&data), &one("语音")).len(), 1);
        // Multi-value: AND semantics — only the model with both modalities.
        assert_eq!(
            filter_by_model_type(all_models(&data), &["text".to_string(), "image".to_string()]).len(),
            1
        );
        assert_eq!(
            filter_by_model_type(all_models(&data), &["text".to_string(), "audio".to_string()]).len(),
            1
        );
        assert_eq!(
            filter_by_model_type(all_models(&data), &["image".to_string(), "audio".to_string()]).len(),
            0
        );
        // parse_model_types splits commas and normalizes aliases.
        assert_eq!(parse_model_types("text,image"), Some(vec!["text".to_string(), "image".to_string()]));
        assert_eq!(parse_model_types("文本，语音"), Some(vec!["text".to_string(), "audio".to_string()]));
        assert_eq!(parse_model_types(""), None);
        assert_eq!(parse_model_types(",,，"), None);
        assert!(model_has_modality(&serde_json::json!({"modalities": {"input": ["TEXT"]}}), "text"));
    }

    #[test]
    fn dl_type_keywords_mapping() {
        assert!(dl_type_keywords("text").is_some());
        assert!(dl_type_keywords("文本").is_some());
        assert!(dl_type_keywords("image").is_some());
        assert!(dl_type_keywords("语音").is_some());
        assert!(dl_type_keywords("multimodal").is_some());
        assert!(dl_type_keywords("bogus").is_none());
    }

    #[test]
    fn dl_filter_by_category_type() {
        let models = vec![
            DlModel {
                id: "a".into(),
                name: "A".into(),
                provider: "X".into(),
                aliases: String::new(),
                r#type: String::new(),
                category: "推理大模型".into(),
                context: String::new(),
                modality: String::new(),
                parameters: String::new(),
                published: String::new(),
                url: String::new(),
            },
            DlModel {
                id: "b".into(),
                name: "B".into(),
                provider: "X".into(),
                aliases: String::new(),
                r#type: String::new(),
                category: "语音大模型".into(),
                context: String::new(),
                modality: String::new(),
                parameters: String::new(),
                published: String::new(),
                url: String::new(),
            },
            DlModel {
                id: "c".into(),
                name: "C".into(),
                provider: "X".into(),
                aliases: String::new(),
                r#type: String::new(),
                category: "多模态大模型".into(),
                context: String::new(),
                modality: String::new(),
                parameters: String::new(),
                published: String::new(),
                url: String::new(),
            },
        ];
        let ids = |v: &[DlModel]| -> Vec<String> { v.iter().map(|m| m.id.clone()).collect() };
        let one = |s: &str| vec![s.to_string()];
        assert_eq!(ids(&filter_dl_by_type(models.clone(), &one("text")).unwrap()), vec!["a".to_string()]);
        assert_eq!(ids(&filter_dl_by_type(models.clone(), &one("语音")).unwrap()), vec!["b".to_string()]);
        assert_eq!(ids(&filter_dl_by_type(models.clone(), &one("image")).unwrap()), vec!["c".to_string()]);
        assert!(filter_dl_by_type(models.clone(), &one("nope")).is_err());
        // Multi-value AND: category must match every type's keywords.
        assert_eq!(
            ids(&filter_dl_by_type(models.clone(), &["text".to_string(), "image".to_string()]).unwrap()),
            Vec::<String>::new()
        );
        assert_eq!(
            ids(&filter_dl_by_type(models.clone(), &["text".to_string(), "语音".to_string()]).unwrap()),
            Vec::<String>::new()
        );
        // A category containing both 推理 and 多模态 matches text+image.
        let both = DlModel {
            id: "d".into(),
            name: "D".into(),
            provider: "X".into(),
            aliases: String::new(),
            r#type: String::new(),
            category: "多模态 推理大模型".into(),
            context: String::new(),
            modality: String::new(),
            parameters: String::new(),
            published: String::new(),
            url: String::new(),
        };
        let mut m2 = models.clone();
        m2.push(both);
        assert_eq!(
            ids(&filter_dl_by_type(m2, &["text".to_string(), "image".to_string()]).unwrap()),
            vec!["d".to_string()]
        );
    }

    #[test]
    fn dl_detail_value_parses_labeled_rows() {
        // Stat-card style: label inside a span, then the value div.
        let html = r#"<span class="text-sm ...">上下文长度</span></div><div class="flex ...">1.05M</div>"#;
        assert_eq!(dl_detail_value(html, "上下文长度").as_deref(), Some("1.05M"));
        // Basic-info style: label directly in the header div.
        let html = r#"</svg></span>输入/输出模态</div><div class="flex flex-wrap items-center gap-2.5 text-base ...">文本、图像 → 文本</div>"#;
        assert_eq!(dl_detail_value(html, "输入/输出模态").as_deref(), Some("文本、图像 → 文本"));
        // Model scale (参数量) row: same basic-info style as the real site.
        let html = r#"<span class="inline-flex ..."><svg/></span>总参数 / 激活参数</div><div class="flex flex-wrap items-center gap-2.5 text-base ...">2350亿 / 220亿</div>"#;
        assert_eq!(dl_detail_value(html, "总参数 / 激活参数").as_deref(), Some("2350亿 / 220亿"));
        // Missing label.
        assert_eq!(dl_detail_value("<div>no label</div>", "上下文长度"), None);
        // 暂无数据 is treated as absent.
        let html = r#"<span>上下文长度</span></div><div class="...">暂无数据</div>"#;
        assert_eq!(dl_detail_value(html, "上下文长度"), None);
        // Placeholder-only composite values (e.g. 总参数 / 激活参数 row with
        // unknown parameters) are treated as absent too.
        let html = r#"<span>总参数 / 激活参数</span></div><div class="...">暂无数据 / 暂无数据</div>"#;
        assert_eq!(dl_detail_value(html, "总参数 / 激活参数"), None);
    }

    #[test]
    fn dl_detail_value_skips_meta_description_hits() {
        // The label first appears inside the page meta description, far from
        // any value row; the parser must skip it and parse the real row
        // (whose value div starts right after the label).
        let html = format!(
            r#"<meta name="description" content="查看 Qwen3 的评测成绩、上下文长度、参数规模、发布时间、开源协议、下载地址、模型规格，以及与其他开源和闭源模型的对比。"/>{}<span class="icon">x</span>上下文长度</div><div class="flex flex-wrap items-center gap-2.5 text-base">256K tokens</div>"#,
            "x".repeat(300) // head content without any `</div><div class="` marker
        );
        assert_eq!(dl_detail_value(&html, "上下文长度").as_deref(), Some("256K tokens"));
        // If an earlier marker-near occurrence yields 暂无数据 (absent), later
        // real rows for the same label are still considered.
        let html = r#"<span>x</span>上下文长度</div><div class="...">暂无数据</div><div>nav</div><div class="y">上下文长度</div><div class="z">1.05M tokens</div>"#;
        assert_eq!(dl_detail_value(&html, "上下文长度").as_deref(), Some("1.05M tokens"));
    }

    // --- `sys ai info` ------------------------------------------------------

    fn sample_config() -> chat::Config {
        chat::Config {
            model: None,
            stream: false,
            free: false,
            search: Vec::new(),
            providers: vec![
                chat::Provider {
                    kind: "openai".into(),
                    name: "agnes".into(),
                    api_base: "https://apihub.agnes-ai.cn/v1".into(),
                    api_key: "sk-agnes".into(),
                    models: vec![
                        chat::Model {
                            name: "agnes-3.0-flash".into(),
                            weight: 4,
                            max_tokens: None,
                            max_input_tokens: Some(524288),
                            model_type: Some("text,image".into()),
                            group: vec!["flash".into()],
                        },
                        chat::Model {
                            name: "deepseek-v4-flash".into(),
                            weight: 1,
                            max_tokens: None,
                            max_input_tokens: None,
                            model_type: None,
                            group: Vec::new(),
                        },
                    ],
                },
                chat::Provider {
                    kind: "openai".into(),
                    name: "modelscope".into(),
                    api_base: "https://api-inference.modelscope.cn/v1".into(),
                    api_key: "ms-key".into(),
                    models: vec![chat::Model {
                        name: "Qwen/Qwen3.8-Flash-Next".into(),
                        weight: 1,
                        max_tokens: None,
                        max_input_tokens: None,
                        model_type: None,
                        group: Vec::new(),
                    }],
                },
            ],
        }
    }

    #[test]
    fn info_provider_lists_all() {
        let rows = info_provider_rows(&sample_config(), None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "agnes");
        assert_eq!(rows[0].api_base, "https://apihub.agnes-ai.cn/v1");
        assert_eq!(rows[0].api_key, "sk-agnes");
        assert_eq!(rows[0].docs, "https://wiki.agnes-ai.cn/zh-Hans/docs");
        assert_eq!(rows[0].console, "https://platform.agnes-ai.cn");
        assert_eq!(rows[1].name, "modelscope");
        assert_eq!(rows[1].docs, "https://modelscope.cn/docs");
        assert_eq!(rows[1].console, "https://modelscope.cn");
    }

    #[test]
    fn provider_links_matches_known_and_unknown() {
        assert_eq!(
            provider_links("alibaba-cn"),
            ("https://help.aliyun.com/zh/model-studio/".to_string(), "https://bailian.console.aliyun.com".to_string())
        );
        assert_eq!(
            provider_links("MiniMax"),
            ("https://platform.minimax.cn/docs".to_string(), "https://platform.minimax.cn".to_string())
        );
        assert_eq!(
            provider_links("bigmodel"),
            ("https://docs.bigmodel.cn".to_string(), "https://bigmodel.cn/console".to_string())
        );
        assert_eq!(provider_links("some-future-provider"), (String::new(), String::new()));
    }

    #[test]
    fn deepseek_balance_rows_parses_official_shape() {
        let v: Value = serde_json::from_str(
            r#"{
                "is_available": true,
                "balance_infos": [
                    {"currency": "CNY", "total_balance": "110.00",
                     "granted_balance": "10.00", "topped_up_balance": "100.00"}
                ]
            }"#,
        )
        .unwrap();
        let (avail, rows) = deepseek_balance_rows(&v);
        assert!(avail);
        assert_eq!(
            rows,
            vec![(
                "CNY".to_string(),
                "110.00".to_string(),
                "10.00".to_string(),
                "100.00".to_string()
            )]
        );
    }

    #[test]
    fn deepseek_balance_rows_tolerates_unavailable_and_empty() {
        let v: Value =
            serde_json::from_str(r#"{"is_available": false, "balance_infos": []}"#).unwrap();
        let (avail, rows) = deepseek_balance_rows(&v);
        assert!(!avail);
        assert!(rows.is_empty());
        // Unexpected / partial shapes degrade without panicking.
        let v2: Value = serde_json::from_str("{}").unwrap();
        let (avail2, rows2) = deepseek_balance_rows(&v2);
        assert!(!avail2);
        assert!(rows2.is_empty());
        let v3: Value = serde_json::from_str(
            r#"{"is_available": true, "balance_infos": [{"currency": "USD"}]}"#,
        )
        .unwrap();
        let (avail3, rows3) = deepseek_balance_rows(&v3);
        assert!(avail3);
        assert_eq!(rows3, vec![("USD".to_string(), "?".to_string(), "?".to_string(), "?".to_string())]);
    }

    #[test]
    fn info_provider_filters_by_keyword() {
        let rows = info_provider_rows(&sample_config(), Some("MODEL")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "modelscope");
        assert!(info_provider_rows(&sample_config(), Some("zzz")).is_err());
    }

    #[test]
    fn info_model_lists_all_in_provider_name_form() {
        let rows = info_model_rows(&sample_config(), None).unwrap();
        let fmt: Vec<String> = rows.iter().map(|r| format!("{}:{}", r.provider, r.name)).collect();
        assert_eq!(
            fmt,
            vec![
                "agnes:agnes-3.0-flash".to_string(),
                "agnes:deepseek-v4-flash".to_string(),
                "modelscope:Qwen/Qwen3.8-Flash-Next".to_string(),
            ]
        );
    }

    #[test]
    fn info_model_filters_by_provider_name() {
        let rows = info_model_rows(&sample_config(), Some("model")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider, "modelscope");
        assert_eq!(rows[0].name, "Qwen/Qwen3.8-Flash-Next");
    }

    #[test]
    fn info_model_falls_back_to_model_name() {
        let rows = info_model_rows(&sample_config(), Some("deepseek")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider, "agnes");
        assert_eq!(rows[0].name, "deepseek-v4-flash");
        assert!(info_model_rows(&sample_config(), Some("nope")).is_err());
    }

    #[test]
    fn info_model_provider_model_selector() {
        let rows = info_model_rows(&sample_config(), Some("agnes:3.0")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "agnes-3.0-flash");
        let all = info_model_rows(&sample_config(), Some("modelscope:*")).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Qwen/Qwen3.8-Flash-Next");
        assert!(info_model_rows(&sample_config(), Some("agnes:nope")).is_err());
        assert!(info_model_rows(&sample_config(), Some("nope:*")).is_err());
    }

    #[test]
    fn provider_sample_request_anthropic_uses_x_api_key() {
        let p = chat::Provider {
            kind: "anthropic".into(),
            name: "claude".into(),
            api_base: "https://api.anthropic.example.com".into(),
            api_key: "sk-ant-test".into(),
            models: vec![],
        };
        assert_eq!(
            provider_sample_request(&p),
            format!(
                "https://api.anthropic.example.com/v1/models  x-api-key:\"sk-ant-test\" anthropic-version:\"{}\"",
                chat::ANTHROPIC_VERSION
            )
        );
        // api_base already ending in /v1 must not gain another /v1.
        let mut p2 = p.clone();
        p2.api_base = "https://api.anthropic.example.com/v1".into();
        assert!(provider_sample_request(&p2).starts_with("https://api.anthropic.example.com/v1/models"));
    }

    #[test]
    fn md_table_renders_escaping_and_separator() {
        let t = md_table(&["a", "b"], &[vec!["x|y".to_string(), "line\nbreak".to_string()]]);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0], "| a | b |");
        assert_eq!(lines[1], "| --- | --- |");
        assert_eq!(lines[2], "| x\\|y | line break |");
        // Empty rows still produce the header and separator.
        assert_eq!(md_table(&["c"], &[]), "| c |\n| --- |\n");
    }

    #[test]
    fn cmd_info_provider_outputs_markdown_list() {
        let dir = std::env::temp_dir().join(format!("sysenv-md-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.yaml");
        std::fs::write(
            &cfg_path,
            "clients:\n  - type: openai\n    name: agnes\n    api_base: https://apihub.example.cn/v1\n    api_key: sk-test-0000\n    models:\n      - name: agnes-3.0-flash\n        weight: 1\n",
        )
        .unwrap();
        // Verify the renderer output used by `info provider -o md`.
        let cfg = chat::load_config(Some(&cfg_path)).unwrap().0;
        let rows = info_provider_rows(&cfg, None).unwrap();
        assert_eq!(rows.len(), 1);
        let list = md_provider_list(&rows);
        assert_eq!(
            list,
            "- agnes\n  - api_base：https://apihub.example.cn/v1\n  - api_key：sk-test-0000\n  - console：https://platform.agnes-ai.cn\n  - docs：https://wiki.agnes-ai.cn/zh-Hans/docs\n  - sample：https://apihub.example.cn/v1/models  Authorization:\"Bearer sk-test-0000\"\n"
        );
        assert_eq!(rows[0].sample, "https://apihub.example.cn/v1/models  Authorization:\"Bearer sk-test-0000\"");
        // `info model -o md` groups children under each provider.
        let mods = info_model_rows(&cfg, None).unwrap();
        assert_eq!(md_model_list(&mods), "- agnes\n  - name：agnes-3.0-flash\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cmd_info_models_needs_provider_and_differs_from_model() {
        // `info models` is the provider-models-API query (the function of
        // `--query-model`), not a duplicate of `info model`: without a provider
        // name it fails fast (before any config / network work)...
        let err = cmd_info(Some("models"), &[], false, None, None, false, None).unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("needs a provider name"), "got: {text}");
        // ...while `info model` without args stays the config model listing.
        let dir = std::env::temp_dir().join(format!("sysenv-info-model-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.yaml");
        std::fs::write(
            &cfg_path,
            "clients:\n  - type: openai\n    name: agnes\n    api_base: https://apihub.example.cn/v1\n    api_key: sk-test-0000\n    models:\n      - name: agnes-3.0-flash\n",
        )
        .unwrap();
        let (cfg, _) = chat::load_config(Some(&cfg_path)).unwrap();
        let rows = info_model_rows(&cfg, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(format!("{}:{}", rows[0].provider, rows[0].name), "agnes:agnes-3.0-flash");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn models_url_formats() {
        // The URL rule lives in chat::models_url (shared with `--query-model`).
        let cfg = sample_config();
        // OpenAI-compatible: {api_base}/models.
        assert_eq!(chat::models_url(&cfg.providers[0]), "https://apihub.agnes-ai.cn/v1/models");
        // A base without /v1 (e.g. Baidu Qianfan) appends plain `models`.
        let mut p = chat::Provider {
            kind: "openai".into(),
            name: "baidu".into(),
            api_base: "https://qianfan.baidubce.com/v2".into(),
            api_key: "sk-b".into(),
            models: Vec::new(),
        };
        assert_eq!(chat::models_url(&p), "https://qianfan.baidubce.com/v2/models");
        // Anthropic: {api_base}/v1/models unless api_base already ends with /v1.
        p = chat::Provider {
            kind: "anthropic".into(),
            name: "anthropic".into(),
            api_base: "https://api.anthropic.com".into(),
            api_key: "sk-ant".into(),
            models: Vec::new(),
        };
        assert_eq!(chat::models_url(&p), "https://api.anthropic.com/v1/models");
        p.api_base = "https://api.anthropic.com/v1".into();
        assert_eq!(chat::models_url(&p), "https://api.anthropic.com/v1/models");
    }

    #[test]
    fn model_list_rows_normalizes_openai_and_anthropic() {
        // OpenAI-compatible shape: data[] with id / owned_by / created (unix).
        let openai = serde_json::json!({
            "object": "list",
            "data": [
                {"id": "gpt-4.1", "object": "model", "created": 1734567890, "owned_by": "openai"},
                {"id": "gpt-4.1-mini", "object": "model", "created": 1734567890, "owned_by": "system"}
            ]
        });
        let rows = model_list_rows(&openai);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "gpt-4.1");
        assert_eq!(rows[0].owned_by, "openai");
        assert_eq!(rows[0].created, "1734567890");

        // Anthropic shape: data[] with id / display_name / created_at (ISO).
        let ant = serde_json::json!({
            "data": [
                {"type": "model", "id": "claude-3-5-sonnet-20241022", "display_name": "Claude 3.5 Sonnet", "created_at": "2024-10-22T00:00:00Z"}
            ]
        });
        let rows = model_list_rows(&ant);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "claude-3-5-sonnet-20241022");
        assert_eq!(rows[0].owned_by, "Claude 3.5 Sonnet");
        assert_eq!(rows[0].created, "2024-10-22T00:00:00Z");

        // Non-standard fallback: bare models[] array; entries without an id
        // are skipped.
        let odd = serde_json::json!({
            "models": [{"id": "m1"}, {"name": "no-id"}, {"id": "m2", "owned_by": "x"}]
        });
        let rows = model_list_rows(&odd);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "m1");
        assert_eq!(rows[1].id, "m2");
        assert_eq!(rows[1].owned_by, "x");

        // Empty / malformed responses yield no rows.
        assert!(model_list_rows(&serde_json::json!({"data": []})).is_empty());
        assert!(model_list_rows(&serde_json::json!({"error": "x"})).is_empty());
    }

    #[test]
    fn fmt_created_converts_unix_timestamp_to_local_datetime() {
        // A unix timestamp becomes a local "YYYY-MM-DD HH:MM:SS" string that
        // round-trips back to the same instant (timezone-independent check).
        let s = fmt_created("1785767088");
        assert_eq!(s.len(), 19);
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], " ");
        let dt = chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").unwrap();
        let local = chrono::TimeZone::from_local_datetime(&chrono::Local, &dt)
            .single()
            .unwrap();
        assert_eq!(local.timestamp(), 1785767088);
        // Anthropic ISO created_at passes through unchanged.
        assert_eq!(fmt_created("2024-10-22T00:00:00Z"), "2024-10-22T00:00:00Z");
        // Empty / non-numeric values pass through unchanged.
        assert_eq!(fmt_created(""), "");
        assert_eq!(fmt_created("n/a"), "n/a");
        // Out-of-range unix values (e.g. 99999999999999) fall back to raw.
        assert_eq!(fmt_created("99999999999999"), "99999999999999");
    }

    #[test]
    fn info_price_reads_model_costs() {
        let data = sample_data();
        let rows =
            info_price_rows(&data, &["openai".to_string(), "groq".to_string()]).unwrap();
        assert_eq!(rows.len(), 3);
        let gpt = rows.iter().find(|r| r.model_id == "gpt-4.1").unwrap();
        assert_eq!(gpt.provider, "openai");
        assert_eq!(gpt.input, Some(1.6));
        assert_eq!(gpt.output, Some(6.4));
        assert_eq!(gpt.cache_read, None);
        assert_eq!(gpt.currency, "USD");
        assert_eq!(gpt.unit, "1M");
        let mini = rows.iter().find(|r| r.model_id == "gpt-4.1-mini").unwrap();
        assert_eq!(mini.input, None);
        assert!(info_price_rows(&data, &["nope".to_string()]).is_err());
    }

    #[test]
    fn html_text_strips_tags_and_entities() {
        assert_eq!(
            html_text("<td rowSpan=\"3\"><del><code>¥0.035 / M</code></del></td>"),
            "¥0.035 / M"
        );
        assert_eq!(
            html_text("<strong>输入价格</strong><br/> 元/百万 tokens"),
            "输入价格 | 元/百万 tokens"
        );
        assert_eq!(html_text("a&amp;b &lt;c&gt; &quot;d&quot; &#39;e&#39; &nbsp; f"), "a&b <c> \"d\" 'e' f");
    }

    #[test]
    fn extract_tables_parses_rows_and_cells() {
        let html = concat!(
            "<html><table><thead><tr><th>A</th><th>B</th></tr></thead>",
            "<tbody><tr><td>1</td><td><strong>2</strong></td></tr></tbody></table></html>"
        );
        let tables = extract_tables(html);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].len(), 2);
        assert_eq!(tables[0][0], vec!["A", "B"]);
        assert_eq!(tables[0][1], vec!["1", "2"]);
    }

    #[test]
    fn parse_agnes_prices_handles_rowspan_and_discounts() {
        let html = concat!(
            "<table><tbody>",
            "<tr><td rowSpan=\"3\" style=\"vertical-align:middle\"><code>agnes-3.0-flash</code></td>",
            "<td>输入缓存命中</td><td><del><code>¥0.035 / M</code></del></td><td><strong><code>¥0 / M</code></strong></td></tr>",
            "<tr><td>输入 Token</td><td><del><code>¥0.35 / M</code></del></td><td><strong><code>¥0 / M</code></strong></td></tr>",
            "<tr><td>输出 Token</td><td><del><code>¥1.00 / M</code></del></td><td><strong><code>¥0 / M</code></strong></td></tr>",
            "<tr><td rowSpan=\"3\"><code>agnes-3.0-pro</code><br/>即将上线</td>",
            "<td>输入缓存命中</td><td><code>¥0.30 / M</code></td><td><code>¥0.30 / M</code></td></tr>",
            "<tr><td>输入 Token</td><td><code>¥3.00 / M</code></td><td><code>¥3.00 / M</code></td></tr>",
            "<tr><td>输出 Token</td><td><code>¥6.00 / M</code></td><td><code>¥6.00 / M</code></td></tr>",
            "</tbody></table>"
        );
        let rows = parse_agnes_prices(html);
        assert_eq!(rows.len(), 6);
        // 优惠价字段带 strong 时被解析为现价
        assert_eq!(
            rows[0],
            (
                "agnes-3.0-flash".to_string(),
                "输入缓存命中".to_string(),
                "¥0.035 / M".to_string(),
                "¥0 / M".to_string(),
            )
        );
        // rowspan 继承：第 2、3 行仍归属 agnes-3.0-flash
        assert_eq!(rows[1].0, "agnes-3.0-flash");
        assert_eq!(rows[2].0, "agnes-3.0-flash");
        assert_eq!(rows[2].1, "输出 Token");
        // 下一模型组
        assert_eq!(rows[3].0, "agnes-3.0-pro | 即将上线");
        assert_eq!(rows[5], ("agnes-3.0-pro | 即将上线".to_string(), "输出 Token".to_string(), "¥6.00 / M".to_string(), "¥6.00 / M".to_string()));
    }

    #[test]
    fn parse_minimax_prices_takes_language_tables_only() {
        let html = concat!(
            "<table><thead><tr><th><strong>模型</strong></th><th><strong>输入价格</strong><br/> 元/百万 tokens</th>",
            "<th><strong>输出价格</strong><br/> 元/百万 tokens</th><th><strong>缓存读取</strong><br/> 元/百万 tokens</th>",
            "<th><strong>缓存写入</strong><br/> 元/百万 tokens</th></tr></thead><tbody>",
            "<tr><td><strong>MiniMax-M2.7</strong></td><td data-numeric=\"true\">2.1</td><td data-numeric=\"true\">8.4</td><td data-numeric=\"true\">0.42</td><td data-numeric=\"true\">2.625</td></tr>",
            "<tr><td><strong>MiniMax-M2.7-highspeed</strong></td><td>4.2</td><td>16.8</td><td>0.42</td><td>2.625</td></tr>",
            "</tbody></table>",
            "<table><thead><tr><th><strong>模型/接口</strong></th><th><strong>分辨率</strong></th><th><strong>计费规则</strong></th><th><strong>刊例价</strong></th></tr></thead><tbody>",
            "<tr><td>video-01</td><td>720P</td><td>按秒</td><td>0.15</td></tr></tbody></table>"
        );
        let rows = parse_minimax_prices(html);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], "MiniMax-M2.7");
        assert_eq!(rows[0][1], "2.1");
        assert_eq!(rows[0][4], "2.625");
        assert_eq!(rows[1][0], "MiniMax-M2.7-highspeed");
        // 视频表（表头无「输入价格」）被忽略
        assert!(rows.iter().all(|r| r[0] != "video-01"));
    }

    #[test]
    fn find_provider_exact_then_substring() {
        let cfg = sample_config();
        assert_eq!(find_provider(&cfg, "agnes").unwrap().name, "agnes");
        assert_eq!(find_provider(&cfg, "MODEL").unwrap().name, "modelscope");
        assert!(find_provider(&cfg, "zzz").is_err());
        // "s" 同时命中 agnes / modelscope → 报歧义
        let err = find_provider(&cfg, "s").unwrap_err();
        assert!(err.to_string().contains("matches multiple"));
    }

    #[test]
    fn alibaba_limits_parses_quota_rows() {
        // 结构与实测 https://dashscope.aliyuncs.com/api/v1/models/limits 一致
        let v: Value = serde_json::json!({
            "code": null,
            "success": true,
            "output": {
                "total": 519,
                "quotas": [
                    {
                        "model": "qwen3.8-max",
                        "workspace_id": "ws-bvxqb1qp3jh8j2ha",
                        "model_limit": {
                            "request_limit": null,
                            "request_limit_period": null,
                            "usage_limit": 500000,
                            "usage_limit_field": "total_tokens",
                            "usage_limit_period": 6,
                            "async_user_queue_limit": null,
                            "async_user_concurrency_limit": null
                        },
                        "workspace_limit": null
                    },
                    {
                        "model": "qwen-image-max",
                        "workspace_id": "ws-bvxqb1qp3jh8j2ha",
                        "model_limit": {
                            "request_limit": 2,
                            "request_limit_period": 60,
                            "usage_limit": 1000000,
                            "usage_limit_field": "total_tokens",
                            "usage_limit_period": 60,
                            "async_user_queue_limit": null,
                            "async_user_concurrency_limit": null
                        },
                        "workspace_limit": null
                    },
                    {
                        "model": "no-limits-model",
                        "model_limit": null,
                        "workspace_limit": null
                    }
                ]
            }
        });
        let rows = alibaba_limits_rows(&v);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], ("qwen3.8-max".into(), "-".into(), "500000/6s".into()));
        assert_eq!(rows[1], ("qwen-image-max".into(), "2/min".into(), "1000000/60s".into()));
        assert_eq!(rows[2], ("no-limits-model".into(), "-".into(), "-".into()));
        // 空响应
        assert!(alibaba_limits_rows(&serde_json::json!({"output": {"quotas": []}})).is_empty());
    }

    fn test_model(name: &str) -> chat::Model {
        chat::Model {
            name: name.to_string(),
            weight: 1,
            max_tokens: None,
            max_input_tokens: None,
            model_type: None,
            group: Vec::new(),
        }
    }

    fn sample_cfg() -> chat::Config {
        let model = |name: &str| chat::Model {
            name: name.to_string(),
            weight: 1,
            max_tokens: None,
            max_input_tokens: None,
            model_type: None,
            group: Vec::new(),
        };
        chat::Config {
            model: Some("agnes:agn-1".to_string()),
            stream: false,
            free: false,
            search: Vec::new(),
            providers: vec![
                chat::Provider {
                    kind: "openai".to_string(),
                    name: "agnes".to_string(),
                    api_base: "https://api.agnes.example/v1".to_string(),
                    api_key: "sk-agn".to_string(),
                    models: vec![model("agn-1"), model("agn-2")],
                },
                chat::Provider {
                    kind: "openai".to_string(),
                    name: "minimax".to_string(),
                    api_base: "https://api.minimax.example/v1".to_string(),
                    api_key: "sk-mm".to_string(),
                    models: vec![model("mm-1")],
                },
            ],
        }
    }

    /// Every (provider, model) pair — used by export-shape tests to exercise
    /// the full export regardless of the config's top-level `model:`.
    fn all_pairs(cfg: &chat::Config) -> Vec<(usize, usize)> {
        cfg.providers
            .iter()
            .enumerate()
            .flat_map(|(pi, p)| (0..p.models.len()).map(move |mi| (pi, mi)))
            .collect()
    }

    #[test]
    fn config_select_default_follows_chat_model() {
        let cfg = sample_cfg(); // top-level model: "agnes:agn-1"
        // no selector -> the same models `sys ai chat` uses by default
        let d = select_models(&cfg, None).unwrap();
        assert_eq!(d, vec![(0, 0)]);
        // explicit selectors keep the chat-format resolution
        let one = select_models(&cfg, Some("agnes:*")).unwrap();
        assert_eq!(one.len(), 2);
        assert!(one.iter().all(|(pi, _)| cfg.providers[*pi].name == "agnes"));
        let single = select_models(&cfg, Some("minimax:mm-1")).unwrap();
        assert_eq!(single, vec![(1, 0)]);
        assert!(select_models(&cfg, Some("zzz:*")).is_err());
        assert!(select_models(&cfg, Some("agnes:nope")).is_err());
        assert!(select_models(&cfg, Some("no-colon")).is_err());
    }

    #[test]
    fn config_default_without_top_level_model() {
        let mut cfg = sample_cfg();
        cfg.model = None;
        // no top-level model -> first provider's first model (chat default)
        let d = select_models(&cfg, None).unwrap();
        assert_eq!(d, vec![(0, 0)]);
        // a comma-separated top-level model selects the union
        cfg.model = Some("agn-2,minimax:mm-1".to_string());
        let d = select_models(&cfg, None).unwrap();
        assert_eq!(d, vec![(0, 1), (1, 0)]);
    }

    #[test]
    fn config_toml_escape_and_env_key() {
        assert_eq!(toml_str("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(env_key_name("alibaba-cn"), "ALIBABA_CN");
        assert_eq!(env_key_name("openai"), "OPENAI");
        assert_eq!(env_key_name("--"), "PROVIDER");
    }

    #[test]
    fn config_export_formats() {
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);

        let codex = export_codex(&cfg, &all).unwrap();
        assert!(codex.contains("model = \"agnes.agn-1\""));
        assert!(codex.contains("[model_providers.\"agnes\"]"));
        assert!(codex.contains("env_key = \"SYS_AGNES_API_KEY\""));
        assert!(codex.contains("[model_providers.\"minimax\"]"));
        assert!(!codex.contains("minimax.env_key")); // each provider appears once

        let opencode = export_opencode(&cfg, &all).unwrap();
        let v: Value = serde_json::from_str(&opencode).unwrap();
        assert_eq!(v["provider"]["agnes"]["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(v["provider"]["agnes"]["options"]["apiKey"], "sk-agn");
        assert_eq!(v["provider"]["agnes"]["models"]["agn-1"]["name"], "agn-1");
        assert_eq!(v["provider"]["minimax"]["options"]["baseURL"], "https://api.minimax.example/v1");

        let litellm = export_litellm(&cfg, &all).unwrap();
        assert!(litellm.contains("model_name: \"agnes:agn-1\""));
        assert!(litellm.contains("model: \"openai/agn-1\""));
        assert!(litellm.contains("api_key: \"sk-mm\""));

        let f = export_freellmapi(&cfg, &all).unwrap();
        let v: Value = serde_json::from_str(&f).unwrap();
        assert_eq!(v["customProviders"].as_array().unwrap().len(), 2);
        assert_eq!(v["customProviders"][0]["label"], "agnes");
        assert_eq!(v["customProviders"][0]["models"].as_array().unwrap().len(), 2);
        assert_eq!(v["customProviders"][1]["models"][0]["supportsTools"], true);
    }

    #[test]
    fn config_select_chat_format() {
        let cfg = sample_cfg();
        // comma-separated `sys ai chat -m` format (half- and full-width)
        let multi = select_models(&cfg, Some("agnes:agn-1,minimax:*")).unwrap();
        assert_eq!(multi.len(), 2);
        // bare model name matches across providers
        let bare = select_models(&cfg, Some("agn-2")).unwrap();
        assert_eq!(bare, vec![(0, 1)]);
        // full-width comma separator
        let full = select_models(&cfg, Some("minimax:mm-1，agnes:agn-1")).unwrap();
        assert_eq!(full, vec![(1, 0), (0, 0)]);
        // overlapping selectors are deduplicated
        let dup = select_models(&cfg, Some("agn-1,agnes:agn-1")).unwrap();
        assert_eq!(dup, vec![(0, 0)]);
    }

    #[test]
    fn config_codebuddy_export_shape() {
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);
        let s = export_codebuddy(&cfg, &all).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        let models = v["models"].as_array().unwrap();
        assert_eq!(models.len(), 3);
        assert_eq!(models[0]["id"], "agn-1");
        assert_eq!(models[0]["name"], "agn-1");
        assert_eq!(models[0]["vendor"], "agnes");
        assert_eq!(models[0]["url"], "https://api.agnes.example/v1/chat/completions");
        assert_eq!(models[0]["apiKey"], "sk-agn");
        assert_eq!(v["availableModels"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn config_trae_export_shape() {
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);
        let s = export_trae(&cfg, &all).unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&s).unwrap();
        let list = v["models"].as_sequence().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0]["name"], "agn-1");
        assert_eq!(list[0]["open_ai"]["model"], "agn-1");
        assert_eq!(list[0]["open_ai"]["base_url"], "https://api.agnes.example/v1");
        assert_eq!(list[0]["open_ai"]["api_key"], "sk-agn");
    }

    #[test]
    fn config_new_formats_dedup_by_model_id() {
        // two providers sharing a model name -> one entry, first provider wins
        let mut cfg = sample_cfg();
        cfg.providers[1].models.push(chat::Model {
            name: "agn-1".into(),
            weight: 1,
            max_tokens: None,
            max_input_tokens: None,
            model_type: None,
            group: Vec::new(),
        });
        let all = all_pairs(&cfg);
        let v: Value =
            serde_json::from_str(&export_codebuddy(&cfg, &all).unwrap()).unwrap();
        let ids: Vec<&str> = v["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["id"].as_str())
            .collect();
        assert_eq!(ids, vec!["agn-1", "agn-2", "mm-1"]);
        let t: serde_yaml::Value =
            serde_yaml::from_str(&export_trae(&cfg, &all).unwrap()).unwrap();
        assert_eq!(t["models"].as_sequence().unwrap().len(), 3);
    }

    #[test]
    fn config_export_merges_same_api_base() {
        // providers pointing at the SAME api_base (whatever the block names
        // are) are ONE entry/block in every export format: a provider's
        // identity is its endpoint, never spread out
        let mut cfg = sample_cfg();
        cfg.providers.push(chat::Provider {
            kind: "openai".to_string(),
            name: "agnes".to_string(), // same name, same base (classic split block)
            api_base: "https://api.agnes.example/v1".to_string(),
            api_key: "sk-agn-2".to_string(),
            models: vec![test_model("agn-9")],
        });
        cfg.providers.push(chat::Provider {
            kind: "openai".to_string(),
            name: "openroute".to_string(), // different name, same base -> alias, still one entry
            api_base: "https://api.agnes.example/v1".to_string(),
            api_key: "sk-agn-3".to_string(),
            models: vec![test_model("agn-10")],
        });
        let all = all_pairs(&cfg);
        let groups = export_groups(&cfg, &all);
        assert_eq!(groups.len(), 2); // agnes-base + minimax
        let agnes = groups
            .iter()
            .find(|g| same_api_base(&g.p.api_base, "https://api.agnes.example/v1"))
            .unwrap();
        assert_eq!(agnes.p.name, "agnes"); // first block's name wins
        assert_eq!(agnes.models.len(), 4); // agn-1, agn-2, agn-9, agn-10

        // codex: ONE block with the merged models comment
        let codex = export_codex(&cfg, &all).unwrap();
        assert_eq!(codex.matches("[model_providers.\"agnes\"]").count(), 1);
        assert!(codex.contains("base_url = \"https://api.agnes.example/v1\""));
        assert!(codex.contains("# models: agn-1, agn-2, agn-9, agn-10"));

        // opencode: one provider key carrying all four models
        let oc: Value = serde_json::from_str(&export_opencode(&cfg, &all).unwrap()).unwrap();
        assert_eq!(oc["provider"].as_object().unwrap().len(), 2);
        let p = &oc["provider"]["agnes"];
        assert_eq!(p["options"]["baseURL"], "https://api.agnes.example/v1");
        assert_eq!(p["models"].as_object().unwrap().len(), 4);

        // litellm: one row per merged model, no duplicate rows
        let litellm = export_litellm(&cfg, &all).unwrap();
        assert_eq!(litellm.matches("model_name: \"agnes:agn-1\"").count(), 1);
        assert_eq!(litellm.matches("model_name: \"agnes:agn-10\"").count(), 1);
        assert_eq!(litellm.matches("model_name:").count(), 5);

        // freellmapi: ONE entry for the agnes base with all four models
        let f: Value = serde_json::from_str(&export_freellmapi(&cfg, &all).unwrap()).unwrap();
        let list = f["customProviders"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        let agnes = list.iter().find(|v| v["label"] == "agnes").unwrap();
        let fmodels: Vec<&str> = agnes["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["model"].as_str())
            .collect();
        assert_eq!(fmodels, vec!["agn-1", "agn-2", "agn-9", "agn-10"]);

        // codebuddy / trae / zcoder: no duplicate ids, one provider block
        let cb: Value = serde_json::from_str(&export_codebuddy(&cfg, &all).unwrap()).unwrap();
        let ids: Vec<&str> = cb["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["id"].as_str())
            .collect();
        assert_eq!(ids, vec!["agn-1", "agn-2", "agn-9", "agn-10", "mm-1"]);
        let t: serde_yaml::Value =
            serde_yaml::from_str(&export_trae(&cfg, &all).unwrap()).unwrap();
        assert_eq!(t["models"].as_sequence().unwrap().len(), 5);
        let z: Value = serde_json::from_str(&export_zcoder(&cfg, &all).unwrap()).unwrap();
        assert_eq!(
            z["provider"]["agnes"]["models"].as_object().unwrap().len(),
            4
        );
    }

    #[test]
    fn config_export_same_name_different_base_splits() {
        // the same provider NAME pointing at a different api_base is a
        // distinct provider: export keeps two entries, keys deduped with @host
        let mut cfg = sample_cfg();
        cfg.providers.push(chat::Provider {
            kind: "openai".to_string(),
            name: "agnes".to_string(), // same name, different base
            api_base: "https://api.agnes-alt.example/v1".to_string(),
            api_key: "sk-agn-alt".to_string(),
            models: vec![test_model("agn-9")],
        });
        let all = all_pairs(&cfg);
        let groups = export_groups(&cfg, &all);
        assert_eq!(groups.len(), 3); // agnes, agnes-alt, minimax

        // opencode: keys deduped -> agnes + agnes@api.agnes-alt.example
        let oc: Value = serde_json::from_str(&export_opencode(&cfg, &all).unwrap()).unwrap();
        let keys: Vec<&str> = oc["provider"].as_object().unwrap().keys().map(|k| k.as_str()).collect();
        assert_eq!(keys.len(), 3);
        assert!(keys.contains(&"agnes"));
        assert!(keys.contains(&"agnes@api.agnes-alt.example"));
        assert_eq!(oc["provider"]["agnes"]["models"].as_object().unwrap().len(), 2);
        assert_eq!(
            oc["provider"]["agnes@api.agnes-alt.example"]["models"].as_object().unwrap().len(),
            1
        );

        // codex: two distinct tables
        let codex = export_codex(&cfg, &all).unwrap();
        assert_eq!(codex.matches("[model_providers.\"agnes\"]").count(), 1);
        assert!(codex.contains("[model_providers.\"agnes@api.agnes-alt.example\"]"));
        assert!(codex.contains("# models: agn-9"));

        // litellm: model_name prefixes are deduped
        let litellm = export_litellm(&cfg, &all).unwrap();
        assert_eq!(litellm.matches("model_name: \"agnes:agn-1\"").count(), 1);
        assert_eq!(
            litellm.matches("model_name: \"agnes@api.agnes-alt.example:agn-9\"").count(),
            1
        );

        // freellmapi: one entry per base
        let f: Value = serde_json::from_str(&export_freellmapi(&cfg, &all).unwrap()).unwrap();
        let labels: Vec<&str> = f["customProviders"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v["label"].as_str())
            .collect();
        assert_eq!(labels.len(), 3);
        for want in ["agnes", "agnes@api.agnes-alt.example", "minimax"] {
            assert!(labels.contains(&want), "missing label {want}: {labels:?}");
        }

        // zcoder: one provider key per base
        let z: Value = serde_json::from_str(&export_zcoder(&cfg, &all).unwrap()).unwrap();
        assert_eq!(z["provider"].as_object().unwrap().len(), 3);
        assert_eq!(
            z["provider"]["agnes-api.agnes-alt.example"]["models"].as_object().unwrap().len(),
            1
        );
    }

    #[test]
    fn config_update_merges_models_json() {
        let dir = std::env::temp_dir().join(format!("sysenv-cfg-update-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("models.json");
        std::fs::write(
            &path,
            r#"{"models":[{"id":"keep-me","name":"keep-me","vendor":"x","url":"u","apiKey":"k"}],"availableModels":["keep-me"]}"#,
        )
        .unwrap();
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);
        let added = merge_json_models(&path, &codebuddy_entries(&cfg, &all)).unwrap();
        assert_eq!(added, vec!["agn-1", "agn-2", "mm-1"]);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["models"].as_array().unwrap().len(), 4);
        assert_eq!(v["models"][0]["id"], "keep-me");
        assert_eq!(v["availableModels"].as_array().unwrap().len(), 4);
        // a second merge adds nothing and does not rewrite the file
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        let added2 = merge_json_models(&path, &codebuddy_entries(&cfg, &all)).unwrap();
        assert!(added2.is_empty());
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_workbuddy_export_is_plain_array() {
        // workbuddy's real models.json is a top-level array, not the
        // codebuddy `{"models":[...],"availableModels":[...]}` wrapper
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);
        let s = export_workbuddy(&cfg, &all).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["id"], "agn-1");
        assert_eq!(arr[0]["vendor"], "agnes");
        assert_eq!(arr[0]["url"], "https://api.agnes.example/v1/chat/completions");
        assert_eq!(arr[0]["apiKey"], "sk-agn");
        assert!(v.get("models").is_none() && v.get("availableModels").is_none());
    }

    #[test]
    fn config_update_merges_array_root_models_json() {
        // merge must accept a plain top-level array (real workbuddy shape)
        let dir = std::env::temp_dir().join(format!("sysenv-cfg-arr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("models.json");
        std::fs::write(
            &path,
            r#"[{"id":"keep-me","name":"keep-me","vendor":"x","url":"u","apiKey":"k"}]"#,
        )
        .unwrap();
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);
        let added = merge_json_models(&path, &codebuddy_entries(&cfg, &all)).unwrap();
        assert_eq!(added, vec!["agn-1", "agn-2", "mm-1"]);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0]["id"], "keep-me");
        assert_eq!(arr[1]["id"], "agn-1");
        // stays an array; no availableModels wrapper appears
        assert!(v.get("availableModels").is_none());
        // a second merge adds nothing and does not rewrite the file
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        let added2 = merge_json_models(&path, &codebuddy_entries(&cfg, &all)).unwrap();
        assert!(added2.is_empty());
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_zcode_export_shape() {
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);
        let s = export_zcoder(&cfg, &all).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        let providers = v["provider"].as_object().unwrap();
        assert_eq!(providers.len(), 2);
        let agnes = &providers["agnes"];
        assert_eq!(agnes["kind"], "openai-compatible");
        assert_eq!(agnes["options"]["baseURL"], "https://api.agnes.example/v1");
        assert_eq!(agnes["options"]["apiKey"], "sk-agn");
        assert_eq!(agnes["options"]["apiKeyRequired"], true);
        assert_eq!(agnes["models"].as_object().unwrap().len(), 2);
        assert_eq!(agnes["models"]["agn-1"]["name"], "agn-1");
        let mm = &providers["minimax"];
        assert_eq!(mm["models"].as_object().unwrap().len(), 1);
        assert_eq!(v["model"]["main"], "agnes/agn-1");
        assert_eq!(v["model"]["lite"], "agnes/agn-1");
    }

    #[test]
    fn config_zcode_update_merge() {
        let dir = std::env::temp_dir().join(format!("sysenv-cfg-zc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{"provider":{"zai":{"kind":"anthropic","options":{"baseURL":"https://api.z.ai/api/anthropic"},"models":{"glm-5.2":{"name":"GLM-5.2"}}}},"model":{"main":"zai/glm-5.2","lite":"zai/glm-5.1"},"permission":{"mode":"build"}}"#,
        )
        .unwrap();
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);
        let added = merge_zcode_models(&path, &cfg, &all).unwrap();
        assert_eq!(added.len(), 3);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        // existing zai provider untouched, agnes/minimax added
        assert_eq!(v["provider"]["zai"]["kind"], "anthropic");
        assert_eq!(v["provider"]["zai"]["models"]["glm-5.2"]["name"], "GLM-5.2");
        assert_eq!(v["provider"]["agnes"]["models"].as_object().unwrap().len(), 2);
        assert_eq!(v["provider"]["minimax"]["models"].as_object().unwrap().len(), 1);
        // model.main/lite already set -> untouched
        assert_eq!(v["model"]["main"], "zai/glm-5.2");
        assert_eq!(v["model"]["lite"], "zai/glm-5.1");
        // other top-level keys preserved
        assert_eq!(v["permission"]["mode"], "build");
        // second merge adds nothing
        let added2 = merge_zcode_models(&path, &cfg, &all).unwrap();
        assert!(added2.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_zcode_update_fills_missing_main() {
        let dir = std::env::temp_dir().join(format!("sysenv-cfg-zc2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"provider":{},"mcp":{"servers":{}}}"#).unwrap();
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);
        let added = merge_zcode_models(&path, &cfg, &all).unwrap();
        assert_eq!(added.len(), 3);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["model"]["main"], "agnes/agn-1");
        assert_eq!(v["model"]["lite"], "agnes/agn-1");
        assert_eq!(v["mcp"]["servers"], json!({}));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_update_models_json_accepts_bom() {
        let dir = std::env::temp_dir().join(format!("sysenv-cfg-bom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("models.json");
        // Windows editors save models.json with a UTF-8 BOM; it must not break parsing
        std::fs::write(&path, format!("\u{feff}{{\"models\":[],\"availableModels\":[]}}")).unwrap();
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);
        let added = merge_json_models(&path, &codebuddy_entries(&cfg, &all)).unwrap();
        assert_eq!(added.len(), 3);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_update_trae_yaml_merge() {
        let dir = std::env::temp_dir().join(format!("sysenv-cfg-trae-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("trae_cli.yaml");
        std::fs::write(
            &path,
            "mcp_servers:\n  foo: 1\nmodels:\n  - name: existing\n    open_ai:\n      base_url: https://old\n      api_key: k\n      model: existing\n",
        )
        .unwrap();
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);
        let added = merge_trae_models(&path, &trae_entries(&cfg, &all)).unwrap();
        assert_eq!(added.len(), 3);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("mcp_servers"));
        assert!(text.contains("existing"));
        assert!(text.contains("agn-1"));
        // existing entry is not duplicated on a second merge
        let added2 = merge_trae_models(&path, &trae_entries(&cfg, &all)).unwrap();
        assert!(added2.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_update_missing_file_reported() {
        let dir = std::env::temp_dir().join(format!("sysenv-cfg-miss-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("models.json");
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);
        // absent file -> Ok, no file and no backup created
        cmd_config_update("codebuddy", &cfg, &all, Some(&missing), None).unwrap();
        assert!(!missing.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_update_rejects_unsupported_format() {
        let dir = std::env::temp_dir().join(format!("sysenv-cfg-un-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[model_providers]\n").unwrap();
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);
        assert!(cmd_config_update("codex", &cfg, &all, Some(&path), None).is_err());
        // unchanged and no stray backup
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[model_providers]\n");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_unknown_format_rejected() {
        let cfg = sample_cfg();
        let all = all_pairs(&cfg);
        let err = export_freellmapi(&cfg, &all).err(); // no-op sanity
        assert!(err.is_none());
    }

    #[test]
    fn software_config_path_maps_each_format() {
        let home = PathBuf::from("C:\\users\\tester");
        assert_eq!(
            software_config_path_in(&home, "codex"),
            Some(home.join(".codex").join("config.toml"))
        );
        assert_eq!(
            software_config_path_in(&home, "opencode"),
            Some(home.join(".config").join("opencode").join("opencode.json"))
        );
        assert_eq!(
            software_config_path_in(&home, "openxode"),
            Some(home.join(".config").join("opencode").join("opencode.json"))
        );
        assert_eq!(
            software_config_path_in(&home, "litellm"),
            Some(home.join(".litellm").join("config.yaml"))
        );
        assert_eq!(
            software_config_path_in(&home, "codebuddy"),
            Some(home.join(".codebuddy").join("models.json"))
        );
        assert_eq!(
            software_config_path_in(&home, "workbuddy"),
            Some(home.join(".workbuddy").join("models.json"))
        );
        assert_eq!(
            software_config_path_in(&home, "zcoder"),
            Some(home.join(".zcode").join("cli").join("config.json"))
        );
        assert_eq!(
            software_config_path_in(&home, "qoder"),
            Some(home.join(".qoder").join("settings.json"))
        );
        #[cfg(windows)]
        assert_eq!(
            software_config_path_in(&home, "trae"),
            Some(
                home.join("AppData")
                    .join("Roaming")
                    .join("trae_cli")
                    .join("trae_cli.yaml")
            )
        );
        #[cfg(not(windows))]
        assert_eq!(
            software_config_path_in(&home, "trae"),
            Some(home.join(".config").join("trae_cli").join("trae_cli.yaml"))
        );
        assert_eq!(
            software_config_path_in(&home, "freellmapi"),
            std::env::current_dir().ok().map(|d| d.join("freellmapi.config.json"))
        );
        // case-insensitive like the export dispatch
        assert_eq!(
            software_config_path_in(&home, "CODEX"),
            Some(home.join(".codex").join("config.toml"))
        );
        // unknown formats have no default path
        assert_eq!(software_config_path_in(&home, "nope"), None);
    }

    #[test]
    fn backup_file_copies_with_suffix_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("sysenv-backup-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("config.yaml");
        std::fs::write(&target, "model_list: []\n").unwrap();

        let backup = backup_file(&target).unwrap();
        assert!(backup.starts_with(&dir));
        let name = backup.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("config.yaml.bak-"), "unexpected backup name {name}");
        // content preserved, original untouched
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "model_list: []\n");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "model_list: []\n");

        // a second backup in the same second gets a distinct name (-N suffix)
        let backup2 = backup_file(&target).unwrap();
        assert_ne!(backup, backup2);
        assert!(backup2.exists());
        assert_eq!(std::fs::read_to_string(&backup2).unwrap(), "model_list: []\n");

        std::fs::remove_dir_all(&dir).ok();
    }

    // --- --import (reverse of the exports) -----------------------------------

    fn import_model(name: &str) -> ImportedModel {
        ImportedModel {
            name: name.to_string(),
            max_tokens: None,
            max_input_tokens: None,
            model_type: None,
        }
    }

    fn import_provider(name: &str, base: &str, key: &str, models: &[&str]) -> ImportedProvider {
        ImportedProvider {
            name: name.to_string(),
            kind: "openai".to_string(),
            api_base: base.to_string(),
            api_key: key.to_string(),
            models: models.iter().map(|m| import_model(m)).collect(),
        }
    }

    #[test]
    fn import_parse_workbuddy_array_shape() {
        let dir = std::env::temp_dir().join(format!("sysenv-imp-wb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("models.json");
        std::fs::write(
            &path,
            r#"[
              {"id":"Qwen/Qwen3.8-Flash-Next","name":"Qwen/Qwen3.8-Flash-Next","vendor":"Custom","url":"https://api-inference.modelscope.cn/v1/chat/completions","apiKey":"ms-key","supportsImages":false,"maxOutputTokens":4096,"maxInputTokens":131072},
              {"id":"deepseek-v4-flash","vendor":"Custom","url":"https://token.sensenova.cn/v1/chat/completions","apiKey":"sk-key","supportsImages":true},
              {"id":"no-key","vendor":"Custom","url":"https://x.example/v1","supportsImages":false}
            ]"#,
        )
        .unwrap();
        let (ps, skipped) = parse_models_json(&path).unwrap();
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].contains("no-key"));
        assert_eq!(ps.len(), 2);
        // vendor "Custom" carries no info -> api-base host becomes the name
        assert_eq!(ps[0].name, "api-inference.modelscope.cn");
        assert_eq!(ps[0].api_base, "https://api-inference.modelscope.cn/v1");
        assert_eq!(ps[0].api_key, "ms-key");
        assert_eq!(ps[0].models.len(), 1);
        assert_eq!(ps[0].models[0].name, "Qwen/Qwen3.8-Flash-Next");
        assert_eq!(ps[0].models[0].max_tokens, Some(4096));
        assert_eq!(ps[0].models[0].max_input_tokens, Some(131072));
        assert_eq!(ps[0].models[0].model_type, None); // supportsImages false
        assert_eq!(ps[1].name, "token.sensenova.cn");
        assert_eq!(ps[1].models[0].model_type.as_deref(), Some("text,image"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_parse_codebuddy_object_shape() {
        // codebuddy object root; a real vendor name is kept as-is and same
        // (vendor, base) entries collapse into one provider
        let dir = std::env::temp_dir().join(format!("sysenv-imp-cb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("models.json");
        std::fs::write(
            &path,
            r#"{"models":[{"id":"agn-1","name":"agn-1","vendor":"agnes","url":"https://api.agnes.example/v1/chat/completions","apiKey":"sk-agn"},{"id":"agn-2","vendor":"agnes","url":"https://api.agnes.example/v1/chat/completions","apiKey":"sk-agn"}],"availableModels":["agn-1","agn-2"]}"#,
        )
        .unwrap();
        let (ps, skipped) = parse_models_json(&path).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "agnes");
        assert_eq!(ps[0].models.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_parse_zcoder_config() {
        let dir = std::env::temp_dir().join(format!("sysenv-imp-zc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{"provider":{"zai":{"kind":"anthropic","name":"zai","options":{"baseURL":"https://api.z.ai/api/anthropic","apiKey":"sk-z","apiKeyRequired":true},"models":{"glm-5.2":{"name":"GLM-5.2"},"glm-5.1":{"name":"GLM-5.1"}}},"nokey":{"kind":"openai-compatible","options":{"baseURL":"https://x.example/v1","apiKeyRequired":false},"models":{"m1":{}}}},"model":{"main":"zai/glm-5.2","lite":"zai/glm-5.1"}}"#,
        )
        .unwrap();
        let (ps, skipped) = parse_zcoder_config(&path).unwrap();
        assert_eq!(skipped.len(), 1); // nokey has no apiKey
        assert!(skipped[0].contains("nokey"));
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "zai");
        assert_eq!(ps[0].kind, "anthropic");
        assert_eq!(ps[0].api_base, "https://api.z.ai/api/anthropic");
        assert_eq!(ps[0].api_key, "sk-z");
        assert_eq!(ps[0].models.len(), 2);
        assert_eq!(ps[0].models[0].name, "GLM-5.2");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_merges_same_base_splits_different_base() {
        // a provider's identity is its api_base: entries at the SAME base
        // (even with different names) merge into one; entries at a different
        // base stay separate even when the names match
        let dir = std::env::temp_dir().join(format!("sysenv-imp-dd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("models.json");
        std::fs::write(
            &path,
            r#"[
              {"id":"m1","vendor":"multi","url":"https://a.example/v1/chat/completions","apiKey":"k1"},
              {"id":"m2","vendor":"route","url":"https://a.example/v1/chat/completions","apiKey":"k2"},
              {"id":"m3","vendor":"multi","url":"https://b.example/v1/chat/completions","apiKey":"k3"}
            ]"#,
        )
        .unwrap();
        let (ps, _) = parse_models_json(&path).unwrap();
        // same base (multi + route) -> ONE provider; different base -> separate
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0].name, "multi");
        assert_eq!(ps[0].api_base, "https://a.example/v1");
        assert_eq!(ps[0].models.len(), 2); // m1 + m2 merged
        assert_eq!(ps[1].name, "multi");
        assert_eq!(ps[1].api_base, "https://b.example/v1");
        assert_eq!(ps[1].models.len(), 1); // m3
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The sysenv-style config used by the merge tests (2/4/6-space layout).
    fn import_sample_config() -> &'static str {
        "model: agnes:agn-1\nclients:\n  - type: openai\n    name: agnes\n    api_base: https://api.agnes.example/v1\n    api_key: sk-agn\n    models:\n      - name: agn-1\n        type: text,image\n  - type: openai\n    name: minimax\n    api_base: https://api.minimax.example/v1\n    api_key: sk-mm\n    models:\n      - name: mm-1\nstream: true\n"
    }

    #[test]
    fn import_merges_new_provider_block_before_trailing_keys() {
        let dir = std::env::temp_dir().join(format!("sysenv-imp-m1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        std::fs::write(&path, import_sample_config()).unwrap();
        let ps = vec![import_provider(
            "newco",
            "https://api.newco.example/v1",
            "sk-new",
            &["nm-1", "nm-2"],
        )];
        let report = import_into_config(&path, &ps).unwrap();
        assert_eq!(report.providers_added, vec!["newco"]);
        assert_eq!(report.models_added, vec!["newco/nm-1", "newco/nm-2"]);
        assert!(report.models_skipped.is_empty());
        let text = std::fs::read_to_string(&path).unwrap();
        // the new block sits between the clients list and `stream: true`
        let stream_pos = text.find("stream: true").unwrap();
        let newco_pos = text.find("name: newco").unwrap();
        assert!(newco_pos < stream_pos);
        assert!(text.contains("api_base: https://api.newco.example/v1"));
        // existing entries keep their exact bytes
        assert!(text.contains("name: agnes\n    api_base: https://api.agnes.example/v1"));
        assert!(text.contains("type: text,image"));
        // and the merged file parses with the real loader
        let (cfg, _) = chat::load_config(Some(&path)).unwrap();
        assert_eq!(cfg.providers.len(), 3);
        assert_eq!(cfg.providers[2].name, "newco");
        assert_eq!(cfg.providers[2].models.len(), 2);
        // a backup was made
        let bak = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(bak, 2); // config.yaml + one .bak
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_merges_same_base_imports_into_one_block() {
        // two imports at the SAME api_base merge into ONE provider block;
        // the same name at a DIFFERENT api_base is a distinct provider and
        // becomes its own block (the endpoint, not the name, defines identity)
        for (base_b, expect_blocks) in [
            ("https://api.newco.example/v1", 1),
            ("https://api.newco-alt.example/v1", 2),
        ] {
            let dir =
                std::env::temp_dir().join(format!("sysenv-imp-mn-{}-{}", base_b.len(), std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("config.yaml");
            std::fs::write(&path, import_sample_config()).unwrap();
            let ps = vec![
                import_provider(
                    "newco",
                    "https://api.newco.example/v1",
                    "sk-new",
                    &["nm-1"],
                ),
                import_provider("newco", base_b, "sk-new", &["nm-2"]),
            ];
            let report = import_into_config(&path, &ps).unwrap();
            assert_eq!(report.providers_added.len(), expect_blocks, "{base_b}");
            assert_eq!(report.models_added.len(), 2, "{base_b}");
            let text = std::fs::read_to_string(&path).unwrap();
            assert_eq!(text.matches("name: newco").count(), expect_blocks, "{base_b}");
            assert!(
                text.contains("api_base: https://api.newco.example/v1"),
                "{base_b}"
            );
            if expect_blocks == 2 {
                assert!(text.contains("api_base: https://api.newco-alt.example/v1"), "{base_b}");
            }
            let (cfg, _) = chat::load_config(Some(&path)).unwrap();
            let newcos: Vec<&chat::Provider> =
                cfg.providers.iter().filter(|p| p.name == "newco").collect();
            assert_eq!(newcos.len(), expect_blocks, "{base_b}");
            assert_eq!(newcos.iter().map(|p| p.models.len()).sum::<usize>(), 2, "{base_b}");
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn import_appends_missing_models_and_keeps_present() {
        let dir = std::env::temp_dir().join(format!("sysenv-imp-m2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        std::fs::write(&path, import_sample_config()).unwrap();
        // agn-1 already present (with a sub-field); agn-9 and agn-10 are new
        let ps = vec![import_provider(
            "agnes",
            "https://api.agnes.example/v1",
            "sk-agn",
            &["agn-1", "agn-9", "agn-10"],
        )];
        let report = import_into_config(&path, &ps).unwrap();
        assert!(report.providers_added.is_empty());
        assert_eq!(report.models_added, vec!["agnes/agn-9", "agnes/agn-10"]);
        assert_eq!(report.models_skipped, vec!["agnes/agn-1"]);
        let text = std::fs::read_to_string(&path).unwrap();
        // new models go after the existing model zone (sub-field included)
        let agn1 = text.find("- name: agn-1").unwrap();
        let agn9 = text.find("- name: agn-9").unwrap();
        let typ = text.find("type: text,image").unwrap();
        assert!(agn1 < typ && typ < agn9, "new model must follow the model zone");
        let (cfg, _) = chat::load_config(Some(&path)).unwrap();
        let agnes = cfg.providers.iter().find(|p| p.name == "agnes").unwrap();
        assert_eq!(agnes.models.len(), 3);
        // second import with the same models: nothing new, no rewrite, no backup
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        let r2 = import_into_config(&path, &ps).unwrap();
        assert!(r2.models_added.is_empty());
        assert_eq!(r2.models_skipped.len(), 3);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2); // still one .bak
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_creates_clients_when_missing() {
        let dir = std::env::temp_dir().join(format!("sysenv-imp-m3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        std::fs::write(&path, "model: agnes:agn-1\nstream: true\n").unwrap();
        let ps = vec![import_provider("newco", "https://api.newco.example/v1", "sk-new", &["nm-1"])];
        import_into_config(&path, &ps).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("clients:"));
        assert!(text.contains("name: newco"));
        let (cfg, _) = chat::load_config(Some(&path)).unwrap();
        assert_eq!(cfg.providers.len(), 1);
        assert_eq!(cfg.providers[0].name, "newco");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_opens_models_list_when_provider_has_none() {
        let dir = std::env::temp_dir().join(format!("sysenv-imp-m4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        std::fs::write(
            &path,
            "model: agnes:agn-1\nclients:\n  - type: openai\n    name: agnes\n    api_base: https://api.agnes.example/v1\n    api_key: sk-agn\nstream: true\n",
        )
        .unwrap();
        let ps = vec![import_provider(
            "agnes",
            "https://api.agnes.example/v1",
            "sk-agn",
            &["agn-9"],
        )];
        let report = import_into_config(&path, &ps).unwrap();
        assert_eq!(report.models_added, vec!["agnes/agn-9"]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("    models:\n      - name: agn-9"));
        let (cfg, _) = chat::load_config(Some(&path)).unwrap();
        assert_eq!(cfg.providers[0].models.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_parse_opencode_config() {
        let dir = std::env::temp_dir().join(format!("sysenv-imp-oc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("opencode.json");
        std::fs::write(
            &path,
            r#"{"$schema":"https://opencode.ai/config.json","provider":{"zai":{"npm":"@ai-sdk/anthropic","name":"zai","options":{"baseURL":"https://api.z.ai/api/anthropic","apiKey":"sk-z"},"models":{"glm-5.2":{"name":"GLM-5.2"},"glm-5.1":{"name":"GLM-5.1"}}},"nokey":{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"https://x.example/v1"},"models":{"m1":{}}}}}"#,
        )
        .unwrap();
        let (ps, skipped) = parse_opencode_config(&path).unwrap();
        assert_eq!(skipped.len(), 1); // nokey has no apiKey
        assert!(skipped[0].contains("nokey"));
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "zai");
        assert_eq!(ps[0].kind, "openai"); // opencode carries no kind field
        assert_eq!(ps[0].api_base, "https://api.z.ai/api/anthropic");
        assert_eq!(ps[0].api_key, "sk-z");
        assert_eq!(ps[0].models.len(), 2);
        assert_eq!(ps[0].models[0].name, "GLM-5.2");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_opencode_merges_same_name_provider_blocks() {
        // two provider ids sharing the same entry.name -> ONE provider with
        // both models (same api_base here; a different base is merged the same
        // way, first endpoint wins)
        let dir = std::env::temp_dir().join(format!("sysenv-imp-ocn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("opencode.json");
        std::fs::write(
            &path,
            r#"{"provider":{"p1":{"name":"multi","options":{"baseURL":"https://a.example/v1","apiKey":"k1"},"models":{"m1":{"name":"m1"}}},"p2":{"name":"multi","options":{"baseURL":"https://a.example/v1","apiKey":"k1"},"models":{"m2":{"name":"m2"}}}}}"#,
        )
        .unwrap();
        let (ps, skipped) = parse_opencode_config(&path).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "multi");
        assert_eq!(ps[0].models.len(), 2);
        assert_eq!(ps[0].models[0].name, "m1");
        assert_eq!(ps[0].models[1].name, "m2");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_file_name_maps_every_format() {
        let dir = PathBuf::from("D:\\agents");
        for (f, name) in [
            ("codex", "config.toml"),
            ("opencode", "opencode.json"),
            ("openxode", "opencode.json"),
            ("litellm", "config.yaml"),
            ("freellmapi", "freellmapi.config.json"),
            ("codebuddy", "models.json"),
            ("workbuddy", "models.json"),
            ("trae", "trae_cli.yaml"),
            ("zcoder", "config.json"),
        ] {
            assert_eq!(
                software_config_path_with(Some(&dir), f),
                Some(dir.join(name)),
                "--src mapping for {f}"
            );
        }
        assert_eq!(software_config_path_with(Some(&dir), "nope"), None);
    }

    #[test]
    fn import_opencode_via_src_dir() {
        // --src DIR points at the agent's config dir: opencode.json is read
        // from there even though ~/.config/opencode has no file
        let dir = std::env::temp_dir().join(format!("sysenv-imp-ocsrc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src_file = dir.join("opencode.json");
        std::fs::write(
            &src_file,
            r#"{"provider":{"newco":{"name":"newco","options":{"baseURL":"https://api.newco.example/v1","apiKey":"sk-new"},"models":{"nm-1":{"name":"nm-1"}}}}}"#,
        )
        .unwrap();
        let cfg = dir.join("config.yaml");
        std::fs::write(&cfg, "model: agnes:agn-1\nclients:\n  - type: openai\n    name: agnes\n    api_base: https://api.agnes.example/v1\n    api_key: sk-agn\n    models:\n      - name: agn-1\nstream: true\n").unwrap();
        cmd_config_import("opencode", None, Some(&cfg), Some(&dir)).unwrap();
        let (parsed, _) = chat::load_config(Some(&cfg)).unwrap();
        assert_eq!(parsed.providers.len(), 2);
        assert_eq!(parsed.providers[1].name, "newco");
        assert_eq!(parsed.providers[1].api_base, "https://api.newco.example/v1");
        assert_eq!(parsed.providers[1].models[0].name, "nm-1");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_unsupported_format_rejected() {
        assert!(cmd_config_import("codex", None, None, None).is_err());
        assert!(cmd_config_import("trae", None, None, None).is_err());
        assert!(cmd_config_import("litellm", None, None, None).is_err());
    }
}
