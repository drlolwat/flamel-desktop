use serde_json::Value;
use std::path::Path;
use tree_sitter::{Language, Node, Parser};

// ── Quote normalization ───────────────────────────────────────────────────────

/// Normalize curly/smart quotes to straight quotes so AI-generated needles
/// match real source files that use standard ASCII punctuation.
fn normalize_quotes(s: &str) -> String {
    s.replace('\u{201C}', "\"") // " left double
        .replace('\u{201D}', "\"") // " right double
        .replace('\u{2018}', "'") // ' left single
        .replace('\u{2019}', "'") // ' right single
        .replace('\u{2032}', "'") // ′ prime
        .replace('\u{FF02}', "\"") // ＂ fullwidth double
        .replace('\u{FF07}', "'") // ＇ fullwidth single
}

// ── Language detection ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lang {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
}

pub fn detect_language(path: &str) -> Option<Lang> {
    match Path::new(path).extension().and_then(|e| e.to_str()) {
        Some("rs") => Some(Lang::Rust),
        Some("ts") => Some(Lang::TypeScript),
        Some("tsx") => Some(Lang::Tsx),
        Some("js") | Some("mjs") | Some("cjs") | Some("jsx") => Some(Lang::JavaScript),
        _ => None,
    }
}

fn make_parser(lang: Lang) -> Result<Parser, String> {
    let mut parser = Parser::new();
    let language: Language = match lang {
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
    };
    parser
        .set_language(&language)
        .map_err(|e| format!("set_language: {e}"))?;
    Ok(parser)
}

// ── Symbol metadata (no lifetimes — just data) ────────────────────────────────

#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: String,
    pub start_line: usize, // 1-based
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn node_text<'a>(node: Node<'a>, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

fn symbol_name(node: Node, src: &[u8], lang: Lang) -> Option<String> {
    if let Some(n) = node.child_by_field_name("name") {
        return Some(node_text(n, src).to_string());
    }
    // Rust impl items: name is the type being implemented
    if lang == Lang::Rust && node.kind() == "impl_item" {
        if let Some(t) = node.child_by_field_name("type") {
            return Some(node_text(t, src).to_string());
        }
    }
    None
}

fn is_top_level(kind: &str, lang: Lang) -> bool {
    match lang {
        Lang::Rust => matches!(
            kind,
            "function_item"
                | "struct_item"
                | "enum_item"
                | "trait_item"
                | "impl_item"
                | "type_item"
                | "const_item"
                | "static_item"
                | "mod_item"
                | "macro_definition"
                | "macro_rules"
        ),
        Lang::TypeScript | Lang::Tsx => matches!(
            kind,
            "function_declaration"
                | "class_declaration"
                | "interface_declaration"
                | "type_alias_declaration"
                | "enum_declaration"
                | "lexical_declaration"
                | "variable_declaration"
                | "export_statement"
                | "abstract_class_declaration"
        ),
        Lang::JavaScript => matches!(
            kind,
            "function_declaration"
                | "class_declaration"
                | "lexical_declaration"
                | "variable_declaration"
                | "export_statement"
        ),
    }
}

fn is_method(kind: &str, lang: Lang) -> bool {
    match lang {
        Lang::Rust => kind == "function_item",
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
            matches!(kind, "method_definition" | "public_field_definition")
        }
    }
}

fn body_node(node: Node) -> Option<Node> {
    for i in 0..node.named_child_count() {
        if let Some(c) = node.named_child(i as u32) {
            if matches!(c.kind(), "declaration_list" | "class_body" | "object_type") {
                return Some(c);
            }
        }
    }
    None
}

fn make_sym(node: Node, src: &[u8], lang: Lang) -> Option<SymbolInfo> {
    let name = symbol_name(node, src, lang)?;
    Some(SymbolInfo {
        name,
        kind: node.kind().to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    })
}

fn collect_methods(parent: Node, src: &[u8], lang: Lang) -> Vec<SymbolInfo> {
    let mut out = vec![];
    let body = match body_node(parent) {
        Some(b) => b,
        None => return out,
    };
    for i in 0..body.named_child_count() {
        let Some(child) = body.named_child(i as u32) else {
            continue;
        };
        if is_method(child.kind(), lang) {
            if let Some(sym) = make_sym(child, src, lang) {
                out.push(sym);
            }
        }
    }
    out
}

