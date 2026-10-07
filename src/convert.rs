//! `sysenv con` — JSON / CSV / Markdown / YAML conversion.
//!
//! Usage:
//!   cat a.json | sysenv con                        # detect input format, print parsed (defaults to the input format)
//!   sysenv con -file a.json -o csv                 # read file, convert to CSV (stdout)
//!   sysenv con -i csv -o json < a.csv              # CSV -> JSON
//!   sysenv con -file a.yaml -o md -out out.md      # YAML -> Markdown, write to out.md
//!
//! Input is read from stdin unless `-file` is given; the input format is
//! auto-detected unless `-i` is given; the output format defaults to the
//! input format unless `-o` is given. The result goes to stdout unless
//! `-out FILE` is given.
//!
//! Tables (CSV / Markdown) map to/from arrays of objects.

use anyhow::{Context, Result, bail};
use serde_json::{Map, Number, Value};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

/// Supported formats.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Json,
    Csv,
    Md,
    Yaml,
}

impl Format {
    fn parse(s: &str) -> Result<Format> {
        match s.to_ascii_lowercase().as_str() {
            "json" => Ok(Format::Json),
            "csv" => Ok(Format::Csv),
            "md" | "markdown" => Ok(Format::Md),
            "yaml" | "yml" => Ok(Format::Yaml),
            other => bail!("unknown format `{other}` (expected: json | csv | md | yaml)"),
        }
    }

    fn from_ext(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "json" => Some(Format::Json),
            "csv" => Some(Format::Csv),
            "md" | "markdown" => Some(Format::Md),
            "yaml" | "yml" => Some(Format::Yaml),
            _ => None,
        }
    }
}

/// Best-effort detection of the input format from the content.
fn detect_format(text: &str) -> Result<Format> {
    let t = text.trim_start();
    if t.is_empty() {
        bail!("empty input; cannot detect the format (use -i json|csv|md|yaml)");
    }
    if t.starts_with('{') || t.starts_with('[') {
        if serde_json::from_str::<Value>(text).is_ok() {
            return Ok(Format::Json);
        }
        // JSON syntax is a YAML subset: a non-JSON `{`/`[` may still be YAML.
        if serde_yaml::from_str::<Value>(text).is_ok() {
            return Ok(Format::Yaml);
        }
        bail!("input starts like JSON/YAML but cannot be parsed (use -i json|yaml)");
    }
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    // Markdown table: `| a | b |` followed by a `|---|` separator row.
    if lines.len() >= 2 && lines[0].starts_with('|') {
        let sep: String = lines[1].chars().filter(|&c| c != '|' && c != ' ').collect();
        if !sep.is_empty() && sep.chars().all(|c| c == '-' || c == ':') {
            return Ok(Format::Md);
        }
    }
    // CSV: a comma appears in the first non-empty line.
    if lines.first().map(|l| l.contains(',')).unwrap_or(false) {
        return Ok(Format::Csv);
    }
    // YAML: try a real parse (key: value / list items).
    if serde_yaml::from_str::<Value>(text).is_ok() {
        return Ok(Format::Yaml);
    }
    // Fallback: single-column CSV (no commas).
    if lines.len() > 1 {
        return Ok(Format::Csv);
    }
    bail!("cannot detect the input format (use -i json|csv|md|yaml)")
}

// ---------------------------------------------------------------------------
// Parsing (text -> Value)
// ---------------------------------------------------------------------------

fn parse(text: &str, fmt: Format) -> Result<Value> {
    match fmt {
        Format::Json => serde_json::from_str(text).context("invalid JSON input"),
        Format::Yaml => parse_yaml_value(text),
        Format::Csv => csv_to_value(text),
        Format::Md => md_to_value(text),
    }
}

/// Parse YAML: strict serde_yaml first, then fall back to the project's
/// tab-tolerant block parser (real configs use tab indentation).
fn parse_yaml_value(text: &str) -> Result<Value> {
    match serde_yaml::from_str::<Value>(text) {
        Ok(v) => Ok(v),
        Err(_) => {
            let y = crate::chat::parse_yaml(text).context("invalid YAML input")?;
            Ok(yval_to_json(&y))
        }
    }
}

