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

/// CSV rows -> array of objects.
///
/// Column headers are virtualised as object keys, mirroring `-i table`:
/// * `header = Some(true)` — the first non-blank row is the column header.
/// * `header = Some(false)` — every row is data; columns are named
///   `col1..colN` (N = widest row).
/// * `header = None` (auto) — the first non-blank row is the header iff it
///   contains at least one non-numeric cell; an all-numeric first row is data.
///
/// Empty header cells fall back to `colN`. Blank rows are skipped; cell
/// values are type-inferred (null / bool / int / float / string).
pub(crate) fn csv_to_value(text: &str, header: Option<bool>) -> Result<Value> {
    let rows = parse_csv_rows(text)?;
    if rows.is_empty() {
        return Ok(Value::Array(Vec::new()));
    }
    let first = &rows[0];
    let use_header = match header {
        Some(h) => h,
        None => first.iter().any(|c| !table_cell_is_numeric(c)),
    };
    let mut arr = Vec::new();
    if use_header {
        let keys: Vec<String> = first
            .iter()
            .enumerate()
            .map(|(i, c)| if c.trim().is_empty() { format!("col{}", i + 1) } else { c.clone() })
            .collect();
        for row in &rows[1..] {
            let mut obj = Map::new();
            for (i, k) in keys.iter().enumerate() {
                obj.insert(k.clone(), cell_to_value(row.get(i).cloned().unwrap_or_default()));
            }
            arr.push(Value::Object(obj));
        }
    } else {
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        let keys: Vec<String> = (1..=width).map(|i| format!("col{i}")).collect();
        for row in &rows {
            let mut obj = Map::new();
            for (i, k) in keys.iter().enumerate() {
                obj.insert(k.clone(), cell_to_value(row.get(i).cloned().unwrap_or_default()));
            }
            arr.push(Value::Object(obj));
        }
    }
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

/// Split a whitespace-separated table line into cells (runs of spaces / tabs).
fn split_table_cells(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

/// A cell counts as numeric when it parses as a plain integer or float; an
/// all-numeric first line is treated as data rather than a header.
fn table_cell_is_numeric(cell: &str) -> bool {
    !cell.is_empty() && (cell.parse::<i64>().is_ok() || cell.parse::<f64>().is_ok())
}

/// `-i table`: parse whitespace-separated table output (like `ps`) into an
/// array of objects.
///
/// * `header = Some(true)` — the first non-blank line is the column header.
/// * `header = Some(false)` — every line is data; columns are named
///   `col1..colN` (N = widest row).
/// * `header = None` (auto) — the first non-blank line is the header iff it
///   contains at least one non-numeric cell; an all-numeric first line is data.
///
/// Rows with more cells than headers append the overflow cells (joined with a
/// single space) to the last column — the common `ps` shape where the COMMAND
/// field may contain spaces. Rows with fewer cells are padded with empty
/// strings. Blank lines are skipped. Cell values are type-inferred exactly
/// like CSV (null / bool / int / float / string).
pub(crate) fn table_to_value(text: &str, header: Option<bool>) -> Result<Value> {
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return Ok(Value::Array(Vec::new()));
    }
    let first = split_table_cells(lines[0]);
    let use_header = match header {
        Some(h) => h,
        None => first.iter().any(|c| !table_cell_is_numeric(c)),
    };
    let mut arr = Vec::new();
    if use_header {
        let keys: Vec<String> = first
            .iter()
            .enumerate()
            .map(|(i, c)| if c.is_empty() { format!("col{}", i + 1) } else { c.clone() })
            .collect();
        for raw in &lines[1..] {
            let mut cells = split_table_cells(raw);
            if cells.len() > keys.len() {
                // Overflow cells (e.g. COMMAND with spaces) join the last column.
                let tail = cells.split_off(keys.len()).join(" ");
                if let Some(last) = cells.last_mut() {
                    last.push(' ');
                    last.push_str(&tail);
                } else {
                    cells.push(tail);
                }
            }
            let mut obj = Map::new();
            for (i, k) in keys.iter().enumerate() {
                obj.insert(k.clone(), cell_to_value(cells.get(i).cloned().unwrap_or_default()));
            }
            arr.push(Value::Object(obj));
        }
    } else {
        let width = lines.iter().map(|l| split_table_cells(l).len()).max().unwrap_or(0);
        let keys: Vec<String> = (1..=width).map(|i| format!("col{i}")).collect();
        for raw in &lines {
            let cells = split_table_cells(raw);
            let mut obj = Map::new();
            for (i, k) in keys.iter().enumerate() {
                obj.insert(k.clone(), cell_to_value(cells.get(i).cloned().unwrap_or_default()));
            }
            arr.push(Value::Object(obj));
        }
    }
    Ok(Value::Array(arr))
}

/// Render one table cell: null / missing -> empty, strings verbatim (embedded
/// newlines JSON-escaped), numbers / bools in their JSON form.
fn table_cell_string(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => {
            if s.contains('\n') || s.contains('\r') {
                serde_json::to_string(s).unwrap_or_else(|_| s.clone())
            } else {
                s.clone()
            }
        }
        Some(other) => other.to_string(),
    }
}

