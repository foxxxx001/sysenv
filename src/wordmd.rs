//! `sys file --word2md` / `--md2word` — Word (.docx) <-> Markdown conversion
//! with image support.
//!
//! Usage:
//!   sys file --word2md SRC.docx [DST.md]     docx -> markdown (DST omitted -> stdout)
//!   sys file --md2word SRC.md [DST.docx]     markdown -> docx (DST required)
//!
//! Images:
//!   docx -> md:  embedded images are extracted to `<DST-dir>/media/` (or
//!                `<SRC-dir>/media/` when DST is stdout) and referenced with
//!                relative paths `![alt](media/image1.png)`
//!   md  -> docx: local paths and http(s) URLs are embedded into the docx
//!                (`word/media/`), magic-byte sniffed (png/jpg/gif/webp/bmp)

use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::path::Path;

// ---------------------------------------------------------------------------
// docx reading (minimal OOXML reader: zip + word/document.xml)
// ---------------------------------------------------------------------------

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
const REL_PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

struct DocxImage {
    /// relationship id (rId)
    rid: String,
    /// original zip path, e.g. word/media/image1.png
    part: String,
    bytes: Vec<u8>,
    ext: String,
}

/// Read `word/document.xml` + relationships + media from a .docx.
fn read_docx(path: &Path) -> Result<(String, Vec<DocxImage>)> {
    let f = std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut zip = zip::ZipArchive::new(f)
        .with_context(|| format!("{} is not a valid docx (zip) file", path.display()))?;
    let mut xml = String::new();
    for i in 0..zip.len() {
        let mut e = zip.by_index(i)?;
        if e.name() == "word/document.xml" {
            e.read_to_string(&mut xml).context("cannot read document.xml")?;
        }
    }
    if xml.is_empty() {
        bail!("{} has no word/document.xml (not a Word document?)", path.display());
    }
    // relationship id -> target part
    let mut rels: Vec<(String, String)> = Vec::new();
    if let Ok(mut e) = zip.by_name("word/_rels/document.xml.rels") {
        let mut s = String::new();
        let _ = e.read_to_string(&mut s);
        rels = parse_relationships(&s);
    }
    // read every media part referenced
    let mut images = Vec::new();
    for (rid, target) in &rels {
        if !target.contains("media/") {
            continue;
        }
        let part = format!("word/{}", target.trim_start_matches('/').trim_start_matches("word/"));
        if let Ok(mut e) = zip.by_name(&part) {
            let mut buf = Vec::new();
            if e.read_to_end(&mut buf).is_ok() && !buf.is_empty() {
                let ext = Path::new(&part)
                    .extension()
                    .and_then(|x| x.to_str())
                    .unwrap_or("bin")
                    .to_ascii_lowercase();
                images.push(DocxImage { rid: rid.clone(), part, bytes: buf, ext });
            }
        }
    }
    Ok((xml, images))
}

/// Parse `.rels` XML into (Id, Target) pairs.
fn parse_relationships(xml: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(p) = rest.find("<Relationship ") {
        let seg = &rest[p..];
        let end = match seg.find(">") {
            Some(e) => e,
            None => break,
        };
        let tag = &seg[..=end];
        let id = xml_attr(tag, "Id").unwrap_or_default();
        let target = xml_attr(tag, "Target").unwrap_or_default();
        if !id.is_empty() && !target.is_empty() {
            out.push((id, target));
        }
        rest = &rest[p + end + 1..];
    }
    out
}

/// Extract an XML attribute value from a single tag string.
fn xml_attr(tag: &str, name: &str) -> Option<String> {
    for pat in [format!("{name}=\""), format!("{name}='")] {
        if let Some(i) = tag.find(&pat) {
            let after = &tag[i + pat.len()..];
            let q = pat.as_bytes()[pat.len() - 1] as char;
            if let Some(j) = after.find(q) {
                return Some(after[..j].to_string());
            }
        }
    }
    None
}

