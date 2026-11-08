//! parse：examples 里的文档渲染成 docx，再解析回 JSON，检查结构能对上，解析出来的 JSON 能再渲染。

use std::path::Path;

use fmtagno::components::Component;
use fmtagno::doc::Node;
use fmtagno::render::Format;
use serde_json::Value;

fn env() -> fmtagno::Env {
    fmtagno::load_env(Some(Path::new("examples")), &[], &[], None).unwrap()
}

fn to_docx(json: &str) -> Vec<u8> {
    fmtagno::render_with(json, Format::Docx, Some(Path::new("examples")), &env())
        .unwrap()
        .bytes
}

fn parse(bytes: &[u8]) -> fmtagno::parse::Parsed {
    fmtagno::parse::docx(
        bytes,
        &fmtagno::parse::Options {
            assets_dir: "x.assets".into(),
        },
    )
    .unwrap()
}

fn tree(json: &str) -> Node {
    fmtagno::prepare(json, &env()).unwrap().1
}

/// 按顺序列出块级元素的种类。parse 认不出的块（引用、提示框、代码、分割线）都算正文。
fn kinds(n: &Node, out: &mut Vec<String>) {
    let push = |out: &mut Vec<String>, s: &str| out.push(s.to_string());
    match &n.component {
        Component::Document(_) | Component::Section(_) => {
            for c in &n.children {
                kinds(c, out);
            }
        }
        Component::Heading(h) => out.push(format!("H{}", h.level)),
        Component::Text(_)
        | Component::Quote(_)
        | Component::Callout(_)
        | Component::Code(_)
        | Component::Divider(_) => push(out, "T"),
        Component::List(_) => push(out, "L"),
        Component::Table(_) => push(out, "Tb"),
        Component::Image(p) => {
            // SVG 放不进 docx，渲染成 alt 文字，也不出图片说明
            if p.src.ends_with(".svg") {
                push(out, "T");
            } else {
                push(out, "I");
                if p.caption.is_some() {
                    push(out, "T");
                }
            }
        }
        Component::Math(_) => push(out, "M"),
        Component::PageBreak(_) => push(out, "PB"),
        Component::TableOfContents(p) => {
            if p.title.is_some() {
                push(out, "T");
            }
            push(out, "TOC");
        }
        Component::Slide(p) => {
            if p.title.is_some() {
                push(out, "H2");
            }
            for c in &n.children {
                kinds(c, out);
            }
            push(out, "PB");
        }
    }
}

fn shape(root: &Node) -> Vec<String> {
    let mut out = Vec::new();
    for (i, sec) in root.children.iter().enumerate() {
        if i > 0 {
            out.push("S".into());
        }
        kinds(sec, &mut out);
    }
    out
}

fn headings(n: &Node, out: &mut Vec<String>) {
    match &n.component {
        Component::Heading(h) => out.push(h.text.plain_text()),
        Component::Slide(p) => {
            if let Some(t) = &p.title {
                out.push(t.clone());
            }
        }
        _ => {}
    }
    for c in &n.children {
        headings(c, out);
    }
}

fn find<'a>(n: &'a Node, pred: &dyn Fn(&Node) -> bool, out: &mut Vec<&'a Node>) {
    if pred(n) {
        out.push(n);
    }
    for c in &n.children {
        find(c, pred, out);
    }
}

const EXAMPLES: &[(&str, &str)] = &[
    ("hello", include_str!("../examples/hello.json")),
    ("showcase", include_str!("../examples/showcase.json")),
    ("blocks", include_str!("../examples/blocks.json")),
    ("styles", include_str!("../examples/styles.json")),
    ("borders", include_str!("../examples/borders.json")),
    ("math", include_str!("../examples/math.json")),
    ("sections", include_str!("../examples/sections.json")),
    ("pages", include_str!("../examples/pages.json")),
    ("custom", include_str!("../examples/custom.json")),
];

