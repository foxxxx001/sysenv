//! `sys ai image` — generate images through the OpenAI-compatible Images
//! API of a provider configured in `~/.sys/config.yaml` (default location;
//! `-c/--config` overrides it).
//!
//! The request goes to `POST {api_base}/images/generations` with a Bearer key
//! (same `clients` schema and model selection rules as `ai chat`). The response
//! `data[]` items may carry `b64_json` (default request) or `url` (`--url`);
//! every image is saved to the output directory (default: current directory)
//! and the saved paths are printed. Only OpenAI-compatible providers
//! (`type: openai`) are supported; `type: anthropic` targets are rejected.

use crate::chat;
use anyhow::{Context, Result, bail};
use base64::Engine;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Value};
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Requested image size sent to the API when `--size` is not given.
pub(crate) const DEFAULT_SIZE: &str = "1024x1024";
/// HTTP timeout for the generation request and for URL downloads.
const IMAGE_TIMEOUT: Duration = Duration::from_secs(300);

/// `POST {api_base}/images/generations` — the OpenAI Images API endpoint.
fn image_url(p: &chat::Provider) -> String {
    format!("{}/images/generations", p.api_base.trim_end_matches('/'))
}

/// Request body of the Images API. `response_format` is `b64_json` by default
/// (saved locally without extra network hops) and `url` when `want_url` is set
/// (the URLs are downloaded afterwards).
fn image_body(m: &chat::Model, prompt: &str, n: u32, size: &str, want_url: bool) -> Value {
    json!({
        "model": m.name,
        "prompt": prompt,
        "n": n,
        "size": size,
        "response_format": if want_url { "url" } else { "b64_json" },
    })
}

/// One generated image from the response `data` array: either base64 pixels
/// (`b64_json`) or a remote URL (`url`).
#[derive(Debug)]
struct ImageItem {
    b64: Option<String>,
    url: Option<String>,
}

/// Parse an Images API response body into its `data` items. Provider `error`
/// objects are reported as a proper error message.
fn parse_images(body: &str) -> Result<Vec<ImageItem>> {
    let v: Value = serde_json::from_str(body)
        .with_context(|| format!("provider returned invalid JSON: {}", body.chars().take(200).collect::<String>()))?;
    if let Some(err) = v.get("error") {
        let raw = err.to_string();
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or(&raw);
        bail!("provider error: {msg}");
    }
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .context("openai image response missing data")?;
    if data.is_empty() {
        bail!("provider returned no images in data");
    }
    let mut out = Vec::with_capacity(data.len());
    for item in data {
        let b64 = item.get("b64_json").and_then(|b| b.as_str()).map(str::to_string);
        let url = item.get("url").and_then(|u| u.as_str()).map(str::to_string);
        if b64.is_none() && url.is_none() {
            bail!("image item has neither b64_json nor url: {item}");
        }
        out.push(ImageItem { b64, url });
    }
    Ok(out)
}

/// Decode a `b64_json` value; tolerates a `data:image/...;base64,` prefix that
/// some providers prepend.
fn decode_b64(s: &str) -> Result<Vec<u8>> {
    let s = match s.find(',') {
        Some(i) if s[..i].starts_with("data:") => &s[i + 1..],
        _ => s,
    };
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .context("cannot decode b64_json")
}

/// Guess the image file extension from the decoded magic bytes.
fn sniff_format(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "jpg"
    } else if bytes.starts_with(b"GIF8") {
        "gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "webp"
    } else {
        "bin"
    }
}