/// Convert the project YAML value model to a JSON value.
fn yval_to_json(v: &crate::chat::YVal) -> Value {
    match v {
        crate::chat::YVal::Scalar(s) => cell_to_value(s.clone()),
        crate::chat::YVal::List(l) => Value::Array(l.iter().map(yval_to_json).collect()),
        crate::chat::YVal::Map(m) => {
            let mut obj = Map::new();
            for (k, val) in m {
                obj.insert(k.clone(), yval_to_json(val));
            }
            Value::Object(obj)
        }
    }
}

/// Parse RFC-4180-ish CSV into rows of fields (quoted fields, "" escapes,
/// commas and newlines inside quotes, CRLF tolerated).
fn parse_csv_rows(text: &str) -> Result<Vec<Vec<String>>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut in_quotes = false;
    while let Some(c) = chars.next() {
        match c {
            '"' if !in_quotes => in_quotes = true,
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            }
            ',' if !in_quotes => {
                row.push(std::mem::take(&mut field));
            }
            '\n' if !in_quotes => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            '\r' if !in_quotes => {}
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    // Drop blank rows (a single empty field).
    rows.retain(|r| !(r.len() == 1 && r[0].is_empty()));
    Ok(rows)
}

/// Type-infer a CSV / Markdown cell ("", null, bool, int, float, else string).
fn cell_to_value(s: String) -> Value {
    match s.trim() {
        "" => Value::String(s),
        "null" | "~" => Value::Null,
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        t => {
            if let Ok(n) = t.parse::<i64>() {
                Value::Number(n.into())
            } else if let Ok(f) = t.parse::<f64>() {
                Number::from_f64(f).map(Value::Number).unwrap_or(Value::String(s))
            } else {
                Value::String(s)
            }
        }
    }
}

/// CSV rows -> array of objects (first row = header).
fn csv_to_value(text: &str) -> Result<Value> {
    let rows = parse_csv_rows(text)?;
    if rows.is_empty() {
        return Ok(Value::Array(Vec::new()));
    }
    let header = &rows[0];
    if header.iter().all(|h| h.is_empty()) {
        bail!("CSV header row is empty");
    }
    let arr: Vec<Value> = rows[1..]
        .iter()
        .map(|row| {
            let mut obj = Map::new();
            for (i, h) in header.iter().enumerate() {
                obj.insert(h.clone(), cell_to_value(row.get(i).cloned().unwrap_or_default()));
            }
            Value::Object(obj)
        })
        .collect();
    Ok(Value::Array(arr))
}

/// Markdown table -> array of objects (first table wins).
fn md_to_value(text: &str) -> Result<Value> {
    let lines: Vec<&str> = text.lines().collect();
    let mut idx = 0;
    while idx + 1 < lines.len() {
        let header_line = lines[idx].trim();
        if header_line.starts_with('|') {
            let sep = lines[idx + 1].trim();
            let sep_body: String = sep.chars().filter(|&c| c != '|' && c != ' ').collect();
            let is_sep = !sep_body.is_empty() && sep_body.chars().all(|c| c == '-' || c == ':');
            if is_sep {
                let header = parse_md_row(header_line)?;
                if header.iter().all(|h| h.is_empty()) {
                    bail!("Markdown table header row is empty");
                }
                let mut arr = Vec::new();
                for j in idx + 2..lines.len() {
                    let r = lines[j].trim();
                    if r.is_empty() {
                        continue;
                    }
                    if !r.starts_with('|') {
                        break;
                    }
                    let cells = parse_md_row(r)?;
                    let mut obj = Map::new();
                    for (i, h) in header.iter().enumerate() {
                        obj.insert(h.clone(), cell_to_value(cells.get(i).cloned().unwrap_or_default()));
                    }
                    arr.push(Value::Object(obj));
                }
                return Ok(Value::Array(arr));
            }
        }
        idx += 1;
    }
    bail!("no Markdown table found in the input")
}

