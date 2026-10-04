//! `sysenv http` — an httpie-compatible HTTP client.
//!
//! Mirrors httpie's syntax: `http [flags] [METHOD] URL [ITEM...]`
//! with `key=value` (JSON/form data), `key:=json` (raw JSON),
//! `key==value` (query), `key:value` (headers), `key@file` (upload),
//! `key=@file` (embed file content), `@file` (raw body), `--raw` and stdin.

use anyhow::{Context, Result, bail};
use reqwest::blocking::multipart;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE, USER_AGENT};
use serde_json::{Map, Value};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const USER_AGENT_STR: &str = "HTTPie/3.2.4 (sysenv/0.1.0)";

pub struct HttpConfig {
    pub json: bool,
    pub form: bool,
    pub multipart: bool,
    pub raw: Option<String>,
    pub print: Option<String>,
    pub headers_only: bool,
    pub body_only: bool,
    pub meta_only: bool,
    pub verbose: bool,
    pub output: Option<PathBuf>,
    pub download: bool,
    pub quiet: bool,
    pub pretty: Option<String>,
    pub auth: Option<String>,
    pub auth_type: Option<String>,
    pub proxies: Vec<String>,
    pub follow: bool,
    pub max_redirects: Option<u32>,
    pub timeout: Option<f64>,
    pub check_status: bool,
    pub offline: bool,
    pub verify: Option<String>,
    pub ignore_stdin: bool,
    pub default_scheme: String,
    pub debug: bool,
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
enum Item {
    Header { name: String, value: Option<String>, from_file: Option<PathBuf> },
    Query { name: String, value: String },
    Data { name: String, value: String, raw: bool },
    Upload { name: String, path: PathBuf, mime: Option<String> },
    BodyFile(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
enum PathToken {
    Key(String),
    Index(usize),
    Append,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Raw,
    Query,
    Data,
    Header,
    Upload,
}

/// Is this argument a URL candidate (rather than a request item)?
fn looks_like_url(arg: &str) -> bool {
    if arg.contains("://") || arg.starts_with(':') {
        return true;
    }
    // lowercase hostname:port (e.g. localhost:8000, pie.dev:8080/api)
    let low = arg.to_ascii_lowercase();
    if arg == low {
        let host_port = arg.split('/').next().unwrap_or("");
        if let Some((h, p)) = host_port.split_once(':') {
            if !h.is_empty()
                && h.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
                && !p.is_empty()
                && p.chars().all(|c| c.is_ascii_digit())
            {
                return true;
            }
        }
    }
    false
}

fn is_method_token(arg: &str) -> bool {
    !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_' || c == '-')
        && arg.chars().any(|c| c.is_ascii_uppercase())
}

/// Scan left to right (honouring backslash escapes); at each position try the
/// two-character separators first, then the single-character ones.
fn find_sep(s: &str) -> Option<(usize, Op)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        let rest = &s[i..];
        if rest.starts_with(":=") {
            return Some((i, Op::Raw));
        }
        if rest.starts_with("==") {
            return Some((i, Op::Query));
        }
        match rest.as_bytes()[0] {
            b'=' => return Some((i, Op::Data)),
            b':' => return Some((i, Op::Header)),
            b'@' => {
                // An '@' only counts as an upload separator when there is a key.
                if !unescape(&s[..i]).is_empty() {
                    return Some((i, Op::Upload));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Remove backslash escapes from a key segment.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn read_stripped(path: &Path) -> Result<String> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read file `{}`", path.display()))?;
    Ok(content.trim_end_matches(['\n', '\r']).to_string())
}

fn parse_item(item: &str) -> Result<Item> {
    // Raw body from file: @file
    if let Some(rest) = item.strip_prefix('@') {
        if !rest.is_empty() && rest != "-" {
            return Ok(Item::BodyFile(PathBuf::from(rest)));
        }
    }

    let (pos, op) = find_sep(item).ok_or_else(|| {
        anyhow::anyhow!(
            "`{item}` is not a valid request item (expected key=value, key:=json, key==value, key:value, key@file, or @file)"
        )
    })?;
    let key = unescape(&item[..pos]);

    match op {
        Op::Raw => {
            if key.is_empty() {
                bail!("invalid request item `{item}`: empty field name");
            }
            let value = &item[pos + 2..];
            if let Some(path) = value.strip_prefix('@') {
                if !path.is_empty() {
                    let content = read_stripped(Path::new(path))?;
                    return Ok(Item::Data { name: key, value: content, raw: true });
                }
            }
            Ok(Item::Data { name: key, value: value.to_string(), raw: true })
        }
        Op::Query => {
            if key.is_empty() {
                bail!("invalid request item `{item}`: empty query name");
            }
            let value = &item[pos + 2..];
            if let Some(path) = value.strip_prefix('@') {
                if !path.is_empty() {
                    let content = read_stripped(Path::new(path))?;
                    return Ok(Item::Query { name: key, value: content });
                }
            }
            Ok(Item::Query { name: key, value: value.to_string() })
        }
        Op::Data => {
            if key.is_empty() {
                bail!("invalid request item `{item}`: empty field name");
            }
            let value = &item[pos + 1..];
            if let Some(path) = value.strip_prefix('@') {
                if !path.is_empty() {
                    let content = read_stripped(Path::new(path))?;
                    return Ok(Item::Data { name: key, value: content, raw: false });
                }
            }
            Ok(Item::Data { name: key, value: value.to_string(), raw: false })
        }
        Op::Header => {
            if key.is_empty() {
                bail!("invalid request item `{item}`: empty header name");
            }
            let value = &item[pos + 1..];
            if let Some(path) = value.strip_prefix('@') {
                if !path.is_empty() {
                    return Ok(Item::Header {
                        name: key,
                        value: None,
                        from_file: Some(PathBuf::from(path)),
                    });
                }
            }
            Ok(Item::Header {
                name: key,
                value: Some(value.to_string()),
                from_file: None,
            })
        }
        Op::Upload => {
            if key.is_empty() {
                bail!("invalid request item `{item}`: empty field name");
            }
            let value = &item[pos + 1..];
            if value.is_empty() {
                bail!("invalid upload item `{item}` (expected key@path[;type=mime])");
            }
            let (path, mime) = match value.split_once(';') {
                Some((p, m)) if m.starts_with("type=") => (p.to_string(), Some(m[5..].to_string())),
                _ => (value.to_string(), None),
            };
            Ok(Item::Upload { name: key, path: PathBuf::from(path), mime })
        }
    }
}

// ---------------------------------------------------------------------------
// JSON path building (httpie nested syntax: a[b][c], a[], a[1], []:=1)
// ---------------------------------------------------------------------------

fn parse_path(path: &str) -> Result<Vec<PathToken>> {
    let mut tokens: Vec<PathToken> = Vec::new();
    let bytes = path.as_bytes();
    let mut i = 0;
    let mut cur = String::new();
    let mut in_bracket = false;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '[' {
            if !cur.is_empty() {
                tokens.push(PathToken::Key(std::mem::take(&mut cur)));
            }
            let close_rel = path[i..]
                .find(']')
                .ok_or_else(|| anyhow::anyhow!("Expecting ']' in path `{path}`"))?;
            let close = i + close_rel;
            let inner = &path[i + 1..close];
            if inner.is_empty() {
                tokens.push(PathToken::Append);
            } else if inner.chars().all(|ch| ch.is_ascii_digit()) {
                tokens.push(PathToken::Index(
                    inner
                        .parse::<usize>()
                        .with_context(|| format!("invalid index in `{path}`"))?,
                ));
            } else {
                tokens.push(PathToken::Key(inner.to_string()));
            }
            i = close + 1;
            in_bracket = true;
        } else if c == ']' {
            bail!("Unexpected ']' in path `{path}`");
        } else if c == '\\' && i + 1 < bytes.len() {
            cur.push(bytes[i + 1] as char);
            i += 2;
        } else {
            cur.push(c);
            i += 1;
        }
    }
    if !cur.is_empty() {
        if in_bracket {
            bail!("Unexpected trailing text after ']' in path `{path}`");
        }
        tokens.push(PathToken::Key(cur));
    }
    if tokens.is_empty() {
        bail!("empty JSON path in `{path}`");
    }
    Ok(tokens)
}

fn json_apply(root: &mut Value, tokens: &[PathToken], value: Value) -> Result<()> {
    match tokens[0] {
        PathToken::Append | PathToken::Index(_) => {
            if root.is_null() {
                *root = Value::Array(Vec::new());
            }
            if !root.is_array() {
                bail!("type conflict: array access on a JSON object");
            }
        }
        PathToken::Key(_) => {
            if root.is_null() {
                *root = Value::Object(Map::new());
            }
            if !root.is_object() {
                bail!("type conflict: key access on a JSON array");
            }
        }
    }
    let mut node = root;
    for (idx, t) in tokens.iter().enumerate() {
        let last = idx + 1 == tokens.len();
        // An intermediate null node takes its container type from the next token.
        if node.is_null() {
            match t {
                PathToken::Key(_) => *node = Value::Object(Map::new()),
                PathToken::Append | PathToken::Index(_) => *node = Value::Array(Vec::new()),
            }
        }
        match t {
            PathToken::Key(k) => {
                let obj = node.as_object_mut().ok_or_else(|| {
                    anyhow::anyhow!("type conflict: key access on a JSON array at `{k}`")
                })?;
                if last {
                    obj.insert(k.clone(), value.clone());
                    return Ok(());
                }
                if !obj.contains_key(k) {
                    obj.insert(k.clone(), Value::Null);
                }
                node = obj.get_mut(k).unwrap();
            }
            PathToken::Append => {
                let arr = node.as_array_mut()
                    .ok_or_else(|| anyhow::anyhow!("type conflict: append on a JSON object"))?;
                if last {
                    arr.push(value.clone());
                    return Ok(());
                }
                arr.push(Value::Null);
                node = arr.last_mut().unwrap();
            }
            PathToken::Index(i) => {
                let arr = node.as_array_mut()
                    .ok_or_else(|| anyhow::anyhow!("type conflict: index access on a JSON object"))?;
                if arr.len() <= *i {
                    arr.resize(*i + 1, Value::Null);
                }
                if last {
                    arr[*i] = value.clone();
                    return Ok(());
                }
                node = &mut arr[*i];
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// URL / request building
// ---------------------------------------------------------------------------

fn normalize_url(raw: &str, default_scheme: &str) -> Result<String> {
    let s = raw.trim();
    if s.is_empty() {
        bail!("empty URL");
    }
    if s == ":" {
        return Ok(format!("{default_scheme}://localhost/"));
    }
    if let Some(rest) = s.strip_prefix("://") {
        return Ok(format!("{default_scheme}://{rest}"));
    }
    if let Some(rest) = s.strip_prefix(':') {
        // localhost shorthand: :3000/bar -> http://localhost:3000/bar, :/foo -> http://localhost/foo
        let port_like = rest.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false);
        if port_like {
            return Ok(format!("{default_scheme}://localhost:{rest}"));
        }
        return Ok(format!("{default_scheme}://localhost{rest}"));
    }
    if s.contains("://") {
        return Ok(s.to_string());
    }
    Ok(format!("{default_scheme}://{s}"))
}

#[derive(Default)]
struct RequestSpec {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    unset_headers: Vec<String>,
    queries: Vec<(String, String)>,
    data: Vec<(String, String, bool)>,
    uploads: Vec<(String, PathBuf, Option<String>)>,
    raw_body: Option<Vec<u8>>,
}

fn parse_positionals(args: &[String], default_scheme: &str) -> Result<RequestSpec> {
    let mut args = args.to_vec();
    let mut spec = RequestSpec::default();

    // METHOD
    if args.len() >= 2 && is_method_token(&args[0]) {
        spec.method = args.remove(0).to_ascii_uppercase();
    }

    // URL = first arg that looks like a URL or is not a valid request item.
    let mut url_idx: Option<usize> = None;
    for (i, arg) in args.iter().enumerate() {
        if looks_like_url(arg) || parse_item(arg).is_err() {
            url_idx = Some(i);
            break;
        }
    }
    if let Some(i) = url_idx {
        spec.url = normalize_url(&args.remove(i), default_scheme)?;
    } else {
        bail!("missing URL argument (e.g. `sysenv http example.org`)");
    }

    for arg in args {
        match parse_item(&arg)? {
            Item::Header { name, value, from_file } => {
                if let Some(p) = from_file {
                    let v = read_stripped(&p)?;
                    spec.headers.push((name, v));
                } else if let Some(v) = value {
                    if v.is_empty() {
                        spec.unset_headers.push(name);
                    } else {
                        spec.headers.push((name, v));
                    }
                } else {
                    spec.unset_headers.push(name);
                }
            }
            Item::Query { name, value } => spec.queries.push((name, value)),
            Item::Data { name, value, raw } => spec.data.push((name, value, raw)),
            Item::Upload { name, path, mime } => spec.uploads.push((name, path, mime)),
            Item::BodyFile(p) => {
                let bytes = std::fs::read(&p)
                    .with_context(|| format!("cannot read body file `{}`", p.display()))?;
                spec.raw_body = Some(bytes);
            }
        }
    }
    Ok(spec)
}

/// Probe stdin without hanging: spawn a reader thread and wait up to `timeout`
/// for EOF. Returns None when no data arrives in time (e.g. an interactive
/// session that merely reports a non-tty stdin, like PowerShell piping).
fn try_read_stdin(timeout: Duration) -> Option<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    match rx.recv_timeout(timeout) {
        Ok(b) if b.is_empty() => None,
        Ok(b) => Some(b),
        Err(_) => None,
    }
}

enum BodyKind {
    None,
    Bytes(Vec<u8>),
    Multipart(multipart::Form),
}

/// Build the request body; returns the body and an optional Content-Type
/// to apply when the user did not set one.
fn build_body(spec: &RequestSpec, cfg: &HttpConfig) -> Result<(BodyKind, Option<String>)> {
    // Raw body wins: --raw, then @file/stdin body.
    if let Some(raw) = &cfg.raw {
        if !spec.data.is_empty() || !spec.uploads.is_empty() {
            bail!("--raw cannot be combined with request data items");
        }
        let ct = if cfg.json {
            Some("application/json".to_string())
        } else {
            Some("text/plain; charset=utf-8".to_string())
        };
        return Ok((BodyKind::Bytes(raw.clone().into_bytes()), ct));
    }

    if let Some(bytes) = &spec.raw_body {
        if !spec.data.is_empty() || !spec.uploads.is_empty() {
            bail!("cannot mix raw request body with data items");
        }
        let ct = if cfg.json {
            Some("application/json".to_string())
        } else {
            Some("text/plain; charset=utf-8".to_string())
        };
        return Ok((BodyKind::Bytes(bytes.clone()), ct));
    }

    let has_uploads = !spec.uploads.is_empty();
    if has_uploads && !cfg.form && !cfg.multipart {
        eprintln!("sysenv: warning: file upload fields force multipart/form-data");
    }

    if has_uploads || cfg.multipart {
        let mut form = multipart::Form::new();
        for (name, value, raw) in &spec.data {
            let s = if *raw {
                match serde_json::from_str::<Value>(value) {
                    Ok(v) if v.is_string() => v.as_str().unwrap().to_string(),
                    Ok(v) if v.is_number() || v.is_boolean() => v.to_string(),
                    _ => bail!(
                        "raw JSON field `{name}` is not a scalar; not allowed in multipart/form mode"
                    ),
                }
            } else {
                value.clone()
            };
            form = form.text(name.clone(), s);
        }
        for (name, path, mime) in &spec.uploads {
            let part = multipart::Part::file(path)
                .with_context(|| format!("cannot open upload file `{}`", path.display()))?;
            let part = if let Some(m) = mime {
                part.mime_str(m).with_context(|| format!("invalid mime type `{m}`"))?
            } else {
                part
            };
            form = form.part(name.clone(), part);
        }
        return Ok((BodyKind::Multipart(form), None));
    }

    if cfg.form {
        let mut pairs: Vec<(String, String)> = Vec::new();
        for (name, value, raw) in &spec.data {
            let v = if *raw {
                match serde_json::from_str::<Value>(value) {
                    Ok(v) if v.is_string() => v.as_str().unwrap().to_string(),
                    Ok(v) if v.is_number() || v.is_boolean() => v.to_string(),
                    _ => bail!("raw JSON field `{name}` is not a scalar; not allowed with --form"),
                }
            } else {
                value.clone()
            };
            pairs.push((name.clone(), v));
        }
        if pairs.is_empty() {
            return Ok((BodyKind::None, None));
        }
        let body = urlencode_pairs(&pairs);
        return Ok((
            BodyKind::Bytes(body.into_bytes()),
            Some("application/x-www-form-urlencoded; charset=utf-8".to_string()),
        ));
    }

    // Default: JSON object (only when there is data).
    if !spec.data.is_empty() {
        let mut root = Value::Null;
        for (name, value, raw) in &spec.data {
            let tokens = parse_path(name)?;
            let v = if *raw {
                serde_json::from_str::<Value>(value)
                    .with_context(|| format!("invalid JSON value for field `{name}`: `{value}`"))?
            } else {
                Value::String(value.clone())
            };
            json_apply(&mut root, &tokens, v)?;
        }
        let body = serde_json::to_vec(&root).context("cannot serialize JSON body")?;
        return Ok((BodyKind::Bytes(body), Some("application/json".to_string())));
    }

    Ok((BodyKind::None, None))
}

fn urlencode_pairs(pairs: &[(String, String)]) -> String {
    let mut out = String::new();
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        out.push_str(&encode_component(k));
        out.push('=');
        out.push_str(&encode_component(v));
    }
    out
}

fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn add_header(map: &mut HeaderMap, name: &str, value: &str) -> Result<()> {
    let hn = HeaderName::from_bytes(name.as_bytes())
        .with_context(|| format!("invalid header name `{name}`"))?;
    let hv = HeaderValue::from_str(value)
        .with_context(|| format!("invalid header value for `{name}`"))?;
    map.append(hn, hv);
    Ok(())
}

fn header_present(map: &HeaderMap, name: &str) -> bool {
    map.keys().any(|k| k.as_str().eq_ignore_ascii_case(name))
}

// ---------------------------------------------------------------------------
// Response output
// ---------------------------------------------------------------------------

struct PrintSet {
    request_headers: bool,
    request_body: bool,
    response_headers: bool,
    response_body: bool,
    meta: bool,
}

fn parse_print_set(spec: &str) -> Result<PrintSet> {
    let mut p = PrintSet {
        request_headers: false,
        request_body: false,
        response_headers: false,
        response_body: false,
        meta: false,
    };
    for c in spec.chars() {
        match c {
            'H' => p.request_headers = true,
            'B' => p.request_body = true,
            'h' => p.response_headers = true,
            'b' => p.response_body = true,
            'm' => p.meta = true,
            _ => bail!("invalid --print choice `{c}` (valid: H, B, h, b, m)"),
        }
    }
    Ok(p)
}

fn status_line(resp: &reqwest::blocking::Response) -> String {
    let version = match resp.version() {
        reqwest::Version::HTTP_09 => "HTTP/0.9",
        reqwest::Version::HTTP_10 => "HTTP/1.0",
        reqwest::Version::HTTP_11 => "HTTP/1.1",
        reqwest::Version::HTTP_2 => "HTTP/2",
        reqwest::Version::HTTP_3 => "HTTP/3",
        _ => "HTTP/?",
    };
    format!(
        "{version} {} {}",
        resp.status().as_u16(),
        resp.status().canonical_reason().unwrap_or("")
    )
}

fn pretty_json(body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    let trimmed = text.trim_start();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return None;
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => serde_json::to_string_pretty(&v).ok(),
        Err(_) => None,
    }
}

fn guess_download_name(headers: &HeaderMap, url: &str) -> String {
    if let Some(cd) = headers.get("content-disposition") {
        if let Ok(s) = cd.to_str() {
            if let Some(pos) = s.find("filename=") {
                let rest = &s[pos + 9..];
                let name = rest.trim_matches('"').split(';').next().unwrap_or("").trim();
                if !name.is_empty() {
                    return name.to_string();
                }
            }
        }
    }
    let path = url.split('?').next().unwrap_or("");
    let last = path.rsplit('/').find(|s| !s.is_empty()).unwrap_or("");
    if !last.is_empty() {
        last.to_string()
    } else {
        "response".to_string()
    }
}

// ---------------------------------------------------------------------------
// Help & debug output
// ---------------------------------------------------------------------------

/// Print the `sysenv http --help` text (parameter descriptions + examples).
pub fn print_help() {
    println!(
        r#"sysenv http - httpie-compatible HTTP client

Usage: sysenv http [flags] [METHOD] URL [ITEM...]

位置参数 / Positional:
  METHOD       请求方法 GET/POST/PUT/DELETE/PATCH/HEAD/OPTIONS（缺省：有请求体时 POST，否则 GET）
  URL          目标地址（缺省协议为 http；localhost 简写 :3000）
  ITEM         请求项，见下表

请求项 / Request items:
  key=value       JSON 数据字段（默认 JSON；-f 时表单）
  key:=json       原始 JSON 值（数字/布尔/对象/数组）
  key==value      URL 查询参数
  key:value       请求头（key: 空值 = 取消默认头）
  key@file        multipart 文件上传（;type=mime 指定类型）
  key=@file       将文件内容作为字段值
  @file           以文件内容作为原始请求体（管道 stdin 亦可）

参数 / Flags:
  -j, --json              数据项序列化为 JSON（默认）
  -f, --form              序列化为 application/x-www-form-urlencoded
      --multipart         强制 multipart/form-data
      --raw DATA          显式原始请求体
  -p, --print WHAT        打印内容 H B h b m 任意组合（请求头/体、响应头/体、状态行）
  -h, --headers           只打印响应头
  -b, --body              只打印响应体
  -m, --meta              只打印状态行
  -v, --verbose           打印完整请求与响应
  -o, --output FILE       响应体保存到文件（其余信息打到 stderr）
  -d, --download          wget 式下载，自动猜测文件名
  -q, --quiet             静默（仅错误输出）
      --pretty MODE       none|all|colors|format 控制输出美化
  -a, --auth USER[:PASS]|TOKEN   认证凭据
  -A, --auth-type TYPE    basic（默认）或 bearer
      --proxy PROTO:URL   http/https/all 代理（可重复）
  -F, --follow            跟随 30x 重定向
      --max-redirects N   最大重定向次数（默认 30）
      --timeout SECONDS   连接超时（0 = 不限）
      --check-status      3xx/4xx/5xx 退出码 3/4/5
      --offline           只构建并打印请求，不发送
      --verify MODE       yes|no|CA文件路径 控制证书校验
  -I, --ignore-stdin      不读取 stdin
      --default-scheme S  缺省协议（默认 http）
      --debug             打印实际 HTTP 请求（方法/URL/头/体）与响应（状态/头/体）到 stderr
      --help              显示本帮助（注意：http 子命令的 -h 是“只打印响应头”）

示例 / Examples:
  sysenv http pie.dev/get
  sysenv http pie.dev/post name=John age:=29
  sysenv http -f POST pie.dev/post name='John Smith'
  sysenv http -v pie.dev/get
  sysenv http GET pie.dev/get q==httpie per_page==1
  sysenv http pie.dev/post X-API-Token:123 name=John
  sysenv http -d pie.dev/image.png
  sysenv http POST pie.dev/post @data.json
  sysenv http pie.dev/post cv@resume.pdf
  sysenv http -a user:pass pie.dev/anything
  sysenv http -A bearer -a TOKEN pie.dev/anything
  sysenv http --check-status pie.dev/404
  sysenv http --offline pie.dev/post a=1
  sysenv http --debug pie.dev/get
  echo '{{"a":1}}' | sysenv http POST pie.dev/post
"#
    );
}

/// Dump the actual request (method, URL with query, headers, body) to stderr.
fn debug_dump_request(spec: &RequestSpec, req: &reqwest::blocking::Request, body: &Option<Vec<u8>>) {
    let mut w = std::io::stderr().lock();
    let _ = writeln!(w, "# request");
    let path_and_query = match req.url().query() {
        Some(q) => format!("{}?{q}", req.url().path()),
        None => req.url().path().to_string(),
    };
    let _ = writeln!(w, "{} {path_and_query} HTTP/1.1", spec.method);
    let mut headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .map(|(n, v)| (n.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        if let Some(h) = req.url().host_str() {
            headers.push(("Host".to_string(), h.to_string()));
        }
    }
    headers.sort_by(|a, b| a.0.cmp(&b.0));
    for (n, v) in &headers {
        let _ = writeln!(w, "{n}: {v}");
    }
    if let Some(b) = body {
        if !b.is_empty() && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-length")) {
            let _ = writeln!(w, "Content-Length: {}", b.len());
        }
    }
    let _ = writeln!(w);
    if let Some(b) = body {
        if !b.is_empty() {
            let pretty = pretty_json(b).unwrap_or_else(|| String::from_utf8_lossy(b).into_owned());
            let _ = writeln!(w, "{pretty}");
            let _ = writeln!(w);
        }
    }
}

/// Dump the actual response (status, headers, body) to stderr.
fn debug_dump_response(status: reqwest::StatusCode, headers: &HeaderMap, body: &[u8]) {
    let mut w = std::io::stderr().lock();
    let _ = writeln!(w, "# response");
    let _ = writeln!(
        w,
        "HTTP {} {}",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );
    for (name, value) in headers.iter() {
        let _ = writeln!(w, "{name}: {}", value.to_str().unwrap_or(""));
    }
    let _ = writeln!(w);
    if !body.is_empty() {
        let pretty = pretty_json(body).unwrap_or_else(|| String::from_utf8_lossy(body).into_owned());
        let _ = writeln!(w, "{pretty}");
        let _ = writeln!(w);
    }
}

// ---------------------------------------------------------------------------
// Main entry
// ---------------------------------------------------------------------------

/// Run the http command. Returns the process exit code (0 on success).
pub fn run(cfg: &HttpConfig) -> Result<i32> {
    let stdout_tty = std::io::stdout().is_terminal();

    let mut spec = parse_positionals(&cfg.args, &cfg.default_scheme)?;

    // Probe piped stdin before deciding the default method, so a non-interactive
    // session with an idle stdin does not turn a GET into a POST or block.
    let stdin_bytes = if !cfg.ignore_stdin
        && !std::io::stdin().is_terminal()
        && spec.raw_body.is_none()
        && cfg.raw.is_none()
        && spec.data.is_empty()
        && spec.uploads.is_empty()
    {
        try_read_stdin(Duration::from_millis(400))
    } else {
        None
    };

    // Default method: POST when there is a body, GET otherwise.
    if spec.method.is_empty() {
        let has_body_hint = !spec.data.is_empty()
            || !spec.uploads.is_empty()
            || spec.raw_body.is_some()
            || cfg.raw.is_some()
            || stdin_bytes.is_some();
        spec.method = if has_body_hint { "POST".to_string() } else { "GET".to_string() };
    }

    if let Some(bytes) = stdin_bytes {
        if !bytes.is_empty() {
            spec.raw_body = Some(bytes);
        }
    }

    let (body_kind, body_ct) = build_body(&spec, cfg)?;
    let json_mode = cfg.json || (!cfg.form && !cfg.multipart && !spec.data.is_empty());

    // --- Build URL with query parameters.
    let mut url: reqwest::Url = spec
        .url
        .parse()
        .with_context(|| format!("invalid URL `{}`", spec.url))?;
    if !spec.queries.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in &spec.queries {
            pairs.append_pair(k, v);
        }
    }

    // --- Headers (defaults first, item headers override).
    let mut headers = HeaderMap::new();
    if !header_present(&headers, "User-Agent") {
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR));
    }
    let default_accept = if json_mode {
        "application/json, */*;q=0.5"
    } else {
        "*/*"
    };
    if !header_present(&headers, "Accept") {
        headers.insert(ACCEPT, HeaderValue::from_static(default_accept));
    }
    for (name, value) in &spec.headers {
        add_header(&mut headers, name, value)?;
    }
    for name in &spec.unset_headers {
        if let Ok(hn) = HeaderName::from_bytes(name.as_bytes()) {
            headers.remove(&hn);
        }
    }
    if let Some(ct) = &body_ct {
        if !header_present(&headers, "Content-Type") {
            headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_str(ct).with_context(|| "invalid Content-Type")?,
            );
        }
    }

