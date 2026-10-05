//! `sysenv ai chat` / `sysenv ai task` — chat with LLM providers configured in
//! `~/.sysenv/config.yaml` (default location; `-c/--config` overrides it).
//!
//! The config file follows the schema shown in `doc/config.yaml`:
//!
//! ```yaml
//! model: agnes-3.0-flash            # optional; see resolve_targets() for the
//!                                   # {provider}:{model} / {provider}:* / {model} forms
//! clients:                          # provider list
//!   - type: openai                  # openai (default) or anthropic
//!     name: agnes
//!     api_base: https://api.example.com/v1
//!     api_key: sk-xxxx
//!     models:
//!       - name: agnes-3.0-flash
//!         weight: 1                 # optional, default 1
//! tasks:                            # optional; used by `sysenv ai task`
//!   - name: weather
//!     desc: 获取天气信息
//!     msg: 我在{country:深圳},今天的天气如何
//! stream: true                      # optional, default false
//! ```
//!
//! `openai` clients talk to `{api_base}/chat/completions` with a Bearer key;
//! `anthropic` clients talk to `{api_base}/v1/messages` (or `{api_base}/messages`
//! when api_base already ends with `/v1`) with `x-api-key` / `anthropic-version`.

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

const DEFAULT_MAX_TOKENS: u64 = 1024;
const ANTHROPIC_VERSION: &str = "2023-06-01";
const STREAM_TIMEOUT: Duration = Duration::from_secs(600);
const USER_AGENT_STR: &str = concat!("sysenv/", env!("CARGO_PKG_VERSION"));

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Config {
    model: Option<String>,
    stream: bool,
    providers: Vec<Provider>,
    tasks: Vec<Task>,
}

#[derive(Debug, Clone)]
struct Provider {
    /// "openai" or "anthropic" (anything else defaults to openai)
    kind: String,
    name: String,
    api_base: String,
    api_key: String,
    models: Vec<Model>,
}

#[derive(Debug, Clone)]
struct Model {
    name: String,
    weight: u32,
    max_tokens: Option<u64>,
    /// Max input length (chars) enforced before sending; when absent it is
    /// filled from the models.dev `limit.context` of the first matching model
    /// and persisted back into the config file.
    max_input_tokens: Option<u64>,
    /// Modality type (e.g. `text,image`); filled from models.dev when absent.
    model_type: Option<String>,
}

#[derive(Debug, Clone)]
struct Task {
    name: String,
    desc: String,
    msg: String,
    /// Optional HTTP API URL template executed when the task is selected by
    /// function_call. `{key}` / `{key:default}` placeholders are filled from
    /// the model's tool arguments; the response body is fed back to the model
    /// as the tool result. Implemented in code (no local shell commands).
    api: Option<String>,
    /// Optional built-in search source executed when the task is selected:
    /// `zhihu` (daily news), `baidu` (hot search), `bilibili` (popular
    /// videos), `github` (trending repos), `hn` (Hacker News). The extracted
    /// headline list is fed back to the model as the tool result.
    search: Option<String>,
    /// Optional parameter names declared for the tool function; each becomes a
    /// string property of the function schema.
    params: Option<Vec<String>>,
}

/// Resolve the config path: explicit `-c` wins, otherwise `~/.sysenv/config.yaml`.
fn resolve_config_path(override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        let p = p.to_path_buf();
        if !p.exists() {
            bail!("config file not found: {} (see doc/config.yaml for the format)", p.display());
        }
        return Ok(p);
    }
    let home = home_dir().context("cannot locate the home directory")?;
    let p = home.join(".sysenv").join("config.yaml");
    if !p.exists() {
        bail!(
            "config file not found: {} (create it from doc/config.yaml: clients -> type/name/api_base/api_key/models)",
            p.display()
        );
    }
    Ok(p)
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let v = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let v = std::env::var_os("HOME");
    v.map(PathBuf::from)
}

/// Load and parse the config; returns the config plus the resolved path.
fn load_config(path: Option<&Path>) -> Result<(Config, PathBuf)> {
    let path = resolve_config_path(path)?;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read config `{}`", path.display()))?;
    let y = parse_yaml(&text)?;
    let cfg = config_from_yaml(&y).with_context(|| format!("invalid config `{}`", path.display()))?;
    Ok((cfg, path))
}

// --- minimal block-YAML parser (scalars / lists / maps, tab-tolerant) ------

#[derive(Debug, Clone, PartialEq)]
enum YVal {
    Scalar(String),
    List(Vec<YVal>),
    Map(Vec<(String, YVal)>),
}

impl YVal {
    fn get(&self, key: &str) -> Option<&YVal> {
        match self {
            YVal::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn scalar(&self) -> Option<&str> {
        match self {
            YVal::Scalar(s) => Some(s),
            _ => None,
        }
    }
    fn list(&self) -> Option<&[YVal]> {
        match self {
            YVal::List(l) => Some(l),
            _ => None,
        }
    }
}

struct Line {
    indent: usize,
    text: String,
}

fn preprocess_yaml(text: &str) -> Vec<Line> {
    // Strip a UTF-8 BOM so a leading `model:` key is not silently lost.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = Vec::new();
    for raw in text.lines() {
        // Tabs are illegal in YAML indentation but appear in real-world configs.
        let line = raw.replace('\t', " ");
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let content = match line.find(" #") {
            Some(i) => &line[..i],
            None => line.as_str(),
        };
        let content = content.trim_end();
        if content.trim().is_empty() {
            continue;
        }
        let indent = content.len() - content.trim_start().len();
        out.push(Line { indent, text: content.trim_start().to_string() });
    }
    out
}

fn split_kv_opt(line: &str) -> Option<(String, String)> {
    let i = line.find(':')?;
    let key = line[..i].trim();
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), line[i + 1..].trim().to_string()))
}

