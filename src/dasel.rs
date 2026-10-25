//! `sys data` (alias `dasel`) — query, modify and convert structured data
//! (JSON / JSONL / YAML / TOML / CSV), a Rust port of the dasel core workflow.
//!
//! Usage:
//!   sys data -i yaml 'a.b[0].name'                     # read a value
//!   sys data -f cfg.yaml -w 'server.port = 8080'       # modify + write back
//!   sys data -i json -o yaml '{}'                      # format conversion
//!   echo '{"a":{"b":"你好"}}' | sys data 'a.b'          # read from stdin
//!   sys data --root 'a.b = "x"' < in.json              # modify, print whole doc
//!
//! JSONL (JSON Lines / NDJSON): one JSON value per line; the input parses to a
//! root array, so selectors / filters work as on a JSON array — e.g.
//! `sys data -f app.log 'level=="error"'` auto-filters the log lines, and
//! `sys data -f app.log '[*].filter(level=="error")' --limit 10` is equivalent.
//!
//! Selector syntax (dasel-compatible subset):
//!   .a.b            dotted keys (a leading `.` is optional)
//!   list[0]         array index (negative counts from the end)
//!   list[-1]        last element
//!   list[*]         every element
//!   list[:2]        Python-style slice: first 2 elements (omit = default
//!   list[1:3]       bound; negative indices count from the end)
//!   list[1:]        from element 1 to the end
//!   list[::2]       every 2nd element; [::-1] reverses; step 0 is an error
//!   m["中文名"]      quoted key (spaces / dots / CJK)
//!
//! Slices are read-only: they work in plain selectors, `|count()` pipelines
//! (`data[:2]|count()` = 2) and projection left sides
//! (`data[1:3]|{id,name}`), but not in `PATH = ...` writes or
//! filter / search / remove paths.
//!
//! Expressions:
//!   PATH            print the value at PATH
//!   PATH = VALUE    set (JSON literals or plain text; `= null` deletes)
//!   PATH = @FILE    set from a file (format by extension, else JSON / YAML,
//!                   else the file text verbatim)
//!   PATH = strenv(NAME)  set from an environment variable (string; an unset
//!                   variable is an error)
//!   PATH += VALUE / -= / *= / /=    numeric (or string append for +=)
//!   list.filter(predicate)          keep matching array elements
//!   list.remove(predicate)          delete matching array elements
//!   search(predicate)               recursively collect matching nodes
//!   name.endsWith("x")              string / key functions
//!   data[*]|{id,name}               jq-style key projection (pipes a
//!                                   selector or filter / search result onto
//!                                   the listed keys; missing keys -> null)
//!
//! On csv / table / jsonl input (root = row array), a bare method predicate
//! (`Name.contains("x")`, `has("k")`, …) auto-filters the root array, the
//! same as `[*].filter(...)`; on an object-root document it stays a scalar
//! string function.
//! Predicate language (dasel-compatible): `$this` / `$key`, field paths
//! (`name`, `a.b[0]`), literals, `== != > >= < <= && || !`, and
//! `contains("x")` / `startsWith("x")` / `endsWith("x")` / `indexOf("x")` /
//! `has("key")`. Example: `sys data 'sites.filter(type==1)'`.
//!
//! `--depth N` (default 1): delete every node deeper than N levels from the
//! whole document (root = level 0; levels 0..=N are kept), e.g.
//! `sys data --depth 1 -f cfg.yaml` keeps the top-level keys only.
//!
//! `--limit N`: keep only the first N rows when the document is an array
//! (csv / table input parses to an array of rows), e.g.
//! `sys data -i table --limit 3` prints the first 3 rows (header included).
//! Applied to the whole document before the expression.
//!
//! Files and stdin are read as bytes and decoded tolerantly: UTF-8 (BOM
//! stripped), UTF-16 (BOM), GBK, GB18030, then a lossy fallback — so legacy
//! Windows logs (e.g. GBK device logs) can be read directly.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::io::{self, Read};
use std::path::Path;


// ---------------------------------------------------------------------------
// Formats
// ---------------------------------------------------------------------------

/// Formats supported by `sys data` (superset of the removed `sys con`: + TOML + table + jsonl).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFormat {
    Json,
    Jsonl,
    Yaml,
    Toml,
    Csv,
    Md,
    Table,
}

fn parse_format(s: &str) -> Result<DataFormat> {
    match s.to_ascii_lowercase().as_str() {
        "json" => Ok(DataFormat::Json),
        "jsonl" | "ndjson" => Ok(DataFormat::Jsonl),
        "yaml" | "yml" => Ok(DataFormat::Yaml),
        "toml" => Ok(DataFormat::Toml),
        "csv" => Ok(DataFormat::Csv),
        "md" | "markdown" => Ok(DataFormat::Md),
        "table" => Ok(DataFormat::Table),
        other => bail!("unknown format `{other}` (expected: json | jsonl | yaml | toml | csv | md | table)"),
    }
}

fn format_from_ext(path: &Path) -> Option<DataFormat> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "json" => Some(DataFormat::Json),
        "jsonl" | "ndjson" => Some(DataFormat::Jsonl),
        "yaml" | "yml" => Some(DataFormat::Yaml),
        "toml" => Some(DataFormat::Toml),
        "csv" => Some(DataFormat::Csv),
        "md" | "markdown" => Some(DataFormat::Md),
        _ => None,
    }
}

/// Parse the input text into a JSON value model.
///
/// YAML is intentionally tab-tolerant: real-world configs (Windows editors,
/// generated files) frequently indent with tabs, which strict YAML rejects.
/// Tabs are normalised to 4 spaces before the strict parser, and the
/// project's tolerant block parser is used as a last-resort fallback.
pub fn parse_input(text: &str, fmt: DataFormat, table_header: Option<bool>) -> Result<Value> {
    match fmt {
        DataFormat::Json => serde_json::from_str(text).context("invalid JSON input"),
        DataFormat::Jsonl => {
            // JSON Lines: one standalone JSON value per line, collected into a
            // root array so selectors / filters work as on a JSON array.
            let mut arr = Vec::new();
            for (i, line) in text.lines().enumerate() {
                let t = line.trim();
                if t.is_empty() {
                    continue;
                }
                arr.push(serde_json::from_str(t).with_context(|| {
                    format!("invalid JSONL at line {} (each line must be one JSON value)", i + 1)
                })?);
            }
            Ok(Value::Array(arr))
        }
        DataFormat::Yaml => parse_yaml_tolerant(text),
        DataFormat::Toml => {
            toml::from_str(text).context("invalid TOML input")
        }
        DataFormat::Csv => crate::convert::csv_to_value(text, table_header),
        DataFormat::Md => crate::convert::md_to_value(text),
        DataFormat::Table => crate::convert::table_to_value(text, table_header),
    }
}

/// Tab-tolerant YAML parse: normalise tabs, strict parse, then the project's
/// fallback block parser (which itself tab-normalises and BOM-strips).
fn parse_yaml_tolerant(text: &str) -> Result<Value> {
    let normalised = normalise_yaml_tabs(text);
    if let Ok(v) = serde_yaml::from_str::<Value>(&normalised) {
        return Ok(v);
    }
    let y = crate::chat::parse_yaml(&normalised)
        .context("invalid YAML input (parsed as tab-normalised YAML)")?;
    Ok(crate::convert::yval_to_json(&y))
}

/// Replace tabs used for indentation with spaces (4 per tab). Tabs inside
/// quoted scalars are preserved by only rewriting the leading whitespace run.
pub fn normalise_yaml_tabs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for raw in text.lines() {
        let indent_len = raw.len() - raw.trim_start_matches(['\t', ' ']).len();
        let (indent, rest) = raw.split_at(indent_len.min(raw.len()));
        let mut spaces = String::new();
        for c in indent.chars() {
            match c {
                '\t' => spaces.push_str("    "),
                ' ' => spaces.push(' '),
                _ => {}
            }
        }
        out.push_str(&spaces);
        out.push_str(rest);
        out.push('\n');
    }
    // `lines()` drops a trailing newline distinction; restore it if present.
    if !text.ends_with('\n') {
        while out.ends_with('\n') {
            out.pop();
        }
    }
    out
}

/// Serialise a JSON value model into the target format.
///
/// Multi-line strings are written as JSON-escaped single-line strings before
/// YAML serialisation is *not* needed — serde_yaml emits valid block scalars
/// for embedded newlines and keeps CJK text as-is (no \uXXXX escaping), so
/// both JSON (serde_json, non-ASCII preserved) and YAML round-trip Chinese
/// content losslessly.
pub fn serialize_output(v: &Value, fmt: DataFormat, compact: bool) -> Result<String> {
    match fmt {
        DataFormat::Json => {
            if compact {
                Ok(format!("{}\n", serde_json::to_string(v)?))
            } else {
                Ok(format!("{}\n", serde_json::to_string_pretty(v)?))
            }
        }
        DataFormat::Jsonl => {
            // JSON Lines output: one compact JSON value per line (an array is
            // unwrapped into one line per element, mirroring the input).
            match v {
                Value::Array(arr) => {
                    let mut out = String::new();
                    for item in arr {
                        out.push_str(&serde_json::to_string(item)?);
                        out.push('\n');
                    }
                    Ok(out)
                }
                other => Ok(format!("{}\n", serde_json::to_string(other)?)),
            }
        }
        DataFormat::Yaml => {
            let s = serde_yaml::to_string(v).context("cannot serialize YAML")?;
            let s = s.strip_prefix("---\n").unwrap_or(&s);
            Ok(s.to_string())
        }
        DataFormat::Toml => {
            if !matches!(v, Value::Object(_)) {
                bail!("TOML output requires a top-level object (got {})", type_name(v));
            }
            let s = toml::to_string_pretty(v).context("cannot serialize TOML")?;
            Ok(s)
        }
        DataFormat::Csv => crate::convert::value_to_csv(v),
        DataFormat::Md => crate::convert::value_to_md(v),
        DataFormat::Table => crate::convert::value_to_table(v),
    }
}

