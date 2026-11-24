//! Atlassian Document Format to markdown, for Jira Cloud issue bodies and
//! comments. Jira API v3 returns ADF JSON; the detail pane renders markdown.

use serde_json::Value;

const ATTACHMENT: &str = "[attachment]";

/// Render an ADF document as markdown. Unknown nodes degrade to their text,
/// and malformed input renders empty instead of panicking.
pub fn adf_to_markdown(doc: &Value) -> String {
    let blocks = if kind(doc) == "doc" {
        child_blocks(doc)
    } else {
        block(doc).into_iter().collect()
    };
    finish(blocks)
}

fn kind(node: &Value) -> &str {
    node["type"].as_str().unwrap_or("")
}

fn child_blocks(node: &Value) -> Vec<String> {
    node["content"]
        .as_array()
        .map(|kids| kids.iter().filter_map(block).collect())
        .unwrap_or_default()
}

/// One block: a paragraph, a list, a table, or a whole fence. Lists and
/// quotes keep their inner line breaks in a single block.
fn block(node: &Value) -> Option<String> {
    match kind(node) {
        "doc" => joined(child_blocks(node)),
        "paragraph" => non_empty(&inline(node)),
        "heading" => non_empty(&heading(node)),
        "bulletList" => list(node, false),
        "orderedList" => list(node, true),
        "codeBlock" => Some(code_block(node)),
        "blockquote" | "panel" => quote(node),
        "rule" => Some("---".to_string()),
        "table" => table(node),
        "media" | "mediaInline" | "mediaSingle" | "mediaGroup" => Some(ATTACHMENT.to_string()),
        "text" | "hardBreak" | "mention" | "emoji" | "inlineCard" | "blockCard" | "status"
        | "date" => non_empty(&inline(node)),
        _ => fallback_block(node),
    }
}

/// Inline content: text with its marks, and the leaf inline nodes Jira uses.
fn inline(node: &Value) -> String {
    match kind(node) {
        "text" => marked_text(node),
        "hardBreak" => "\\\n".to_string(),
        "mention" => mention(node),
        "emoji" => attr_text(node)
            .or_else(|| short_name(node))
            .unwrap_or_default(),
        "inlineCard" | "blockCard" => card(node),
        "media" | "mediaInline" | "mediaSingle" | "mediaGroup" => ATTACHMENT.to_string(),
        "status" => attr_text(node).unwrap_or_default(),
        "date" => date(node),
        "rule" => "---".to_string(),
        _ => inline_children(node),
    }
}

fn inline_children(node: &Value) -> String {
    let Some(kids) = node["content"].as_array() else {
        return scalar_text(node);
    };
    let mut text = String::new();
    for kid in kids {
        if is_block(kind(kid)) {
            let piece = inline_children(kid);
            if piece.is_empty() {
                continue;
            }
            // Block children of a cell or unknown wrapper stay on one line.
            if !text.is_empty() && !text.ends_with(' ') && !text.ends_with('\n') {
                text.push(' ');
            }
            text.push_str(&piece);
        } else {
            text.push_str(&inline(kid));
        }
    }
    text
}

fn is_block(kind: &str) -> bool {
    matches!(
        kind,
        "paragraph"
            | "heading"
            | "blockquote"
            | "panel"
            | "codeBlock"
            | "rule"
            | "table"
            | "tableRow"
            | "bulletList"
            | "orderedList"
            | "listItem"
            | "mediaSingle"
            | "mediaGroup"
    )
}

fn marked_text(node: &Value) -> String {
    let mut text = scalar_text(node);
    let mut link = None;
    let mut strong = false;
    let mut em = false;
    let mut code = false;
    let mut strike = false;
    for mark in node["marks"].as_array().into_iter().flatten() {
        match kind(mark) {
            "strong" => strong = true,
            "em" => em = true,
            "code" => code = true,
            "strike" => strike = true,
            "link" => {
                link = mark["attrs"]["href"]
                    .as_str()
                    .filter(|href| !href.is_empty())
            }
            _ => {}
        }
    }
    if code {
        text = format!("`{text}`");
    }
    if strike {
        text = format!("~~{text}~~");
    }
    if em {
        text = format!("_{text}_");
    }
    if strong {
        text = format!("**{text}**");
    }
    if let Some(href) = link {
        text = format!("[{text}]({href})");
    }
    text
}