fn split_kv(line: &str) -> Result<(String, String)> {
    split_kv_opt(line).ok_or_else(|| anyhow::anyhow!("invalid config line `{line}` (expected `key: value`)"))
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() >= 2 && ((b[0] == b'"' && b[s.len() - 1] == b'"') || (b[0] == b'\'' && b[s.len() - 1] == b'\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Parse a scalar value, or an inline list like `[a, b, c]` into a YVal list.
fn parse_scalar_or_list(value: &str) -> YVal {
    let v = value.trim();
    if v.len() >= 2 && v.starts_with('[') && v.ends_with(']') {
        let inner = &v[1..v.len() - 1];
        if inner.trim().is_empty() {
            YVal::List(Vec::new())
        } else {
            let items = inner
                .split(',')
                .map(|s| YVal::Scalar(unquote(s.trim())))
                .collect();
            YVal::List(items)
        }
    } else {
        YVal::Scalar(unquote(v))
    }
}

fn parse_yaml(text: &str) -> Result<YVal> {
    let lines = preprocess_yaml(text);
    let (v, next) = parse_block(&lines, 0, 0)?;
    if next != lines.len() {
        bail!("unexpected content at line {}", next + 1);
    }
    Ok(v)
}

fn parse_block(lines: &[Line], idx: usize, min_indent: usize) -> Result<(YVal, usize)> {
    if idx >= lines.len() || lines[idx].indent < min_indent {
        return Ok((YVal::Scalar(String::new()), idx));
    }
    if lines[idx].text.starts_with('-') {
        let mut items = Vec::new();
        let mut i = idx;
        while i < lines.len() && lines[i].indent == min_indent && lines[i].text.starts_with('-') {
            let rest = lines[i].text[1..].trim_start().to_string();
            let (item, ni) = parse_list_item(lines, i, min_indent, &rest)?;
            items.push(item);
            i = ni;
        }
        Ok((YVal::List(items), i))
    } else {
        let mut map = Vec::new();
        let mut i = idx;
        while i < lines.len() && lines[i].indent == min_indent && !lines[i].text.starts_with('-') {
            let (key, value) = split_kv(&lines[i].text)?;
            let (val, next_i) = if value.is_empty() {
                if i + 1 < lines.len() && lines[i + 1].indent > min_indent {
                    let (v, ni) = parse_block(lines, i + 1, lines[i + 1].indent)?;
                    (v, ni)
                } else {
                    (YVal::Scalar(String::new()), i + 1)
                }
            } else {
                (parse_scalar_or_list(&value), i + 1)
            };
            map.push((key, val));
            i = next_i;
        }
        Ok((YVal::Map(map), i))
    }
}

fn parse_list_item(lines: &[Line], idx: usize, indent: usize, rest: &str) -> Result<(YVal, usize)> {
    if let Some((key, value)) = split_kv_opt(rest) {
        let mut map = vec![(
            key,
            if value.is_empty() {
                YVal::Scalar(String::new())
            } else {
                YVal::Scalar(unquote(&value))
            },
        )];
        let mut i = idx + 1;
        while i < lines.len() && lines[i].indent > indent && !lines[i].text.starts_with('-') {
            let (k2, v2) = split_kv(&lines[i].text)?;
            let (val, next_i) = if v2.is_empty() {
                if i + 1 < lines.len() && lines[i + 1].indent > lines[i].indent {
                    let (v, ni) = parse_block(lines, i + 1, lines[i + 1].indent)?;
                    (v, ni)
                } else {
                    (YVal::Scalar(String::new()), i + 1)
                }
            } else {
                (parse_scalar_or_list(&v2), i + 1)
            };
            map.push((k2, val));
            i = next_i;
        }
        Ok((YVal::Map(map), i))
    } else {
        Ok((YVal::Scalar(unquote(rest)), idx + 1))
    }
}

// --- config -> typed structs -------------------------------------------------

fn normalize_kind(k: &str) -> String {
    match k.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => "anthropic".to_string(),
        // "open", "openai" and anything unknown default to the OpenAI format
        _ => "openai".to_string(),
    }
}

fn provider_from_yaml(c: &YVal) -> Result<Provider> {
    let name = c.get("name").and_then(|v| v.scalar()).map(str::to_string).unwrap_or_default();
    let api_base = c.get("api_base").and_then(|v| v.scalar()).map(str::to_string).unwrap_or_default();
    let api_key = c.get("api_key").and_then(|v| v.scalar()).map(str::to_string).unwrap_or_default();
    let kind = normalize_kind(c.get("type").and_then(|v| v.scalar()).unwrap_or("openai"));
    if name.is_empty() {
        bail!("a client is missing `name`");
    }
    if api_base.is_empty() {
        bail!("client `{name}` is missing api_base");
    }
    if api_key.is_empty() {
        bail!("client `{name}` is missing api_key");
    }
    let mut models: Vec<Model> = Vec::new();
    let mut last: Option<usize> = None;
    if let Some(list) = c.get("models").and_then(|v| v.list()) {
        for item in list {
            if let Some(mname) = item.get("name").and_then(|v| v.scalar()) {
                let weight = item
                    .get("weight")
                    .and_then(|v| v.scalar())
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(1)
                    .min(9);
                let max_tokens = item
                    .get("max_tokens")
                    .and_then(|v| v.scalar())
                    .and_then(|s| s.parse::<u64>().ok());
                let max_input_tokens = item
                    .get("max_input_tokens")
                    .and_then(|v| v.scalar())
                    .and_then(|s| s.parse::<u64>().ok());
                let model_type = item
                    .get("type")
                    .and_then(|v| v.scalar())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                models.push(Model {
                    name: mname.to_string(),
                    weight,
                    max_tokens,
                    max_input_tokens,
                    model_type,
                });
                last = Some(models.len() - 1);
            } else if let Some(w) = item.get("weight").and_then(|v| v.scalar()) {
                // Tolerate a stray "- weight: N" item that was meant as the
                // weight of the preceding model entry.
                if let (Ok(w), Some(li)) = (w.parse::<u32>(), last) {
                    models[li].weight = w.min(9);
                }
            }
        }
    }
    if models.is_empty() {
        bail!("client `{name}` has no models");
    }
    Ok(Provider { kind, name, api_base, api_key, models })
}

fn config_from_yaml(y: &YVal) -> Result<Config> {
    let model = y
        .get("model")
        .and_then(|v| v.scalar())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let stream = y
        .get("stream")
        .and_then(|v| v.scalar())
        .map(|s| matches!(s.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on"))
        .unwrap_or(false);
    let mut providers = Vec::new();
    if let Some(clients) = y.get("clients").and_then(|v| v.list()) {
        for c in clients {
            providers.push(provider_from_yaml(c)?);
        }
    }
    let mut tasks = Vec::new();
    if let Some(list) = y.get("tasks").and_then(|v| v.list()) {
        for t in list {
            let name = t.get("name").and_then(|v| v.scalar()).map(str::to_string).unwrap_or_default();
            let desc = t.get("desc").and_then(|v| v.scalar()).unwrap_or("").to_string();
            let msg = t.get("msg").and_then(|v| v.scalar()).unwrap_or("").to_string();
            let api = t.get("api").and_then(|v| v.scalar()).map(str::to_string);
            let search = t.get("search").and_then(|v| v.scalar()).map(str::to_string);
            let params = t.get("params").and_then(|v| v.list()).map(|l| {
                l.iter()
                    .filter_map(|p| p.scalar().map(str::to_string))
                    .collect()
            });
            tasks.push(Task { name, desc, msg, api, search, params });
        }
    }
    Ok(Config { model, stream, providers, tasks })
}

// ---------------------------------------------------------------------------
// Model selection (top-level `model`)
// ---------------------------------------------------------------------------

fn ci_eq(a: &str, b: &str) -> bool {
    a.to_ascii_lowercase() == b.to_ascii_lowercase()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Target {
    provider: usize,
    model: usize,
}

fn find_provider_idx(cfg: &Config, name: &str) -> Result<usize> {
    cfg.providers
        .iter()
        .position(|p| ci_eq(&p.name, name))
        .ok_or_else(|| {
            let avail: Vec<&str> = cfg.providers.iter().map(|p| p.name.as_str()).collect();
            anyhow::anyhow!("no provider named `{name}` (available: {})", avail.join(", "))
        })
}

/// Split a model selector into parts on half-width (`,`) or full-width
/// (`，`) commas. Empty parts are dropped so `a,,b` / `a，b` both work.
fn split_selector(s: &str) -> Vec<String> {
    s.split([',', '，'])
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Resolve a model selector to one or more (provider, model) targets:
/// - missing / empty   -> the first model of the first provider
/// - `prov:model`      -> one matching model of one provider
/// - `prov:*`          -> every model of that provider (weighted round-robin)
/// - `model`           -> every matching model across all providers (weighted round-robin)
///
/// The selector may contain several comma-separated (half- or full-width)
/// selectors, each resolved by the rules above; the union is returned.
fn resolve_targets(cfg: &Config, selector: &str) -> Result<Vec<Target>> {
    // `<default>` is the sentinel for "no selector configured at all".
    let parts = split_selector(selector);
    if parts.is_empty() || (parts.len() == 1 && parts[0] == "<default>") {
        let p = cfg
            .providers
            .first()
            .ok_or_else(|| anyhow::anyhow!("no providers configured under `clients`"))?;
        if p.models.is_empty() {
            bail!("provider `{}` has no models", p.name);
        }
        return Ok(vec![Target { provider: 0, model: 0 }]);
    }
    let mut out: Vec<Target> = Vec::new();
    for sel in parts {
        if let Some((prov, m)) = sel.split_once(':') {
            let prov = prov.trim();
            let m = m.trim();
            if m == "*" {
                let pi = find_provider_idx(cfg, prov)?;
                let n = cfg.providers[pi].models.len();
                if n == 0 {
                    bail!("provider `{prov}` has no models");
                }
                out.extend((0..n).map(|mi| Target { provider: pi, model: mi }));
            } else {
                let pi = find_provider_idx(cfg, prov)?;
                let mi = cfg.providers[pi]
                    .models
                    .iter()
                    .position(|mdl| ci_eq(&mdl.name, m))
                    .ok_or_else(|| anyhow::anyhow!("provider `{prov}` has no model `{m}`"))?;
                out.push(Target { provider: pi, model: mi });
            }
        } else {
            for (pi, p) in cfg.providers.iter().enumerate() {
                for (mi, m) in p.models.iter().enumerate() {
                    if ci_eq(&m.name, &sel) {
                        out.push(Target { provider: pi, model: mi });
                    }
                }
            }
            if !out.iter().any(|t| {
                cfg.providers[t.provider].models[t.model].name.eq_ignore_ascii_case(&sel)
            }) {
                let avail: Vec<String> = cfg
                    .providers
                    .iter()
                    .flat_map(|p| p.models.iter().map(|m| format!("{}/{}", p.name, m.name)))
                    .collect();
                bail!("no model named `{sel}` in any provider (available: {})", avail.join(", "));
            }
        }
    }
    // De-duplicate identical (provider, model) pairs from overlapping
    // selectors, keeping the original resolution order.
    let mut seen: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    out.retain(|t| seen.insert((t.provider, t.model)));
    Ok(out)
}

// --- weighted round-robin with persisted state ------------------------------

#[derive(Serialize, Deserialize, Default)]
struct WrrStore {
    selectors: HashMap<String, WrrState>,
}

#[derive(Serialize, Deserialize)]
struct WrrState {
    keys: Vec<String>,
    current: Vec<f64>,
}

fn wrr_state_file(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("chat_state.json")
}

/// Smooth weighted round-robin (nginx style) over the candidate targets.
/// The rotation state persists in `chat_state.json` next to the config so that
/// successive invocations keep rotating. Weights are the configured 0-9 values;
/// a weight of 0 means the model is not picked unless every candidate is 0
/// (in which case all are treated as weight 1). Returns the selected target
/// index into `targets`.
fn pick_weighted(cfg: &Config, targets: &[Target], selector: &str, config_path: &Path) -> Result<usize> {
    if targets.len() == 1 {
        return Ok(0);
    }
    let raw_weights: Vec<f64> = targets
        .iter()
        .map(|t| cfg.providers[t.provider].models[t.model].weight as f64)
        .collect();
    let total_raw: f64 = raw_weights.iter().sum();
    // All-zero weights (or weights that sum to 0) degenerate to equal weights.
    let weights: Vec<f64> = if total_raw <= 0.0 {
        vec![1.0; raw_weights.len()]
    } else {
        raw_weights
    };
    let total: f64 = weights.iter().sum();
    let keys: Vec<String> = targets
        .iter()
        .map(|t| format!("{}:{}", cfg.providers[t.provider].name, cfg.providers[t.provider].models[t.model].name))
        .collect();
    let key = format!("{}|{}", config_path.display(), selector);

    let state_file = wrr_state_file(config_path);
    let mut store: WrrStore = std::fs::read_to_string(&state_file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let changed = store
        .selectors
        .get(&key)
        .map(|st| st.keys != keys)
        .unwrap_or(true);
    if changed {
        store.selectors.insert(
            key.clone(),
            WrrState { keys: keys.clone(), current: vec![0.0; keys.len()] },
        );
    }
    {
        let st = store.selectors.get_mut(&key).unwrap();
        for (i, w) in weights.iter().enumerate() {
            st.current[i] += w;
        }
    }
    let pick = {
        let st = store.selectors.get(&key).unwrap();
        let mut best = 0usize;
        for i in 1..st.current.len() {
            if st.current[i] > st.current[best] {
                best = i;
            }
        }
        best
    };
    store.selectors.get_mut(&key).unwrap().current[pick] -= total;
    if let Some(dir) = state_file.parent() {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&state_file, serde_json::to_string(&store).unwrap_or_default());
    }
    Ok(pick)
}

// ---------------------------------------------------------------------------
// Persisting weight adjustments back into the YAML config file
// ---------------------------------------------------------------------------

/// Split one raw config line into (leading-space indent, de-commented content).
fn line_parts(line: &str) -> (usize, String) {
    let expanded = line.replace('\t', " ");
    let trimmed = expanded.trim();
    if trimmed.is_empty() {
        return (0, String::new());
    }
    let content = match expanded.find(" #") {
        Some(i) => &expanded[..i],
        None => expanded.as_str(),
    };
    let content = content.trim_end();
    let indent = content.len() - content.trim_start().len();
    (indent, content.trim_start().to_string())
}

/// Replace the numeric value of a `weight:` (or `- weight:`) line, preserving
/// indentation, spacing and any trailing comment.
#[allow(dead_code)] // exercised by unit tests
fn replace_weight_value(line: &str, new_value: u32) -> String {
    replace_scalar_value(line, &new_value.to_string())
}

/// Replace the scalar value of the first `key:` occurrence in `line`, keeping
/// the key prefix, one space, the new value, and everything after the value
/// (e.g. a trailing comment). Works for both `key: N` and `- key: N` lines.
fn replace_scalar_value(line: &str, new_value: &str) -> String {
    let idx = match line.find(':') {
        Some(i) => i,
        None => return line.to_string(),
    };
    let prefix = &line[..=idx];
    let rest = &line[idx + 1..];
    let value_start = match rest.find(|c: char| !c.is_whitespace()) {
        Some(i) => i,
        None => return line.to_string(),
    };
    let value_end = rest[value_start..]
        .find(|c: char| c.is_whitespace())
        .map(|i| value_start + i)
        .unwrap_or(rest.len());
    if value_end == value_start {
        return line.to_string();
    }
    format!("{prefix} {new_value}{}", &rest[value_end..])
}

/// Update the `weight` of `model_name` under `provider_name` in the YAML
/// config file, writing the change back in place (comments and layout are
/// preserved). Accepts both the conventional form (`weight: N` as a sub-key of
/// the model entry) and the mangled form (`- weight: N` as a separate list
/// item right after `- name:`). When no `weight:` line exists at all, a new
/// `weight:` sub-key is inserted after the `- name:` line.
fn update_weight_in_config(config_path: &Path, provider_name: &str, model_name: &str, new_weight: u32) -> Result<()> {
    update_model_field_in_config(config_path, provider_name, model_name, "weight", &new_weight.to_string())
}

/// Update a scalar field `key` of `model_name` under `provider_name` in the
/// YAML config file, writing the change back in place (comments and layout are
/// preserved). For `key == "weight"` the mangled form (`- weight: N` as a
/// separate list item right after `- name:`) is also handled. When no such key
/// exists in the model entry, a new `key: value` sub-key is inserted after the
/// `- name:` line.
fn update_model_field_in_config(
    config_path: &Path,
    provider_name: &str,
    model_name: &str,
    key: &str,
    value: &str,
) -> Result<()> {
    let text = std::fs::read_to_string(config_path)
        .with_context(|| format!("cannot read config `{}`", config_path.display()))?;
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();

    let mut provider_indent: Option<usize> = None;
    let mut cur_provider: Option<String> = None;
    let mut in_models = false;
    let mut models_indent = 0usize;

    let mut done = false;
    let mut i = 0usize;
    while i < lines.len() && !done {
        let (indent, content) = line_parts(&lines[i]);
        if content.is_empty() {
            i += 1;
            continue;
        }
        let is_dash = content.starts_with('-');

        // Leaving the current provider block entirely.
        if let Some(pi) = provider_indent {
            if indent < pi {
                provider_indent = None;
                cur_provider = None;
                in_models = false;
            }
        }

        if is_dash {
            let rest = content[1..].trim_start();
            if let Some(pi) = provider_indent {
                if indent == pi {
                    // A new sibling provider entry at the same indent.
                    provider_indent = Some(indent);
                    cur_provider = None;
                    in_models = false;
                    if let Some((k, v)) = split_kv_opt(rest) {
                        if k == "name" {
                            cur_provider = Some(unquote(&v));
                        }
                    }
                    i += 1;
                    continue;
                }
            } else {
                // The first `- ` entry we meet is a provider under `clients:`.
                provider_indent = Some(indent);
                cur_provider = None;
                in_models = false;
                if let Some((k, v)) = split_kv_opt(rest) {
                    if k == "name" {
                        cur_provider = Some(unquote(&v));
                    }
                }
                i += 1;
                continue;
            }
            if in_models && indent > models_indent {
                // A model entry inside the models list.
                if let Some((k, v)) = split_kv_opt(rest) {
                    if k == "name" {
                        let mname = unquote(&v);
                        let provider_ok = cur_provider
                            .as_deref()
                            .map(|p| ci_eq(p, provider_name))
                            .unwrap_or(false);
                        if provider_ok && ci_eq(&mname, model_name) {
                            let name_indent = indent;
                            // Mangled form (weight only): the very next line is
                            // `- weight: N` at the same indent.
                            if key == "weight" && i + 1 < lines.len() {
                                let (ni, nc) = line_parts(&lines[i + 1]);
                                if nc.starts_with("- weight:") && ni == name_indent {
                                    lines[i + 1] = replace_scalar_value(&lines[i + 1], value);
                                    done = true;
                                    break;
                                }
                            }
                            // Conventional form: a deeper `key:` sub-key
                            // anywhere inside this model entry.
                            let marker = format!("{key}:");
                            let mut j = i + 1;
                            while j < lines.len() {
                                let (nj, nc) = line_parts(&lines[j]);
                                if nc.is_empty() {
                                    j += 1;
                                    continue;
                                }
                                if nj <= name_indent {
                                    break;
                                }
                                if nc.starts_with(&marker) {
                                    lines[j] = replace_scalar_value(&lines[j], value);
                                    done = true;
                                    break;
                                }
                                j += 1;
                            }
                            if !done {
                                // No such key: insert one after the name line.
                                let pad = " ".repeat(name_indent + 2);
                                lines.insert(i + 1, format!("{pad}{key}: {value}"));
                                done = true;
                                break;
                            }
                        }
                    }
                }
                i += 1;
                continue;
            }
            i += 1;
            continue;
        }

        // Plain `key: value` line inside a provider entry.
        if let Some((k, v)) = split_kv_opt(&content) {
            if provider_indent.is_some() {
                if !in_models {
                    if k == "name" {
                        cur_provider = Some(unquote(&v));
                    } else if k == "models" {
                        in_models = true;
                        models_indent = indent;
                    }
                }
            }
        }
        i += 1;
    }

    if !done {
        bail!("cannot locate model `{model_name}` of provider `{provider_name}` in `{}`", config_path.display());
    }
    std::fs::write(config_path, lines.join("\n"))
        .with_context(|| format!("cannot write config `{}`", config_path.display()))
}

// ---------------------------------------------------------------------------
// Chat requests (OpenAI / Anthropic)
// ---------------------------------------------------------------------------

fn chat_url(p: &Provider) -> String {
    let base = p.api_base.trim_end_matches('/');
    if p.kind == "anthropic" {
        if base.ends_with("/v1") {
            format!("{base}/messages")
        } else {
            format!("{base}/v1/messages")
        }
    } else {
        format!("{base}/chat/completions")
    }
}

fn chat_body(p: &Provider, m: &Model, msg: &str, stream: bool) -> Value {
    let messages = json!([{ "role": "user", "content": msg }]);
    let mut body = if p.kind == "anthropic" {
        json!({
            "model": m.name,
            "max_tokens": m.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            "messages": messages,
        })
    } else {
        json!({
            "model": m.name,
            "messages": messages,
        })
    };
    if stream {
        body["stream"] = json!(true);
    }
    if p.kind != "anthropic" {
        if let Some(mt) = m.max_tokens {
            body["max_tokens"] = json!(mt);
        }
    }
    body
}

fn pretty_json_or_raw(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            if let Ok(pretty) = serde_json::to_string_pretty(&v) {
                return pretty;
            }
        }
    }
    text.into_owned()
}

fn debug_print_request(url: &str, headers: &HeaderMap, body: &[u8]) {
    eprintln!("# request");
    eprintln!("POST {url}");
    for (n, v) in headers.iter() {
        eprintln!("{n}: {}", v.to_str().unwrap_or("<binary>"));
    }
    if !body.is_empty() {
        eprintln!();
        eprintln!("{}", pretty_json_or_raw(body));
    }
    eprintln!();
}

fn debug_print_response(status: reqwest::StatusCode, headers: &HeaderMap, body: &str) {
    eprintln!("# response");
    eprintln!("HTTP {} {}", status.as_u16(), status.canonical_reason().unwrap_or(""));
    for (n, v) in headers.iter() {
        eprintln!("{n}: {}", v.to_str().unwrap_or("<binary>"));
    }
    if !body.is_empty() {
        eprintln!();
        eprintln!("{}", pretty_json_or_raw(body.as_bytes()));
    }
    eprintln!();
}

/// Extract the assistant text from a non-streaming response body.
fn extract_content(body: &str, kind: &str) -> Result<String> {
    let v: Value =
        serde_json::from_str(body).with_context(|| format!("provider returned invalid JSON: {}", body.chars().take(200).collect::<String>()))?;
    if let Some(err) = v.get("error") {
        let raw = err.to_string();
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or(&raw);
        bail!("provider error: {msg}");
    }
    match kind {
        "anthropic" => {
            let mut text = String::new();
            if let Some(arr) = v.get("content").and_then(|c| c.as_array()) {
                for block in arr {
                    if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                        text.push_str(t);
                    }
                }
            }
            if text.is_empty() {
                bail!("anthropic response contained no text content");
            }
            Ok(text)
        }
        _ => {
            let choice = v
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|a| a.first())
                .context("openai response missing choices")?;
            let content = choice
                .get("message")
                .and_then(|m| m.get("content"))
                .context("openai response missing message.content")?;
            match content {
                Value::String(s) => Ok(s.clone()),
                Value::Array(parts) => {
                    let mut text = String::new();
                    for p in parts {
                        if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                            text.push_str(t);
                        }
                    }
                    Ok(text)
                }
                Value::Null => Ok(String::new()),
                other => Ok(other.to_string()),
            }
        }
    }
}

/// Apply one streaming delta (printed to stdout as it arrives).
fn stream_delta(kind: &str, v: &Value, out: &mut String) -> Result<()> {
    if let Some(err) = v.get("error") {
        let raw = err.to_string();
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or(&raw);
        bail!("provider error: {msg}");
    }
    let text = if kind == "anthropic" {
        if v.get("type").and_then(|t| t.as_str()) == Some("content_block_delta") {
            v.pointer("/delta/text").and_then(|t| t.as_str()).unwrap_or("").to_string()
        } else {
            String::new()
        }
    } else {
        match v.pointer("/choices/0/delta/content") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(parts)) => parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<String>(),
            _ => String::new(),
        }
    };
    if !text.is_empty() {
        print!("{text}");
        out.push_str(&text);
        let _ = std::io::stdout().flush();
    }
    Ok(())
}