    // --- Client.
    let mut builder = Client::builder();
    builder = builder.redirect(if cfg.follow {
        reqwest::redirect::Policy::limited(cfg.max_redirects.unwrap_or(30) as usize)
    } else {
        reqwest::redirect::Policy::none()
    });
    if let Some(t) = cfg.timeout {
        if t > 0.0 {
            builder = builder.timeout(Duration::from_secs_f64(t));
        }
    }
    match cfg.verify.as_deref() {
        None | Some("yes") | Some("true") => {}
        Some("no") | Some("false") => {
            builder = builder.danger_accept_invalid_certs(true);
        }
        Some(path) => {
            let pem = std::fs::read(path)
                .with_context(|| format!("cannot read CA bundle `{path}`"))?;
            let cert = reqwest::Certificate::from_pem(&pem)
                .with_context(|| format!("invalid CA bundle `{path}`"))?;
            builder = builder.add_root_certificate(cert);
        }
    }
    for p in &cfg.proxies {
        let (proto, url_str) = p
            .split_once(':')
            .with_context(|| format!("invalid --proxy `{p}` (expected PROTOCOL:URL)"))?;
        match proto {
            "http" => builder = builder.proxy(reqwest::Proxy::http(url_str)?),
            "https" => builder = builder.proxy(reqwest::Proxy::https(url_str)?),
            "all" => builder = builder.proxy(reqwest::Proxy::all(url_str)?),
            other => bail!("unsupported proxy protocol `{other}` (use http, https or all)"),
        }
    }
    let client = builder.build().context("failed to build HTTP client")?;

