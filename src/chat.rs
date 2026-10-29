//! `sys ai chat` — chat with LLM providers configured in
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
use std::time::{Duration, Instant};

const DEFAULT_MAX_TOKENS: u64 = 1024;
pub(crate) const ANTHROPIC_VERSION: &str = "2023-06-01";
const STREAM_TIMEOUT: Duration = Duration::from_secs(600);
/// `sys ai chat --test` timeouts: connect and absolute cap for one model
/// test (only the first token is awaited).
const TEST_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TEST_TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
/// `sys ai chat --query-model` timeouts for the provider's models call.
const QUERY_MODEL_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const QUERY_MODEL_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const USER_AGENT_STR: &str = concat!("sys/", env!("CARGO_PKG_VERSION"));

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct Config {
    pub(crate) model: Option<String>,
    pub(crate) stream: bool,
    pub(crate) providers: Vec<Provider>,
    /// Built-in free-routers mode (--free or missing config): no config file,
    /// weight rotation state is kept in memory and nothing is written back.
    pub(crate) free: bool,
    /// Web-search API keys configured under the top-level `search` key
    /// (e.g. `- name: bochaai, key: sk-...`), used by `sys ai chat --web`.
    /// The engine is detected from the entry name: containing `yuanbao`
    /// drives Tencent Yuanbao Search, everything else drives Bocha.
    pub(crate) search: Vec<SearchKey>,
}

#[derive(Debug, Clone)]
pub(crate) struct SearchKey {
    pub(crate) name: String,
    pub(crate) key: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Provider {
    /// "openai" or "anthropic" (anything else defaults to openai)
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) api_base: String,
    pub(crate) api_key: String,
    pub(crate) models: Vec<Model>,
}

#[derive(Debug, Clone)]
pub(crate) struct Model {
    pub(crate) name: String,
    pub(crate) weight: u32,
    pub(crate) max_tokens: Option<u64>,
    /// Max input length (chars) enforced before sending; when absent no
    /// truncation happens (values come from the config only, never fetched
    /// unless `sys ai chat --update` is given).
    pub(crate) max_input_tokens: Option<u64>,
    /// Modality type (e.g. `text,image`) read from the config as-is; only
    /// `sys ai chat --update` consults it to decide whether to fill it in.
    pub(crate) model_type: Option<String>,
    /// Model groups read from the config's `group:` field (comma-separated,
    /// several allowed). Selectors `group:<name>` in the top-level `model:`
    /// or `-m` pick every model whose groups contain the name.
    pub(crate) group: Vec<String>,
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
pub(crate) fn load_config(path: Option<&Path>) -> Result<(Config, PathBuf)> {
    let path = resolve_config_path(path)?;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read config `{}`", path.display()))?;
    let y = parse_yaml(&text)?;
    let cfg = config_from_yaml(&y).with_context(|| format!("invalid config `{}`", path.display()))?;
    Ok((cfg, path))
}

// --- Built-in free routers (`--free`, or when config.yaml is absent) --------

/// Kilo Gateway free auto-router (keyless / anonymous).
const FREE_KILO_BASE: &str = "https://ai-gateway.kilo.ai/api/v1";
/// BlockRun free anonymous endpoint (keyless; model: nvidia/llama-3.2-11b-vision).
const FREE_BLOCKRUN_BASE: &str = "https://blockrun.ai/api/v1";
/// OpenRouter free models router (needs OPENROUTER_API_KEY).
const FREE_OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";

fn free_model(name: &str) -> Model {
    Model {
        name: name.to_string(),
        weight: 1,
        max_tokens: None,
        max_input_tokens: None,
        model_type: None,
        group: Vec::new(),
    }
}

/// Free providers: `kilo-auto/free` and `blockrun/free` always (anonymous);
/// `openrouter/free` only when a non-empty OpenRouter key is supplied.
fn free_providers(openrouter_key: Option<String>) -> Vec<Provider> {
    let mut providers = vec![
        Provider {
            kind: "openai".to_string(),
            name: "kilo-auto".to_string(),
            api_base: FREE_KILO_BASE.to_string(),
            api_key: String::new(),
            models: vec![free_model("kilo-auto/free")],
        },
        Provider {
            kind: "openai".to_string(),
            name: "blockrun".to_string(),
            api_base: FREE_BLOCKRUN_BASE.to_string(),
            api_key: String::new(),
            models: vec![free_model("nvidia/llama-3.2-11b-vision")],
        },
    ];
    if let Some(key) = openrouter_key.filter(|k| !k.trim().is_empty()) {
        providers.push(Provider {
            kind: "openai".to_string(),
            name: "openrouter".to_string(),
            api_base: FREE_OPENROUTER_BASE.to_string(),
            api_key: key,
            models: vec![free_model("openrouter/free")],
        });
    }
    providers
}

/// Parse a `--free[=ROUTER]` selector to the matching built-in provider name.
/// Accepts kilo / kilo-auto / kilo-auto/free, blockrun / blockrun/free and
/// openrouter / openrouter/free (case-insensitive); unknown values are
/// rejected.
pub(crate) fn parse_free_router(s: &str) -> Result<&'static str> {
    match s.trim().to_ascii_lowercase().as_str() {
        "kilo" | "kilo-auto" | "kilo-auto/free" => Ok("kilo-auto"),
        "blockrun" | "blockrun/free" => Ok("blockrun"),
        "openrouter" | "openrouter/free" => Ok("openrouter"),
        other => bail!("unknown free router `{other}` (use kilo, blockrun or openrouter)"),
    }
}

/// Apply a `--free[=router]` selector to an already-built free provider
/// list. `None` (or `"all"`) keeps every router; a named router filters the
/// list to it and bails when it is unavailable (openrouter needs
/// `OPENROUTER_API_KEY`).
fn filter_free_selector(providers: Vec<Provider>, selector: Option<&str>) -> Result<Config> {
    let mut cfg = Config {
        model: None,
        stream: false,
        providers,
        free: true,
        search: Vec::new(),
    };
    let Some(sel) = selector.map(str::trim).filter(|s| !s.is_empty() && *s != "all") else {
        return Ok(cfg);
    };
    let want = parse_free_router(sel)?;
    cfg.providers.retain(|p| p.name == want);
    if cfg.providers.is_empty() {
        bail!(
            "free router `{want}` is unavailable: {}",
            if want == "openrouter" {
                "set OPENROUTER_API_KEY"
            } else {
                "internal error"
            }
        );
    }
    cfg.model = Some(cfg.providers[0].models[0].name.clone());
    Ok(cfg)
}

/// Build the free Config from the environment, optionally pinned to one
/// router (`kilo` | `blockrun` | `openrouter`).
pub(crate) fn free_config(selector: Option<&str>) -> Result<Config> {
    let key = std::env::var("OPENROUTER_API_KEY").ok();
    filter_free_selector(free_providers(key), selector)
}

/// Does the default `~/.sysenv/config.yaml` exist? Used to fall back to the
/// free routers when no config file is present.
fn default_config_exists() -> bool {
    home_dir()
        .map(|h| h.join(".sysenv").join("config.yaml").exists())
        .unwrap_or(false)
}

/// Free mode on either trigger: `--free` forces it, OR the default
/// config.yaml was not found (when no explicit `-c` file was named).
fn use_free_mode(free_flag: bool, explicit_config: bool, default_exists: bool) -> bool {
    free_flag || (!explicit_config && !default_exists)
}

