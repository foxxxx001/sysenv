//! `sys data` (alias `dasel`) — query, modify and convert structured data
//! (JSON / YAML / TOML / CSV), a Rust port of the dasel core workflow.
//!
//! Usage:
//!   sys data -i yaml 'a.b[0].name'                     # read a value
//!   sys data -f cfg.yaml -w 'server.port = 8080'       # modify + write back
//!   sys data -i json -o yaml '{}'                      # format conversion
//!   echo '{"a":{"b":"你好"}}' | sys data 'a.b'          # read from stdin
//!   sys data --root 'a.b = "x"' < in.json              # modify, print whole doc
//!
//! Selector syntax (dasel-compatible subset):
//!   .a.b            dotted keys (a leading `.` is optional)
//!   list[0]         array index (negative counts from the end)
//!   list[-1]        last element
//!   list[*]         every element
//!   m["中文名"]      quoted key (spaces / dots / CJK)
//!
//! Expressions:
//!   PATH            print the value at PATH
//!   PATH = VALUE    set (JSON literals or plain text; `= null` deletes)
//!   PATH += VALUE / -= / *= / /=    numeric (or string append for +=)
//!   list.filter(predicate)          keep matching array elements
//!   list.remove(predicate)          delete matching array elements
//!   search(predicate)               recursively collect matching nodes
//!   name.endsWith("x")              string / key functions
//!
//! Predicate language (dasel-compatible): `$this` / `$key`, field paths
//! (`name`, `a.b[0]`), literals, `== != > >= < <= && || !`, and
//! `contains("x")` / `startsWith("x")` / `endsWith("x")` / `indexOf("x")` /
//! `has("key")`. Example: `sys data 'sites.filter(type==1)'`.
//!
//! `--depth N` (default 1): delete every node deeper than N levels from the
//! whole document (root = level 0; levels 0..=N are kept), e.g.
//! `sys data --depth 1 -f cfg.yaml` keeps the top-level keys only.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::io::{self, Read};
use std::path::Path;


// ---------------------------------------------------------------------------
// Formats
// ---------------------------------------------------------------------------

/// Formats supported by `sys data` (superset of the removed `sys con`: + TOML).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFormat {
    Json,
    Yaml,
    Toml,
    Csv,
    Md,
}

fn parse_format(s: &str) -> Result<DataFormat> {
    match s.to_ascii_lowercase().as_str() {
        "json" => Ok(DataFormat::Json),
        "yaml" | "yml" => Ok(DataFormat::Yaml),
        "toml" => Ok(DataFormat::Toml),
        "csv" => Ok(DataFormat::Csv),
        "md" | "markdown" => Ok(DataFormat::Md),
        other => bail!("unknown format `{other}` (expected: json | yaml | toml | csv | md)"),
    }
}