    let method = reqwest::Method::from_bytes(spec.method.as_bytes())
        .with_context(|| format!("invalid HTTP method `{}`", spec.method))?;

    // --- Build request.
    let mut req_builder: RequestBuilder = client.request(method, url.clone());
    req_builder = req_builder.headers(headers);
    let body_bytes: Option<Vec<u8>> = match &body_kind {
        BodyKind::None => None,
        BodyKind::Bytes(b) => {
            req_builder = req_builder.body(b.clone());
            Some(b.clone())
        }
        BodyKind::Multipart(_) => None,
    };
    if let BodyKind::Multipart(form) = body_kind {
        req_builder = req_builder.multipart(form);
    }

    if let Some(auth) = &cfg.auth {
        let at = cfg.auth_type.as_deref().unwrap_or("basic");
        match at {
            "basic" => {
                let (u, p) = if let Some((u, p)) = auth.split_once(':') {
                    (u.to_string(), Some(p.to_string()))
                } else {
                    bail!(
                        "basic auth expects USER:PASS (got `{auth}`); for token auth use -A bearer"
                    );
                };
                req_builder = req_builder.basic_auth(u, p);
            }
            "bearer" => {
                req_builder = req_builder.bearer_auth(auth);
            }
            "digest" => bail!("auth type `digest` is not supported by sysenv"),
            other => bail!("unsupported auth type `{other}` (use basic or bearer)"),
        }
    }

