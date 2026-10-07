//! `sys ai server [ADDR]` — serve the configured providers as an
//! OpenAI-compatible chat-completions API.
//!
//! - Default bind address: `127.0.0.1:10000`.
//! - A bare port (`--server 8080`) keeps the default host; a bare IP
//!   (`--server 0.0.0.0`) keeps the default port.
//! - Endpoints: `POST /v1/chat/completions` (stream and non-stream passthrough
//!   to the upstream provider) and `GET /v1/models` (the configured models).
//! - Model names follow the chat selector rules: `{provider}:{model}`,
//!   `{provider}:*`, a bare `{model}` (first match), or `auto` / empty (the
//!   config default). Only OpenAI-compatible providers (`type: openai`) are
//!   served; anthropic targets are rejected with a clear error.

use crate::chat::{self, Config};
use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

pub(crate) const DEFAULT_HOST: &str = "127.0.0.1";
pub(crate) const DEFAULT_PORT: u16 = 10000;
const STREAM_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_BODY: usize = 32 * 1024 * 1024;

/// Parse the `--server` value: empty -> host:port; a bare decimal port ->
/// host:port; a bare IP -> ip:port; `IP:PORT` -> as-is. IPv4 only.
pub(crate) fn parse_addr(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return format!("{DEFAULT_HOST}:{DEFAULT_PORT}");
    }
    if let Ok(port) = s.parse::<u16>() {
        return format!("{DEFAULT_HOST}:{port}");
    }
    if let Some((ip, port)) = s.rsplit_once(':') {
        if !ip.is_empty() {
            if port.parse::<u16>().is_ok() {
                return s.to_string();
            }
            return format!("{ip}:{DEFAULT_PORT}");
        }
    }
    format!("{s}:{DEFAULT_PORT}")
}

/// Probe whether an OpenAI-compatible server is already listening at `addr`
/// (GET /v1/models with a short timeout). Bind-all hosts (0.0.0.0 / ::) are
/// probed through loopback, which is where the service is reachable locally.
pub(crate) fn is_running(addr: &str) -> bool {
    let probe = if let Some((host, port)) = addr.rsplit_once(':') {
        if host == "0.0.0.0" || host == "::" || host == "[::]" {
            format!("127.0.0.1:{port}")
        } else {
            addr.to_string()
        }
    } else {
        addr.to_string()
    };
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return false;
    };
    client
        .get(format!("http://{probe}/v1/models"))
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Start the OpenAI-compatible server and block forever.
pub(crate) fn serve(config: Option<&Path>, addr: &str) -> Result<()> {
    let (cfg, _path) = chat::load_config(config)?;
    let listener = TcpListener::bind(addr).with_context(|| format!("cannot listen on {addr}"))?;
    eprintln!("sys: serving OpenAI-compatible API on http://{addr}/v1/chat/completions (Ctrl-C to stop)");
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                if let Err(e) = handle(s, &cfg) {
                    eprintln!("sys: request error: {e:#}");
                }
            }
            Err(e) => eprintln!("sys: accept error: {e}"),
        }
    }
    Ok(())
}

struct Request {
    method: String,
    path: String,
    body: String,
}

fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let mut reader = BufReader::new(stream.try_clone().context("cannot clone stream")?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        bail!("empty request");
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let path = target.split('?').next().unwrap_or("").to_string();
    if method.is_empty() || path.is_empty() {
        bail!("malformed request line: {line:?}");
    }
    let mut content_length = 0usize;
    loop {
        let mut l = String::new();
        if reader.read_line(&mut l)? == 0 {
            break;
        }
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
    }
    if content_length > MAX_BODY {
        bail!("request body too large ({content_length} bytes)");
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;
    let body = String::from_utf8_lossy(&body).into_owned();
    Ok(Request { method, path, body })
}

fn write_head(stream: &mut TcpStream, status: u16, reason: &str, extra: &[(&str, String)]) -> Result<()> {
    write!(stream, "HTTP/1.1 {status} {reason}\r\n")?;
    for (k, v) in extra {
        write!(stream, "{k}: {v}\r\n")?;
    }
    write!(stream, "\r\n")?;
    Ok(())
}

fn write_error(stream: &mut TcpStream, status: u16, message: &str) -> Result<()> {
    let body = serde_json::to_string(&json!({ "error": { "message": message, "type": "invalid_request_error" } }))?;
    write_head(
        stream,
        status,
        if status == 404 { "Not Found" } else { "Bad Request" },
        &[
            ("Content-Type", "application/json".to_string()),
            ("Content-Length", body.len().to_string()),
            ("Connection", "close".to_string()),
        ],
    )?;
    stream.write_all(body.as_bytes())?;
    Ok(())
}

fn handle(mut stream: TcpStream, cfg: &Config) -> Result<()> {
    let req = match read_request(&mut stream) {
        Ok(r) => r,
        Err(e) => {
            let _ = write_error(&mut stream, 400, &format!("bad request: {e:#}"));
            return Ok(());
        }
    };
    if req.method == "GET" && (req.path == "/v1/models" || req.path == "/models") {
        return list_models(&mut stream, cfg);
    }
    if req.method != "POST" || (req.path != "/v1/chat/completions" && req.path != "/chat/completions") {
        return write_error(&mut stream, 404, "not found");
    }
    let body: Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => return write_error(&mut stream, 400, &format!("invalid JSON body: {e}")),
    };
    let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("");
    let selector = if model.is_empty() || model == "auto" { "<default>" } else { model };
    let targets = match chat::resolve_targets(cfg, selector) {
        Ok(t) => t,
        Err(e) => return write_error(&mut stream, 404, &format!("unknown model `{model}`: {e:#}")),
    };
    let t = &targets[0];
    let p = &cfg.providers[t.provider];
    let m = &p.models[t.model];
    if p.kind != "openai" {
        return write_error(
            &mut stream,
            400,
            &format!("provider `{}` is type `{}`; the server only serves OpenAI-compatible providers", p.name, p.kind),
        );
    }
    if let Err(e) = proxy(&mut stream, p, m, body) {
        eprintln!("sys: upstream error: {e:#}");
        let _ = write_error(&mut stream, 502, &format!("upstream error: {e:#}"));
    }
    Ok(())
}

