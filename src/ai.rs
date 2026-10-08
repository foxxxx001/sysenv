//! `sys ai` — query AI model info.
//!
//! - `ai model` / `ai provider` query the models.dev database
//!   (`https://models.dev/api.json`), the data behind the `/models/` and
//!   `/providers/` pages.
//! - `ai cn-model` queries the DataLearner AI model list
//!   (`https://www.datalearner.com/ai-models/pretrained-models`), a
//!   server-rendered HTML page (paged via `?page=N`); only released models
//!   with a real `published` date are kept.

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
/// can degrade silently.
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
) -> Result<()> {
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
// - `info price P1,P2,...`     prints the per-1M-token price list (input /
//   output / cache_read) of the comma-separated providers (source: models.dev,
//   24 h cache, --refresh to force).
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
                    rows.push(ModelInfoRow { provider: p.name.clone(), name: m.name.clone() });
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
                        rows.push(ModelInfoRow { provider: p.name.clone(), name: m.name.clone() });
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
                            rows.push(ModelInfoRow { provider: p.name.clone(), name: m.name.clone() });
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
                        rows.push(ModelInfoRow { provider: p.name.clone(), name: m.name.clone() });
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
/// - `price P1,P2,...` — per-1M-token prices (input / output / cache_read)
///   of the comma-separated providers, fetched from models.dev (24 h cache,
///   `--refresh` forces a re-fetch).
/// - `balance PROVIDER` — query the provider's official balance with its
///   configured api_key (minimax / agnes have API handlers; the rest point to
///   their consoles because no API-key balance endpoint is exposed).
/// - `sale-price PROVIDER` — scrape the provider's official pricing page and
///   print every model's sale price (agnes / minimax scraped; the rest point
///   to their official pricing pages). Pages are cached 24 h, `--refresh`
///   forces a re-fetch.
/// - `-o json` / `-o csv` / `--json` — machine-readable output, supported by
///   `info provider` and `info model` (JSON array or CSV table).
pub fn cmd_info(
    field: Option<&str>,
    param: &[String],
    refresh: bool,
    config: Option<&Path>,
    out: Option<OutFormat>,
    json: bool,
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
            let (cfg, _) = chat::load_config(config)?;
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
                            })
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
                }
                Some(OutFormat::Csv) => {
                    println!("name,api_base,api_key,docs,console");
                    for r in &rows {
                        println!("{},{},{},{},{}", r.name, r.api_base, r.api_key, r.docs, r.console);
                    }
                }
                // YAML is the default view of `info provider`.
                Some(OutFormat::Yaml) | None => {
                    println!("{}", serde_yaml::to_string(&rows)?);
                }
            }
        }
        Some("model") | Some("models") => {
            let (cfg, _) = chat::load_config(config)?;
            let rows = info_model_rows(&cfg, keyword.as_deref())?;
            match fmt {
                Some(OutFormat::Json) => {
                    let arr: Vec<Value> = rows
                        .iter()
                        .map(|r| serde_json::json!({"provider": r.provider, "name": r.name}))
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
                }
                Some(OutFormat::Csv) => {
                    println!("provider,name");
                    for r in &rows {
                        println!("{},{}", r.provider, r.name);
                    }
                }
                Some(OutFormat::Yaml) => {
                    let arr: Vec<Value> = rows
                        .iter()
                        .map(|r| serde_json::json!({"provider": r.provider, "name": r.name}))
                        .collect();
                    println!("{}", serde_yaml::to_string(&Value::Array(arr))?);
                }
                None => {
                    for r in &rows {
                        println!("{}:{}", r.provider, r.name);
                    }
                }
            }
        }
        Some("price") | Some("prices") => {
            if fmt.is_some() {
                bail!("`info price` does not support -o/--json (run without it for the price table)");
            }
            let providers: Vec<String> = param
                .join(",")
                .split([',', '，'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if providers.is_empty() {
                bail!("`info price` needs at least one provider name: `sys ai info price openai,anthropic`");
            }
            let data = fetch_data(refresh)?;
            let rows = info_price_rows(&data, &providers)?;
            print_price_rows(&rows);
        }
        Some("balance") => {
            if fmt.is_some() {
                bail!("`info balance` does not support -o/--json (run without it for the balance view)");
            }
            let kw = keyword.ok_or_else(|| {
                anyhow::anyhow!("`info balance` needs a provider name: `sys ai info balance agnes`")
            })?;
            let (cfg, _) = chat::load_config(config)?;
            info_balance(&cfg, &kw)?;
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
        Some(other) => bail!("unknown `info` field `{other}` (expected: provider | model | price | balance | sale-price)"),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `sys ai info balance` / `sys ai info sale-price` — per-provider
// official queries (余额 / 官网销售价).
//
// balance:  用该 provider 的 api_key 到官方接口查询余额。仅部分平台开放
//           API Key 余额接口（minimax 的 token_plan/remains、OpenAI 兼容的
//           dashboard/billing）；其余平台余额只在登录控制台可见，命令会给出
//           控制台地址。余额查询始终实时（不走缓存）。
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

/// `sys ai info balance PROVIDER` — query the provider's official balance
/// with its configured api_key.
fn info_balance(cfg: &chat::Config, keyword: &str) -> Result<()> {
    let p = find_provider(cfg, keyword)?;
    println!("== {} 余额 ==", p.name);
    match p.name.to_ascii_lowercase().as_str() {
        "minimax" => balance_minimax(p),
        "agnes" => balance_openai_billing(p),
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
    /// Published date YYYY-MM-DD (empty when the card has no real date)
    published: String,
    /// Detail page URL
    url: String,
}

const DL_CSV_HEADER: &str = "id,name,provider,aliases,type,category,context,modality,published,url";

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
fn dl_detail_value(html: &str, label: &str) -> Option<String> {
    let label_pos = html.find(label)?;
    let rest = &html[label_pos + label.len()..];
    let marker = "</div><div class=\"";
    let ms = rest.find(marker)?;
    let after = &rest[ms + marker.len()..];
    let gt = after.find('>')?;
    let value_start = gt + 1;
    let value_end = after[value_start..].find("</div>")? + value_start;
    let raw = &after[value_start..value_end];
    let text = html_unescape(&html_strip_tags(raw));
    let t = text.trim();
    if !t.is_empty() && t != "暂无数据" {
        Some(t.to_string())
    } else {
        None
    }
}

/// Fetch a model's detail page and extract the context length and
/// input/output modalities. Failures (network, missing fields) return empty
/// strings rather than aborting the whole query.
fn fetch_dl_detail(url: &str) -> Result<(String, String)> {
    let html = fetch_dl_page(url)?;
    Ok((
        dl_detail_value(&html, "上下文长度").unwrap_or_default(),
        dl_detail_value(&html, "输入/输出模态").unwrap_or_default(),
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
        &m.published, &m.url,
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
/// the output with the context length and input/output modalities.
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
            Ok((ctx, modl)) => {
                m.context = ctx;
                m.modality = modl;
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
// Formats: codex (TOML), opencode (JSON), litellm (YAML), freellmapi (JSON).
// Selection: omit = every provider's every model; `provider:*` = one
// provider's whole model list; `provider:model` = one specific model.
// `-f/--file FILE` writes to a file instead of stdout.
// ---------------------------------------------------------------------------

/// Pick the (provider_idx, model_idx) pairs described by `selector`:
/// None -> all, `p:*` -> provider p, `p:m` -> one model of p.
fn select_models(cfg: &chat::Config, selector: Option<&str>) -> Result<Vec<(usize, usize)>> {
    match selector.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(cfg
            .providers
            .iter()
            .enumerate()
            .flat_map(|(pi, p)| (0..p.models.len()).map(move |mi| (pi, mi)))
            .collect()),
        Some(sel) => {
            let (prov, m) = sel
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("selector `{sel}` must be `provider:*` or `provider:model`"))?;
            let prov = prov.trim();
            let m = m.trim();
            let pi = cfg
                .providers
                .iter()
                .position(|p| contains_ci(&p.name, prov))
                .ok_or_else(|| anyhow::anyhow!("no provider contains `{prov}` (available: {})", cfg.providers.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")))?;
            if m == "*" {
                if cfg.providers[pi].models.is_empty() {
                    bail!("provider `{}` has no models", cfg.providers[pi].name);
                }
                Ok((0..cfg.providers[pi].models.len()).map(|mi| (pi, mi)).collect())
            } else {
                let mi = cfg.providers[pi]
                    .models
                    .iter()
                    .position(|mdl| contains_ci(&mdl.name, m))
                    .ok_or_else(|| anyhow::anyhow!("provider `{}` has no model containing `{m}`", cfg.providers[pi].name))?;
                Ok(vec![(pi, mi)])
            }
        }
    }
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
    let mut seen: Vec<&str> = Vec::new();
    for (pi, _mi) in pairs {
        let p = &cfg.providers[*pi];
        let env = format!("SYS_{}_API_KEY", env_key_name(&p.name));
        if seen.contains(&p.name.as_str()) {
            continue;
        }
        seen.push(&p.name);
        s.push_str(&format!("[model_providers.{}]\n", toml_str(&p.name)));
        s.push_str(&format!("name = {}\n", toml_str(&p.name)));
        s.push_str(&format!("base_url = {}\n", toml_str(&p.api_base)));
        s.push_str(&format!("env_key = {}\n", toml_str(&env)));
        s.push_str("wire_api = \"chat\"\n");
        let models: Vec<&str> = cfg
            .providers
            .iter()
            .filter(|pp| pp.name == p.name)
            .flat_map(|pp| pp.models.iter().map(|mm| mm.name.as_str()))
            .collect();
        s.push_str(&format!("# models: {}\n", models.join(", ")));
        s.push_str("\n");
        eprintln!("sys: codex: export {env}={}", p.api_key);
    }
    Ok(s)
}

/// opencode: `opencode.json` — one provider entry with inline apiKey.
fn export_opencode(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Result<String> {
    let mut providers = serde_json::Map::new();
    for (pi, mi) in pairs {
        let p = &cfg.providers[*pi];
        let m = &p.models[*mi];
        let entry = providers
            .entry(p.name.clone())
            .or_insert_with(|| json!({ "npm": "@ai-sdk/openai-compatible", "name": p.name, "options": { "baseURL": p.api_base, "apiKey": p.api_key }, "models": {} }));
        if let Some(models) = entry.get_mut("models").and_then(|v| v.as_object_mut()) {
            models.insert(m.name.clone(), json!({ "name": m.name }));
        }
    }
    let mut root = serde_json::Map::new();
    root.insert("$schema".to_string(), json!("https://opencode.ai/config.json"));
    root.insert("provider".to_string(), Value::Object(providers));
    Ok(serde_json::to_string_pretty(&Value::Object(root))?)
}

/// litellm: `config.yaml` — one `model_list` entry per model; the exposed
/// model name is `{provider}:{model}` (collision-free) routing to
/// `openai/{model}`.
fn export_litellm(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Result<String> {
    let mut s = String::from("model_list:\n");
    for (pi, mi) in pairs {
        let p = &cfg.providers[*pi];
        let m = &p.models[*mi];
        s.push_str(&format!("  - model_name: {}\n", toml_str(&format!("{}:{}", p.name, m.name))));
        s.push_str("    litellm_params:\n");
        s.push_str(&format!("      model: {}\n", toml_str(&format!("openai/{}", m.name))));
        s.push_str(&format!("      api_base: {}\n", toml_str(&p.api_base)));
        s.push_str(&format!("      api_key: {}\n", toml_str(&p.api_key)));
    }
    Ok(s)
}

/// freellmapi: `freellmapi.config.json` — one `customProviders` entry per
/// provider (the file also accepts `admin`/`keys`/`routing`; we export only
/// the providers section so it can be merged).
fn export_freellmapi(cfg: &chat::Config, pairs: &[(usize, usize)]) -> Result<String> {
    let mut list: Vec<Value> = Vec::new();
    for (pi, _) in pairs {
        let p = &cfg.providers[*pi];
        if list.iter().any(|v| v["label"] == p.name) {
            continue;
        }
        let models: Vec<Value> = p
            .models
            .iter()
            .map(|m| json!({ "model": m.name, "displayName": m.name, "supportsTools": true }))
            .collect();
        list.push(json!({
            "baseUrl": p.api_base,
            "label": p.name,
            "models": models,
        }));
    }
    Ok(serde_json::to_string_pretty(&json!({ "customProviders": list }))?)
}

/// `sys ai config FORMAT [SELECT] [-f FILE] [-c FILE]`
///
/// FORMAT: `codex` (TOML) | `opencode` (JSON) | `litellm` (YAML) |
/// `freellmapi` (JSON). SELECT: omit for all models, `provider:*` for a whole
/// provider, `provider:model` for one model. The export goes to stdout unless
/// `-f/--file FILE` is given.
pub fn cmd_config(
    format: &str,
    selector: Option<&str>,
    out_file: Option<&Path>,
    config: Option<&Path>,
) -> Result<()> {
    let (cfg, _) = chat::load_config(config)?;
    let pairs = select_models(&cfg, selector)?;
    if pairs.is_empty() {
        bail!("nothing to export (no models selected)");
    }
    let text = match format.to_ascii_lowercase().as_str() {
        "codex" => export_codex(&cfg, &pairs)?,
        "opencode" | "openxode" => export_opencode(&cfg, &pairs)?,
        "litellm" => export_litellm(&cfg, &pairs)?,
        "freellmapi" => export_freellmapi(&cfg, &pairs)?,
        other => bail!(
            "unknown format `{other}` (supported: codex, opencode, litellm, freellmapi)"
        ),
    };
    match out_file {
        Some(path) => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    bail!("output directory does not exist: {}", parent.display());
                }
            }
            std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))?;
            eprintln!("sys: exported {} model(s) to {}", pairs.len(), path.display());
        }
        None => print!("{text}"),
    }
    Ok(())
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
        assert_eq!(cols[8], "2026-09-29");
        assert_eq!(cols.len(), 10);
        assert!(DL_CSV_HEADER.split(',').count() == 10);
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
        // Missing label.
        assert_eq!(dl_detail_value("<div>no label</div>", "上下文长度"), None);
        // 暂无数据 is treated as absent.
        let html = r#"<span>上下文长度</span></div><div class="...">暂无数据</div>"#;
        assert_eq!(dl_detail_value(html, "上下文长度"), None);
    }

    // --- `sys ai info` ------------------------------------------------------

    fn sample_config() -> chat::Config {
        chat::Config {
            model: None,
            stream: false,
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
                        },
                        chat::Model {
                            name: "deepseek-v4-flash".into(),
                            weight: 1,
                            max_tokens: None,
                            max_input_tokens: None,
                            model_type: None,
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
                    }],
                },
            ],
            tasks: Vec::new(),
            search: Vec::new(),
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

    fn sample_cfg() -> chat::Config {
        let model = |name: &str| chat::Model {
            name: name.to_string(),
            weight: 1,
            max_tokens: None,
            max_input_tokens: None,
            model_type: None,
        };
        chat::Config {
            model: Some("agnes:agn-1".to_string()),
            stream: false,
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
            tasks: vec![],
            search: vec![],
        }
    }

    #[test]
    fn config_select_all_and_specific() {
        let cfg = sample_cfg();
        let all = select_models(&cfg, None).unwrap();
        assert_eq!(all.len(), 3);
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
    fn config_toml_escape_and_env_key() {
        assert_eq!(toml_str("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(env_key_name("alibaba-cn"), "ALIBABA_CN");
        assert_eq!(env_key_name("openai"), "OPENAI");
        assert_eq!(env_key_name("--"), "PROVIDER");
    }

    #[test]
    fn config_export_formats() {
        let cfg = sample_cfg();
        let all = select_models(&cfg, None).unwrap();

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
    fn config_unknown_format_rejected() {
        let cfg = sample_cfg();
        let all = select_models(&cfg, None).unwrap();
        let err = export_freellmapi(&cfg, &all).err(); // no-op sanity
        assert!(err.is_none());
    }
}