/// Download a remote image URL and return its bytes.
fn download_url(url: &str) -> Result<Vec<u8>> {
    let client = Client::builder()
        .timeout(IMAGE_TIMEOUT)
        .build()
        .context("failed to build HTTP client")?;
    let resp = client
        .get(url)
        .send()
        .with_context(|| format!("download {url} failed"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        let snippet: String = text.chars().take(300).collect();
        bail!("download {url} returned HTTP {status}: {snippet}");
    }
    resp.bytes().map(|b| b.to_vec()).context("cannot read downloaded image")
}

/// Local UTC stamp `YYYYMMDD-HHMMSS` used in output file names.
fn now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Civil-from-days (Howard Hinnant): convert days since 1970-01-01 to a date.
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// A path in `dir` that does not exist yet; collisions get a `-1`, `-2`, ...
/// suffix instead of being overwritten.
fn unique_path(dir: &Path, base: &str, ext: &str) -> PathBuf {
    let mut k = 0;
    loop {
        let name = if k == 0 {
            format!("{base}.{ext}")
        } else {
            format!("{base}-{k}.{ext}")
        };
        let p = dir.join(name);
        if !p.exists() {
            return p;
        }
        k += 1;
    }
}

/// Send one Images API request and return the parsed image items.
fn send_image(
    p: &chat::Provider,
    m: &chat::Model,
    prompt: &str,
    n: u32,
    size: &str,
    want_url: bool,
    debug: bool,
) -> Result<Vec<ImageItem>> {
    if p.kind == "anthropic" {
        bail!(
            "provider `{}` is type anthropic; the Images API needs an OpenAI-compatible provider (type: openai)",
            p.name
        );
    }
    let url = image_url(p);
    let body = image_body(m, prompt, n, size, want_url);
    let body_bytes = serde_json::to_vec(&body).context("cannot serialize request body")?;

    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(chat::USER_AGENT_STR));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let auth = format!("Bearer {}", p.api_key);
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&auth).context("invalid api_key")?);

    if debug {
        chat::debug_print_request(&url, &headers, &body_bytes);
    }

    let client = Client::builder()
        .timeout(IMAGE_TIMEOUT)
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
            chat::debug_print_response(status, &resp_headers, &text);
        }
        let snippet: String = text.chars().take(500).collect();
        bail!("{url} returned HTTP {status}: {snippet}");
    }

    let text = resp.text().context("cannot read response body")?;
    if debug {
        chat::debug_print_response(status, &resp_headers, &text);
    }
    parse_images(&text)
}