/// Read an SSE stream, printing the deltas; falls back to a plain JSON body
/// when the response is not SSE.
fn stream_response(resp: reqwest::blocking::Response, kind: &str) -> Result<String> {
    let mut reader = std::io::BufReader::new(resp);
    let mut line = String::new();
    let mut full = String::new();
    let mut first = true;
    loop {
        line.clear();
        let n = reader.read_line(&mut line).context("failed reading response stream")?;
        if n == 0 {
            break;
        }
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if first {
            first = false;
            if !t.starts_with("data:") && !t.starts_with("event:") {
                let mut rest = String::new();
                let _ = reader.read_to_string(&mut rest);
                return extract_content(&format!("{t}\n{rest}"), kind);
            }
        }
        if let Some(d) = t.strip_prefix("data:") {
            let d = d.trim();
            if d == "[DONE]" {
                break;
            }
            if let Ok(v) = serde_json::from_str::<Value>(d) {
                stream_delta(kind, &v, &mut full)?;
            }
        }
    }
    println!();
    Ok(full)
}

/// Send one chat request and return the raw response body text. When streaming
/// is active (and not in debug mode) the deltas are printed and the full text
/// is returned.
fn send_chat_raw(p: &Provider, body: &Value, stream: bool, debug: bool) -> Result<String> {
    let url = chat_url(p);
    let body_bytes = serde_json::to_vec(body).context("cannot serialize request body")?;

    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if p.kind == "anthropic" {
        headers.insert("x-api-key", HeaderValue::from_str(&p.api_key).context("invalid api_key")?);
        headers.insert("anthropic-version", HeaderValue::from_static(ANTHROPIC_VERSION));
    } else {
        let auth = format!("Bearer {}", p.api_key);
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&auth).context("invalid api_key")?);
    }
    if stream {
        headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    }

    if debug {
        debug_print_request(&url, &headers, &body_bytes);
    }

    let client = Client::builder()
        .timeout(STREAM_TIMEOUT)
        .build()
        .context("failed to build HTTP client")?;
    let resp = client
        .post(&url)
        .headers(headers)
        .body(body_bytes.clone())
        .send()
        .with_context(|| format!("request to {url} failed"))?;
    let status = resp.status();
    let resp_headers = resp.headers().clone();

    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        if debug {
            debug_print_response(status, &resp_headers, &text);
        }
        let snippet: String = text.chars().take(500).collect();
        bail!("{url} returned HTTP {status}: {snippet}");
    }

    if stream && !debug {
        return stream_response(resp, &p.kind);
    }

    let text = resp.text().context("cannot read response body")?;
    if debug {
        debug_print_response(status, &resp_headers, &text);
    }
    Ok(text)
}

/// Send one chat request. Returns the assistant text (streaming prints as it goes).
fn send_chat(p: &Provider, m: &Model, msg: &str, stream: bool, debug: bool) -> Result<String> {
    let body = chat_body(p, m, msg, stream);
    let text = send_chat_raw(p, &body, stream, debug)?;
    if stream && !debug {
        return Ok(text);
    }
    extract_content(&text, &p.kind)
}

// ---------------------------------------------------------------------------
// Task templates
// ---------------------------------------------------------------------------

/// Every configured task becomes one tool; the task name is the function name
/// and the task description is the function description. The `input` parameter
/// carries the user's request; an optional `params` list declares additional
/// string parameters for the tool command.
fn task_tools(cfg: &Config, kind: &str) -> Result<Value> {
    let mut arr = Vec::new();
    for t in &cfg.tasks {
        if t.name.is_empty()
            || !t
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            bail!(
                "task name `{}` is not a valid function name (use letters, digits, '_' or '-')",
                t.name
            );
        }
        let mut props = serde_json::Map::new();
        props.insert(
            "input".to_string(),
            json!({ "type": "string", "description": "用户请求的内容" }),
        );
        if let Some(params) = &t.params {
            for p in params {
                props.insert(p.clone(), json!({ "type": "string", "description": format!("参数 {p}") }));
            }
        }
        let schema = json!({
            "type": "object",
            "properties": props,
            "required": ["input"]
        });
        if kind == "anthropic" {
            arr.push(json!({
                "name": t.name,
                "description": t.desc,
                "input_schema": schema,
            }));
        } else {
            arr.push(json!({
                "type": "function",
                "function": { "name": t.name, "description": t.desc, "parameters": schema },
            }));
        }
    }
    Ok(json!(arr))
}

const TOOL_SYSTEM: &str = "你是任务路由助手。根据用户的请求，从提供的任务函数中选择最合适的一个并调用。每个函数的 description 说明了它的用途。调用时把用户请求的关键信息放入 input 参数。只调用与请求相关的任务，不要编造不存在的任务。";

/// Chat request body with the task tools attached.
fn tool_chat_body(p: &Provider, m: &Model, input: &str, system: &str, tools: &Value, stream: bool) -> Value {
    let mut body = if p.kind == "anthropic" {
        json!({
            "model": m.name,
            "max_tokens": m.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            "system": system,
            "messages": [{"role": "user", "content": input}],
            "tools": tools,
        })
    } else {
        json!({
            "model": m.name,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": input},
            ],
            "tools": tools,
        })
    };
    if stream {
        body["stream"] = json!(true);
    }
    if p.kind != "anthropic" {
        if let Some(mt) = m.max_tokens {
            body["max_tokens"] = json!(mt);
        }
    }
    body
}

/// Parse the tool calls out of a non-streaming response body.
///
/// OpenAI: `choices[0].message.tool_calls[].function.{name,arguments}`;
/// Anthropic: `content[]` blocks with `type == "tool_use"` (`name` / `input`).
/// Returns an empty vector when the model replied with plain text instead.
fn extract_tool_calls(body: &str, kind: &str) -> Result<Vec<(String, Value)>> {
    let v: Value = serde_json::from_str(body).with_context(|| {
        format!(
            "provider returned invalid JSON: {}",
            body.chars().take(200).collect::<String>()
        )
    })?;
    if let Some(err) = v.get("error") {
        let raw = err.to_string();
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or(&raw);
        bail!("provider error: {msg}");
    }
    let mut calls = Vec::new();
    if kind == "anthropic" {
        if let Some(arr) = v.get("content").and_then(|c| c.as_array()) {
            for block in arr {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    if let (Some(name), Some(input)) = (
                        block.get("name").and_then(|n| n.as_str()),
                        block.get("input"),
                    ) {
                        calls.push((name.to_string(), input.clone()));
                    }
                }
            }
        }
    } else if let Some(tc) = v.pointer("/choices/0/message/tool_calls").and_then(|c| c.as_array()) {
        for call in tc {
            let fname = call.pointer("/function/name").and_then(|n| n.as_str());
            let args = call.pointer("/function/arguments").and_then(|a| a.as_str());
            if let Some(name) = fname {
                let parsed = args
                    .and_then(|a| serde_json::from_str::<Value>(a).ok())
                    .unwrap_or_else(|| {
                        args.map(|a| Value::String(a.to_string()))
                            .unwrap_or(Value::Null)
                    });
                calls.push((name.to_string(), parsed));
            }
        }
    }
    Ok(calls)
}

/// Ask the configured LLM to pick the task(s) matching the user's request via
/// function calls. Weighted fallback across models works like `chat_once`.
fn route_tasks(cfg: &mut Config, path: &Path, input: &str, debug: bool) -> Result<Vec<(String, Value)>> {
    let selector = cfg.model.as_deref().unwrap_or("<default>");
    let targets = resolve_targets(cfg, selector)?;
    if targets.is_empty() {
        bail!("no model targets to chat with (selector `{selector}` matched nothing)");
    }

    let first = pick_weighted(cfg, &targets, selector, path)?;
    let mut order: Vec<usize> = Vec::with_capacity(targets.len());
    order.push(first);
    for i in 0..targets.len() {
        if i != first {
            order.push(i);
        }
    }

    let mut had_failure = false;
    let mut errors: Vec<String> = Vec::new();
    for ti in order {
        let t = &targets[ti];
        ensure_model_capabilities(cfg, path, t.provider, t.model);
        let p = &cfg.providers[t.provider];
        let m = &p.models[t.model];
        eprintln!("sysenv: using {} / {} ({})", p.name, m.name, p.kind);
        let tools = task_tools(cfg, &p.kind)?;
        let body = tool_chat_body(p, m, input, TOOL_SYSTEM, &tools, false);
        match send_chat_raw(p, &body, false, debug) {
            Ok(text) => match extract_tool_calls(&text, &p.kind) {
                Ok(calls) => {
                    if had_failure && m.weight < 9 {
                        if let Err(e) = update_weight_in_config(path, &p.name, &m.name, m.weight + 1) {
                            eprintln!(
                                "sysenv: warning: failed to persist weight bump for {} / {}: {e:#}",
                                p.name, m.name
                            );
                        }
                    }
                    return Ok(calls);
                }
                Err(e) => {
                    errors.push(format!("{} / {}: {e:#}", p.name, m.name));
                    if m.weight > 1 {
                        if let Err(e2) = update_weight_in_config(path, &p.name, &m.name, m.weight - 1) {
                            eprintln!(
                                "sysenv: warning: failed to persist weight drop for {} / {}: {e2:#}",
                                p.name, m.name
                            );
                        }
                    }
                    had_failure = true;
                }
            },
            Err(e) => {
                errors.push(format!("{} / {}: {e:#}", p.name, m.name));
                if m.weight > 1 {
                    if let Err(e2) = update_weight_in_config(path, &p.name, &m.name, m.weight - 1) {
                        eprintln!(
                            "sysenv: warning: failed to persist weight drop for {} / {}: {e2:#}",
                            p.name, m.name
                        );
                    }
                }
                had_failure = true;
            }
        }
    }
    bail!("all {} model(s) failed: {}", targets.len(), errors.join(" | "));
}