fn parse_md_row(line: &str) -> Result<Vec<String>> {
    let t = line.trim();
    let inner = t
        .strip_prefix('|')
        .and_then(|s| s.strip_suffix('|'))
        .unwrap_or(t);
    // Scan char by char so an escaped `\|` is kept as a literal `|`, not a separator.
    let mut cells: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                cur.push('|');
            }
            '|' => {
                cells.push(cur.trim().to_string());
                cur.clear();
            }
            c => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    Ok(cells)
}

// ---------------------------------------------------------------------------
// Serialization (Value -> text)
// ---------------------------------------------------------------------------

fn serialize(v: &Value, fmt: Format) -> Result<String> {
    match fmt {
        Format::Json => Ok(format!("{}\n", serde_json::to_string_pretty(v)?)),
        Format::Yaml => {
            let s = serde_yaml::to_string(v).context("cannot serialize YAML")?;
            // Strip the leading "---\n" document marker for a cleaner output.
            Ok(s.strip_prefix("---\n").unwrap_or(&s).to_string())
        }
        Format::Csv => value_to_csv(v),
        Format::Md => value_to_md(v),
    }
}

/// Object keys of an array (union in first-seen order).
fn collect_keys(v: &Value) -> Result<Vec<String>> {
    let arr = match v {
        Value::Array(a) => a,
        Value::Object(o) => {
            let keys: Vec<String> = o.keys().cloned().collect();
            if keys.is_empty() {
                bail!("table output requires objects with keys");
            }
            return Ok(keys);
        }
        _ => bail!("table output requires an array of objects (or a single object)"),
    };
    let mut keys: Vec<String> = Vec::new();
    for item in arr {
        let o = item
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("table rows must be objects"))?;
        for k in o.keys() {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
    }
    if keys.is_empty() {
        bail!("table output requires objects with keys");
    }
    Ok(keys)
}