    let req = req_builder.build().context("failed to build HTTP request")?;

    // --debug: dump the actual request (method/URL/headers/body) to stderr.
    if cfg.debug {
        debug_dump_request(&spec, &req, &body_bytes);
    }

    // --- Determine what to print.
    let print = if cfg.offline {
        parse_print_set(cfg.print.as_deref().unwrap_or("HB"))?
    } else if let Some(p) = &cfg.print {
        parse_print_set(p)?
    } else if cfg.verbose {
        parse_print_set("BHbh")?
    } else if cfg.headers_only {
        parse_print_set("h")?
    } else if cfg.body_only {
        parse_print_set("b")?
    } else if cfg.meta_only {
        parse_print_set("m")?
    } else if cfg.output.is_some() || cfg.download {
        parse_print_set("h")?
    } else if stdout_tty {
        parse_print_set("hb")?
    } else {
        parse_print_set("b")?
    };

    let pretty_enabled = match cfg.pretty.as_deref() {
        Some("none") => false,
        Some(_) => true,
        None => stdout_tty,
    };

    let save_body_to_file = cfg.output.is_some() || cfg.download;
    let out_path: Option<PathBuf> = cfg.output.clone().or_else(|| {
        if cfg.download {
            Some(PathBuf::new()) // placeholder, filled after response
        } else {
            None
        }
    });