/// Probe stdin without hanging: wait up to `timeout` for EOF.
fn try_read_stdin(timeout: Duration) -> Option<String> {
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::stdin().read_to_string(&mut s);
        let _ = tx.send(s);
    });
    match rx.recv_timeout(timeout) {
        Ok(s) if s.trim().is_empty() => None,
        Ok(s) => Some(s),
        Err(_) => None,
    }
}

/// Format a list of `(title, url)` pairs as a plain text list for the model.
fn fmt_headlines(rows: &[(String, String)]) -> String {
    rows.iter()
        .map(|(t, u)| format!("- {t} ({u})"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Strip HTML tags and collapse whitespace inside a text fragment.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Decode the common HTML entities (named + numeric) found in scraped pages.
fn html_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi) = s[i..].find(';') {
                let ent = &s[i + 1..i + semi];
                let decoded = match ent {
                    "amp" => Some("&".to_string()),
                    "lt" => Some("<".to_string()),
                    "gt" => Some(">".to_string()),
                    "quot" => Some("\"".to_string()),
                    "apos" | "#39" | "#x27" => Some("'".to_string()),
                    _ => {
                        if let Some(hex) = ent.strip_prefix("#x") {
                            u32::from_str_radix(hex, 16).ok().and_then(|c| char::from_u32(c)).map(|c| c.to_string())
                        } else if let Some(dec) = ent.strip_prefix('#') {
                            dec.parse::<u32>().ok().and_then(|c| char::from_u32(c)).map(|c| c.to_string())
                        } else {
                            None
                        }
                    }
                };
                if let Some(d) = decoded {
                    out.push_str(&d);
                    i += semi + 1;
                    continue;
                }
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Percent-encode a string for use in a query parameter.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Browser User-Agent used for sites that reject bare client UAs.
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

/// Parse a toutiao hot-news feed response (`category=news_hot`) into rows.
fn parse_toutiao(body: &str) -> Result<Vec<(String, String)>> {
    let v: Value = serde_json::from_str(body).context("toutiao returned invalid JSON")?;
    let mut rows = Vec::new();
    if let Some(data) = v.get("data").and_then(|d| d.as_array()) {
        for it in data {
            let t = it.get("title").and_then(|x| x.as_str());
            let u = it.get("source_url").and_then(|x| x.as_str()).unwrap_or("");
            if let Some(t) = t {
                let url = if u.starts_with("http") {
                    u.to_string()
                } else if u.starts_with('/') {
                    format!("https://www.toutiao.com{u}")
                } else {
                    "https://www.toutiao.com/".to_string()
                };
                rows.push((t.to_string(), url));
            }
        }
    }
    Ok(rows)
}

/// Parse a tophub.today board page into rows (`<span class="t">` titles + href).
fn parse_tophub(body: &str) -> Result<Vec<(String, String)>> {
    let mut rows = Vec::new();
    let mut pos = 0;
    while let Some(start) = body[pos..].find("cc-cd-cb-ll") {
        // Back up to the enclosing `<a href=...>` so the URL is captured.
        let a_pos = body[..pos + start].rfind("<a ");
        let begin = a_pos.unwrap_or_else(|| body[..pos + start].rfind('<').unwrap_or(pos + start));
        let end = body[pos + start..].find("</a>").map(|i| pos + start + i).unwrap_or(body.len());
        let block = &body[begin..end];
        if let Some(href) = block.find("href=\"") {
            let after = &block[href + 6..];
            if let Some(q) = after.find('"') {
                let url = after[..q].to_string();
                if url.starts_with("http") {
                    if let Some(t) = block.find("class=\"t\">") {
                        let t_rest = &block[t + 9..];
                        if let Some(close) = t_rest.find("</span>") {
                            let title = html_unescape(&strip_tags(&t_rest[..close]));
                            if !title.is_empty() {
                                rows.push((title, url));
                            }
                        }
                    }
                }
            }
        }
        pos = (end + 4).min(body.len());
    }
    Ok(rows)
}

/// Parse an oschina.net news list page into rows (`data-url` + `title` attr).
fn parse_oschina(body: &str) -> Result<Vec<(String, String)>> {
    let mut urls = Vec::new();
    let mut pos = 0;
    while let Some(i) = body[pos..].find("data-url=\"") {
        let after = &body[pos + i + 10..];
        if let Some(q) = after.find('"') {
            urls.push(after[..q].to_string());
        }
        pos = pos + i + 10;
    }
    let mut titles = Vec::new();
    let mut pos = 0;
    let needle = "class=\"title\" title=\"";
    while let Some(i) = body[pos..].find(needle) {
        let after = &body[pos + i + needle.len()..];
        if let Some(q) = after.find('"') {
            let t = html_unescape(&strip_tags(&after[..q]));
            if !t.is_empty() {
                titles.push(t);
            }
        }
        pos = pos + i + needle.len();
    }
    let mut rows = Vec::new();
    let n = urls.len().min(titles.len());
    for k in 0..n {
        rows.push((titles[k].clone(), urls[k].clone()));
    }
    Ok(rows)
}

/// Parse an smzdm homepage into deal rows: title + price (—— link).
fn parse_smzdm(body: &str) -> Result<Vec<(String, String)>> {
    let mut rows = Vec::new();
    let mut pos = 0;
    while let Some(start) = body[pos..].find("feed-hot-title") {
        let begin = body[..pos + start].rfind("<a href=\"").map(|i| i).unwrap_or(pos + start);
        // The whole `<a>` block carries the title and the price span.
        let end = body[pos + start..].find("</a>").map(|i| pos + start + i + 4).unwrap_or(body.len());
        let block = &body[begin..end];
        let url = if let Some(href) = block.find("href=\"") {
            let after = &block[href + 6..];
            after[..after.find('"').unwrap_or(0)].to_string()
        } else {
            String::new()
        };
        if let Some(ti) = block.find("feed-hot-title\">") {
            let t_rest = &block[ti + 16..];
            let title = html_unescape(&strip_tags(&t_rest[..t_rest.find("</div>").unwrap_or(0)]));
            if !title.is_empty() {
                let price = if let Some(pi) = block.find("z-highlight\">") {
                    let p_rest = &block[pi + 13..];
                    html_unescape(&strip_tags(&p_rest[..p_rest.find("</span>").unwrap_or(0)]))
                } else {
                    String::new()
                };
                let display = if price.is_empty() { title } else { format!("{title} —— {price}") };
                rows.push((display, if url.is_empty() { "https://www.smzdm.com/".to_string() } else { url }));
            }
        }
        pos = end;
    }
    Ok(rows)
}

/// Parse a Sogou web search results page into rows: title (url).
/// Result blocks look like `<h3 class="vr-title"><a href="...">标题</a></h3>`;
/// `href` is a Sogou `/link?url=` redirect or a direct URL.
fn parse_sogou(body: &str) -> Result<Vec<(String, String)>> {
    let mut rows = Vec::new();
    let mut pos = 0;
    while let Some(start) = body[pos..].find("vr-title") {
        let begin = body[..pos + start].rfind("<h3").unwrap_or(pos + start);
        let end = body[pos + start..].find("</h3>").map(|i| pos + start + i).unwrap_or(body.len());
        let block = &body[begin..end];
        let mut url = String::new();
        if let Some(href) = block.find("href=\"") {
            let after = &block[href + 6..];
            let q = after.find('"').unwrap_or(0);
            url = after[..q].to_string();
        }
        if let Some(gt) = block.find('>') {
            let t_rest = &block[gt + 1..];
            if let Some(close) = t_rest.find("</a>") {
                let title = html_unescape(&strip_tags(&t_rest[..close]));
                if !title.is_empty() && !url.is_empty() {
                    rows.push((title, url));
                }
            }
        }
        pos = (end + 5).min(body.len());
    }
    Ok(rows)
}

/// Run a Sogou web search for `query` and return the parsed headline rows.
fn sogou_search(query: &str, get: &dyn Fn(&str) -> Result<String>) -> Result<Vec<(String, String)>> {
    let url = format!("https://www.sogou.com/web?query={}", urlencode(query));
    parse_sogou(&get(&url)?)
}

/// Parse a Bing search results page into rows: title (url). Used as the
/// fallback engine when Sogou serves a CAPTCHA page or no results.
fn parse_bing(body: &str) -> Result<Vec<(String, String)>> {
    let mut rows = Vec::new();
    let mut pos = 0;
    while let Some(_start) = body[pos..].find("b_algo") {
        let begin = body[pos..].find('<').map(|i| pos + i).unwrap_or(pos);
        let end = body[begin..]
            .find("</li>")
            .map(|i| begin + i)
            .unwrap_or(body.len());
        let block = &body[begin..end];
        let mut title = String::new();
        let mut url = String::new();
        if let Some(h2) = block.find("<h2") {
            let rest = &block[h2..];
            if let Some(ta) = rest.find("<a ") {
                let a_rest = &rest[ta..];
                if let Some(href) = a_rest.find("href=\"") {
                    let after = &a_rest[href + 6..];
                    if let Some(q) = after.find('"') {
                        url = after[..q].to_string();
                    }
                }
                if let Some(gt) = a_rest.find('>') {
                    let t_rest = &a_rest[gt + 1..];
                    if let Some(close) = t_rest.find("</a>") {
                        title = html_unescape(&strip_tags(&t_rest[..close]));
                    }
                }
            }
        }
        if !title.is_empty() && !url.is_empty() {
            rows.push((title, url));
        }
        pos = (end + 5).min(body.len());
    }
    Ok(rows)
}

/// Low-value search rows that clutter the results (encyclopedia entries,
/// wiki mirrors). Filtered from aggregate search: `title` catches Sogou rows
/// whose URL is a `/link?url=` redirect (real domain hidden), `url` catches
/// Bing rows with a real URL.
fn is_low_value_row(title: &str, url: &str) -> bool {
    let t = title.replace(' ', "");
    ["百度百科", "搜狗百科", "维基百科", "互动百科"]
        .iter()
        .any(|k| t.contains(k))
        || [
            "baike.baidu.com",
            "zh.wikipedia.org",
            "en.wikipedia.org",
            "baike.sogou.com",
            "baike.com",
        ]
        .iter()
        .any(|d| url.contains(d))
}

/// Aggregate search: try Sogou first (reliable Chinese tokenization), then
/// fall back to Bing when Sogou serves a CAPTCHA page or an empty result set.
/// Encyclopedia-style entries are filtered out; when both engines return
/// nothing usable, the raw (deduplicated) results are returned as a last
/// resort so the task never fails on an over-filtered page.
fn aggregate_search(query: &str, get: &dyn Fn(&str) -> Result<String>) -> Result<Vec<(String, String)>> {
    let sg = sogou_search(query, get).unwrap_or_default();
    let sg_clean: Vec<(String, String)> = sg
        .iter()
        .filter(|(t, u)| !is_low_value_row(t, u))
        .cloned()
        .collect();
    if !sg_clean.is_empty() {
        return Ok(sg_clean);
    }
    let url = format!("https://www.bing.com/search?q={}&mkt=zh-CN", urlencode(query));
    let bg = parse_bing(&get(&url)?).unwrap_or_default();
    let bg_clean: Vec<(String, String)> = bg
        .iter()
        .filter(|(t, u)| !is_low_value_row(t, u))
        .cloned()
        .collect();
    if !bg_clean.is_empty() {
        return Ok(bg_clean);
    }
    // Last resort: merge whatever the engines returned, deduplicated by URL.
    let mut all = sg;
    all.extend(bg);
    all.dedup_by(|a, b| a.1 == b.1);
    Ok(all)
}

/// Run a zhihu daily-news response into headline rows.
fn parse_zhihu(body: &str) -> Result<Vec<(String, String)>> {
    let v: Value = serde_json::from_str(body).context("zhihu returned invalid JSON")?;
    let mut rows = Vec::new();
    if let Some(stories) = v.get("stories").and_then(|s| s.as_array()) {
        for s in stories {
            let title = s.get("title").and_then(|x| x.as_str());
            let id = s
                .get("id")
                .and_then(|x| x.as_str())
                .map(str::to_string)
                .or_else(|| s.get("id").and_then(|x| x.as_u64()).map(|n| n.to_string()));
            if let (Some(t), Some(id)) = (title, id) {
                rows.push((t.to_string(), format!("https://daily.zhihu.com/story/{id}")));
            }
        }
    }
    Ok(rows)
}

/// Parse a baidu hot-search response into headline rows.
fn parse_baidu(body: &str) -> Result<Vec<(String, String)>> {
    let v: Value = serde_json::from_str(body).context("baidu returned invalid JSON")?;
    let mut rows = Vec::new();
    let content = v
        .pointer("/data/cards/0/content/0/content")
        .and_then(|c| c.as_array());
    if let Some(items) = content {
        for it in items {
            if let Some(w) = it.get("word").and_then(|x| x.as_str()) {
                let url = it
                    .get("url")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                rows.push((w.to_string(), url));
            }
        }
    }
    Ok(rows)
}

/// Parse a bilibili popular-videos response into headline rows.
fn parse_bilibili(body: &str) -> Result<Vec<(String, String)>> {
    let v: Value = serde_json::from_str(body).context("bilibili returned invalid JSON")?;
    let mut rows = Vec::new();
    if let Some(list) = v.pointer("/data/list").and_then(|l| l.as_array()) {
        for it in list {
            if let (Some(t), Some(bvid)) = (
                it.get("title").and_then(|x| x.as_str()),
                it.get("bvid").and_then(|x| x.as_str()),
            ) {
                rows.push((t.to_string(), format!("https://www.bilibili.com/video/{bvid}")));
            }
        }
    }
    Ok(rows)
}

/// Parse a GitHub search response into headline rows.
fn parse_github(body: &str) -> Result<Vec<(String, String)>> {
    let v: Value = serde_json::from_str(body).context("github returned invalid JSON")?;
    let mut rows = Vec::new();
    if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
        for it in items {
            if let (Some(name), Some(url)) = (
                it.get("full_name").and_then(|x| x.as_str()),
                it.get("html_url").and_then(|x| x.as_str()),
            ) {
                let stars = it.get("stargazers_count").and_then(|x| x.as_u64()).unwrap_or(0);
                rows.push((format!("{name} ★{stars}"), url.to_string()));
            }
        }
    }
    Ok(rows)
}

/// Parse a Hacker News item response (title + url).
fn parse_hn_item(body: &str) -> Result<Option<(String, String)>> {
    let v: Value = serde_json::from_str(body).context("hacker news returned invalid JSON")?;
    let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
    if title.is_empty() {
        return Ok(None);
    }
    let url = v
        .get("url")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            v.get("id")
                .and_then(|x| x.as_u64())
                .map(|id| format!("https://news.ycombinator.com/item?id={id}"))
                .unwrap_or_default()
        });
    Ok(Some((title, url)))
}