// --- minimal block-YAML parser (scalars / lists / maps, tab-tolerant) ------

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum YVal {
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

pub(crate) fn parse_yaml(text: &str) -> Result<YVal> {
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
                let group = item
                    .get("group")
                    .and_then(|v| v.scalar())
                    .map(|s| {
                        s.split([',', '，'])
                            .map(str::trim)
                            .filter(|g| !g.is_empty())
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                models.push(Model {
                    name: mname.to_string(),
                    weight,
                    max_tokens,
                    max_input_tokens,
                    model_type,
                    group,
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
        // A provider's identity is its api_base: blocks sharing the endpoint
        // are the same provider and must live in one block (informational —
        // both blocks still load). Same name at a different endpoint is a
        // distinct provider and is NOT a duplicate.
        let mut seen: Vec<String> = Vec::new();
        for c in clients {
            let p = provider_from_yaml(c)?;
            let base = p.api_base.trim_end_matches('/').to_ascii_lowercase();
            if !base.is_empty() && seen.iter().any(|s| *s == base) {
                eprintln!(
                    "sys: warning: duplicate provider (same api_base) `{}` in config; models of the same provider should live in one block",
                    p.api_base
                );
            }
            seen.push(base);
            providers.push(p);
        }
    }
    let mut search = Vec::new();
    if let Some(list) = y.get("search").and_then(|v| v.list()) {
        for s in list {
            let name = s.get("name").and_then(|v| v.scalar()).unwrap_or("").trim().to_string();
            let key = s.get("key").and_then(|v| v.scalar()).unwrap_or("").trim().to_string();
            if !name.is_empty() && !key.is_empty() {
                search.push(SearchKey { name, key });
            }
        }
    }
    Ok(Config { model, stream, providers, free: false, search })
}

// ---------------------------------------------------------------------------
// Model selection (top-level `model`)
// ---------------------------------------------------------------------------

fn ci_eq(a: &str, b: &str) -> bool {
    a.to_ascii_lowercase() == b.to_ascii_lowercase()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Target {
    pub(crate) provider: usize,
    pub(crate) model: usize,
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
/// - `group:<name>`    -> every model across all providers whose `group`
///   list contains the name (case-insensitive)
///
/// The selector may contain several comma-separated (half- or full-width)
/// selectors, each resolved by the rules above; the union is returned.
pub(crate) fn resolve_targets(cfg: &Config, selector: &str) -> Result<Vec<Target>> {
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
        let lower = sel.to_ascii_lowercase();
        if let Some(name) = lower.strip_prefix("group:") {
            let name = name.trim();
            if name.is_empty() {
                bail!("empty group name in selector `{sel}`");
            }
            let mut found = false;
            for (pi, p) in cfg.providers.iter().enumerate() {
                for (mi, m) in p.models.iter().enumerate() {
                    if m.group.iter().any(|g| ci_eq(g, name)) {
                        out.push(Target { provider: pi, model: mi });
                        found = true;
                    }
                }
            }
            if !found {
                let mut seen: Vec<&str> = Vec::new();
                for p in &cfg.providers {
                    for m in &p.models {
                        for g in &m.group {
                            if !seen.iter().any(|s| ci_eq(s, g)) {
                                seen.push(g);
                            }
                        }
                    }
                }
                if seen.is_empty() {
                    bail!("no model belongs to group `{name}` (no groups configured; add `group:` to models in config.yaml)");
                }
                bail!(
                    "no model belongs to group `{name}` (available groups: {})",
                    seen.join(", ")
                );
            }
            continue;
        }
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
pub(crate) fn pick_weighted(cfg: &Config, targets: &[Target], selector: &str, config_path: &Path) -> Result<usize> {
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
/// `weight:` sub-key is inserted after the `- name:` line. The first matching
/// model entry is updated.
pub(crate) fn update_weight_in_config(config_path: &Path, provider_name: &str, model_name: &str, new_weight: u32) -> Result<()> {
    update_model_field_in_config(config_path, provider_name, model_name, "weight", &new_weight.to_string(), 0)
}

/// Like `update_weight_in_config`, but targets the `occurrence`-th (0-based)
/// model entry whose name matches, so duplicate model names under one provider
/// can each be updated individually (used by `sys ai chat --test`, where every
/// configured entry is a separate target).
pub(crate) fn update_weight_in_config_at(
    config_path: &Path,
    provider_name: &str,
    model_name: &str,
    occurrence: usize,
    new_weight: u32,
) -> Result<()> {
    update_model_field_in_config(config_path, provider_name, model_name, "weight", &new_weight.to_string(), occurrence)
}

/// Update a scalar field `key` of the `occurrence`-th (0-based) model entry
/// named `model_name` under `provider_name` in the YAML config file, writing
/// the change back in place (comments and layout are preserved). For
/// `key == "weight"` the mangled form (`- weight: N` as a separate list item
/// right after `- name:`) is also handled. When no such key exists in the
/// model entry, a new `key: value` sub-key is inserted after the `- name:`
/// line.
fn update_model_field_in_config(
    config_path: &Path,
    provider_name: &str,
    model_name: &str,
    key: &str,
    value: &str,
    occurrence: usize,
) -> Result<()> {
    let text = std::fs::read_to_string(config_path)
        .with_context(|| format!("cannot read config `{}`", config_path.display()))?;
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();

    let mut provider_indent: Option<usize> = None;
    let mut cur_provider: Option<String> = None;
    let mut in_models = false;
    let mut models_indent = 0usize;
    let mut occ = 0usize;

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
                            if occ != occurrence {
                                occ += 1;
                                i += 1;
                                continue;
                            }
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

/// Remove the `occurrence`-th (0-based) model entry named `model_name` under
/// `provider_name` from the YAML config file (used by `sys ai chat --test
/// --remove`). Deletes the `- name:` line, its deeper sub-keys, and the
/// mangled same-indent `- weight:` continuation; a later entry at the same
/// indent is left untouched. Returns an error when the entry is not found.
fn remove_model_entry_from_config(
    config_path: &Path,
    provider_name: &str,
    model_name: &str,
    occurrence: usize,
) -> Result<()> {
    let text = std::fs::read_to_string(config_path)
        .with_context(|| format!("cannot read config `{}`", config_path.display()))?;
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();

    let mut provider_indent: Option<usize> = None;
    let mut cur_provider: Option<String> = None;
    let mut in_models = false;
    let mut models_indent = 0usize;
    let mut occ = 0usize;
    let mut target: Option<usize> = None;

    let mut i = 0usize;
    while i < lines.len() && target.is_none() {
        let (indent, content) = line_parts(&lines[i]);
        if content.is_empty() {
            i += 1;
            continue;
        }
        let is_dash = content.starts_with('-');

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
                    // A new sibling provider entry.
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
                if let Some((k, v)) = split_kv_opt(rest) {
                    if k == "name" {
                        let mname = unquote(&v);
                        let provider_ok = cur_provider
                            .as_deref()
                            .map(|p| ci_eq(p, provider_name))
                            .unwrap_or(false);
                        if provider_ok && ci_eq(&mname, model_name) {
                            if occ == occurrence {
                                target = Some(i);
                            } else {
                                occ += 1;
                            }
                        }
                    }
                }
            }
            i += 1;
            continue;
        }

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

    let start = target.ok_or_else(|| {
        anyhow::anyhow!("cannot locate model `{model_name}` of provider `{provider_name}` in `{}`", config_path.display())
    })?;
    let name_indent = line_parts(&lines[start]).0;
    // The entry ends at the next model entry (same indent, not `- weight:`),
    // at the end of the models list, or at a shallower indent.
    let mut end = start + 1;
    while end < lines.len() {
        let (indent, content) = line_parts(&lines[end]);
        if content.is_empty() {
            end += 1;
            continue;
        }
        if indent > name_indent {
            end += 1;
            continue;
        }
        if indent == name_indent && content.starts_with("- weight:") {
            // Mangled form: the weight was written as a same-indent list item
            // and belongs to this entry.
            end += 1;
            continue;
        }
        break;
    }
    lines.drain(start..end);
    std::fs::write(config_path, lines.join("\n"))
        .with_context(|| format!("cannot write config `{}`", config_path.display()))
}

/// Remove the whole provider entry named `provider_name` from the YAML config
/// file (used by `sys ai chat --test --remove` when a provider is left
/// without any models). Returns an error when the provider is not found.
fn remove_provider_entry_from_config(config_path: &Path, provider_name: &str) -> Result<()> {
    let text = std::fs::read_to_string(config_path)
        .with_context(|| format!("cannot read config `{}`", config_path.display()))?;
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();

    let mut provider_indent: Option<usize> = None;
    let mut entry_start: Option<usize> = None;
    let mut cur_provider: Option<String> = None;
    let mut range: Option<(usize, usize)> = None;

    let mut i = 0usize;
    while i < lines.len() && range.is_none() {
        let (indent, content) = line_parts(&lines[i]);
        if content.is_empty() {
            i += 1;
            continue;
        }
        let is_dash = content.starts_with('-');

        if let Some(pi) = provider_indent {
            if indent < pi {
                // Left the clients list: the current provider ended here.
                if let Some(s) = entry_start {
                    if cur_provider.as_deref().map(|c| ci_eq(c, provider_name)).unwrap_or(false) {
                        range = Some((s, i));
                        break;
                    }
                }
                provider_indent = None;
                entry_start = None;
                cur_provider = None;
                i += 1;
                continue;
            }
        }

        if is_dash {
            let rest = content[1..].trim_start();
            if let Some(pi) = provider_indent {
                if indent == pi {
                    // A new sibling provider entry: the previous one ended here.
                    if let Some(s) = entry_start {
                        if cur_provider.as_deref().map(|c| ci_eq(c, provider_name)).unwrap_or(false) {
                            range = Some((s, i));
                            break;
                        }
                    }
                    entry_start = Some(i);
                    cur_provider = None;
                    if let Some((k, v)) = split_kv_opt(rest) {
                        if k == "name" {
                            cur_provider = Some(unquote(&v));
                        }
                    }
                    i += 1;
                    continue;
                }
            } else {
                provider_indent = Some(indent);
                entry_start = Some(i);
                cur_provider = None;
                if let Some((k, v)) = split_kv_opt(rest) {
                    if k == "name" {
                        cur_provider = Some(unquote(&v));
                    }
                }
                i += 1;
                continue;
            }
            i += 1;
            continue;
        }

        if let Some((k, v)) = split_kv_opt(&content) {
            if provider_indent.is_some() && k == "name" {
                cur_provider = Some(unquote(&v));
            }
        }
        i += 1;
    }
    // The target provider is the last entry in the list: it ends at EOF.
    if range.is_none() {
        if let Some(s) = entry_start {
            if cur_provider.as_deref().map(|c| ci_eq(c, provider_name)).unwrap_or(false) {
                range = Some((s, lines.len()));
            }
        }
    }

    let (start, end) = range.ok_or_else(|| {
        anyhow::anyhow!("cannot locate provider `{provider_name}` in `{}`", config_path.display())
    })?;
    lines.drain(start..end);
    std::fs::write(config_path, lines.join("\n"))
        .with_context(|| format!("cannot write config `{}`", config_path.display()))
}

/// Drop dangling model selectors from the top-level `model:` line: parts that
/// no longer resolve to any remaining configured model (deleted models, fully
/// removed providers) are removed; when nothing is left the `model:` line is
/// deleted. No-op when the config has no top-level `model:`.
fn clean_top_level_model_selector(config_path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(config_path)
        .with_context(|| format!("cannot read config `{}`", config_path.display()))?;
    let y = parse_yaml(&text)?;
    let cfg = config_from_yaml(&y)
        .with_context(|| format!("invalid config `{}`", config_path.display()))?;
    let Some(sel) = cfg.model.as_deref() else {
        return Ok(());
    };
    let mut keep: Vec<String> = Vec::new();
    for part in split_selector(sel) {
        if let Some((prov, m)) = part.split_once(':') {
            let prov = prov.trim();
            let m = m.trim();
            if m == "*" {
                if cfg.providers.iter().any(|p| ci_eq(&p.name, prov) && !p.models.is_empty()) {
                    keep.push(part.clone());
                }
            } else if cfg
                .providers
                .iter()
                .any(|p| ci_eq(&p.name, prov) && p.models.iter().any(|mdl| ci_eq(&mdl.name, m)))
            {
                keep.push(part.clone());
            }
        } else if cfg
            .providers
            .iter()
            .any(|p| p.models.iter().any(|mdl| ci_eq(&mdl.name, &part)))
        {
            keep.push(part.clone());
        }
    }

    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let mut done = false;
    for line in &mut lines {
        let (indent, content) = line_parts(line);
        if content.is_empty() {
            continue;
        }
        if indent != 0 {
            continue;
        }
        if let Some((k, _)) = split_kv_opt(&content) {
            if k == "model" {
                if keep.is_empty() {
                    *line = String::new();
                } else {
                    *line = format!("model: {}", keep.join(","));
                }
                done = true;
                break;
            }
        }
    }
    if !done {
        return Ok(());
    }
    std::fs::write(config_path, lines.join("\n"))
        .with_context(|| format!("cannot write config `{}`", config_path.display()))
}

/// Apply `--remove` results: back up the original config, delete the failed
/// model entries, delete providers left without any models, and clean the
/// top-level `model:` selector. `removals` holds (provider_idx, model_idx)
/// into `cfg`.
fn apply_test_removals(cfg: &Config, path: &Path, removals: &[(usize, usize)]) -> Result<()> {
    if removals.is_empty() {
        return Ok(());
    }
    // Back up the original config before deleting anything.
    let backup = PathBuf::from(format!(
        "{}.bak-{}",
        path.display(),
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    std::fs::copy(path, &backup)
        .with_context(|| format!("cannot back up config to `{}`", backup.display()))?;
    println!("sys: backed up original config to `{}`", backup.display());
    // Duplicate model names under one provider are targeted by occurrence.
    let mut occ: HashMap<(usize, String), usize> = HashMap::new();
    for &(pi, mi) in removals {
        let p = &cfg.providers[pi];
        let m = &p.models[mi];
        let o = occ.entry((pi, m.name.clone())).or_insert(0);
        remove_model_entry_from_config(path, &p.name, &m.name, *o)?;
        *o += 1;
    }
    // Providers left with no models are removed entirely.
    let mut removed_by_provider: HashMap<usize, Vec<usize>> = HashMap::new();
    for &(pi, mi) in removals {
        removed_by_provider.entry(pi).or_default().push(mi);
    }
    for (pi, mis) in removed_by_provider {
        let remaining = cfg.providers[pi].models.iter().enumerate().any(|(i, _)| !mis.contains(&i));
        if !remaining {
            remove_provider_entry_from_config(path, &cfg.providers[pi].name)?;
        }
    }
    clean_top_level_model_selector(path)
}

// ---------------------------------------------------------------------------
// Chat requests (OpenAI / Anthropic)
// ---------------------------------------------------------------------------

pub(crate) fn chat_url(p: &Provider) -> String {
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

/// The provider's model-list endpoint. For OpenAI-compatible providers the
/// `models` path is appended to the configured api_base as-is — the models
/// address is not always under `v1` (e.g. Baidu Qianfan serves the list at
/// `https://qianfan.baidubce.com/v2/models`, not `/v2/v1/models`). Anthropic
/// uses `{api_base}/v1/models`, or the plain `/models` suffix when api_base
/// already ends with `/v1` (the common convention).
pub(crate) fn models_url(p: &Provider) -> String {
    let base = p.api_base.trim_end_matches('/');
    if p.kind == "anthropic" {
        if base.ends_with("/v1") {
            format!("{base}/models")
        } else {
            format!("{base}/v1/models")
        }
    } else {
        format!("{base}/models")
    }
}

/// Parse one page of a models response: entries come from `data[]` (OpenAI
/// style) or `models[]` (fallback), each entry contributing its `id` plus the
/// optional `owned_by`; a `next_page` token (when present) is returned for
/// pagination. A leading UTF-8 BOM is stripped before strict JSON parsing, and
/// an `error` object in the body is reported as an Err.
fn parse_models_response(body: &str) -> Result<(Vec<(String, Option<String>)>, Option<String>)> {
    let body = body.trim_start_matches('\u{feff}').trim();
    if body.is_empty() {
        bail!("empty response from the models endpoint");
    }
    let v: Value = serde_json::from_str(body).context("invalid JSON from the models endpoint")?;
    if let Some(err) = v.get("error") {
        let raw = err.to_string();
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or(&raw);
        bail!("provider error: {msg}");
    }
    let arr = v
        .get("data")
        .and_then(|d| d.as_array())
        .or_else(|| v.get("models").and_then(|m| m.as_array()))
        .with_context(|| {
            format!(
                "unexpected models response (expected a `data`/`models` array): {}",
                body.chars().take(200).collect::<String>()
            )
        })?;
    let mut entries = Vec::with_capacity(arr.len());
    for item in arr {
        let id = item
            .get("id")
            .and_then(|i| i.as_str())
            .with_context(|| format!("model entry without an `id`: {item}"))?
            .to_string();
        let owned_by = item.get("owned_by").and_then(|o| o.as_str()).map(|s| s.to_string());
        entries.push((id, owned_by));
    }
    let next_page = v
        .get("next_page")
        .and_then(|n| n.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    Ok((entries, next_page))
}

/// `sys ai chat --query-model PROVIDER`: query the provider's model list via
/// its models endpoint (`{api_base}/models` for OpenAI-compatible,
/// `{api_base}/v1/models` for Anthropic). Pagination (`next_page`) is followed
/// with a page cap; duplicate ids are dropped.
pub(crate) fn query_provider_models(cfg: &Config, provider_name: &str) -> Result<()> {
    let p = cfg
        .providers
        .iter()
        .find(|p| ci_eq(&p.name, provider_name))
        .with_context(|| {
            format!(
                "no provider named `{provider_name}` (available: {})",
                cfg.providers.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
            )
        })?;
    let url = models_url(p);
    let entries = fetch_provider_models(p, &url)?;
    let mut seen = std::collections::HashSet::new();
    let ids: Vec<&str> = entries
        .iter()
        .map(|(id, _)| id.as_str())
        .filter(|id| seen.insert(id.to_string()))
        .collect();
    println!("{} ({}) — {}  ({} model(s) via API)", p.name, p.kind, url, ids.len());
    for id in &ids {
        println!("- {id}");
    }
    Ok(())
}

/// HTTP part of `--query-model`: GET the models endpoint (following `next_page`
/// up to 20 pages) and return the raw entries.
fn fetch_provider_models(p: &Provider, first_url: &str) -> Result<Vec<(String, Option<String>)>> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR));
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    if p.kind == "anthropic" {
        headers.insert("x-api-key", HeaderValue::from_str(&p.api_key).context("invalid api_key")?);
        headers.insert("anthropic-version", HeaderValue::from_static(ANTHROPIC_VERSION));
    } else {
        let auth = format!("Bearer {}", p.api_key);
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&auth).context("invalid api_key")?);
    }
    let client = Client::builder()
        .connect_timeout(QUERY_MODEL_CONNECT_TIMEOUT)
        .timeout(QUERY_MODEL_TIMEOUT)
        .build()
        .context("failed to build HTTP client")?;
    let mut out = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();
    let mut next: Option<String> = None;
    for _ in 0..20 {
        let url = match &next {
            None => first_url.to_string(),
            Some(n) => format!("{first_url}?next_page={n}"),
        };
        if !seen_urls.insert(url.clone()) {
            break;
        }
        let resp = client
            .get(&url)
            .headers(headers.clone())
            .send()
            .with_context(|| format!("request to {url} failed"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            let snippet: String = text.chars().take(300).collect::<String>().trim().to_string();
            bail!("{url} returned HTTP {status}: {snippet}");
        }
        let text = resp.text().context("cannot read the models response")?;
        let (entries, np) = parse_models_response(&text)?;
        out.extend(entries);
        match np {
            None => break,
            Some(tok) => next = Some(tok),
        }
    }
    Ok(out)
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

pub(crate) fn debug_print_request(url: &str, headers: &HeaderMap, body: &[u8]) {
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

pub(crate) fn debug_print_response(status: reqwest::StatusCode, headers: &HeaderMap, body: &str) {
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

// ---------------------------------------------------------------------------
// `sys ai chat --web[=PROVIDER]`: web search providers
// (Bocha `POST /v1/web-search` / Tencent Yuanbao `SearchPro`)
// ---------------------------------------------------------------------------

/// Bocha AI web search endpoint (official: `POST /v1/web-search`).
const BOCHA_API: &str = "https://api.bochaai.com/v1/web-search";

/// Tencent Yuanbao Search (WSA) service-API-KEY endpoint (`SearchPro`).
/// Docs: https://cloud.tencent.com/document/product/1806/130615
const YUANBAO_API: &str = "https://api.wsa.cloud.tencent.com/SearchPro";

/// The web-search engine an entry drives, detected from its `name`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchEngine {
    /// 博查（Bocha AI，`POST /v1/web-search`）。
    Bocha,
    /// 腾讯元宝搜索（WSA，服务 API KEY 方式 `POST /SearchPro`）。
    Yuanbao,
}

/// Map an entry name to the engine it drives: containing `yuanbao` selects
/// Tencent Yuanbao Search, every other name drives Bocha.
fn search_engine_of(name: &str) -> SearchEngine {
    if name.to_ascii_lowercase().contains("yuanbao") {
        SearchEngine::Yuanbao
    } else {
        SearchEngine::Bocha
    }
}

/// Resolve the web-search entry for `--web[=PROVIDER]`. `sel` is the optional
/// `--web` value: `None` / `"auto"` prefer the first entry whose name mentions
/// `bocha`, then one mentioning `yuanbao`, then the first entry; `"bocha"` /
/// `"yuanbao"` pin that engine (the first entry whose name contains the word);
/// anything else must match an entry name (case-insensitive substring).
/// Missing entries produce a clear error pointing at the config path.
fn pick_search_entry<'a>(cfg: &'a Config, sel: Option<&str>, path: &Path) -> Result<&'a SearchKey> {
    let sel = sel.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("auto");
    let sel_low = sel.to_ascii_lowercase();
    let entry = match sel_low.as_str() {
        "auto" => cfg
            .search
            .iter()
            .find(|s| s.name.to_ascii_lowercase().contains("bocha"))
            .or_else(|| cfg.search.iter().find(|s| s.name.to_ascii_lowercase().contains("yuanbao")))
            .or_else(|| cfg.search.first()),
        "bocha" => cfg.search.iter().find(|s| s.name.to_ascii_lowercase().contains("bocha")),
        "yuanbao" => cfg.search.iter().find(|s| s.name.to_ascii_lowercase().contains("yuanbao")),
        other => cfg.search.iter().find(|s| s.name.to_ascii_lowercase().contains(other)),
    }
    .ok_or_else(|| match sel_low.as_str() {
        "auto" => anyhow::anyhow!(
            "no web-search key configured: add a `search:` entry with `name: bocha` (Bocha) or `name: yuanbao` (腾讯元宝搜索) and its key to {}",
            path.display()
        ),
        "bocha" => anyhow::anyhow!(
            "no Bocha search key configured: add a `search:` entry like `- name: bochaai, key: sk-...` to {}",
            path.display()
        ),
        "yuanbao" => anyhow::anyhow!(
            "no Yuanbao search key configured: add a `search:` entry like `- name: yuanbao, key: <API KEY>` to {}",
            path.display()
        ),
        _ => anyhow::anyhow!(
            "unknown search provider `{sel}` (expected: auto | bocha | yuanbao); add a matching `search:` entry to {}",
            path.display()
        ),
    })?;
    if entry.key.trim().is_empty() {
        bail!("search entry `{}` has an empty key in {}", entry.name, path.display());
    }
    Ok(entry)
}

/// Run the web search for `--web[=PROVIDER]` and return the formatted result
/// text: Bocha entries get the usual 5-row summary search, Yuanbao entries a
/// `SearchPro` request (10 rows requested, the best 5 kept).
fn web_search(cfg: &Config, sel: Option<&str>, path: &Path, msg: &str, debug: bool) -> Result<String> {
    let entry = pick_search_entry(cfg, sel, path)?;
    match search_engine_of(&entry.name) {
        SearchEngine::Bocha => bocha_search(&entry.key, msg, None, true, 5, 1, &[], &[], debug),
        SearchEngine::Yuanbao => yuanbao_search(&entry.key, msg, debug),
    }
}

/// Send one Bocha request with `body` to `endpoint` and return the parsed
/// `data` value. The envelope is `{code, log_id, msg, data}`; a non-200 `code`
/// is an error even on HTTP 200. `debug` prints the actual HTTP request and
/// response.
fn bocha_request(endpoint: &str, key: &str, body: &Value, debug: bool) -> Result<Value> {
    let body_bytes = serde_json::to_vec(body).context("cannot serialize bocha request body")?;
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {key}")).context("invalid bocha api_key")?);
    if debug {
        debug_print_request(endpoint, &headers, &body_bytes);
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")?;
    let resp = client
        .post(endpoint)
        .headers(headers)
        .body(body_bytes.clone())
        .send()
        .with_context(|| format!("request to {endpoint} failed"))?;
    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let text = resp.text().context("cannot read bocha search response")?;
    if debug {
        debug_print_response(status, &resp_headers, &text);
    }
    if !status.is_success() {
        let snippet: String = text.chars().take(500).collect();
        bail!("{endpoint} returned HTTP {status}: {snippet}");
    }
    let v: Value = serde_json::from_str(&text).context("bocha search returned invalid JSON")?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(200);
    if code != 200 {
        let msg = v.get("msg").and_then(|m| m.as_str()).unwrap_or("unknown error");
        bail!("bocha search failed (code {code}): {msg}");
    }
    Ok(v.get("data").cloned().unwrap_or(v))
}

/// Send one Bocha web-search request with the official request parameters
/// (`query` / `freshness` / `summary` / `count` / `page` / `include_domains` /
/// `exclude_domains`) and return the parsed JSON response.
#[allow(clippy::too_many_arguments)]
fn bocha_search_value(
    key: &str,
    query: &str,
    freshness: Option<&str>,
    summary: bool,
    count: u32,
    page: u32,
    include_domains: &[String],
    exclude_domains: &[String],
    debug: bool,
) -> Result<Value> {
    let mut body = json!({ "query": query });
    if summary {
        body["summary"] = json!(true);
    }
    if let Some(f) = freshness.filter(|s| !s.trim().is_empty()) {
        body["freshness"] = json!(f.trim());
    }
    if count > 0 {
        body["count"] = json!(count.clamp(1, 50));
    }
    if page > 1 {
        body["page"] = json!(page);
    }
    if !include_domains.is_empty() {
        body["include_domains"] = json!(include_domains);
    }
    if !exclude_domains.is_empty() {
        body["exclude_domains"] = json!(exclude_domains);
    }
    bocha_request(BOCHA_API, key, &body, debug)
}

/// Collect result-item nodes from every shape the Bocha API returns:
/// `data.webPages.value[]` (current), `data.web_results[]` (legacy) and
/// `data.messages[]` entries with `type == "source"` whose `content.value`
/// holds the webpage list (AI-search shape).
fn bocha_result_items(data: &Value) -> Vec<&Value> {
    let mut items: Vec<&Value> = Vec::new();
    if let Some(arr) = data.get("messages").and_then(|m| m.as_array()) {
        for m in arr {
            let is_source = m
                .get("type")
                .and_then(|t| t.as_str())
                .map(|t| t == "source")
                .unwrap_or(false);
            if is_source {
                if let Some(v) = m.get("content").and_then(|c| c.get("value")).and_then(|v| v.as_array()) {
                    items.extend(v.iter());
                }
            }
        }
    }
    if let Some(arr) = data.get("webPages").and_then(|w| w.get("value")).and_then(|v| v.as_array()) {
        items.extend(arr.iter());
    }
    if let Some(arr) = data.get("web_results").and_then(|v| v.as_array()) {
        items.extend(arr.iter());
    }
    items
}

/// Format a Bocha search result into a plain text block: a numbered list with
/// URL / snippet / site / publish time (`include_summary` adds the detail
/// lines).
fn bocha_result_text(data: &Value, include_summary: bool) -> String {
    let items = bocha_result_items(data);
    if items.is_empty() {
        return "（未返回搜索结果）".to_string();
    }
    let mut out = String::new();
    let mut seen = std::collections::HashSet::new();
    let mut n = 0usize;
    for it in items.iter() {
        let title = it
            .get("name")
            .or_else(|| it.get("webpage"))
            .or_else(|| it.get("title"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        let url = it.get("url").and_then(|x| x.as_str()).unwrap_or("").trim();
        // Skip rows without a title/url and duplicate URLs (keeps the first).
        if title.is_empty() || url.is_empty() || !seen.insert(url) {
            continue;
        }
        n += 1;
        out.push_str(&format!("[{:>2}] {title}\n    {url}\n", n));
        if include_summary {
            let snippet = it
                .get("summary")
                .or_else(|| it.get("snippet"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim();
            let site = it
                .get("siteName")
                .or_else(|| it.get("site_name"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim();
            let date = it
                .get("datePublished")
                .or_else(|| it.get("dateLastCrawled"))
                .or_else(|| it.get("page_timestamp"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim();
            if !snippet.is_empty() {
                out.push_str(&format!("    {snippet}\n"));
            }
            if !site.is_empty() || !date.is_empty() {
                out.push_str(&format!("    {} · {}\n", site, date));
            }
        }
    }
    out.trim_end().to_string()
}

/// Run one Bocha web search and return the formatted result text (title /
/// URL / snippet / site / publish time rows).
#[allow(clippy::too_many_arguments)]
fn bocha_search(
    key: &str,
    query: &str,
    freshness: Option<&str>,
    summary: bool,
    count: u32,
    page: u32,
    include_domains: &[String],
    exclude_domains: &[String],
    debug: bool,
) -> Result<String> {
    let data = bocha_search_value(key, query, freshness, summary, count, page, include_domains, exclude_domains, debug)?;
    Ok(bocha_result_text(&data, summary))
}

// --- Tencent Yuanbao Search (WSA, 服务 API KEY 方式) -----------------------

/// Extract the WSA error (`Response.Error`) from a parsed response, if any.
fn yuanbao_response_error(v: &Value) -> Option<String> {
    let resp = v.get("Response").unwrap_or(v);
    let err = resp.get("Error")?;
    let code = err.get("Code").and_then(|c| c.as_str()).unwrap_or("unknown");
    let msg = err.get("Message").and_then(|m| m.as_str()).unwrap_or("unknown error");
    Some(format!("yuanbao search failed ({code}): {msg}"))
}

/// Send one Yuanbao `SearchPro` request with `body` and return the parsed
/// `Response` object. Auth is the WSA service API KEY (`Authorization:
/// Bearer <key>`); the envelope is `{"Response": {..., "Error": {...}}}` and
/// errors are reported via `Response.Error.Code/Message`. `debug` prints the
/// actual HTTP request and response.
fn yuanbao_request(key: &str, body: &Value, debug: bool) -> Result<Value> {
    let body_bytes = serde_json::to_vec(body).context("cannot serialize yuanbao request body")?;
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {key}")).context("invalid yuanbao api key")?);
    if debug {
        debug_print_request(YUANBAO_API, &headers, &body_bytes);
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")?;
    let resp = client
        .post(YUANBAO_API)
        .headers(headers)
        .body(body_bytes.clone())
        .send()
        .with_context(|| format!("request to {YUANBAO_API} failed"))?;
    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let text = resp.text().context("cannot read yuanbao search response")?;
    if debug {
        debug_print_response(status, &resp_headers, &text);
    }
    if !status.is_success() {
        let snippet: String = text.chars().take(500).collect();
        bail!("{YUANBAO_API} returned HTTP {status}: {snippet}");
    }
    let v: Value = serde_json::from_str(&text).context("yuanbao search returned invalid JSON")?;
    if let Some(err) = yuanbao_response_error(&v) {
        bail!("{err}");
    }
    Ok(v.get("Response").cloned().unwrap_or(v))
}

/// Send one Yuanbao web-search (`SearchPro`) request with `Query` and `Cnt`
/// (the API only accepts 10/20/30/40/50; we request 10) and return the parsed
/// `Response`.
fn yuanbao_search_value(key: &str, query: &str, debug: bool) -> Result<Value> {
    let body = json!({ "Query": query, "Cnt": 10 });
    yuanbao_request(key, &body, debug)
}

/// Parse the `Response.Pages` array of JSON-string page objects into their
/// values, skipping entries without a usable title/url (or that fail to
/// parse). Each `Pages` element is itself a JSON string.
fn yuanbao_result_items(data: &Value) -> Vec<Value> {
    let mut items = Vec::new();
    // Accept both the raw API envelope (`Response.Pages`) and the already
    // unwrapped `Response` object returned by `yuanbao_request` (`Pages`).
    let pages = data
        .get("Response")
        .and_then(|r| r.get("Pages"))
        .or_else(|| data.get("Pages"))
        .and_then(|p| p.as_array());
    let Some(pages) = pages else {
        return items;
    };
    for page in pages {
        let Ok(v) = serde_json::from_str::<Value>(page.as_str().unwrap_or("")) else {
            continue;
        };
        let title = v.get("title").and_then(|t| t.as_str()).unwrap_or("").trim();
        let url = v.get("url").and_then(|u| u.as_str()).unwrap_or("").trim();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        items.push(v);
    }
    items
}

/// Format a Yuanbao search response into the same numbered plain-text block
/// used by Bocha (title / URL / snippet / site · date), keeping the best 5
/// rows (the API returns them relevance-ranked). `passage` is the standard
/// summary; it may be a long paragraph, so the snippet is capped at 500 chars.
fn yuanbao_result_text(data: &Value) -> String {
    let items = yuanbao_result_items(data);
    if items.is_empty() {
        return "（未返回搜索结果）".to_string();
    }
    let mut out = String::new();
    for (i, it) in items.iter().take(5).enumerate() {
        let title = it.get("title").and_then(|t| t.as_str()).unwrap_or("").trim();
        let url = it.get("url").and_then(|u| u.as_str()).unwrap_or("").trim();
        let snippet: String = it
            .get("passage")
            .or_else(|| it.get("content"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .chars()
            .take(500)
            .collect();
        let site = it.get("site").and_then(|x| x.as_str()).unwrap_or("").trim();
        let date = it.get("date").and_then(|x| x.as_str()).unwrap_or("").trim();
        out.push_str(&format!("[{:>2}] {title}\n    {url}\n", i + 1));
        if !snippet.is_empty() {
            out.push_str(&format!("    {snippet}\n"));
        }
        if !site.is_empty() || !date.is_empty() {
            out.push_str(&format!("    {} · {}\n", site, date));
        }
    }
    out.trim_end().to_string()
}

/// Run one Yuanbao web search and return the formatted result text.
fn yuanbao_search(key: &str, query: &str, debug: bool) -> Result<String> {
    let data = yuanbao_search_value(key, query, debug)?;
    Ok(yuanbao_result_text(&data))
}

/// Build the augmented user message for `--web`: the web search results as
/// grounding context plus the original question.
fn web_search_message(results: &str, question: &str) -> String {
    format!(
        "请基于以下联网搜索结果回答用户的问题。优先引用搜索结果中的信息，并在回答中附上来源链接；如果搜索结果不足以回答，请明确说明。\n\n=== 联网搜索结果 ===\n{results}\n\n=== 用户问题 ===\n{question}"
    )
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

/// Truncate `msg` to `limit` characters when a limit is set; otherwise the
/// message passes through unchanged. A notice is printed when truncation
/// actually happens.
fn truncate_to_limit(msg: &str, limit: Option<u64>) -> String {
    match limit {
        Some(l) if (msg.chars().count() as u64) > l => {
            let truncated: String = msg.chars().take(l as usize).collect();
            eprintln!(
                "sys: message of {} chars truncated to {} (model input limit)",
                msg.chars().count(),
                l
            );
            truncated
        }
        _ => msg.to_string(),
    }
}

/// Which capability fields are missing for a model: `(max_input_tokens, type)`.
/// `--update` fills exactly the missing ones.
fn missing_capability_fields(m: &Model) -> (bool, bool) {
    (m.max_input_tokens.is_none(), m.model_type.is_none())
}

/// `--update`: fill the target model's missing `max_input_tokens` / `type`
/// from models.dev (24 h cache) and persist them into the config file. Only
/// the missing fields are written, and only when the lookup finds a value.
/// Best effort: lookup or persistence failures only warn and never block the
/// request. Returns the fields that were actually filled (for reporting).
fn update_model_capabilities(cfg: &mut Config, path: &Path, pi: usize, mi: usize) -> (Option<u64>, Option<String>) {
    let m = &cfg.providers[pi].models[mi];
    let (need_ctx, need_type) = missing_capability_fields(m);
    if !need_ctx && !need_type {
        return (None, None);
    }
    let (ctx, modl) = crate::ai::lookup_model_capabilities(&m.name);
    let pname = cfg.providers[pi].name.clone();
    let mname = cfg.providers[pi].models[mi].name.clone();
    let model = &mut cfg.providers[pi].models[mi];
    let mut filled_ctx = None;
    let mut filled_type = None;
    if need_ctx {
        if let Some(c) = ctx {
            model.max_input_tokens = Some(c);
            if let Err(e) = update_model_field_in_config(path, &pname, &mname, "max_input_tokens", &c.to_string(), 0) {
                eprintln!("sys: warning: cannot persist max_input_tokens for {pname} / {mname}: {e:#}");
            } else {
                filled_ctx = Some(c);
            }
        }
    }
    if need_type {
        if let Some(t) = &modl {
            model.model_type = Some(t.clone());
            if let Err(e) = update_model_field_in_config(path, &pname, &mname, "type", t, 0) {
                eprintln!("sys: warning: cannot persist type for {pname} / {mname}: {e:#}");
            } else {
                filled_type = Some(t.clone());
            }
        }
    }
    (filled_ctx, filled_type)
}

/// One chat round-trip through the weighted rotation. Before each HTTP request
/// the message is truncated to the model's configured input limit
/// (`max_input_tokens` from the config; absent = no truncation). With
/// `update` set, the target model's missing `max_input_tokens` / `type` are
/// first filled from models.dev and persisted (see `update_model_capabilities`).
/// Returns the model's reply text (which has already been streamed to stdout
/// when streaming is active).
fn chat_once(
    cfg: &mut Config,
    path: &Path,
    msg: &str,
    debug: bool,
    no_stream: bool,
    model_override: Option<&str>,
    update: bool,
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
        if update {
            let (ctx, typ) = update_model_capabilities(cfg, path, t.provider, t.model);
            if ctx.is_some() || typ.is_some() {
                let p = &cfg.providers[t.provider];
                let m = &p.models[t.model];
                eprintln!(
                    "sys: update: filled {} for {} / {} (from models.dev)",
                    [ctx.map(|c| format!("max_input_tokens={c}")), typ.map(|t| format!("type={t}"))]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(", "),
                    p.name,
                    m.name
                );
            }
        }
        let p = &cfg.providers[t.provider];
        let m = &p.models[t.model];
        eprintln!("sys: using {} / {} ({})", p.name, m.name, p.kind);
        let payload = truncate_to_limit(msg, m.max_input_tokens);
        match send_chat(p, m, &payload, stream, debug) {
            Ok(text) => {
                // A model that succeeded after a previous failure gets its
                // weight bumped (max 9), persisted to the config file.
                if had_failure && m.weight < 9 && !cfg.free {
                    if let Err(e) = update_weight_in_config(path, &p.name, &m.name, m.weight + 1) {
                        eprintln!("sys: warning: failed to persist weight bump for {} / {}: {e:#}", p.name, m.name);
                    }
                }
                return Ok(text);
            }
            Err(e) => {
                errors.push(format!("{} / {}: {e:#}", p.name, m.name));
                // Failure decrements the weight (min 1), persisted to config,
                // then the next model is tried automatically.
                if m.weight > 1 && !cfg.free {
                    if let Err(e2) = update_weight_in_config(path, &p.name, &m.name, m.weight - 1) {
                        eprintln!("sys: warning: failed to persist weight drop for {} / {}: {e2:#}", p.name, m.name);
                    }
                }
                had_failure = true;
            }
        }
    }
    bail!("all {} model(s) failed: {}", targets.len(), errors.join(" | "));
}

/// `sys ai chat` core: with `--web[=PROVIDER]`, first runs the selected web
/// search (Bocha / Tencent Yuanbao) on the message and augments it with the
/// results as grounding context, then runs `chat_once` and prints the reply
/// when the request was non-streaming (streaming already printed to stdout).
fn chat_with(
    cfg: &mut Config,
    path: &Path,
    msg: &str,
    web: Option<&str>,
    debug: bool,
    no_stream: bool,
    model_override: Option<&str>,
    update: bool,
) -> Result<()> {
    let payload = if web.is_some() {
        let results = web_search(cfg, web, path, msg, debug)?;
        eprintln!(
            "sys: web search done ({} chars of results fed to the model)",
            results.chars().count()
        );
        web_search_message(&results, msg)
    } else {
        msg.to_string()
    };
    let text = chat_once(cfg, path, &payload, debug, no_stream, model_override, update)?;
    let stream = cfg.stream && !debug && !no_stream;
    if !stream {
        println!("{text}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `sys ai chat --test`: parallel chat feedback-time probe
// ---------------------------------------------------------------------------

/// Resolve the models to test: with a `-m` override the usual selector rules
/// apply (see `resolve_targets`); without one, every configured model is
/// tested.
fn test_targets(cfg: &Config, model_override: Option<&str>) -> Result<Vec<Target>> {
    if let Some(sel) = model_override {
        return resolve_targets(cfg, sel);
    }
    let mut out = Vec::new();
    for (pi, p) in cfg.providers.iter().enumerate() {
        for mi in 0..p.models.len() {
            out.push(Target { provider: pi, model: mi });
        }
    }
    if out.is_empty() {
        bail!("no models configured under `clients` to test");
    }
    Ok(out)
}

/// Read a streaming response just until the first non-empty content delta
/// arrives, then stop (the connection is dropped, so a full reply is never
/// awaited). A body that turns out not to be SSE is judged as a whole.
/// Returns the elapsed time since `start`.
fn first_token_elapsed(resp: reqwest::blocking::Response, kind: &str, start: Instant) -> Result<Duration> {
    let mut reader = std::io::BufReader::new(resp);
    let mut line = String::new();
    let mut first = true;
    loop {
        line.clear();
        let n = reader.read_line(&mut line).context("failed reading response stream")?;
        if n == 0 {
            bail!("stream ended before any content arrived");
        }
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if first {
            first = false;
            if !t.starts_with("data:") && !t.starts_with("event:") {
                // Not an SSE stream: judge the whole body as one response.
                let mut rest = String::new();
                let _ = reader.read_to_string(&mut rest);
                let text = extract_content(&format!("{t}\n{rest}"), kind)?;
                if text.trim().is_empty() {
                    bail!("response contained no text content");
                }
                return Ok(start.elapsed());
            }
        }
        if let Some(d) = t.strip_prefix("data:") {
            let d = d.trim();
            if d == "[DONE]" {
                bail!("stream ended before any content arrived");
            }
            let v: Value = serde_json::from_str(d)
                .with_context(|| format!("invalid stream chunk: {}", d.chars().take(200).collect::<String>()))?;
            if let Some(err) = v.get("error") {
                let raw = err.to_string();
                let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or(&raw);
                bail!("provider error: {msg}");
            }
            let text: String = if kind == "anthropic" {
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
            if !text.trim().is_empty() {
                return Ok(start.elapsed());
            }
        }
    }
}

/// Test one model: POST a streaming chat request and measure the time until
/// the first token arrives (the connection is dropped right afterwards, so a
/// full reply is never awaited). Any failure — HTTP error such as 429,
/// connection error, timeout, empty response — is returned as Err.
fn test_one(p: &Provider, m: &Model, msg: &str) -> Result<Duration> {
    let url = chat_url(p);
    let body = chat_body(p, m, msg, true);
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
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));

    let client = Client::builder()
        .connect_timeout(TEST_CONNECT_TIMEOUT)
        .timeout(TEST_TOTAL_TIMEOUT)
        .build()
        .context("failed to build HTTP client")?;
    let start = Instant::now();
    let resp = client
        .post(&url)
        .headers(headers)
        .body(body_bytes)
        .send()
        .with_context(|| format!("request to {url} failed"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        let snippet: String = text.chars().take(300).collect::<String>().trim().to_string();
        bail!("{url} returned HTTP {status}: {snippet}");
    }
    first_token_elapsed(resp, &p.kind, start)
}

/// `sys ai chat --test [MSG...] [-m MODEL] [--parallel N] [--remove]`
///
/// Runs a streaming first-token chat test over the selected models (default:
/// every configured model) and prints each model's response time the moment it
/// completes. Models run in parallel: by default one worker per provider among
/// the targets (capped at the CPU core count); `--parallel` overrides. Models
/// that fail — no feedback, HTTP error such as 429, connection error, timeout
/// — get their `weight` set to 0 in the config file; with `--remove` they are
/// deleted from the config instead (the original config is backed up first;
/// providers left without any models are removed too).
fn chat_test(cfg: &Config, path: &Path, msg: &str, model_override: Option<&str>, parallel: Option<usize>, remove: bool) -> Result<()> {
    let targets = test_targets(cfg, model_override)?;
    let n = targets.len();
    let provider_count = {
        let mut seen = std::collections::HashSet::new();
        for t in &targets {
            seen.insert(t.provider);
        }
        seen.len()
    };
    let cpu = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1);
    let workers = test_worker_count(provider_count, n, cpu, parallel);
    let w = targets
        .iter()
        .map(|t| format!("{} / {}", cfg.providers[t.provider].name, cfg.providers[t.provider].models[t.model].name).chars().count())
        .max()
        .unwrap_or(8)
        .max(8);

    println!("sys: testing {n} model(s) across {provider_count} provider(s) in parallel ({workers} worker(s)) with `{msg}`{}", if remove { " (--remove: failures deleted from config)" } else { "" });

    // Worker pool: each worker pulls the next target index and sends its result
    // as soon as that single test finishes. The main thread prints each result
    // immediately (completion order). Weight updates / removals are applied
    // serially below so the config file is never written concurrently.
    let targets = &targets;
    let next = std::sync::Arc::new(std::sync::Mutex::new(0usize));
    let (tx, rx) = mpsc::channel::<(usize, Result<Duration>)>();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let next = next.clone();
            scope.spawn(move || loop {
                let idx = {
                    let mut guard = next.lock().unwrap();
                    let i = *guard;
                    *guard += 1;
                    i
                };
                if idx >= n {
                    break;
                }
                let t = targets[idx];
                let p = &cfg.providers[t.provider];
                let m = &p.models[t.model];
                let _ = tx.send((idx, test_one(p, m, msg)));
            });
        }
    });
    drop(tx);
    let mut ok = 0usize;
    let mut failed = 0usize;
    let mut removals: Vec<(usize, usize)> = Vec::new();
    for (idx, r) in rx.iter() {
        let t = targets[idx];
        let p = &cfg.providers[t.provider];
        let m = &p.models[t.model];
        let name = format!("{} / {}", p.name, m.name);
        match r {
            Ok(elapsed) => {
                ok += 1;
                println!("[OK]   {:<w$}  first token in {:.2}s", name, elapsed.as_secs_f64());
            }
            Err(e) => {
                failed += 1;
                if remove {
                    println!("[FAIL] {:<w$}  {e:#}  (removed)", name);
                    removals.push((t.provider, t.model));
                } else {
                    println!("[FAIL] {:<w$}  {e:#}", name);
                    if m.weight != 0 {
                        // Duplicate model names under one provider are separate
                        // targets; zero the weight of this exact entry.
                        let occ = targets[..idx]
                            .iter()
                            .filter(|x| {
                                x.provider == t.provider
                                    && ci_eq(&cfg.providers[x.provider].models[x.model].name, &m.name)
                            })
                            .count();
                        if let Err(e2) = update_weight_in_config_at(path, &p.name, &m.name, occ, 0) {
                            eprintln!(
                                "sys: warning: failed to persist weight->0 for {} / {}: {e2:#}",
                                p.name, m.name
                            );
                        }
                    }
                }
            }
        }
    }
    if remove {
        if let Err(e2) = apply_test_removals(cfg, path, &removals) {
            eprintln!("sys: warning: failed to remove failed models from `{}`: {e2:#}", path.display());
        }
        println!("sys: {ok}/{n} model(s) responded; {failed} failed (removed from config)");
    } else {
        println!("sys: {ok}/{n} model(s) responded; {failed} failed (weight -> 0)");
    }
    Ok(())
}

/// Parallel worker count for `--test`: an explicit `--parallel` wins (capped at
/// the number of targets); otherwise one worker per distinct provider among the
/// targets, capped at the CPU core count.
fn test_worker_count(provider_count: usize, target_count: usize, cpu: usize, user: Option<usize>) -> usize {
    match user {
        Some(p) => p.max(1).min(target_count.max(1)),
        None => provider_count.max(1).min(cpu.max(1)),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Print every provider (name / kind / model count), one line each.
pub(crate) fn print_providers(cfg: &Config) {
    println!("{:<16}  {:<10}  {}", "PROVIDER", "KIND", "MODELS");
    for p in &cfg.providers {
        println!("{:<16}  {:<10}  {}", p.name, p.kind, p.models.len());
    }
}

/// Print every model grouped by provider (with weights when not 1).
pub(crate) fn print_models(cfg: &Config) {
    for p in &cfg.providers {
        println!("{} ({}):", p.name, p.kind);
        for m in &p.models {
            let w = if m.weight == 1 {
                String::new()
            } else {
                format!(" (weight {})", m.weight)
            };
            let g = if m.group.is_empty() {
                String::new()
            } else {
                format!(" [group: {}]", m.group.join(", "))
            };
            println!("  - {}{}{}", m.name, w, g);
        }
    }
}

/// `sys ai chat [MSG...] [-m MODEL] [--list-model] [--list-provider] [-c FILE] [--debug] [--no-stream] [--update]`
/// `sys ai chat --test [MSG...] [-m MODEL] [--parallel N] [--remove]`
/// `sys ai chat --query-model PROVIDER`
///
/// `-m/--model` overrides the top-level `model` from the config (same
/// `{provider}:{model}` / `{model}` / comma-separated rules). `--list-model`
/// prints every configured model (grouped by provider); `--list-provider`
/// prints every configured provider. `--update` fills the target models'
/// missing `max_input_tokens` / `type` from models.dev into the config file
/// (best effort) before the request. `--query-model PROVIDER` queries the
/// provider's model list through its models endpoint
/// (`{api_base}/models` for OpenAI-compatible, `{api_base}/v1/models` for
/// Anthropic) and exits.
/// `--test` runs a parallel chat feedback-time test instead of a normal chat
/// (see `chat_test`); `--remove` deletes the failed models from the config
/// instead of setting weight to 0.
pub fn cmd_chat(
    words: &[String],
    config: Option<&Path>,
    debug: bool,
    no_stream: bool,
    model: Option<&str>,
    list_model: bool,
    list_provider: bool,
    update: bool,
    test: bool,
    parallel: Option<usize>,
    remove: bool,
    query_model: Option<&str>,
    free: Option<&str>,
    web: Option<&str>,
) -> Result<()> {
    // Free mode on either trigger: `--free[=router]`, or the default
    // config.yaml being absent. An explicit `-c` always loads and validates
    // the named file.
    let (mut cfg, path) = if use_free_mode(free.is_some(), config.is_some(), default_config_exists())
    {
        let p = home_dir()
            .map(|h| h.join(".sysenv").join("config.yaml"))
            .unwrap_or_else(|| PathBuf::from("config.yaml"));
        (free_config(free)?, p)
    } else {
        load_config(config)?
    };

    if list_model || list_provider {
        if list_provider {
            print_providers(&cfg);
        }
        if list_model {
            if list_provider {
                println!();
            }
            print_models(&cfg);
        }
        return Ok(());
    }

    if cfg.free && (query_model.is_some() || test || update) {
        bail!("free mode does not support --query-model, --test or --update");
    }

    if web.is_some() && test {
        bail!("--web is not supported with --test");
    }

    if let Some(pname) = query_model {
        return query_provider_models(&cfg, pname);
    }

    if test {
        let msg = if !words.is_empty() {
            words.join(" ")
        } else {
            "hello".to_string()
        };
        return chat_test(&cfg, &path, &msg, model, parallel, remove);
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
        bail!("provide a message: `sys ai chat \"your message\"` (or pipe text via stdin)");
    };
    chat_with(&mut cfg, &path, &msg, web, debug, no_stream, model, update)
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
    fn duplicate_api_base_still_loads_but_warns() {
        // the same api_base twice: loading must not break (a warning goes to
        // stderr), both blocks stay — a provider's identity is its endpoint,
        // so same-endpoint blocks must live together
        let yaml = r#"clients:
  - type: openai
    name: agnes
    api_base: https://a.example/v1
    api_key: sk-a
    models:
      - name: m1
  - type: openai
    name: agnes
    api_base: https://a.example/v1
    api_key: sk-b
    models:
      - name: m2
"#;
        let y = parse_yaml(yaml).unwrap();
        let cfg = config_from_yaml(&y).unwrap();
        assert_eq!(cfg.providers.len(), 2);
        assert_eq!(cfg.providers[0].name, "agnes");
        assert_eq!(cfg.providers[1].name, "agnes");
        // same name at a DIFFERENT endpoint is a distinct provider, not a
        // duplicate: loading stays clean
        let yaml2 = r#"clients:
  - type: openai
    name: agnes
    api_base: https://a.example/v1
    api_key: sk-a
    models:
      - name: m1
  - type: openai
    name: agnes
    api_base: https://b.example/v1
    api_key: sk-b
    models:
      - name: m2
"#;
        let cfg2 = config_from_yaml(&parse_yaml(yaml2).unwrap()).unwrap();
        assert_eq!(cfg2.providers.len(), 2);
        assert_eq!(cfg2.providers[0].api_base, "https://a.example/v1");
        assert_eq!(cfg2.providers[1].api_base, "https://b.example/v1");
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
    fn resolve_targets_group_form() {
        let y = parse_yaml(
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
        group: fast, cheap
      - name: m2
        group: fast
  - type: anthropic
    name: claude
    api_base: https://api.anthropic.example.com
    api_key: sk-ant-test
    models:
      - name: c1
        group: cheap
      - name: c2
"#,
        )
        .unwrap();
        let cfg = config_from_yaml(&y).unwrap();

        // `group:<name>` picks every model whose groups contain the name,
        // across providers, in config order.
        let t = resolve_targets(&cfg, "group:fast").unwrap();
        assert_eq!(t, vec![Target { provider: 0, model: 0 }, Target { provider: 0, model: 1 }]);
        let t = resolve_targets(&cfg, "group:cheap").unwrap();
        assert_eq!(t, vec![Target { provider: 0, model: 0 }, Target { provider: 1, model: 0 }]);

        // Case-insensitive on both the `group:` prefix and the name.
        let t = resolve_targets(&cfg, "GROUP:FAST").unwrap();
        assert_eq!(t.len(), 2);

        // Comma lists mix group and provider selectors, de-duplicated.
        let t = resolve_targets(&cfg, "group:fast,claude:c1").unwrap();
        assert_eq!(t.len(), 3);

        // Unknown group -> error listing available groups (deduplicated).
        let err = resolve_targets(&cfg, "group:nope").unwrap_err();
        assert!(err.to_string().contains("available groups: fast, cheap"), "{err}");

        // Empty group name -> error.
        assert!(resolve_targets(&cfg, "group:").is_err());

        // No groups configured at all -> explicit hint.
        let y = parse_yaml(
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
"#,
        )
        .unwrap();
        let cfg = config_from_yaml(&y).unwrap();
        let err = resolve_targets(&cfg, "group:fast").unwrap_err();
        assert!(err.to_string().contains("no groups configured"), "{err}");
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
    fn free_providers_without_openrouter_key_keeps_anonymous_routers() {
        let provs = free_providers(None);
        assert_eq!(provs.len(), 2);
        assert_eq!(provs[0].name, "kilo-auto");
        assert_eq!(provs[0].models[0].name, "kilo-auto/free");
        assert!(provs[0].api_key.is_empty());
        assert_eq!(
            chat_url(&provs[0]),
            "https://ai-gateway.kilo.ai/api/v1/chat/completions"
        );
        // blockrun anonymous free endpoint (the real model ID is sent as-is).
        assert_eq!(provs[1].name, "blockrun");
        assert_eq!(provs[1].models[0].name, "nvidia/llama-3.2-11b-vision");
        assert!(provs[1].api_key.is_empty());
        assert_eq!(
            chat_url(&provs[1]),
            "https://blockrun.ai/api/v1/chat/completions"
        );
    }

    #[test]
    fn free_providers_with_openrouter_key_adds_router() {
        let provs = free_providers(Some("sk-or-test".to_string()));
        assert_eq!(provs.len(), 3);
        let or = &provs[2];
        assert_eq!(or.name, "openrouter");
        assert_eq!(or.models[0].name, "openrouter/free");
        assert_eq!(or.api_key, "sk-or-test");
        assert_eq!(chat_url(or), "https://openrouter.ai/api/v1/chat/completions");
        // A blank / whitespace key drops openrouter and keeps the two anonymous routers.
        assert_eq!(free_providers(Some("  ".to_string())).len(), 2);
    }

    #[test]
    fn free_mode_triggers_on_flag_or_missing_default() {
        // --free forces free regardless of config presence.
        assert!(use_free_mode(true, false, true));
        assert!(use_free_mode(true, true, false));
        // No flag: free when the default config is absent (and no -c named).
        assert!(use_free_mode(false, false, false));
        // Default present, no flag -> normal config load.
        assert!(!use_free_mode(false, false, true));
        // An explicit -c file is honored even when the default is missing.
        assert!(!use_free_mode(false, true, false));
    }

    #[test]
    fn parse_free_router_accepts_aliases_and_rejects_unknown() {
        assert_eq!(parse_free_router("kilo").unwrap(), "kilo-auto");
        assert_eq!(parse_free_router("KILO-AUTO").unwrap(), "kilo-auto");
        assert_eq!(parse_free_router("kilo-auto/free").unwrap(), "kilo-auto");
        assert_eq!(parse_free_router("blockrun").unwrap(), "blockrun");
        assert_eq!(parse_free_router("BLOCKRUN/free").unwrap(), "blockrun");
        assert_eq!(parse_free_router("openrouter").unwrap(), "openrouter");
        assert_eq!(parse_free_router("openrouter/free").unwrap(), "openrouter");
        assert!(parse_free_router("gemini").is_err());
    }

    #[test]
    fn free_selector_pins_one_router_or_bails_on_missing() {
        let both = free_providers(Some("sk-or-test".to_string()));
        // None / "all" keeps every available router untouched.
        let all = filter_free_selector(both.clone(), None).unwrap();
        assert_eq!(all.providers.len(), 3);
        let all = filter_free_selector(both.clone(), Some("all")).unwrap();
        assert_eq!(all.providers.len(), 3);
        // Pinning kilo keeps only the anonymous router and sets its model.
        let k = filter_free_selector(both.clone(), Some("kilo")).unwrap();
        assert_eq!(k.providers.len(), 1);
        assert_eq!(k.providers[0].name, "kilo-auto");
        assert_eq!(k.model.as_deref(), Some("kilo-auto/free"));
        // Pinning blockrun keeps only that router and its real free model.
        let b = filter_free_selector(both.clone(), Some("blockrun")).unwrap();
        assert_eq!(b.providers.len(), 1);
        assert_eq!(b.providers[0].name, "blockrun");
        assert_eq!(b.model.as_deref(), Some("nvidia/llama-3.2-11b-vision"));
        // Pinning openrouter keeps only that router and its model.
        let o = filter_free_selector(both.clone(), Some("openrouter/free")).unwrap();
        assert_eq!(o.providers.len(), 1);
        assert_eq!(o.providers[0].name, "openrouter");
        assert_eq!(o.model.as_deref(), Some("openrouter/free"));
        // Pinning openrouter when it was never built (no key) bails with guidance.
        let err = filter_free_selector(free_providers(None), Some("openrouter")).unwrap_err();
        assert!(err.to_string().contains("OPENROUTER_API_KEY"), "got: {err}");
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

    // ------------------------------------------------------------------
    // weight persistence (update_weight_in_config)
    // ------------------------------------------------------------------

    fn tmp_config(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("sys-chat-test");
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
        update_model_field_in_config(&p, "agnes", "m1", "max_input_tokens", "1048576", 0).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("      - name: m1\n        max_input_tokens: 1048576"));
        // Replace an existing key.
        update_model_field_in_config(&p, "agnes", "m1", "weight", "5", 0).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("        weight: 5"));
        // Insert the type field (generic key editing; not read back into Model).
        update_model_field_in_config(&p, "agnes", "m1", "type", "text,image", 0).unwrap();
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
        assert!(update_model_field_in_config(&p, "agnes", "nope", "type", "text", 0).is_err());
    }

    #[test]
    fn update_weight_in_config_at_targets_duplicate_entries() {
        let p = tmp_config(
            "dup-weight",
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
        weight: 1
      - name: m1
"#,
        );
        // Occurrence 1 = the second duplicate entry (no weight line yet);
        // inserting weight: 0 there must not touch the first entry.
        update_weight_in_config_at(&p, "agnes", "m1", 1, 0).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("      - name: m1\n        weight: 1"), "first entry changed: {t}");
        assert!(t.contains("      - name: m1\n        weight: 0"), "second entry not updated: {t}");
        // Occurrence 0 still addresses the first entry alone.
        update_weight_in_config_at(&p, "agnes", "m1", 0, 2).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("      - name: m1\n        weight: 2"), "first entry not updated: {t}");
        assert!(t.contains("      - name: m1\n        weight: 0"), "second entry changed: {t}");
    }

    #[test]
    fn remove_model_entry_deletes_subkeys_and_keeps_siblings() {
        let p = tmp_config(
            "rm-model",
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
        weight: 3
        type: text,image
      - name: m2
        weight: 1
"#,
        );
        remove_model_entry_from_config(&p, "agnes", "m1", 0).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(!t.contains("m1"), "m1 entry not removed: {t}");
        assert!(t.contains("      - name: m2\n        weight: 1"), "sibling m2 damaged: {t}");
        // The remaining config still parses into exactly one model.
        let cfg = config_from_yaml(&parse_yaml(&t).unwrap()).unwrap();
        assert_eq!(cfg.providers[0].models.len(), 1);
        assert_eq!(cfg.providers[0].models[0].name, "m2");
    }

    #[test]
    fn remove_model_entry_handles_mangled_weight_line() {
        let p = tmp_config(
            "rm-mangled",
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
      - weight: 1
      - name: m2
"#,
        );
        remove_model_entry_from_config(&p, "agnes", "m1", 0).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(!t.contains("m1") && !t.contains("- weight:"), "entry not fully removed: {t}");
        assert!(t.contains("      - name: m2"), "sibling m2 damaged: {t}");
    }

    #[test]
    fn remove_model_entry_targets_duplicate_by_occurrence() {
        let p = tmp_config(
            "rm-dup",
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
        weight: 1
      - name: m1
        weight: 2
"#,
        );
        // Remove the second duplicate only.
        remove_model_entry_from_config(&p, "agnes", "m1", 1).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("      - name: m1\n        weight: 1"), "first entry removed: {t}");
        assert!(!t.contains("weight: 2"), "second entry not removed: {t}");
        let cfg = config_from_yaml(&parse_yaml(&t).unwrap()).unwrap();
        assert_eq!(cfg.providers[0].models.len(), 1);
        assert_eq!(cfg.providers[0].models[0].weight, 1);
    }

    #[test]
    fn remove_provider_entry_deletes_whole_block() {
        let p = tmp_config(
            "rm-provider",
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
  - type: anthropic
    name: claude
    api_base: https://api.example.com
    api_key: sk-ant
    models:
      - name: c1
"#,
        );
        remove_provider_entry_from_config(&p, "agnes").unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(!t.contains("agnes") && !t.contains("m1"), "provider block not removed: {t}");
        assert!(t.contains("name: claude"), "sibling provider damaged: {t}");
        let cfg = config_from_yaml(&parse_yaml(&t).unwrap()).unwrap();
        assert_eq!(cfg.providers.len(), 1);
        assert_eq!(cfg.providers[0].name, "claude");
    }

    #[test]
    fn clean_top_level_model_drops_dangling_selectors() {
        // m1 and agnes are deleted from the config; the top-level model line
        // keeps only the still-resolvable claude:c1 selector.
        let p = tmp_config(
            "clean-model",
            r#"model: m1,claude:c1
clients:
  - type: anthropic
    name: claude
    api_base: https://api.example.com
    api_key: sk-ant
    models:
      - name: c1
"#,
        );
        clean_top_level_model_selector(&p).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("model: claude:c1"), "dangling m1 kept: {t}");
        // Removing the last resolvable selector deletes the model line.
        let p2 = tmp_config(
            "clean-model2",
            r#"model: claude:c1
clients:
  - type: anthropic
    name: claude
    api_base: https://api.example.com
    api_key: sk-ant
    models:
      - name: c1
"#,
        );
        remove_provider_entry_from_config(&p2, "claude").unwrap();
        clean_top_level_model_selector(&p2).unwrap();
        let t2 = std::fs::read_to_string(&p2).unwrap();
        assert!(!t2.contains("model:"), "empty model line not removed: {t2}");
    }

    #[test]
    fn apply_test_removals_removes_models_providers_and_selector() {
        // agnes has two models; both fail -> provider removed and the top-level
        // selector (which only names agnes models) is dropped. claude survives.
        let p = tmp_config(
            "apply-rm",
            r#"model: agnes-3.0-flash,m1
clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: agnes-3.0-flash
        weight: 1
      - name: m1
        weight: 2
  - type: anthropic
    name: claude
    api_base: https://api.example.com
    api_key: sk-ant
    models:
      - name: c1
"#,
        );
        let cfg = config_from_yaml(&parse_yaml(&std::fs::read_to_string(&p).unwrap()).unwrap()).unwrap();
        // agnes = provider 0, models 0 and 1 -> both removed -> provider gone.
        apply_test_removals(&cfg, &p, &[(0, 0), (0, 1)]).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(!t.contains("agnes") && !t.contains("m1"), "agnes not removed: {t}");
        assert!(t.contains("name: claude"), "claude damaged: {t}");
        assert!(!t.contains("model:"), "dangling top-level model left: {t}");
    }

    #[test]
    fn apply_test_removals_backs_up_config_before_deleting() {
        let p = tmp_config(
            "rm-backup",
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
      - name: m2
"#,
        );
        let before = std::fs::read_to_string(&p).unwrap();
        let cfg = config_from_yaml(&parse_yaml(&before).unwrap()).unwrap();
        // Remove stale backups left by earlier runs in the shared temp dir.
        let stem = std::path::Path::new(&p).file_stem().unwrap().to_string_lossy().to_string();
        let dir = std::path::Path::new(&p).parent().unwrap();
        for e in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with(&stem) && n.contains(".bak-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
        apply_test_removals(&cfg, &p, &[(0, 0)]).unwrap();
        // The original config is preserved verbatim in a timestamped .bak-* copy
        // named after this config file.
        let backups: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|q| q.file_name().map(|n| n.to_string_lossy().starts_with(&stem)).unwrap_or(false))
            .filter(|q| q.to_string_lossy().contains(".bak-"))
            .collect();
        assert_eq!(backups.len(), 1, "expected exactly one backup: {backups:?}");
        assert_eq!(
            std::fs::read_to_string(&backups[0]).unwrap(),
            before,
            "backup content differs from original"
        );
        // And the live file was modified by the removal.
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(!after.contains("m1"), "m1 not removed: {after}");
        assert!(after.contains("m2"), "m2 damaged: {after}");
    }

    #[test]
    fn apply_test_removals_skips_backup_when_nothing_to_remove() {
        let p = tmp_config(
            "rm-nobackup",
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
"#,
        );
        apply_test_removals(&Config { model: None, stream: false, providers: Vec::new(), free: false, search: Vec::new() }, &p, &[]).unwrap();
        let stem = std::path::Path::new(&p).file_stem().unwrap().to_string_lossy().to_string();
        let dir = std::path::Path::new(&p).parent().unwrap();
        let n = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(&stem))
            .count();
        assert_eq!(n, 1, "no backup expected for empty removals");
    }

    #[test]
    fn test_worker_count_defaults_to_provider_count_capped_at_cpu() {
        // Default: one worker per provider, never above the CPU core count.
        assert_eq!(test_worker_count(3, 9, 8, None), 3);
        assert_eq!(test_worker_count(5, 5, 8, None), 5);
        // CPU cap wins when providers exceed cores.
        assert_eq!(test_worker_count(12, 20, 8, None), 8);
        assert_eq!(test_worker_count(4, 4, 2, None), 2);
        // Explicit --parallel overrides (capped at the target count).
        assert_eq!(test_worker_count(3, 9, 8, Some(2)), 2);
        assert_eq!(test_worker_count(3, 9, 8, Some(100)), 9);
        assert_eq!(test_worker_count(3, 9, 8, Some(0)), 1);
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

    #[test]
    fn model_parses_group_field() {
        let y = parse_yaml(
            r#"clients:
  - type: openai
    name: agnes
    api_base: https://x.example/v1
    api_key: sk-test
    models:
      - name: m1
        group: fast, cheap
      - name: m2
        group: fast
      - name: m3
        group: ""
      - name: m4
"#,
        )
        .unwrap();
        let cfg = config_from_yaml(&y).unwrap();
        let groups: Vec<Vec<String>> = cfg.providers[0]
            .models
            .iter()
            .map(|m| m.group.clone())
            .collect();
        assert_eq!(groups, vec![
            vec!["fast".to_string(), "cheap".to_string()],
            vec!["fast".to_string()],
            Vec::<String>::new(),
            Vec::<String>::new(),
        ]);
    }

    #[test]
    fn missing_capability_fields_reports_only_absent_ones() {
        let full = Model {
            name: "m".into(),
            weight: 1,
            max_tokens: None,
            max_input_tokens: Some(4096),
            model_type: Some("text,image".into()),
            group: Vec::new(),
        };
        assert_eq!(missing_capability_fields(&full), (false, false));
        let no_ctx = Model { max_input_tokens: None, ..full.clone() };
        assert_eq!(missing_capability_fields(&no_ctx), (true, false));
        let no_type = Model { model_type: None, ..full.clone() };
        assert_eq!(missing_capability_fields(&no_type), (false, true));
        let bare = Model { max_input_tokens: None, model_type: None, ..full.clone() };
        assert_eq!(missing_capability_fields(&bare), (true, true));
    }

    // --- `sys ai chat --test` ----------------------------------------------

    fn local_provider(port: u16) -> Provider {
        Provider {
            kind: "openai".to_string(),
            name: "local".to_string(),
            api_base: format!("http://127.0.0.1:{port}/v1"),
            api_key: "sk-local".to_string(),
            models: Vec::new(),
        }
    }

    fn local_model() -> Model {
        Model {
            name: "m1".to_string(),
            weight: 1,
            max_tokens: None,
            max_input_tokens: None,
            model_type: None,
            group: Vec::new(),
        }
    }

    /// Serve one plain HTTP response (status line + body), then close.
    fn spawn_one_shot_server(status_line: &'static str, body: &'static [u8]) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf);
                let head = format!("{status_line}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n");
                let _ = sock.write_all(head.as_bytes());
                let _ = sock.write_all(body);
                let _ = sock.flush();
            }
        });
        port
    }

    /// SSE server: headers + an empty-role delta immediately, the first
    /// content delta only after a 300 ms sleep, then close.
    fn spawn_sse_first_token_server() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf);
                let _ = sock.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                );
                let _ = sock.flush();
                let _ = sock.write_all(
                    b"data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"},\"index\":0}]}\r\n\r\n",
                );
                let _ = sock.flush();
                std::thread::sleep(Duration::from_millis(300));
                let _ = sock.write_all(
                    b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"index\":0}]}\r\n\r\ndata: [DONE]\r\n\r\n",
                );
                let _ = sock.flush();
            }
        });
        port
    }

    #[test]
    fn test_targets_uses_override_or_all_models() {
        let cfg = sample_config(); // agnes (1 model) + claude (1 model)
        let all = test_targets(&cfg, None).unwrap();
        assert_eq!(all.len(), 2);
        let one = test_targets(&cfg, Some("agnes-3.0-flash")).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(cfg.providers[one[0].provider].name, "agnes");
        assert_eq!(cfg.providers[one[0].provider].models[one[0].model].name, "agnes-3.0-flash");
        let sel = test_targets(&cfg, Some("claude:claude-3-5-sonnet")).unwrap();
        assert_eq!(sel.len(), 1);
        assert_eq!(cfg.providers[sel[0].provider].name, "claude");
    }

    #[test]
    fn first_token_measures_until_content_delta() {
        let port = spawn_sse_first_token_server();
        let p = local_provider(port);
        let m = local_model();
        let elapsed = test_one(&p, &m, "hello").unwrap();
        // The role-only delta must be skipped; the measured time must include
        // the 300 ms server delay before the first content delta.
        assert!(
            elapsed.as_millis() >= 250,
            "expected >= 250 ms (300 ms server delay), got {elapsed:?}"
        );
    }

    #[test]
    fn first_token_fails_on_http_error() {
        let port = spawn_one_shot_server("HTTP/1.1 429 Too Many Requests", b"{\"error\":{\"message\":\"rate limited\"}}");
        let p = local_provider(port);
        let m = local_model();
        let err = test_one(&p, &m, "hello").unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("429"), "expected HTTP 429 in error, got: {text}");
    }

    #[test]
    fn first_token_fails_on_stream_error() {
        let port = spawn_one_shot_server("HTTP/1.1 200 OK", b"data: {\"error\":{\"message\":\"boom\"}}\r\n\r\n");
        let p = local_provider(port);
        let m = local_model();
        let err = test_one(&p, &m, "hello").unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("boom"), "expected provider error message, got: {text}");
    }

    #[test]
    fn first_token_fails_when_stream_has_no_content() {
        let port = spawn_one_shot_server(
            "HTTP/1.1 200 OK",
            b"data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"},\"index\":0}]}\r\n\r\ndata: [DONE]\r\n\r\n",
        );
        let p = local_provider(port);
        let m = local_model();
        assert!(test_one(&p, &m, "hello").is_err());
    }

    #[test]
    fn models_url_builds_models_endpoint() {
        // OpenAI-compatible: `models` is appended to the api_base as-is.
        let p = local_provider(1234); // api_base ends with /v1
        assert_eq!(models_url(&p), "http://127.0.0.1:1234/v1/models");
        let p2 = Provider {
            api_base: "https://gateway.example.com".to_string(),
            ..local_provider(1)
        };
        assert_eq!(models_url(&p2), "https://gateway.example.com/models");
        // Baidu Qianfan: the list lives at /v2/models, not /v2/v1/models.
        let p3 = Provider {
            api_base: "https://qianfan.baidubce.com/v2".to_string(),
            ..local_provider(1)
        };
        assert_eq!(models_url(&p3), "https://qianfan.baidubce.com/v2/models");
        // Anthropic: {api_base}/v1/models, or plain /models when base ends with /v1.
        let ant = |base: &str| Provider {
            kind: "anthropic".to_string(),
            api_base: base.to_string(),
            ..local_provider(1)
        };
        assert_eq!(models_url(&ant("https://api.anthropic.com")), "https://api.anthropic.com/v1/models");
        assert_eq!(models_url(&ant("https://api.anthropic.com/v1")), "https://api.anthropic.com/v1/models");
    }

    #[test]
    fn parse_models_response_extracts_data_and_models_arrays() {
        let (entries, next) = parse_models_response(
            r#"{"data":[{"id":"a-1","owned_by":"acme"},{"id":"b-2"}],"next_page":"tok"}"#,
        )
        .unwrap();
        assert_eq!(
            entries,
            vec![("a-1".to_string(), Some("acme".to_string())), ("b-2".to_string(), None)]
        );
        assert_eq!(next.as_deref(), Some("tok"));
        // `models[]` fallback shape.
        let (entries2, next2) = parse_models_response(r#"{"models":[{"id":"m1"}]}"#).unwrap();
        assert_eq!(entries2, vec![("m1".to_string(), None)]);
        assert!(next2.is_none());
    }

    #[test]
    fn parse_models_response_strips_bom_and_reports_bad_shapes() {
        // A leading UTF-8 BOM must not break strict JSON parsing.
        let body = "\u{feff}{\"data\":[{\"id\":\"x\"}]}";
        let (entries, _) = parse_models_response(body).unwrap();
        assert_eq!(entries, vec![("x".to_string(), None)]);
        // `error` object -> Err with the provider message.
        let err = parse_models_response(r#"{"error":{"message":"invalid key"}}"#).unwrap_err();
        assert!(format!("{err:#}").contains("invalid key"));
        // Neither data nor models -> Err.
        assert!(parse_models_response(r#"{"foo":[]}"#).is_err());
        // Entry without an id -> Err.
        assert!(parse_models_response(r#"{"data":[{"owned_by":"x"}]}"#).is_err());
    }

    #[test]
    fn fetch_provider_models_reads_local_endpoint() {
        let port = spawn_one_shot_server(
            "HTTP/1.1 200 OK",
            b"{\"data\":[{\"id\":\"a\"},{\"id\":\"b\",\"owned_by\":\"o\"}]}",
        );
        let p = local_provider(port);
        let entries = fetch_provider_models(&p, &models_url(&p)).unwrap();
        let ids: Vec<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(entries[1].1.as_deref(), Some("o"));
    }

    #[test]
    fn fetch_provider_models_fails_on_http_error() {
        let port = spawn_one_shot_server("HTTP/1.1 403 Forbidden", b"{\"error\":{\"message\":\"denied\"}}");
        let p = local_provider(port);
        let err = fetch_provider_models(&p, &models_url(&p)).unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("403"), "expected HTTP 403 in error, got: {text}");
    }

    #[test]
    fn fetch_provider_models_follows_next_page() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let pages: [&[u8]; 2] = [
                br#"{"data":[{"id":"p1"}],"next_page":"abc"}"#,
                br#"{"data":[{"id":"p2"}]}"#,
            ];
            for body in pages {
                if let Ok((mut sock, _)) = listener.accept() {
                    let mut buf = [0u8; 4096];
                    let _ = sock.read(&mut buf);
                    let _ = sock.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                    );
                    let _ = sock.write_all(body);
                    let _ = sock.flush();
                }
            }
        });
        let p = local_provider(port);
        let entries = fetch_provider_models(&p, &models_url(&p)).unwrap();
        let ids: Vec<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["p1", "p2"]);
    }

    #[test]
    fn query_provider_models_rejects_unknown_provider() {
        let cfg = Config {
            model: None,
            stream: false,
            providers: vec![local_provider(1234)],
            free: false,
            search: Vec::new(),
        };
        let err = query_provider_models(&cfg, "nope").unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("available"), "expected available list, got: {text}");
    }

    // --- `sys ai chat --web`: config search section + provider selection + result parsing ---

    #[test]
    fn config_parses_search_section() {
        let y = parse_yaml(
            r#"search:
  - name: bochaai
    key: sk-test-key
"#,
        )
        .unwrap();
        let cfg = config_from_yaml(&y).unwrap();
        assert_eq!(cfg.search.len(), 1);
        assert_eq!(cfg.search[0].name, "bochaai");
        assert_eq!(cfg.search[0].key, "sk-test-key");
        assert!(cfg.providers.is_empty());
        // Entries missing name or key are skipped silently.
        let y = parse_yaml(
            r#"search:
  - name: nokey
  - key: noname
  - name: ok
    key: sk-good
"#,
        )
        .unwrap();
        let cfg = config_from_yaml(&y).unwrap();
        assert_eq!(cfg.search.len(), 1);
        assert_eq!(cfg.search[0].name, "ok");
        assert_eq!(cfg.search[0].key, "sk-good");
    }

    #[test]
    fn pick_search_entry_auto_bocha_then_yuanbao_then_first() {
        let key = |n: &str| SearchKey { name: n.to_string(), key: "k".to_string() };
        // auto: the entry whose name mentions `bocha` wins even when not first.
        let cfg = Config { model: None, stream: false, providers: Vec::new(), free: false, search: vec![key("other"), key("bochaai")] };
        assert_eq!(pick_search_entry(&cfg, None, Path::new("c.yaml")).unwrap().name, "bochaai");
        // auto with both engines configured: bocha still preferred.
        let cfg = Config { model: None, stream: false, providers: Vec::new(), free: false, search: vec![key("yuanbao"), key("bocha")] };
        assert_eq!(pick_search_entry(&cfg, Some("auto"), Path::new("c.yaml")).unwrap().name, "bocha");
        // auto without bocha: the yuanbao entry.
        let cfg = Config { model: None, stream: false, providers: Vec::new(), free: false, search: vec![key("other"), key("yuanbao")] };
        assert_eq!(pick_search_entry(&cfg, None, Path::new("c.yaml")).unwrap().name, "yuanbao");
        // auto with neither engine: the first entry.
        let cfg = Config { model: None, stream: false, providers: Vec::new(), free: false, search: vec![key("first")] };
        assert_eq!(pick_search_entry(&cfg, None, Path::new("c.yaml")).unwrap().name, "first");
        // No entries: a clear error pointing at the config path.
        let cfg = Config { model: None, stream: false, providers: Vec::new(), free: false, search: Vec::new() };
        let err = pick_search_entry(&cfg, None, Path::new("c.yaml")).unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("no web-search key configured"), "got: {text}");
        assert!(text.contains("c.yaml"), "got: {text}");
    }

    #[test]
    fn pick_search_entry_explicit_provider_and_unknown() {
        let key = |n: &str| SearchKey { name: n.to_string(), key: "k".to_string() };
        let cfg = Config { model: None, stream: false, providers: Vec::new(), free: false, search: vec![key("bocha"), key("yuanbao")] };
        assert_eq!(pick_search_entry(&cfg, Some("yuanbao"), Path::new("c.yaml")).unwrap().name, "yuanbao");
        assert_eq!(pick_search_entry(&cfg, Some("bocha"), Path::new("c.yaml")).unwrap().name, "bocha");
        // Pinned provider missing: engine-specific error.
        let cfg = Config { model: None, stream: false, providers: Vec::new(), free: false, search: vec![key("bocha")] };
        let err = pick_search_entry(&cfg, Some("yuanbao"), Path::new("c.yaml")).unwrap_err();
        assert!(format!("{err:#}").contains("no Yuanbao search key configured"), "got: {err:#}");
        // Unknown selector: clear error.
        let err = pick_search_entry(&cfg, Some("nope"), Path::new("c.yaml")).unwrap_err();
        assert!(format!("{err:#}").contains("unknown search provider `nope`"), "got: {err:#}");
        // An entry whose name matches the selector is used as-is.
        let cfg = Config { model: None, stream: false, providers: Vec::new(), free: false, search: vec![key("my-bocha-proxy")] };
        assert_eq!(pick_search_entry(&cfg, Some("my-bocha-proxy"), Path::new("c.yaml")).unwrap().name, "my-bocha-proxy");
        // Empty key is rejected.
        let cfg = Config { model: None, stream: false, providers: Vec::new(), free: false, search: vec![SearchKey { name: "yuanbao".to_string(), key: String::new() }] };
        let err = pick_search_entry(&cfg, Some("yuanbao"), Path::new("c.yaml")).unwrap_err();
        assert!(format!("{err:#}").contains("empty key"), "got: {err:#}");
    }

    #[test]
    fn search_engine_of_detects_yuanbao_by_name() {
        assert_eq!(search_engine_of("yuanbao"), SearchEngine::Yuanbao);
        assert_eq!(search_engine_of("YuanBao-1"), SearchEngine::Yuanbao);
        assert_eq!(search_engine_of("bochaai"), SearchEngine::Bocha);
        assert_eq!(search_engine_of("anything"), SearchEngine::Bocha);
    }

    #[test]
    fn bocha_rows_parse_webpages_value() {
        let v: Value = serde_json::from_str(
            r#"{
  "queryContext": {"originalQuery": "x"},
  "webPages": {
    "totalEstimatedMatches": 2,
    "value": [
      {"name": "标题A", "url": "https://a.example/1", "snippet": "摘要A", "siteName": "站点A", "datePublished": "2026-10-01T00:00:00+08:00"},
      {"name": "标题B", "url": "https://b.example/2"}
    ]
  }
}"#,
        )
        .unwrap();
        let text = bocha_result_text(&v, false);
        assert!(text.contains("[ 1] 标题A\n    https://a.example/1"), "got: {text}");
        assert!(text.contains("[ 2] 标题B\n    https://b.example/2"), "got: {text}");
        assert!(!text.contains("摘要A"), "summary must be off by default: {text}");
        let text = bocha_result_text(&v, true);
        assert!(text.contains("摘要A"), "got: {text}");
        assert!(text.contains("站点A · 2026-10-01T00:00:00+08:00"), "got: {text}");
    }

    #[test]
    fn bocha_rows_parse_legacy_web_results() {
        let v: Value = serde_json::from_str(
            r#"{
  "web_results": [
    {"title": "老格式A", "url": "https://old.example/a", "summary": "摘要", "site_name": "站", "page_timestamp": "2026-10-01"},
    {"title": "老格式B", "url": "https://old.example/b"}
  ]
}"#,
        )
        .unwrap();
        let text = bocha_result_text(&v, true);
        assert!(text.contains("[ 1] 老格式A\n    https://old.example/a"), "got: {text}");
        assert!(text.contains("[ 2] 老格式B\n    https://old.example/b"), "got: {text}");
    }

    #[test]
    fn bocha_rows_skip_invalid_and_dedupe() {
        let v: Value = serde_json::from_str(
            r#"{
  "webPages": {"value": [
    {"name": "", "url": "https://x.example/empty-title"},
    {"name": "无链接", "url": ""},
    {"name": "重复", "url": "https://x.example/dup"},
    {"name": "重复", "url": "https://x.example/dup"},
    {"name": "有效", "url": "https://x.example/ok"}
  ]}
}"#,
        )
        .unwrap();
        let text = bocha_result_text(&v, false);
        assert!(text.contains("[ 1] 重复\n    https://x.example/dup"), "got: {text}");
        assert!(text.contains("[ 2] 有效\n    https://x.example/ok"), "got: {text}");
        assert_eq!(text.matches("https://x.example/dup").count(), 1);
    }

    #[test]
    fn bocha_result_items_covers_messages_and_webpages() {
        // Rows are gathered from messages-source and webPages.value alike.
        let v: Value = serde_json::from_str(
            r#"{
  "messages": [
    {"role": "assistant", "type": "source", "content": {"value": [
      {"name": "消息源A", "url": "https://x.example/a"}
    ]}}
  ],
  "webPages": {"value": [
    {"name": "网页B", "url": "https://x.example/b"}
  ]}
}"#,
        )
        .unwrap();
        let text = bocha_result_text(&v, false);
        assert!(text.contains("[ 1] 消息源A\n    https://x.example/a"), "got: {text}");
        assert!(text.contains("[ 2] 网页B\n    https://x.example/b"), "got: {text}");
    }

    #[test]
    fn bocha_result_text_empty_when_no_rows() {
        let v: Value = serde_json::from_str(r#"{"webPages": {"value": []}}"#).unwrap();
        assert_eq!(bocha_result_text(&v, true), "（未返回搜索结果）");
        assert_eq!(bocha_result_text(&Value::Null, true), "（未返回搜索结果）");
    }

    #[test]
    fn web_search_message_contains_results_and_question() {
        let msg = web_search_message("[ 1] 标题\n    https://a.example/1\n    摘要A", "今天天气如何");
        assert!(msg.starts_with("请基于以下联网搜索结果回答用户的问题"), "got: {msg}");
        assert!(msg.contains("=== 联网搜索结果 ==="), "got: {msg}");
        assert!(msg.contains("[ 1] 标题\n    https://a.example/1"), "got: {msg}");
        assert!(msg.contains("=== 用户问题 ==="), "got: {msg}");
        assert!(msg.ends_with("今天天气如何"), "got: {msg}");
    }

    // --- Tencent Yuanbao Search (WSA) result parsing -------------------------

    #[test]
    fn yuanbao_pages_parse_json_strings() {
        let v: Value = serde_json::from_str(
            r#"{
  "Response": {
    "Pages": [
      "{\"title\":\"三星堆的由来\",\"url\":\"https://x.example/a\",\"passage\":\"长摘要A\",\"site\":\"腾讯网\",\"date\":\"2024-09-03 12:59:00\",\"score\":0.9}",
      "{\"title\":\"无链接\",\"url\":\"\"}",
      "not json at all",
      "{\"url\":\"https://x.example/no-title\"}",
      "{\"title\":\"第二条\",\"url\":\"https://x.example/b\",\"passage\":\"摘要B\"}"
    ],
    "Query": "三星堆",
    "RequestId": "r1"
  }
}"#,
        )
        .unwrap();
        let text = yuanbao_result_text(&v);
        assert!(text.contains("[ 1] 三星堆的由来\n    https://x.example/a"), "got: {text}");
        assert!(text.contains("长摘要A"), "got: {text}");
        assert!(text.contains("腾讯网 · 2024-09-03 12:59:00"), "got: {text}");
        assert!(text.contains("[ 2] 第二条\n    https://x.example/b"), "got: {text}");
        assert!(text.contains("摘要B"), "got: {text}");
        // Rows without a title/url and unparseable strings are skipped.
        assert!(!text.contains("无链接"), "got: {text}");
        assert!(!text.contains("no-title"), "got: {text}");
    }

    #[test]
    fn yuanbao_result_keeps_five_best() {
        let mut pages = Vec::new();
        for i in 0..8 {
            let p = json!({"title": format!("标题{i}"), "url": format!("https://x.example/{i}")}).to_string();
            pages.push(json!(p));
        }
        let v = json!({"Response": {"Pages": pages}});
        let text = yuanbao_result_text(&v);
        assert!(text.contains("[ 5] 标题4\n    https://x.example/4"), "got: {text}");
        assert!(!text.contains("标题5"), "got: {text}");
    }

    #[test]
    fn yuanbao_result_text_empty_when_no_pages() {
        let v: Value = serde_json::from_str(r#"{"Response": {"Pages": []}}"#).unwrap();
        assert_eq!(yuanbao_result_text(&v), "（未返回搜索结果）");
        assert_eq!(yuanbao_result_text(&Value::Null), "（未返回搜索结果）");
    }

    #[test]
    fn yuanbao_response_error_reports_code_and_message() {
        let v: Value = serde_json::from_str(
            r#"{"Response": {"Error": {"Code": "UnauthorizedOperation", "Message": "请检查服务密钥 API KEY 是否正确"}, "RequestId": "r1"}}"#,
        )
        .unwrap();
        let err = yuanbao_response_error(&v).expect("error must be detected");
        assert!(err.contains("UnauthorizedOperation"), "got: {err}");
        assert!(err.contains("服务密钥"), "got: {err}");
        // No Error object: None.
        let v: Value = serde_json::from_str(r#"{"Response": {"Pages": []}}"#).unwrap();
        assert!(yuanbao_response_error(&v).is_none());
    }
}