    // --- Print request part (verbose / offline / --print H,B).
    if (print.request_headers || print.request_body) && !cfg.quiet {
        let mut rh: Vec<(String, String)> = req
            .headers()
            .iter()
            .map(|(n, v)| (n.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        if !rh.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
            if let Some(h) = req.url().host_str() {
                rh.push(("Host".to_string(), h.to_string()));
            }
        }
        if body_bytes.is_some() && !rh.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-length")) {
            rh.push(("Content-Length".to_string(), body_bytes.as_ref().unwrap().len().to_string()));
        }
        rh.sort_by(|a, b| a.0.cmp(&b.0));

        let mut out: Box<dyn Write> = if save_body_to_file {
            Box::new(std::io::stderr())
        } else {
            Box::new(std::io::stdout())
        };
        let path_and_query = match req.url().query() {
            Some(q) => format!("{}?{q}", req.url().path()),
            None => req.url().path().to_string(),
        };
        let _ = writeln!(out, "{} {path_and_query} HTTP/1.1", spec.method);
        for (n, v) in &rh {
            let _ = writeln!(out, "{n}: {v}");
        }
        let _ = writeln!(out);
        if print.request_body {
            if let Some(b) = &body_bytes {
                let pretty = pretty_json(b).unwrap_or_else(|| String::from_utf8_lossy(b).into_owned());
                let _ = writeln!(out, "{pretty}");
                let _ = writeln!(out);
            }
        }
    }