fn format_from_ext(path: &Path) -> Option<DataFormat> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "json" => Some(DataFormat::Json),
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
pub fn parse_input(text: &str, fmt: DataFormat) -> Result<Value> {
    match fmt {
        DataFormat::Json => serde_json::from_str(text).context("invalid JSON input"),
        DataFormat::Yaml => parse_yaml_tolerant(text),
        DataFormat::Toml => {
            toml::from_str(text).context("invalid TOML input")
        }
        DataFormat::Csv => crate::convert::csv_to_value(text),
        DataFormat::Md => crate::convert::md_to_value(text),
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
    }
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
                '[' => bail!("missing key before `[` at position {i} in `{sel}`"),
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
        Value::String(s) if matches!(fmt, DataFormat::Json | DataFormat::Yaml) && !s.contains('\n') => {
            // dasel prints scalar strings unquoted.
            Ok(format!("{s}\n"))
        }
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
    // Find the operator outside quotes.
    let chars: Vec<char> = expr.chars().collect();
    let mut in_quote: Option<char> = None;
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
    if !FUNCTION_NAMES.contains(&name.as_str()) {
        return Some(Err(anyhow::anyhow!(
            "unknown function `{name}` (expected: filter | search | remove | contains | startsWith | endsWith | indexOf | has)"
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
        _ => bail!(
            "`{fn_name}` path `{path}` matched {} values (expected exactly one)",
            got.len()
        ),
    }
}

/// `path.filter(predicate)`: keep the matching array elements (read-only).
fn apply_filter(v: &Value, call: &FunctionCall, out_f: DataFormat, compact: bool) -> Result<String> {
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
    serialize_output(&Value::Array(kept), out_f, compact)
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
/// node matching the predicate ($this = the node, $key = its key / index).
fn apply_search(v: &Value, call: &FunctionCall, out_f: DataFormat, compact: bool) -> Result<String> {
    let start = match &call.path {
        Some(p) => resolve_single(v, p, "search")?,
        None => v,
    };
    let mut found = Vec::new();
    search_collect(start, None, &call.args, &mut found)?;
    serialize_output(&Value::Array(found), out_f, compact)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataOpts {
    pub input: Option<DataFormat>,
    pub output: Option<DataFormat>,
    pub compact: bool,
    pub root: bool,
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
) -> Result<()> {
    if file.is_some() && url.is_some() {
        bail!("-f/--file and -u/--url cannot be combined");
    }
    // 1. Read the input: -f FILE, else -u URL (GET), else stdin.
    let (text, source_fmt) = match (file, url) {
        (Some(p), _) => {
            let t = std::fs::read_to_string(p).with_context(|| format!("cannot read {p}"))?;
            (t, format_from_ext(Path::new(p)))
        }
        (None, Some(u)) => (fetch_url(u)?, Some(DataFormat::Json)),
        (None, None) => {
            // No stdin piped and no expression -> nothing to do.
            if is_stdin_tty() && expr.is_none() {
                bail!("nothing to do: pass an expression, or pipe input via stdin / use -f FILE / -u URL");
            }
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf).context("cannot read stdin")?;
            (buf, None)
        }
    };
    if text.trim().is_empty() {
        bail!("empty input");
    }

    // 2. Input format: -i wins; else -f extension (file) / json (URL);
    // else content detection.
    let in_f = match in_fmt {
        Some(s) => parse_format(s)?,
        None => source_fmt.unwrap_or_else(|| detect_format(&text).expect("detect input format")),
    };
    let mut v = parse_input(&text, in_f)?;

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
    } else if let Some(fc) = parse_function_expr(expr) {
        // Function expression: filter / search / remove / string functions.
        let fc = fc?;
        result_string = match fc.name.as_str() {
            "filter" => apply_filter(&v, &fc, out_f, compact)?,
            "search" => apply_search(&v, &fc, out_f, compact)?,
            "remove" => apply_remove(&mut v, &fc, out_f, compact, root)?,
            _ => apply_string_fn(&v, &fc, out_f, compact)?,
        };
    } else if let Ok((path, op, raw)) = split_expression(expr) {
        // Assignment / delete.
        let steps = parse_selector(&path)?;
        let delete = matches!(parse_value(&raw), Value::Null) && op == "=";
        if delete {
            set_value(&mut v, &steps, Value::Null, true)?;
        } else {
            let rhs = parse_value(&raw);
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
            h.write_all(result_string.as_bytes())?;
            h.flush()?;
        }
    } else {
        use std::io::Write;
        let mut h = io::stdout().lock();
        h.write_all(result_string.as_bytes())?;
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

/// Best-effort format detection (JSON / YAML first, then CSV / MD).
fn detect_format(text: &str) -> Result<DataFormat> {
    let t = text.trim_start();
    if t.starts_with('{') || t.starts_with('[') {
        if serde_json::from_str::<Value>(text).is_ok() {
            return Ok(DataFormat::Json);
        }
        if parse_yaml_tolerant(text).is_ok() {
            return Ok(DataFormat::Yaml);
        }
        bail!("input starts like JSON/YAML but cannot be parsed (use -i json|yaml|toml)");
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
    if lines.first().map(|l| l.contains(',')).unwrap_or(false) && lines.len() >= 2 {
        return Ok(DataFormat::Csv);
    }
    if looks_like_yaml(&text) && parse_yaml_tolerant(text).is_ok() {
        return Ok(DataFormat::Yaml);
    }
    if lines.len() > 1 {
        return Ok(DataFormat::Csv);
    }
    bail!("cannot detect the input format (use -i json|yaml|toml|csv|md)")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        assert!(parse_selector("[0]").is_err());
        assert!(parse_selector("a[x]").is_err());
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
        let v = parse_input("a:\n\tb: 1\n\tc: 中文\n", DataFormat::Yaml).unwrap();
        assert_eq!(v, json!({"a": {"b": 1, "c": "中文"}}));
        // Mixed tabs + spaces.
        let v2 = parse_input("a:\n  b:\n\t\tc: 值\n", DataFormat::Yaml).unwrap();
        assert_eq!(v2, json!({"a": {"b": {"c": "值"}}}));
        // Normal list style keeps working.
        let v3 = parse_input("list:\n  - 苹果\n  - 香蕉\n", DataFormat::Yaml).unwrap();
        assert_eq!(v3, json!({"list": ["苹果", "香蕉"]}));
    }

    #[test]
    fn normalise_tabs_preserves_inline_tabs() {
        let s = normalise_yaml_tabs("a:\n\tb: \"x\ty\"\n");
        assert!(s.starts_with("a:\n    b: \"x\ty\"\n"), "got: {s:?}");
    }

    #[test]
    fn chinese_roundtrip_json_yaml() {
        let src = r#"{"名称":"中文测试","嵌套":{"值":"你好世界"},"列表":["苹果","香蕉"]}"#;
        let v: Value = serde_json::from_str(src).unwrap();
        // JSON -> YAML -> JSON keeps CJK text intact.
        let y = serialize_output(&v, DataFormat::Yaml, false).unwrap();
        assert!(y.contains("中文测试"), "yaml: {y}");
        let back = parse_input(&y, DataFormat::Yaml).unwrap();
        assert_eq!(v, back);
        // YAML -> JSON keeps CJK text intact.
        let j = serialize_output(&back, DataFormat::Json, false).unwrap();
        assert!(j.contains("中文测试"), "json: {j}");
    }

    #[test]
    fn yaml_multiline_chinese_survives() {
        let src = "a:\n  b: |\n    第一行\n    第二行\n";
        let v = parse_input(src, DataFormat::Yaml).unwrap();
        // `|` keeps the trailing newline (YAML standard).
        assert_eq!(v["a"]["b"], json!("第一行\n第二行\n"));
        let y = serialize_output(&v, DataFormat::Yaml, false).unwrap();
        let back = parse_input(&y, DataFormat::Yaml).unwrap();
        assert_eq!(back["a"]["b"], json!("第一行\n第二行\n"));
        // `|-` strips it.
        let src2 = "a:\n  b: |-\n    第一行\n    第二行\n";
        let v2 = parse_input(src2, DataFormat::Yaml).unwrap();
        assert_eq!(v2["a"]["b"], json!("第一行\n第二行"));
    }

    #[test]
    fn toml_roundtrip_chinese() {
        let src = "name = \"中文\"\n[server]\nport = 8080\n";
        let v = parse_input(src, DataFormat::Toml).unwrap();
        assert_eq!(v["name"], json!("中文"));
        let out = serialize_output(&v, DataFormat::Toml, false).unwrap();
        let back = parse_input(&out, DataFormat::Toml).unwrap();
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
        let v = parse_input("名称,数量\n苹果,3\n香蕉,5\n", DataFormat::Csv).unwrap();
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
    fn render_get_scalar_string() {
        // dasel-style: scalar strings print unquoted.
        let s = render_get(&json!("你好"), DataFormat::Json, false).unwrap();
        assert_eq!(s, "你好\n");
        let s = render_get(&json!(42), DataFormat::Json, false).unwrap();
        assert_eq!(s, "42\n");
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
}