fn extract_var_syms(node: Node, src: &[u8]) -> Vec<SymbolInfo> {
    let mut out = vec![];
    for i in 0..node.named_child_count() {
        let Some(decl) = node.named_child(i as u32) else {
            continue;
        };
        if decl.kind() != "variable_declarator" {
            continue;
        }
        let Some(nn) = decl.child_by_field_name("name") else {
            continue;
        };
        let name = node_text(nn, src).to_string();
        let Some(value) = decl.child_by_field_name("value") else {
            continue;
        };
        if matches!(
            value.kind(),
            "arrow_function" | "function" | "function_expression" | "class"
        ) {
            out.push(SymbolInfo {
                name,
                kind: format!("const:{}", value.kind()),
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
            });
        }
    }
    out
}

// ── Top-level collection ──────────────────────────────────────────────────────

pub fn collect_top_level(root: Node, src: &[u8], lang: Lang) -> Vec<(SymbolInfo, Vec<SymbolInfo>)> {
    let mut out = vec![];
    for i in 0..root.named_child_count() {
        let Some(child) = root.named_child(i as u32) else {
            continue;
        };
        let kind = child.kind();

        if kind == "export_statement" {
            for j in 0..child.named_child_count() {
                let Some(inner) = child.named_child(j as u32) else {
                    continue;
                };
                if is_top_level(inner.kind(), lang) {
                    if let Some(sym) = make_sym(inner, src, lang) {
                        out.push((sym, collect_methods(inner, src, lang)));
                    }
                } else if matches!(inner.kind(), "lexical_declaration" | "variable_declaration") {
                    for sym in extract_var_syms(inner, src) {
                        out.push((sym, vec![]));
                    }
                }
            }
            continue;
        }

        if matches!(kind, "lexical_declaration" | "variable_declaration") {
            for sym in extract_var_syms(child, src) {
                out.push((sym, vec![]));
            }
            continue;
        }

        if is_top_level(kind, lang) {
            if let Some(sym) = make_sym(child, src, lang) {
                out.push((sym, collect_methods(child, src, lang)));
            }
        }
    }
    out
}

// ── Symbol finding by name_path ───────────────────────────────────────────────

type ByteRange = (usize, usize, usize, usize); // start_byte, end_byte, start_line, end_line

fn find_by_path(root: Node, src: &[u8], lang: Lang, parts: &[&str]) -> Option<ByteRange> {
    if parts.is_empty() {
        return None;
    }
    let target = parts[0];

    for i in 0..root.named_child_count() {
        let Some(child) = root.named_child(i as u32) else {
            continue;
        };
        let kind = child.kind();

        if kind == "export_statement" {
            for j in 0..child.named_child_count() {
                let Some(inner) = child.named_child(j as u32) else {
                    continue;
                };
                if let Some(r) = try_match(inner, src, lang, target, &parts[1..]) {
                    return Some(r);
                }
            }
            continue;
        }

        if matches!(kind, "lexical_declaration" | "variable_declaration") {
            for j in 0..child.named_child_count() {
                let Some(decl) = child.named_child(j as u32) else {
                    continue;
                };
                if decl.kind() != "variable_declarator" {
                    continue;
                }
                if let Some(nn) = decl.child_by_field_name("name") {
                    if node_text(nn, src) == target && parts.len() == 1 {
                        return Some((
                            child.start_byte(),
                            child.end_byte(),
                            child.start_position().row + 1,
                            child.end_position().row + 1,
                        ));
                    }
                }
            }
            continue;
        }

        if let Some(r) = try_match(child, src, lang, target, &parts[1..]) {
            return Some(r);
        }
    }
    None
}

