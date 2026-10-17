//! Shared structured-data conversion helpers (used by `sys data` / `dasel`).
//!
//! Parsing / serialization between text and `serde_json::Value` for JSON /
//! CSV / Markdown / YAML. The standalone `sys con` command was removed in
//! v0.5.10 — `sys data` covers the same conversions (plus TOML) with
//! `-i` / `-o` / `-f` / `-w`.

use anyhow::{Result, bail};
use serde_json::{Map, Number, Value};

// ---------------------------------------------------------------------------
// Parsing (text -> Value)
// ---------------------------------------------------------------------------

/// Convert the project YAML value model to a JSON value.
pub(crate) fn yval_to_json(v: &crate::chat::YVal) -> Value {
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
pub(crate) fn parse_csv_rows(text: &str) -> Result<Vec<Vec<String>>> {
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
pub(crate) fn cell_to_value(s: String) -> Value {
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
pub(crate) fn csv_to_value(text: &str) -> Result<Value> {
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
pub(crate) fn md_to_value(text: &str) -> Result<Value> {
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

pub(crate) fn parse_md_row(line: &str) -> Result<Vec<String>> {
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

/// Object keys of an array (union in first-seen order).
pub(crate) fn collect_keys(v: &Value) -> Result<Vec<String>> {
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

pub(crate) fn cell_from_value(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

pub(crate) fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub(crate) fn value_to_csv(v: &Value) -> Result<String> {
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

pub(crate) fn md_escape(s: &str) -> String {
    s.replace('|', "\\|")
}

pub(crate) fn value_to_md(v: &Value) -> Result<String> {
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
    fn scalar_and_single_object() {
        // 单个对象 → CSV 一行
        let j: Value = serde_json::from_str(r#"{"a":1,"b":2}"#).unwrap();
        assert_eq!(value_to_csv(&j).unwrap(), "a,b\n1,2\n");
        // 标量 → 报错
        assert!(value_to_csv(&Value::from(1)).is_err());
    }
}
