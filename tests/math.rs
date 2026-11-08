use std::io::Read;

use fmtagno::render::Format;

const EXAMPLE: &str = include_str!("../examples/math.json");

fn render(json: &str, f: Format) -> fmtagno::render::Output {
    fmtagno::render_str(json, f, None).unwrap()
}

fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut s)
        .unwrap();
    s
}

/// 一段正文里放一个行内公式，再放一个块级公式。
fn doc(inline: &str, block: &str) -> String {
    serde_json::json!({
        "root": "d",
        "elements": {
            "d": { "type": "Document", "children": ["t", "m"] },
            "t": { "type": "Text", "props": { "text": ["前", { "type": "InlineMath", "text": inline }, "后"] } },
            "m": { "type": "Math", "props": { "latex": block } }
        }
    })
    .to_string()
}

#[test]
fn example_renders_in_every_format() {
    for f in [Format::Html, Format::Docx, Format::Pdf, Format::Markdown] {
        let out = render(EXAMPLE, f);
        assert!(!out.bytes.is_empty());
        assert!(
            !out.warnings.iter().any(|w| w.contains("公式")),
            "{f:?}: {:?}",
            out.warnings
        );
    }
}

#[test]
fn pdf_is_pdf() {
    let out = render(EXAMPLE, Format::Pdf);
    assert!(out.bytes.starts_with(b"%PDF"));
}

#[test]
fn docx_has_omml() {
    let out = render(EXAMPLE, Format::Docx);
    let x = document_xml(&out.bytes);
    assert!(x.contains("xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\""));
    assert!(x.contains("<m:oMath>"));
    assert!(x.contains("<m:oMathPara>"));
    assert!(x.contains("<m:f>"));
    assert!(x.contains("<m:nary>"));
    assert!(x.contains("<m:rad>"));
    assert!(x.contains("<m:m>"));
    assert!(x.contains("<m:d>"));
    assert!(!x.contains('\u{E000}'), "公式记号没换掉");
    // 编号放在右对齐制表位后面
    assert!(x.contains("(1)"));
}

#[test]
fn docx_inline_math_sits_in_paragraph() {
    let out = render(&doc(r"\frac{a}{b}", r"\sum_{i=1}^{n} i"), Format::Docx);
    let x = document_xml(&out.bytes);
    // 行内公式夹在两个普通 run 中间
    let i = x.find("<m:oMath>").unwrap();
    let before = &x[..i];
    assert!(before.rfind("前").is_some());
    assert!(!before[before.rfind("<w:p").unwrap()..].contains("</w:p>"));
    assert!(x[i..].find("后").unwrap() > 0);
    assert!(x.contains("<m:f>"));
    let s = &x[x.find("<m:oMathPara>").unwrap()..];
    assert!(s.contains("<m:nary>"));
    assert!(s.contains("m:val=\"∑\""));
    assert!(s.contains("<m:limLoc m:val=\"undOvr\"/>"));
    // 字体换成 Cambria Math
    assert!(x.contains("Cambria Math"));
}

#[test]
fn html_has_mathml() {
    let out = render(&doc("x^2", r"\int_0^1 x\,dx"), Format::Html);
    let s = String::from_utf8(out.bytes).unwrap();
    assert!(s.contains("<math><msup>"), "{s}");
    assert!(s.contains("<math display=\"block\">"), "{s}");
    assert!(!s.contains("<script"), "公式不该靠脚本");
}

#[test]
fn markdown_uses_dollars() {
    let out = render(&doc("x^2", r"\frac{a}{b}"), Format::Markdown);
    let s = String::from_utf8(out.bytes).unwrap();
    assert!(s.contains("前$x^2$后"), "{s}");
    assert!(s.contains("$$\n\\frac{a}{b}\n$$"), "{s}");
    let out = render(EXAMPLE, Format::Markdown);
    let s = String::from_utf8(out.bytes).unwrap();
    assert!(s.contains("\\tag{1}\n$$"), "{s}");
}

#[test]
fn bad_latex_falls_back() {
    let json = doc(r"\nosuchcommand{x}", r"\frac{a}{");
    for f in [Format::Html, Format::Docx, Format::Pdf, Format::Markdown] {
        let out = render(&json, f);
        assert!(!out.bytes.is_empty());
        if f != Format::Markdown {
            assert!(
                out.warnings.iter().any(|w| w.contains("nosuchcommand")),
                "{f:?}: {:?}",
                out.warnings
            );
        }
    }
    let out = render(&json, Format::Docx);
    let x = document_xml(&out.bytes);
    assert!(x.contains("\\nosuchcommand{x}"));
    let (_, tree) = fmtagno::prepare(&json, &Default::default()).unwrap();
    assert_eq!(fmtagno::check_math(&tree).len(), 2);
}

#[test]
fn pdf_typst_source() {
    let d = fmtagno::doc::parse(&doc("x^2", r"\frac{1}{2}")).unwrap();
    let tree = d.tree().unwrap();
    let (src, _, _) = fmtagno::render::pdf::markup(&d, &tree, &Default::default());
    assert!(src.contains(r"$attach(\u{78}, t: \u{32})$"), "{src}");
    assert!(src.contains(r"$display(frac(\u{31}, \u{32}))$"), "{src}");
    assert!(src.contains("New Computer Modern Math"), "{src}");
}

#[test]
fn math_props_are_checked() {
    let json = r#"{"root":"d","elements":{"d":{"type":"Document","children":["m"]},"m":{"type":"Math","props":{"tex":"x"}}}}"#;
    assert!(fmtagno::render_str(json, Format::Html, None).is_err());
    let json = r#"{"root":"d","elements":{"d":{"type":"Document","children":["m"]},"m":{"type":"InlineMath","props":{}}}}"#;
    assert!(fmtagno::render_str(json, Format::Html, None).is_err());
}

#[test]
fn catalog_lists_math() {
    let s = fmtagno::components::spec("Math").unwrap();
    let schema = serde_json::to_string(&(s.schema)()).unwrap();
    assert!(schema.contains("latex"));
    assert!(schema.contains("number"));
    let t = fmtagno::components::spec("Text").unwrap();
    let schema = serde_json::to_string(&(t.schema)()).unwrap();
    assert!(schema.contains("InlineMath"), "{schema}");
}