/// YYYY-MM-DD for `n` days before today (UTC).
fn days_ago(n: u64) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_sub(n * 86400);
    let days = secs / 86400;
    let (mut y, mut m, mut d) = (1970, 1, 1);
    let mut rem = days;
    loop {
        let yl = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
        if rem >= yl {
            rem -= yl;
            y += 1;
        } else {
            break;
        }
    }
    let mdays = [31, if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for (i, ml) in mdays.iter().enumerate() {
        if rem >= *ml {
            rem -= ml;
        } else {
            m = i + 1;
            d = rem + 1;
            break;
        }
    }
    format!("{y:04}-{m:02}-{d:02}")
}

/// Run a built-in search source and return a plain headline list. All sources
/// are plain HTTP requests implemented in code (no local shell commands).
fn builtin_search(source: &str, args: &HashMap<String, String>) -> Result<String> {
    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
    let get = |url: &str| -> Result<String> {
        let resp = client
            .get(url)
            .header(USER_AGENT, BROWSER_UA)
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
            .send()
            .with_context(|| format!("search request to {url} failed"))?;
        if !resp.status().is_success() {
            bail!("{url} returned HTTP {}", resp.status());
        }
        Ok(resp.text().context("cannot read search response")?)
    };
    let rows: Vec<(String, String)> = match source.to_ascii_lowercase().as_str() {
        "zhihu" => parse_zhihu(&get("https://news-at.zhihu.com/api/4/news/latest")?)?,
        "baidu" => parse_baidu(&get("https://top.baidu.com/api/board?platform=wise&tab=realtime")?)?,
        "bilibili" => parse_bilibili(&get("https://api.bilibili.com/x/web-interface/popular?ps=10")?)?,
        "github" => {
            let date = args.get("date").cloned().unwrap_or_else(|| days_ago(7));
            let url = format!(
                "https://api.github.com/search/repositories?q=created:%3E{date}&sort=stars&order=desc&per_page=10"
            );
            parse_github(&get(&url)?)?
        }
        "hn" => {
            let ids: Vec<u64> = serde_json::from_str(&get("https://hacker-news.firebaseio.com/v0/topstories.json")?)
                .context("hacker news returned invalid JSON")?;
            let mut out = Vec::new();
            for id in ids.iter().take(5) {
                let body = get(&format!("https://hacker-news.firebaseio.com/v0/item/{id}.json"))?;
                if let Some(row) = parse_hn_item(&body)? {
                    out.push(row);
                }
            }
            out
        }
        // Direct sources
        "toutiao" => parse_toutiao(&get("https://www.toutiao.com/api/pc/feed/?category=news_hot&offset=0&count=10")?)?,
        "tophub" => parse_tophub(&get("https://tophub.today/c/developer")?)?,
        "oschina" => parse_oschina(&get("https://www.oschina.net/news/")?)?,
        "smzdm" => parse_smzdm(&get("https://www.smzdm.com/")?)?,
        // Aggregate search (Sogou first, Bing fallback) for sites whose own
        // data endpoints are signed / WAF-gated.
        "bing" | "sogou" => {
            let q = args.get("q").or_else(|| args.get("query")).cloned().unwrap_or_default();
            if q.is_empty() {
                bail!("search source `{source}` requires a query via args `q` / `query`");
            }
            aggregate_search(&q, &get)?
        }
        "dxtower" => aggregate_search("德塔文 电视剧景气指数 今日 榜单", &get)?,
        "enlightent" => aggregate_search("云合数据 热播电视剧 霸屏榜 今日", &get)?,
        "cls" => aggregate_search("财联社 电报 今日 财经", &get)?,
        "dongchedi" => aggregate_search("懂车帝 汽车 资讯 新闻", &get)?,
        "autohome" => aggregate_search("汽车之家 汽车 新闻 资讯", &get)?,
        // Shenzhen housing sales (fdc.zjj.sz.gov.cn is behind a Ruishi WAF, so
        // use aggregate search over the public housing-market data).
        "szhousing" => aggregate_search("深圳 新房 成交 套数 深圳房地产信息平台", &get)?,
        // Administrative penalty / dishonest-executor records. Official
        // portals (creditchina / court execution / gsxt) require CAPTCHA or
        // login, so aggregate search over public disclosure pages is used.
        // The entity name comes from the model's `name` / `input` argument.
        "penalty" => {
            let name = args.get("name").or_else(|| args.get("input")).cloned().unwrap_or_default();
            let q = if name.trim().is_empty() {
                "行政处罚 失信被执行人 公示".to_string()
            } else {
                format!("{name} 行政处罚 失信 公示")
            };
            aggregate_search(&q, &get)?
        }
        // Company registration info. Same approach: the name comes from the
        // model's `name` / `input` argument.
        "company" => {
            let name = args.get("name").or_else(|| args.get("input")).cloned().unwrap_or_default();
            let q = if name.trim().is_empty() {
                "企业工商信息 查询 注册".to_string()
            } else {
                format!("{name} 企业工商信息")
            };
            aggregate_search(&q, &get)?
        }
        other => bail!(
            "unknown search source `{other}` (available: zhihu, baidu, bilibili, github, hn, toutiao, tophub, oschina, smzdm, bing, sogou, dxtower, enlightent, cls, dongchedi, autohome, szhousing, penalty, company)"
        ),
    };
    if rows.is_empty() {
        bail!("search source `{source}` returned no results");
    }
    Ok(fmt_headlines(&rows))
}

/// Execute a task's tool: either a fixed HTTP API template (`api`) or a
/// built-in search source (`search`). `{key}` / `{key:default}` placeholders
/// are substituted from the model's tool arguments. Returns the raw tool
/// result text fed back to the model.
fn run_task_tool(task: &Task, args: &HashMap<String, String>) -> Result<String> {
    if let Some(tpl) = &task.api {
        if !tpl.trim().is_empty() {
            let url = substitute(tpl, args);
            let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
            let resp = client
                .get(&url)
                .header(USER_AGENT, format!("sysenv/{}", env!("CARGO_PKG_VERSION")))
                .send()
                .with_context(|| format!("api request to {url} failed"))?;
            if !resp.status().is_success() {
                bail!("{url} returned HTTP {}", resp.status());
            }
            return resp.text().context("cannot read api response");
        }
    }
    if let Some(src) = &task.search {
        if !src.trim().is_empty() {
            return builtin_search(src, args);
        }
    }
    bail!("task `{}` has no api/search tool configured", task.name);
}

/// Route the user's request to the configured tasks via function calls and
/// execute the first match: the task description plus the user request are
/// assembled into the chat message. When the task declares an `api` URL or a
/// `search` source, the tool is executed first (placeholders filled from the
/// model arguments) and its result is fed back, so the model's reply is
/// grounded in real data.
fn route_and_execute(cfg: &mut Config, path: &Path, input: &str, debug: bool, no_stream: bool) -> Result<()> {
    let calls = route_tasks(cfg, path, input, debug)?;
    if calls.is_empty() {
        bail!(
            "模型未选择任何任务（当前配置 {} 个任务）；可加 `-t NAME` 手动指定",
            cfg.tasks.len()
        );
    }
    for (i, (tname, args)) in calls.iter().enumerate() {
        let desc = cfg
            .tasks
            .iter()
            .find(|t| ci_eq(&t.name, tname))
            .map(|t| t.desc.as_str())
            .unwrap_or("");
        println!("[{i}] task: {tname}");
        println!("    desc: {desc}");
        println!("    args: {args}");
    }
    let (tname, args) = &calls[0];
    let task = cfg
        .tasks
        .iter()
        .find(|t| ci_eq(&t.name, tname))
        .ok_or_else(|| anyhow::anyhow!("模型返回了未配置的任务名 `{tname}`"))?;
    let user_arg = args
        .get("input")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| input.to_string());

    // Flatten the model's tool arguments into a string map for substitution.
    let mut arg_map = HashMap::new();
    if let Some(obj) = args.as_object() {
        for (k, v) in obj {
            let s = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
            arg_map.insert(k.clone(), s);
        }
    }

    // Execute the task tool (fixed HTTP API or built-in search) when declared;
    // feed its result back to the model so the final reply uses real data.
    let tool_result = if task.api.is_some() || task.search.is_some() {
        Some(run_task_tool(task, &arg_map)?)
    } else {
        None
    };

    let exec_msg = match &tool_result {
        Some(tool_out) => format!(
            "任务说明：{}\n\n用户请求：{}\n\n工具执行结果：\n{}\n\n请基于工具执行结果回答用户。",
            task.desc, user_arg, tool_out
        ),
        None => format!("任务说明：{}\n\n用户请求：{}", task.desc, user_arg),
    };
    println!("\n→ 执行任务 {tname}");
    chat_with(cfg, path, &exec_msg, debug, no_stream, None)
}

fn parse_params(items: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for item in items {
        let (k, v) = if let Some(i) = item.find(':') {
            (item[..i].trim(), item[i + 1..].trim())
        } else if let Some(i) = item.find('=') {
            (item[..i].trim(), item[i + 1..].trim())
        } else {
            continue;
        };
        if !k.is_empty() {
            map.insert(k.to_string(), v.to_string());
        }
    }
    map
}

