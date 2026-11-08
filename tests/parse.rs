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
        Component::ColumnBreak(_) => push(out, "CB"),
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
    ("frontmatter", include_str!("../examples/frontmatter.json")),
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
        "i": { "type": "Image", "props": { "src": "dot.png", "width": 80, "alt": "点 \"蓝\" & <小>" }, "style": { "horizontalAlign": "center" } },
        "p": { "type": "Text", "props": { "text": ["前", { "image": "dot.png", "text": "行内小点", "width": 16, "height": 8 }, "后"] } }
      }
    }"##;
    let docx = fmtagno::render_str(json, Format::Docx, Some(&dir))
        .unwrap()
        .bytes;
    // 替代文字写进 wp:docPr 的 descr，每张图的 id 不重复
    let xml = String::from_utf8_lossy(&docx);
    assert!(
        xml.contains(
            r#"<wp:docPr id="1" name="Picture 1" descr="点 &quot;蓝&quot; &amp; &lt;小&gt;" />"#
        ),
        "块级图片的替代文字"
    );
    assert!(xml.contains(r#"<wp:docPr id="2" name="Picture 2" descr="行内小点" />"#));
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
    assert_eq!(img["props"]["alt"], "点 \"蓝\" & <小>");
    assert_eq!(img["style"]["horizontalAlign"], "center");
    let runs = parsed.json["elements"]["p1"]["props"]["text"]
        .as_array()
        .unwrap();
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[1]["width"], 16);
    assert_eq!(runs[1]["text"], "行内小点");
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

#[test]
fn page_number_formats_and_column_breaks_come_back() {
    let json = include_str!("../examples/frontmatter.json");
    let parsed = parse(&to_docx(json));
    let w = parsed.warnings.join("\n");
    assert!(!w.contains("页码格式"), "{w}");
    assert!(!w.contains("分栏符"), "{w}");
    let page = &parsed.json["meta"]["page"];
    assert_eq!(page["pageNumberFormat"], "lowerRoman");
    assert_eq!(page["pageNumberStart"], 1);
    // 后面的节只写和上一节不同的；正文换回 decimal 要写出来
    let els = &parsed.json["elements"];
    let secs: Vec<&Value> = parsed.json["elements"]["doc"]["children"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|id| els.get(id.as_str()?))
        .filter(|e| e["type"] == "Section")
        .collect();
    let fmts: Vec<&Value> = secs
        .iter()
        .map(|s| &s["props"]["pageNumberFormat"])
        .collect();
    assert_eq!(fmts, ["decimal", "upperLetter", "chineseCounting"]);
    let cbs = els
        .as_object()
        .unwrap()
        .values()
        .filter(|e| e["type"] == "ColumnBreak")
        .count();
    assert_eq!(cbs, 1);
    // 再渲染回 docx，页码格式还在
    let text = serde_json::to_string(&parsed.json).unwrap();
    let again = fmtagno::render_str(&text, Format::Docx, None).unwrap();
    let x = String::from_utf8_lossy(&again.bytes);
    assert!(x.contains(r#"w:fmt="lowerRoman""#));
    assert!(x.contains(r#"w:fmt="chineseCounting""#));
    assert!(x.contains(r#"<w:br w:type="column" />"#));
}

/// 改 docx 里的 word/document.xml。
fn edit_document(docx: &[u8], f: impl Fn(String) -> String) -> Vec<u8> {
    use std::io::{Read, Write};
    let mut zin = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..zin.len() {
        let mut e = zin.by_index(i).unwrap();
        let name = e.name().to_string();
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).unwrap();
        if name == "word/document.xml" {
            buf = f(String::from_utf8(buf).unwrap()).into_bytes();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        out.write_all(&buf).unwrap();
    }
    out.finish().unwrap().into_inner()
}

#[test]
fn unknown_page_number_format_warns() {
    let json = r#"{"meta":{"page":{"pageNumberFormat":"upperRoman"}},"root":"d","elements":{"d":{"type":"Document","children":["t"]},"t":{"type":"Text","props":{"text":"a"}}}}"#;
    let docx = edit_document(&to_docx(json), |x| {
        x.replace(r#"w:fmt="upperRoman""#, r#"w:fmt="ordinal""#)
    });
    let parsed = parse(&docx);
    assert!(
        parsed
            .warnings
            .iter()
            .any(|w| w.contains("不认识的页码格式 `ordinal`")),
        "{:?}",
        parsed.warnings
    );
    assert!(
        parsed.json["meta"]["page"]
            .get("pageNumberFormat")
            .is_none()
    );
}

#[test]
fn image_alt_falls_back_to_title() {
    let dir = std::env::temp_dir().join("fmtagno-parse-title");
    std::fs::create_dir_all(&dir).unwrap();
    let tmp =
        dir.join(format!("dot-{:?}.png", std::thread::current().id()).replace(['(', ')'], ""));
    image::RgbImage::from_pixel(4, 4, image::Rgb([200, 20, 20]))
        .save(&tmp)
        .unwrap();
    std::fs::rename(&tmp, dir.join("dot.png")).unwrap();
    let json = r#"{"root":"d","elements":{"d":{"type":"Document","children":["i"]},"i":{"type":"Image","props":{"src":"dot.png","width":20}}}}"#;
    let docx = fmtagno::render_str(json, Format::Docx, Some(&dir))
        .unwrap()
        .bytes;
    // 没写 descr、只有 title 的图（别的软件这么存）
    let docx = edit_document(&docx, |x| {
        x.replace(
            r#"<wp:docPr id="1" name="Picture 1" />"#,
            r#"<wp:docPr id="1" name="Picture 1" title="红点" />"#,
        )
    });
    let parsed = parse(&docx);
    assert_eq!(parsed.json["elements"]["image1"]["props"]["alt"], "红点");
}

const WHITE_HEADER: &str = r##"{
  "root": "d",
  "elements": {
    "d": { "type": "Document", "children": ["t"] },
    "t": {
      "type": "Table",
      "props": { "columns": ["名称", { "text": "数量", "style": { "background": "#DDEEFF" } }], "rows": [["a", "1"], ["b", "2"]] },
      "parts": { "header": { "background": "none" } }
    }
  }
}"##;

#[test]
fn white_table_header_stays_white() {
    // 渲染：三种格式都不画内置的灰底，单格自己的底纹照画
    let x = String::from_utf8_lossy(&to_docx(WHITE_HEADER)).into_owned();
    assert!(!x.contains("F3F3F3"), "docx 不该有灰底");
    assert!(x.contains("DDEEFF"));
    assert!(x.contains("<w:tblHeader"), "表头跨页重复");
    let h = fmtagno::render_str(WHITE_HEADER, Format::Html, None).unwrap();
    let h = String::from_utf8(h.bytes).unwrap();
    assert!(!h.to_uppercase().contains("F3F3F3"), "html 不该有灰底");
    assert!(h.to_uppercase().contains("DDEEFF"));
    let (d, tree) = fmtagno::prepare(WHITE_HEADER, &fmtagno::Env::default()).unwrap();
    let (src, _, _) = fmtagno::render::pdf::markup(&d, &tree, &Default::default());
    assert!(!src.to_uppercase().contains("F3F3F3"), "pdf 不该有灰底");
    assert!(src.to_uppercase().contains("DDEEFF"));
    // 往返：还是 columns，parts.header 写 background none，再渲染也不变灰
    let parsed = parse(&to_docx(WHITE_HEADER));
    let t = &parsed.json["elements"]["table1"];
    assert_eq!(t["props"]["columns"].as_array().unwrap().len(), 2, "{t}");
    assert_eq!(t["parts"]["header"]["background"], "none", "{t}");
    assert_eq!(t["props"]["columns"][1]["style"]["background"], "#DDEEFF");
    let text = serde_json::to_string(&parsed.json).unwrap();
    let again = fmtagno::render_str(&text, Format::Docx, None).unwrap();
    let again = String::from_utf8_lossy(&again.bytes);
    assert!(!again.contains("F3F3F3"));
    assert!(again.contains("<w:tblHeader"));
}

#[test]
fn header_is_decided_by_repeat_header_rows() {
    // 默认灰底表头往返：还是 columns，不写 background
    let grey = r#"{"root":"d","elements":{"d":{"type":"Document","children":["t"]},"t":{"type":"Table","props":{"columns":["甲","乙"],"rows":[["1","2"]]}}}}"#;
    let parsed = parse(&to_docx(grey));
    let t = &parsed.json["elements"]["table1"];
    assert!(t["props"]["columns"].is_array(), "{t}");
    assert!(
        t.get("parts").and_then(|p| p.get("header")).is_none(),
        "{t}"
    );
    // 不重复表头：第一行虽然有灰底，也当普通行
    let no_repeat = r#"{"root":"d","elements":{"d":{"type":"Document","children":["t"]},"t":{"type":"Table","props":{"columns":["甲","乙"],"rows":[["1","2"]]},"style":{"repeatHeader":false}}}}"#;
    let x = String::from_utf8_lossy(&to_docx(no_repeat)).into_owned();
    assert!(!x.contains("<w:tblHeader"));
    let parsed = parse(&to_docx(no_repeat));
    let t = &parsed.json["elements"]["table1"];
    assert!(t["props"].get("columns").is_none(), "{t}");
    assert_eq!(t["props"]["rows"].as_array().unwrap().len(), 2);
}

#[test]
fn multi_row_repeat_header_keeps_first_row() {
    let json = r#"{"root":"d","elements":{"d":{"type":"Document","children":["t"]},"t":{"type":"Table","props":{"columns":["甲","乙"],"rows":[["1","2"],["3","4"]]}}}}"#;
    // 第二行也标成重复标题行
    let docx = edit_document(&to_docx(json), |x| {
        let first = x.find("<w:tblHeader").unwrap();
        let end = first + x[first..].find("</w:tr>").unwrap();
        let pr = end + x[end..].find("<w:trPr").unwrap();
        let (head, tail) = x.split_at(pr);
        let tail = if let Some(t) = tail.strip_prefix("<w:trPr />") {
            format!("<w:trPr><w:tblHeader /></w:trPr>{t}")
        } else {
            tail.replacen("<w:trPr>", "<w:trPr><w:tblHeader />", 1)
        };
        format!("{head}{tail}")
    });
    let parsed = parse(&docx);
    assert!(
        parsed.warnings.iter().any(|w| w.contains("表头有好几行")),
        "{:?}",
        parsed.warnings
    );
    let t = &parsed.json["elements"]["table1"];
    let c0 = &t["props"]["columns"][0];
    assert_eq!(c0.as_str().or(c0["text"].as_str()), Some("甲"), "{t}");
    assert_eq!(t["props"]["rows"].as_array().unwrap().len(), 2);
}

#[test]
fn word_auto_line_spacing_comes_back_as_multiple() {
    let json = r#"{"root":"d","elements":{"d":{"type":"Document","children":["a","b"]},"a":{"type":"Text","props":{"text":"一"},"style":{"lineHeight":{"multiple":1.15}}},"b":{"type":"Text","props":{"text":"二"},"style":{"lineHeight":1.5}}}}"#;
    let docx = to_docx(json);
    let x = String::from_utf8_lossy(&docx).into_owned();
    assert!(x.contains(r#"w:line="276" w:lineRule="auto""#));
    let parsed = parse(&docx);
    let text = serde_json::to_string(&parsed.json).unwrap();
    assert!(text.contains(r#""lineHeight":{"multiple":1.15}"#), "{text}");
    // fmtagno 自己的倍数（写成「至少」）还是读回数字
    assert!(!text.contains(r#""lineHeight":{"multiple":1.5}"#), "{text}");
    // 第二轮不变
    let again = parse(
        &fmtagno::render_str(&text, Format::Docx, None)
            .unwrap()
            .bytes,
    );
    assert_eq!(parsed.json["elements"], again.json["elements"]);
}

#[test]
fn doc_grid_lines_warns() {
    let json = r#"{"root":"d","elements":{"d":{"type":"Document","children":["a"]},"a":{"type":"Text","props":{"text":"一"}}}}"#;
    let docx = edit_document(&to_docx(json), |x| {
        if x.contains("<w:docGrid ") {
            x.replacen("<w:docGrid ", r#"<w:docGrid w:type="lines" "#, 1)
        } else {
            x.replacen(
                "</w:sectPr>",
                r#"<w:docGrid w:type="lines" w:linePitch="312" /></w:sectPr>"#,
                1,
            )
        }
    });
    let parsed = parse(&docx);
    assert!(
        parsed.warnings.iter().any(|w| w.contains("docGrid")),
        "{:?}",
        parsed.warnings
    );
    // 没有 type 的 docGrid（Word 默认）不警告
    let plain = parse(&to_docx(json));
    assert!(!plain.warnings.iter().any(|w| w.contains("docGrid")));
}