fn try_match(node: Node, src: &[u8], lang: Lang, target: &str, rest: &[&str]) -> Option<ByteRange> {
    if !is_top_level(node.kind(), lang) && !is_method(node.kind(), lang) {
        return None;
    }
    let name = symbol_name(node, src, lang)?;
    if name != target {
        return None;
    }

    if rest.is_empty() {
        return Some((
            node.start_byte(),
            node.end_byte(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ));
    }

    let body = body_node(node)?;
    for i in 0..body.named_child_count() {
        let Some(child) = body.named_child(i as u32) else {
            continue;
        };
        if let Some(r) = try_match(child, src, lang, rest[0], &rest[1..]) {
            return Some(r);
        }
    }
    None
}

// ── Public tool handlers ──────────────────────────────────────────────────────

pub fn get_symbols_overview(args: &Value) -> Result<String, String> {
    let path = args
        .get("relative_path")
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str())
        .ok_or("path required")?;

    let lang = detect_language(path).ok_or_else(|| format!("unsupported file type: {path}"))?;
    let src = std::fs::read(path).map_err(|e| format!("read error: {e}"))?;
    let mut parser = make_parser(lang)?;
    let tree = parser.parse(&src, None).ok_or("parse failed")?;

    let symbols = collect_top_level(tree.root_node(), &src, lang);
    if symbols.is_empty() {
        return Ok(format!("No symbols found in {path}"));
    }

    let mut out = format!("Symbols in {path}:\n");
    for (sym, methods) in &symbols {
        out.push_str(&format!(
            "  [{}] {} (L{}-{})\n",
            sym.kind, sym.name, sym.start_line, sym.end_line
        ));
        for m in methods {
            out.push_str(&format!(
                "    [{}] {} (L{}-{})\n",
                m.kind, m.name, m.start_line, m.end_line
            ));
        }
    }
    Ok(out)
}

