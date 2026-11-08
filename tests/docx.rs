use std::path::Path;

use fmtagno::render::Format;

fn has(bytes: &[u8], needle: &str) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn showcase_renders_to_docx() {
    let out = fmtagno::render_str(
        include_str!("../examples/showcase.json"),
        Format::Docx,
        Some(Path::new("examples")),
    )
    .unwrap();
    assert!(out.bytes.starts_with(b"PK"));
    assert!(has(&out.bytes, "word/document.xml"));
    // logo.svg 放不进 docx，应该给警告而不是报错
    assert!(out.warnings.iter().any(|w| w.contains("logo")));
}

#[test]
fn png_image_is_embedded() {
    let dir = std::env::temp_dir().join("fmtagno-docx-test");
    std::fs::create_dir_all(&dir).unwrap();
    image::RgbImage::from_pixel(4, 2, image::Rgb([200, 30, 30]))
        .save(dir.join("dot.png"))
        .unwrap();
    let json = r#"{"root":"d","elements":{"d":{"type":"Document","children":["i"]},"i":{"type":"Image","props":{"src":"dot.png","width":40}}}}"#;
    let out = fmtagno::render_str(json, Format::Docx, Some(&dir)).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert!(has(&out.bytes, "word/media/"));
}

/// Word 实机里看到的两个问题：合计行的上边框画了两条、二级编号出成「二.1」。
#[test]
fn styles_example_borders_and_legal_numbering() {
    let out = fmtagno::render_str(
        include_str!("../examples/styles.json"),
        Format::Docx,
        Some(Path::new("examples")),
    )
    .unwrap();
    let b = &out.bytes;
    // 格子之间的线只写在上面一格的下边，格子里的段落不带边框
    assert!(has(b, r#"<w:bottom w:val="double""#));
    assert!(!has(b, r#"<w:top w:val="double""#));
    // 二级是 decimal，上级编号用阿拉伯数字
    assert!(has(b, "<w:isLgl"));
}

/// 表头默认跨页重复：docx-rs 不支持 w:tblHeader，渲染后补进 XML。
#[test]
fn table_header_repeats_and_page_break() {
    let json = include_str!("../examples/pages.json");
    let out = fmtagno::render_str(json, Format::Docx, None).unwrap();
    assert!(has(&out.bytes, "<w:trPr><w:tblHeader /></w:trPr>"));
    assert!(has(&out.bytes, r#"<w:br w:type="page""#));
    // repeatHeader: false 时不写
    let off = json.replacen(
        r#""type": "Table","#,
        r#""type": "Table", "style": { "repeatHeader": false },"#,
        1,
    );
    let out = fmtagno::render_str(&off, Format::Docx, None).unwrap();
    assert!(!has(&out.bytes, "<w:tblHeader"));
}

/// 倍数行距按 CSS 算（字号 × 倍数），docx 写成「至少」，不用 Word 的 auto。
#[test]
fn line_height_multiple_is_at_least_font_size_times() {
    let json = r#"{
      "version": "0.1",
      "root": "doc",
      "elements": {
        "doc": { "type": "Document", "children": ["a", "b"] },
        "a": { "type": "Text", "props": { "text": "倍数" }, "style": { "fontSize": 10, "lineHeight": 1.5 } },
        "b": { "type": "Text", "props": { "text": "固定" }, "style": { "fontSize": 10, "lineHeight": "20pt" } }
      }
    }"#;
    let out = fmtagno::render_str(json, Format::Docx, None).unwrap();
    let b = &out.bytes;
    assert!(has(b, r#"w:line="300" w:lineRule="atLeast""#));
    assert!(has(b, r#"w:line="400" w:lineRule="exact""#));
    assert!(!has(b, r#"w:lineRule="auto""#));
}

/// 固定列 + auto 列：表格不撑满，auto 列写 tcW auto、表格 autofit；部位 padding 落成 tcMar。
#[test]
fn auto_columns_and_part_padding() {
    let json = r##"{
      "version": "0.1",
      "root": "doc",
      "elements": {
        "doc": { "type": "Document", "children": ["t"] },
        "t": {
          "type": "Table",
          "props": { "rows": [["", "基本信息"]], "widths": ["5.65pt", "auto"], "look": { "firstRow": false, "firstColumn": true } },
          "style": { "fontSize": 10 },
          "parts": { "lastColumn": { "paddingLeft": 12, "paddingRight": 18 } }
        }
      }
    }"##;
    let json = json.replace(
        r#""firstColumn": true }"#,
        r#""firstColumn": true, "lastColumn": true }"#,
    );
    let out = fmtagno::render_str(&json, Format::Docx, None).unwrap();
    let b = &out.bytes;
    // 1460 = 4 个全角字 × 10pt + 左右 padding 12 + 18 + 3pt 余量
    assert!(has(
        b,
        r#"<w:gridCol w:w="113" w:type="dxa" /><w:gridCol w:w="1460""#
    ));
    assert!(has(b, r#"<w:tcW w:w="0" w:type="auto""#));
    assert!(has(b, r#"<w:tblLayout w:type="autofit""#));
    assert!(has(b, r#"<w:left w:w="240" w:type="dxa""#));
    assert!(has(b, r#"<w:right w:w="360" w:type="dxa""#));
    assert!(
        has(b, r#"<w:tblW w:w="0" w:type="auto""#),
        "auto 表格不该撑满版心"
    );
}

/// em 缩进同时写 Word 的字符单位（leftChars、rightChars、firstLineChars、hangingChars）。
#[test]
fn em_indent_writes_chars() {
    let json = r##"{
      "root": "d",
      "elements": {
        "d": { "type": "Document", "children": ["a", "b", "c"] },
        "a": { "type": "Text", "props": { "text": "a" },
               "style": { "fontSize": 12, "marginLeft": "2em", "marginRight": "1em", "textIndent": "2em" } },
        "b": { "type": "Text", "props": { "text": "b" },
               "style": { "fontSize": 12, "marginLeft": "1em", "hangingIndent": "2em" } },
        "c": { "type": "Text", "props": { "text": "c" },
               "style": { "fontSize": 12, "marginLeft": "20pt", "marginRight": 10 } }
      }
    }"##;
    let out = fmtagno::render_str(json, Format::Docx, None).unwrap();
    let b = &out.bytes;
    assert!(has(
        b,
        r#"<w:ind w:left="480" w:right="240" w:firstLine="480" w:leftChars="200" w:rightChars="100" w:firstLineChars="200" />"#
    ));
    assert!(has(
        b,
        r#"<w:ind w:left="720" w:right="0" w:hanging="480" w:leftChars="300" w:hangingChars="200" />"#
    ));
    assert!(has(b, r#"<w:ind w:left="400" w:right="200" />"#));
    assert!(!has(b, "w:right=\"100000"));
}

#[test]
fn word_border_styles_map_through() {
    let json = include_str!("../examples/borders.json");
    let out = fmtagno::render_str(json, fmtagno::render::Format::Docx, None).unwrap();
    for v in [
        "thinThickThinLargeGap",
        "doubleWave",
        "threeDEngrave",
        "dashDotStroked",
        "dotDotDash",
    ] {
        assert!(has(&out.bytes, &format!("w:val=\"{v}\"")), "缺 {v}");
    }
}
