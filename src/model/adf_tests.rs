//! Tests for model::adf: ADF JSON to markdown.

use super::*;
use serde_json::json;

fn md(doc: serde_json::Value) -> String {
    adf_to_markdown(&doc)
}

fn para(nodes: serde_json::Value) -> serde_json::Value {
    json!({"type": "paragraph", "content": nodes})
}

fn text(value: &str) -> serde_json::Value {
    json!({"type": "text", "text": value})
}

fn item(nodes: serde_json::Value) -> serde_json::Value {
    json!({"type": "listItem", "content": nodes})
}

fn marked(value: &str, mark: serde_json::Value) -> serde_json::Value {
    json!({"type": "text", "text": value, "marks": [mark]})
}

#[test]
fn realistic_description_renders_full_markdown() {
    let doc = json!({
        "type": "doc",
        "version": 1,
        "content": [
            {"type": "heading", "attrs": {"level": 2}, "content": [text("Release checklist")]},
            {"type": "paragraph", "content": [
                text("Ping "),
                {"type": "mention", "attrs": {"id": "557058:abc", "text": "@Dana"}},
                text(" on "),
                marked("slash", json!({"type": "strong"})),
                text(" and read the "),
                marked(
                    "runbook",
                    json!({"type": "link", "attrs": {"href": "https://example.com/runbook"}}),
                ),
                text(".")
            ]},
            {"type": "bulletList", "content": [
                item(json!([para(json!([text("Bump the version")]))])),
                item(json!([
                    para(json!([text("Tag the release")])),
                    {"type": "bulletList", "content": [
                        item(json!([para(json!([text("signed tag")]))]))
                    ]}
                ]))
            ]},
            {"type": "codeBlock", "attrs": {"language": "bash"},
             "content": [text("git tag -s v1\n")]},
            {"type": "table", "content": [
                {"type": "tableRow", "content": [
                    {"type": "tableHeader", "content": [para(json!([text("Item")]))]},
                    {"type": "tableHeader", "content": [para(json!([text("State")]))]}
                ]},
                {"type": "tableRow", "content": [
                    {"type": "tableCell", "content": [para(json!([text("docs | site")]))]},
                    {"type": "tableCell", "content": [para(json!([text("done")]))]}
                ]}
            ]}
        ]
    });
    let expected = concat!(
        "## Release checklist\n",
        "\n",
        "Ping @Dana on **slash** and read the [runbook](https://example.com/runbook).\n",
        "\n",
        "- Bump the version\n",
        "- Tag the release\n",
        "  - signed tag\n",
        "\n",
        "```bash\n",
        "git tag -s v1\n",
        "```\n",
        "\n",
        "| Item | State |\n",
        "| --- | --- |\n",
        "| docs \\| site | done |",
    );
    assert_eq!(md(doc), expected);
}

#[test]
fn headings_carry_their_level_and_clamp_to_six() {
    let heading = |level: serde_json::Value| json!({"type": "heading", "attrs": {"level": level}, "content": [text("T")]});
    assert_eq!(md(heading(json!(1))), "# T");
    assert_eq!(md(heading(json!(4))), "#### T");
    assert_eq!(md(heading(json!(9))), "###### T");
    assert_eq!(
        md(json!({"type": "heading", "content": [text("T")]})),
        "# T",
        "missing level falls back to 1"
    );
}

#[test]
fn ordered_lists_count_from_their_order_attr() {
    let doc = json!({
        "type": "orderedList",
        "attrs": {"order": 3},
        "content": [
            item(json!([para(json!([text("third")]))])),
            item(json!([para(json!([text("fourth")]))]))
        ]
    });
    assert_eq!(md(doc), "3. third\n4. fourth");
}

#[test]
fn nested_lists_indent_under_their_parent_marker() {
    let doc = json!({
        "type": "orderedList",
        "attrs": {"order": 9},
        "content": [
            item(json!([
                para(json!([text("nine")])),
                {"type": "orderedList", "attrs": {"order": 1}, "content": [
                    item(json!([para(json!([text("one")]))]))
                ]}
            ])),
            item(json!([para(json!([text("ten")]))]))
        ]
    });
    assert_eq!(md(doc), "9. nine\n   1. one\n10. ten");
}

#[test]
fn code_block_fences_with_its_language() {
    let doc = json!({
        "type": "codeBlock",
        "attrs": {"language": "rust"},
        "content": [text("fn main() {}\n")]
    });
    assert_eq!(md(doc), "```rust\nfn main() {}\n```");
    let bare = json!({"type": "codeBlock", "content": [text("plain\n\n")]});
    assert_eq!(md(bare), "```\nplain\n```");
}

#[test]
fn quotes_and_panels_render_as_blockquotes() {
    let quoted = json!({
        "type": "blockquote",
        "content": [para(json!([text("quoted")]))]
    });
    assert_eq!(md(quoted), "> quoted");
    let panel = json!({
        "type": "panel",
        "attrs": {"panelType": "info"},
        "content": [para(json!([text("note")]))]
    });
    assert_eq!(md(panel), "> note");
}

#[test]
fn rule_becomes_a_horizontal_rule() {
    assert_eq!(md(json!({"type": "rule"})), "---");
}

#[test]
fn table_escapes_pipes_and_pads_short_rows() {
    let doc = json!({
        "type": "table",
        "content": [
            {"type": "tableRow", "content": [
                {"type": "tableHeader", "content": [para(json!([text("Key | value")]))]},
                {"type": "tableHeader", "content": [para(json!([text("Note")]))]}
            ]},
            {"type": "tableRow", "content": [
                {"type": "tableCell", "content": [
                    para(json!([text("a | b")])),
                    para(json!([text("continued")]))
                ]}
            ]}
        ]
    });
    assert_eq!(
        md(doc),
        "| Key \\| value | Note |\n| --- | --- |\n| a \\| b continued |  |"
    );
}

