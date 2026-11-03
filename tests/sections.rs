//! 分节、页眉页脚、页码。

use fmtagno::render::Format;

const EXAMPLE: &str = include_str!("../examples/sections.json");

fn render(json: &str, f: Format) -> String {
    let out = fmtagno::render_str(json, f, None).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    String::from_utf8_lossy(&out.bytes).into_owned()
}

fn count(s: &str, needle: &str) -> usize {
    s.matches(needle).count()
}

fn err(json: &str) -> String {
    match fmtagno::render_str(json, Format::Html, None) {
        Ok(_) => panic!("应该报错"),
        Err(e) => format!("{e:#}"),
    }
}

#[test]
fn docx_writes_every_section() {
    let x = render(EXAMPLE, Format::Docx);
    // 三节：两个节尾段落里的 sectPr 加正文最后的 sectPr
    assert_eq!(count(&x, "<w:sectPr>"), 3);
    // 占位节都收拾掉了
    assert!(!x.contains(r#"<w:pgSz w:w="1" w:h="1""#));
    // 每节三种页眉、三种页脚都写全，第一节首页留空也算
    assert_eq!(count(&x, "<w:headerReference"), 9);
    assert_eq!(count(&x, "<w:footerReference"), 9);
    // 页眉引用在 sectPr 最前面
    assert_eq!(count(&x, "<w:sectPr><w:headerReference"), 3);
    assert!(x.contains(r#"<w:pgNumType w:start="1" />"#));
    assert_eq!(count(&x, "<w:titlePg />"), 3);
    assert!(x.contains(r#"w:orient="landscape""#));
    assert!(x.contains(r#"w:num="2" />"#));
    assert!(x.contains("<w:evenAndOddHeaders"));
    // 页码域
    assert!(x.contains("PAGE"));
    assert!(x.contains("NUMPAGES"));
    assert!(x.contains("SECTIONPAGES"));
}

#[test]
fn docx_without_headers_has_no_references() {
    let json = r#"{"root":"d","elements":{"d":{"type":"Document","children":["t"]},"t":{"type":"Text","props":{"text":"a"}}}}"#;
    let x = render(json, Format::Docx);
    assert!(!x.contains("headerReference"));
    assert_eq!(count(&x, "<w:sectPr>"), 1);
}

#[test]
fn pdf_renders_sections() {
    let out = fmtagno::render_str(EXAMPLE, Format::Pdf, None).unwrap();
    assert!(out.bytes.starts_with(b"%PDF-"));
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn html_has_named_pages_and_templates() {
    let x = render(EXAMPLE, Format::Html);
    assert_eq!(count(&x, "<section class=\"fmtagno-section\""), 3);
    assert!(x.contains("@page fmtagno-s2 { size: 841.8898pt 595.2756pt;"));
    assert!(
        x.contains(".fmtagno-section[data-section=\"2\"] { page: fmtagno-s2; column-count: 2;")
    );
    assert!(x.contains(r#"data-kind="header-first""#));
    assert!(x.contains(r#"data-kind="header-even""#));
    assert!(x.contains(r#"data-page-start="1""#));
    assert!(x.contains(r#"<span class="fmtagno-field" data-field="sectionPages">"#));
    // 不加 --paged 时没有脚本，页眉页脚只是 <template>
    assert!(!x.contains("FmtagnoHeaders"));
}

#[test]
fn no_section_means_one_implicit_section() {
    let json = r#"{"meta":{"page":{"footer":"f"}},"root":"d","elements":{"d":{"type":"Document","children":["t"]},"t":{"type":"Text","props":{"text":"a"}},"f":{"type":"Text","props":{"text":["第 ",{"field":"page"}," 页"]}}}}"#;
    let x = render(json, Format::Html);
    assert_eq!(count(&x, "<section class=\"fmtagno-section\""), 1);
    assert!(x.contains(r#"data-kind="footer-default""#));
    let d = render(json, Format::Docx);
    assert_eq!(count(&d, "<w:footerReference"), 1);
    assert_eq!(count(&d, "<w:headerReference"), 1);
}

#[test]
fn rejects_bad_sections() {
    let doc = |children: &str, extra: &str| {
        format!(
            r#"{{"root":"d","elements":{{"d":{{"type":"Document","children":{children}}},"s":{{"type":"Section","children":["t"]}},"t":{{"type":"Text","props":{{"text":"a"}}}},"u":{{"type":"Text","props":{{"text":"b"}}}}{extra}}}}}"#
        )
    };
    // Section 后面的内容没放进 Section
    assert!(err(&doc(r#"["s","u"]"#, "")).contains("要放进 Section"));
    // Section 嵌在别的元素里
    assert!(
        err(&doc(
            r#"["x"]"#,
            r#","x":{"type":"Document","children":["s"]}"#
        ))
        .contains("只能直接放在 root 下面")
    );
    // 页眉用的元素同时出现在正文里
    let both = r#"{"meta":{"page":{"header":"t"}},"root":"d","elements":{"d":{"type":"Document","children":["t"]},"t":{"type":"Text","props":{"text":"a"}}}}"#;
    assert!(err(both).contains("不止一次"));
    // 页眉里分页
    let pb = r#"{"meta":{"page":{"header":"h"}},"root":"d","elements":{"d":{"type":"Document","children":["t"]},"t":{"type":"Text","props":{"text":"a"}},"h":{"type":"Document","children":["p"]},"p":{"type":"PageBreak"}}}"#;
    assert!(err(pb).contains("页眉页脚里不能放 PageBreak"));
    // 页眉指向不存在的元素
    let missing = r#"{"meta":{"page":{"footer":{"first":"nope"}}},"root":"d","elements":{"d":{"type":"Document","children":["t"]},"t":{"type":"Text","props":{"text":"a"}}}}"#;
    assert!(err(missing).contains("`nope` 不存在"));
}
