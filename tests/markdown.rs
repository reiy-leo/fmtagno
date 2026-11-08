use fmtagno::render::Format;

fn md(json: &str) -> (String, Vec<String>) {
    let out = fmtagno::render_str(json, Format::Markdown, None).unwrap();
    (String::from_utf8(out.bytes).unwrap(), out.warnings)
}

#[test]
fn blocks_example() {
    let (s, _) = md(include_str!("../examples/blocks.json"));
    assert!(s.starts_with("<a id=\"title\"></a>\n# 季度报告\n"), "{s}");
    assert!(
        s.contains(
            "**目录**\n\n- [季度报告](#title)\n  - [一、概况](#h-a)\n  - [二、下季度计划](#h-b)"
        ),
        "{s}"
    );
    assert!(s.contains("<a id=\"h-a\"></a>\n## 一、概况"), "{s}");
    // 3 级标题没进目录，不加锚点
    assert!(s.contains("\n\n### 2.1 招聘"), "{s}");
    assert!(
        s.contains("> 导出 Word 和 PDF 终于一模一样了，**省了我们整整一天。**"),
        "{s}"
    );
    assert!(
        s.contains("> [!TIP]\n> 收入按到账日期统计，不含预收款。"),
        "{s}"
    );
    assert!(s.contains("> [!WARNING]\n> 第三个月"), "{s}");
    assert!(s.contains("\n\n---\n\n"), "{s}");
}

#[test]
fn lists_tables_code() {
    let json = r##"{
      "root": "d",
      "elements": {
        "d": { "type": "Document", "children": ["l", "t", "m", "c", "pb", "cb", "p"] },
        "l": { "type": "List", "props": { "ordered": true, "items": [
          "一", { "text": "二", "items": ["二.1", "二.2"], "ordered": false }, "三" ] } },
        "t": { "type": "Table", "props": { "columns": ["名称", "数量"], "rows": [["a|b", "1"], ["c", "2"]] } },
        "m": { "type": "Table", "props": { "rows": [[{ "text": "合并", "colSpan": 2 }], ["x", "y"]] } },
        "c": { "type": "Code", "props": { "language": "rust", "code": "let a = `b`;\n" } },
        "pb": { "type": "PageBreak" },
        "cb": { "type": "ColumnBreak" },
        "p": { "type": "Text", "props": { "text": ["第一行\n第二行 ", { "text": "链接", "href": "https://example.com", "italic": true }] } }
      }
    }"##;
    let (s, w) = md(json);
    assert!(
        s.contains("1. 一\n2. 二\n   - 二.1\n   - 二.2\n3. 三"),
        "{s}"
    );
    assert!(
        s.contains("| 名称 | 数量 |\n| --- | --- |\n| a\\|b | 1 |"),
        "{s}"
    );
    assert!(s.contains("<td colspan=\"2\">合并</td>"), "{s}");
    assert!(s.contains("```rust\nlet a = `b`;\n```"), "{s}");
    assert!(
        s.contains("第一行\\\n第二行 [*链接*](https://example.com)"),
        "{s}"
    );
    assert!(w.iter().any(|x| x.contains("PageBreak")), "{w:?}");
    assert!(w.iter().any(|x| x.contains("ColumnBreak")), "{w:?}");
}
