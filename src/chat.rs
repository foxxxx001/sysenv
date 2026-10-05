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
}

#[derive(Debug, Clone)]
struct Task {
    name: String,
    desc: String,
    msg: String,
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
                (YVal::Scalar(unquote(&value)), i + 1)
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
                (YVal::Scalar(unquote(&v2)), i + 1)
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
                models.push(Model { name: mname.to_string(), weight, max_tokens });
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
            tasks.push(Task { name, desc, msg });
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
fn replace_weight_value(line: &str, new_value: u32) -> String {
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
                            // Mangled form: the very next line is `- weight: N`
                            // at the same indent.
                            if i + 1 < lines.len() {
                                let (ni, nc) = line_parts(&lines[i + 1]);
                                if nc.starts_with("- weight:") && ni == name_indent {
                                    lines[i + 1] = replace_weight_value(&lines[i + 1], new_weight);
                                    done = true;
                                    break;
                                }
                            }
                            // Conventional form: a deeper `weight:` sub-key
                            // anywhere inside this model entry.
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
                                if nc.starts_with("weight:") {
                                    lines[j] = replace_weight_value(&lines[j], new_weight);
                                    done = true;
                                    break;
                                }
                                j += 1;
                            }
                            if !done {
                                // No weight line: insert one after the name line.
                                let pad = " ".repeat(name_indent + 2);
                                lines.insert(i + 1, format!("{pad}weight: {new_weight}"));
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

/// Send one chat request. Returns the assistant text (streaming prints as it goes).
fn send_chat(p: &Provider, m: &Model, msg: &str, stream: bool, debug: bool) -> Result<String> {
    let url = chat_url(p);
    let body = chat_body(p, m, msg, stream);
    let body_bytes = serde_json::to_vec(&body).context("cannot serialize request body")?;

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
    extract_content(&text, &p.kind)
}

// ---------------------------------------------------------------------------
// Task templates
// ---------------------------------------------------------------------------

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
                    Some(v) => out.push_str(v),
                    None => match def {
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
    let map = parse_params(params);
    Ok(substitute(&raw, &map))
}

/// Chat with the configured providers using the given message.
///
/// `model_override` (the `-m/--model` argument) takes precedence over the
/// top-level `model` from the config; both may hold comma-separated selectors.
/// Weighted round-robin picks the first candidate; on request failure the
/// failed model's weight is decremented (>1) and persisted to the config, and
/// the next model in order is tried. A model that succeeds after a failure has
/// its weight incremented (<9) and persisted.
fn chat_with(cfg: &Config, path: &Path, msg: &str, debug: bool, no_stream: bool, model_override: Option<&str>) -> Result<()> {
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
        let p = &cfg.providers[t.provider];
        let m = &p.models[t.model];
        eprintln!("sysenv: using {} / {} ({})", p.name, m.name, p.kind);
        match send_chat(p, m, msg, stream, debug) {
            Ok(text) => {
                // A model that succeeded after a previous failure gets its
                // weight bumped (max 9), persisted to the config file.
                if had_failure && m.weight < 9 {
                    if let Err(e) = update_weight_in_config(path, &p.name, &m.name, m.weight + 1) {
                        eprintln!("sysenv: warning: failed to persist weight bump for {} / {}: {e:#}", p.name, m.name);
                    }
                }
                if !stream {
                    println!("{text}");
                }
                return Ok(());
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
    let (cfg, path) = load_config(config)?;

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
    chat_with(&cfg, &path, &msg, debug, no_stream, model)
}

/// `sysenv ai task [-t NAME] [key:value...] [-c FILE] [--debug] [--no-stream]`
pub fn cmd_task(
    sel: Option<&str>,
    params: &[String],
    config: Option<&Path>,
    debug: bool,
    no_stream: bool,
) -> Result<()> {
    let (cfg, path) = load_config(config)?;
    match sel {
        None => {
            if cfg.tasks.is_empty() {
                println!("(no tasks configured under `tasks` in {})", path.display());
                return Ok(());
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
            chat_with(&cfg, &path, &msg, debug, no_stream, None)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(tasks.len(), 1);
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
        assert_eq!(cfg.tasks.len(), 1);
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
}