fn proxy(stream: &mut TcpStream, p: &chat::Provider, m: &chat::Model, mut body: Value) -> Result<()> {
    let want_stream = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    body["model"] = json!(m.name);
    let url = chat::chat_url(p);
    let client = Client::builder().timeout(STREAM_TIMEOUT).build().context("failed to build HTTP client")?;
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(chat::USER_AGENT_STR));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let auth = format!("Bearer {}", p.api_key);
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&auth).context("invalid api_key")?);
    if want_stream {
        headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    }
    let resp = client
        .post(&url)
        .headers(headers)
        .body(serde_json::to_vec(&body).context("cannot serialize request body")?)
        .send()
        .with_context(|| format!("request to {url} failed"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        let snippet: String = text.chars().take(500).collect();
        bail!("{url} returned HTTP {status}: {snippet}");
    }
    if want_stream {
        // SSE passthrough, chunked so the client knows when the stream ends.
        write_head(
            stream,
            200,
            "OK",
            &[
                ("Content-Type", "text/event-stream".to_string()),
                ("Transfer-Encoding", "chunked".to_string()),
                ("Connection", "close".to_string()),
                ("Cache-Control", "no-cache".to_string()),
            ],
        )?;
        let mut reader = resp;
        let mut buf = [0u8; 8192];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            write!(stream, "{n:x}\r\n")?;
            stream.write_all(&buf[..n])?;
            stream.write_all(b"\r\n")?;
        }
        stream.write_all(b"0\r\n\r\n")?;
    } else {
        let bytes = resp.bytes().context("cannot read upstream response")?;
        write_head(
            stream,
            200,
            "OK",
            &[
                ("Content-Type", "application/json".to_string()),
                ("Content-Length", bytes.len().to_string()),
                ("Connection", "close".to_string()),
            ],
        )?;
        stream.write_all(&bytes)?;
    }
    Ok(())
}

fn list_models(stream: &mut TcpStream, cfg: &Config) -> Result<()> {
    let data: Vec<Value> = cfg
        .providers
        .iter()
        .flat_map(|p| {
            p.models.iter().map(move |m| {
                json!({
                    "id": format!("{}:{}", p.name, m.name),
                    "object": "model",
                    "created": 0,
                    "owned_by": p.name,
                })
            })
        })
        .collect();
    let body = serde_json::to_vec(&json!({ "object": "list", "data": data }))?;
    write_head(
        stream,
        200,
        "OK",
        &[
            ("Content-Type", "application/json".to_string()),
            ("Content-Length", body.len().to_string()),
            ("Connection", "close".to_string()),
        ],
    )?;
    stream.write_all(&body)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_addr_variants() {
        assert_eq!(parse_addr(""), "127.0.0.1:10000");
        assert_eq!(parse_addr("   "), "127.0.0.1:10000");
        assert_eq!(parse_addr("8080"), "127.0.0.1:8080");
        assert_eq!(parse_addr("0.0.0.0"), "0.0.0.0:10000");
        assert_eq!(parse_addr("0.0.0.0:9000"), "0.0.0.0:9000");
        assert_eq!(parse_addr("192.168.1.5:1234"), "192.168.1.5:1234");
        assert_eq!(parse_addr("127.0.0.1"), "127.0.0.1:10000");
    }

    #[test]
    fn parse_addr_ignores_non_numeric_port_suffix() {
        assert_eq!(parse_addr("127.0.0.1:http"), "127.0.0.1:10000");
        assert_eq!(parse_addr("localhost:8080"), "localhost:8080");
    }
}