fn cell_from_value(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn value_to_csv(v: &Value) -> Result<String> {
    let keys = collect_keys(v)?;
    let mut out = String::new();
    out.push_str(&keys.iter().map(|k| csv_escape(k)).collect::<Vec<_>>().join(","));
    out.push('\n');
    match v {
        Value::Array(arr) => {
            for item in arr {
                let o = item.as_object().expect("rows checked to be objects");
                let cells: Vec<String> = keys
                    .iter()
                    .map(|k| csv_escape(&cell_from_value(o.get(k))))
                    .collect();
                out.push_str(&cells.join(","));
                out.push('\n');
            }
        }
        Value::Object(o) => {
            let cells: Vec<String> = keys
                .iter()
                .map(|k| csv_escape(&cell_from_value(o.get(k))))
                .collect();
            out.push_str(&cells.join(","));
            out.push('\n');
        }
        _ => unreachable!("collect_keys rejects non-object input"),
    }
    Ok(out)
}

fn md_escape(s: &str) -> String {
    s.replace('|', "\\|")
}

fn value_to_md(v: &Value) -> Result<String> {
    let keys = collect_keys(v)?;
    let mut out = String::new();
    out.push_str(&format!("| {} |\n", keys.iter().map(|k| md_escape(k)).collect::<Vec<_>>().join(" | ")));
    out.push_str(&format!("| {} |\n", keys.iter().map(|_| "---").collect::<Vec<_>>().join(" | ")));
    match v {
        Value::Array(arr) => {
            for item in arr {
                let o = item.as_object().expect("rows checked to be objects");
                let cells: Vec<String> = keys
                    .iter()
                    .map(|k| md_escape(&cell_from_value(o.get(k))))
                    .collect();
                out.push_str(&format!("| {} |\n", cells.join(" | ")));
            }
        }
        Value::Object(o) => {
            let cells: Vec<String> = keys
                .iter()
                .map(|k| md_escape(&cell_from_value(o.get(k))))
                .collect();
            out.push_str(&format!("| {} |\n", cells.join(" | ")));
        }
        _ => unreachable!("collect_keys rejects non-object input"),
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Parse the raw token list into options.
///
/// Accepts single-dash long flags (`-file`, `-out`) as the user writes them,
/// plus their double-dash / short forms:
///   -file F | --file F | -f F
///   -i FMT | --input FMT
///   -o FMT | --output FMT
///   -out F | --out F
fn parse_args(raw: &[String]) -> Result<(Option<String>, Option<String>, Option<String>, Option<String>)> {
    let mut file: Option<String> = None;
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut out: Option<String> = None;
    let mut i = 0;
    let need_value = |i: &mut usize, what: &str, raw: &[String]| -> Result<String> {
        *i += 1;
        raw.get(*i)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("{what} requires a value"))
    };
    while i < raw.len() {
        let t = raw[i].as_str();
        match t {
            "-file" | "--file" | "-f" => file = Some(need_value(&mut i, "-file", raw)?),
            "-i" | "--input" => input = Some(need_value(&mut i, "-i", raw)?),
            "-o" | "--output" => output = Some(need_value(&mut i, "-o", raw)?),
            "-out" | "--out" => out = Some(need_value(&mut i, "-out", raw)?),
            "-h" | "--help" => {
                print!("{}", CON_HELP);
                return Ok((None, None, None, None));
            }
            other => bail!("unknown argument `{other}` (expected -file F | -i FMT | -o FMT | -out F)"),
        }
        i += 1;
    }
    Ok((file, input, output, out))
}

const CON_HELP: &str = "Convert between json / csv / md / yaml.

Usage:
  cat a.json | sysenv con                    # stdin, auto-detect input, print (defaults to the input format)
  sysenv con -file a.json -o csv             # read a file, convert to csv (stdout)
  sysenv con -i csv -o json                  # csv from stdin -> json
  sysenv con -file a.yaml -o md -out out.md  # yaml -> markdown, write to out.md

Options:
  -file F     read from F instead of stdin
  -i FMT      input format: json | csv | md | yaml (auto-detected when omitted)
  -o FMT      output format: json | csv | md | yaml (defaults to the input format)
  -out F      write the result to F instead of stdout

Tables (csv / md) map to/from arrays of objects.
";

pub fn cmd_con(raw: &[String]) -> Result<()> {
    let (file, input, output, out) = parse_args(raw)?;
    let file = file.unwrap_or_default();
    // 1. Read the input.
    let text = if !file.is_empty() {
        fs::read_to_string(&file).with_context(|| format!("cannot read {}", file))?
    } else {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).context("cannot read stdin")?;
        buf
    };

    // 2. Input format: -i, else file extension, else content detection.
    let in_fmt = match input {
        Some(s) => Format::parse(&s)?,
        None => {
            let p = Path::new(&file);
            match Format::from_ext(p) {
                Some(f) => f,
                None => detect_format(&text)?,
            }
        }
    };

    // 3. Parse.
    let v = parse(&text, in_fmt)?;

    // 4. Output format: -o, else the input format.
    let out_fmt = match output {
        Some(s) => Format::parse(&s)?,
        None => in_fmt,
    };

    // 5. Serialize.
    let result = serialize(&v, out_fmt)?;

    // 6. Emit: -out file, else stdout.
    match out {
        Some(path) => fs::write(&path, result).with_context(|| format!("cannot write {}", path))?,
        None => {
            use std::io::Write;
            let stdout = io::stdout();
            let mut h = stdout.lock();
            h.write_all(result.as_bytes())?;
            h.flush()?;
        }
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
    fn format_parse_and_ext() {
        assert_eq!(Format::parse("json").unwrap(), Format::Json);
        assert_eq!(Format::parse("CSV").unwrap(), Format::Csv);
        assert_eq!(Format::parse("markdown").unwrap(), Format::Md);
        assert_eq!(Format::parse("yml").unwrap(), Format::Yaml);
        assert!(Format::parse("xml").is_err());
        assert_eq!(Format::from_ext(Path::new("a.json")), Some(Format::Json));
        assert_eq!(Format::from_ext(Path::new("a.md")), Some(Format::Md));
        assert_eq!(Format::from_ext(Path::new("a.txt")), None);
    }

    #[test]
    fn csv_rows_quotes_and_commas() {
        let rows = parse_csv_rows("a,b,c\n\"x,y\",\"he said \"\"hi\"\"\",z\n").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["a", "b", "c"]);
        assert_eq!(rows[1], vec!["x,y", "he said \"hi\"", "z"]);
        // CRLF
        let rows2 = parse_csv_rows("a,b\r\n1,2\r\n").unwrap();
        assert_eq!(rows2[1], vec!["1", "2"]);
    }

    #[test]
    fn csv_to_value_types() {
        let v = csv_to_value("name,age,score,active\nAlice,30,9.5,true\nBob,,\n").unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0]["name"], "Alice");
        assert_eq!(arr[0]["age"], 30);
        assert_eq!(arr[0]["score"], 9.5);
        assert_eq!(arr[0]["active"], true);
        assert_eq!(arr[1]["age"], Value::String(String::new()));
    }

    #[test]
    fn json_csv_roundtrip() {
        let j: Value = serde_json::from_str(
            r#"[{"name":"Alice","age":30},{"name":"Bob","age":25}]"#,
        )
        .unwrap();
        let csv = value_to_csv(&j).unwrap();
        assert_eq!(csv, "name,age\nAlice,30\nBob,25\n");
        let back = csv_to_value(&csv).unwrap();
        assert_eq!(back, j);
    }

    #[test]
    fn json_md_roundtrip() {
        let j: Value = serde_json::from_str(r#"[{"a":1,"b":"x"},{"a":2,"b":"y|z"}]"#).unwrap();
        let md = value_to_md(&j).unwrap();
        assert_eq!(md, "| a | b |\n| --- | --- |\n| 1 | x |\n| 2 | y\\|z |\n");
        let back = md_to_value(&md).unwrap();
        assert_eq!(back, j);
    }

    #[test]
    fn md_to_value_skips_surrounding_text() {
        let md = "intro text\n| name | v |\n| --- | --- |\n| k | 1 |\n| m | true |\nfooter";
        let v = md_to_value(md).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0]["name"], "k");
        assert_eq!(arr[0]["v"], 1);
        assert_eq!(arr[1]["v"], true);
    }

    #[test]
    fn json_yaml_roundtrip() {
        let j: Value = serde_json::from_str(
            r#"{"name":"sysenv","version":"0.4.20","features":["con","file"],"nested":{"ok":true}}"#,
        )
        .unwrap();
        let yaml = serialize(&j, Format::Yaml).unwrap();
        assert!(yaml.contains("name: sysenv"));
        assert!(!yaml.starts_with("---"));
        let back = serde_yaml::from_str::<Value>(&yaml).unwrap();
        assert_eq!(back, j);
    }

    #[test]
    fn detect_common_formats() {
        assert_eq!(detect_format("{\"a\":1}").unwrap(), Format::Json);
        assert_eq!(detect_format("[1,2]").unwrap(), Format::Json);
        assert_eq!(detect_format("a,b\n1,2").unwrap(), Format::Csv);
        assert_eq!(detect_format("| a | b |\n| --- | --- |\n| 1 | 2 |").unwrap(), Format::Md);
        assert_eq!(detect_format("name: sysenv\nversion: 0.4").unwrap(), Format::Yaml);
        assert!(detect_format("").is_err());
    }

    #[test]
    fn yaml_tab_indentation_fallback() {
        // Real-world configs use tab indentation; serde_yaml rejects it, the
        // project parser must handle it (tabs expand to spaces).
        let yaml = "providers:\n  - name: a\n  \tkey: 1\n  \tenabled: true\n";
        let v = parse_yaml_value(yaml).unwrap();
        let prov = &v["providers"][0];
        assert_eq!(prov["name"], "a");
        assert_eq!(prov["key"], 1);
        assert_eq!(prov["enabled"], true);
    }

    #[test]
    fn scalar_and_single_object() {
        // 单个对象 → CSV 一行
        let j: Value = serde_json::from_str(r#"{"a":1,"b":2}"#).unwrap();
        assert_eq!(value_to_csv(&j).unwrap(), "a,b\n1,2\n");
        // 标量 → 报错
        assert!(value_to_csv(&Value::from(1)).is_err());
    }
}