/// Unescape the five XML predefined entities (+ numeric refs).
fn xml_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '&' {
            if let Some(sc) = b[i..].iter().position(|&c| c == ';') {
                let ent: String = b[i + 1..i + sc].iter().collect();
                let rep = match ent.as_str() {
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "amp" => Some('&'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    _ => {
                        if let Some(h) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                            u32::from_str_radix(h, 16).ok().and_then(char::from_u32)
                        } else if let Some(d) = ent.strip_prefix('#') {
                            d.parse::<u32>().ok().and_then(char::from_u32)
                        } else {
                            None
                        }
                    }
                };
                if let Some(c) = rep {
                    out.push(c);
                    i += sc + 1;
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Skip an XML tag: given `<w:r>` return the index after the matching `</w:r>`,
/// handling self-closing `<w:tab/>` (returns after `/>`).
/// `start` points at the `<`. Returns `None` when unbalanced.
fn skip_tag(s: &[char], start: usize, name: &str) -> Option<usize> {
    // find end of the opening tag
    let mut i = start;
    while i < s.len() && s[i] != '>' {
        i += 1;
    }
    if i >= s.len() {
        return None;
    }
    // self-closing?
    if s[i - 1] == '/' {
        return Some(i + 1);
    }
    let close = format!("</{}>", name);
    let clos: Vec<char> = close.chars().collect();
    let mut depth = 1usize;
    let mut j = i + 1;
    while j < s.len() {
        if s[j] == '<' {
            if starts_with_seq(&s[j..], &clos) {
                depth -= 1;
                if depth == 0 {
                    return Some(j + clos.len());
                }
                j += clos.len();
                continue;
            }
            // Scan the whole tag generically.
            let mut k = j + 1;
            let mut nm = String::new();
            let mut is_close = false;
            if k < s.len() && s[k] == '/' {
                is_close = true;
                k += 1;
            }
            while k < s.len() && s[k] != '>' && !s[k].is_whitespace() && s[k] != '/' {
                nm.push(s[k]);
                k += 1;
            }
            let mut selfclose = false;
            while k < s.len() && s[k] != '>' {
                if s[k] == '/' && k + 1 < s.len() && s[k + 1] == '>' {
                    selfclose = true;
                    break;
                }
                k += 1;
            }
            if k >= s.len() {
                return None;
            }
            if !selfclose && nm == name {
                if is_close {
                    depth -= 1;
                } else {
                    depth += 1;
                }
            }
            j = k + 1;
            continue;
        }
        j += 1;
    }
    None
}

fn starts_with_seq(hay: &[char], needle: &[char]) -> bool {
    hay.len() >= needle.len() && hay[..needle.len()] == needle[..]
}

/// The text of one `<w:t>` element (or "" when absent).
fn w_t_text(seg: &[char], open_start: usize) -> Option<String> {
    // seg starts at '<' of <w:t ...>text</w:t>
    let mut i = open_start;
    while i < seg.len() && seg[i] != '>' {
        i += 1;
    }
    if i >= seg.len() {
        return None;
    }
    let text_start = i + 1;
    let close = "</w:t>";
    let clos: Vec<char> = close.chars().collect();
    let mut j = text_start;
    while j < seg.len() {
        if starts_with_seq(&seg[j..], &clos) {
            let t: String = seg[text_start..j].iter().collect();
            return Some(xml_unescape(&t));
        }
        j += 1;
    }
    None
}

/// One paragraph rendered from XML: (markdown text, image rIds in order).
fn render_paragraph(seg: &[char], para_name: &str) -> (String, Vec<String>) {
    // paragraph style
    let style = if para_name.trim_start_matches("w:") == "p" {
        let mut st = String::new();
        if let Some(pps) = find_tag(seg, 0, "w:pPr") {
            let pps_end = skip_tag(seg, pps, "w:pPr").unwrap_or(seg.len());
            let inner: String = seg[pps..pps_end].iter().collect();
            if let Some(i) = inner.find("<w:pStyle") {
                let tag_end = inner[i..].find('>').unwrap_or(0) + i;
                st = xml_attr(&inner[i..=tag_end], "w:val").unwrap_or_default();
            }
            if st.is_empty() && inner.contains("<w:numPr>") {
                st = "ListParagraph".to_string();
            }
        }
        st
    } else {
        para_name.trim_start_matches("w:").to_string()
    };

    let mut md = String::new();
    let mut images: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < seg.len() {
        if seg[i] == '<' {
            // detect tag name
            let mut k = i + 1;
            let mut name = String::new();
            while k < seg.len() && seg[k] != '>' && !seg[k].is_whitespace() && seg[k] != '/' {
                name.push(seg[k]);
                k += 1;
            }
            match name.as_str() {
                "w:r" => {
                    let end = skip_tag(seg, i, "w:r").unwrap_or(seg.len());
                    let rseg = &seg[i..end];
                    let (t, imgs, b, it, tab, br) = render_run(rseg);
                    if !t.is_empty() {
                        let t = if b { format!("**{t}**") } else { t.clone() };
                        let t = if it { format!("*{t}*") } else { t };
                        md.push_str(&t);
                    }
                    images.extend(imgs);
                    if tab {
                        md.push(' ');
                    }
                    if br {
                        md.push_str("  \n");
                    }
                    i = end;
                    continue;
                }
                "w:hyperlink" => {
                    let end = skip_tag(seg, i, "w:hyperlink").unwrap_or(seg.len());
                    let hseg = &seg[i..end];
                    let rid = xml_attr(&hseg.iter().collect::<String>(), "r:id").unwrap_or_default();
                    // extract anchor + inner text
                    let inner = strip_outer_tag(hseg);
                    let mut txt = String::new();
                    let mut j = 0;
                    let hc: Vec<char> = inner.chars().collect();
                    while j < hc.len() {
                        if find_tag(&hc[j..], 0, "w:t").is_some() {
                            let p = find_tag(&hc[j..], 0, "w:t").unwrap();
                            if let Some(t) = w_t_text(&hc[p..], 0) {
                                txt.push_str(&t);
                                let e = skip_tag(&hc[p..], 0, "w:t").unwrap_or(hc.len() - p);
                                j += p + e;
                                continue;
                            }
                        }
                        j += 1;
                    }
                    if !txt.trim().is_empty() {
                        md.push('[');
                        md.push_str(txt.trim());
                        md.push(']');
                        md.push_str(&format!("(#{})", xml_unescape(&xml_attr(&hseg.iter().collect::<String>(), "w:anchor").unwrap_or_default())));
                        let _ = rid;
                    }
                    i = end;
                    continue;
                }
                _ => {}
            }
            i = k.max(i + 1);
        } else {
            i += 1;
        }
    }
    // Strip the manual bullet char written by md2word (the ListParagraph
    // style mapping below adds the markdown "- ").
    let md = md
        .strip_prefix('\u{2022}')
        .map(|t| t.trim_start().to_string())
        .unwrap_or(md);
    // Heading mapping
    let out = match style.as_str() {
        "Heading1" | "1" => format!("# {md}"),
        "Heading2" | "2" => format!("## {md}"),
        "Heading3" | "3" => format!("### {md}"),
        "Heading4" | "4" => format!("#### {md}"),
        "Heading5" | "5" => format!("##### {md}"),
        "Heading6" | "6" => format!("###### {md}"),
        "Title" => format!("# {md}"),
        "ListParagraph" => format!("- {md}"),
        "Quote" => format!("> {md}"),
        "Code" | "HTMLPreformatted" => format!("```\n{md}\n```"),
        _ => md,
    };
    (out, images)
}

fn find_tag(seg: &[char], from: usize, name: &str) -> Option<usize> {
    let pat: Vec<char> = format!("<{name}").chars().collect();
    let mut i = from;
    while i + pat.len() <= seg.len() {
        if starts_with_seq(&seg[i..], &pat) {
            // must not be a longer tag name (w:t vs w:tab)
            let after = i + pat.len();
            if after >= seg.len() || !is_name_char(seg[after]) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == ':' || c == '_' || c == '-'
}

fn strip_outer_tag(seg: &[char]) -> String {
    let mut i = 0;
    while i < seg.len() && seg[i] != '>' {
        i += 1;
    }
    let inner_start = (i + 1).min(seg.len());
    let mut end = seg.len();
    let mut j = seg.len();
    while j > inner_start {
        if seg[j - 1] == '>' {
            // check for </w:hyperlink>
            let mut k = j - 2;
            let mut nm = String::new();
            while k > 0 && seg[k] != '<' && seg[k] != '>' {
                nm.push(seg[k]);
                k -= 1;
            }
            let nm: String = nm.chars().rev().collect();
            if seg[k] == '<' && nm.starts_with('/') {
                end = k;
            }
            break;
        }
        j -= 1;
    }
    seg[inner_start..end].iter().collect()
}

/// One `<w:r>` run: (text, image rIds, bold, italic, tab, line-break).
fn render_run(seg: &[char]) -> (String, Vec<String>, bool, bool, bool, bool) {
    let s = seg.iter().collect::<String>();
    let bold = s.contains("<w:b/>") || s.contains("<w:b ") && !s.contains("<w:b w:val=\"0\"") || s.contains("<w:bCs/>");
    let italic = s.contains("<w:i/>") || s.contains("<w:i ") && !s.contains("<w:i w:val=\"0\"");
    let tab = s.contains("<w:tab/>");
    let br = s.contains("<w:br/>");
    let mut text = String::new();
    let mut images = Vec::new();
    let mut i = 0;
    let sc: Vec<char> = s.chars().collect();
    while let Some(p) = find_tag(&sc, i, "w:t") {
        let end = skip_tag(&sc, p, "w:t").unwrap_or(sc.len());
        if let Some(t) = w_t_text(&sc[p..], 0) {
            text.push_str(&t);
        }
        i = end;
    }
    // images: blip embed rId (a:blip r:embed="rId4")
    if let Some(p) = s.find("r:embed=\"") {
        let after = &s[p + 9..];
        if let Some(q) = after.find('"') {
            images.push(after[..q].to_string());
        }
    }
    (text, images, bold, italic, tab, br)
}

/// Detect tables inside the body and render markdown pipe tables.
/// Walks the body children in order, producing markdown lines.
fn render_body(xml: &str, images: &[DocxImage], media_dir_name: &str) -> Result<String> {
    // isolate <w:body>...</w:body>
    let s: Vec<char> = xml.chars().collect();
    let body_start = match find_tag(&s, 0, "w:body") {
        Some(p) => p,
        None => bail!("document.xml has no <w:body>"),
    };
    let body_end = skip_tag(&s, body_start, "w:body").unwrap_or(s.len());
    let inner: Vec<char> = {
        let bs = body_start;
        let mut i = bs;
        while i < s.len() && s[i] != '>' {
            i += 1;
        }
        let start = i + 1;
        let mut end = body_end;
        let clos: Vec<char> = "</w:body>".chars().collect();
        let mut j = body_end;
        while j >= clos.len() {
            if starts_with_seq(&s[j - clos.len()..j], &clos) {
                end = j - clos.len();
                break;
            }
            j -= 1;
        }
        s[start.min(end)..end].to_vec()
    };

    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < inner.len() {
        if inner[i] != '<' {
            i += 1;
            continue;
        }
        // tag name
        let mut k = i + 1;
        let mut name = String::new();
        while k < inner.len() && inner[k] != '>' && !inner[k].is_whitespace() && inner[k] != '/' {
            name.push(inner[k]);
            k += 1;
        }
        match name.as_str() {
            "w:p" => {
                let end = skip_tag(&inner, i, "w:p").unwrap_or(inner.len());
                let seg = &inner[i..end];
                let (md, rids) = render_paragraph(seg, "w:p");
                // images in this paragraph
                for rid in &rids {
                    if let Some(img) = images.iter().find(|x| &x.rid == rid) {
                        let fname = Path::new(&img.part)
                            .file_name()
                            .and_then(|x| x.to_str())
                            .unwrap_or("image.bin")
                            .to_string();
                        out.push(format!("![{fname}]({media_dir_name}/{fname})"));
                    }
                }
                if md.trim().is_empty() && rids.is_empty() {
                    // blank line between blocks (skip leading)
                    if !out.is_empty() && out.last().map(|l| !l.is_empty()).unwrap_or(false) {
                        out.push(String::new());
                    }
                } else if !md.trim().is_empty() {
                    out.push(md);
                }
                i = end;
            }
            "w:tbl" => {
                let end = skip_tag(&inner, i, "w:tbl").unwrap_or(inner.len());
                let seg = &inner[i..end];
                out.push(render_table(seg));
                i = end;
            }
            "w:sectPr" => {
                i = skip_tag(&inner, i, "w:sectPr").unwrap_or(inner.len());
            }
            _ => {
                i = k.max(i + 1);
            }
        }
    }
    // trim trailing blank lines
    while out.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        out.pop();
    }
    Ok(out.join("\n") + "\n")
}

/// Render a `<w:tbl>` as a markdown pipe table.
fn render_table(seg: &[char]) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut i = 0;
    while let Some(p) = find_tag(seg, i, "w:tr") {
        let end = skip_tag(seg, p, "w:tr").unwrap_or(seg.len());
        let tr = &seg[p..end];
        let mut cells = Vec::new();
        let mut j = 0;
        while let Some(cp) = find_tag(tr, j, "w:tc") {
            let cend = skip_tag(tr, cp, "w:tc").unwrap_or(tr.len());
            let tc = &tr[cp..cend];
            // all paragraph texts joined with <br>
            let mut cell = String::new();
            let mut k = 0;
            while let Some(pp) = find_tag(tc, k, "w:p") {
                let pend = skip_tag(tc, pp, "w:p").unwrap_or(tc.len());
                let (t, _) = render_paragraph(&tc[pp..pend], "w:p");
                if !t.trim().is_empty() {
                    if !cell.is_empty() {
                        cell.push_str("<br>");
                    }
                    cell.push_str(t.trim());
                }
                k = pend;
            }
            cells.push(cell.replace('|', "\\|"));
            j = cend;
        }
        rows.push(cells);
        i = end;
    }
    if rows.is_empty() {
        return String::new();
    }
    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut lines: Vec<String> = Vec::new();
    for (n, r) in rows.iter().enumerate() {
        let mut cells = r.clone();
        cells.resize(width, String::new());
        let line = format!("| {} |", cells.join(" | "));
        lines.push(line);
        if n == 0 {
            lines.push(format!("|{}|", vec![" --- "; width].join("|")));
        }
    }
    lines.join("\n")
}

/// docx -> markdown.
fn word2md(src: &Path, dst: Option<&Path>) -> Result<()> {
    if src.extension().and_then(|e| e.to_str()).map(|e| !e.eq_ignore_ascii_case("docx")).unwrap_or(false) {
        bail!("--word2md expects a .docx file (got {})", src.display());
    }
    let (xml, images) = read_docx(src)?;
    // media output directory: next to DST (or SRC when stdout)
    let base_dir = dst
        .map(|d| d.parent().unwrap_or(Path::new(".")).to_path_buf())
        .unwrap_or_else(|| src.parent().unwrap_or(Path::new(".")).to_path_buf());
    let media_dir = base_dir.join("media");
    if !images.is_empty() {
        std::fs::create_dir_all(&media_dir).with_context(|| format!("cannot create {}", media_dir.display()))?;
        for img in &images {
            let fname = Path::new(&img.part)
                .file_name()
                .and_then(|x| x.to_str())
                .unwrap_or("image.bin");
            std::fs::write(media_dir.join(fname), &img.bytes)
                .with_context(|| format!("cannot write {}", fname))?;
        }
    }
    let md = render_body(&xml, &images, "media")?;
    match dst {
        None => {
            let mut so = std::io::stdout().lock();
            so.write_all(md.as_bytes())?;
            so.flush()?;
        }
        Some(d) => {
            std::fs::write(d, &md).with_context(|| format!("cannot write {}", d.display()))?;
            eprintln!(
                "sys: word2md: {} -> {} ({} images)",
                src.display(),
                d.display(),
                images.len()
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// markdown -> docx
// ---------------------------------------------------------------------------

const CT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;

const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="240" w:after="120"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="240" w:after="120"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:sz w:val="28"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="240" w:after="120"/><w:outlineLvl w:val="2"/></w:pPr><w:rPr><w:b/><w:sz w:val="24"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading4"><w:name w:val="heading 4"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="240" w:after="120"/><w:outlineLvl w:val="3"/></w:pPr><w:rPr><w:b/><w:sz w:val="22"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading5"><w:name w:val="heading 5"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:outlineLvl w:val="4"/></w:pPr><w:rPr><w:b/><w:sz w:val="21"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading6"><w:name w:val="heading 6"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:outlineLvl w:val="5"/></w:pPr><w:rPr><w:b/><w:sz w:val="20"/></w:rPr></w:style><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style><w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:pPr><w:ind w:left="720"/></w:pPr></w:style><w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont"><w:name w:val="Default Paragraph Font"/></w:style></w:styles>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="word/styles.xml"/></Relationships>"#;

fn escape_xml(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '&' => o.push_str("&amp;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            _ => o.push(c),
        }
    }
    o
}

/// Sniff an image format from magic bytes.
fn sniff_img_ext(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if b.starts_with(b"\xff\xd8\xff") {
        Some("jpg")
    } else if b.starts_with(b"GIF8") {
        Some("gif")
    } else if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else if b.starts_with(b"BM") {
        Some("bmp")
    } else {
        None
    }
}

struct DocImage {
    bytes: Vec<u8>,
    ext: String,
    name: String,
    /// EMU width / height (from PNG/JPG header when available)
    cx: i64,
    cy: i64,
}

/// Try to read intrinsic pixel size of png/jpg/gif/bmp for EMU sizing.
fn img_size(b: &[u8], ext: &str) -> (i64, i64) {
    let px = match ext {
        "png" => png_size(b),
        "jpg" => jpg_size(b),
        "gif" => gif_size(b),
        "bmp" => bmp_size(b),
        _ => None,
    };
    match px {
        Some((w, h)) => {
            // 96 DPI -> 9525 EMU per pixel; cap width at 6in (5486400 EMU)
            let mut cx = (w as i64) * 9525;
            let cy = (h as i64) * 9525;
            if cx > 5_486_400 {
                let scale = 5_486_400f64 / cx as f64;
                cx = 5_486_400;
                return (cx, (cy as f64 * scale) as i64);
            }
            (cx, cy)
        }
        None => (3_657_600, 2_743_200), // 4in x 3in fallback
    }
}

fn png_size(b: &[u8]) -> Option<(i64, i64)> {
    if b.len() < 24 {
        return None;
    }
    let w = u32::from_be_bytes(b[16..20].try_into().ok()?) as i64;
    let h = u32::from_be_bytes(b[20..24].try_into().ok()?) as i64;
    Some((w, h))
}

fn gif_size(b: &[u8]) -> Option<(i64, i64)> {
    if b.len() < 10 {
        return None;
    }
    let w = u16::from_le_bytes([b[6], b[7]]) as i64;
    let h = u16::from_le_bytes([b[8], b[9]]) as i64;
    Some((w, h))
}

fn bmp_size(b: &[u8]) -> Option<(i64, i64)> {
    if b.len() < 26 {
        return None;
    }
    let w = i32::from_le_bytes(b[18..22].try_into().ok()?) as i64;
    let h = i32::from_le_bytes(b[22..26].try_into().ok()?).abs() as i64;
    Some((w, h))
}

/// Scan JPEG segments for SOF0/SOF2 dimensions.
fn jpg_size(b: &[u8]) -> Option<(i64, i64)> {
    let mut i = 2usize;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = b[i + 1];
        if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as i64;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as i64;
            return Some((w, h));
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        i += 2 + len;
    }
    None
}

/// A parsed markdown block ready for docx rendering.
enum Block {
    Heading(u8, String),
    Para(String),
    Bullet(String),
    Ordered(String, u32),
    Quote(String),
    Code(String),
    Table(Vec<Vec<String>>),
    Image(String, String), // path-or-url, alt
    Rule,
}

/// Parse markdown text into blocks (block-level only; inline kept as markdown
/// text — bold/italic/code spans are rendered as styled runs).
fn parse_markdown(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0usize;
    let mut para: Vec<String> = Vec::new();
    let mut flush = |para: &mut Vec<String>, blocks: &mut Vec<Block>| {
        if !para.is_empty() {
            blocks.push(Block::Para(para.join(" ")));
            para.clear();
        }
    };
    while i < lines.len() {
        let raw = lines[i];
        let t = raw.trim();
        if t.is_empty() {
            flush(&mut para, &mut blocks);
            i += 1;
            continue;
        }
        if let Some(h) = t.strip_prefix('#') {
            flush(&mut para, &mut blocks);
            let level = h.chars().take_while(|&c| c == '#').count().min(6) as u8;
            let txt = h[level as usize..].trim();
            if !txt.is_empty() {
                blocks.push(Block::Heading(level.max(1), txt.to_string()));
            }
            i += 1;
            continue;
        }
        if t == "---" || t == "***" || t == "___" {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Rule);
            i += 1;
            continue;
        }
        if t.starts_with("```") {
            flush(&mut para, &mut blocks);
            let mut code: Vec<String> = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                code.push(lines[i].to_string());
                i += 1;
            }
            i += 1; // closing fence
            blocks.push(Block::Code(code.join("\n")));
            continue;
        }
        // image line: ![alt](path)
        if let Some((alt, path)) = parse_image_token(t) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Image(path, alt));
            i += 1;
            continue;
        }
        // table: | a | b | then | --- |
        if t.starts_with('|') && i + 1 < lines.len() {
            let sep = lines[i + 1].trim();
            let sep_clean: String = sep.chars().filter(|&c| c != '|' && c != ' ' && c != ':').collect();
            if !sep_clean.is_empty() && sep_clean.chars().all(|c| c == '-') {
                flush(&mut para, &mut blocks);
                let mut rows = Vec::new();
                rows.push(parse_md_row(t));
                i += 2;
                while i < lines.len() {
                    let l = lines[i].trim();
                    if !l.starts_with('|') {
                        break;
                    }
                    rows.push(parse_md_row(l));
                    i += 1;
                }
                blocks.push(Block::Table(rows));
                continue;
            }
        }
        if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Bullet(rest.trim().to_string()));
            i += 1;
            continue;
        }
        if let Some(rest) = t.strip_prefix("> ") {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Quote(rest.trim().to_string()));
            i += 1;
            continue;
        }
        // ordered list "1. "
        let bytes = t.as_bytes();
        if bytes.len() > 2
            && bytes[0].is_ascii_digit()
            && bytes[1] == b'.'
            && bytes[2] == b' '
        {
            flush(&mut para, &mut blocks);
            let n: u32 = t[..1].parse().unwrap_or(1);
            blocks.push(Block::Ordered(t[3..].trim().to_string(), n));
            i += 1;
            continue;
        }
        // continuation of a list item (indented)
        if let Some(Block::Bullet(b)) = blocks.last_mut() {
            if raw.starts_with("  ") && !t.is_empty() {
                b.push(' ');
                b.push_str(t);
                i += 1;
                continue;
            }
        }
        para.push(t.to_string());
        i += 1;
    }
    flush(&mut para, &mut blocks);
    blocks
}