// --- Screen colorization (jq-like: keys blue, scalar values green) ----------
// Applied only to stdout on a TTY; file writes (`-w`) and piped output stay
// plain. NO_COLOR disables it, matching the common convention.
const ANSI_BLUE: &str = "\x1b[34m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_RESET: &str = "\x1b[0m";

fn color_enabled() -> bool {
    let color_wanted = std::env::var("NO_COLOR")
        .map(|v| v.is_empty())
        .unwrap_or(true);
    color_wanted && std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// Colorize a rendered JSON / YAML body for the screen; other formats and
/// disabled colors return the text unchanged.
fn colorize_screen(text: &str, fmt: DataFormat) -> String {
    if !color_enabled() {
        return text.to_string();
    }
    match fmt {
        DataFormat::Json => text
            .lines()
            .map(colorize_json_line)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
        DataFormat::Yaml => text
            .lines()
            .map(colorize_yaml_line)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
        _ => text.to_string(),
    }
}

/// Span `(start, end)` of a quoted `"..."` key inside a head segment.
fn quoted_key_span(head: &str) -> Option<(usize, usize)> {
    let start = head.find('"')?;
    let end = head[start + 1..].find('"')? + start + 1;
    Some((start, end + 1))
}

/// Colorize one pretty-JSON line: `"key"` blue, scalar values green.
fn colorize_json_line(line: &str) -> String {
    let indent_len = line.chars().take_while(|c| c.is_whitespace()).count();
    let indent: String = line.chars().take(indent_len).collect();
    let rest = &line[indent_len..];

    // Object entry: `"key": <value>` (checked before the array-element branch
    // so a quoted key is not mistaken for a string item).
    if let Some(colon) = rest.find(':') {
        let (head, after_colon) = rest.split_at(colon);
        if let Some((ks, ke)) = quoted_key_span(head) {
            let (before, key, after) = (&head[..ks], &head[ks..ke], &head[ke..]);
            let value_part = after_colon[1..].trim_start();
            let (val, tail) = color_json_scalar(value_part);
            return format!("{indent}{before}{ANSI_BLUE}{key}{ANSI_RESET}{after}:{val}{tail}");
        }
    }

    // Array element: `"str",` or a bare scalar `123,` at line start.
    if let Some(open) = rest.strip_prefix('"') {
        if let Some(rel) = open.find('"') {
            let (elem, tail) = rest.split_at(rel + 2);
            return format!("{indent}{ANSI_GREEN}{elem}{ANSI_RESET}{tail}");
        }
    }
    line.to_string()
}

/// Wrap a leading scalar (string / number / true / false / null) in green;
/// structural starts (`[`, `{`) are returned untouched.
fn color_json_scalar(s: &str) -> (String, &str) {
    if let Some(open) = s.strip_prefix('"') {
        if let Some(rel) = open.find('"') {
            let (elem, tail) = s.split_at(rel + 2);
            return (format!("{ANSI_GREEN}{elem}{ANSI_RESET}"), tail);
        }
        return (String::new(), s);
    }
    let token = s.split(',').next().unwrap_or(s);
    if token.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == 'e' || c == 'E')
        || matches!(token, "true" | "false" | "null")
    {
        (
            format!("{ANSI_GREEN}{token}{ANSI_RESET}"),
            &s[token.len()..],
        )
    } else {
        (String::new(), s)
    }
}

/// Colorize one YAML line: `key:` blue, scalar value green; list item green.
fn colorize_yaml_line(line: &str) -> String {
    let indent_len = line.chars().take_while(|c| c.is_whitespace()).count();
    let indent: String = line.chars().take(indent_len).collect();
    let rest = &line[indent_len..];

    if let Some(item) = rest.strip_prefix("- ") {
        return format!("{indent}- {ANSI_GREEN}{item}{ANSI_RESET}");
    }
    if let Some(colon) = rest.find(':') {
        let (k, v) = rest.split_at(colon);
        let v = v[1..].trim_start();
        if v.is_empty() {
            return format!("{indent}{ANSI_BLUE}{k}{ANSI_RESET}:");
        }
        return format!("{indent}{ANSI_BLUE}{k}{ANSI_RESET}: {ANSI_GREEN}{v}{ANSI_RESET}");
    }
    line.to_string()
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ---------------------------------------------------------------------------
// Selector parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Step {
    /// Object key (unquoted or quoted).
    Key(String),
    /// Array index (may be negative; `*` = every element).
    Index(Option<isize>),
    /// Python-style array slice `[start:end]` / `[:end]` / `[start:]` /
    /// `[start:end:step]` — omitted bounds default like Python (negative
    /// indices count from the end), step may be negative for reverse.
    Slice {
        start: Option<isize>,
        end: Option<isize>,
        step: Option<isize>,
    },
}

/// Parse a dasel-style selector into steps: `.a.b[0]`, `m["名字"]`, `list[*]`.
fn parse_selector(sel: &str) -> Result<Vec<Step>> {
    let s = sel.trim();
    if s.is_empty() {
        bail!("empty selector");
    }
    let bytes: Vec<char> = s.chars().collect();
    let mut steps = Vec::new();
    let mut i = 0usize;
    let mut expect_segment = true;

    // Optional leading dot.
    if bytes[0] == '.' {
        i = 1;
        if i >= bytes.len() {
            bail!("selector ends with a stray `.`");
        }
    }

    while i < bytes.len() {
        if expect_segment {
            match bytes[i] {
                '.' => bail!("unexpected `.` at position {i} in `{sel}`"),
                '[' => {
                    // Root array index: `[0].key`, `[-1]`, `[*]` on a root array.
                    i = parse_brackets(&bytes, i, &mut steps, sel)?;
                }
                '"' | '\'' => {
                    let quote = bytes[i];
                    i += 1;
                    let start = i;
                    while i < bytes.len() && bytes[i] != quote {
                        i += 1;
                    }
                    if i >= bytes.len() {
                        bail!("unterminated quoted key in `{sel}`");
                    }
                    let key: String = bytes[start..i].iter().collect();
                    i += 1; // closing quote
                    steps.push(Step::Key(key));
                    // Brackets may follow directly: m["x"][0]
                    i = parse_brackets(&bytes, i, &mut steps, sel)?;
                }
                _ => {
                    let start = i;
                    while i < bytes.len() && bytes[i] != '.' && bytes[i] != '[' {
                        i += 1;
                    }
                    let key: String = bytes[start..i].iter().collect();
                    steps.push(Step::Key(key));
                    i = parse_brackets(&bytes, i, &mut steps, sel)?;
                }
            }
            expect_segment = false;
        } else {
            match bytes[i] {
                '.' => {
                    i += 1;
                    if i >= bytes.len() {
                        bail!("selector ends with a stray `.`");
                    }
                    expect_segment = true;
                }
                _ => bail!("unexpected character `{}` at position {i} in `{sel}`", bytes[i]),
            }
        }
    }
    if expect_segment {
        bail!("selector ends with a stray `.`");
    }
    Ok(steps)
}

/// Parse zero or more `[...]` groups after a key.
fn parse_brackets(
    bytes: &[char],
    mut i: usize,
    steps: &mut Vec<Step>,
    sel: &str,
) -> Result<usize> {
    while i < bytes.len() && bytes[i] == '[' {
        let start = i + 1;
        let mut j = start;
        while j < bytes.len() && bytes[j] != ']' {
            j += 1;
        }
        if j >= bytes.len() {
            bail!("unterminated `[` in `{sel}`");
        }
        let inner: String = bytes[start..j].iter().collect();
        i = j + 1;
        // Quoted key inside brackets: m["中文名"], list['a b'].
        if (inner.starts_with('"') && inner.ends_with('"') && inner.len() >= 2)
            || (inner.starts_with('\'') && inner.ends_with('\'') && inner.len() >= 2)
        {
            steps.push(Step::Key(inner[1..inner.len() - 1].to_string()));
            continue;
        }
        if inner == "*" {
            steps.push(Step::Index(None));
        } else if inner.contains(':') {
            // Python-style array slice: `[start:end]`, `[:end]`, `[start:]`,
            // `[start:end:step]` (empty part = default bound; negative
            // indices count from the end; step may be negative for reverse).
            let parts: Vec<&str> = inner.split(':').collect();
            if parts.len() > 3 {
                bail!("invalid slice `[{inner}]` in `{sel}` (at most start:end:step)");
            }
            let parse_part = |p: &str, what: &str| -> Result<Option<isize>> {
                let t = p.trim();
                if t.is_empty() {
                    return Ok(None);
                }
                t.parse()
                    .map(Some)
                    .with_context(|| format!("invalid {what} `[{inner}]` in `{sel}`"))
            };
            let start = parse_part(parts[0], "slice start")?;
            let end = if parts.len() >= 2 {
                parse_part(parts[1], "slice end")?
            } else {
                None
            };
            let step = if parts.len() >= 3 {
                parse_part(parts[2], "slice step")?
            } else {
                None
            };
            if step == Some(0) {
                bail!("slice step cannot be zero in `{sel}`");
            }
            steps.push(Step::Slice { start, end, step });
        } else {
            let n: isize = inner
                .parse()
                .with_context(|| format!("invalid array index `[{inner}]` in `{sel}`"))?;
            steps.push(Step::Index(Some(n)));
        }
    }
    Ok(i)
}

// ---------------------------------------------------------------------------
// Get
// ---------------------------------------------------------------------------

/// Python-style slice bounds, mirroring CPython `PySlice_Unpack` +
/// `PySlice_AdjustIndices`: returns `(start, stop, length)` for an array of
/// `len` elements, the given `step` and the (possibly omitted) `start` /
/// `end` bounds. Negative indices count from the end; omitted bounds default
/// to 0 / len for a positive step and len-1 / -1 for a negative step
/// (implemented with the same PY_SSIZE_T_MAX / MIN sentinels CPython uses).
fn python_slice_bounds(
    len: isize,
    start: Option<isize>,
    end: Option<isize>,
    step: isize,
) -> (isize, isize, isize) {
    let (mut s, mut e) = if step < 0 {
        (start.unwrap_or(isize::MAX), end.unwrap_or(isize::MIN))
    } else {
        (start.unwrap_or(0), end.unwrap_or(isize::MAX))
    };
    if step < 0 {
        s = if s < 0 { s + len } else { s };
        s = if s < 0 { -1 } else if s >= len { len - 1 } else { s };
        e = if e < 0 { e + len } else { e };
        e = if e < 0 { -1 } else if e >= len { len - 1 } else { e };
    } else {
        s = if s < 0 { s + len } else { s };
        s = if s < 0 { 0 } else if s >= len { len } else { s };
        e = if e < 0 { e + len } else { e };
        e = if e < 0 { 0 } else if e >= len { len } else { e };
    }
    let n = if step < 0 {
        if e < s {
            (s - e - 1) / (-step) + 1
        } else {
            0
        }
    } else if s < e {
        (e - s - 1) / step + 1
    } else {
        0
    };
    (s, e, n)
}

/// Read the value at `steps`; returns every match (wildcards may expand).
fn get_values<'a>(root: &'a Value, steps: &[Step]) -> Vec<&'a Value> {
    let mut current: Vec<&Value> = vec![root];
    for step in steps {
        let mut next = Vec::new();
        for v in current {
            match step {
                Step::Key(k) => {
                    if let Value::Object(m) = v {
                        if let Some(x) = m.get(k) {
                            next.push(x);
                        }
                    }
                }
                Step::Index(None) => {
                    if let Value::Array(a) = v {
                        next.extend(a.iter());
                    }
                }
                Step::Index(Some(n)) => {
                    if let Value::Array(a) = v {
                        let idx = if *n < 0 { a.len() as isize + *n } else { *n };
                        if idx >= 0 && (idx as usize) < a.len() {
                            next.push(&a[idx as usize]);
                        }
                    }
                }
                Step::Slice { start, end, step } => {
                    if let Value::Array(a) = v {
                        let st = step.unwrap_or(1);
                        let (_s, _e, n) = python_slice_bounds(a.len() as isize, *start, *end, st);
                        for k in 0..n.max(0) as usize {
                            let idx = _s + (k as isize) * st;
                            if idx >= 0 && (idx as usize) < a.len() {
                                next.push(&a[idx as usize]);
                            }
                        }
                    }
                }
            }
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    current
}

/// Print a fetched value: bare scalars are printed raw (dasel-style),
/// containers are serialised in the output format.
fn render_get(v: &Value, fmt: DataFormat, compact: bool) -> Result<String> {
    match v {
        Value::Null => Ok(String::new()),
        Value::String(s) if matches!(fmt, DataFormat::Json | DataFormat::Yaml | DataFormat::Table) && !s.contains('\n') => {
            // dasel prints scalar strings unquoted.
            Ok(format!("{s}\n"))
        }
        // `-o table` scalar reads (e.g. `sys data -i table '[0].PID'`) print the
        // plain JSON scalar instead of failing the table serializer.
        Value::Number(_) | Value::Bool(_) if fmt == DataFormat::Table => Ok(format!("{v}\n")),
        _ => serialize_output(v, fmt, compact),
    }
}

// ---------------------------------------------------------------------------
// Set / delete
// ---------------------------------------------------------------------------

/// Assignment operators, in precedence order (check `+=` before `=`).
const OPS: [&str; 5] = ["+=", "-=", "*=", "/=", "="];

/// Split an expression into (path, op, value).
fn split_expression(expr: &str) -> Result<(String, &'static str, String)> {
    // Find the operator outside quotes and outside nested brackets /
    // parentheses (a `=` inside `filter(...)` / `strenv(...)` arguments is
    // part of the expression, not an assignment).
    let chars: Vec<char> = expr.chars().collect();
    let mut in_quote: Option<char> = None;
    let mut depth = 0i32;
    let mut best: Option<(usize, &'static str)> = None;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match in_quote {
            Some(q) if c == q => in_quote = None,
            Some(_) => {}
            None => {
                if c == '"' || c == '\'' {
                    in_quote = Some(c);
                } else {
                    // A bare `=` that belongs to `==` / `!=` / `>=` / `<=` is a
                    // comparison operator inside a function expression
                    // (e.g. `list.filter(type==1)`), not an assignment.
                    let part_of_compare = c == '='
                        && ((i > 0 && matches!(chars[i - 1], '=' | '!' | '>' | '<'))
                            || (i + 1 < chars.len() && chars[i + 1] == '='));
                    if !part_of_compare {
                        if matches!(c, '(' | '[' | '{') {
                            depth += 1;
                        } else if matches!(c, ')' | ']' | '}') {
                            depth -= 1;
                        } else if depth == 0 {
                            for op in OPS {
                                let op_chars: Vec<char> = op.chars().collect();
                                if i + op_chars.len() <= chars.len()
                                    && chars[i..i + op_chars.len()] == op_chars[..]
                                {
                                    // Prefer the longest match found first (+= before =).
                                    best = Some((i, op));
                                    break;
                                }
                            }
                        }
                        if best.is_some() {
                            break;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    let (pos, op) = best.ok_or_else(|| {
        anyhow::anyhow!("expression needs `PATH = VALUE` or a bare selector (got: {expr})")
    })?;
    let path: String = chars[..pos].iter().collect();
    let value: String = chars[pos + op.len()..].iter().collect();
    if path.trim().is_empty() {
        bail!("missing selector before the operator in `{expr}`");
    }
    Ok((path.trim().to_string(), op, value.trim().to_string()))
}

/// Parse the right-hand side of an assignment: JSON literals first
/// (numbers / bools / null / objects / arrays / quoted strings), else raw text.
fn parse_value(raw: &str) -> Value {
    let t = raw.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return v;
    }
    Value::String(t.to_string())
}

/// `@PATH` on the right side of an assignment: load a value from a file.
/// The format comes from the file extension (json / jsonl / yaml / toml /
/// csv / md); with an unknown extension the content is tried as JSON, then
/// YAML, and undetectable text falls back to a plain string (like
/// `parse_value`'s raw-text path). Returns `None` when `raw` is not an
/// unquoted `@PATH` (so quoted `"@x"` literals are unaffected).
fn load_file_value(raw: &str, table_header: Option<bool>) -> Option<Result<Value>> {
    let t = raw.trim();
    let p = t.strip_prefix('@')?;
    if p.is_empty() {
        return Some(Err(anyhow::anyhow!(
            "`@` needs a file path (e.g. `PATH = @file.json`)"
        )));
    }
    Some((|| {
        let path = Path::new(p);
        let bytes =
            std::fs::read(path).with_context(|| format!("cannot read value file {p}"))?;
        let text = decode_text(&bytes);
        match format_from_ext(path) {
            Some(f) => parse_input(&text, f, table_header),
            None => {
                // Unknown extension: try the structured formats most useful as
                // values, then fall back to the whole text as a string.
                for f in [DataFormat::Json, DataFormat::Yaml] {
                    if let Ok(v) = parse_input(&text, f, table_header) {
                        return Ok(v);
                    }
                }
                Ok(Value::String(text.trim_end().to_string()))
            }
        }
    })())
}

/// `strenv(NAME)` on the right side of an assignment: read the environment
/// variable `NAME` and use its value as a string. An unset variable is an
/// explicit error (deterministic — never a silent empty string). Returns
/// `None` when `raw` is not a `strenv(...)` call.
fn parse_env_value(raw: &str) -> Option<Result<Value>> {
    let t = raw.trim();
    let rest = t.strip_prefix("strenv(")?;
    let name = rest.strip_suffix(")")?;
    if name.contains('(') || name.contains(')') {
        return Some(Err(anyhow::anyhow!(
            "invalid `strenv(...)` expression `{t}` (expected `strenv(NAME)`)"
        )));
    }
    let name = name.trim();
    if name.is_empty() {
        return Some(Err(anyhow::anyhow!(
            "`strenv(...)` needs an environment variable name (e.g. `strenv(ROOT)`)"
        )));
    }
    Some(match std::env::var_os(name) {
        Some(os) => match os.to_str() {
            Some(s) => Ok(Value::String(s.to_string())),
            None => Err(anyhow::anyhow!(
                "environment variable `{name}` is not valid UTF-8"
            )),
        },
        None => Err(anyhow::anyhow!(
            "environment variable `{name}` is not set"
        )),
    })
}

/// Apply `op` to the value found at `old` (may be missing => None).
fn apply_op(op: &str, old: Option<&Value>, rhs: &Value) -> Result<Value> {
    match op {
        "=" => Ok(rhs.clone()),
        "+=" => match (old, rhs) {
            (None, r) => Ok(r.clone()),
            (Some(Value::Number(a)), Value::Number(b)) => num_op(a, b, |x, y| x + y),
            (Some(Value::String(a)), Value::String(b)) => Ok(Value::String(format!("{a}{b}"))),
            (Some(Value::Null), r) => Ok(r.clone()),
            (Some(o), _) => bail!("`+=` needs two numbers or two strings (got {} and {})", type_name(o), type_name(rhs)),
        },
        "-=" => arith(op, old, rhs, |x, y| x - y),
        "*=" => arith(op, old, rhs, |x, y| x * y),
        "/=" => arith(op, old, rhs, |x, y| x / y),
        _ => bail!("unsupported operator {op}"),
    }
}

fn arith(op: &str, old: Option<&Value>, rhs: &Value, f: fn(f64, f64) -> f64) -> Result<Value> {
    match (old, rhs) {
        (Some(Value::Number(a)), Value::Number(b)) => num_op(a, b, f),
        _ => bail!("`{op}` needs numeric values"),
    }
}

fn num_op(a: &serde_json::Number, b: &serde_json::Number, f: fn(f64, f64) -> f64) -> Result<Value> {
    let (ai, bi) = (a.as_i64(), b.as_i64());
    if let (Some(x), Some(y)) = (ai, bi) {
        let r = f(x as f64, y as f64);
        if r.fract() == 0.0 && r.abs() < 9e15 {
            return Ok(Value::Number(serde_json::Number::from(r as i64)));
        }
        return num_f64(r);
    }
    let x = a.as_f64().unwrap_or(0.0);
    let y = b.as_f64().unwrap_or(0.0);
    num_f64(f(x, y))
}

fn num_f64(r: f64) -> Result<Value> {
    serde_json::Number::from_f64(r)
        .map(Value::Number)
        .context("non-finite number result")
}

/// Set the value at `steps` (in place). `delete=true` removes the target.
fn set_value(root: &mut Value, steps: &[Step], new: Value, delete: bool) -> Result<()> {
    if steps.is_empty() {
        bail!("cannot set the document root");
    }
    // Walk to the parent.
    let mut cur: &mut Value = root;
    for (i, step) in steps.iter().enumerate() {
        let last = i + 1 == steps.len();
        match step {
            Step::Key(k) => {
                if !cur.is_object() {
                    if last && cur.is_null() {
                        *cur = Value::Object(serde_json::Map::new());
                    } else if !cur.is_object() {
                        bail!("cannot index {} with key `{k}`", type_name(cur));
                    }
                }
                let m = cur.as_object_mut().unwrap();
                if last {
                    if delete {
                        m.remove(k);
                        return Ok(());
                    }
                    m.insert(k.clone(), new);
                    return Ok(());
                }
                if !m.contains_key(k) {
                    m.insert(k.clone(), Value::Object(serde_json::Map::new()));
                }
                cur = m.get_mut(k).unwrap();
            }
            Step::Index(Some(n)) => {
                if !cur.is_array() {
                    bail!("cannot index {} with `[...]`", type_name(cur));
                }
                let a = cur.as_array_mut().unwrap();
                let idx = if *n < 0 { a.len() as isize + *n } else { *n };
                let idx = idx as usize;
                if idx >= a.len() {
                    bail!("array index {idx} out of bounds (len {})", a.len());
                }
                if last {
                    if delete {
                        a.remove(idx);
                        return Ok(());
                    }
                    a[idx] = new;
                    return Ok(());
                }
                cur = &mut a[idx];
            }
            Step::Index(None) => bail!("`[*]` is only supported when reading"),
            Step::Slice { .. } => bail!("array slicing `[a:b]` is only supported when reading"),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Function expressions: filter / search / remove / string functions
// ---------------------------------------------------------------------------
//
// dasel-compatible function calls usable as a trailing expression:
//   list.filter(age >= 18)          keep matching array elements
//   list.remove(name.contains("web"))   delete matching array elements
//   search(has("name"))             recursively collect matching nodes
//   name.endsWith("x")              string / key functions on a selector
//
// jq-style key projection pipelines (this file):
//   data[*]|{id,name}               project each match onto the listed keys
//   clients.filter(name=="x")|{id}  filter result array spreads into the
//                                   projection (jq `.[]|select|{...}`)
//   data[0]|{id}                    single match emits an object, not an array
//   {"n":"Mike",id,a.b,"c":d.e}     object construction: new fields from JSON
//                                   literals (`"n":"Mike"`) or selector paths
//                                   (`"c":d.e`); bare paths keep the node
//                                   under their last segment (`id`, or lift
//                                   `a.b` -> `"b": ...`); missing -> null
//
// File-valued assignments (this file):
//   PATH = @FILE                    value loaded & parsed from FILE (extension
//                                   decides the format; else JSON / YAML /
//                                   plain text), e.g. `wallpaper=@file.json`
//   PATH = strenv(NAME)             value read from the environment variable
//                                   NAME as a string (unset -> error), e.g.
//                                   `wallpaper=strenv(ROOT)`
//
// Predicate mini-language (used by filter / search / remove):
//   $this / $key    current node / current key or index
//   field.path      a.b[0], m["中文键"]   (relative to $this)
//   literals        123 | 1.5 | "text" | 'text' | true | false | null
//   operators       == != > >= < <=  &&  ||  !
//   functions       contains("x") startsWith("x") endsWith("x")
//                   indexOf("x") has("key") has(0)

const FUNCTION_NAMES: &[&str] = &[
    "filter",
    "search",
    "remove",
    "count",
    "contains",
    "startsWith",
    "endsWith",
    "indexOf",
    "has",
];

/// A parsed trailing function call: `[path.]name(args)`.
struct FunctionCall {
    /// Selector prefix (None = document root, only valid for `search`).
    path: Option<String>,
    name: String,
    /// Raw argument text (predicate for filter/search/remove, a single
    /// expression for the string / key functions).
    args: String,
}

/// A bare comparison predicate (e.g. `Id==7360`, `age >= 18`,
/// `!has("x")`): when the expression contains a comparison operator or a
/// leading negation, csv / table documents auto-filter the root array instead
/// of treating the whole string as a literal selector key.
fn looks_like_bare_predicate(expr: &str) -> bool {
    let chars: Vec<char> = expr.chars().collect();
    let mut in_quote: Option<char> = None;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match in_quote {
            Some(q) if c == q => in_quote = None,
            Some(_) => {}
            None => {
                if c == '"' || c == '\'' {
                    in_quote = Some(c);
                } else if c == '>' || c == '<' {
                    return true;
                } else if c == '!' {
                    // `!=` or a logical-not predicate (`!has(...)`, `!(a==1)`,
                    // `!x`); a bare `!` outside quotes almost never appears in
                    // a csv/table key, so treat it as a predicate signal.
                    return true;
                } else if c == '=' && i + 1 < chars.len() && chars[i + 1] == '=' {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Parse `[path.]func(args)`. Returns `None` when the expression does not end
/// in a balanced function call; `Some(Err)` when the shape looks like a call
/// but the function name or parentheses are invalid.
fn parse_function_expr(expr: &str) -> Option<Result<FunctionCall>> {
    let s = expr.trim();
    if !s.ends_with(')') {
        return None;
    }
    // Find the `(` that opens the final balanced group, skipping quotes.
    let chars: Vec<char> = s.chars().collect();
    let mut depth = 0i32;
    let mut in_quote: Option<char> = None;
    let mut open: Option<usize> = None;
    for (i, &c) in chars.iter().enumerate().rev() {
        match in_quote {
            Some(q) if c == q => in_quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => in_quote = Some(c),
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        open = Some(i);
                        break;
                    }
                }
                _ => {}
            },
        }
    }
    let open = open?;
    let prefix: String = chars[..open].iter().collect();
    let args: String = chars[open + 1..chars.len() - 1].iter().collect();
    let (path, name) = match prefix.rfind('.') {
        Some(k) => (Some(prefix[..k].to_string()), prefix[k + 1..].to_string()),
        None => (None, prefix),
    };
    let name = name.trim().to_string();
    if name.contains('|') {
        // `path | count()` / `filter(...) | count()` style pipeline: the call
        // name region carries a top-level pipe, so let split_pipeline handle it.
        return None;
    }
    if !FUNCTION_NAMES.contains(&name.as_str()) {
        return Some(Err(anyhow::anyhow!(
            "unknown function `{name}` (expected: filter | search | remove | count | contains | startsWith | endsWith | indexOf | has)"
        )));
    }
    if matches!(name.as_str(), "filter" | "remove" | "search") && args.trim().is_empty() {
        return Some(Err(anyhow::anyhow!("`{name}` needs a predicate argument")));
    }
    Some(Ok(FunctionCall {
        path: path.filter(|p| !p.trim().is_empty()),
        name,
        args: args.trim().to_string(),
    }))
}

// ---------------------------------------------------------------------------
// jq-style pipelines: `path | {key1, key2, ...}` key projection
// ---------------------------------------------------------------------------

/// Split `left | right` at the first top-level pipe. Pipes inside quotes,
/// brackets, braces or parentheses are ignored, and `||` never splits.
fn split_pipeline(expr: &str) -> Option<(String, String)> {
    let chars: Vec<char> = expr.trim().chars().collect();
    let mut depth = 0i32;
    let mut in_quote: Option<char> = None;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match in_quote {
            Some(q) if c == q => in_quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => in_quote = Some(c),
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                '|' if depth == 0 && chars.get(i + 1) != Some(&'|') => {
                    let left: String = chars[..i].iter().collect();
                    let right: String = chars[i + 1..].iter().collect();
                    if !left.trim().is_empty() && !right.trim().is_empty() {
                        return Some((left.trim().to_string(), right.trim().to_string()));
                    }
                    // A lone `|` with an empty side is not a pipeline; keep
                    // scanning so the whole expression falls through.
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// One element of an object-construction projection `{...}`: either a
/// literal value under a new key, or the value at a selector path (bare
/// paths lift the deepest node up, named after its last path segment).
#[derive(Debug, Clone)]
enum ProjItem {
    /// `"key": <json literal>` — e.g. `"name":"Mike"`, `"n":1`, `"ok":true`,
    /// `"cfg":{"a":1}`. The literal is inserted under `key` verbatim.
    Literal { key: String, value: Value },
    /// `key` / `path` / `"key": path` — the value at `path` in each matched
    /// node. A bare path (no alias) emits the value under the path's last
    /// segment, lifting a child node to the current level
    /// (`{key2.child1}` -> `"child1": ...`); a missing path yields null.
    FromPath { key: String, path: String },
}

/// Parse an object-construction projection `{key1, "new":lit, a.b, "c":d.e}`
/// into the ordered element list. Empty slots (e.g. a trailing comma) are
/// skipped. Every element is one of:
///   - a bare path `key` / `a.b` / `list[0]` — kept / lifted under its last
///     path segment;
///   - `"key": <json literal>` — a literal value (`"Mike"`, `123`, `true`,
///     `[1,2]`, `{"a":1}`, ...) inserted under `key`;
///   - `"key": path` — the value copied from a selector path.
fn parse_projection(s: &str) -> Result<Vec<ProjItem>> {
    let s = s.trim();
    if !(s.starts_with('{') && s.ends_with('}')) || s.len() < 2 {
        bail!("projection must look like `{{k1, k2, ...}}` (got `{s}`)");
    }
    let inner = &s[1..s.len() - 1];
    let mut items = Vec::new();
    for part in split_projection_parts(inner) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        items.push(parse_proj_item(part)?);
    }
    if items.is_empty() {
        bail!("projection needs at least one key (got `{s}`)");
    }
    Ok(items)
}

/// Whether the char at `i` is escaped by an odd number of backslashes.
fn is_escaped(chars: &[char], i: usize) -> bool {
    let mut n = 0usize;
    let mut j = i;
    while j > 0 {
        j -= 1;
        if chars[j] == '\\' {
            n += 1;
        } else {
            break;
        }
    }
    n % 2 == 1
}

/// Split the `{...}` body on top-level commas, ignoring commas inside
/// quotes, brackets or nested braces (a literal value may itself be a JSON
/// object / array).
fn split_projection_parts(inner: &str) -> Vec<String> {
    let chars: Vec<char> = inner.chars().collect();
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_quote: Option<char> = None;
    let mut start = 0usize;
    for (i, &c) in chars.iter().enumerate() {
        match in_quote {
            Some(q) if c == q && !is_escaped(&chars, i) => in_quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => in_quote = Some(c),
                '[' | '{' | '(' => depth += 1,
                ']' | '}' | ')' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(chars[start..i].iter().collect());
                    start = i + 1;
                }
                _ => {}
            },
        }
    }
    parts.push(chars[start..].iter().collect());
    parts
}

/// Parse one `{...}` element: `"key": <literal|path>`, or a bare path
/// `key` / `a.b` / `list[0]` (kept / lifted under its last path segment).
fn parse_proj_item(part: &str) -> Result<ProjItem> {
    // Top-level colon: `"key": value` (a colon inside quotes / brackets does
    // not split — JSON literals like `{"a":1}` and paths with quoted keys
    // `m["a:b"]` stay intact).
    let colon = {
        let chars: Vec<char> = part.chars().collect();
        let mut in_quote: Option<char> = None;
        let mut depth = 0i32;
        let mut found = None;
        for (i, &c) in chars.iter().enumerate() {
            match in_quote {
                Some(q) if c == q && !is_escaped(&chars, i) => in_quote = None,
                Some(_) => {}
                None => match c {
                    '"' | '\'' => in_quote = Some(c),
                    '[' | '{' | '(' => depth += 1,
                    ']' | '}' | ')' => depth -= 1,
                    ':' if depth == 0 => {
                        found = Some(i);
                        break;
                    }
                    _ => {}
                },
            }
        }
        found
    };
    match colon {
        Some(i) => {
            let name = part[..i].trim();
            let rest = part[i + 1..].trim();
            let key = parse_proj_key(name)
                .with_context(|| format!("invalid projection key `{name}`"))?;
            // A full JSON literal (`"Mike"`, `123`, `true`, `[1,2]`,
            // `{"a":1}`, ...) is inserted verbatim; anything else is a
            // selector path copied from the node.
            match serde_json::from_str::<Value>(rest) {
                Ok(value) => Ok(ProjItem::Literal { key, value }),
                Err(_) => {
                    if rest.is_empty() {
                        bail!("projection alias `{part}` needs a value");
                    }
                    Ok(ProjItem::FromPath { key, path: rest.to_string() })
                }
            }
        }
        None => {
            // Bare path. A quoted JSON string (`"中文键"`) keeps that field
            // under the same name; otherwise the value is taken from the
            // selector path and emitted under its last path segment (lift).
            if part.starts_with('"') || part.starts_with('\'') {
                if let Ok(key) = serde_json::from_str::<String>(part) {
                    return Ok(ProjItem::FromPath { key: key.clone(), path: key });
                }
                if part.len() >= 2 && part.starts_with('\'') && part.ends_with('\'') {
                    let key = part[1..part.len() - 1].to_string();
                    return Ok(ProjItem::FromPath { key: key.clone(), path: key });
                }
            }
            let path = part.to_string();
            let key = projection_leaf_name(&path);
            Ok(ProjItem::FromPath { key, path })
        }
    }
}

/// The key part of an alias: either a quoted string (`"中文键"` / `'中文键'`)
/// or a bare field name (`country`, `name2`).
fn parse_proj_key(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("empty key");
    }
    if name.starts_with('"') || name.starts_with('\'') {
        if let Ok(k) = serde_json::from_str::<String>(name) {
            return Ok(k);
        }
        if name.len() >= 2 && name.starts_with('\'') && name.ends_with('\'') {
            return Ok(name[1..name.len() - 1].to_string());
        }
        bail!("key `{name}` is not a quoted string");
    }
    if name
        .chars()
        .any(|c| matches!(c, '.' | '[' | ']' | '{' | '}' | '"' | '\'' | ',' | ':' | '(' | ')'))
    {
        bail!("key `{name}` contains selector or delimiter characters (quote it: `\"{name}\"`)");
    }
    Ok(name.to_string())
}

/// The output key for a bare path: the segment after the last dot
/// (`a.b.c` -> `c`), with any trailing index groups stripped
/// (`data[0]` / `m["中文键"]` -> `data` / `m`).
fn projection_leaf_name(path: &str) -> String {
    let base = path.rsplit('.').next().unwrap_or(path).trim();
    match base.find('[') {
        Some(i) => base[..i].trim().to_string(),
        None => base.to_string(),
    }
}

/// Resolve one projection item against a node: a literal is cloned verbatim,
/// a path is looked up with the full selector machinery (first match wins; a
/// missing path yields null, a malformed path is an error).
fn resolve_proj_item(node: &Value, item: &ProjItem) -> Result<(String, Value)> {
    match item {
        ProjItem::Literal { key, value } => Ok((key.clone(), value.clone())),
        ProjItem::FromPath { key, path } => {
            let steps = parse_selector(path)
                .with_context(|| format!("invalid projection path `{path}`"))?;
            let got = get_values(node, &steps);
            let value = got.into_iter().next().cloned().unwrap_or(Value::Null);
            Ok((key.clone(), value))
        }
    }
}

/// Project each node onto the given construction items (missing paths become
/// null, jq-style); a single projected value is emitted as-is, several as an
/// array.
fn project_values(
    nodes: Vec<Value>,
    items: &[ProjItem],
    out_f: DataFormat,
    compact: bool,
) -> Result<String> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        let mut obj = serde_json::Map::new();
        for item in items {
            let (key, value) = resolve_proj_item(&node, item)?;
            obj.insert(key, value);
        }
        out.push(Value::Object(obj));
    }
    let value = if out.len() == 1 {
        out.into_iter().next().unwrap()
    } else {
        Value::Array(out)
    };
    serialize_output(&value, out_f, compact)
}

/// `path | {k1, k2}`: resolve a plain selector and project every match.
fn apply_projection_path(
    v: &Value,
    path: &str,
    items: &[ProjItem],
    out_f: DataFormat,
    compact: bool,
) -> Result<String> {
    let steps = parse_selector(path).with_context(|| format!("invalid projection path `{path}`"))?;
    let got = get_values(v, &steps);
    if got.is_empty() {
        bail!("projection path `{path}` matched nothing");
    }
    project_values(got.into_iter().cloned().collect(), items, out_f, compact)
}

/// Bare object-construction projection `{k1, "new":lit, a.b}` applied to the
/// root document: an object root is projected directly (single object out),
/// an array root element-wise (every row projected, same as `[*]|{...}`).
/// Bare paths keep / lift their last segment; missing paths become null.
fn apply_bare_projection(
    v: &Value,
    expr: &str,
    out_f: DataFormat,
    compact: bool,
) -> Result<String> {
    let items = parse_projection(expr)
        .with_context(|| format!("invalid projection `{expr}`"))?;
    let nodes: Vec<Value> = match v.as_array() {
        Some(arr) => arr.clone(),
        None => vec![v.clone()],
    };
    project_values(nodes, &items, out_f, compact)
}

/// Split a trailing key projection `PATH{k1,k2}` / `PATH.{k1,k2}` into the
/// selector path and the `{...}` part (the pipe is optional on deep nodes:
/// `sites[0]{key,name}` == `sites[0]|{key,name}`). Returns None for a bare
/// `{k1,k2}` (path empty), for a pipeline (handled by split_pipeline) or when
/// no valid trailing projection is present.
fn split_trailing_projection(expr: &str) -> Option<(String, String)> {
    let s = expr.trim();
    if !s.ends_with('}') || s.starts_with('{') {
        return None;
    }
    let chars: Vec<char> = s.chars().collect();
    let mut depth = 0i32;
    let mut in_quote: Option<char> = None;
    for (i, &c) in chars.iter().enumerate() {
        match in_quote {
            Some(q) if c == q => in_quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => in_quote = Some(c),
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                '|' if depth == 0 && chars.get(i + 1) != Some(&'|') => return None, // 管道交给 split_pipeline
                '{' if depth == 0 => {
                    let rest: String = chars[i..].iter().collect();
                    if parse_projection(&rest).is_ok() {
                        let left: String = chars[..i].iter().collect();
                        let left = left.trim();
                        // `PATH.{k1,k2}`: drop the dot right before the brace.
                        if left.ends_with('.') {
                            let cut = left[..left.len() - 1].trim_end();
                            if cut.is_empty() {
                                return None;
                            }
                            return Some((cut.to_string(), rest));
                        }
                        return Some((left.to_string(), rest));
                    }
                }
                _ => {}
            },
        }
    }
    None
}

/// `PATH{k1,k2}` / `PATH.{k1,k2}`: resolve the selector path and project
/// every match — the deep-node form of the bare root projection.
fn apply_trailing_projection(
    v: &Value,
    expr: &str,
    out_f: DataFormat,
    compact: bool,
) -> Result<String> {
    let (path, proj) = split_trailing_projection(expr)
        .with_context(|| format!("invalid projection `{expr}`"))?;
    let items = parse_projection(&proj)
        .with_context(|| format!("invalid projection `{expr}`"))?;
    apply_projection_path(v, &path, &items, out_f, compact)
}

// --- Predicate mini-language -----------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    This,
    Key,
    True,
    False,
    Null,
    Op(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    Comma,
}

fn lex_predicate(s: &str) -> Result<Vec<Tok>> {
    let chars: Vec<char> = s.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '(' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(Tok::RParen);
                i += 1;
            }
            '[' => {
                toks.push(Tok::LBracket);
                i += 1;
            }
            ']' => {
                toks.push(Tok::RBracket);
                i += 1;
            }
            '.' => {
                toks.push(Tok::Dot);
                i += 1;
            }
            ',' => {
                toks.push(Tok::Comma);
                i += 1;
            }
            '"' | '\'' => {
                let q = c;
                i += 1;
                let mut out = String::new();
                while i < chars.len() && chars[i] != q {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        out.push(chars[i + 1]);
                        i += 2;
                    } else {
                        out.push(chars[i]);
                        i += 1;
                    }
                }
                if i >= chars.len() {
                    bail!("unterminated string in predicate");
                }
                i += 1;
                toks.push(Tok::Str(out));
            }
            '$' => {
                i += 1;
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                match name.as_str() {
                    "this" => toks.push(Tok::This),
                    "key" => toks.push(Tok::Key),
                    _ => bail!("unknown $ variable `${name}` (supported: $this, $key)"),
                }
            }
            _ if c.is_ascii_digit() || (c == '-' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit()) => {
                let start = i;
                if c == '-' {
                    i += 1;
                }
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let num: String = chars[start..i].iter().collect();
                let v: f64 = num
                    .parse()
                    .with_context(|| format!("invalid number `{num}` in predicate"))?;
                toks.push(Tok::Num(v));
            }
            _ if c.is_alphabetic() || c == '_' || !c.is_ascii() => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '-' || !chars[i].is_ascii())
                {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                match word.as_str() {
                    "true" => toks.push(Tok::True),
                    "false" => toks.push(Tok::False),
                    "null" => toks.push(Tok::Null),
                    _ => toks.push(Tok::Ident(word)),
                }
            }
            _ => {
                let two: String = if i + 1 < chars.len() {
                    chars[i..i + 2].iter().collect()
                } else {
                    String::new()
                };
                match two.as_str() {
                    "==" | "!=" | ">=" | "<=" | "&&" | "||" => {
                        toks.push(Tok::Op(two));
                        i += 2;
                    }
                    _ => match c {
                        '>' | '<' | '=' | '!' | '+' | '-' => {
                            toks.push(Tok::Op(c.to_string()));
                            i += 1;
                        }
                        other => bail!("unexpected character `{other}` in predicate"),
                    },
                }
            }
        }
    }
    Ok(toks)
}