#[test]
fn inline_marks_wrap_text() {
    let doc = para(json!([
        marked("bold", json!({"type": "strong"})),
        text(" "),
        marked("italic", json!({"type": "em"})),
        text(" "),
        marked("code", json!({"type": "code"})),
        text(" "),
        marked("gone", json!({"type": "strike"})),
        text(" "),
        marked(
            "site",
            json!({"type": "link", "attrs": {"href": "https://x.test"}})
        ),
        text(" plain")
    ]));
    assert_eq!(
        md(doc),
        "**bold** _italic_ `code` ~~gone~~ [site](https://x.test) plain"
    );
}

#[test]
fn unrendered_marks_keep_their_text() {
    let doc = para(json!([
        marked("under", json!({"type": "underline"})),
        text(" "),
        marked(
            "colored",
            json!({"type": "textColor", "attrs": {"color": "#fff"}})
        ),
        text(" "),
        marked("super", json!({"type": "subsup", "attrs": {"type": "sup"}}))
    ]));
    assert_eq!(md(doc), "under colored super");
}

#[test]
fn mentions_statuses_and_emojis_render_their_text() {
    let doc = para(json!([
        {"type": "mention", "attrs": {"id": "acc-1"}},
        text(" "),
        {"type": "status", "attrs": {"text": "In progress"}},
        text(" "),
        {"type": "emoji", "attrs": {"shortName": ":rocket:", "text": "🚀"}},
        text(" "),
        {"type": "emoji", "attrs": {"shortName": ":rocket:"}}
    ]));
    assert_eq!(md(doc), "@acc-1 In progress 🚀 :rocket:");
}

#[test]
fn dates_render_as_iso_days() {
    let doc = para(json!([{"type": "date", "attrs": {"timestamp": 1735689600000_i64}}]));
    assert_eq!(md(doc), "2025-01-01");
    let bogus = para(json!([{"type": "date", "attrs": {"timestamp": "soon"}}]));
    assert_eq!(md(bogus), "");
}

#[test]
fn cards_become_links_and_media_becomes_a_placeholder() {
    let card = para(json!([
        {"type": "inlineCard", "attrs": {"url": "https://jira.example/browse/ABC-1"}}
    ]));
    assert_eq!(
        md(card),
        "[https://jira.example/browse/ABC-1](https://jira.example/browse/ABC-1)"
    );
    let block_card = json!({"type": "blockCard", "attrs": {"data": {"url": "https://x.test"}}});
    assert_eq!(md(block_card), "[https://x.test](https://x.test)");
    let media = json!({
        "type": "mediaSingle",
        "attrs": {"layout": "center"},
        "content": [{"type": "media", "attrs": {"id": "asset-1"}}]
    });
    assert_eq!(md(media), "[attachment]");
    assert_eq!(
        md(para(json!([{"type": "mediaInline", "attrs": {}}]))),
        "[attachment]"
    );
}

#[test]
fn hard_break_starts_a_new_line() {
    let doc = para(json!([text("one"), {"type": "hardBreak"}, text("two")]));
    assert_eq!(md(doc), "one\\\ntwo");
}

#[test]
fn unknown_nodes_degrade_to_their_text() {
    let doc = json!({
        "type": "doc",
        "content": [
            {"type": "mystery", "content": [text("kept")]},
            {"type": "mystery", "content": [para(json!([text("block")]))]},
            {"type": "mystery", "text": "scalar"}
        ]
    });
    assert_eq!(md(doc), "kept\n\nblock\n\nscalar");
    let inline_unknown = para(json!([{"type": "mystery", "content": [text("x")]}]));
    assert_eq!(md(inline_unknown), "x");
}

#[test]
fn blocks_are_separated_by_one_blank_line() {
    let doc = json!({
        "type": "doc",
        "content": [
            para(json!([text("a")])),
            para(json!([])),
            para(json!([text("b")]))
        ]
    });
    assert_eq!(md(doc), "a\n\nb");
    assert_eq!(md(json!("bare string")), "bare string");
}

#[test]
fn malformed_input_never_panics() {
    let inputs = vec![
        json!(null),
        json!(42),
        json!([1, 2, {"type": "doc"}]),
        json!({"type": "doc", "content": "not an array"}),
        json!({"type": "doc", "content": [null, 1, "text", {"type": 7}]}),
        para(json!([{"type": "text", "text": 9}])),
        json!({"type": "heading", "attrs": {"level": "deep"}, "content": []}),
        json!({"type": "bulletList", "content": [{"type": "listItem"}]}),
        json!({"type": "table", "content": [{"type": "tableRow", "content": "x"}]}),
        json!({"type": "table", "content": []}),
        json!({"type": "mention", "attrs": {"id": 5, "text": null}}),
        json!({"type": "date", "attrs": {"timestamp": "soon"}}),
        json!({"type": "text", "text": "x", "marks": [{"type": "link", "attrs": {"href": 3}}]}),
        json!({"type": "codeBlock", "attrs": {"language": 3}, "content": [null]}),
        json!({"type": "unknown"}),
    ];
    for input in inputs {
        let out = adf_to_markdown(&input);
        for line in out.lines() {
            assert_eq!(
                line,
                line.trim_end(),
                "trailing whitespace for {input}: {out:?}"
            );
        }
        assert_eq!(
            out,
            out.trim_end(),
            "trailing blank line for {input}: {out:?}"
        );
    }
    assert_eq!(
        md(json!({"type": "doc", "content": [null, 1, "text"]})),
        "text"
    );
    assert_eq!(
        md(json!({"type": "heading", "attrs": {"level": "deep"}, "content": []})),
        "#"
    );
}