fn heading(node: &Value) -> String {
    let level = node["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6);
    format!("{} {}", "#".repeat(level as usize), inline(node))
}

fn list(node: &Value, ordered: bool) -> Option<String> {
    let items = node["content"].as_array()?;
    let mut next = if ordered { order(node) } else { 0 };
    let mut blocks = Vec::new();
    for item in items {
        let item_blocks = child_blocks(item);
        if item_blocks.is_empty() {
            continue;
        }
        let marker = if ordered {
            format!("{next}. ")
        } else {
            "- ".to_string()
        };
        next += 1;
        let pad = " ".repeat(marker.len());
        let mut parts = Vec::new();
        for (index, text) in item_blocks.iter().enumerate() {
            let first = if index == 0 {
                marker.as_str()
            } else {
                pad.as_str()
            };
            parts.push(prefix_lines(text, first, &pad));
        }
        blocks.push(parts.join("\n"));
    }
    if blocks.is_empty() {
        None
    } else {
        Some(blocks.join("\n"))
    }
}

fn order(node: &Value) -> u64 {
    let order = &node["attrs"]["order"];
    order
        .as_u64()
        .or_else(|| order.as_str().and_then(|text| text.parse().ok()))
        .unwrap_or(1)
}

fn prefix_lines(text: &str, first: &str, pad: &str) -> String {
    text.lines()
        .enumerate()
        .map(|(index, line)| {
            if line.is_empty() {
                String::new()
            } else {
                let prefix = if index == 0 { first } else { pad };
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn quote(node: &Value) -> Option<String> {
    let blocks = child_blocks(node);
    if blocks.is_empty() {
        return None;
    }
    let lines = blocks
        .join("\n\n")
        .lines()
        .map(|line| {
            if line.is_empty() {
                ">".to_string()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>();
    Some(lines.join("\n"))
}

fn table(node: &Value) -> Option<String> {
    let rows: Vec<Vec<String>> = node["content"]
        .as_array()?
        .iter()
        .filter(|row| kind(row) == "tableRow")
        .map(|row| {
            row["content"]
                .as_array()
                .map(|cells| cells.iter().map(cell_text).collect())
                .unwrap_or_default()
        })
        .filter(|cells: &Vec<String>| !cells.is_empty())
        .collect();
    let width = rows.iter().map(Vec::len).max()?;
    let line = |cells: &[String]| {
        let padded = (0..width)
            .map(|index| cells.get(index).cloned().unwrap_or_default())
            .collect::<Vec<_>>();
        format!("| {} |", padded.join(" | "))
    };
    let mut lines = vec![line(&rows[0]), line(&vec!["---".to_string(); width])];
    lines.extend(rows[1..].iter().map(|row| line(row)));
    Some(lines.join("\n"))
}

fn cell_text(cell: &Value) -> String {
    inline_children(cell)
        .lines()
        .map(|line| line.trim().trim_end_matches('\\').trim_end())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

fn code_block(node: &Value) -> String {
    let language = node["attrs"]["language"].as_str().unwrap_or("");
    let body = node["content"]
        .as_array()
        .map(|kids| kids.iter().map(scalar_text).collect::<String>())
        .unwrap_or_default();
    let body = body.trim_matches(['\n', '\r']).trim_end();
    if body.is_empty() {
        format!("```{language}\n```")
    } else {
        format!("```{language}\n{body}\n```")
    }
}

fn mention(node: &Value) -> String {
    if let Some(text) = attr_text(node) {
        return text;
    }
    node["attrs"]["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(|id| format!("@{id}"))
        .unwrap_or_default()
}

fn card(node: &Value) -> String {
    let url = node["attrs"]["url"]
        .as_str()
        .filter(|url| !url.is_empty())
        .or_else(|| {
            node["attrs"]["data"]["url"]
                .as_str()
                .filter(|url| !url.is_empty())
        });
    url.map(|url| format!("[{url}]({url})")).unwrap_or_default()
}

fn date(node: &Value) -> String {
    let stamp = &node["attrs"]["timestamp"];
    let millis = stamp
        .as_i64()
        .or_else(|| stamp.as_str().and_then(|text| text.parse().ok()));
    millis
        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
        .map(|at| at.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn attr_text(node: &Value) -> Option<String> {
    node["attrs"]["text"]
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn short_name(node: &Value) -> Option<String> {
    node["attrs"]["shortName"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

fn scalar_text(node: &Value) -> String {
    node["text"].as_str().unwrap_or("").to_string()
}

fn fallback_block(node: &Value) -> Option<String> {
    if let Value::String(text) = node {
        return non_empty(text);
    }
    if let Some(kids) = node["content"].as_array() {
        if kids.iter().any(|kid| is_block(kind(kid))) {
            let blocks = child_blocks(node);
            if !blocks.is_empty() {
                return Some(blocks.join("\n"));
            }
        } else {
            return non_empty(&inline_children(node));
        }
    }
    non_empty(&scalar_text(node))
}

fn non_empty(text: &str) -> Option<String> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text.trim_end().to_string())
    }
}

fn joined(blocks: Vec<String>) -> Option<String> {
    if blocks.is_empty() {
        None
    } else {
        Some(blocks.join("\n\n"))
    }
}

fn finish(blocks: Vec<String>) -> String {
    let text = blocks
        .into_iter()
        .filter(|block| !block.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let trimmed = text
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    trimmed.trim_end().to_string()
}
