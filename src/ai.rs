//! `sysenv ai` — query AI model info.
//!
//! - `ai model` / `ai provider` query the models.dev database
//!   (`https://models.dev/api.json`), the data behind the `/models/` and
//!   `/providers/` pages.
//! - `ai cn-model` queries the DataLearner AI model list
//!   (`https://www.datalearner.com/ai-models/pretrained-models`), a
//!   server-rendered HTML page (paged via `?page=N`); only released models
//!   with a real `published` date are kept.

use anyhow::{Context, Result, bail};
use crate::OutFormat;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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
            PathBuf::from(p).join("sysenv")
        } else if let Some(p) = std::env::var_os("USERPROFILE") {
            PathBuf::from(p).join(".sysenv").join("cache")
        } else {
            std::env::temp_dir().join("sysenv")
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(p) = std::env::var_os("XDG_CACHE_HOME") {
            PathBuf::from(p).join("sysenv")
        } else if let Some(h) = std::env::var_os("HOME") {
            PathBuf::from(h).join(".cache").join("sysenv")
        } else {
            std::env::temp_dir().join("sysenv")
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
    eprintln!("sysenv: fetching {DATA_URL} ...");
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
) -> Result<()> {
    let data = fetch_data(refresh)?;
    let fmt = out.or(if json { Some(OutFormat::Json) } else { None });

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
        let total_hits = hits.len();
        let take = limit.unwrap_or(default_limit).max(1);
        match fmt {
            Some(OutFormat::Json) => {
                let arr: Vec<Value> = hits.into_iter().take(take).map(|(_, m)| m).collect();
                println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
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
// `sysenv ai cn-model` — query the DataLearner AI model list.
//
// The DataLearner page (https://www.datalearner.com/ai-models/pretrained-models)
// is server-rendered HTML. Models are listed as cards in the "全部模型" grid,
// one page at a time (`?page=N`, page 1 has no query string). Each released
// model card carries: name, provider, optional aliases (又名), type badges
// (预览版 / 精选 / 开源模型 / 闭源模型 ...), a published date and a category.
// Rumor (传闻) cards carry "预计发布" instead of a real date and are skipped.
// ---------------------------------------------------------------------------

const DL_BASE: &str = "https://www.datalearner.com/ai-models/pretrained-models";
const DL_UA: &str = "Mozilla/5.0 (compatible; sysenv)";
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
    eprintln!("sysenv: fetching {DL_BASE} ...");
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

/// `sysenv ai cn-model [NAME | -s QUERY] [--date DATE] [--model-type TYPE] [--limit N] [-o json|csv] [--refresh]`
///
/// Query-only: a NAME or `--search` is required (no `--list` / bare full
/// listing, no `--open` — DataLearner exposes no structured open-weights
/// field). `--date YYYY-MM-DD` keeps only models published strictly after it;
/// `--model-type` keeps only models whose category matches the given kind
/// (text / image / audio / video / multimodal). A single exact hit fetches the
/// detail page to enrich the output with the context length and
/// input/output modalities.
pub fn cmd_cn_model(
    name: Option<&str>,
    search: Option<&str>,
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

    // --date / --model-type alone lists every matching model.
    if name.is_none() && search.is_none() {
        if date.is_none() && model_type.is_none() {
            bail!("provide a model NAME or --search QUERY (data: {DL_BASE})");
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
        return output_dl(&hits, limit.unwrap_or(20).max(1), fmt, true);
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
            Err(e) => eprintln!("sysenv: warning: cannot fetch detail page for {}: {e:#}", m.name),
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
}