fn table_pad(s: &str, width: usize) -> String {
    let n = width.saturating_sub(s.chars().count());
    format!("{s}{}", " ".repeat(n))
}

/// `-o table`: serialise an array of objects (or a single object, or an array
/// of scalars) into a whitespace-aligned table. Column keys are the object
/// keys in first-appearance order (`col1` for a scalar array); every column is
/// padded to its widest cell and columns are joined with a single space.
pub(crate) fn value_to_table(v: &Value) -> Result<String> {
    let mut keys: Vec<String> = Vec::new();
    let mut matrix: Vec<Vec<Option<&Value>>> = Vec::new();
    match v {
        Value::Object(o) => {
            keys = o.keys().cloned().collect();
            matrix.push(keys.iter().map(|k| o.get(k)).collect());
        }
        Value::Array(arr) if arr.iter().all(|x| x.is_object()) => {
            keys = collect_keys(v)?;
            for item in arr {
                let o = item.as_object().expect("rows checked to be objects");
                matrix.push(keys.iter().map(|k| o.get(k)).collect());
            }
        }
        Value::Array(arr) => {
            keys.push("col1".to_string());
            for item in arr {
                matrix.push(vec![Some(item)]);
            }
        }
        _ => {
            bail!(
                "table output requires an array of objects, an object, or an array of scalars (got {})",
                if v.is_string() { "a string" } else if v.is_number() { "a number" } else if v.is_boolean() { "a boolean" } else if v.is_null() { "null" } else { "an unexpected value" }
            )
        }
    }
    if keys.is_empty() {
        return Ok(String::new());
    }
    let mut widths: Vec<usize> = keys.iter().map(|k| k.chars().count()).collect();
    let mut rows: Vec<Vec<String>> = Vec::new();
    for row in &matrix {
        let mut cells = Vec::with_capacity(row.len());
        for (i, cv) in row.iter().enumerate() {
            let s = table_cell_string(*cv);
            if i < widths.len() {
                widths[i] = widths[i].max(s.chars().count());
            }
            cells.push(s);
        }
        rows.push(cells);
    }
    let mut out = String::new();
    let hdr: Vec<String> = keys.iter().enumerate().map(|(i, k)| table_pad(k, widths[i])).collect();
    out.push_str(&hdr.join(" "));
    out.push('\n');
    for row in &rows {
        let line: Vec<String> = row.iter().enumerate().map(|(i, c)| table_pad(c, widths[i])).collect();
        out.push_str(&line.join(" "));
        out.push('\n');
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
        let v = csv_to_value("name,age,score,active\nAlice,30,9.5,true\nBob,,\n", None).unwrap();
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
        let back = csv_to_value(&csv, None).unwrap();
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

    // --- `-i table` ---------------------------------------------------------

    const PS_EF: &str = "UID          PID    PPID  C STIME TTY          TIME CMD
root            1       0  0 10:14 ?        00:00:02 /sbin/init splash
root          123       1  0 10:15 ?        00:00:00 /usr/sbin/sshd -D
gary         4567    123  0 10:20 pts/0    00:00:00 bash --norc
";

    #[test]
    fn table_auto_header_ps_style() {
        let v = table_to_value(PS_EF, None).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        let row0 = arr[0].as_object().unwrap();
        assert_eq!(row0["UID"], "root");
        assert_eq!(row0["PID"], 1);
        assert_eq!(row0["PPID"], 0);
        assert_eq!(row0["C"], 0);
        assert_eq!(row0["STIME"], "10:14");
        assert_eq!(row0["TTY"], "?");
        assert_eq!(row0["TIME"], "00:00:02");
        // COMMAND 含空格 → 溢出单元格并入最后一列
        assert_eq!(row0["CMD"], "/sbin/init splash");
        let row2 = arr[2].as_object().unwrap();
        assert_eq!(row2["CMD"], "bash --norc");
    }

    #[test]
    fn table_all_numeric_first_line_means_no_header() {
        let v = table_to_value("1 2 3\n4 5 6\n", None).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["col1"], 1);
        assert_eq!(arr[0]["col2"], 2);
        assert_eq!(arr[0]["col3"], 3);
        assert_eq!(arr[1]["col1"], 4);
        assert_eq!(arr[1]["col3"], 6);
    }

    #[test]
    fn table_force_no_header_overrides_text_first_row() {
        let v = table_to_value("alpha beta\ngamma delta\n", Some(false)).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0]["col1"], "alpha");
        assert_eq!(arr[0]["col2"], "beta");
        assert_eq!(arr[1]["col1"], "gamma");
        assert_eq!(arr[1]["col2"], "delta");
    }

    #[test]
    fn table_force_header_overrides_numeric_first_row() {
        let v = table_to_value("1 2\n3 4\n", Some(true)).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0].as_object().unwrap().get("1"), Some(&Value::from(3)));
        assert_eq!(arr[0].as_object().unwrap().get("2"), Some(&Value::from(4)));
    }

    #[test]
    fn csv_auto_header_uses_text_first_row() {
        let v = csv_to_value("name,age\nAlice,30\nBob,25\n", None).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0]["name"], "Alice");
        assert_eq!(arr[0]["age"], 30);
        assert_eq!(arr[1]["name"], "Bob");
        // 空表头单元格回退为 colN。
        let v = csv_to_value(",age\nAlice,30\n", None).unwrap();
        assert_eq!(v[0]["col1"], "Alice");
        assert_eq!(v[0]["age"], 30);
    }

    #[test]
    fn csv_all_numeric_first_line_means_no_header() {
        let v = csv_to_value("1,2,3\n4,5,6\n", None).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["col1"], 1);
        assert_eq!(arr[0]["col2"], 2);
        assert_eq!(arr[0]["col3"], 3);
        assert_eq!(arr[1]["col1"], 4);
        assert_eq!(arr[1]["col3"], 6);
    }

    #[test]
    fn csv_force_no_header_overrides_text_first_row() {
        let v = csv_to_value("alpha,beta\ngamma,delta\n", Some(false)).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0]["col1"], "alpha");
        assert_eq!(arr[0]["col2"], "beta");
        assert_eq!(arr[1]["col1"], "gamma");
    }

    #[test]
    fn csv_force_header_overrides_numeric_first_row() {
        let v = csv_to_value("1,2\n3,4\n", Some(true)).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0].as_object().unwrap().get("1"), Some(&Value::from(3)));
        assert_eq!(arr[0].as_object().unwrap().get("2"), Some(&Value::from(4)));
    }

    #[test]
    fn table_ragged_rows_pad_missing_cells() {
        let v = table_to_value("A B C\n1 2\n3 4 5 6\n", None).unwrap();
        let arr = v.as_array().unwrap();
        let o = arr[0].as_object().unwrap();
        assert_eq!(o["A"], 1);
        assert_eq!(o["B"], 2);
        assert_eq!(o["C"], ""); // 缺失列补空串
        let o = arr[1].as_object().unwrap();
        assert_eq!(o["A"], 3);
        assert_eq!(o["C"], "5 6"); // 溢出并入最后一列
    }

    #[test]
    fn table_skips_blank_lines_and_crlf() {
        let v = table_to_value("H1 H2\r\n\r\n1 2\r\n  \r\n3 4\r\n", None).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["H1"], 1);
        assert_eq!(arr[1]["H2"], 4);
    }

    #[test]
    fn table_types_null_bool_float() {
        let v = table_to_value("k v\nnull 1.5\ntrue -2\n", None).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0]["k"], Value::Null);
        assert_eq!(arr[0]["v"], 1.5);
        assert_eq!(arr[1]["k"], true);
        assert_eq!(arr[1]["v"], -2);
    }

    #[test]
    fn table_empty_input_is_empty_array() {
        let v = table_to_value("  \n\n", None).unwrap();
        assert_eq!(v, Value::Array(Vec::new()));
    }

    // --- `-o table` ---------------------------------------------------------

    #[test]
    fn value_to_table_object_array_aligned() {
        let j: Value = serde_json::from_str(r#"[{"name":"Alice","age":30},{"name":"Bob","age":5}]"#).unwrap();
        let t = value_to_table(&j).unwrap();
        assert_eq!(t, "name  age\nAlice 30 \nBob   5  \n");
    }

    #[test]
    fn value_to_table_single_object_and_scalars() {
        let j: Value = serde_json::from_str(r#"{"a":1,"b":"x"}"#).unwrap();
        assert_eq!(value_to_table(&j).unwrap(), "a b\n1 x\n");
        let scalars: Value = serde_json::from_str(r#"[1,2]"#).unwrap();
        assert_eq!(value_to_table(&scalars).unwrap(), "col1\n1   \n2   \n");
        assert!(value_to_table(&Value::from(1)).is_err());
    }

    #[test]
    fn value_to_table_null_and_newline_cells() {
        let j: Value = serde_json::from_str(r#"[{"a":null,"b":"line1\nline2"}]"#).unwrap();
        let t = value_to_table(&j).unwrap();
        assert_eq!(t, "a b             \n  \"line1\\nline2\"\n");
    }

    #[test]
    fn table_roundtrip_preserves_rows() {
        let src = "name  age\nAlice 30 \nBob   25 \n";
        let v = table_to_value(src, None).unwrap();
        let back = value_to_table(&v).unwrap();
        assert_eq!(back, src);
    }
}