#[test]
fn examples_round_trip_structure() {
    for (name, json) in EXAMPLES {
        let original = tree(json);
        let parsed = parse(&to_docx(json));
        let text = serde_json::to_string(&parsed.json).unwrap();
        let back = fmtagno::prepare(&text, &fmtagno::Env::default())
            .unwrap_or_else(|e| panic!("{name}：parse 出来的 JSON 不合法：{e:#}\n{text}"));
        assert_eq!(
            shape(&original),
            shape(&back.1),
            "{name}：块的顺序和种类对不上"
        );
        let (mut a, mut b) = (Vec::new(), Vec::new());
        headings(&original, &mut a);
        headings(&back.1, &mut b);
        assert_eq!(a, b, "{name}：标题文字对不上");
        // 解析出来的 JSON 能再渲染成 docx
        let again = fmtagno::render_str(&text, Format::Docx, None).unwrap();
        assert!(again.bytes.starts_with(b"PK"));
    }
}

#[test]
fn second_round_is_stable() {
    // parse → render → parse，两次 parse 的 JSON 应该一样（图片路径相同）
    for (name, json) in EXAMPLES {
        let first = parse(&to_docx(json));
        let text = serde_json::to_string(&first.json).unwrap();
        let dir = std::env::temp_dir().join(format!("fmtagno-parse-stable-{name}"));
        std::fs::create_dir_all(dir.join("x.assets")).unwrap();
        for (n, b) in &first.assets {
            std::fs::write(dir.join("x.assets").join(n), b).unwrap();
        }
        let docx = fmtagno::render_str(&text, Format::Docx, Some(&dir))
            .unwrap()
            .bytes;
        let second = parse(&docx);
        assert_eq!(
            first.json["elements"], second.json["elements"],
            "{name}：第二轮解析出来不一样"
        );
        assert_eq!(
            first.json["styles"], second.json["styles"],
            "{name}：styles 不一样"
        );
    }
}

#[test]
fn merged_cells_widths_and_row_heights() {
    let json = r##"{
      "root": "doc",
      "elements": {
        "doc": { "type": "Document", "children": ["t"] },
        "t": {
          "type": "Table",
          "props": {
            "widths": [1, 2, 4],
            "rowHeights": ["20pt", { "height": "36pt", "rule": "exact" }, null],
            "rows": [
              [ { "text": "一", "rowSpan": 2, "style": { "borderLeft": "6pt solid #8DC63F", "background": "#EEEEEE" } },
                { "text": "标题", "colSpan": 2, "style": { "align": "center", "bold": true } } ],
              [ "中", { "text": "右", "style": { "verticalAlign": "middle" } } ],
              [ "a", "b", "c" ]
            ]
          }
        }
      }
    }"##;
    let parsed = parse(&to_docx(json));
    let t = &parsed.json["elements"]["table1"]["props"];
    let rows = t["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0][0]["rowSpan"], 2);
    assert_eq!(rows[0][0]["style"]["borderLeft"], "6pt solid #8DC63F");
    assert_eq!(rows[0][0]["style"]["background"], "#EEEEEE");
    assert_eq!(rows[0][1]["colSpan"], 2);
    assert_eq!(rows[0][1]["style"]["align"], "center");
    assert_eq!(rows[1].as_array().unwrap().len(), 2);
    assert_eq!(rows[1][1]["style"]["verticalAlign"], "middle");
    let w: Vec<f64> = t["widths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    assert!(
        (w[1] / w[0] - 2.0).abs() < 0.01 && (w[2] / w[0] - 4.0).abs() < 0.01,
        "{w:?}"
    );
    assert_eq!(t["rowHeights"][0], "20pt");
    assert_eq!(t["rowHeights"][1]["rule"], "exact");
    assert!(t["rowHeights"][2].is_null());
}

#[test]
fn nested_lists_keep_levels_and_kinds() {
    let json = r##"{
      "root": "doc",
      "elements": {
        "doc": { "type": "Document", "children": ["l"] },
        "l": {
          "type": "List",
          "props": {
            "ordered": true,
            "items": [
              "一",
              { "text": "二", "items": ["二.1", { "text": "二.2", "items": ["深"] }] },
              { "text": "三", "ordered": false, "items": ["点"] }
            ]
          },
          "parts": { "level2": { "listMarkerFormat": "(%2)" } }
        }
      }
    }"##;
    let parsed = parse(&to_docx(json));
    let l = &parsed.json["elements"]["list1"];
    assert_eq!(l["props"]["ordered"], true);
    let items = l["props"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[1]["items"][1]["items"][0], "深");
    // 同一级在 Word 里只有一种编号定义，第三项的子列表跟着第二项的走（渲染器的限制），这里不查
    assert_eq!(l["parts"]["level2"]["listMarkerFormat"], "(%2)");
}

