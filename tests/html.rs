use fmtagno::render::Format;

fn html(json: &str) -> String {
    String::from_utf8(
        fmtagno::render_str(json, Format::Html, Some(std::path::Path::new("examples")))
            .unwrap()
            .bytes,
    )
    .unwrap()
}

#[test]
fn hello_example_renders() {
    let s = html(include_str!("../examples/hello.json"));
    assert!(s.contains(">季度报告</h1>"));
    assert!(s.contains(">格式</th>"));
    assert!(s.contains("color:#666666"));
    assert!(s.contains("@page { size: 595.2756pt 841.8898pt;"));
}

#[test]
fn escapes_text_and_css() {
    let s = html(
        r##"{
      "root": "d",
      "elements": {
        "d": { "type": "Document", "children": ["t"] },
        "t": { "type": "Text", "props": { "text": [{ "text": "<b>x</b>", "style": { "fontFamily": "x\";}body{display:none" } }] } }
      }
    }"##,
    );
    assert!(s.contains("&lt;b&gt;x&lt;/b&gt;"));
    assert!(!s.contains("}body{"));
}

#[test]
fn rich_runs_and_slide() {
    let s = html(
        r##"{
      "root": "d",
      "elements": {
        "d": { "type": "Document", "children": ["s"] },
        "s": { "type": "Slide", "props": { "title": "封面" }, "children": ["t"] },
        "t": { "type": "Text", "props": { "text": [{ "text": "粗", "bold": true }, { "text": "链", "href": "https://example.com" }] } }
      }
    }"##,
    );
    assert!(s.contains("class=\"fmtagno-slide\""));
    assert!(s.contains("font-weight:700;\">粗</span>"));
    assert!(s.contains("<a href=\"https://example.com\"><span"));
}

fn err(json: &str) -> String {
    match fmtagno::render_str(json, Format::Html, None) {
        Ok(_) => panic!("应该报错"),
        Err(e) => format!("{e:#}"),
    }
}

#[test]
fn rejects_bad_documents() {
    assert!(err(r#"{"root":"x","elements":{}}"#).contains("`x` 不存在"));
    assert!(err(r#"{"root":"d","elements":{"d":{"type":"Nope"}}}"#).contains("没注册"));
    assert!(
        err(r#"{"root":"d","elements":{"d":{"type":"Heading","props":{"level":9,"text":"a"}}}}"#)
            .contains("level")
    );
    assert!(
        err(
            r#"{"root":"d","elements":{"d":{"type":"Text","props":{"text":"a"},"children":["d"]}}}"#
        )
        .contains("不能有 children")
    );
    assert!(
        err(r#"{"root":"d","elements":{"d":{"type":"Document","children":["d"]}}}"#)
            .contains("不止一次")
    );
    assert!(
        err(r#"{"root":"d","elements":{"d":{"type":"Text","props":{"text":"a"},"style":"nope"}}}"#)
            .contains("nope")
    );
}

#[test]
fn showcase_example_renders() {
    let s = html(include_str!("../examples/showcase.json"));
    assert!(s.contains("class=\"fmtagno-list\" id=\"steps\""));
    assert!(s.contains(">1.</span>"));
    assert!(s.contains("<code style="));
    assert!(s.contains("<col style=\"width:20.00%\">"));
    assert!(s.contains("class=\"fmtagno-pagebreak\""));
    assert!(s.contains("font-size:11pt"));
    assert!(s.contains("src=\"data:image/svg+xml;base64,"));
}

#[test]
fn paged_mode_embeds_pagedjs() {
    let json = include_str!("../examples/pages.json");
    let (d, tree) = fmtagno::prepare(json, &fmtagno::Env::default()).unwrap();
    let opts = fmtagno::render::Options {
        paged: true,
        ..Default::default()
    };
    let out = fmtagno::render::render(&d, &tree, fmtagno::render::Format::Html, &opts).unwrap();
    let s = String::from_utf8(out.bytes).unwrap();
    assert!(s.contains("Paged.js v0.4.3"));
    assert!(s.contains(".pagedjs_page"));
    // 默认不带
    let plain = fmtagno::render_str(json, fmtagno::render::Format::Html, None).unwrap();
    assert!(!String::from_utf8(plain.bytes).unwrap().contains("Paged.js"));
}

/// 固定列 + auto 列：不写 width:100%，用自动布局；部位 padding 盖在 cellPadding 后面。
#[test]
fn auto_columns_shrink_to_content() {
    let json = r##"{"root":"d","elements":{"d":{"type":"Document","children":["t"]},
      "t":{"type":"Table","props":{"rows":[["","基本信息"]],"widths":["5.65pt","auto"],"look":{"firstRow":false,"lastColumn":true}},
           "parts":{"lastColumn":{"paddingLeft":12}}}}}"##;
    let out = fmtagno::render_str(json, Format::Html, None).unwrap();
    let html = String::from_utf8(out.bytes).unwrap();
    assert!(!html.contains("width:100%"), "{html}");
    assert!(html.contains("table-layout:auto"), "{html}");
    assert!(
        html.contains(r#"<col style="width:5.65pt"><col>"#),
        "{html}"
    );
    assert!(html.contains("padding:4pt;padding-left:12pt;"), "{html}");
}

#[test]
fn empty_text_keeps_a_line() {
    let s = html(
        r##"{
      "root": "d",
      "elements": {
        "d": { "type": "Document", "children": ["t"] },
        "t": { "type": "Text", "props": { "text": "" } }
      }
    }"##,
    );
    assert!(s.contains("<p id=\"t\"><br></p>"));
}