    // --- Offline mode: done.
    if cfg.offline {
        return Ok(0);
    }

    // --- Send.
    let resp = client.execute(req).context("request failed")?;
    let status = resp.status();
    let st_line = status_line(&resp);
    let resp_headers = resp.headers().clone();

    // --debug: dump the real response (status/headers/body) to stderr and keep
    // the body for the regular output path below.
    let mut body_opt: Option<Vec<u8>> = None;
    let resp = if cfg.debug {
        let bytes = resp.bytes().context("cannot read response body")?;
        debug_dump_response(status, &resp_headers, &bytes);
        body_opt = Some(bytes.to_vec());
        None
    } else {
        Some(resp)
    };

    let out_path = if let Some(p) = out_path {
        if p.as_os_str().is_empty() {
            Some(PathBuf::from(guess_download_name(&resp_headers, spec.url.as_str())))
        } else {
            Some(p)
        }
    } else {
        None
    };

    // --- Print response status/headers.
    if !cfg.quiet && !save_body_to_file {
        let mut w = std::io::stdout().lock();
        if print.meta || print.response_headers {
            let _ = writeln!(w, "{st_line}");
        }
        if print.response_headers {
            for (name, value) in resp_headers.iter() {
                let _ = writeln!(w, "{name}: {}", value.to_str().unwrap_or(""));
            }
            let _ = writeln!(w);
        }
    } else if !cfg.quiet {
        // Saving to a file: httpie prints the rest of the exchange to stderr.
        let mut w = std::io::stderr().lock();
        if print.meta || print.response_headers {
            let _ = writeln!(w, "{st_line}");
        }
        if print.response_headers {
            for (name, value) in resp_headers.iter() {
                let _ = writeln!(w, "{name}: {}", value.to_str().unwrap_or(""));
            }
            let _ = writeln!(w);
        }
    }