#[test]
fn math_becomes_latex_again() {
    let json = include_str!("../examples/math.json");
    let parsed = parse(&to_docx(json));
    let els = parsed.json["elements"].as_object().unwrap();
    let maths: Vec<&Value> = els.values().filter(|e| e["type"] == "Math").collect();
    assert_eq!(maths.len(), 7);
    for m in &maths {
        let latex = m["props"]["latex"].as_str().unwrap();
        fmtagno::math::to_mathml(latex, true).unwrap_or_else(|e| panic!("{latex}：{e}"));
    }
    assert!(maths.iter().any(|m| m["props"]["number"] == 1));
    let text = serde_json::to_string(&parsed.json).unwrap();
    assert!(text.contains("InlineMath"));
    assert!(text.contains(r"\frac{n(n+1)(2n+1)}{6}"), "{text}");
    assert!(text.contains("pmatrix") && text.contains("cases"));
}

#[test]
fn all_border_styles_come_back_by_name() {
    let json = include_str!("../examples/borders.json");
    let original = tree(json);
    let parsed = parse(&to_docx(json));
    let text = serde_json::to_string(&parsed.json).unwrap();
    let mut texts = Vec::new();
    find(
        &original,
        &|n| matches!(n.component, Component::Text(_)),
        &mut texts,
    );
    for n in texts {
        if let Some(b) = &n.style.border {
            let name = serde_json::to_value(b.style).unwrap();
            let name = name.as_str().unwrap();
            assert!(
                text.contains(&format!(" {name} ")),
                "线型 {name} 没解析回来"
            );
        }
    }
}

#[test]
fn sections_headers_and_page_numbers() {
    let json = include_str!("../examples/sections.json");
    let parsed = parse(&to_docx(json));
    let els = parsed.json["elements"].as_object().unwrap();
    let sections: Vec<&Value> = els.values().filter(|e| e["type"] == "Section").collect();
    assert_eq!(
        sections.len(),
        2,
        "第一节放在 meta.page，后面两节是 Section"
    );
    let text = serde_json::to_string(&parsed.json).unwrap();
    assert!(text.contains(r#""field":"page""#));
    assert!(text.contains(r#""pageNumberStart":1"#));
    assert!(text.contains(r#""landscape":true"#));
    assert!(text.contains(r#""even""#));
    let page = &parsed.json["meta"]["page"];
    assert!(page["header"].is_object() || page["header"].is_string());
}

#[test]
fn images_are_exported_next_to_json() {
    let dir = std::env::temp_dir().join("fmtagno-parse-img");
    std::fs::create_dir_all(&dir).unwrap();
    let tmp =
        dir.join(format!("dot-{:?}.png", std::thread::current().id()).replace(['(', ')'], ""));
    image::RgbImage::from_pixel(8, 4, image::Rgb([10, 120, 200]))
        .save(&tmp)
        .unwrap();
    std::fs::rename(&tmp, dir.join("dot.png")).unwrap();
    let json = r##"{
      "root": "d",
      "elements": {
        "d": { "type": "Document", "children": ["i", "p"] },
        "i": { "type": "Image", "props": { "src": "dot.png", "width": 80, "alt": "点" }, "style": { "horizontalAlign": "center" } },
        "p": { "type": "Text", "props": { "text": ["前", { "image": "dot.png", "width": 16, "height": 8 }, "后"] } }
      }
    }"##;
    let docx = fmtagno::render_str(json, Format::Docx, Some(&dir))
        .unwrap()
        .bytes;
    let parsed = parse(&docx);
    assert!(!parsed.assets.is_empty());
    let img = &parsed.json["elements"]["image1"];
    assert_eq!(img["type"], "Image");
    assert!(
        img["props"]["src"]
            .as_str()
            .unwrap()
            .starts_with("x.assets/")
    );
    assert_eq!(img["props"]["width"], 80);
    assert_eq!(img["style"]["horizontalAlign"], "center");
    let runs = parsed.json["elements"]["p1"]["props"]["text"]
        .as_array()
        .unwrap();
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[1]["width"], 16);
}

#[test]
fn not_a_docx_is_an_error() {
    let e = fmtagno::parse::docx(
        b"hello",
        &fmtagno::parse::Options {
            assets_dir: "a".into(),
        },
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("zip"));
}