/// `sys ai image [PROMPT...] [-m MODEL] [-o DIR] [-n N] [-s SIZE] [--url] [-c FILE] [--debug] [--list-model] [--list-provider]`
///
/// The prompt is the joined arguments (or piped stdin when no argument is
/// given). The model selector (`-m/--model` or the top-level `model`) follows
/// the same `{provider}:{model}` / `{model}` / comma-separated rules as
/// `ai chat`, with weighted round-robin and weight adjustments on failure.
/// Every generated image is saved under the output directory and its path is
/// printed; the image bytes come from `b64_json` (default) or are downloaded
/// from the returned `url` (`--url`).
pub fn cmd_image(
    words: &[String],
    config: Option<&Path>,
    debug: bool,
    model: Option<&str>,
    list_model: bool,
    list_provider: bool,
    output: Option<PathBuf>,
    count: u32,
    size: &str,
    want_url: bool,
) -> Result<()> {
    let (cfg, path) = chat::load_config(config)?;

    if list_model || list_provider {
        if list_provider {
            chat::print_providers(&cfg);
        }
        if list_model {
            if list_provider {
                println!();
            }
            chat::print_models(&cfg);
        }
        return Ok(());
    }

    if count == 0 {
        bail!("--count must be at least 1");
    }

    let prompt = if !words.is_empty() {
        words.join(" ")
    } else if !std::io::stdin().is_terminal() {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).context("cannot read stdin")?;
        if s.trim().is_empty() {
            bail!("empty prompt from stdin; provide a prompt or pipe text via stdin");
        }
        s
    } else {
        bail!("provide a prompt: `sys ai image \"a red fox in the snow\"` (or pipe text via stdin)");
    };

    let selector = model.or(cfg.model.as_deref()).unwrap_or("<default>");
    let targets = chat::resolve_targets(&cfg, selector)?;
    if targets.is_empty() {
        bail!("no model targets for image generation (selector `{selector}` matched nothing)");
    }

    // Try the weighted pick first, then the remaining targets in order.
    let first = chat::pick_weighted(&cfg, &targets, selector, &path)?;
    let mut order: Vec<usize> = Vec::with_capacity(targets.len());
    order.push(first);
    for i in 0..targets.len() {
        if i != first {
            order.push(i);
        }
    }

    let out_dir = output.unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("cannot create directory {}", out_dir.display()))?;

    let mut had_failure = false;
    let mut errors: Vec<String> = Vec::new();
    for ti in order {
        let t = &targets[ti];
        let p = &cfg.providers[t.provider];
        let m = &p.models[t.model];
        eprintln!("sys: using {} / {} ({})", p.name, m.name, p.kind);
        match send_image(p, m, &prompt, count, size, want_url, debug) {
            Ok(items) => {
                let stamp = now_stamp();
                let mut saved: Vec<PathBuf> = Vec::with_capacity(items.len());
                for (i, item) in items.iter().enumerate() {
                    let bytes = if let Some(b64) = &item.b64 {
                        decode_b64(b64).with_context(|| format!("cannot decode b64_json of image {}", i + 1))?
                    } else {
                        let url = item.url.as_deref().context("image item has no url")?;
                        download_url(url)?
                    };
                    let ext = sniff_format(&bytes);
                    let base = format!("sys-ai-image-{stamp}-{}", i + 1);
                    let out_path = unique_path(&out_dir, &base, ext);
                    std::fs::write(&out_path, &bytes)
                        .with_context(|| format!("cannot write {}", out_path.display()))?;
                    saved.push(out_path);
                }
                // A model that succeeded after a previous failure gets its
                // weight bumped (max 9), persisted to the config file.
                if had_failure && m.weight < 9 {
                    if let Err(e) = chat::update_weight_in_config(&path, &p.name, &m.name, m.weight + 1) {
                        eprintln!("sys: warning: failed to persist weight bump for {} / {}: {e:#}", p.name, m.name);
                    }
                }
                for out_path in &saved {
                    println!("{}", out_path.display());
                }
                return Ok(());
            }
            Err(e) => {
                errors.push(format!("{} / {}: {e:#}", p.name, m.name));
                // Failure decrements the weight (min 1), persisted to config,
                // then the next model is tried automatically.
                if m.weight > 1 {
                    if let Err(e2) = chat::update_weight_in_config(&path, &p.name, &m.name, m.weight - 1) {
                        eprintln!("sys: warning: failed to persist weight drop for {} / {}: {e2:#}", p.name, m.name);
                    }
                }
                had_failure = true;
            }
        }
    }
    bail!("all {} model(s) failed: {}", targets.len(), errors.join(" | "));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn model(name: &str) -> chat::Model {
        chat::Model {
            name: name.to_string(),
            weight: 1,
            max_tokens: None,
            max_input_tokens: None,
            model_type: None,
        }
    }

    fn provider(kind: &str, name: &str, api_base: &str, models: Vec<chat::Model>) -> chat::Provider {
        chat::Provider {
            kind: kind.to_string(),
            name: name.to_string(),
            api_base: api_base.to_string(),
            api_key: "sk-test".to_string(),
            models,
        }
    }

    #[test]
    fn image_url_formats() {
        let p = provider("openai", "agnes", "https://api.example.com/v1", vec![model("gpt-image-1")]);
        assert_eq!(image_url(&p), "https://api.example.com/v1/images/generations");
        let p2 = provider("openai", "bigmodel", "https://open.bigmodel.cn/api/paas/v4/", vec![model("cogview")]);
        assert_eq!(image_url(&p2), "https://open.bigmodel.cn/api/paas/v4/images/generations");
    }

    #[test]
    fn image_body_shapes() {
        let m = model("dall-e-3");
        let b = image_body(&m, "a cat", 1, "1024x1024", false);
        assert_eq!(b["model"], "dall-e-3");
        assert_eq!(b["prompt"], "a cat");
        assert_eq!(b["n"], 1);
        assert_eq!(b["size"], "1024x1024");
        assert_eq!(b["response_format"], "b64_json");
        let b2 = image_body(&m, "a dog", 2, "512x512", true);
        assert_eq!(b2["n"], 2);
        assert_eq!(b2["size"], "512x512");
        assert_eq!(b2["response_format"], "url");
    }

    #[test]
    fn parse_images_extracts_b64_and_url() {
        let body = r#"{"created":123,"data":[{"b64_json":"aGVsbG8="},{"url":"https://cdn.example.com/a.png"}]}"#;
        let items = parse_images(body).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].b64.as_deref(), Some("aGVsbG8="));
        assert!(items[0].url.is_none());
        assert!(items[1].b64.is_none());
        assert_eq!(items[1].url.as_deref(), Some("https://cdn.example.com/a.png"));
    }

    #[test]
    fn parse_images_reports_provider_error() {
        let body = r#"{"error":{"message":"invalid api key"}}"#;
        let err = parse_images(body).unwrap_err();
        assert!(err.to_string().contains("invalid api key"), "got: {err}");
    }

    #[test]
    fn parse_images_missing_data_or_empty() {
        let err = parse_images(r#"{"created":1}"#).unwrap_err();
        assert!(err.to_string().contains("missing data"), "got: {err}");
        let err = parse_images(r#"{"created":1,"data":[]}"#).unwrap_err();
        assert!(err.to_string().contains("no images"), "got: {err}");
        let err = parse_images(r#"{"created":1,"data":[{"foo":1}]}"#).unwrap_err();
        assert!(err.to_string().contains("neither b64_json nor url"), "got: {err}");
    }

    #[test]
    fn decode_b64_plain_and_data_url() {
        assert_eq!(decode_b64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_b64("data:image/png;base64,aGVsbG8=").unwrap(), b"hello");
        assert!(decode_b64("!!!").is_err());
    }

    #[test]
    fn sniff_format_detects_common_types() {
        assert_eq!(sniff_format(b"\x89PNG\r\n\x1a\nrest"), "png");
        assert_eq!(sniff_format(b"\xff\xd8\xff\xe0"), "jpg");
        assert_eq!(sniff_format(b"GIF89a"), "gif");
        assert_eq!(sniff_format(b"RIFF\x00\x00\x00\x00WEBPVP8 "), "webp");
        assert_eq!(sniff_format(b"random"), "bin");
    }

    #[test]
    fn now_stamp_shape() {
        let s = now_stamp();
        assert_eq!(s.len(), 15, "got: {s}");
        let (date, time) = s.split_once('-').unwrap();
        assert_eq!(date.len(), 8);
        assert_eq!(time.len(), 6);
        assert!(date.chars().all(|c| c.is_ascii_digit()));
        assert!(time.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn unique_path_avoids_collisions() {
        let dir = std::env::temp_dir().join(format!("sys-image-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let a = unique_path(&dir, "img", "png");
        fs::write(&a, b"x").unwrap();
        let b = unique_path(&dir, "img", "png");
        assert_ne!(a, b);
        assert!(b.to_string_lossy().ends_with("img-1.png"));
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A minimal config used by the no-network tests below.
    fn write_config(dir: &Path, content: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let p = dir.join("config.yaml");
        fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn anthropic_target_rejected_before_http() {
        let dir = std::env::temp_dir().join(format!("sys-image-anthropic-{}", std::process::id()));
        let cfg = write_config(
            &dir,
            "model: claude:claude-3-5-sonnet\nclients:\n  - type: anthropic\n    name: claude\n    api_base: https://api.anthropic.com/v1\n    api_key: sk-test\n    models:\n      - name: claude-3-5-sonnet\n",
        );
        let err = cmd_image(
            &["a red fox".to_string()],
            Some(&cfg),
            false,
            None,
            false,
            false,
            None,
            1,
            "1024x1024",
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("type anthropic"), "got: {err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn listing_works_without_prompt() {
        let dir = std::env::temp_dir().join(format!("sys-image-list-{}", std::process::id()));
        let cfg = write_config(
            &dir,
            "clients:\n  - type: openai\n    name: agnes\n    api_base: https://api.example.com/v1\n    api_key: sk-test\n    models:\n      - name: gpt-image-1\n",
        );
        cmd_image(&[], Some(&cfg), false, None, true, true, None, 1, "1024x1024", false).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn zero_count_rejected() {
        let dir = std::env::temp_dir().join(format!("sys-image-count-{}", std::process::id()));
        let cfg = write_config(
            &dir,
            "clients:\n  - type: openai\n    name: agnes\n    api_base: https://api.example.com/v1\n    api_key: sk-test\n    models:\n      - name: gpt-image-1\n",
        );
        let err = cmd_image(
            &["x".to_string()],
            Some(&cfg),
            false,
            None,
            false,
            false,
            None,
            0,
            "1024x1024",
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("--count"), "got: {err}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