/// Parse `![alt](path "title")` -> (alt, path); the whole trimmed line must be
/// one image token.
fn parse_image_token(t: &str) -> Option<(String, String)> {
    if !t.starts_with("![") {
        return None;
    }
    let close_alt = t.find("](")?;
    let alt = t[2..close_alt].to_string();
    let rest = &t[close_alt + 2..];
    let close = rest.rfind(')')?;
    let mut path = rest[..close].to_string();
    // strip optional title: `url "title"` (title may be unterminated after the cut)
    if let Some(sp) = path.find('"') {
        path.truncate(sp);
    }
    path = path.trim().to_string();
    if path.is_empty() {
        return None;
    }
    // Not an image when the "path" is really inline text with brackets etc.
    if path.contains('[') || path.contains(']') {
        return None;
    }
    Some((alt, path))
}

fn parse_md_row(line: &str) -> Vec<String> {
    let l = line.trim();
    let l = l.strip_prefix('|').unwrap_or(l);
    let l = l.strip_suffix('|').unwrap_or(l);
    l.split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

/// Inline markdown -> runs of (text, bold, italic, code).
fn parse_inline(t: &str) -> Vec<(String, bool, bool, bool)> {
    let c: Vec<char> = t.chars().collect();
    let mut runs = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    macro_rules! flush {
        () => {
            if !buf.is_empty() {
                runs.push((std::mem::take(&mut buf), false, false, false));
            }
        };
    }
    while i < c.len() {
        if c[i] == '*' && i + 1 < c.len() && c[i + 1] == '*' {
            if let Some(j) = find_seq(&c, i + 2, &['*', '*']) {
                flush!();
                let inner: String = c[i + 2..j].iter().collect();
                if !inner.is_empty() {
                    runs.push((inner, true, false, false));
                }
                i = j + 2;
                continue;
            }
        }
        if c[i] == '*' {
            if let Some(j) = c[i + 1..].iter().position(|&x| x == '*').map(|p| p + i + 1) {
                flush!();
                let inner: String = c[i + 1..j].iter().collect();
                if !inner.is_empty() {
                    runs.push((inner, false, true, false));
                }
                i = j + 1;
                continue;
            }
        }
        if c[i] == '`' {
            if let Some(j) = c[i + 1..].iter().position(|&x| x == '`').map(|p| p + i + 1) {
                flush!();
                let inner: String = c[i + 1..j].iter().collect();
                if !inner.is_empty() {
                    runs.push((inner, false, false, true));
                }
                i = j + 1;
                continue;
            }
        }
        if c[i] == '_' && i + 1 < c.len() && c[i + 1] == '_' {
            if let Some(j) = find_seq(&c, i + 2, &['_', '_']) {
                flush!();
                let inner: String = c[i + 2..j].iter().collect();
                if !inner.is_empty() {
                    runs.push((inner, true, false, false));
                }
                i = j + 2;
                continue;
            }
        }
        buf.push(c[i]);
        i += 1;
    }
    flush!();
    runs
}

fn find_seq(hay: &[char], from: usize, seq: &[char]) -> Option<usize> {
    let mut i = from;
    while i + seq.len() <= hay.len() {
        if hay[i..i + seq.len()] == *seq {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// markdown -> docx.
fn md2word(src: &Path, dst: &Path) -> Result<()> {
    let mut text = std::fs::read_to_string(src).with_context(|| format!("cannot read {}", src.display()))?;
    if text.starts_with('\u{feff}') {
        text.remove(0);
    }
    let blocks = parse_markdown(&text);

    // collect images
    let mut images: Vec<DocImage> = Vec::new();
    let src_dir = src.parent().unwrap_or(Path::new("."));
    for b in &blocks {
        if let Block::Image(path, _alt) = b {
            let bytes = load_image_bytes(path, src_dir)?;
            let ext = sniff_img_ext(&bytes)
                .map(|e| e.to_string())
                .unwrap_or_else(|| {
                    Path::new(path)
                        .extension()
                        .and_then(|x| x.to_str())
                        .unwrap_or("png")
                        .to_ascii_lowercase()
                });
            let (cx, cy) = img_size(&bytes, &ext);
            let name = format!("image{}.{}", images.len() + 1, ext);
            images.push(DocImage { bytes, ext, name, cx, cy });
        }
    }

    // build document.xml
    let mut body = String::new();
    let mut img_idx = 0usize;
    for b in &blocks {
        match b {
            Block::Heading(level, t) => {
                body.push_str(&para_xml(
                    &[(
                        &escape_xml(t),
                        false,
                        false,
                        false,
                    )],
                    &format!("Heading{level}"),
                ));
            }
            Block::Para(t) => {
                let runs: Vec<(String, bool, bool, bool)> =
                    parse_inline(t).into_iter().map(|(s, b, i, c)| (escape_xml(&s), b, i, c)).collect();
                body.push_str(&para_xml_refs(&runs, None));
            }
            Block::Bullet(t) => {
                let runs: Vec<(String, bool, bool, bool)> =
                    parse_inline(t).into_iter().map(|(s, b, i, c)| (escape_xml(&s), b, i, c)).collect();
                body.push_str(&bullet_xml(&runs, false));
            }
            Block::Ordered(t, n) => {
                let runs: Vec<(String, bool, bool, bool)> =
                    parse_inline(t).into_iter().map(|(s, b, i, c)| (escape_xml(&s), b, i, c)).collect();
                body.push_str(&ordered_xml(&runs, *n));
            }
            Block::Quote(t) => {
                let runs: Vec<(String, bool, bool, bool)> =
                    parse_inline(t).into_iter().map(|(s, b, i, c)| (escape_xml(&s), b, i, c)).collect();
                body.push_str(&quote_xml(&runs));
            }
            Block::Code(t) => {
                for line in t.lines() {
                    body.push_str(&code_para_xml(&escape_xml(line)));
                }
                // trailing empty code para looks odd; keep it simple
            }
            Block::Rule => {
                body.push_str(r#"<w:p><w:pPr><w:pBdr><w:bottom w:val="single" w:sz="6" w:space="1" w:color="auto"/></w:pBdr></w:pPr></w:p>"#);
            }
            Block::Table(rows) => {
                body.push_str(&table_xml(rows));
            }
            Block::Image(_path, alt) => {
                let img = &images[img_idx];
                body.push_str(&image_para_xml(img_idx + 1, img, alt));
                img_idx += 1;
            }
        }
    }

    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1800" w:bottom="1440" w:left="1800" w:header="851" w:footer="992" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );

    // relationships: rId1 = styles? (not needed), images start at rId10
    let mut rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    );
    for (n, img) in images.iter().enumerate() {
        rels.push_str(&format!(
            r#"<Relationship Id="rIdImg{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{}"/>"#,
            n + 1,
            img.name
        ));
    }
    rels.push_str("</Relationships>");

    // content types with image extensions
    let mut ct = CT_TYPES.trim_end().to_string();
    let exts: Vec<&String> = images.iter().map(|i| &i.ext).collect();
    for e in exts.iter().map(|s| s.as_str()).collect::<std::collections::BTreeSet<_>>() {
        let mime = match e {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            _ => "application/octet-stream",
        };
        ct = ct.replace(
            "</Types>",
            &format!(r#"<Default Extension="{e}" ContentType="{mime}"/></Types>"#),
        );
    }

    // pack
    if dst.exists() {
        std::fs::remove_file(dst).with_context(|| format!("cannot overwrite {}", dst.display()))?;
    }
    let f = std::fs::File::create(dst).with_context(|| format!("cannot create {}", dst.display()))?;
    let mut zip = zip::ZipWriter::new(f);
    let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("[Content_Types].xml", opts)?;
    zip.write_all(ct.as_bytes())?;
    zip.start_file("_rels/.rels", opts)?;
    zip.write_all(ROOT_RELS.as_bytes())?;
    zip.start_file("word/document.xml", opts)?;
    zip.write_all(document.as_bytes())?;
    zip.start_file("word/_rels/document.xml.rels", opts)?;
    zip.write_all(rels.as_bytes())?;
    zip.start_file("word/styles.xml", opts)?;
    zip.write_all(STYLES_XML.as_bytes())?;
    for img in &images {
        zip.start_file(format!("word/media/{}", img.name), opts)?;
        zip.write_all(&img.bytes)?;
    }
    zip.finish()?;
    eprintln!(
        "sys: md2word: {} -> {} ({} images)",
        src.display(),
        dst.display(),
        images.len()
    );
    Ok(())
}

/// Load an image from a local path (relative to the md file) or an http(s) URL.
fn load_image_bytes(path: &str, base_dir: &Path) -> Result<Vec<u8>> {
    if path.starts_with("http://") || path.starts_with("https://") {
        let url = url::Url::parse(path).context("invalid image URL")?;
        let resp = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?
            .get(url)
            .send()
            .with_context(|| format!("image download failed: {path}"))?;
        if !resp.status().is_success() {
            bail!("image download failed: {path} (HTTP {})", resp.status());
        }
        let bytes = resp.bytes().context("cannot read image response")?;
        return Ok(bytes.to_vec());
    }
    let p = Path::new(path);
    let p = if p.is_absolute() { p.to_path_buf() } else { base_dir.join(p) };
    std::fs::read(&p).with_context(|| format!("cannot read image `{}`", p.display()))
}

/// A simple paragraph with optional style; runs = (text, bold, italic, code).
fn para_xml(runs: &[(&str, bool, bool, bool)], style: &str) -> String {
    let mut r = String::new();
    for (t, b, i, _c) in runs {
        let mut rpr = String::new();
        if *b {
            rpr.push_str("<w:b/>");
        }
        if *i {
            rpr.push_str("<w:i/>");
        }
        r.push_str(&format!(r#"<w:r>{rpr}<w:t xml:space="preserve">{t}</w:t></w:r>"#));
    }
    format!(r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr>{r}</w:p>"#)
}

fn para_xml_refs(runs: &[(String, bool, bool, bool)], style: Option<&str>) -> String {
    let mut r = String::new();
    for (t, b, i, c) in runs {
        let mut rpr = String::new();
        if *c {
            rpr.push_str(r#"<w:rFonts w:ascii="Consolas" w:hAnsi="Consolas"/><w:shd w:val="clear" w:fill="F2F2F2"/>"#);
        }
        if *b {
            rpr.push_str("<w:b/>");
        }
        if *i {
            rpr.push_str("<w:i/>");
        }
        r.push_str(&format!(r#"<w:r>{rpr}<w:t xml:space="preserve">{t}</w:t></w:r>"#));
    }
    match style {
        Some(s) => format!(r#"<w:p><w:pPr><w:pStyle w:val="{s}"/></w:pPr>{r}</w:p>"#),
        None => format!("<w:p>{r}</w:p>"),
    }
}

fn bullet_xml(runs: &[(String, bool, bool, bool)], _ordered: bool) -> String {
    // Manual bullet: "• text" with hanging indent + ListParagraph style so a
    // docx->md read-back recognizes it as a list item.
    let s = para_xml_refs(runs, Some("ListParagraph"));
    s.replace(
        "<w:p>",
        r#"<w:p><w:pPr><w:pStyle w:val="ListParagraph"/><w:ind w:left="720" w:hanging="360"/></w:pPr><w:r><w:t xml:space="preserve">• </w:t></w:r>"#,
    )
}

fn ordered_xml(runs: &[(String, bool, bool, bool)], n: u32) -> String {
    let s = para_xml_refs(runs, None);
    s.replace(
        "<w:p>",
        &format!(
            r#"<w:p><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr><w:r><w:t xml:space="preserve">{n}. </w:t></w:r>"#
        ),
    )
}

fn quote_xml(runs: &[(String, bool, bool, bool)]) -> String {
    let s = para_xml_refs(runs, None);
    s.replace(
        "<w:p>",
        r#"<w:p><w:pPr><w:ind w:left="720"/><w:pBdr><w:left w:val="single" w:sz="12" w:space="4" w:color="808080"/></w:pBdr></w:pPr>"#,
    )
}

fn code_para_xml(t: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:ind w:left="360"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Consolas" w:hAnsi="Consolas"/><w:shd w:val="clear" w:fill="F2F2F2"/></w:rPr><w:t xml:space="preserve">{t}</w:t></w:r></w:p>"#
    )
}

fn image_para_xml(idx: usize, img: &DocImage, alt: &str) -> String {
    let rid = format!("rIdImg{idx}");
    let name = escape_xml(&img.name);
    let alt = escape_xml(alt);
    let (cx, cy) = (img.cx, img.cy);
    format!(
        r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{idx}" name="{name}" descr="{alt}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{idx}" name="{name}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
    )
}

fn table_xml(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0).max(1);
    let col_w = 9360 / width;
    let mut x = String::from(
        r#"<w:tbl><w:tblPr><w:tblBorders>"#,
    );
    for e in ["top", "left", "bottom", "right", "insideH", "insideV"] {
        x.push_str(&format!(
            r#"<w:{e} w:val="single" w:sz="4" w:space="0" w:color="auto"/>"#
        ));
    }
    x.push_str("</w:tblBorders></w:tblPr><w:tblGrid>");
    for _ in 0..width {
        x.push_str(&format!(r#"<w:gridCol w:w="{col_w}"/>"#));
    }
    x.push_str("</w:tblGrid>");
    for (n, row) in rows.iter().enumerate() {
        x.push_str("<w:tr>");
        let mut cells = row.clone();
        cells.resize(width, String::new());
        for c in &cells {
            let bold = if n == 0 { "<w:b/>" } else { "" };
            let runs = parse_inline(c);
            let mut rr = String::new();
            for (t, b, i, code) in runs {
                let mut rpr = String::new();
                if n == 0 || b {
                    rpr.push_str("<w:b/>");
                }
                if i {
                    rpr.push_str("<w:i/>");
                }
                if code {
                    rpr.push_str(r#"<w:rFonts w:ascii="Consolas" w:hAnsi="Consolas"/>"#);
                }
                let _ = bold;
                rr.push_str(&format!(
                    r#"<w:r>{rpr}<w:t xml:space="preserve">{}</w:t></w:r>"#,
                    escape_xml(&t)
                ));
            }
            x.push_str(&format!(
                r#"<w:tc><w:tcPr><w:tcW w:w="{col_w}" w:type="dxa"/></w:tcPr><w:p>{rr}</w:p></w:tc>"#
            ));
        }
        x.push_str("</w:tr>");
    }
    x.push_str("</w:tbl>");
    // A table must be followed by a paragraph.
    x.push_str("<w:p/>");
    x
}

// ---------------------------------------------------------------------------
// Entry points (wired from cmd_file)
// ---------------------------------------------------------------------------

/// `--word2md SRC [DST]` / `--md2word SRC [DST]` argument form.
pub struct WordMdArgs {
    pub to_word: bool,
    pub rest: Vec<String>,
}

/// Scan ARGS for the `--word2md` / `--md2word` markers.
pub fn split_wordmd_args(args: &[String]) -> Result<Option<WordMdArgs>> {
    let mut to_word = false;
    let mut mode = false;
    let mut rest = Vec::new();
    for a in args {
        let t = a.trim_start_matches('-');
        match t {
            "word2md" | "docx2md" | "doc2md" => {
                if mode {
                    bail!("duplicate --word2md / --md2word marker");
                }
                mode = true;
                to_word = false;
            }
            "md2word" | "md2docx" | "md2doc" => {
                if mode {
                    bail!("duplicate --word2md / --md2word marker");
                }
                mode = true;
                to_word = true;
            }
            _ => {
                if mode {
                    rest.push(a.clone());
                }
            }
        }
    }
    if !mode {
        return Ok(None);
    }
    Ok(Some(WordMdArgs { to_word, rest }))
}

/// Run the conversion.
pub fn run_wordmd(args: WordMdArgs) -> Result<()> {
    let (src, dst) = match args.rest.len() {
        1 => (args.rest[0].as_str(), None),
        2 => (args.rest[0].as_str(), Some(args.rest[1].as_str())),
        0 => bail!("--word2md/--md2word requires a file (usage: sys file --word2md SRC.docx [DST.md] | --md2word SRC.md [DST.docx])"),
        _ => bail!("too many arguments for --word2md/--md2word (expected SRC [DST])"),
    };
    let sp = Path::new(src);
    if !sp.is_file() {
        bail!("`{src}` is not a file (no such file?)");
    }
    if args.to_word {
        let d = match dst {
            Some(d) => d.to_string(),
            None => bail!("--md2word requires DST (output .docx path)"),
        };
        let dp = Path::new(&d);
        let (sa, da) = (std::path::absolute(sp)?, std::path::absolute(dp)?);
        if sa == da {
            bail!("output must differ from the input file (use a different DST)");
        }
        md2word(sp, dp)
    } else {
        let d = dst.map(|d| d.to_string());
        if let Some(d) = &d {
            let (sa, da) = (std::path::absolute(sp)?, std::path::absolute(Path::new(d))?);
            if sa == da {
                bail!("output must differ from the input file (use a different DST)");
            }
        }
        word2md(
            sp,
            d.as_deref().map(Path::new),
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_unescape_basics() {
        assert_eq!(xml_unescape("a&lt;b&gt;c&amp;d"), "a<b>c&d");
        assert_eq!(xml_unescape("&#20013;&#25991;"), "中文");
        assert_eq!(xml_unescape("x&#xA9;y"), "x©y");
        assert_eq!(xml_unescape("plain"), "plain");
        assert_eq!(xml_unescape("bad &unknown; ent"), "bad &unknown; ent");
    }

    #[test]
    fn xml_attr_extraction() {
        let tag = r#"<Relationship Id="rId4" Target="media/image1.png"/>"#;
        assert_eq!(xml_attr(tag, "Id").unwrap(), "rId4");
        assert_eq!(xml_attr(tag, "Target").unwrap(), "media/image1.png");
        assert_eq!(xml_attr(tag, "Nope"), None);
        let tag2 = "<w:pStyle w:val='Heading2'/>";
        assert_eq!(xml_attr(tag2, "w:val").unwrap(), "Heading2");
    }

    #[test]
    fn parse_relationships_docx() {
        let xml = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="x/styles" Target="styles.xml"/><Relationship Id="rId4" Type="x/image" Target="media/image1.png"/></Relationships>"#;
        let rels = parse_relationships(xml);
        assert_eq!(rels.len(), 2);
        assert_eq!(rels[1], ("rId4".to_string(), "media/image1.png".to_string()));
    }

    #[test]
    fn skip_tag_nested_and_selfclosing() {
        let s: Vec<char> = "<w:p><w:r><w:t>a</w:t></w:r><w:tab/><w:r><w:t>b</w:t></w:r></w:p>".chars().collect();
        let end = skip_tag(&s, 0, "w:p").unwrap();
        assert_eq!(s.iter().collect::<String>().len(), end);
        let s2: Vec<char> = "<w:tab/>rest".chars().collect();
        assert_eq!(skip_tag(&s2, 0, "w:tab"), Some(8));
        // unknown inner tag does not break depth
        let s3: Vec<char> = "<w:p><w:xxx><w:t>z</w:t></w:xxx></w:p>".chars().collect();
        assert_eq!(skip_tag(&s3, 0, "w:p"), Some(s3.len()));
    }

    #[test]
    fn image_token_parsing() {
        assert_eq!(
            parse_image_token("![截图](media/img1.png)"),
            Some(("截图".into(), "media/img1.png".into()))
        );
        assert_eq!(
            parse_image_token("![alt](https://x.com/a.png \"title\")"),
            Some(("alt".into(), "https://x.com/a.png".into()))
        );
        assert_eq!(parse_image_token("![a]()"), None);
        assert_eq!(parse_image_token("hello ![a](b) world"), None);
    }

    #[test]
    fn md_block_parsing() {
        let md = "# 标题\n\n段落一。\n\n- 甲\n- 乙\n\n1. one\n2. two\n\n> 引用\n\n```\ncode line\n```\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\n---\n\n![图](media/p.png)\n";
        let blocks = parse_markdown(md);
        let mut heading = 0;
        let mut bullets = 0;
        let mut ordered = 0;
        let mut table = false;
        let mut code = false;
        let mut img = false;
        let mut rule = false;
        let mut quote = false;
        for b in &blocks {
            match b {
                Block::Heading(_, t) => {
                    heading += 1;
                    assert_eq!(t, "标题");
                }
                Block::Bullet(_) => bullets += 1,
                Block::Ordered(_, n) => {
                    ordered += 1;
                    assert!(*n >= 1 && *n <= 2);
                }
                Block::Table(r) => {
                    table = true;
                    assert_eq!(r.len(), 2);
                    assert_eq!(r[0], vec!["A", "B"]);
                }
                Block::Code(c) => {
                    code = true;
                    assert_eq!(c, "code line");
                }
                Block::Image(p, a) => {
                    img = true;
                    assert_eq!(p, "media/p.png");
                    assert_eq!(a, "图");
                }
                Block::Rule => rule = true,
                Block::Quote(_) => quote = true,
                Block::Para(_) => {}
            }
        }
        assert_eq!(heading, 1);
        assert_eq!(bullets, 2);
        assert_eq!(ordered, 2);
        assert!(table && code && img && rule && quote);
    }

    #[test]
    fn inline_markdown_runs() {
        let runs = parse_inline("普通**粗体**和*斜体*与`code`与中文。");
        assert_eq!(runs.len(), 7);
        assert_eq!(runs[0], ("普通".into(), false, false, false));
        assert_eq!(runs[1], ("粗体".into(), true, false, false));
        assert_eq!(runs[2], ("和".into(), false, false, false));
        assert_eq!(runs[3], ("斜体".into(), false, true, false));
        assert_eq!(runs[4], ("与".into(), false, false, false));
        assert_eq!(runs[5], ("code".into(), false, false, true));
        assert_eq!(runs[6], ("与中文。".into(), false, false, false));
        // unmatched markers stay literal
        let r2 = parse_inline("a * b");
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].0, "a * b");
    }

    #[test]
    fn img_size_sniffing() {
        // PNG 8x2 minimal header bytes
        let png = [
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
            0, 0, 0, 8, 0, 0, 0, 2, 8, 2, 0, 0, 0,
        ];
        assert_eq!(png_size(&png), Some((8, 2)));
        assert_eq!(img_size(&png, "png").0, 8 * 9525);
        // GIF
        let mut gif = b"GIF89a".to_vec();
        gif.extend([0x08, 0x00, 0x02, 0x00]); // 8x2
        assert_eq!(gif_size(&gif), Some((8, 2)));
        // BMP
        let mut bmp = vec![0u8; 26];
        bmp[18] = 8;
        bmp[22] = 2;
        assert_eq!(bmp_size(&bmp), Some((8, 2)));
        // unknown -> fallback EMU
        assert_eq!(img_size(&[], "webp").0, 3_657_600);
    }

    #[test]
    fn sniff_img_ext_all() {
        assert_eq!(sniff_img_ext(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(sniff_img_ext(b"\xff\xd8\xff\xe0"), Some("jpg"));
        assert_eq!(sniff_img_ext(b"GIF89a"), Some("gif"));
        assert_eq!(sniff_img_ext(b"RIFF1234WEBPVP8 "), Some("webp"));
        assert_eq!(sniff_img_ext(b"BM...."), Some("bmp"));
        assert_eq!(sniff_img_ext(b"nope"), None);
    }

    #[test]
    fn split_wordmd_args_scan() {
        let a: Vec<String> = ["--word2md", "a.docx", "b.md"].iter().map(|s| s.to_string()).collect();
        let r = split_wordmd_args(&a).unwrap().unwrap();
        assert!(!r.to_word);
        assert_eq!(r.rest, vec!["a.docx", "b.md"]);
        let b: Vec<String> = ["-l", "--md2word", "x.md", "y.docx"].iter().map(|s| s.to_string()).collect();
        let r = split_wordmd_args(&b).unwrap().unwrap();
        assert!(r.to_word);
        assert_eq!(r.rest, vec!["x.md", "y.docx"]);
        let c: Vec<String> = ["--enc", "a", "b"].iter().map(|s| s.to_string()).collect();
        assert!(split_wordmd_args(&c).unwrap().is_none());
        let d: Vec<String> = ["--word2md", "--md2word"].iter().map(|s| s.to_string()).collect();
        assert!(split_wordmd_args(&d).is_err());
        // aliases
        let e: Vec<String> = ["--docx2md", "a.docx"].iter().map(|s| s.to_string()).collect();
        assert!(!split_wordmd_args(&e).unwrap().unwrap().to_word);
        let f: Vec<String> = ["--md2docx", "a.md"].iter().map(|s| s.to_string()).collect();
        assert!(split_wordmd_args(&f).unwrap().unwrap().to_word);
    }

    #[test]
    fn escape_xml_all_five() {
        assert_eq!(escape_xml("<a>&\"'"), "&lt;a&gt;&amp;&quot;&apos;");
        assert_eq!(escape_xml("中文"), "中文");
    }

    #[test]
    fn md_row_parsing() {
        assert_eq!(parse_md_row("| a | b |"), vec!["a", "b"]);
        assert_eq!(parse_md_row("a | b"), vec!["a", "b"]);
        assert_eq!(parse_md_row("| 含|竖线 |"), vec!["含", "竖线"]);
    }

    #[test]
    fn table_xml_structure() {
        let rows = vec![vec!["A".into(), "B".into()], vec!["1".into(), "2".into()]];
        let x = table_xml(&rows);
        assert!(x.starts_with("<w:tbl>"));
        assert_eq!(x.matches("<w:tr>").count(), 2);
        assert!(x.contains("Heading") == false);
        assert!(x.contains("w:gridCol"));
    }

    #[test]
    fn roundtrip_docx_via_lib() {
        // Build a docx from markdown, then read it back and check the text.
        let md = "# 标题一\n\n这是**加粗**段落。\n\n- 列表甲\n- 列表乙\n\n| 列A | 列B |\n| --- | --- |\n| 甲1 | 乙2 |\n";
        let blocks = parse_markdown(md);
        let mut body = String::new();
        for b in &blocks {
            match b {
                Block::Heading(l, t) => {
                    body.push_str(&para_xml(&[(&escape_xml(t), false, false, false)], &format!("Heading{l}")));
                }
                Block::Para(t) => {
                    let runs: Vec<(String, bool, bool, bool)> =
                        parse_inline(t).into_iter().map(|(s, b, i, c)| (escape_xml(&s), b, i, c)).collect();
                    body.push_str(&para_xml_refs(&runs, None));
                }
                Block::Bullet(t) => {
                    let runs: Vec<(String, bool, bool, bool)> =
                        parse_inline(t).into_iter().map(|(s, b, i, c)| (escape_xml(&s), b, i, c)).collect();
                    body.push_str(&bullet_xml(&runs, false));
                }
                Block::Table(rows) => body.push_str(&table_xml(rows)),
                _ => {}
            }
        }
        let document = format!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
        );
        let path = std::env::temp_dir().join("sys-file-wordmd-test.docx");
        let f = std::fs::File::create(&path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        z.start_file("[Content_Types].xml", opts).unwrap();
        z.write_all(CT_TYPES.as_bytes()).unwrap();
        z.start_file("_rels/.rels", opts).unwrap();
        z.write_all(ROOT_RELS.as_bytes()).unwrap();
        z.start_file("word/document.xml", opts).unwrap();
        z.write_all(document.as_bytes()).unwrap();
        z.finish().unwrap();

        // read back
        let (xml, imgs) = read_docx(&path).unwrap();
        assert!(imgs.is_empty());
        let md_out = render_body(&xml, &imgs, "media").unwrap();
        assert!(md_out.contains("# 标题一"), "md: {md_out}");
        assert!(md_out.contains("**加粗**"), "md: {md_out}");
        assert!(md_out.contains("- 列表甲"), "md: {md_out}");
        assert!(md_out.contains("| **列A** | **列B** |") || md_out.contains("| 列A | 列B |"), "md: {md_out}");
        assert!(md_out.contains("| --- | --- |"), "md: {md_out}");
        let _ = std::fs::remove_file(&path);
    }
}