/// Evaluation context: the current node (`$this`) and its key / index (`$key`).
struct PredCtx<'a> {
    node: &'a Value,
    key: Option<&'a Value>,
}

struct PredParser<'a> {
    toks: &'a [Tok],
    pos: usize,
}

impl<'a> PredParser<'a> {
    fn peek_op(&self, op: &str) -> bool {
        matches!(self.toks.get(self.pos), Some(Tok::Op(o)) if o == op)
    }
    fn eat_tok(&mut self, t: &Tok) -> bool {
        if self.toks.get(self.pos) == Some(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn peek_tok(&self, t: &Tok) -> bool {
        self.toks.get(self.pos) == Some(t)
    }
    fn next_tok(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_expr(&mut self, ctx: &PredCtx) -> Result<Value> {
        let v = self.parse_or(ctx)?;
        if self.pos < self.toks.len() {
            bail!("unexpected trailing tokens in predicate");
        }
        Ok(v)
    }

    fn parse_or(&mut self, ctx: &PredCtx) -> Result<Value> {
        let mut v = self.parse_and(ctx)?;
        while self.peek_op("||") {
            self.pos += 1;
            let r = self.parse_and(ctx)?;
            v = Value::Bool(as_bool(&v)? || as_bool(&r)?);
        }
        Ok(v)
    }

    fn parse_and(&mut self, ctx: &PredCtx) -> Result<Value> {
        let mut v = self.parse_not(ctx)?;
        while self.peek_op("&&") {
            self.pos += 1;
            let r = self.parse_not(ctx)?;
            v = Value::Bool(as_bool(&v)? && as_bool(&r)?);
        }
        Ok(v)
    }

    fn parse_not(&mut self, ctx: &PredCtx) -> Result<Value> {
        if self.peek_op("!") {
            self.pos += 1;
            let v = self.parse_not(ctx)?;
            return Ok(Value::Bool(!as_bool(&v)?));
        }
        self.parse_cmp(ctx)
    }

    fn parse_cmp(&mut self, ctx: &PredCtx) -> Result<Value> {
        let l = self.parse_add(ctx)?;
        let cmp_op = match self.toks.get(self.pos) {
            Some(Tok::Op(o)) if matches!(o.as_str(), "==" | "!=" | ">" | ">=" | "<" | "<=" | "=") => {
                Some(o.clone())
            }
            _ => None,
        };
        if let Some(op) = cmp_op {
            self.pos += 1;
            let r = self.parse_add(ctx)?;
            return Ok(Value::Bool(compare(&op, &l, &r)));
        }
        Ok(l)
    }

    fn parse_add(&mut self, ctx: &PredCtx) -> Result<Value> {
        let mut v = self.parse_primary(ctx)?;
        loop {
            let op = match self.toks.get(self.pos) {
                Some(Tok::Op(o)) if o == "+" || o == "-" => Some(o.clone()),
                _ => None,
            };
            let Some(op) = op else { break };
            self.pos += 1;
            let r = self.parse_primary(ctx)?;
            v = match (op.as_str(), &v, &r) {
                ("+", Value::Number(a), Value::Number(b)) => num_op(a, b, |x, y| x + y)?,
                ("-", Value::Number(a), Value::Number(b)) => num_op(a, b, |x, y| x - y)?,
                (o, _, _) => bail!("`{o}` needs two numbers (or two strings for `+`)"),
            };
        }
        Ok(v)
    }

    fn parse_primary(&mut self, ctx: &PredCtx) -> Result<Value> {
        let mut cur = match self.next_tok() {
            Some(Tok::Num(n)) => num_value(n),
            Some(Tok::Str(s)) => Value::String(s),
            Some(Tok::True) => Value::Bool(true),
            Some(Tok::False) => Value::Bool(false),
            Some(Tok::Null) => Value::Null,
            Some(Tok::This) => ctx.node.clone(),
            Some(Tok::Key) => ctx.key.cloned().unwrap_or(Value::Null),
            Some(Tok::Ident(name)) => {
                if self.peek_tok(&Tok::LParen) {
                    let args = self.parse_args(ctx)?;
                    return apply_function(&name, ctx.node, &args);
                }
                lookup_key(ctx.node, &name)
            }
            Some(Tok::LParen) => {
                let v = self.parse_or(ctx)?;
                if !self.eat_tok(&Tok::RParen) {
                    bail!("missing `)` in predicate");
                }
                v
            }
            other => bail!("unexpected token in predicate (got {:?})", other),
        };
        // Path steps (`a.b[0]`) and method calls (`value.contains("x")`).
        loop {
            if self.peek_tok(&Tok::Dot) {
                let save = self.pos;
                self.pos += 1;
                match self.next_tok() {
                    Some(Tok::Ident(k)) => {
                        if self.peek_tok(&Tok::LParen) {
                            let args = self.parse_args(ctx)?;
                            cur = apply_function(&k, &cur, &args)?;
                        } else {
                            cur = lookup_key(&cur, &k);
                        }
                    }
                    _ => {
                        self.pos = save;
                        break;
                    }
                }
            } else if self.peek_tok(&Tok::LBracket) {
                self.pos += 1;
                let idx = self.parse_or(ctx)?;
                if !self.eat_tok(&Tok::RBracket) {
                    bail!("missing `]` in predicate");
                }
                cur = lookup_index(&cur, &idx);
            } else {
                break;
            }
        }
        Ok(cur)
    }

    fn parse_args(&mut self, ctx: &PredCtx) -> Result<Vec<Value>> {
        if !self.eat_tok(&Tok::LParen) {
            bail!("expected `(`");
        }
        let mut args = Vec::new();
        if self.eat_tok(&Tok::RParen) {
            return Ok(args);
        }
        loop {
            let v = self.parse_or(ctx)?;
            args.push(v);
            if self.eat_tok(&Tok::Comma) {
                continue;
            }
            if self.eat_tok(&Tok::RParen) {
                break;
            }
            bail!("expected `,` or `)` in function arguments");
        }
        Ok(args)
    }
}

fn as_bool(v: &Value) -> Result<bool> {
    match v {
        Value::Bool(b) => Ok(*b),
        other => bail!("predicate operator needs boolean operands (got {})", type_name(other)),
    }
}

fn num_value(n: f64) -> Value {
    if n.is_finite() && n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_992.0 {
        Value::Number(serde_json::Number::from(n as i64))
    } else {
        serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}

fn lookup_key(node: &Value, key: &str) -> Value {
    match node {
        Value::Object(m) => m.get(key).cloned().unwrap_or(Value::Null),
        Value::Array(a) => a
            .get(key.parse::<usize>().unwrap_or(usize::MAX))
            .cloned()
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn lookup_index(node: &Value, idx: &Value) -> Value {
    match (node, idx) {
        (Value::Array(a), Value::Number(n)) => {
            let i = n.as_i64().unwrap_or(-1);
            let len = a.len() as i64;
            let real = if i < 0 { len + i } else { i };
            if real >= 0 && real < len {
                a[real as usize].clone()
            } else {
                Value::Null
            }
        }
        (Value::Object(m), Value::String(k)) => m.get(k).cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn stringify(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// `contains` on a container checks every string leaf inside it, so
/// `sites.remove(contains("web"))` deletes elements whose content mentions
/// the value anywhere.
fn string_contains(v: &Value, needle: &str) -> bool {
    match v {
        Value::String(s) => s.contains(needle),
        Value::Object(m) => m.values().any(|c| string_contains(c, needle)),
        Value::Array(a) => a.iter().any(|c| string_contains(c, needle)),
        other => stringify(other).contains(needle),
    }
}

fn arg_as_string(args: &[Value], idx: usize, name: &str) -> Result<String> {
    let v = args
        .get(idx)
        .ok_or_else(|| anyhow::anyhow!("`{name}` needs an argument"))?;
    Ok(stringify(v))
}

fn apply_function(name: &str, value: &Value, args: &[Value]) -> Result<Value> {
    match name {
        "contains" => {
            let needle = arg_as_string(args, 0, "contains")?;
            Ok(Value::Bool(string_contains(value, &needle)))
        }
        "startsWith" => {
            let needle = arg_as_string(args, 0, "startsWith")?;
            Ok(Value::Bool(stringify(value).starts_with(&needle)))
        }
        "endsWith" => {
            let needle = arg_as_string(args, 0, "endsWith")?;
            Ok(Value::Bool(stringify(value).ends_with(&needle)))
        }
        "indexOf" => {
            let needle = arg_as_string(args, 0, "indexOf")?;
            let hay = stringify(value);
            match hay.find(&needle) {
                Some(b) => Ok(num_value(hay[..b].chars().count() as f64)),
                None => Ok(num_value(-1.0)),
            }
        }
        "has" => {
            let arg = args
                .first()
                .ok_or_else(|| anyhow::anyhow!("`has` needs a key argument"))?;
            let hit = match (value, arg) {
                (Value::Object(m), Value::String(k)) => m.contains_key(k),
                (Value::Array(a), Value::Number(n)) => {
                    let i = n.as_i64().unwrap_or(-1);
                    i >= 0 && (i as usize) < a.len()
                }
                _ => false,
            };
            Ok(Value::Bool(hit))
        }
        other => bail!("unknown function `{other}`"),
    }
}

fn compare(op: &str, l: &Value, r: &Value) -> bool {
    let eq = match (l, r) {
        (Value::Number(a), Value::Number(b)) => {
            a.as_f64().unwrap_or(f64::NAN) == b.as_f64().unwrap_or(f64::NAN)
        }
        _ => l == r,
    };
    match op {
        "==" | "=" => eq,
        "!=" => !eq,
        _ => match (l, r) {
            (Value::Number(a), Value::Number(b)) => {
                cmp_f64(op, a.as_f64().unwrap_or(0.0), b.as_f64().unwrap_or(0.0))
            }
            (Value::String(a), Value::String(b)) => cmp_ord(op, a, b),
            _ => false,
        },
    }
}

fn cmp_f64(op: &str, x: f64, y: f64) -> bool {
    match op {
        ">" => x > y,
        ">=" => x >= y,
        "<" => x < y,
        "<=" => x <= y,
        _ => false,
    }
}

fn cmp_ord(op: &str, x: &str, y: &str) -> bool {
    match op {
        ">" => x > y,
        ">=" => x >= y,
        "<" => x < y,
        "<=" => x <= y,
        _ => false,
    }
}

/// Evaluate a predicate string against one node; `key` is the current
/// array index / object key exposed as `$key`.
fn eval_predicate(node: &Value, key: Option<&Value>, pred: &str) -> Result<bool> {
    let toks = lex_predicate(pred)?;
    let mut p = PredParser { toks: &toks, pos: 0 };
    let ctx = PredCtx { node, key };
    match p.parse_expr(&ctx)? {
        Value::Bool(b) => Ok(b),
        other => bail!("predicate `{pred}` must evaluate to a boolean (got {})", type_name(&other)),
    }
}

// --- filter / search / remove ----------------------------------------------

fn resolve_single<'v>(root: &'v Value, path: &str, fn_name: &str) -> Result<&'v Value> {
    let steps = parse_selector(path).with_context(|| format!("invalid path for `{fn_name}`"))?;
    let got = get_values(root, &steps);
    match got.len() {
        1 => Ok(got[0]),
        0 => bail!("`{fn_name}` path `{path}` matched nothing"),
        _ => {
            // Root-array special case: a bare `[*]` on a root array addresses
            // the array itself, so `[*].filter(pred)` / `[*].search(pred)`
            // run over the whole document instead of erroring on the expanded
            // elements.
            if root.is_array() && steps.len() == 1 && matches!(steps[0], Step::Index(None)) {
                return Ok(root);
            }
            if steps.iter().any(|s| matches!(s, Step::Slice { .. })) {
                bail!("array slicing `[a:b]` is only supported when reading (selector / count / projection), not in a `{fn_name}` path");
            }
            bail!(
                "`{fn_name}` path `{path}` matched {} values (expected exactly one)",
                got.len()
            )
        }
    }
}

/// `path.filter(predicate)`: keep the matching array elements (read-only),
/// returned as a value so pipelines can project it (`filter(...)|{...}`).
fn apply_filter_value(v: &Value, call: &FunctionCall) -> Result<Value> {
    let path = call.path.as_deref().ok_or_else(|| {
        anyhow::anyhow!("`filter` needs an array path (e.g. `list.filter(...)`)")
    })?;
    let node = resolve_single(v, path, "filter")?;
    let arr = node
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("`filter` needs an array (got {})", type_name(node)))?;
    let mut kept = Vec::new();
    for (i, e) in arr.iter().enumerate() {
        let key = Value::Number(serde_json::Number::from(i as u64));
        if eval_predicate(e, Some(&key), &call.args)? {
            kept.push(e.clone());
        }
    }
    Ok(Value::Array(kept))
}

/// `path.filter(predicate)`: keep the matching array elements (read-only).
fn apply_filter(v: &Value, call: &FunctionCall, out_f: DataFormat, compact: bool) -> Result<String> {
    serialize_output(&apply_filter_value(v, call)?, out_f, compact)
}

/// `path.remove(predicate)`: delete the matching array elements in place
/// (each deleted element is the parent node of the content it contains).
fn apply_remove(
    v: &mut Value,
    call: &FunctionCall,
    out_f: DataFormat,
    compact: bool,
    root: bool,
) -> Result<String> {
    let path = call.path.as_deref().ok_or_else(|| {
        anyhow::anyhow!("`remove` needs an array path (e.g. `list.remove(...)`)")
    })?;
    let steps = parse_selector(path).with_context(|| format!("invalid path for `remove`"))?;
    let node = array_mut_at(v, &steps)?;
    if !node.is_array() {
        bail!("`remove` needs an array (got {})", type_name(node));
    }
    let a = node.as_array_mut().unwrap();
    let mut kept = Vec::new();
    for (i, e) in a.iter().enumerate() {
        let key = Value::Number(serde_json::Number::from(i as u64));
        if !eval_predicate(e, Some(&key), &call.args)? {
            kept.push(e.clone());
        }
    }
    *a = kept;
    if root {
        serialize_output(v, out_f, compact)
    } else {
        serialize_output(node, out_f, compact)
    }
}

/// `search(predicate)` / `path.search(predicate)`: recursively collect every
/// node matching the predicate ($this = the node, $key = its key / index),
/// returned as a value so pipelines can project it (`search(...)|{...}`).
fn apply_search_value(v: &Value, call: &FunctionCall) -> Result<Value> {
    let start = match &call.path {
        Some(p) => resolve_single(v, p, "search")?,
        None => v,
    };
    let mut found = Vec::new();
    search_collect(start, None, &call.args, &mut found)?;
    Ok(Value::Array(found))
}

/// `search(predicate)` / `path.search(predicate)`: recursively collect every
/// node matching the predicate ($this = the node, $key = its key / index).
fn apply_search(v: &Value, call: &FunctionCall, out_f: DataFormat, compact: bool) -> Result<String> {
    serialize_output(&apply_search_value(v, call)?, out_f, compact)
}

/// `count()` semantics: the size of the matched-node set. A single array
/// result counts its elements (`data|count()` = row count,
/// `filter(...)|count()` = kept count); several matches count the matches;
/// an empty set counts 0; any other single node counts 1.
fn count_matches(matched: &[&Value]) -> Value {
    let n = if matched.len() == 1 {
        match &matched[0] {
            Value::Array(a) => a.len(),
            _ => 1,
        }
    } else {
        matched.len()
    };
    num_value(n as f64)
}

/// `path.count()` / `count()`: count the nodes matched by the selector path
/// (the whole document when no path is given). Takes no arguments; an empty
/// match counts 0 instead of erroring.
fn apply_count(v: &Value, call: &FunctionCall, out_f: DataFormat, compact: bool) -> Result<String> {
    if !call.args.trim().is_empty() {
        bail!("`count` takes no arguments (got `{}`)", call.args);
    }
    let matched = match &call.path {
        Some(p) => {
            let steps = parse_selector(p).with_context(|| format!("invalid path for `count`"))?;
            get_values(v, &steps)
        }
        None => vec![v],
    };
    render_get(&count_matches(&matched), out_f, compact)
}

fn search_collect(node: &Value, key: Option<Value>, pred: &str, found: &mut Vec<Value>) -> Result<()> {
    if eval_predicate(node, key.as_ref(), pred)? {
        found.push(node.clone());
    }
    match node {
        Value::Object(m) => {
            for (k, c) in m {
                search_collect(c, Some(Value::String(k.clone())), pred, found)?;
            }
        }
        Value::Array(a) => {
            for (i, c) in a.iter().enumerate() {
                search_collect(
                    c,
                    Some(Value::Number(serde_json::Number::from(i as u64))),
                    pred,
                    found,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Standalone string / key functions on a selector: `name.endsWith("x")`,
/// `list[0].indexOf("y")`, `obj.has("key")`.
fn apply_string_fn(v: &Value, call: &FunctionCall, out_f: DataFormat, compact: bool) -> Result<String> {
    let path = call.path.as_deref().ok_or_else(|| {
        anyhow::anyhow!("`{}` needs a selector path (e.g. `name.{}())`)", call.name, call.name)
    })?;
    let node = resolve_single(v, path, &call.name)?;
    let toks = lex_predicate(&call.args)?;
    let mut p = PredParser { toks: &toks, pos: 0 };
    let ctx = PredCtx { node, key: None };
    let arg = p.parse_expr(&ctx)?;
    let val = apply_function(&call.name, node, &[arg])?;
    render_get(&val, out_f, compact)
}

/// Walk to the parent of `steps` and return a mutable reference to the value
/// at the final step (creating missing object keys along the way).
fn array_mut_at<'v>(root: &'v mut Value, steps: &[Step]) -> Result<&'v mut Value> {
    if steps.is_empty() {
        bail!("cannot use a function on the document root");
    }
    let mut cur: &mut Value = root;
    for (i, step) in steps.iter().enumerate() {
        let last = i + 1 == steps.len();
        match step {
            Step::Key(k) => {
                if !cur.is_object() {
                    if cur.is_null() {
                        *cur = Value::Object(serde_json::Map::new());
                    } else {
                        bail!("cannot index {} with key `{k}`", type_name(cur));
                    }
                }
                let m = cur.as_object_mut().unwrap();
                if last {
                    return Ok(m.entry(k.clone()).or_insert(Value::Null));
                }
                if !m.contains_key(k) {
                    m.insert(k.clone(), Value::Object(serde_json::Map::new()));
                }
                cur = m.get_mut(k).unwrap();
            }
            Step::Index(Some(n)) => {
                if !cur.is_array() {
                    bail!("cannot index {} with `[...]`", type_name(cur));
                }
                let a = cur.as_array_mut().unwrap();
                let idx = if *n < 0 { a.len() as isize + *n } else { *n };
                let idx = idx as usize;
                if idx >= a.len() {
                    bail!("array index {idx} out of bounds (len {})", a.len());
                }
                if last {
                    return Ok(&mut a[idx]);
                }
                cur = &mut a[idx];
            }
            Step::Index(None) => bail!("`[*]` is only supported when reading"),
            Step::Slice { .. } => bail!("array slicing `[a:b]` is only supported when reading"),
        }
    }
    unreachable!()
}

/// `--depth N`: delete every node deeper than N levels from the whole
/// document (root = level 0, nodes at level > N are removed entirely; the
/// kept levels remain intact).
fn prune_depth(v: &mut Value, max: usize) {
    prune_at(v, 0, max);
}

fn prune_at(v: &mut Value, level: usize, max: usize) {
    match v {
        Value::Object(m) => {
            let mut del = Vec::new();
            for (k, c) in m.iter_mut() {
                let cl = level + 1;
                if cl > max {
                    del.push(k.clone());
                } else {
                    prune_at(c, cl, max);
                }
            }
            for k in del {
                m.remove(&k);
            }
        }
        Value::Array(a) => {
            let mut kept = Vec::with_capacity(a.len());
            for c in a.iter_mut() {
                let cl = level + 1;
                if cl > max {
                    continue;
                }
                prune_at(c, cl, max);
                kept.push(c.clone());
            }
            *a = kept;
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

/// Keep only the first `limit` elements of the root array (csv / table input
/// parses to an array of rows). Non-array roots are rejected so a forgotten
/// `--limit` is reported instead of silently ignored.
pub(crate) fn apply_limit(v: &mut Value, limit: usize) -> Result<()> {
    match v {
        Value::Array(a) => {
            a.truncate(limit);
            Ok(())
        }
        other => bail!(
            "`--limit` requires an array document (use with -i csv / -i table; got {})",
            type_name(other)
        ),
    }
}

/// Decode raw input bytes into text, tolerating the legacy encodings found in
/// Windows-produced files (e.g. GBK device logs). Order:
/// 1. a UTF-8 BOM is an explicit declaration — strip it and decode as UTF-8;
/// 2. a UTF-16 LE/BE BOM selects that encoding;
/// 3. UTF-8 strict; when it succeeds it is still cross-checked against GBK —
///    legacy GBK byte pairs are often *also* valid UTF-8 (e.g. `专业` = D7 A8
///    D2 B5 decodes to Hebrew/Cyrillic), so GBK wins when it cleanly decodes
///    to real CJK while the UTF-8 view has none;
/// 4. GBK (Windows ANSI 936), then GB18030 (its superset, covers CJK
///    Extension A);
/// 5. lossy UTF-8 as a last resort (invalid bytes become U+FFFD).
fn decode_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        if let Some(s) = decode_clean(encoding_rs::UTF_16LE, &bytes[2..]) {
            return s;
        }
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        if let Some(s) = decode_clean(encoding_rs::UTF_16BE, &bytes[2..]) {
            return s;
        }
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        if !has_cjk(s) {
            if let Some(g) = decode_clean(encoding_rs::GBK, bytes) {
                if has_cjk(&g) {
                    return g;
                }
            }
        }
        return s.to_string();
    }
    if let Some(s) = decode_clean(encoding_rs::GBK, bytes) {
        return s;
    }
    if let Some(s) = decode_clean(encoding_rs::GB18030, bytes) {
        return s;
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// Decode with the given encoding; `None` when the decoder reports errors,
/// i.e. the byte stream is not a clean instance of that encoding.
pub(crate) fn decode_clean(enc: &'static encoding_rs::Encoding, bytes: &[u8]) -> Option<String> {
    let (s, _used, had_errors) = enc.decode(bytes);
    if had_errors { None } else { Some(s.into_owned()) }
}

/// Any CJK ideograph in the text (used to disambiguate GBK vs UTF-8).
pub(crate) fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF))
}

/// Run `sys data`.
#[allow(clippy::too_many_arguments)]
pub fn cmd_data(
    expr: Option<&str>,
    file: Option<&str>,
    url: Option<&str>,
    in_fmt: Option<&str>,
    out_fmt: Option<&str>,
    write_back: Option<&str>,
    compact: bool,
    root: bool,
    depth: Option<usize>,
    table_header: Option<bool>,
    limit: Option<usize>,
) -> Result<()> {
    if file.is_some() && url.is_some() {
        bail!("-f/--file and -u/--url cannot be combined");
    }
    // 1. Read the input: -f FILE, else -u URL (GET), else stdin.
    let (text, source_fmt) = match (file, url) {
        (Some(p), _) => {
            let b = std::fs::read(p).with_context(|| format!("cannot read {p}"))?;
            (decode_text(&b), format_from_ext(Path::new(p)))
        }
        (None, Some(u)) => (fetch_url(u)?, Some(DataFormat::Json)),
        (None, None) => {
            // No stdin piped and no expression -> nothing to do.
            if is_stdin_tty() && expr.is_none() {
                bail!("nothing to do: pass an expression, or pipe input via stdin / use -f FILE / -u URL");
            }
            let mut buf = Vec::new();
            io::stdin().read_to_end(&mut buf).context("cannot read stdin")?;
            (decode_text(&buf), None)
        }
    };
    if text.trim().is_empty() {
        bail!("empty input");
    }

    // 2. Input format: -i wins; else -f extension (file) / json (URL);
    // else content detection.
    let in_f = match in_fmt {
        Some(s) => parse_format(s)?,
        None => match source_fmt {
            Some(f) => f,
            None => detect_format(&text)?,
        },
    };
    let mut v = parse_input(&text, in_f, table_header)?;

    // 2.5. --limit N: keep only the first N rows of an array root (csv /
    // table input parses to an array of rows).
    if let Some(n) = limit {
        apply_limit(&mut v, n)?;
        if n == 0 {
            // head -n 0 semantics: the document is empty, so emit nothing
            // (the table / csv serialisers would reject the empty array).
            if let Some(p) = write_back {
                std::fs::write(p, "").with_context(|| format!("cannot write {p}"))?;
                eprintln!("sys data: wrote {p}");
            }
            return Ok(());
        }
    }

    // 3. Output format: -o, else input format.
    let out_f = match out_fmt {
        Some(s) => parse_format(s)?,
        None => in_f,
    };

    // 4. --depth N: delete nodes deeper than N levels from the whole document
    // (applied before the expression so every operation sees the pruned doc).
    if let Some(d) = depth {
        prune_depth(&mut v, d);
    }

    let expr = expr.map(str::trim).unwrap_or("");
    let result_string;

    if expr.is_empty() {
        // Format conversion only.
        result_string = serialize_output(&v, out_f, compact)?;
    } else if let Ok((path, op, raw)) = split_expression(expr) {
        // Assignment / delete. The right side may be a literal, `@FILE`
        // (read & parse a file) or `strenv(NAME)` (read an environment
        // variable as a string); neither `@` nor `strenv` is a delete.
        // Checked before function expressions so `PATH = strenv(X)` and
        // `PATH = foo(y)` are assignments, not unknown-function calls.
        let steps = parse_selector(&path)?;
        let file_value = load_file_value(&raw, table_header);
        let env_value = if file_value.is_none() {
            parse_env_value(&raw)
        } else {
            None
        };
        let delete = file_value.is_none()
            && env_value.is_none()
            && matches!(parse_value(&raw), Value::Null)
            && op == "=";
        if delete {
            set_value(&mut v, &steps, Value::Null, true)?;
        } else {
            let rhs = match (file_value, env_value) {
                (Some(res), _) => res?,
                (None, Some(res)) => res?,
                (None, None) => parse_value(&raw),
            };
            if op == "=" {
                set_value(&mut v, &steps, rhs, false)?;
            } else {
                let old = get_values(&v, &steps).into_iter().next().cloned();
                let new = apply_op(op, old.as_ref(), &rhs)?;
                set_value(&mut v, &steps, new, false)?;
            }
        }
        result_string = if root {
            serialize_output(&v, out_f, compact)?
        } else {
            // Match dasel v3: print the modified value at the selector
            // (empty for a delete). With -w the same string is written back.
            let got = get_values(&v, &steps);
            match got.first() {
                Some(x) => render_get(x, out_f, compact)?,
                None => serialize_output(&v, out_f, compact)?, // deleted: print the doc
            }
        };
    } else if let Some(fc) = parse_function_expr(expr) {
        // Function expression: filter / search / remove / string functions.
        let fc = fc?;
        result_string = match fc.name.as_str() {
            "count" => apply_count(&v, &fc, out_f, compact)?,
            "filter" => apply_filter(&v, &fc, out_f, compact)?,
            "search" => apply_search(&v, &fc, out_f, compact)?,
            "remove" => apply_remove(&mut v, &fc, out_f, compact, root)?,
            // 裸方法谓词（`NAME.contains("x")` / `has("k")` 等）在
            // csv / table / jsonl 的根数组上自动按 `[*].filter(...)` 过滤；
            // 对象根文档仍走标量字符串函数（apply_string_fn）。
            n if matches!(n, "contains" | "startsWith" | "endsWith" | "has")
                && v.is_array()
                && matches!(in_f, DataFormat::Csv | DataFormat::Table | DataFormat::Jsonl) =>
            {
                // 完整原表达式作为逐行谓词（不能只传 fc.args，否则丢掉
                // 字段路径，如 Name.contains('ph') 只剩字面量 'ph'）。
                let auto = FunctionCall {
                    path: Some("[*]".to_string()),
                    name: "filter".to_string(),
                    args: expr.to_string(),
                };
                apply_filter(&v, &auto, out_f, compact)?
            }
            _ => apply_string_fn(&v, &fc, out_f, compact)?,
        };
    } else if let Some((left, right)) = split_pipeline(expr) {
        if right.trim() == "count()" {
            // `path | count()` / `filter(...) | count()` / `search(...) | count()`:
            // count the nodes matched on the left, printed as a bare number.
            result_string = match parse_function_expr(&left) {
                Some(Ok(fc)) => match fc.name.as_str() {
                    "filter" => {
                        let arr = apply_filter_value(&v, &fc)?;
                        render_get(&count_matches(&[&arr]), out_f, compact)?
                    }
                    "search" => {
                        let arr = apply_search_value(&v, &fc)?;
                        render_get(&count_matches(&[&arr]), out_f, compact)?
                    }
                    other => bail!("cannot pipe `{other}(...)` into count()"),
                },
                Some(Err(e)) => return Err(e),
                None => {
                    let steps = parse_selector(&left)?;
                    let got = get_values(&v, &steps);
                    render_get(&count_matches(&got), out_f, compact)?
                }
            };
        } else {
            // jq-style key projection: `path | {k1, k2}`. The left side is a
            // plain selector or a filter / search call; the projection applies
            // to each matched node (function results spread their array, like
            // jq's `.[] | select(...) | {...}`).
            let items = parse_projection(&right)
                .with_context(|| format!("invalid projection in `{expr}`"))?;
            result_string = match parse_function_expr(&left) {
                Some(Ok(fc)) => match fc.name.as_str() {
                    "filter" => {
                        let arr = apply_filter_value(&v, &fc)?;
                        project_values(
                            arr.as_array().cloned().unwrap_or_default(),
                            &items,
                            out_f,
                            compact,
                        )?
                    }
                    "search" => {
                        let arr = apply_search_value(&v, &fc)?;
                        project_values(
                            arr.as_array().cloned().unwrap_or_default(),
                            &items,
                            out_f,
                            compact,
                        )?
                    }
                    other => bail!("cannot pipe `{other}(...)` into a projection"),
                },
                Some(Err(e)) => return Err(e),
                None => apply_projection_path(&v, &left, &items, out_f, compact)?,
            };
        }
    } else if expr.starts_with('{') && expr.ends_with('}') {
        // Bare key projection `{k1, k2}` applied to the root document:
        // an object root is projected directly (jq `. | {k1,k2}`), an array
        // root element-wise (same as `[*]|{k1,k2}`) — so
        // `cat 4k.json | sys data '{wallpaper,warningText}'` keeps only those
        // root keys, and `ps | sys data -i table '{Id,Name}'` projects every
        // row. Key list validation is shared with `path | {k1,k2}`.
        result_string = apply_bare_projection(&v, expr, out_f, compact)?;
    } else if split_trailing_projection(expr).is_some() {
        // Trailing key projection on a deep node: `sites[0]{key,name}` /
        // `sites[0].{key,name}` == `sites[0]|{key,name}` — the deep-node
        // form of the bare root projection (arrays project every row).
        result_string = apply_trailing_projection(&v, expr, out_f, compact)?;
    } else if looks_like_bare_predicate(expr)
        && matches!(in_f, DataFormat::Csv | DataFormat::Table | DataFormat::Jsonl)
    {
        // Bare comparison predicate on csv / table / jsonl input: auto-filter
        // the root array (e.g. `sys data -i table 'Id==7360'` or
        // `sys data -f x.jsonl 'level=="error"'`).
        let fc = FunctionCall {
            path: Some("[*]".to_string()),
            name: "filter".to_string(),
            args: expr.to_string(),
        };
        result_string = apply_filter(&v, &fc, out_f, compact)?;
    } else {
        // Plain selector.
        let steps = parse_selector(expr)?;
        let got = get_values(&v, &steps);
        if got.is_empty() {
            bail!("selector `{expr}` matched nothing");
        }
        let mut s = String::new();
        for x in got {
            s.push_str(&render_get(x, out_f, compact)?);
        }
        result_string = s;
    }

    // 4. Emit: with -w the full document goes to the file; stdout gets the
    // value view (or the full doc with --root).
    if let Some(p) = write_back {
        let doc = if root {
            result_string.clone()
        } else {
            serialize_output(&v, out_f, compact)?
        };
        std::fs::write(p, doc).with_context(|| format!("cannot write {p}"))?;
        eprintln!("sys data: wrote {p}");
        if !root {
            use std::io::Write;
            let mut h = io::stdout().lock();
            h.write_all(colorize_screen(&result_string, out_f).as_bytes())?;
            h.flush()?;
        }
    } else {
        use std::io::Write;
        let mut h = io::stdout().lock();
        h.write_all(colorize_screen(&result_string, out_f).as_bytes())?;
        h.flush()?;
    }
    Ok(())
}

/// Heuristic YAML structure check: at least one `key:` / `- item` line.
fn looks_like_yaml(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("- ") || t.split_once(':').map(|(_, r)| r.is_empty() || r.starts_with(' ')).unwrap_or(false)
    })
}

/// GET the given URL and return its body as text (default input format: JSON).
fn fetch_url(url: &str) -> Result<String> {
    let url = if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("https://{url}")
    };
    let resp = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("cannot build HTTP client")?
        .get(&url)
        .header("User-Agent", "sys-data/0.5.8")
        .header("Accept", "application/json, text/*;q=0.9, */*;q=0.8")
        .send()
        .with_context(|| format!("request failed: {url}"))?;
    let status = resp.status();
    let body = resp.text().context("cannot read the response body")?;
    if !status.is_success() {
        bail!("HTTP {status} from {url}: {}", truncate_for_msg(&body));
    }
    Ok(body)
}

fn truncate_for_msg(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() > 200 {
        let cut: String = t.chars().take(200).collect();
        format!("{cut}…")
    } else {
        t.to_string()
    }
}

/// Best-effort stdin-tty check without libstd's `is_terminal` (MSRV-safe).
fn is_stdin_tty() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::isatty(0) == 1 }
    }
    #[cfg(not(unix))]
    {
        // Windows console handle check via GetStdHandle/GetConsoleFileType fallback:
        // a piped stdin has no console handle. Use kernel32 GetConsoleMode via
        // windows-sys with the Console feature missing on some builds -> fallback
        // to detecting an invalid handle through GetStdHandle semantics.
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetStdHandle(nStdHandle: u32) -> *mut core::ffi::c_void;
            fn GetConsoleMode(hConsoleHandle: *mut core::ffi::c_void, lpMode: *mut u32) -> i32;
        }
        unsafe {
            let h = GetStdHandle(/* STD_INPUT_HANDLE */ 0xFFFF_FFF6u32 as u32);
            if h.is_null() || h == INVALID_HANDLE_VALUE as *mut core::ffi::c_void {
                return false;
            }
            let mut mode = 0u32;
            GetConsoleMode(h, &mut mode) != 0
        }
    }
}

/// Best-effort format detection (JSON / JSONL / YAML first, then CSV / MD).
fn detect_format(text: &str) -> Result<DataFormat> {
    let t = text.trim_start();
    if t.starts_with('{') || t.starts_with('[') {
        if serde_json::from_str::<Value>(text).is_ok() {
            return Ok(DataFormat::Json);
        }
        // JSON Lines: the whole text is not one JSON value, but every line is
        // (pretty-printed JSON already succeeded above).
        if lines_are_jsonl(text) {
            return Ok(DataFormat::Jsonl);
        }
        if parse_yaml_tolerant(text).is_ok() {
            return Ok(DataFormat::Yaml);
        }
        bail!("input starts like JSON/YAML but cannot be parsed (use -i json|jsonl|yaml|toml|table)");
    }
    if toml::from_str::<toml::Value>(text).is_ok() && t.lines().any(|l| l.contains('=')) {
        return Ok(DataFormat::Toml);
    }
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    // CSV / MD before YAML: `a,b` parses as a YAML scalar, so a comma in the
    // first line (with a data-like second line) is more likely CSV.
    if lines.len() >= 2 && lines[0].starts_with('|') {
        let sep: String = lines[1].chars().filter(|&c| c != '|' && c != ' ').collect();
        if !sep.is_empty() && sep.chars().all(|c| c == '-' || c == ':') {
            return Ok(DataFormat::Md);
        }
    }
    // Block YAML (`key:` / `- item`) that parses must win over the comma-CSV
    // heuristic: a `model: a,b,c` selector line or values with commas must not
    // be misread as CSV. Plain comma tables have no `key:`/`-` structure and
    // still fall through to CSV below.
    if looks_like_yaml(&text) && parse_yaml_tolerant(text).is_ok() {
        return Ok(DataFormat::Yaml);
    }
    if lines.first().map(|l| l.contains(',')).unwrap_or(false) && lines.len() >= 2 {
        return Ok(DataFormat::Csv);
    }
    if lines.len() > 1 {
        return Ok(DataFormat::Csv);
    }
    bail!("cannot detect the input format (use -i json|jsonl|yaml|toml|csv|md|table)")
}

/// True when every non-blank line of `text` is a standalone JSON value
/// (JSON Lines / NDJSON).
fn lines_are_jsonl(text: &str) -> bool {
    let mut n = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if serde_json::from_str::<Value>(line.trim()).is_err() {
            return false;
        }
        n += 1;
    }
    n > 0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The output keys of a parsed projection, in order.
    fn proj_keys(items: &[ProjItem]) -> Vec<&str> {
        items
            .iter()
            .map(|it| match it {
                ProjItem::Literal { key, .. } | ProjItem::FromPath { key, .. } => key.as_str(),
            })
            .collect()
    }

    #[test]
    fn selector_parsing() {
        assert_eq!(
            parse_selector("a.b.c").unwrap(),
            vec![Step::Key("a".into()), Step::Key("b".into()), Step::Key("c".into())]
        );
        assert_eq!(
            parse_selector(".a[0]").unwrap(),
            vec![Step::Key("a".into()), Step::Index(Some(0))]
        );
        assert_eq!(
            parse_selector("list[-1]").unwrap(),
            vec![Step::Key("list".into()), Step::Index(Some(-1))]
        );
        assert_eq!(
            parse_selector("list[*]").unwrap(),
            vec![Step::Key("list".into()), Step::Index(None)]
        );
        // Quoted keys with dots / CJK / spaces.
        assert_eq!(
            parse_selector(r#"m["中文名.字"]"#).unwrap(),
            vec![Step::Key("m".into()), Step::Key("中文名.字".into())]
        );
        assert_eq!(
            parse_selector(r#"m["a b"].c"#).unwrap(),
            vec![Step::Key("m".into()), Step::Key("a b".into()), Step::Key("c".into())]
        );
        assert_eq!(
            parse_selector("a[0][1]").unwrap(),
            vec![Step::Key("a".into()), Step::Index(Some(0)), Step::Index(Some(1))]
        );
        assert!(parse_selector("a..b").is_err());
        assert!(parse_selector("a.").is_err());
        // 根数组索引自 v0.5.12 起合法（[0] / [-1] / [*]）。
        assert_eq!(parse_selector("[0]").unwrap(), vec![Step::Index(Some(0))]);
        assert!(parse_selector("a[x]").is_err());
    }

    #[test]
    fn slice_parsing() {
        // Python-style slices: [start:end], [:end], [start:], [start:end:step]
        assert_eq!(
            parse_selector("list[:2]").unwrap(),
            vec![Step::Key("list".into()), Step::Slice { start: None, end: Some(2), step: None }]
        );
        assert_eq!(
            parse_selector("list[1:3]").unwrap(),
            vec![Step::Key("list".into()), Step::Slice { start: Some(1), end: Some(3), step: None }]
        );
        assert_eq!(
            parse_selector("list[1:]").unwrap(),
            vec![Step::Key("list".into()), Step::Slice { start: Some(1), end: None, step: None }]
        );
        assert_eq!(
            parse_selector("list[::2]").unwrap(),
            vec![Step::Key("list".into()), Step::Slice { start: None, end: None, step: Some(2) }]
        );
        assert_eq!(
            parse_selector("list[::-1]").unwrap(),
            vec![Step::Key("list".into()), Step::Slice { start: None, end: None, step: Some(-1) }]
        );
        assert_eq!(
            parse_selector("list[-2:]").unwrap(),
            vec![Step::Key("list".into()), Step::Slice { start: Some(-2), end: None, step: None }]
        );
        // 根数组切片
        assert_eq!(
            parse_selector("[:3]").unwrap(),
            vec![Step::Slice { start: None, end: Some(3), step: None }]
        );
        // 冒号键仍走引号键（m["a:b"] 不是切片）
        assert_eq!(
            parse_selector(r#"m["a:b"]"#).unwrap(),
            vec![Step::Key("m".into()), Step::Key("a:b".into())]
        );
        // 非法切片
        assert!(parse_selector("list[::0]").is_err()); // step 0
        assert!(parse_selector("list[a:2]").is_err()); // 非数字
        assert!(parse_selector("list[1:2:3:4]").is_err()); // 超 3 段
    }

    #[test]
    fn slice_get() {
        let v = json!({"list": [0, 1, 2, 3, 4, 5, 6, 7, 8]});
        let nums = |sel: &str| -> Vec<i64> {
            get_values(&v, &parse_selector(sel).unwrap())
                .into_iter()
                .map(|x| x.as_i64().unwrap())
                .collect()
        };
        assert_eq!(nums("list[:2]"), vec![0, 1]);
        assert_eq!(nums("list[1:3]"), vec![1, 2]);
        assert_eq!(nums("list[1:]"), vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(nums("list[-2:]"), vec![7, 8]);
        assert_eq!(nums("list[:-1]"), vec![0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(nums("list[-3:-1]"), vec![6, 7]);
        assert_eq!(nums("list[::2]"), vec![0, 2, 4, 6, 8]);
        assert_eq!(nums("list[1::2]"), vec![1, 3, 5, 7]);
        assert_eq!(nums("list[::-1]"), vec![8, 7, 6, 5, 4, 3, 2, 1, 0]);
        assert_eq!(nums("list[7:2:-1]"), vec![7, 6, 5, 4, 3]);
        assert_eq!(nums("list[:]"), vec![0, 1, 2, 3, 4, 5, 6, 7, 8]);
        // 越界截断 / 空区间
        assert_eq!(nums("list[0:99]"), vec![0, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(nums("list[5:2]"), Vec::<i64>::new());
        assert_eq!(nums("list[5:2:-1]"), vec![5, 4, 3]);
        // 组合：切片后再索引
        let nested = json!({"data": [[1, 2, 3], [4, 5, 6], [7, 8, 9]]});
        let got = get_values(&nested, &parse_selector("data[1:3][0]").unwrap());
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].as_i64().unwrap(), 4);
        assert_eq!(got[1].as_i64().unwrap(), 7);
        // 非数组节点上切片不匹配（静默跳过，与 [*] 一致）
        let obj = json!({"a": {"b": 1}});
        assert!(get_values(&obj, &parse_selector("a[0:1]").unwrap()).is_empty());
        // 空数组切片 → 空
        let empty = json!({"list": []});
        assert!(get_values(&empty, &parse_selector("list[:2]").unwrap()).is_empty());
    }

    #[test]
    fn slice_rejected_on_write_and_functions() {
        let mut v = json!({"list": [1, 2, 3]});
        // 写入路径拒绝切片（同 [*] 只读）
        let err = set_value(
            &mut v,
            &parse_selector("list[:2]").unwrap(),
            json!([9]),
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("only supported when reading"), "{err}");
        // filter / search path 拒绝切片
        let fc = FunctionCall {
            path: Some("list[:2]".to_string()),
            name: "filter".to_string(),
            args: "true".to_string(),
        };
        let err = apply_filter_value(&v, &fc).unwrap_err();
        assert!(err.to_string().contains("only supported when reading"), "{err}");
        // 切片 + count 管道语义：data[:2]|count() = 2
        let doc = json!({"data": [1, 2, 3]});
        let matched = get_values(&doc, &parse_selector("data[:2]").unwrap());
        assert_eq!(count_matches(&matched), json!(2));
        let matched = get_values(&doc, &parse_selector("data[::2]").unwrap());
        assert_eq!(count_matches(&matched), json!(2));
    }

    #[test]
    fn expression_splitting() {
        assert_eq!(
            split_expression("a.b = 1").unwrap(),
            ("a.b".into(), "=", "1".into())
        );
        assert_eq!(
            split_expression("a.b += \"x y\"").unwrap(),
            ("a.b".into(), "+=", "\"x y\"".into())
        );
        // `=` inside quotes must not split.
        assert_eq!(
            split_expression(r#"a["x=y"] = 2"#).unwrap(),
            (r#"a["x=y"]"#.into(), "=", "2".into())
        );
        // Operators inside nested brackets / parens are not assignments:
        // `PATH = strenv(X)` / `PATH = foo(y)` split at the depth-0 `=`.
        assert_eq!(
            split_expression("wallpaper=strenv(ROOT)").unwrap(),
            ("wallpaper".into(), "=", "strenv(ROOT)".into())
        );
        assert_eq!(
            split_expression("a.b = foo(y)").unwrap(),
            ("a.b".into(), "=", "foo(y)".into())
        );
        assert!(split_expression("list.filter(a = b)").is_err()); // predicate, not assignment
        assert!(split_expression("a.b").is_err()); // no operator -> selector
        assert!(split_expression(" = 1").is_err());
    }

    #[test]
    fn value_literals() {
        assert_eq!(parse_value("1"), json!(1));
        assert_eq!(parse_value("1.5"), json!(1.5));
        assert_eq!(parse_value("true"), json!(true));
        assert_eq!(parse_value("null"), Value::Null);
        assert_eq!(parse_value(r#""你好""#), json!("你好"));
        assert_eq!(parse_value("[1,2]"), json!([1, 2]));
        assert_eq!(parse_value("hello"), json!("hello"));
        assert_eq!(parse_value("中文值"), json!("中文值"));
    }

    #[test]
    fn tab_yaml_parsing() {
        // Tabs as indentation: strict YAML rejects this; we must normalise.
        let v = parse_input("a:\n\tb: 1\n\tc: 中文\n", DataFormat::Yaml, None).unwrap();
        assert_eq!(v, json!({"a": {"b": 1, "c": "中文"}}));
        // Mixed tabs + spaces.
        let v2 = parse_input("a:\n  b:\n\t\tc: 值\n", DataFormat::Yaml, None).unwrap();
        assert_eq!(v2, json!({"a": {"b": {"c": "值"}}}));
        // Normal list style keeps working.
        let v3 = parse_input("list:\n  - 苹果\n  - 香蕉\n", DataFormat::Yaml, None).unwrap();
        assert_eq!(v3, json!({"list": ["苹果", "香蕉"]}));
    }

    #[test]
    fn normalise_tabs_preserves_inline_tabs() {
        let s = normalise_yaml_tabs("a:\n\tb: \"x\ty\"\n");
        assert!(s.starts_with("a:\n    b: \"x\ty\"\n"), "got: {s:?}");
    }

    #[test]
    fn json_line_colorization_keys_and_values() {
        // 对象条目：key 蓝，字符串 / 数字值绿
        let out = colorize_json_line("  \"name\": \"web-server\",");
        assert!(out.contains(&format!("{ANSI_BLUE}\"name\"{ANSI_RESET}")));
        assert!(out.contains(&format!("{ANSI_GREEN}\"web-server\"{ANSI_RESET}")));
        let out = colorize_json_line("  \"type\": 1,");
        assert!(out.contains(&format!("{ANSI_GREEN}1{ANSI_RESET}")));
        // 数组元素字符串绿
        let out = colorize_json_line("    \"a\",");
        assert!(out.starts_with("    "));
        assert!(out.contains(&format!("{ANSI_GREEN}\"a\"{ANSI_RESET}")));
        // 结构符行不变
        assert_eq!(colorize_json_line("["), "[");
    }

    #[test]
    fn yaml_line_colorization() {
        let out = colorize_yaml_line("name: web-server");
        assert!(out.contains(&format!("{ANSI_BLUE}name{ANSI_RESET}")));
        assert!(out.contains(&format!("{ANSI_GREEN}web-server{ANSI_RESET}")));
        let out = colorize_yaml_line("tags:");
        assert!(out.contains(&format!("{ANSI_BLUE}tags{ANSI_RESET}:")));
        let out = colorize_yaml_line("  - apple");
        assert!(out.contains(&format!("{ANSI_GREEN}apple{ANSI_RESET}")));
    }

    #[test]
    fn chinese_roundtrip_json_yaml() {
        let src = r#"{"名称":"中文测试","嵌套":{"值":"你好世界"},"列表":["苹果","香蕉"]}"#;
        let v: Value = serde_json::from_str(src).unwrap();
        // JSON -> YAML -> JSON keeps CJK text intact.
        let y = serialize_output(&v, DataFormat::Yaml, false).unwrap();
        assert!(y.contains("中文测试"), "yaml: {y}");
        let back = parse_input(&y, DataFormat::Yaml, None).unwrap();
        assert_eq!(v, back);
        // YAML -> JSON keeps CJK text intact.
        let j = serialize_output(&back, DataFormat::Json, false).unwrap();
        assert!(j.contains("中文测试"), "json: {j}");
    }

    #[test]
    fn yaml_multiline_chinese_survives() {
        let src = "a:\n  b: |\n    第一行\n    第二行\n";
        let v = parse_input(src, DataFormat::Yaml, None).unwrap();
        // `|` keeps the trailing newline (YAML standard).
        assert_eq!(v["a"]["b"], json!("第一行\n第二行\n"));
        let y = serialize_output(&v, DataFormat::Yaml, false).unwrap();
        let back = parse_input(&y, DataFormat::Yaml, None).unwrap();
        assert_eq!(back["a"]["b"], json!("第一行\n第二行\n"));
        // `|-` strips it.
        let src2 = "a:\n  b: |-\n    第一行\n    第二行\n";
        let v2 = parse_input(src2, DataFormat::Yaml, None).unwrap();
        assert_eq!(v2["a"]["b"], json!("第一行\n第二行"));
    }

    #[test]
    fn toml_roundtrip_chinese() {
        let src = "name = \"中文\"\n[server]\nport = 8080\n";
        let v = parse_input(src, DataFormat::Toml, None).unwrap();
        assert_eq!(v["name"], json!("中文"));
        let out = serialize_output(&v, DataFormat::Toml, false).unwrap();
        let back = parse_input(&out, DataFormat::Toml, None).unwrap();
        assert_eq!(back["name"], json!("中文"));
        assert_eq!(back["server"]["port"], json!(8080));
    }

    #[test]
    fn get_and_set() {
        let mut v = json!({"a": {"b": [1, 2, 3]}, "名": "值"});
        let got = get_values(&v, &parse_selector("a.b[1]").unwrap());
        assert_eq!(got[0], &json!(2));
        let got = get_values(&v, &parse_selector("名").unwrap());
        assert_eq!(got[0], &json!("值"));
        set_value(&mut v, &parse_selector("a.b[1]").unwrap(), json!(9), false).unwrap();
        assert_eq!(v["a"]["b"][1], json!(9));
        set_value(&mut v, &parse_selector("a.c").unwrap(), json!("新"), false).unwrap();
        assert_eq!(v["a"]["c"], json!("新"));
        set_value(&mut v, &parse_selector("a.b[-1]").unwrap(), Value::Null, true).unwrap();
        assert_eq!(v["a"]["b"], json!([1, 9]));
    }

    #[test]
    fn wildcard_get() {
        let v = json!({"list": [{"n": "甲"}, {"n": "乙"}]});
        let got = get_values(&v, &parse_selector("list[*].n").unwrap());
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], &json!("甲"));
    }

    #[test]
    fn ops() {
        assert_eq!(apply_op("+=", Some(&json!(1)), &json!(2)).unwrap(), json!(3));
        assert_eq!(apply_op("+=", Some(&json!("a")), &json!("b")).unwrap(), json!("ab"));
        assert_eq!(apply_op("+=", None, &json!(5)).unwrap(), json!(5));
        assert_eq!(apply_op("-=", Some(&json!(5)), &json!(2)).unwrap(), json!(3));
        assert_eq!(apply_op("*=", Some(&json!(3)), &json!(3)).unwrap(), json!(9));
        assert_eq!(apply_op("/=", Some(&json!(7)), &json!(2)).unwrap(), json!(3.5));
    }

    #[test]
    fn expression_delete_via_null() {
        let mut v = json!({"a": 1, "b": {"c": 2}});
        let (p, op, raw) = split_expression("a = null").unwrap();
        set_value(&mut v, &parse_selector(&p).unwrap(), Value::Null, op == "=" && parse_value(&raw) == Value::Null).unwrap();
        assert_eq!(v, json!({"b": {"c": 2}}));
    }

    #[test]
    fn csv_get_chinese() {
        let v = parse_input("名称,数量\n苹果,3\n香蕉,5\n", DataFormat::Csv, None).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0]["名称"], json!("苹果"));
        let out = serialize_output(&v, DataFormat::Md, false).unwrap();
        assert!(out.contains("苹果"));
    }

    #[test]
    fn format_detection() {
        assert_eq!(detect_format("{\"a\":1}").unwrap(), DataFormat::Json);
        assert_eq!(detect_format("a:\n  b: 1\n").unwrap(), DataFormat::Yaml);
        assert_eq!(detect_format("a:\n\tb: 1\n").unwrap(), DataFormat::Yaml);
        assert_eq!(detect_format("name = \"x\"\nport = 1\n").unwrap(), DataFormat::Toml);
        assert_eq!(detect_format("a,b\n1,2\n").unwrap(), DataFormat::Csv);
        // A comma inside a YAML scalar line must not flip detection to CSV.
        assert_eq!(
            detect_format("model: a,b,c\nclients:\n  - type: openai\n").unwrap(),
            DataFormat::Yaml
        );
        // A plain comma table without `key:`/`-` structure stays CSV.
        assert_eq!(detect_format("name,age\ngary,30\n").unwrap(), DataFormat::Csv);
    }

    #[test]
    fn input_format_precedence_file_ext_vs_flag() {
        // -f 后缀决定默认格式；-i 显式传入时覆盖后缀。
        let ext = format_from_ext(Path::new("cfg.yaml")).unwrap();
        assert_eq!(ext, DataFormat::Yaml);
        let overridden = parse_format("toml").unwrap();
        assert_eq!(overridden, DataFormat::Toml);
        // cmd_data 的选择顺序等价于：in_fmt.clone().unwrap_or(source_fmt/detect)
        let chosen = Some(overridden).or(Some(ext)).unwrap();
        assert_eq!(chosen, DataFormat::Toml);
        // URL 默认 json，-i 亦覆盖。
        let url_default = Some(DataFormat::Json);
        let chosen = Some(parse_format("csv").unwrap()).or(url_default).unwrap();
        assert_eq!(chosen, DataFormat::Csv);
    }

    #[test]
    fn table_format_parse_and_roundtrip() {
        assert_eq!(parse_format("table").unwrap(), DataFormat::Table);
        assert!(parse_format("TABLE").is_ok());
        assert!(parse_format("xlsx").is_err());
        let v = parse_input("A B\n1 2\n", DataFormat::Table, None).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0]["A"], 1);
        assert_eq!(arr[0]["B"], 2);
        let t = serialize_output(&v, DataFormat::Table, false).unwrap();
        assert_eq!(t, "A B\n1 2\n");
        // --no-header → colN 键。
        let v = parse_input("1 2\n3 4\n", DataFormat::Table, Some(false)).unwrap();
        assert_eq!(v[0]["col1"], 1);
        assert_eq!(v[0]["col2"], 2);
    }

    #[test]
    fn selector_root_array_index() {
        assert_eq!(
            parse_selector("[0].name").unwrap(),
            vec![Step::Index(Some(0)), Step::Key("name".into())]
        );
        assert_eq!(parse_selector("[-1]").unwrap(), vec![Step::Index(Some(-1))]);
        assert_eq!(
            parse_selector("[*].id").unwrap(),
            vec![Step::Index(None), Step::Key("id".into())]
        );
        // 根数组 + 列标题查询（table 解析结果的直接寻址）。
        let v = parse_input("USER PID\nroot 1\ngary 2\n", DataFormat::Table, None).unwrap();
        let got = get_values(&v, &parse_selector("[1].PID").unwrap());
        assert_eq!(got.len(), 1);
        assert_eq!(*got[0], json!(2));
        let got = get_values(&v, &parse_selector("[0].USER").unwrap());
        assert_eq!(*got[0], json!("root"));
    }

    #[test]
    fn render_get_scalar_string() {
        // dasel-style: scalar strings print unquoted.
        let s = render_get(&json!("你好"), DataFormat::Json, false).unwrap();
        assert_eq!(s, "你好\n");
        let s = render_get(&json!(42), DataFormat::Json, false).unwrap();
        assert_eq!(s, "42\n");
        // `-o table` 下标量同样直接打印（数字 / 布尔 / 字符串）。
        let s = render_get(&json!(42), DataFormat::Table, false).unwrap();
        assert_eq!(s, "42\n");
        let s = render_get(&json!(true), DataFormat::Table, false).unwrap();
        assert_eq!(s, "true\n");
        let s = render_get(&json!("root"), DataFormat::Table, false).unwrap();
        assert_eq!(s, "root\n");
    }

    // ------------------------------------------------------------------
    // Function expressions: filter / search / remove / string functions
    // ------------------------------------------------------------------

    #[test]
    fn function_expr_parsing() {
        let fc = parse_function_expr("list.filter(age>=18)").unwrap().unwrap();
        assert_eq!(fc.path.as_deref(), Some("list"));
        assert_eq!(fc.name, "filter");
        assert_eq!(fc.args, "age>=18");
        let fc = parse_function_expr("sites.remove(name.contains(\"web\"))").unwrap().unwrap();
        assert_eq!(fc.path.as_deref(), Some("sites"));
        assert_eq!(fc.name, "remove");
        let fc = parse_function_expr("search(has(\"name\"))").unwrap().unwrap();
        assert_eq!(fc.path, None);
        assert_eq!(fc.name, "search");
        let fc = parse_function_expr("name.endsWith(\"x\")").unwrap().unwrap();
        assert_eq!(fc.name, "endsWith");
        // Nested / quoted paths keep working.
        let fc = parse_function_expr(r#"m["a.b"].filter($this==1)"#).unwrap().unwrap();
        assert_eq!(fc.path.as_deref(), Some(r#"m["a.b"]"#));
        // Function-like shape with an unknown name errors; plain selectors and
        // assignments are not function calls.
        assert!(parse_function_expr("list.foo(1)").unwrap().is_err());
        assert!(parse_function_expr("a.b").is_none());
        assert!(parse_function_expr("a.b = 1").is_none());
        // `==` inside a function call is not an assignment operator.
        assert!(split_expression("list.filter(type==1)").is_err());
        assert!(split_expression("a.b = 1").is_ok());
    }

    #[test]
    fn predicate_evaluation() {
        let node = json!({"name": "web-server", "type": 1, "tags": ["a", "b"]});
        assert!(eval_predicate(&node, None, "type==1").unwrap());
        assert!(eval_predicate(&node, None, "type >= 1 && name.startsWith(\"web\")").unwrap());
        assert!(eval_predicate(&node, None, "name.contains(\"server\")").unwrap());
        assert!(!eval_predicate(&node, None, "name.endsWith(\"x\")").unwrap());
        assert!(eval_predicate(&node, None, "has(\"type\") && !has(\"nope\")").unwrap());
        assert!(eval_predicate(&node, None, "tags[0] == \"a\"").unwrap());
        assert!(eval_predicate(&node, None, "$this.name == \"web-server\"").unwrap());
        // $key: number for array indices, string for object keys.
        assert!(eval_predicate(&node, Some(&json!(3)), "$key > 2").unwrap());
        assert!(eval_predicate(&json!("v"), Some(&json!("k")), "$key == \"k\"").unwrap());
        // Missing fields are null: `== null` true, ordering false.
        assert!(eval_predicate(&node, None, "missing == null").unwrap());
        assert!(!eval_predicate(&node, None, "missing != null").unwrap());
        assert!(!eval_predicate(&node, None, "missing > 1").unwrap());
        // String functions applied to the node itself.
        assert!(eval_predicate(&json!("hello world"), None, "indexOf(\"o\") >= 0").unwrap());
        assert!(eval_predicate(&json!("hello"), None, "contains(\"ell\")").unwrap());
        // A non-boolean predicate is an error.
        assert!(eval_predicate(&node, None, "type").is_err());
    }

    #[test]
    fn filter_keeps_matching_elements() {
        let v = json!({"sites": [
            {"name": "web", "type": 1},
            {"name": "db", "type": 2},
            {"name": "api", "type": 1},
        ]});
        let fc = parse_function_expr("sites.filter(type==1)").unwrap().unwrap();
        let out = apply_filter(&v, &fc, DataFormat::Json, false).unwrap();
        let got: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(got.as_array().unwrap().len(), 2);
        assert_eq!(got[0]["name"], json!("web"));
        assert_eq!(got[1]["name"], json!("api"));
        // Non-array targets error.
        let fc = parse_function_expr("sites[0].filter(x)").unwrap().unwrap();
        assert!(apply_filter(&v, &fc, DataFormat::Json, false).is_err());
    }

    #[test]
    fn bare_predicate_detection() {
        assert!(looks_like_bare_predicate("Id==7360"));
        assert!(looks_like_bare_predicate("Id == 7360"));
        assert!(looks_like_bare_predicate("age>=18 && name.contains(\"web\")"));
        assert!(looks_like_bare_predicate("!has(\"x\")"));
        assert!(looks_like_bare_predicate("!(a==1)"));
        assert!(looks_like_bare_predicate("a!=b"));
        assert!(!looks_like_bare_predicate("Id"));
        assert!(!looks_like_bare_predicate("a.b[0]"));
        assert!(!looks_like_bare_predicate("m[\"中文键\"]"));
        assert!(!looks_like_bare_predicate(r#""a==b""#)); // 纯引号内容忽略
        assert!(looks_like_bare_predicate(r#"name == "a==b""#)); // 引号外 == 触发
        assert!(!looks_like_bare_predicate("Id=7360")); // 单 = 赋值，走 split_expression
    }

    #[test]
    fn bare_predicate_on_csv_auto_filters() {
        // table/csv 根数组 + 裸谓词 = 自动 filter（等价于 [*].filter(...)）。
        let v = parse_input("Id Name\n1 a\n2 b\n3 c\n", DataFormat::Table, None).unwrap();
        let fc = FunctionCall {
            path: Some("[*]".to_string()),
            name: "filter".to_string(),
            args: "Id==2".to_string(),
        };
        let out = apply_filter(&v, &fc, DataFormat::Json, false).unwrap();
        let got: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(got.as_array().unwrap().len(), 1);
        assert_eq!(got[0]["Name"], "b");
    }

    #[test]
    fn bare_method_predicate_auto_filters_root_array() {
        // `Name.contains("a")` / `has("Id")` 等裸方法谓词在 table/csv/jsonl
        // 根数组上等价于 `[*].filter(...)`（自动过滤分支构造同一合成 fc）。
        let v = parse_input("Id Name\n1 alpha\n2 beta\n3 gamma\n", DataFormat::Table, None).unwrap();
        let cases = [
            ("Name.contains(\"ph\")", 1),    // 仅 alpha 含 ph
            ("Name.startsWith(\"ga\")", 1),   // 仅 gamma
            ("Name.endsWith(\"ma\")", 1),    // 仅 gamma
            ("has(\"Id\")", 3),              // 每行都有 Id 键
        ];
        for (pred, want) in cases {
            let auto = FunctionCall {
                path: Some("[*]".to_string()),
                name: "filter".to_string(),
                args: pred.to_string(),
            };
            let out = apply_filter(&v, &auto, DataFormat::Json, false).unwrap();
            let got: Value = serde_json::from_str(&out).unwrap();
            assert_eq!(got.as_array().unwrap().len(), want, "predicate {pred}");
        }
    }

    #[test]
    fn count_matches_node_set_sizes() {
        // 单个数组结果 → 数组长度
        let arr = json!([1, 2, 3]);
        assert_eq!(count_matches(&[&arr]), json!(3));
        // 空数组 → 0
        let empty = json!([]);
        assert_eq!(count_matches(&[&empty]), json!(0));
        // 单值节点 → 1
        let scalar = json!(5);
        assert_eq!(count_matches(&[&scalar]), json!(1));
        // 多个匹配节点 → 节点数（data[*] 展开）
        let m = vec![json!(1), json!(2), json!(3), json!(4)];
        let refs: Vec<&Value> = m.iter().collect();
        assert_eq!(count_matches(&refs), json!(4));
        // 无匹配 → 0
        assert_eq!(count_matches(&[]), json!(0));
    }

    #[test]
    fn count_dot_style_and_bare_and_arg_errors() {
        let v = json!({"data": [{"n": 1}, {"n": 2}, {"n": 3}]});
        // path.count()
        let fc = parse_function_expr("data.count()").unwrap().unwrap();
        assert_eq!(fc.path.as_deref(), Some("data"));
        assert_eq!(fc.name, "count");
        let out = apply_count(&v, &fc, DataFormat::Json, false).unwrap();
        assert_eq!(out.trim(), "3");
        // 裸 count() 统计整个文档
        let fc = parse_function_expr("count()").unwrap().unwrap();
        assert!(fc.path.is_none());
        let out = apply_count(&v, &fc, DataFormat::Json, false).unwrap();
        assert_eq!(out.trim(), "1"); // 文档是对象 → 1 个节点
        // 数组根文档 → 长度
        let arr = json!([1, 2, 3, 4, 5]);
        let out = apply_count(&arr, &fc, DataFormat::Json, false).unwrap();
        assert_eq!(out.trim(), "5");
        // 无匹配 selector → 0 而非报错
        let fc = parse_function_expr("missing.count()").unwrap().unwrap();
        let out = apply_count(&v, &fc, DataFormat::Json, false).unwrap();
        assert_eq!(out.trim(), "0");
        // count 带参数 → 报错
        let fc = parse_function_expr("data.count(x)").unwrap().unwrap();
        assert!(apply_count(&v, &fc, DataFormat::Json, false).is_err());
    }

    #[test]
    fn count_pipeline_parses_via_split_pipeline() {
        // `path | count()` 的管道形式：parse_function_expr 必须让位（name 含 |）
        assert!(parse_function_expr("data|count()").is_none());
        // `filter(...) | count()` 同样让位给管道
        assert!(parse_function_expr("data.filter(n>=2)|count()").is_none());
        // split_pipeline 正确切分左右
        let (left, right) = split_pipeline("data.filter(n>=2)|count()").unwrap();
        assert_eq!(left, "data.filter(n>=2)");
        assert_eq!(right, "count()");
    }

    #[test]
    fn filter_on_root_array_via_wildcard() {
        // `ps | sys data -i table '[*].filter(Id==7360)'` — a bare `[*]` on a
        // root array addresses the array itself.
        let v = parse_input("Id Name\n1 a\n2 b\n3 c\n", DataFormat::Table, None).unwrap();
        let fc = parse_function_expr("[*].filter(Id >= 2)").unwrap().unwrap();
        assert_eq!(fc.path.as_deref(), Some("[*]"));
        let out = apply_filter(&v, &fc, DataFormat::Json, false).unwrap();
        let got: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(got.as_array().unwrap().len(), 2);
        assert_eq!(got[0]["Id"], json!(2));
        assert_eq!(got[0]["Name"], json!("b"));
        // search works the same way over the root array.
        let fc = parse_function_expr("[*].search(Id == 1)").unwrap().unwrap();
        let out = apply_search(&v, &fc, DataFormat::Json, false).unwrap();
        let got: Value = serde_json::from_str(&out).unwrap();
        let arr = got.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["Name"], json!("a"));
        // Nested arrays under a key keep the previous behaviour (expanded
        // elements still error — no root special case).
        let o = json!({"sites": [{"n": 1}, {"n": 2}]});
        let fc = parse_function_expr("sites[*].filter(n==1)").unwrap().unwrap();
        assert!(apply_filter(&o, &fc, DataFormat::Json, false).is_err());
    }

    #[test]
    fn remove_deletes_matching_parent_elements() {
        let mut v = json!({"sites": [
            {"name": "web", "url": "https://a.example"},
            {"name": "db", "url": "https://b.example"},
            {"name": "web2", "url": "https://c.example"},
        ]});
        let fc = parse_function_expr("sites.remove(name.contains(\"web\"))").unwrap().unwrap();
        let out = apply_remove(&mut v, &fc, DataFormat::Json, false, false).unwrap();
        let got: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(got.as_array().unwrap().len(), 1);
        assert_eq!(got[0]["name"], json!("db"));
        // The document itself was mutated in place.
        assert_eq!(v["sites"].as_array().unwrap().len(), 1);
        // --root prints the whole (mutated) document.
        let mut v = json!({"sites": [{"name": "x"}, {"name": "y"}]});
        let fc = parse_function_expr("sites.remove(name==\"x\")").unwrap().unwrap();
        let out = apply_remove(&mut v, &fc, DataFormat::Json, false, true).unwrap();
        assert!(out.contains("\"sites\""), "root output: {out}");
        assert!(!out.contains("\"x\""), "root output must not contain removed element: {out}");
    }

    #[test]
    fn search_collects_matching_nodes() {
        let v = json!({
            "a": {"name": "x", "id": 1},
            "list": [
                {"name": "y", "id": 2},
                {"name": "z", "id": 3},
                {"plain": "no-name"},
            ],
            "b": {"name": "w", "id": 4},
        });
        let fc = parse_function_expr("search(has(\"name\"))").unwrap().unwrap();
        let out = apply_search(&v, &fc, DataFormat::Json, false).unwrap();
        let got: Value = serde_json::from_str(&out).unwrap();
        let arr = got.as_array().unwrap();
        assert_eq!(arr.len(), 4);
        assert!(arr.iter().all(|n| n.is_object() && n.get("name").is_some()));
        // $key matching: the leaf nodes under the `name` keys.
        let fc = parse_function_expr(r#"search($key == "name")"#).unwrap().unwrap();
        let out = apply_search(&v, &fc, DataFormat::Json, false).unwrap();
        let got: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(got.as_array().unwrap().len(), 4);
        assert!(got.as_array().unwrap().iter().all(|n| n.is_string()));
    }

    #[test]
    fn string_functions_on_selector() {
        let v = json!({"name": "web-server", "list": ["x", "hello"]});
        for (expr, want) in [
            ("name.endsWith(\"server\")", "true"),
            ("name.startsWith(\"web\")", "true"),
            ("name.contains(\"-s\")", "true"),
            ("name.indexOf(\"server\")", "4"),
            ("name.indexOf(\"zz\")", "-1"),
        ] {
            let fc = parse_function_expr(expr).unwrap().unwrap();
            let out = apply_string_fn(&v, &fc, DataFormat::Json, false).unwrap();
            assert_eq!(out.trim(), want, "expr: {expr}");
        }
        let fc = parse_function_expr("list[1].endsWith(\"lo\")").unwrap().unwrap();
        assert_eq!(apply_string_fn(&v, &fc, DataFormat::Json, false).unwrap().trim(), "true");
    }

    #[test]
    fn prune_depth_removes_deeper_levels() {
        let mut v = json!({
            "a": {"b": {"c": 1}, "k": 2},
            "list": [{"deep": {"x": 1}}, "plain"],
        });
        prune_depth(&mut v, 1);
        // Root = level 0; nodes at level > 1 are removed entirely, so only the
        // top-level keys (a, list) survive, emptied of their level-2 children.
        assert_eq!(v, json!({"a": {}, "list": []}));
        let mut v = json!({"a": {"b": {"c": 1}}});
        prune_depth(&mut v, 2);
        assert_eq!(v, json!({"a": {"b": {}}}));
        let mut v = json!({"a": {"b": {"c": 1}}});
        prune_depth(&mut v, 3);
        assert_eq!(v, json!({"a": {"b": {"c": 1}}}));
    }

    // ------------------------------------------------------------------
    // --limit N: keep the first N rows of an array document
    // ------------------------------------------------------------------

    #[test]
    fn apply_limit_truncates_array_rows() {
        let mut v = json!([{"A": 1}, {"A": 2}, {"A": 3}]);
        apply_limit(&mut v, 2).unwrap();
        assert_eq!(v, json!([{"A": 1}, {"A": 2}]));
        // N = 0 -> empty; N >= len -> unchanged.
        let mut v = json!([{"A": 1}]);
        apply_limit(&mut v, 0).unwrap();
        assert_eq!(v, json!([]));
        let mut v = json!([{"A": 1}, {"A": 2}]);
        apply_limit(&mut v, 5).unwrap();
        assert_eq!(v, json!([{"A": 1}, {"A": 2}]));
    }

    #[test]
    fn apply_limit_rejects_non_array() {
        let mut v = json!({"a": 1});
        assert!(apply_limit(&mut v, 1).is_err());
        let mut v = json!("rows");
        assert!(apply_limit(&mut v, 1).is_err());
        let mut v = json!(42);
        assert!(apply_limit(&mut v, 1).is_err());
    }

    #[test]
    fn limit_flows_through_table_and_csv() {
        // `sys data -i table --limit 2` core path: parse -> limit -> serialize
        // keeps the header plus the first 2 data rows.
        let mut v = parse_input("A B\n1 2\n3 4\n5 6\n", DataFormat::Table, None).unwrap();
        apply_limit(&mut v, 2).unwrap();
        assert_eq!(serialize_output(&v, DataFormat::Table, false).unwrap(), "A B\n1 2\n3 4\n");
        // Same for csv.
        let mut v = parse_input("A,B\n1,2\n3,4\n5,6\n", DataFormat::Csv, None).unwrap();
        apply_limit(&mut v, 2).unwrap();
        assert_eq!(serialize_output(&v, DataFormat::Csv, false).unwrap(), "A,B\n1,2\n3,4\n");
        // --no-header: rows are pure data, col1..colN keys.
        let mut v = parse_input("1 2\n3 4\n5 6\n", DataFormat::Table, Some(false)).unwrap();
        apply_limit(&mut v, 1).unwrap();
        assert_eq!(serialize_output(&v, DataFormat::Table, false).unwrap(), "col1 col2\n1    2   \n");
    }

    // ------------------------------------------------------------------
    // Non-UTF-8 input decoding (GBK / UTF-16 / lossy fallback)
    // ------------------------------------------------------------------

    #[test]
    fn decode_text_utf8_and_bom() {
        assert_eq!(decode_text(b"abc"), "abc");
        assert_eq!(decode_text("中文".as_bytes()), "中文");
        // A UTF-8 BOM is an explicit declaration and is stripped.
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice("中文".as_bytes());
        assert_eq!(decode_text(&bom), "中文");
    }

    #[test]
    fn decode_text_gbk_wins_over_lookalike_utf8() {
        // GBK `专业` = D7 A8 D2 B5 is *also* valid UTF-8 (Hebrew + Cyrillic);
        // GBK must win because the UTF-8 view has no CJK.
        assert_eq!(decode_text(&[0xD7, 0xA8, 0xD2, 0xB5]), "专业");
        // Mixed ASCII + GBK (real-world log line shape).
        assert_eq!(decode_text(&[b'a', 0xD7, 0xA8, 0xD2, 0xB5, b'b']), "a专业b");
    }

    #[test]
    fn decode_text_gbk_invalid_utf8_and_utf16() {
        // A GBK byte stream that is not valid UTF-8 (0xB0 0xE6 = `版`).
        assert_eq!(decode_text(&[0xD7, 0xA8, 0xD2, 0xB5, 0xB0, 0xE6]), "专业版");
        // UTF-16LE with BOM (`你好` = U+4F60 U+597D).
        assert_eq!(decode_text(&[0xFF, 0xFE, 0x60, 0x4F, 0x7D, 0x59]), "你好");
        // Garbage that decodes under no encoding -> lossy U+FFFD, no panic.
        let s = decode_text(&[0xFF, 0xFE, 0x01, 0x02, 0x03]);
        assert!(s.chars().any(|c| c == '\u{FFFD}'));
    }

    // ------------------------------------------------------------------
    // JSONL (JSON Lines): one JSON value per line -> root array
    // ------------------------------------------------------------------

    #[test]
    fn jsonl_parse_and_roundtrip() {
        let src = "{\"level\":\"info\",\"pid\":1}\n{\"level\":\"error\",\"pid\":2}\n";
        let v = parse_input(src, DataFormat::Jsonl, None).unwrap();
        assert!(v.is_array());
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["level"], "info");
        assert_eq!(arr[1]["pid"], 2);
        // Round-trip: serialize back to one compact JSON per line.
        assert_eq!(serialize_output(&v, DataFormat::Jsonl, false).unwrap(), src);
        // Blank lines are skipped.
        let v = parse_input("{\"a\":1}\n\n{\"a\":2}\n", DataFormat::Jsonl, None).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
    }

    #[test]
    fn jsonl_invalid_line_reports_line_number() {
        let err = parse_input("{\"a\":1}\nnot json\n{\"a\":3}\n", DataFormat::Jsonl, None)
            .unwrap_err();
        assert!(format!("{err:#}").contains("line 2"), "got: {err:#}");
    }

    #[test]
    fn detect_format_jsonl_vs_pretty_json() {
        // Multi-line pretty JSON is one JSON value -> Json.
        let pretty = "{\n  \"a\": 1,\n  \"b\": [1, 2]\n}\n";
        assert_eq!(detect_format(pretty).unwrap(), DataFormat::Json);
        // One JSON value per line -> Jsonl.
        let jsonl = "{\"a\":1}\n{\"a\":2}\n";
        assert_eq!(detect_format(jsonl).unwrap(), DataFormat::Jsonl);
        // A single JSON object line is still plain Json.
        assert_eq!(detect_format("{\"a\":1}\n").unwrap(), DataFormat::Json);
    }

    #[test]
    fn jsonl_bare_predicate_filters_array() {
        // Core of `sys data -f x.jsonl 'level=="error"'`: parse -> root array
        // -> auto-filter on the bare predicate -> jsonl output.
        let v = parse_input(
            "{\"level\":\"info\",\"pid\":1}\n{\"level\":\"error\",\"pid\":2}\n{\"level\":\"error\",\"pid\":3}\n",
            DataFormat::Jsonl,
            None,
        ).unwrap();
        let fc = FunctionCall {
            path: Some("[*]".to_string()),
            name: "filter".to_string(),
            args: "level==\"error\"".to_string(),
        };
        let out = apply_filter(&v, &fc, DataFormat::Jsonl, false).unwrap();
        assert_eq!(out, "{\"level\":\"error\",\"pid\":2}\n{\"level\":\"error\",\"pid\":3}\n");
        // --limit composes with the array root.
        let mut v = parse_input(
            "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n",
            DataFormat::Jsonl,
            None,
        ).unwrap();
        apply_limit(&mut v, 2).unwrap();
        assert_eq!(serialize_output(&v, DataFormat::Jsonl, false).unwrap(), "{\"a\":1}\n{\"a\":2}\n");
    }

    // ------------------------------------------------------------------
    // jq-style key projection: `path | {k1, k2}`
    // ------------------------------------------------------------------

    #[test]
    fn projection_on_wildcard_keeps_only_listed_keys() {
        let v = json!({"data": [
            {"id": 1, "name": "a", "pricing": 9.9, "extra": true},
            {"id": 2, "name": "b", "pricing": 19.9, "extra": false},
        ]});
        let items = parse_projection("{id, name, pricing}").unwrap();
        let item_keys: Vec<&str> = items
            .iter()
            .map(|it| match it {
                ProjItem::Literal { key, .. } | ProjItem::FromPath { key, .. } => key.as_str(),
            })
            .collect();
        assert_eq!(item_keys, vec!["id", "name", "pricing"]);
        let out = apply_projection_path(&v, "data[*]", &items, DataFormat::Json, false).unwrap();
        let got: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(got.as_array().unwrap().len(), 2);
        assert_eq!(got[0], json!({"id": 1, "name": "a", "pricing": 9.9}));
        assert_eq!(got[1], json!({"id": 2, "name": "b", "pricing": 19.9}));
        assert!(got[0].get("extra").is_none(), "unlisted key leaked: {got}");
    }

    #[test]
    fn projection_single_match_emits_object_not_array() {
        let v = json!({"data": [{"id": 1, "name": "a"}, {"id": 2, "name": "b"}]});
        let keys = parse_projection("{id, name}").unwrap();
        let out = apply_projection_path(&v, "data[1]", &keys, DataFormat::Json, false).unwrap();
        let got: Value = serde_json::from_str(&out).unwrap();
        assert!(got.is_object(), "single match must be an object: {out}");
        assert_eq!(got, json!({"id": 2, "name": "b"}));
    }

    #[test]
    fn projection_missing_key_becomes_null() {
        let v = json!({"data": [{"id": 1}, {"id": 2, "pricing": 5.0}]});
        let keys = parse_projection("{id, pricing}").unwrap();
        let out = apply_projection_path(&v, "data[*]", &keys, DataFormat::Json, false).unwrap();
        let got: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(got[0], json!({"id": 1, "pricing": null}));
        assert_eq!(got[1], json!({"id": 2, "pricing": 5.0}));
    }

    #[test]
    fn projection_after_filter_spreads_result_array() {
        // `data.filter(id>=2)|{id,name}`: the filter result array is spread so
        // the projection applies per element (jq `.[]|select(...)|{...}`).
        let v = json!({"data": [
            {"id": 1, "name": "a", "pricing": 9.9},
            {"id": 2, "name": "b", "pricing": 19.9},
        ]});
        let fc = parse_function_expr("data.filter(id>=2)").unwrap().unwrap();
        let arr = apply_filter_value(&v, &fc).unwrap();
        let keys = parse_projection("{id, name}").unwrap();
        let out = project_values(
            arr.as_array().cloned().unwrap_or_default(),
            &keys,
            DataFormat::Json,
            false,
        )
        .unwrap();
        let got: Value = serde_json::from_str(&out).unwrap();
        // A single projected element is emitted as an object, not an array.
        assert!(got.is_object(), "single projection must be an object: {out}");
        assert_eq!(got, json!({"id": 2, "name": "b"}));
        // filter matching nothing -> `[]`, not an error.
        let fc = parse_function_expr("data.filter(id>=99)").unwrap().unwrap();
        let arr = apply_filter_value(&v, &fc).unwrap();
        let out = project_values(
            arr.as_array().cloned().unwrap_or_default(),
            &keys,
            DataFormat::Json,
            true,
        )
        .unwrap();
        assert_eq!(out.trim(), "[]");
    }

    #[test]
    fn projection_path_matched_nothing_errors() {
        let v = json!({"data": [{"id": 1}]});
        let keys = parse_projection("{id}").unwrap();
        assert!(apply_projection_path(&v, "missing[*]", &keys, DataFormat::Json, false).is_err());
        // Projecting a non-object node yields null values, not an error.
        let v = json!({"data": [1, 2, 3]});
        let out = apply_projection_path(&v, "data[*]", &keys, DataFormat::Json, true).unwrap();
        assert_eq!(out.trim(), "[{\"id\":null},{\"id\":null},{\"id\":null}]");
    }

    #[test]
    fn parse_projection_validation() {
        // 新构造语法：嵌套路径 / 别名 / 下标均合法（子节点提升 / 新字段）。
        let items = parse_projection("{a.b}").unwrap();
        assert_eq!(proj_keys(&items), vec!["b"]);
        let items = parse_projection("{x: y}").unwrap();
        assert_eq!(proj_keys(&items), vec!["x"]);
        assert!(parse_projection("{a[0]}").is_ok());
        assert!(parse_projection("{}").is_err());
        assert!(parse_projection("id").is_err());
        let items = parse_projection("{id, name,}").unwrap();
        assert_eq!(proj_keys(&items), vec!["id", "name"]);
        let items = parse_projection("{ one , two }").unwrap();
        assert_eq!(proj_keys(&items), vec!["one", "two"]);
        // 空键名 / 悬空别名 / 非法键字符报错。
        assert!(parse_projection("{:a}").is_err());
        assert!(parse_projection("{a:}").is_err());
        assert!(parse_projection("{a.b:c}").is_err());
    }

    #[test]
    fn bare_projection_on_object_root_keeps_only_listed_keys() {
        // `sys data '{wallpaper,warningText}'` on an object root: jq
        // `. | {k1,k2}` semantics — a single object, other keys dropped.
        let v = json!({"wallpaper": {"id": 1}, "warningText": "warn", "other": 42});
        let out = apply_bare_projection(&v, "{wallpaper, warningText}", DataFormat::Json, false).unwrap();
        let got: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(got, json!({"wallpaper": {"id": 1}, "warningText": "warn"}));
    }

    #[test]
    fn bare_projection_on_array_root_projects_every_row() {
        // Array root (csv / table / jsonl parse to one): element-wise, the
        // same as `[*]|{Id,Name}`.
        let v = parse_input(
            "[{\"Id\":\"1\",\"Name\":\"a\",\"Drop\":true},{\"Id\":\"2\",\"Name\":\"b\",\"Drop\":true}]",
            DataFormat::Json,
            None,
        )
        .unwrap();
        let out = apply_bare_projection(&v, "{Id, Name}", DataFormat::Json, true).unwrap();
        assert_eq!(out.trim(), "[{\"Id\":\"1\",\"Name\":\"a\"},{\"Id\":\"2\",\"Name\":\"b\"}]");
        // table 输入（首行表头）同样逐行投影；单元格类型推断：数字列解析为数字。
        let v = parse_input("Id Name\n1 a\n2 b\n", DataFormat::Table, None).unwrap();
        let out = apply_bare_projection(&v, "{Id}", DataFormat::Json, true).unwrap();
        assert_eq!(out.trim(), "[{\"Id\":1},{\"Id\":2}]");
    }

    #[test]
    fn bare_projection_missing_key_null_and_scalar_root() {
        // Missing keys become null, jq-style; a scalar root yields one
        // object with all-null values instead of an error.
        let v = json!([{"id": 1}, {"name": "x"}]);
        let out = apply_bare_projection(&v, "{id,name}", DataFormat::Json, true).unwrap();
        assert_eq!(
            out.trim(),
            "[{\"id\":1,\"name\":null},{\"id\":null,\"name\":\"x\"}]"
        );
        let v = json!(5);
        let out = apply_bare_projection(&v, "{a}", DataFormat::Json, true).unwrap();
        assert_eq!(out.trim(), "{\"a\":null}");
    }

    #[test]
    fn bare_projection_validates_key_list() {
        let v = json!({"a": 1});
        // 嵌套路径现在合法：`{a.b}` 提升 b 到当前层（源 a.b 缺失 -> null）。
        let out = apply_bare_projection(&v, "{a.b}", DataFormat::Json, true).unwrap();
        assert_eq!(out.trim(), r#"{"b":null}"#);
        assert!(apply_bare_projection(&v, "{}", DataFormat::Json, false).is_err());
        assert!(apply_bare_projection(&v, "id", DataFormat::Json, false).is_err());
        // 空键列表（仅逗号）同样报错。
        assert!(apply_bare_projection(&v, "{,}", DataFormat::Json, false).is_err());
    }

    // ------------------------------------------------------------------
    // Object construction: `{"new":literal, key, a.b.lift, "alias":path}`
    // ------------------------------------------------------------------

    #[test]
    fn construction_literal_alias_and_lift() {
        // 用户示例语义：字面量新字段 / 保留根字段 / 子节点提升 / 别名路径取值。
        let v = json!({
            "key1": {"deep": 1},
            "key2": {"child1": "lifted", "other": true},
            "key3": {"key2": [1, 2]},
        });
        let out = apply_bare_projection(
            &v,
            r#"{"name":"Mike",key1,key2.child1,"country":key3.key2}"#,
            DataFormat::Json,
            true,
        )
        .unwrap();
        assert_eq!(
            out.trim(),
            r#"{"name":"Mike","key1":{"deep":1},"child1":"lifted","country":[1,2]}"#
        );
    }

    #[test]
    fn construction_literal_kinds() {
        let v = json!({});
        let out = apply_bare_projection(
            &v,
            r#"{"s":"x","n":123,"f":1.5,"b":true,"z":null,"arr":[1,"a"],"obj":{"k":"v"}}"#,
            DataFormat::Json,
            true,
        )
        .unwrap();
        let got: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            got,
            json!({"s":"x","n":123,"f":1.5,"b":true,"z":null,"arr":[1,"a"],"obj":{"k":"v"}})
        );
    }

    #[test]
    fn construction_missing_path_null_and_deep_copy() {
        // 路径缺失 -> null；整棵子树原样拷贝（不浅取）。
        let v = json!({"a": {"b": {"c": [1]}}, "top": {"n": 5}});
        let out = apply_bare_projection(
            &v,
            r#"{top, a.b, "x":missing.x, "m":a.b.c}"#,
            DataFormat::Json,
            true,
        )
        .unwrap();
        assert_eq!(
            out.trim(),
            r#"{"top":{"n":5},"b":{"c":[1]},"x":null,"m":[1]}"#
        );
    }

    #[test]
    fn construction_bare_quoted_key_keeps_field() {
        let v = json!({"中文键": 7, "a": 1});
        let out = apply_bare_projection(&v, r#"{"中文键"}"#, DataFormat::Json, true).unwrap();
        assert_eq!(out.trim(), r#"{"中文键":7}"#);
        // 单引号裸键同样保留字段。
        let out = apply_bare_projection(&v, "{'a'}", DataFormat::Json, true).unwrap();
        assert_eq!(out.trim(), r#"{"a":1}"#);
    }

    #[test]
    fn construction_literal_with_commas_and_braces() {
        // 值里的逗号 / 花括号不参与元素切分。
        let v = json!({"x": 1});
        let out = apply_bare_projection(
            &v,
            r#"{"a":{"k1":1,"k2":2},"b":[1,2],"c":"s,with,commas"}"#,
            DataFormat::Json,
            true,
        )
        .unwrap();
        assert_eq!(
            out.trim(),
            r#"{"a":{"k1":1,"k2":2},"b":[1,2],"c":"s,with,commas"}"#
        );
    }

    #[test]
    fn construction_pipeline_and_trailing() {
        // 管道投影 `path|{...}` 与尾随投影 `path{...}` 同样支持构造语法。
        let v = json!({
            "data": [
                {"id": 1, "meta": {"tag": "a"}},
                {"id": 2, "meta": {"tag": "b"}},
            ]
        });
        let items = parse_projection(r#"{"id", "tag":meta.tag, "name":"x"}"#).unwrap();
        let out = apply_projection_path(&v, "data[*]", &items, DataFormat::Json, true).unwrap();
        assert_eq!(
            out.trim(),
            r#"[{"id":1,"tag":"a","name":"x"},{"id":2,"tag":"b","name":"x"}]"#
        );
        let out =
            apply_trailing_projection(&v, r#"data[0]{"id","t":meta.tag}"#, DataFormat::Json, true)
                .unwrap();
        assert_eq!(out.trim(), r#"{"id":1,"t":"a"}"#);
    }

    #[test]
    fn construction_bad_path_errors() {
        let v = json!({"a": {"b": 1}});
        // 路径语法非法（双点）-> 报错，而不是静默 null。
        assert!(apply_bare_projection(&v, "{a..b}", DataFormat::Json, false).is_err());
        // 值部分是合法 JSON 字面量时优先按字面量处理：`"k":123` 是数字而非路径。
        let out = apply_bare_projection(&v, r#"{"k":123}"#, DataFormat::Json, true).unwrap();
        assert_eq!(out.trim(), r#"{"k":123}"#);
    }

    #[test]
    fn split_trailing_projection_shapes() {
        // PATH{...} / PATH.{...} 拆分为路径 + 投影；裸 / 管道 / 无尾随投影返回 None。
        assert_eq!(
            split_trailing_projection("sites[0]{key,name}"),
            Some(("sites[0]".to_string(), "{key,name}".to_string()))
        );
        assert_eq!(
            split_trailing_projection("sites[0].{key,name}"),
            Some(("sites[0]".to_string(), "{key,name}".to_string()))
        );
        assert_eq!(
            split_trailing_projection("m[\"a{b\"]{c}"),
            Some(("m[\"a{b\"]".to_string(), "{c}".to_string()))
        );
        assert!(split_trailing_projection("{a}").is_none()); // 裸投影
        assert!(split_trailing_projection("data[*] | {id,name}").is_none()); // 管道
        assert!(split_trailing_projection("a.b").is_none()); // 无投影
        // 尾随投影体现在支持嵌套路径（提升）与别名（新字段），仍可拆分。
        assert_eq!(
            split_trailing_projection("a{b.c}"),
            Some(("a".to_string(), "{b.c}".to_string()))
        );
        assert_eq!(
            split_trailing_projection("a{\"n\":b.c}"),
            Some(("a".to_string(), "{\"n\":b.c}".to_string()))
        );
        assert!(split_trailing_projection("a{b").is_none()); // 不以 } 结尾
    }

    #[test]
    fn trailing_projection_on_deep_object_node() {
        // `sites[0]{key,name}` == `sites[0]|{key,name}`：单个深层对象节点
        // 投影，输出单个对象。
        let v = parse_input(
            r#"{"sites":[{"key":"a","name":"A","x":1},{"key":"b","name":"B"}]}"#,
            DataFormat::Json,
            None,
        )
        .unwrap();
        let out = apply_trailing_projection(&v, "sites[0]{key,name}", DataFormat::Json, true).unwrap();
        assert_eq!(out.trim(), r#"{"key":"a","name":"A"}"#);
        // 点式写法等价。
        let out2 = apply_trailing_projection(&v, "sites[0].{key,name}", DataFormat::Json, true).unwrap();
        assert_eq!(out, out2);
    }

    #[test]
    fn trailing_projection_on_deep_array_projects_every_row() {
        // `sites[*]{key}` 数组逐元素投影，与 `sites[*]|{key}` 输出一致。
        let v = parse_input(
            r#"{"sites":[{"key":"a","name":"A"},{"key":"b","name":"B"}]}"#,
            DataFormat::Json,
            None,
        )
        .unwrap();
        let direct = apply_trailing_projection(&v, "sites[*]{key,name}", DataFormat::Json, true).unwrap();
        let piped = apply_projection_path(&v, "sites[*]", &parse_projection("{key,name}").unwrap(), DataFormat::Json, true).unwrap();
        assert_eq!(direct, piped);
        assert_eq!(
            direct.trim(),
            r#"[{"key":"a","name":"A"},{"key":"b","name":"B"}]"#
        );
        // 切片 + 尾随投影（切片可作投影左侧）：前 1 个元素，单匹配输出
        // 单个对象（与 `|{key}` 语义一致）。
        let out = apply_trailing_projection(&v, "sites[:1]{key}", DataFormat::Json, true).unwrap();
        assert_eq!(out.trim(), r#"{"key":"a"}"#);
    }

    #[test]
    fn trailing_projection_invalid_keys_error() {
        let v = json!({"a": {"b": 1}});
        // `{b.c}` 在尾随投影里合法（把 a.b.c 提升为 c；源路径缺失 -> null）。
        let out = apply_trailing_projection(&v, "a{b.c}", DataFormat::Json, true).unwrap();
        assert_eq!(out.trim(), r#"{"c":null}"#);
        assert!(apply_trailing_projection(&v, "a{}", DataFormat::Json, false).is_err());
        // 路径本身非法（左括号函数形态）同样报错。
        assert!(apply_trailing_projection(&v, "a.filter(x==1){b}", DataFormat::Json, false).is_err());
    }

    #[test]
    fn split_pipeline_ignores_inner_pipes() {
        assert_eq!(
            split_pipeline("data[*] | {id,name}"),
            Some(("data[*]".to_string(), "{id,name}".to_string()))
        );
        assert_eq!(
            split_pipeline("x|{a}"),
            Some(("x".to_string(), "{a}".to_string()))
        );
        // `||` is never a split, nor pipes inside quotes / function args.
        assert!(split_pipeline("data.filter(a || b)").is_none());
        assert!(split_pipeline("m[\"a|b\"]").is_none());
        assert!(split_pipeline("name.contains(\"a|b\")").is_none());
        // An empty side is not a pipeline.
        assert!(split_pipeline("data|").is_none());
        assert!(split_pipeline("|{a}").is_none());
    }

    // ------------------------------------------------------------------
    // Assignment value files: `PATH = @FILE`
    // ------------------------------------------------------------------

    fn temp_value_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "sys_dasel_test_{}_{name}",
            std::process::id()
        ))
    }

    #[test]
    fn file_value_loads_json_by_extension() {
        let p = temp_value_path("v.json");
        std::fs::write(&p, r#"{"url": "https://a.example", "w": 3840}"#).unwrap();
        let raw = format!("@{}", p.display());
        let v = load_file_value(&raw, None).unwrap().unwrap();
        assert_eq!(v, json!({"url": "https://a.example", "w": 3840}));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn file_value_loads_yaml_by_extension() {
        let p = temp_value_path("v.yaml");
        std::fs::write(&p, "a: 1\nb: text\n").unwrap();
        let raw = format!("@{}", p.display());
        let v = load_file_value(&raw, None).unwrap().unwrap();
        assert_eq!(v, json!({"a": 1, "b": "text"}));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn file_value_unknown_extension_tries_json_then_yaml_then_text() {
        // JSON content in a .txt file still parses as a value.
        let p = temp_value_path("v.txt");
        std::fs::write(&p, r#"{"a": 1}"#).unwrap();
        let raw = format!("@{}", p.display());
        let v = load_file_value(&raw, None).unwrap().unwrap();
        assert_eq!(v, json!({"a": 1}));
        // Plain text falls back to a string.
        std::fs::write(&p, "hello world").unwrap();
        let v = load_file_value(&raw, None).unwrap().unwrap();
        assert_eq!(v, json!("hello world"));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn file_value_missing_file_and_bare_at_error() {
        let p = temp_value_path("missing.json");
        let raw = format!("@{}", p.display());
        let err = load_file_value(&raw, None).unwrap().unwrap_err();
        assert!(format!("{err:#}").contains("cannot read value file"), "got: {err:#}");
        let err = load_file_value("@", None).unwrap().unwrap_err();
        assert!(format!("{err:#}").contains("needs a file path"), "got: {err:#}");
    }

    #[test]
    fn file_value_ignores_quoted_and_plain_values() {
        // Quoted `"@x"` and non-`@` values are literals, not file references.
        assert!(load_file_value(r#""@x""#, None).is_none());
        assert!(load_file_value("42", None).is_none());
        assert!(load_file_value("x", None).is_none());
        assert!(load_file_value("", None).is_none());
    }

    // ------------------------------------------------------------------
    // Environment variable assignment values: `PATH = strenv(NAME)`
    // ------------------------------------------------------------------

    #[test]
    fn env_value_reads_set_variable() {
        std::env::set_var("SYS_DASEL_TEST_ENV", "hello env");
        let v = parse_env_value("strenv(SYS_DASEL_TEST_ENV)").unwrap().unwrap();
        assert_eq!(v, json!("hello env"));
        // Whitespace around the name is trimmed.
        let v = parse_env_value("strenv( SYS_DASEL_TEST_ENV )").unwrap().unwrap();
        assert_eq!(v, json!("hello env"));
        std::env::remove_var("SYS_DASEL_TEST_ENV");
    }

    #[test]
    fn env_value_missing_or_bad_name_errors() {
        let err = parse_env_value("strenv(SYS_DASEL_TEST_ENV_NOT_SET)")
            .unwrap()
            .unwrap_err();
        assert!(format!("{err:#}").contains("is not set"), "got: {err:#}");
        let err = parse_env_value("strenv()").unwrap().unwrap_err();
        assert!(
            format!("{err:#}").contains("needs an environment variable name"),
            "got: {err:#}"
        );
        let err = parse_env_value("strenv(a(b))").unwrap().unwrap_err();
        assert!(format!("{err:#}").contains("invalid `strenv"), "got: {err:#}");
    }

    #[test]
    fn env_value_ignores_other_rhs_forms() {
        assert!(parse_env_value("42").is_none());
        assert!(parse_env_value("@file.json").is_none());
        assert!(parse_env_value(r#""strenv(X)""#).is_none());
        assert!(parse_env_value("strenv(ROOT").is_none()); // unbalanced -> literal
        assert!(parse_env_value("x").is_none());
        assert!(parse_env_value("").is_none());
    }
}