    // --- Body: file or terminal.
    if let Some(path) = &out_path {
        let body: Vec<u8> = match &body_opt {
            Some(b) => b.clone(),
            None => resp
                .unwrap()
                .bytes()
                .context("cannot read response body")?
                .to_vec(),
        };
        std::fs::write(path, &body)
            .with_context(|| format!("cannot write output file `{}`", path.display()))?;
        if !cfg.quiet {
            eprintln!("Saved response body to {}", path.display());
        }
    } else if print.response_body && !cfg.quiet {
        let body_is_json = resp_headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|ct| ct.contains("json"))
            .unwrap_or(false);
        let text = match &body_opt {
            Some(b) => String::from_utf8_lossy(b).into_owned(),
            None => resp.unwrap().text().unwrap_or_default(),
        };
        let pretty = pretty_json(text.as_bytes());
        let rendered = if pretty_enabled && (body_is_json || pretty.is_some()) {
            pretty.unwrap_or(text)
        } else {
            text
        };
        let mut w = std::io::stdout().lock();
        let _ = writeln!(w, "{rendered}");
    } else if body_opt.is_none() {
        let _ = resp.unwrap().bytes(); // drain
    }

    // --- Exit code semantics.
    if cfg.check_status {
        let code = status.as_u16();
        if (300..400).contains(&code) {
            return Ok(3);
        }
        if (400..500).contains(&code) {
            return Ok(4);
        }
        if (500..600).contains(&code) {
            return Ok(5);
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_kinds() {
        assert!(matches!(
            parse_item("name=John").unwrap(),
            Item::Data { name, raw: false, .. } if name == "name"
        ));
        assert!(matches!(
            parse_item("age:=29").unwrap(),
            Item::Data { name, raw: true, .. } if name == "age"
        ));
        assert!(matches!(
            parse_item("q==x").unwrap(),
            Item::Query { name, value } if name == "q" && value == "x"
        ));
        assert!(matches!(
            parse_item("X-Key:val").unwrap(),
            Item::Header { name, value: Some(v), .. } if name == "X-Key" && v == "val"
        ));
        assert!(matches!(
            parse_item("cv@a.pdf").unwrap(),
            Item::Upload { name, .. } if name == "cv"
        ));
        assert!(matches!(
            parse_item("@body.txt").unwrap(),
            Item::BodyFile(_)
        ));
        assert!(parse_item("plain").is_err());
    }

    #[test]
    fn escaped_separator_in_key() {
        assert!(matches!(
            parse_item(r"foo\==bar").unwrap(),
            Item::Data { name, value, .. } if name == "foo=" && value == "bar"
        ));
    }

    #[test]
    fn json_path_simple() {
        let mut root = Value::Null;
        json_apply(&mut root, &parse_path("name").unwrap(), Value::String("J".into())).unwrap();
        assert_eq!(root, serde_json::json!({"name": "J"}));
    }

    #[test]
    fn json_path_nested_and_arrays() {
        let mut root = Value::Null;
        json_apply(&mut root, &parse_path("a[b]").unwrap(), Value::String("x".into())).unwrap();
        json_apply(&mut root, &parse_path("a[c][]").unwrap(), Value::String("y".into())).unwrap();
        json_apply(&mut root, &parse_path("a[d][2]").unwrap(), Value::from(3)).unwrap();
        assert_eq!(
            root,
            serde_json::json!({"a": {"b": "x", "c": ["y"], "d": [null, null, 3]}})
        );
    }

    #[test]
    fn json_top_level_array() {
        let mut root = Value::Null;
        json_apply(&mut root, &parse_path("[]").unwrap(), Value::from(1)).unwrap();
        json_apply(&mut root, &parse_path("[]").unwrap(), Value::from(2)).unwrap();
        assert_eq!(root, serde_json::json!([1, 2]));
    }

    #[test]
    fn url_shorthands() {
        assert_eq!(normalize_url("example.org", "http").unwrap(), "http://example.org");
        assert_eq!(normalize_url("://x.io", "https").unwrap(), "https://x.io");
        assert_eq!(normalize_url(":3000/bar", "http").unwrap(), "http://localhost:3000/bar");
        assert_eq!(normalize_url(":", "http").unwrap(), "http://localhost/");
        assert_eq!(normalize_url("https://a.io", "http").unwrap(), "https://a.io");
    }

    #[test]
    fn method_token() {
        assert!(is_method_token("GET"));
        assert!(is_method_token("AHOY"));
        assert!(!is_method_token("get"));
        assert!(!is_method_token("name=John"));
    }

    #[test]
    fn url_encoding() {
        assert_eq!(encode_component("a b"), "a+b");
        assert_eq!(encode_component("a&b"), "a%26b");
    }

    #[test]
    fn looks_like_url_cases() {
        assert!(looks_like_url("http://x.io"));
        assert!(looks_like_url("localhost:8000"));
        assert!(looks_like_url("pie.dev:8080/api"));
        assert!(!looks_like_url("X-API-Token:123"));
        assert!(!looks_like_url("name=John"));
        assert!(!looks_like_url("example.org")); // no scheme, no port -> not URL by heuristic
    }
}
