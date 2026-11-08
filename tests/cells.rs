//! 合并单元格、单格样式、行高、runs 里的行内图片，三种格式各看一遍。

use std::path::PathBuf;

use fmtagno::render::Format;

fn has(bytes: &[u8], needle: &str) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn dir() -> PathBuf {
    let dir = std::env::temp_dir().join("fmtagno-cells-test");
    std::fs::create_dir_all(&dir).unwrap();
    // 几个测试并行跑，都会写这张图：先写到各自的临时文件再改名，免得别的测试读到写了一半的图
    let tmp =
        dir.join(format!("logo-{:?}.png", std::thread::current().id()).replace(['(', ')'], ""));
    image::RgbImage::from_pixel(4, 2, image::Rgb([30, 160, 60]))
        .save(&tmp)
        .unwrap();
    std::fs::rename(&tmp, dir.join("logo.png")).unwrap();
    dir
}

const DOC: &str = r##"{
  "version": "0.1",
  "root": "doc",
  "elements": {
    "doc": { "type": "Document", "children": ["t"] },
    "t": {
      "type": "Table",
      "props": {
        "look": { "firstRow": false },
        "widths": [1, 2, 4],
        "rowHeights": ["20pt", { "height": "36pt", "rule": "exact" }],
        "rows": [
          [ { "text": "一", "rowSpan": 2, "style": { "borderLeft": "6pt solid #8DC63F" } },
            { "text": "标题", "colSpan": 2 } ],
          [ { "runs": [ { "image": "logo.png", "width": 40, "height": 20 }, { "text": " 内容" } ] }, "右" ]
        ]
      }
    }
  }
}"##;

#[test]
fn merged_cells_row_heights_inline_image_html() {
    let out = fmtagno::render_str(DOC, Format::Html, Some(&dir())).unwrap();
    let h = String::from_utf8(out.bytes).unwrap();
    assert!(h.contains(r#"rowspan="2""#), "{h}");
    assert!(h.contains(r#"colspan="2""#));
    assert!(h.contains("fmtagno-inline-img"));
    assert!(h.contains("data:image/png;base64,"));
    // 合并后第二行只剩两个格
    let row2 = h.split("<tr").nth(2).unwrap();
    assert_eq!(row2.matches("<td").count(), 2, "{row2}");
}

#[test]
fn merged_cells_row_heights_inline_image_docx() {
    let out = fmtagno::render_str(DOC, Format::Docx, Some(&dir())).unwrap();
    let b = &out.bytes;
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert!(has(b, r#"<w:gridSpan w:val="2""#));
    assert!(has(b, r#"<w:vMerge w:val="restart""#));
    assert!(has(b, r#"<w:vMerge w:val="continue""#) || has(b, "<w:vMerge />"));
    assert!(has(b, r#"w:hRule="exact""#));
    assert!(has(b, r#"w:val="720""#)); // 36pt
    assert!(has(b, r#"w:hRule="atLeast""#));
    assert!(has(b, "word/media/"));
}

#[test]
fn merged_cells_row_heights_inline_image_pdf() {
    let out = fmtagno::render_str(DOC, Format::Pdf, Some(&dir())).unwrap();
    assert!(out.bytes.starts_with(b"%PDF-"));
    assert!(
        !out.warnings.iter().any(|w| w.contains("图片")),
        "{:?}",
        out.warnings
    );
}

#[test]
fn rejects_overlapping_spans() {
    let json = DOC.replace(r#""colSpan": 2"#, r#""colSpan": 3"#);
    let err = fmtagno::render_str(&json, Format::Html, Some(&dir()));
    // 超出列数：报错或给出警告，不能悄悄画坏
    match err {
        Err(_) => {}
        Ok(out) => assert!(!out.warnings.is_empty()),
    }
}

/// 两节、第一节页眉留空、第二节页眉里放行内图：图片关系要挂在真正放图的那个页眉上。
#[test]
fn header_inline_image_gets_its_relationship() {
    let json = r##"{
      "version": "0.1",
      "root": "doc",
      "elements": {
        "doc": { "type": "Document", "children": ["s1", "s2"] },
        "s1": { "type": "Section", "props": { "header": null }, "children": ["a"] },
        "s2": { "type": "Section", "props": { "header": "h" }, "children": ["b"] },
        "a": { "type": "Text", "props": { "text": "一" } },
        "b": { "type": "Text", "props": { "text": "二" } },
        "h": { "type": "Text", "props": { "text": [{ "image": "logo.png", "width": 40 }, " 页眉"] } }
      }
    }"##;
    let out = fmtagno::render_str(json, Format::Docx, Some(&dir())).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(out.bytes)).unwrap();
    let names: Vec<String> = zip.file_names().map(String::from).collect();
    let mut found = false;
    for n in names
        .iter()
        .filter(|n| n.starts_with("word/header") && n.ends_with(".xml"))
    {
        let mut x = String::new();
        std::io::Read::read_to_string(&mut zip.by_name(n).unwrap(), &mut x).unwrap();
        let Some(i) = x.find("r:embed=\"") else {
            continue;
        };
        let id = &x[i + 9..i + 9 + x[i + 9..].find('"').unwrap()];
        let rels = n.replace("word/", "word/_rels/") + ".rels";
        let mut r = String::new();
        std::io::Read::read_to_string(&mut zip.by_name(&rels).unwrap(), &mut r).unwrap();
        assert!(r.contains(&format!("Id=\"{id}\"")), "{rels}: {r}");
        found = true;
    }
    assert!(found, "页眉里没有图片");
}