/// Replace every `{key:default}` / `{key}` placeholder with the user-supplied
/// value or the built-in default.
fn substitute(template: &str, params: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let inner = &after[..close];
                let (key, def) = match inner.split_once(':') {
                    Some((k, d)) => (k.trim(), Some(d.trim())),
                    None => (inner.trim(), None),
                };
                match params.get(key) {
                    // An empty supplied value counts as absent so `{k:default}`
                    // placeholders (e.g. URL parameters) fall back to their default.
                    Some(v) if !v.is_empty() => out.push_str(v),
                    _ => match def {
                        Some(d) => out.push_str(d),
                        None => {
                            out.push('{');
                            out.push_str(inner);
                            out.push('}');
                        }
                    },
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Assemble the chat message of a task: `file://` reads a local file, `url:`
/// fetches a web page, otherwise the template is used; `{key:default}`
/// placeholders are substituted from the command-line parameters.
/// Build the substitution map for a task's `msg`/tool template from the
/// command-line parameters. When the user passes a bare free-text parameter
/// (no `key:value` / `key=value`) and the task declares parameters, the free
/// text fills the first declared parameter (e.g. `-t company 字节跳动` →
/// `name=字节跳动`).
fn task_param_map(task: &Task, params: &[String]) -> HashMap<String, String> {
    let mut map = parse_params(params);
    if let Some(plist) = &task.params {
        if let Some(first) = plist.first() {
            if !map.contains_key(first) {
                if let Some(free) = params.iter().find(|p| !p.contains(':') && !p.contains('=')) {
                    map.insert(first.clone(), free.trim().to_string());
                }
            }
        }
    }
    map
}

fn resolve_task_msg(task: &Task, params: &[String]) -> Result<String> {
    if task.msg.is_empty() {
        bail!("task `{}` has no msg", task.name);
    }
    let raw = if let Some(rest) = task.msg.strip_prefix("file://") {
        let path = PathBuf::from(rest.trim());
        std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read file `{}`", path.display()))?
    } else if let Some(rest) = task.msg.strip_prefix("url:") {
        let url = rest.trim();
        let resp = reqwest::blocking::get(url).with_context(|| format!("cannot fetch {url}"))?;
        if !resp.status().is_success() {
            bail!("{url} returned HTTP {}", resp.status());
        }
        resp.text().with_context(|| format!("cannot read response from {url}"))?
    } else {
        task.msg.clone()
    };
    let map = task_param_map(task, params);
    Ok(substitute(&raw, &map))
}

/// Complete the target model's capabilities in memory and in the config file:
/// when `max_input_tokens` or `type` is missing, look them up in models.dev by
/// model name (first match) and persist the values. Best effort: lookup or
/// persistence failures only warn and never block the request.
fn ensure_model_capabilities(cfg: &mut Config, path: &Path, pi: usize, mi: usize) {
    let (ctx, modl) = {
        let m = &cfg.providers[pi].models[mi];
        if m.max_input_tokens.is_some() && m.model_type.is_some() {
            return;
        }
        crate::ai::lookup_model_capabilities(&m.name)
    };
    let need_ctx = cfg.providers[pi].models[mi].max_input_tokens.is_none() && ctx.is_some();
    let need_type = cfg.providers[pi].models[mi].model_type.is_none() && modl.is_some();
    if !need_ctx && !need_type {
        return;
    }
    let pname = cfg.providers[pi].name.clone();
    let mname = cfg.providers[pi].models[mi].name.clone();
    let model = &mut cfg.providers[pi].models[mi];
    if need_ctx {
        model.max_input_tokens = ctx;
    }
    if need_type {
        model.model_type = modl.clone();
    }
    if need_ctx {
        if let Some(c) = ctx {
            if let Err(e) = update_model_field_in_config(path, &pname, &mname, "max_input_tokens", &c.to_string()) {
                eprintln!("sysenv: warning: cannot persist max_input_tokens for {pname} / {mname}: {e:#}");
            }
        }
    }
    if need_type {
        if let Some(t) = &modl {
            if let Err(e) = update_model_field_in_config(path, &pname, &mname, "type", t) {
                eprintln!("sysenv: warning: cannot persist type for {pname} / {mname}: {e:#}");
            }
        }
    }
}

/// Truncate `msg` to `limit` characters when a limit is set; otherwise the
/// message passes through unchanged. A notice is printed when truncation
/// actually happens.
fn truncate_to_limit(msg: &str, limit: Option<u64>) -> String {
    match limit {
        Some(l) if (msg.chars().count() as u64) > l => {
            let truncated: String = msg.chars().take(l as usize).collect();
            eprintln!(
                "sysenv: message of {} chars truncated to {} (model input limit)",
                msg.chars().count(),
                l
            );
            truncated
        }
        _ => msg.to_string(),
    }
}

/// One chat round-trip through the weighted rotation. Before each HTTP request
/// the target model's capabilities are completed (`max_input_tokens` / `type`
/// looked up in models.dev and persisted when missing) and the message is
/// truncated to the model's input limit. Returns the model's reply text (which
/// has already been streamed to stdout when streaming is active).
fn chat_once(
    cfg: &mut Config,
    path: &Path,
    msg: &str,
    debug: bool,
    no_stream: bool,
    model_override: Option<&str>,
) -> Result<String> {
    let selector = model_override.or(cfg.model.as_deref()).unwrap_or("<default>");
    let targets = resolve_targets(cfg, selector)?;
    if targets.is_empty() {
        bail!("no model targets to chat with (selector `{selector}` matched nothing)");
    }

    // Try the weighted pick first, then the remaining targets in order.
    let first = pick_weighted(cfg, &targets, selector, path)?;
    let mut order: Vec<usize> = Vec::with_capacity(targets.len());
    order.push(first);
    for i in 0..targets.len() {
        if i != first {
            order.push(i);
        }
    }

    let stream = cfg.stream && !debug && !no_stream;
    let mut had_failure = false;
    let mut errors: Vec<String> = Vec::new();
    for ti in order {
        let t = &targets[ti];
        ensure_model_capabilities(cfg, path, t.provider, t.model);
        let p = &cfg.providers[t.provider];
        let m = &p.models[t.model];
        eprintln!("sysenv: using {} / {} ({})", p.name, m.name, p.kind);
        let payload = truncate_to_limit(msg, m.max_input_tokens);
        match send_chat(p, m, &payload, stream, debug) {
            Ok(text) => {
                // A model that succeeded after a previous failure gets its
                // weight bumped (max 9), persisted to the config file.
                if had_failure && m.weight < 9 {
                    if let Err(e) = update_weight_in_config(path, &p.name, &m.name, m.weight + 1) {
                        eprintln!("sysenv: warning: failed to persist weight bump for {} / {}: {e:#}", p.name, m.name);
                    }
                }
                return Ok(text);
            }
            Err(e) => {
                errors.push(format!("{} / {}: {e:#}", p.name, m.name));
                // Failure decrements the weight (min 1), persisted to config,
                // then the next model is tried automatically.
                if m.weight > 1 {
                    if let Err(e2) = update_weight_in_config(path, &p.name, &m.name, m.weight - 1) {
                        eprintln!("sysenv: warning: failed to persist weight drop for {} / {}: {e2:#}", p.name, m.name);
                    }
                }
                had_failure = true;
            }
        }
    }
    bail!("all {} model(s) failed: {}", targets.len(), errors.join(" | "));
}

/// `sysenv ai chat` core: runs `chat_once` and prints the reply when the
/// request was non-streaming (streaming already printed to stdout).
fn chat_with(cfg: &mut Config, path: &Path, msg: &str, debug: bool, no_stream: bool, model_override: Option<&str>) -> Result<()> {
    let text = chat_once(cfg, path, msg, debug, no_stream, model_override)?;
    let stream = cfg.stream && !debug && !no_stream;
    if !stream {
        println!("{text}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// `sysenv ai chat [MSG...] [-m MODEL] [--list-model] [--list-provider] [-c FILE] [--debug] [--no-stream]`
///
/// `-m/--model` overrides the top-level `model` from the config (same
/// `{provider}:{model}` / `{model}` / comma-separated rules). `--list-model`
/// prints every configured model (grouped by provider); `--list-provider`
/// prints every configured provider.
pub fn cmd_chat(
    words: &[String],
    config: Option<&Path>,
    debug: bool,
    no_stream: bool,
    model: Option<&str>,
    list_model: bool,
    list_provider: bool,
) -> Result<()> {
    let (mut cfg, path) = load_config(config)?;

    if list_model || list_provider {
        if list_provider {
            println!("{:<16}  {:<10}  {}", "PROVIDER", "KIND", "MODELS");
            for p in &cfg.providers {
                println!("{:<16}  {:<10}  {}", p.name, p.kind, p.models.len());
            }
        }
        if list_model {
            if list_provider {
                println!();
            }
            for p in &cfg.providers {
                println!("{} ({}):", p.name, p.kind);
                for m in &p.models {
                    let w = if m.weight == 1 {
                        String::new()
                    } else {
                        format!(" (weight {})", m.weight)
                    };
                    println!("  - {}{}", m.name, w);
                }
            }
        }
        return Ok(());
    }

    let msg = if !words.is_empty() {
        words.join(" ")
    } else if !std::io::stdin().is_terminal() {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).context("cannot read stdin")?;
        if s.trim().is_empty() {
            bail!("empty message from stdin; provide MSG or pipe text via stdin");
        }
        s
    } else {
        bail!("provide a message: `sysenv ai chat \"your message\"` (or pipe text via stdin)");
    };
    chat_with(&mut cfg, &path, &msg, debug, no_stream, model)
}

/// One row of the built-in search source catalogue shown by `--list-source`.
struct SourceRow {
    name: &'static str,
    kind: &'static str,
    purpose: &'static str,
    url: &'static str,
}

/// All built-in `search` sources of `sysenv ai task`: name, kind
/// (`api` = direct HTTP API, `search` = aggregate search with Sogou first and
/// Bing fallback, `generic` = general search engine requiring a `q`/`query`
/// argument), purpose and access address.
const SOURCES: &[SourceRow] = &[
    SourceRow { name: "zhihu",      kind: "api",     purpose: "知乎每日热门新闻",             url: "https://news-at.zhihu.com/api/4/news/latest" },
    SourceRow { name: "baidu",      kind: "api",     purpose: "百度实时热搜榜",               url: "https://top.baidu.com/api/board" },
    SourceRow { name: "bilibili",   kind: "api",     purpose: "B站热门视频",                  url: "https://api.bilibili.com/x/web-interface/popular" },
    SourceRow { name: "github",     kind: "api",     purpose: "GitHub 近期热门仓库",          url: "https://api.github.com/search/repositories" },
    SourceRow { name: "hn",         kind: "api",     purpose: "Hacker News 热门",             url: "https://news.ycombinator.com/" },
    SourceRow { name: "toutiao",    kind: "api",     purpose: "今日头条热榜",                 url: "https://www.toutiao.com/api/pc/feed/" },
    SourceRow { name: "tophub",     kind: "api",     purpose: "tophub 开发者热榜",            url: "https://tophub.today/c/developer" },
    SourceRow { name: "oschina",    kind: "api",     purpose: "开源中国技术新闻",             url: "https://www.oschina.net/news/" },
    SourceRow { name: "smzdm",      kind: "api",     purpose: "什么值得买今日特价",           url: "https://www.smzdm.com/" },
    SourceRow { name: "bing",       kind: "generic", purpose: "通用网页搜索（需 q/query 参数）", url: "https://www.bing.com/search" },
    SourceRow { name: "sogou",      kind: "generic", purpose: "通用网页搜索（需 q/query 参数）", url: "https://www.sogou.com/web" },
    SourceRow { name: "dxtower",    kind: "search",  purpose: "德塔文电视剧景气指数/榜单",     url: "https://www.dxtower.com/" },
    SourceRow { name: "enlightent", kind: "search",  purpose: "云合数据热播剧霸屏榜",         url: "https://www.enlightent.cn/" },
    SourceRow { name: "cls",        kind: "search",  purpose: "财联社电报/财经",              url: "https://www.cls.cn/telegraph" },
    SourceRow { name: "dongchedi",  kind: "search",  purpose: "懂车帝汽车资讯",               url: "https://www.dongchedi.com/" },
    SourceRow { name: "autohome",   kind: "search",  purpose: "汽车之家汽车新闻",             url: "https://www.autohome.com.cn/" },
    SourceRow { name: "szhousing",  kind: "search",  purpose: "深圳房源销售/成交情况",         url: "https://fdc.zjj.sz.gov.cn/" },
    SourceRow { name: "penalty",    kind: "search",  purpose: "行政处罚/失信被执行人信息（name 参数）", url: "https://www.creditchina.gov.cn/" },
    SourceRow { name: "company",    kind: "search",  purpose: "公司工商注册信息（name 参数）", url: "https://aiqicha.baidu.com/" },
];

/// Print the built-in search source catalogue (name / kind / purpose / URL).
pub fn list_sources() {
    let w_name = SOURCES.iter().map(|s| s.name.chars().count()).max().unwrap_or(4).max(4);
    let w_kind = SOURCES.iter().map(|s| s.kind.chars().count()).max().unwrap_or(4).max(4);
    let w_purpose = SOURCES.iter().map(|s| s.purpose.chars().count()).max().unwrap_or(4).max(4);
    println!("{:<w_name$}  {:<w_kind$}  {:<w_purpose$}  URL", "NAME", "KIND", "PURPOSE");
    for s in SOURCES {
        println!("{:<w_name$}  {:<w_kind$}  {:<w_purpose$}  {}", s.name, s.kind, s.purpose, s.url);
    }
}

/// `sysenv ai task [-t NAME] [key:value...] [-c FILE] [--debug] [--no-stream] [--list-source]`
///
/// Without `-t`: with a user request (arguments or piped stdin) the request is
/// routed to the configured tasks via LLM function calls and the best match is
/// executed; without input the configured tasks are listed.
pub fn cmd_task(
    sel: Option<&str>,
    params: &[String],
    config: Option<&Path>,
    debug: bool,
    no_stream: bool,
    list_source: bool,
) -> Result<()> {
    if list_source {
        list_sources();
        return Ok(());
    }
    let (mut cfg, path) = load_config(config)?;
    match sel {
        None => {
            if cfg.tasks.is_empty() {
                println!("(no tasks configured under `tasks` in {})", path.display());
                return Ok(());
            }
            // A user request (positional words or piped stdin) triggers the
            // function-call routing; otherwise the tasks are listed.
            let input = if !params.is_empty() {
                Some(params.join(" "))
            } else if !std::io::stdin().is_terminal() {
                try_read_stdin(Duration::from_millis(400))
            } else {
                None
            };
            if let Some(input) = input {
                return route_and_execute(&mut cfg, &path, &input, debug, no_stream);
            }
            let shown = cfg.tasks.len().min(10);
            let w_name = cfg.tasks.iter().take(shown).map(|t| t.name.chars().count()).max().unwrap_or(4).max(4);
            let w_desc = cfg.tasks.iter().take(shown).map(|t| t.desc.chars().count()).max().unwrap_or(4).max(4);
            println!("{:<w_name$}  {:<w_desc$}", "NAME", "DESC");
            for t in cfg.tasks.iter().take(shown) {
                println!("{:<w_name$}  {:<w_desc$}", t.name, t.desc);
            }
            if cfg.tasks.len() > shown {
                println!("--- {} tasks (showing the first {shown})", cfg.tasks.len());
            }
            Ok(())
        }
        Some(name) if name == "*" => score_all_tasks(&mut cfg, &path, params, debug),
        Some(name) => {
            let task = cfg
                .tasks
                .iter()
                .find(|t| ci_eq(&t.name, name))
                .or_else(|| {
                    cfg.tasks
                        .iter()
                        .find(|t| t.name.to_ascii_lowercase().contains(&name.to_ascii_lowercase()))
                })
                .ok_or_else(|| {
                    let avail: Vec<&str> = cfg.tasks.iter().map(|t| t.name.as_str()).collect();
                    anyhow::anyhow!("no task named `{name}` (available: {})", avail.join(", "))
                })?;
            let msg = resolve_task_msg(task, params)?;
            // Manual `-t` selection also executes the task's api/search tool
            // when declared, so the reply is grounded in real data.
            let map = task_param_map(task, params);
            let tool_result = if task.api.is_some() || task.search.is_some() {
                Some(run_task_tool(task, &map)?)
            } else {
                None
            };
            let exec_msg = match &tool_result {
                Some(out) => format!(
                    "任务说明：{}\n\n用户请求：{}\n\n工具执行结果：\n{}\n\n请基于工具执行结果回答用户。",
                    task.desc, msg, out
                ),
                None => msg,
            };
            println!("\n→ 执行任务 {}", task.name);
            chat_with(&mut cfg, &path, &exec_msg, debug, no_stream, None)
        }
    }
}

/// Extract a 0-10 integer score from a model reply. The first number in the
/// text that lies within 0..=10 wins (e.g. `9`, `8/10`, `score: 10`).
fn parse_score(text: &str) -> Option<u32> {
    let mut nums: Vec<u32> = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            if let Ok(v) = cur.parse::<u32>() {
                nums.push(v);
            }
            cur.clear();
        }
    }
    if !cur.is_empty() {
        if let Ok(v) = cur.parse::<u32>() {
            nums.push(v);
        }
    }
    nums.into_iter().find(|&v| v <= 10)
}

/// `sysenv ai task -t * [USER REQUEST]`: ask the configured LLM to score how
/// well each configured task's description matches the user's request (0 = no
/// match, 10 = perfect match). Prints `TASK / DESC / SCORE`, best match first.
fn score_all_tasks(cfg: &mut Config, path: &Path, params: &[String], debug: bool) -> Result<()> {
    let user_msg = if !params.is_empty() {
        params.join(" ")
    } else if !std::io::stdin().is_terminal() {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).context("cannot read stdin")?;
        if s.trim().is_empty() {
            bail!("empty request from stdin; provide the user request as arguments or pipe it via stdin");
        }
        s
    } else {
        bail!("provide the user request: `sysenv ai task -t * \"your request\"` (or pipe it via stdin)");
    };
    if cfg.tasks.is_empty() {
        bail!("no tasks configured under `tasks` in `{}`", path.display());
    }

    let mut rows: Vec<(String, String, Option<u32>)> = Vec::new();
    let tasks: Vec<(String, String)> = cfg
        .tasks
        .iter()
        .map(|t| (t.name.clone(), t.desc.clone()))
        .collect();
    for (tname, tdesc) in &tasks {
        let prompt = format!(
            "你是任务匹配评估器。请评估下面的任务与用户需求是否匹配。\n\
             任务名称：{}\n\
             任务描述：{}\n\
             用户需求：{}\n\
             输出 0 到 10 的整数匹配分数（0=完全不匹配，10=完全匹配），只输出分数数字本身，不要任何其他内容。",
            tname, tdesc, user_msg
        );
        match chat_once(cfg, path, &prompt, debug, true, None) {
            Ok(text) => {
                let score = parse_score(&text);
                if score.is_none() {
                    eprintln!("sysenv: warning: cannot parse a 0-10 score from the reply for task `{}`", tname);
                }
                rows.push((tname.clone(), tdesc.clone(), score));
            }
            Err(e) => {
                eprintln!("sysenv: warning: scoring task `{}` failed: {e:#}", tname);
                rows.push((tname.clone(), tdesc.clone(), None));
            }
        }
    }

    rows.sort_by(|a, b| b.2.cmp(&a.2));
    let w_name = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(4).max(4);
    let w_desc = rows.iter().map(|r| r.1.chars().count()).max().unwrap_or(4).max(4);
    println!("{:<w_name$}  {:<w_desc$}  {}", "TASK", "DESC", "SCORE");
    for (n, d, s) in &rows {
        let sv = s.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string());
        println!("{:<w_name$}  {:<w_desc$}  {}", n, d, sv);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_value_rows_are_detected() {
        assert!(is_low_value_row("字节跳动_百度百科", "https://www.sogou.com/link?url=abc"));
        assert!(is_low_value_row("深圳 - 搜狗百科", "https://www.sogou.com/link?url=def"));
        assert!(is_low_value_row("北京", "https://baike.baidu.com/item/%E5%8C%97%E4%BA%AC"));
        assert!(is_low_value_row("Shenzhen", "https://en.wikipedia.org/wiki/Shenzhen"));
        assert!(!is_low_value_row("字节跳动 - ByteDance", "https://www.bytedance.com/zh/"));
        assert!(!is_low_value_row("深圳 新房成交 套数", "https://www.leyoujia.com/"));
    }

    const SAMPLE: &str = r#"model: agnes-3.0-flash
clients:
  - type: openai
    name: agnes
    api_base: https://apihub.example.cn/v1
    api_key: sk-test-0000
    models:
      - name: agnes-3.0-flash
        weight: 1
  - type: anthropic
    name: claude
    api_base: https://api.anthropic.example.com
    api_key: sk-ant-test
    models:
      - name: claude-3-5-sonnet
tasks:
  - name: weather
    desc: 用来获取天气信息
    msg: 我在{country:深圳},今天的天气如何，我要询问温度、湿度、下雨概率等信息
    api: https://api.open-meteo.com/v1/forecast?latitude={lat:22.54}&longitude={lon:114.06}&current_weather=true
    params: [lat, lon]
  - name: topnews
    desc: 获取今天的热门新闻
    msg: 请基于搜索结果列出今天的新闻头条
    search: zhihu
stream: true
"#;

    fn sample_config() -> Config {
        let y = parse_yaml(SAMPLE).unwrap();
        config_from_yaml(&y).unwrap()
    }

    #[test]
    fn yaml_parses_real_shape() {
        let y = parse_yaml(SAMPLE).unwrap();
        assert_eq!(y.get("model").and_then(|v| v.scalar()), Some("agnes-3.0-flash"));
        assert_eq!(y.get("stream").and_then(|v| v.scalar()), Some("true"));
        let clients = y.get("clients").and_then(|v| v.list()).unwrap();
        assert_eq!(clients.len(), 2);
        let tasks = y.get("tasks").and_then(|v| v.list()).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].get("msg").and_then(|v| v.scalar()).unwrap(), "我在{country:深圳},今天的天气如何，我要询问温度、湿度、下雨概率等信息");
    }

    #[test]
    fn config_typed() {
        let cfg = sample_config();
        assert_eq!(cfg.model.as_deref(), Some("agnes-3.0-flash"));
        assert!(cfg.stream);
        assert_eq!(cfg.providers.len(), 2);
        assert_eq!(cfg.providers[0].kind, "openai");
        assert_eq!(cfg.providers[1].kind, "anthropic");
        assert_eq!(cfg.providers[0].models.len(), 1);
        assert_eq!(cfg.providers[0].models[0].name, "agnes-3.0-flash");
        assert_eq!(cfg.providers[0].models[0].weight, 1);
        assert_eq!(cfg.tasks.len(), 2);
        assert!(cfg.tasks[0].api.as_deref().unwrap_or("").contains("api.open-meteo.com"));
        assert_eq!(cfg.tasks[0].params.as_deref(), Some(["lat".to_string(), "lon".to_string()].as_slice()));
        assert_eq!(cfg.tasks[1].search.as_deref(), Some("zhihu"));
    }

    #[test]
    fn mangled_weight_item_merges_into_previous_model() {
        let yaml = r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
      - weight: 3
"#;
        let y = parse_yaml(yaml).unwrap();
        let cfg = config_from_yaml(&y).unwrap();
        assert_eq!(cfg.providers[0].models.len(), 1);
        assert_eq!(cfg.providers[0].models[0].name, "m1");
        assert_eq!(cfg.providers[0].models[0].weight, 3);
    }

    #[test]
    fn missing_required_fields_error() {
        let yaml = r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    models:
      - name: m1
"#;
        let y = parse_yaml(yaml).unwrap();
        assert!(config_from_yaml(&y).is_err());
    }

    #[test]
    fn resolve_targets_all_forms() {
        // missing -> first provider first model
        let mut cfg = sample_config();
        cfg.model = None;
        let t = resolve_targets(&cfg, "<default>").unwrap();
        assert_eq!((t[0].provider, t[0].model), (0, 0));

        // {provider}:{model}
        cfg.model = Some("claude:claude-3-5-sonnet".to_string());
        let t = resolve_targets(&cfg, "claude:claude-3-5-sonnet").unwrap();
        assert_eq!((t[0].provider, t[0].model), (1, 0));
        assert_eq!(t.len(), 1);

        // {provider}:*
        cfg.model = Some("agnes:*".to_string());
        let t = resolve_targets(&cfg, "agnes:*").unwrap();
        assert_eq!(t.len(), 1);

        // {model} across providers
        cfg.model = Some("agnes-3.0-flash".to_string());
        let t = resolve_targets(&cfg, "agnes-3.0-flash").unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].provider, 0);

        // unknown
        cfg.model = Some("nope:*".to_string());
        assert!(resolve_targets(&cfg, "nope:*").is_err());
        cfg.model = Some("nope".to_string());
        assert!(resolve_targets(&cfg, "nope").is_err());
    }

    #[test]
    fn resolve_targets_comma_lists() {
        let cfg = sample_config();
        // Half-width comma between a {provider}:{model} and a bare {model}.
        let t = resolve_targets(&cfg, "claude:claude-3-5-sonnet,agnes-3.0-flash").unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].provider, t[0].model), (1, 0));
        assert_eq!((t[1].provider, t[1].model), (0, 0));

        // Full-width comma, plus an empty part in the middle.
        let t = resolve_targets(&cfg, "agnes-3.0-flash，claude-3-5-sonnet").unwrap();
        assert_eq!(t.len(), 2);
        let t = resolve_targets(&cfg, "agnes-3.0-flash,,claude-3-5-sonnet").unwrap();
        assert_eq!(t.len(), 2);

        // Overlapping selectors are de-duplicated.
        let t = resolve_targets(&cfg, "agnes:*,agnes-3.0-flash").unwrap();
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn split_selector_handles_commas() {
        assert_eq!(split_selector("a,b"), vec!["a", "b"]);
        assert_eq!(split_selector("a，b"), vec!["a", "b"]);
        assert_eq!(split_selector(" a , b ,c "), vec!["a", "b", "c"]);
        assert_eq!(split_selector("a,,b"), vec!["a", "b"]);
        assert!(split_selector("").is_empty());
        assert!(split_selector("，,").is_empty());
    }

    #[test]
    fn weight_parsed_in_0_9_range() {
        let yaml = r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
        weight: 42
      - name: m2
        weight: 0
"#;
        let y = parse_yaml(yaml).unwrap();
        let cfg = config_from_yaml(&y).unwrap();
        assert_eq!(cfg.providers[0].models[0].weight, 9); // clamped down
        assert_eq!(cfg.providers[0].models[1].weight, 0); // zero allowed
    }

    #[test]
    fn chat_url_formats() {
        let cfg = sample_config();
        assert_eq!(chat_url(&cfg.providers[0]), "https://apihub.example.cn/v1/chat/completions");
        assert_eq!(chat_url(&cfg.providers[1]), "https://api.anthropic.example.com/v1/messages");
        let mut p = cfg.providers[1].clone();
        p.api_base = "https://api.anthropic.example.com/v1".to_string();
        assert_eq!(chat_url(&p), "https://api.anthropic.example.com/v1/messages");
    }

    #[test]
    fn chat_body_shapes() {
        let cfg = sample_config();
        let b = chat_body(&cfg.providers[0], &cfg.providers[0].models[0], "hi", false);
        assert_eq!(b["model"], "agnes-3.0-flash");
        assert_eq!(b["messages"][0]["content"], "hi");
        assert!(b.get("stream").is_none());
        let b = chat_body(&cfg.providers[1], &cfg.providers[1].models[0], "hi", true);
        assert_eq!(b["max_tokens"], DEFAULT_MAX_TOKENS);
        assert_eq!(b["stream"], true);
    }

    #[test]
    fn extract_openai_and_anthropic() {
        let openai = r#"{"choices":[{"message":{"content":"hello world"}}]}"#;
        assert_eq!(extract_content(openai, "openai").unwrap(), "hello world");
        let anthropic = r#"{"content":[{"type":"text","text":"hi"},{"type":"text","text":" there"}]}"#;
        assert_eq!(extract_content(anthropic, "anthropic").unwrap(), "hi there");
        let err = r#"{"error":{"message":"bad key"}}"#;
        assert!(extract_content(err, "openai").is_err());
        let bad = r#"not json"#;
        assert!(extract_content(bad, "openai").is_err());
    }

    #[test]
    fn task_tools_shape_openai_and_anthropic() {
        let cfg = sample_config(); // tasks: weather / 用来获取天气信息
        let openai = task_tools(&cfg, "openai").unwrap();
        assert_eq!(openai[0]["type"], "function");
        assert_eq!(openai[0]["function"]["name"], "weather");
        assert_eq!(openai[0]["function"]["description"], "用来获取天气信息");
        assert_eq!(openai[0]["function"]["parameters"]["required"][0], "input");
        // Declared params become string properties of the schema.
        assert_eq!(openai[0]["function"]["parameters"]["properties"]["lat"]["type"], "string");
        assert_eq!(openai[0]["function"]["parameters"]["properties"]["lon"]["type"], "string");
        let anthropic = task_tools(&cfg, "anthropic").unwrap();
        assert_eq!(anthropic[0]["name"], "weather");
        assert_eq!(anthropic[0]["description"], "用来获取天气信息");
        assert!(anthropic[0].get("input_schema").is_some());
        assert_eq!(anthropic[0]["input_schema"]["properties"]["lat"]["type"], "string");
        // A task name with characters outside [A-Za-z0-9_-] is rejected.
        let mut cfg2 = sample_config();
        cfg2.tasks[0].name = "bad name!".to_string();
        assert!(task_tools(&cfg2, "openai").is_err());
    }

    #[test]
    fn builtin_search_parsers_extract_headlines() {
        // Deterministic parsing of fixed payloads (no network involved).
        let zh = r#"{"stories":[{"title":"标题A","id":123},{"title":"标题B","id":456}]}"#;
        let rows = parse_zhihu(zh).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "标题A");
        assert_eq!(rows[0].1, "https://daily.zhihu.com/story/123");
        let bd = r#"{"data":{"cards":[{"content":[{"content":[{"word":"热词一","url":"https://x/1"},{"word":"热词二","url":"https://x/2"}]}]}]}}"#;
        let rows = parse_baidu(bd).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "热词一");
        assert_eq!(rows[0].1, "https://x/1");
        let bl = r#"{"data":{"list":[{"title":"视频甲","bvid":"BV1"},{"title":"视频乙","bvid":"BV2"}]}}"#;
        let rows = parse_bilibili(bl).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].1, "https://www.bilibili.com/video/BV2");
        let gh = r#"{"items":[{"full_name":"a/b","html_url":"https://github.com/a/b","stargazers_count":999}]}"#;
        let rows = parse_github(gh).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "a/b ★999");
        let hn = r#"{"title":"HN Story","url":"https://example.com","id":1}"#;
        assert_eq!(parse_hn_item(hn).unwrap().unwrap().0, "HN Story");
        // Headline formatting is deterministic.
        assert_eq!(fmt_headlines(&[(String::from("x"), String::from("https://y"))]), "- x (https://y)");
        // days_ago returns a YYYY-MM-DD string.
        let d = days_ago(0);
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
        // Bing fallback parser
        let bing = r#"<ol id="b_results"><li class="b_algo"><h2><a target="_blank" href="https://a.example/p"><strong>标</strong>题一</a></h2></li></ol>"#;
        let rows = parse_bing(bing).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "标题一");
        assert_eq!(rows[0].1, "https://a.example/p");
    }

    #[test]
    fn new_search_parsers_extract_headlines() {
        // toutiao feed JSON
        let tt = r#"{"has_more":true,"data":[{"title":"头条热闻一","source_url":"/group/111/"},{"title":"头条热闻二","source_url":"https://abs.example/x"}]}"#;
        let rows = parse_toutiao(tt).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "头条热闻一");
        assert_eq!(rows[0].1, "https://www.toutiao.com/group/111/");
        assert_eq!(rows[1].1, "https://abs.example/x");
        // Sogou search result page (vr-title blocks)
        let sg = r#"<div class="vrwrap"><h3 class="vr-title"><a id="sogou_snapshot_1" href="/link?url=abc123">德塔文2023年电视剧 微短剧景气指数年榜</a></h3></div><div class="vrwrap"><h3 class="vr-title"><a href="http://mp.weixin.qq.com/s?src=xyz">剧日报|2024年2月15日电视剧景气指数</a></h3></div>"#;
        let rows = parse_sogou(sg).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "德塔文2023年电视剧 微短剧景气指数年榜");
        assert_eq!(rows[0].1, "/link?url=abc123");
        assert_eq!(rows[1].0, "剧日报|2024年2月15日电视剧景气指数");
        assert_eq!(rows[1].1, "http://mp.weixin.qq.com/s?src=xyz");
        // tophub.today board
        let th = r#"<div class="cc-cd-cb"><a href="https://github.com/a/b" target="_blank" rel="nofollow" itemid="1"><div class="cc-cd-cb-ll"><span class="s h">1</span><span class="t">a / b</span><span class="e">1234</span></div></a></div>"#;
        let rows = parse_tophub(th).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "a / b");
        assert_eq!(rows[0].1, "https://github.com/a/b");
        // oschina news list
        let os = r#"<div class="item news-item news-item-hover" data-url="https://www.oschina.net/news/502842"><div class="content"><h3 class="header"><div class="title" title="🔥 开源项目发布新版本">🔥 开源项目发布新版本</div></h3></div></div>"#;
        let rows = parse_oschina(os).unwrap();
        eprintln!("DEBUG oschina rows={:?}", rows);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "🔥 开源项目发布新版本");
        assert_eq!(rows[0].1, "https://www.oschina.net/news/502842");
        // smzdm deals
        let sm = r#"<a href="https://www.smzdm.com/p/183435990/" target="_blank"><div class="feed-hot-pic"></div><div class="feed-hot-title">清洁收纳、今日必买：3M 钢丝球</div><span class="z-highlight">4.9元（需用券）</span></a>"#;
        let rows = parse_smzdm(sm).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "清洁收纳、今日必买：3M 钢丝球 —— 4.9元（需用券）");
        assert_eq!(rows[0].1, "https://www.smzdm.com/p/183435990/");
        // HTML helpers
        assert_eq!(html_unescape("a&amp;b &lt;c&gt; &#39;d&#39; &#233;"), "a&b <c> 'd' é");
        assert_eq!(strip_tags("<b>加粗</b> 与 <i>斜体</i>"), "加粗 与 斜体");
        assert_eq!(urlencode("德塔文 榜单"), "%E5%BE%B7%E5%A1%94%E6%96%87%20%E6%A6%9C%E5%8D%95");
    }

    #[test]
    fn unknown_search_source_rejected() {
        assert!(builtin_search("nope", &HashMap::new()).is_err());
        // Empty api/search on a task is rejected without a network call.
        let task = Task {
            name: "t".into(),
            desc: String::new(),
            msg: String::new(),
            api: None,
            search: None,
            params: None,
        };
        assert!(run_task_tool(&task, &HashMap::new()).is_err());
    }

    #[test]
    fn tool_chat_body_openai_and_anthropic() {
        let cfg = sample_config();
        let tools = task_tools(&cfg, "openai").unwrap();
        let body = tool_chat_body(&cfg.providers[0], &cfg.providers[0].models[0], "今天天气如何", TOOL_SYSTEM, &tools, false);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "今天天气如何");
        assert_eq!(body["tools"][0]["function"]["name"], "weather");
        assert!(body.get("stream").is_none());
        let tools = task_tools(&cfg, "anthropic").unwrap();
        let body = tool_chat_body(&cfg.providers[1], &cfg.providers[1].models[0], "今天天气如何", TOOL_SYSTEM, &tools, true);
        assert_eq!(body["system"], TOOL_SYSTEM);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["tools"][0]["name"], "weather");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn extract_tool_calls_openai_and_anthropic() {
        let openai = r#"{"choices":[{"message":{"tool_calls":[
            {"function":{"name":"weather","arguments":"{\"input\":\"深圳今天天气如何\"}"}}
        ]}}]}"#;
        let calls = extract_tool_calls(openai, "openai").unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "weather");
        assert_eq!(calls[0].1["input"], "深圳今天天气如何");
        let anthropic = r#"{"content":[{"type":"tool_use","name":"weather","input":{"input":"北京天气"}}]}"#;
        let calls = extract_tool_calls(anthropic, "anthropic").unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "weather");
        assert_eq!(calls[0].1["input"], "北京天气");
        // Plain text reply (no tool calls) -> empty vector, not an error.
        let plain = r#"{"choices":[{"message":{"content":"我不知道"}}]}"#;
        assert!(extract_tool_calls(plain, "openai").unwrap().is_empty());
        // Provider error propagates.
        let err = r#"{"error":{"message":"bad key"}}"#;
        assert!(extract_tool_calls(err, "openai").is_err());
    }

    #[test]
    fn substitute_default_and_user_value() {
        let mut params = HashMap::new();
        assert_eq!(substitute("我在{country:深圳}", &params), "我在深圳");
        params.insert("country".to_string(), "北京".to_string());
        assert_eq!(substitute("我在{country:深圳}", &params), "我在北京");
        // no default, no param -> placeholder kept
        assert_eq!(substitute("a {x} b", &params), "a {x} b");
        params.insert("x".to_string(), "y".to_string());
        assert_eq!(substitute("a {x} b", &params), "a y b");
    }

    #[test]
    fn parse_params_colon_and_equal() {
        let items = vec!["country:北京".to_string(), "city=上海".to_string(), "junk".to_string()];
        let map = parse_params(&items);
        assert_eq!(map.get("country").map(String::as_str), Some("北京"));
        assert_eq!(map.get("city").map(String::as_str), Some("上海"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn task_msg_resolution_uses_default() {
        let cfg = sample_config();
        let task = &cfg.tasks[0];
        assert_eq!(task.name, "weather");
        assert_eq!(task.desc, "用来获取天气信息");
        let msg = resolve_task_msg(task, &[]).unwrap();
        assert_eq!(msg, "我在深圳,今天的天气如何，我要询问温度、湿度、下雨概率等信息");
        let msg = resolve_task_msg(task, &["country:北京".to_string()]).unwrap();
        assert_eq!(msg, "我在北京,今天的天气如何，我要询问温度、湿度、下雨概率等信息");
    }

    // ------------------------------------------------------------------
    // weight persistence (update_weight_in_config)
    // ------------------------------------------------------------------

    fn tmp_config(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("sysenv-chat-test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("config-{name}.yaml"));
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn update_weight_conventional_form() {
        let p = tmp_config(
            "conventional",
            r#"model: m1
clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
        weight: 1
      - name: m2
        weight: 3
"#,
        );
        update_weight_in_config(&p, "agnes", "m1", 5).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("      - name: m1\n        weight: 5"));
        assert!(text.contains("      - name: m2\n        weight: 3"));
    }

    #[test]
    fn update_weight_mangled_form() {
        let p = tmp_config(
            "mangled",
            r#"model: agnes-3.0-flash
clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: agnes-3.0-flash
      - weight: 1
"#,
        );
        update_weight_in_config(&p, "agnes", "agnes-3.0-flash", 2).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("      - weight: 2"));
    }

    #[test]
    fn update_weight_inserts_when_missing() {
        let p = tmp_config(
            "insert",
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
"#,
        );
        update_weight_in_config(&p, "agnes", "m1", 4).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("      - name: m1\n        weight: 4"));
    }

    #[test]
    fn update_weight_unknown_model_errors() {
        let p = tmp_config(
            "unknown",
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
"#,
        );
        assert!(update_weight_in_config(&p, "agnes", "nope", 4).is_err());
        assert!(update_weight_in_config(&p, "other", "m1", 4).is_err());
    }

    #[test]
    fn replace_weight_value_preserves_trailing() {
        assert_eq!(replace_weight_value("        weight: 3", 7), "        weight: 7");
        assert_eq!(replace_weight_value("      - weight: 1\t", 2), "      - weight: 2\t");
        assert_eq!(replace_weight_value("        weight: 9 # keep me", 1), "        weight: 1 # keep me");
        assert_eq!(replace_weight_value("no colon here", 3), "no colon here");
    }

    #[test]
    fn update_model_field_inserts_replaces_and_reparses() {
        let p = tmp_config(
            "field",
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
        weight: 1
"#,
        );
        // Insert a missing key after the name line.
        update_model_field_in_config(&p, "agnes", "m1", "max_input_tokens", "1048576").unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("      - name: m1\n        max_input_tokens: 1048576"));
        // Replace an existing key.
        update_model_field_in_config(&p, "agnes", "m1", "weight", "5").unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("        weight: 5"));
        // Insert the type field.
        update_model_field_in_config(&p, "agnes", "m1", "type", "text,image").unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("        type: text,image"));
        // The file must still parse back into the same model entry.
        let y = parse_yaml(&t).unwrap();
        let cfg = config_from_yaml(&y).unwrap();
        let m = &cfg.providers[0].models[0];
        assert_eq!(m.max_input_tokens, Some(1048576));
        assert_eq!(m.model_type.as_deref(), Some("text,image"));
        assert_eq!(m.weight, 5);
        // Unknown model still errors.
        assert!(update_model_field_in_config(&p, "agnes", "nope", "type", "text").is_err());
    }

    #[test]
    fn parse_score_extracts_0_to_10() {
        assert_eq!(parse_score("9"), Some(9));
        assert_eq!(parse_score("8/10"), Some(8));
        assert_eq!(parse_score("匹配分数：10"), Some(10));
        assert_eq!(parse_score("score: 4.5"), Some(4));
        assert_eq!(parse_score("完全匹配"), None);
        assert_eq!(parse_score(""), None);
        assert_eq!(parse_score("7.9 分"), Some(7));
    }

    #[test]
    fn truncate_to_limit_cuts_only_when_needed() {
        assert_eq!(truncate_to_limit("你好世界", Some(2)), "你好");
        assert_eq!(truncate_to_limit("你好世界", Some(4)), "你好世界");
        assert_eq!(truncate_to_limit("你好世界", None), "你好世界");
        assert_eq!(truncate_to_limit("abcdef", Some(3)), "abc");
    }

    #[test]
    fn model_parses_capability_fields() {
        let y = parse_yaml(
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
        max_input_tokens: 4096
        type: text,image
"#,
        )
        .unwrap();
        let cfg = config_from_yaml(&y).unwrap();
        let m = &cfg.providers[0].models[0];
        assert_eq!(m.max_input_tokens, Some(4096));
        assert_eq!(m.model_type.as_deref(), Some("text,image"));
        assert_eq!(m.max_tokens, None);
    }
}
