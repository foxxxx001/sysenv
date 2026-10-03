//! `sysenv ai` — query the models.dev database of AI models & providers.
//!
//! The models.dev `/models/` and `/providers/` pages are generated from the
//! same data that is served at `https://models.dev/api.json`; this command
//! fetches that data and filters it by model / provider name.

use anyhow::{Context, Result, bail};
use crate::OutFormat;
use serde_json::Value;
use std::collections::HashMap;
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
    limit: usize,
    json: bool,
    out: Option<OutFormat>,
    refresh: bool,
) -> Result<()> {
    let data = fetch_data(refresh)?;
    let fmt = out.or(if json { Some(OutFormat::Json) } else { None });

    if list {
        let all = all_models(&data);
        let total = all.len();
        let take = limit.max(1);
        match fmt {
            Some(OutFormat::Json) => {
                let arr: Vec<Value> = all.into_iter().take(take).map(|(_, m)| m).collect();
                println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
            }
            Some(OutFormat::Csv) => {
                println!("{MODEL_CSV_HEADER}");
                for (pid, m) in all.iter().take(take) {
                    println!("{}", model_csv_row(&model_with_provider(pid, m)));
                }
            }
            None => {
                for (pid, m) in all.iter().take(take) {
                    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let nm = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    println!("{id}\t{nm}\t{pid}");
                }
                println!("--- {take} of {total} models (use --limit to show more)");
            }
        }
        return Ok(());
    }

    let query = match (name, search) {
        (Some(n), _) => n,
        (None, Some(s)) => s,
        (None, None) => bail!("provide a MODEL name, or use --search QUERY / --list"),
    };

    let hits = search_models(&data, query);
    if hits.is_empty() {
        bail!("no model matches `{query}` (source: {DATA_URL})");
    }

    // --search: always list the matching entries (grep-style).
    if search.is_some() {
        let take = limit.max(1);
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
                for (pid, m) in hits.iter().take(take) {
                    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let nm = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    println!("{id}\t{nm}\t{pid}");
                }
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
                for (pid, m) in &hits {
                    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let nm = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    println!("  {id}\t{nm}\t{pid}");
                }
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

/// Print a model object as aligned `label: value` lines. Known fields get
/// friendly formatting; unknown fields are appended verbatim.
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
}