pub fn index_file(path: &Path) -> Result<Value, String> {
    let path_text = path.to_string_lossy();
    let lang =
        detect_language(&path_text).ok_or_else(|| format!("unsupported file type: {path_text}"))?;
    let src = std::fs::read(path).map_err(|e| format!("read error: {e}"))?;
    let mut parser = make_parser(lang)?;
    let tree = parser.parse(&src, None).ok_or("parse failed")?;
    let symbols = collect_top_level(tree.root_node(), &src, lang)
        .into_iter()
        .map(|(symbol, methods)| {
            serde_json::json!({
                "name": symbol.name,
                "kind": symbol.kind,
                "startLine": symbol.start_line,
                "endLine": symbol.end_line,
                "methods": methods.into_iter().map(|method| serde_json::json!({
                    "name": method.name,
                    "kind": method.kind,
                    "startLine": method.start_line,
                    "endLine": method.end_line,
                })).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "language": match lang { Lang::Rust => "rust", Lang::TypeScript => "typescript", Lang::Tsx => "tsx", Lang::JavaScript => "javascript" },
        "symbols": symbols,
    }))
}

pub fn find_symbol_result(args: &Value) -> Result<Value, String> {
    let path = args
        .get("relative_path")
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str())
        .ok_or("path required")?;
    let name_path = args
        .get("name_path")
        .and_then(|v| v.as_str())
        .ok_or("name_path required")?;
    let include_body = args
        .get("include_body")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(0);

    let lang = detect_language(path).ok_or_else(|| format!("unsupported: {path}"))?;
    let src = std::fs::read(path).map_err(|e| format!("read error: {e}"))?;
    let mut parser = make_parser(lang)?;
    let tree = parser.parse(&src, None).ok_or("parse failed")?;

    let parts: Vec<&str> = name_path.split('/').collect();
    let (start_byte, end_byte, start_line, end_line) =
        find_by_path(tree.root_node(), &src, lang, &parts)
            .ok_or_else(|| format!("symbol '{name_path}' not found in {path}"))?;
    let source = if include_body {
        Some(
            std::str::from_utf8(&src[start_byte..end_byte])
                .map_err(|_| format!("symbol source in {path} is not UTF-8"))?
                .to_string(),
        )
    } else {
        None
    };
    let children = if depth > 0 {
        collect_top_level(tree.root_node(), &src, lang)
            .into_iter()
            .find(|(symbol, _)| symbol.start_byte == start_byte)
            .map(|(_, methods)| {
                methods
                    .into_iter()
                    .map(|method| {
                        serde_json::json!({
                            "name": method.name,
                            "kind": method.kind,
                            "startLine": method.start_line,
                            "endLine": method.end_line,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(serde_json::json!({
        "path": path,
        "namePath": name_path,
        "language": match lang {
            Lang::Rust => "rust",
            Lang::TypeScript => "typescript",
            Lang::Tsx => "tsx",
            Lang::JavaScript => "javascript",
        },
        "range": {"startLine": start_line, "endLine": end_line},
        "source": source,
        "children": children,
    }))
}

pub fn find_symbol(args: &Value) -> Result<String, String> {
    let result = find_symbol_result(args)?;
    let path = result["path"].as_str().unwrap_or_default();
    let name_path = result["namePath"].as_str().unwrap_or_default();
    let start_line = result["range"]["startLine"].as_u64().unwrap_or(0);
    let end_line = result["range"]["endLine"].as_u64().unwrap_or(0);
    let mut out = format!("Symbol '{name_path}' in {path} (L{start_line}-{end_line}):\n");

    if let Some(source) = result["source"].as_str() {
        for (index, line) in source.lines().enumerate() {
            out.push_str(&format!("{:>4}\t{}\n", start_line + index as u64, line));
        }
    }
    if let Some(children) = result["children"].as_array() {
        for child in children {
            out.push_str(&format!(
                "  child: [{}] {} (L{}-{})\n",
                child["kind"].as_str().unwrap_or_default(),
                child["name"].as_str().unwrap_or_default(),
                child["startLine"].as_u64().unwrap_or(0),
                child["endLine"].as_u64().unwrap_or(0),
            ));
        }
    }
    Ok(out)
}

pub fn replace_symbol_body(args: &Value) -> Result<String, String> {
    let path = args
        .get("relative_path")
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str())
        .ok_or("path required")?;
    let name_path = args
        .get("name_path")
        .and_then(|v| v.as_str())
        .ok_or("name_path required")?;
    let new_body = args
        .get("new_body")
        .and_then(|v| v.as_str())
        .ok_or("new_body required")?;

    let lang = detect_language(path).ok_or_else(|| format!("unsupported: {path}"))?;
    let src = std::fs::read(path).map_err(|e| format!("read error: {e}"))?;
    let mut parser = make_parser(lang)?;
    let tree = parser.parse(&src, None).ok_or("parse failed")?;

    let parts: Vec<&str> = name_path.split('/').collect();
    let (sb, eb, sl, el) = find_by_path(tree.root_node(), &src, lang, &parts)
        .ok_or_else(|| format!("symbol '{name_path}' not found in {path}"))?;

    let content = std::str::from_utf8(&src).map_err(|e| format!("utf8 error: {e}"))?;
    let old_body = std::str::from_utf8(&src[sb..eb]).map_err(|e| format!("utf8 error: {e}"))?;
    let diff = make_unified_diff(content, old_body, new_body, path);
    let (added, removed) = unified_diff_stats(&diff);

    let mut new_src = Vec::with_capacity(src.len());
    new_src.extend_from_slice(&src[..sb]);
    new_src.extend_from_slice(new_body.as_bytes());
    new_src.extend_from_slice(&src[eb..]);

    std::fs::write(path, &new_src).map_err(|e| format!("write error: {e}"))?;
    if diff.is_empty() {
        Ok(format!("Replaced '{name_path}' in {path} (was L{sl}-{el})"))
    } else {
        Ok(format!(
            "Replaced '{name_path}' in {path} (was L{sl}-{el}) (+{added}/-{removed})\n\n{diff}"
        ))
    }
}

pub fn insert_before_symbol(args: &Value) -> Result<String, String> {
    insert_relative(args, true)
}
pub fn insert_after_symbol(args: &Value) -> Result<String, String> {
    insert_relative(args, false)
}

fn insert_relative(args: &Value, before: bool) -> Result<String, String> {
    let path = args
        .get("relative_path")
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str())
        .ok_or("path required")?;
    let name_path = args
        .get("name_path")
        .and_then(|v| v.as_str())
        .ok_or("name_path required")?;
    let code = args
        .get("new_code")
        .or_else(|| args.get("code"))
        .and_then(|v| v.as_str())
        .ok_or("new_code required")?;

    let lang = detect_language(path).ok_or_else(|| format!("unsupported: {path}"))?;
    let src = std::fs::read(path).map_err(|e| format!("read error: {e}"))?;
    let mut parser = make_parser(lang)?;
    let tree = parser.parse(&src, None).ok_or("parse failed")?;

    let parts: Vec<&str> = name_path.split('/').collect();
    let (sb, eb, _, _) = find_by_path(tree.root_node(), &src, lang, &parts)
        .ok_or_else(|| format!("symbol '{name_path}' not found in {path}"))?;

    let (insert_byte, insertion) = if before {
        (sb, format!("{code}\n\n"))
    } else {
        (eb, format!("\n\n{code}"))
    };

    let content = std::str::from_utf8(&src).map_err(|e| format!("utf8 error: {e}"))?;
    let diff = make_insertion_diff(content, insert_byte, &insertion, path);
    let added = insertion.lines().count();

    let mut new_src = Vec::with_capacity(src.len() + insertion.len());
    new_src.extend_from_slice(&src[..insert_byte]);
    new_src.extend_from_slice(insertion.as_bytes());
    new_src.extend_from_slice(&src[insert_byte..]);

    std::fs::write(path, &new_src).map_err(|e| format!("write error: {e}"))?;
    if diff.is_empty() {
        Ok(format!(
            "Inserted {} '{name_path}' in {path}",
            if before { "before" } else { "after" }
        ))
    } else {
        Ok(format!(
            "Inserted {} '{name_path}' in {path} (+{added}/-0)\n\n{diff}",
            if before { "before" } else { "after" }
        ))
    }
}

pub fn find_referencing_symbols(args: &Value) -> Result<String, String> {
    let name = args
        .get("symbol_name")
        .or_else(|| args.get("name"))
        .and_then(|v| v.as_str())
        .ok_or("symbol_name required")?;
    let path = args
        .get("relative_path")
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let context = args
        .get("context_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(2);

    let out = std::process::Command::new("rg")
        .args([
            "--context",
            &context.to_string(),
            "--line-number",
            "--with-filename",
            name,
            path,
        ])
        .output()
        .map_err(|e| format!("rg not found: {e}"))?;

    let result = String::from_utf8_lossy(&out.stdout).to_string();
    if result.trim().is_empty() {
        Ok(format!("No references to '{name}' found in {path}"))
    } else {
        Ok(result)
    }
}

#[derive(Clone, Copy)]
enum LineDiff<'a> {
    Equal(&'a str),
    Add(&'a str),
    Delete(&'a str),
}

fn line_diff<'a>(old_lines: &[&'a str], new_lines: &[&'a str]) -> Vec<LineDiff<'a>> {
    // Exact edits are normally small. Bound the quadratic LCS table so a
    // whole-file replacement cannot turn a tool response into a CPU spike.
    if old_lines.len().saturating_mul(new_lines.len()) > 160_000 {
        return old_lines
            .iter()
            .map(|line| LineDiff::Delete(*line))
            .chain(new_lines.iter().map(|line| LineDiff::Add(*line)))
            .collect();
    }

    let mut table = vec![vec![0usize; new_lines.len() + 1]; old_lines.len() + 1];
    for old_index in (0..old_lines.len()).rev() {
        for new_index in (0..new_lines.len()).rev() {
            table[old_index][new_index] = if old_lines[old_index] == new_lines[new_index] {
                table[old_index + 1][new_index + 1] + 1
            } else {
                table[old_index + 1][new_index].max(table[old_index][new_index + 1])
            };
        }
    }

    let mut result = Vec::new();
    let (mut old_index, mut new_index) = (0usize, 0usize);
    while old_index < old_lines.len() || new_index < new_lines.len() {
        if old_index < old_lines.len()
            && new_index < new_lines.len()
            && old_lines[old_index] == new_lines[new_index]
        {
            result.push(LineDiff::Equal(old_lines[old_index]));
            old_index += 1;
            new_index += 1;
        } else if new_index < new_lines.len()
            && (old_index >= old_lines.len()
                || table[old_index][new_index + 1] > table[old_index + 1][new_index])
        {
            result.push(LineDiff::Add(new_lines[new_index]));
            new_index += 1;
        } else {
            result.push(LineDiff::Delete(old_lines[old_index]));
            old_index += 1;
        }
    }
    result
}

/// Count the actual changed lines in a generated unified diff.
pub fn unified_diff_stats(diff: &str) -> (usize, usize) {
    diff.lines()
        .fold((0usize, 0usize), |(added, removed), line| {
            if line.starts_with('+') && !line.starts_with("+++") {
                (added + 1, removed)
            } else if line.starts_with('-') && !line.starts_with("---") {
                (added, removed + 1)
            } else {
                (added, removed)
            }
        })
}

/// Generate a compact, line-aware unified diff with 3 lines of context.
pub fn make_unified_diff(content: &str, needle: &str, replacement: &str, file: &str) -> String {
    let pos = match content.find(needle) {
        Some(p) => p,
        None => return String::new(),
    };

    let pre = &content[..pos];
    let old_lines: Vec<&str> = needle.lines().collect();
    let new_lines: Vec<&str> = replacement.lines().collect();

    if old_lines.is_empty() && new_lines.is_empty() {
        return String::new();
    }

    let start_line = pre.lines().count() + 1; // 1-based

    let pre_lines: Vec<&str> = pre.lines().collect();
    let ctx_start = pre_lines.len().saturating_sub(3);
    let ctx_before: Vec<&str> = pre_lines[ctx_start..].to_vec();

    let after_pos = pos + needle.len();
    let after = if after_pos <= content.len() {
        &content[after_pos..]
    } else {
        ""
    };
    let ctx_after: Vec<&str> = after.lines().take(3).collect();

    let mut operations = ctx_before
        .iter()
        .map(|line| LineDiff::Equal(*line))
        .chain(line_diff(&old_lines, &new_lines))
        .chain(ctx_after.iter().map(|line| LineDiff::Equal(*line)))
        .collect::<Vec<_>>();
    if !operations
        .iter()
        .any(|operation| !matches!(operation, LineDiff::Equal(_)))
    {
        return String::new();
    }

    let base_line = start_line.saturating_sub(ctx_before.len()).max(1);
    let mut numbered = Vec::with_capacity(operations.len());
    let (mut old_number, mut new_number) = (base_line, base_line);
    for operation in operations.drain(..) {
        numbered.push((operation, old_number, new_number));
        match operation {
            LineDiff::Equal(_) => {
                old_number += 1;
                new_number += 1;
            }
            LineDiff::Add(_) => new_number += 1,
            LineDiff::Delete(_) => old_number += 1,
        }
    }

    let changed = numbered
        .iter()
        .enumerate()
        .filter_map(|(index, (operation, _, _))| {
            (!matches!(operation, LineDiff::Equal(_))).then_some(index)
        })
        .collect::<Vec<_>>();
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    for index in changed {
        let start = index.saturating_sub(3);
        let end = (index + 3).min(numbered.len().saturating_sub(1));
        if let Some((_, previous_end)) = hunks.last_mut() {
            if start <= *previous_end + 1 {
                *previous_end = end;
                continue;
            }
        }
        hunks.push((start, end));
    }

    let mut diff = format!("--- a/{file}\n+++ b/{file}\n");
    for (start, end) in hunks {
        let slice = &numbered[start..=end];
        let old_count = slice
            .iter()
            .filter(|(operation, _, _)| !matches!(operation, LineDiff::Add(_)))
            .count();
        let new_count = slice
            .iter()
            .filter(|(operation, _, _)| !matches!(operation, LineDiff::Delete(_)))
            .count();
        diff.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            slice[0].1, old_count, slice[0].2, new_count
        ));
        for (operation, _, _) in slice {
            match operation {
                LineDiff::Equal(line) => diff.push_str(&format!(" {line}\n")),
                LineDiff::Add(line) => diff.push_str(&format!("+{line}\n")),
                LineDiff::Delete(line) => diff.push_str(&format!("-{line}\n")),
            }
        }
    }
    diff
}

/// Generate a compact unified diff for an insertion at a byte offset.
pub fn make_insertion_diff(
    content: &str,
    insert_byte: usize,
    insertion: &str,
    file: &str,
) -> String {
    if insertion.is_empty() || insert_byte > content.len() || !content.is_char_boundary(insert_byte)
    {
        return String::new();
    }

    let pre = &content[..insert_byte];
    let after = &content[insert_byte..];
    let inserted_lines: Vec<&str> = insertion.lines().collect();
    if inserted_lines.is_empty() {
        return String::new();
    }

    let start_line = pre.lines().count() + 1;
    let pre_lines: Vec<&str> = pre.lines().collect();
    let ctx_start = pre_lines.len().saturating_sub(3);
    let ctx_before: Vec<&str> = pre_lines[ctx_start..].to_vec();
    let ctx_after: Vec<&str> = after.lines().take(3).collect();

    let hunk_old_start = (start_line - ctx_before.len()).max(1);
    let hunk_new_start = hunk_old_start;
    let hunk_old_count = ctx_before.len() + ctx_after.len();
    let hunk_new_count = ctx_before.len() + inserted_lines.len() + ctx_after.len();

    let mut diff = format!("--- a/{file}\n+++ b/{file}\n");
    diff.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        hunk_old_start, hunk_old_count, hunk_new_start, hunk_new_count
    ));

    for line in &ctx_before {
        diff.push_str(&format!(" {line}\n"));
    }
    for line in &inserted_lines {
        diff.push_str(&format!("+{line}\n"));
    }
    for line in &ctx_after {
        diff.push_str(&format!(" {line}\n"));
    }

    diff
}

pub fn replace_content(args: &Value) -> Result<String, String> {
    let path = args
        .get("relative_path")
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str())
        .ok_or("path required")?;
    let needle = args
        .get("needle")
        .or_else(|| args.get("old_string"))
        .or_else(|| args.get("old"))
        .and_then(|v| v.as_str())
        .ok_or("needle required — pass needle=\"exact text to replace\"")?;
    let replacement = args
        .get("repl")
        .or_else(|| args.get("replacement"))
        .and_then(|v| v.as_str())
        .ok_or("repl required")?;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("text");

    let content = std::fs::read_to_string(path).map_err(|e| format!("read error: {e}"))?;

    let (new_content, actual_needle) = if mode == "regex" {
        let re = regex_lite::Regex::new(needle).map_err(|e| format!("regex error: {e}"))?;
        (
            re.replace_all(&content, replacement).to_string(),
            needle.to_string(),
        )
    } else {
        // Try exact match first, then quote-normalized fallback
        if content.contains(needle) {
            (content.replace(needle, replacement), needle.to_string())
        } else {
            let norm = normalize_quotes(needle);
            if norm != needle && content.contains(norm.as_str()) {
                (content.replace(norm.as_str(), replacement), norm)
            } else {
                (content.clone(), needle.to_string()) // unchanged — error below
            }
        }
    };

    if new_content == content {
        return Err(format!("needle not found in {path}"));
    }

    let diff = make_unified_diff(&content, &actual_needle, replacement, path);
    let (added, removed) = unified_diff_stats(&diff);

    std::fs::write(path, new_content).map_err(|e| format!("write error: {e}"))?;

    if diff.is_empty() {
        Ok(format!("Content replaced in {path}"))
    } else {
        Ok(format!(
            "Content replaced in {path} (+{added}/-{removed})\n\n{diff}"
        ))
    }
}

#[cfg(test)]
mod diff_tests {
    use super::{make_unified_diff, unified_diff_stats};

    #[test]
    fn replacement_diff_preserves_unchanged_lines() {
        let content = "before\none\ntwo\nthree\nafter\n";
        let diff = make_unified_diff(
            content,
            "one\ntwo\nthree",
            "one\nchanged\nthree",
            "demo.txt",
        );
        assert_eq!(unified_diff_stats(&diff), (1, 1));
        assert!(diff.contains(" one\n-two\n+changed\n three"));
    }

    #[test]
    fn replacement_diff_splits_distant_changes_into_hunks() {
        let old = (0..20)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut new_lines = old.lines().map(str::to_string).collect::<Vec<_>>();
        new_lines[1] = "first change".into();
        new_lines[18] = "last change".into();
        let content = format!("header\n{old}\nfooter\n");
        let diff = make_unified_diff(&content, &old, &new_lines.join("\n"), "demo.txt");
        assert_eq!(unified_diff_stats(&diff), (2, 2));
        assert_eq!(diff.matches("@@ ").count(), 2);
    }
}
