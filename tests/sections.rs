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

// ---------- 页码格式、分栏符 ----------

const FRONT: &str = include_str!("../examples/frontmatter.json");

#[test]
fn page_number_format_writes_numbers_like_word() {
    use fmtagno::doc::PageNumberFormat as F;
    let cases: &[(F, i64, &str)] = &[
        (F::Decimal, 12, "12"),
        (F::LowerRoman, 4, "iv"),
        (F::LowerRoman, 9, "ix"),
        (F::UpperRoman, 1994, "MCMXCIV"),
        (F::LowerLetter, 1, "a"),
        (F::LowerLetter, 26, "z"),
        // Word 过了 z 重复字母，不是 aa, ab
        (F::LowerLetter, 27, "aa"),
        (F::UpperLetter, 28, "BB"),
        (F::ChineseCounting, 1, "一"),
        (F::ChineseCounting, 10, "十"),
        (F::ChineseCounting, 11, "十一"),
        (F::ChineseCounting, 20, "二十"),
        (F::ChineseCounting, 101, "一百〇一"),
        (F::ChineseCounting, 110, "一百一十"),
        // 0 没有罗马数字写法，照阿拉伯数字
        (F::UpperRoman, 0, "0"),
    ];
    for (f, n, want) in cases {
        assert_eq!(f.format(*n), *want, "{f:?} {n}");
    }
    for f in F::ALL {
        assert_eq!(F::from_word(f.word()), Some(f));
    }
    assert_eq!(F::from_word("ordinal"), None);
}

#[test]
fn docx_writes_page_number_format_and_column_break() {
    let x = render(FRONT, Format::Docx);
    assert!(
        x.contains(r#"<w:pgNumType w:fmt="lowerRoman" w:start="1" />"#),
        "前言"
    );
    assert!(
        x.contains(r#"<w:pgNumType w:fmt="upperLetter" w:start="1" />"#),
        "附录 A"
    );
    assert!(
        x.contains(r#"<w:pgNumType w:fmt="chineseCounting" w:start="11" />"#),
        "附录 B"
    );
    // 正文写的是 decimal，不写 w:fmt
    assert_eq!(count(&x, "w:fmt="), 3);
    assert!(x.contains(r#"<w:pgNumType w:start="1" />"#), "正文");
    assert_eq!(count(&x, r#"<w:br w:type="column" />"#), 1);
    // 分栏符放进下一段开头，不单独占一段（单独占一段时 Word 下一栏顶上会多一个空行）
    let at = x.find(r#"<w:br w:type="column" />"#).unwrap();
    let p_start = x[..at].rfind("<w:p>").max(x[..at].rfind("<w:p ")).unwrap();
    let p_end = at + x[at..].find("</w:p>").unwrap();
    assert!(x[p_start..p_end].contains(">右栏<"), "分栏符应和下一段同段");
}

#[test]
fn docx_column_break_before_table_keeps_own_paragraph() {
    let json = r#"{"root":"d","elements":{"d":{"type":"Document","children":["a","cb","t"]},"a":{"type":"Text","props":{"text":"a"}},"cb":{"type":"ColumnBreak"},"t":{"type":"Table","props":{"rows":[["x"]]}}}}"#;
    let x = render(json, Format::Docx);
    assert_eq!(count(&x, r#"<w:br w:type="column" />"#), 1);
}

#[test]
fn page_number_format_carries_over_to_next_section() {
    // 第二节没写 pageNumberFormat，沿用第一节的 upperRoman
    let json = r#"{"meta":{"page":{"pageNumberFormat":"upperRoman"}},"root":"d","elements":{"d":{"type":"Document","children":["s1","s2"]},"s1":{"type":"Section","children":["a"]},"s2":{"type":"Section","props":{"pageNumberStart":1},"children":["b"]},"a":{"type":"Text","props":{"text":"a"}},"b":{"type":"Text","props":{"text":"b"}}}}"#;
    let x = render(json, Format::Docx);
    assert_eq!(count(&x, r#"w:fmt="upperRoman""#), 2);
    let h = render(json, Format::Html);
    assert_eq!(count(&h, r#"data-page-fmt="upperRoman""#), 2);
}

#[test]
fn pdf_numbers_pages_by_format_and_breaks_columns() {
    let (d, tree) = fmtagno::prepare(FRONT, &fmtagno::Env::default()).unwrap();
    let (src, _, _) = fmtagno::render::pdf::markup(&d, &tree, &Default::default());
    assert!(src.contains(r#"#metadata((start: 1, fmt: "i")) <fmtagno-sec>"#));
    assert!(src.contains(r#"#metadata((start: 1, fmt: "1")) <fmtagno-sec>"#));
    assert!(src.contains(r#"#metadata((start: 1, fmt: "A")) <fmtagno-sec>"#));
    assert!(src.contains(r#"#metadata((start: 11, fmt: "一")) <fmtagno-sec>"#));
    assert_eq!(count(&src, "#colbreak()"), 1);
    // 只有 PAGE 跟格式，总页数和本节页数是阿拉伯数字
    assert!(src.contains("fmtagno-info().page-text"));
    assert!(src.contains("fmtagno-info().total"));
    let out = fmtagno::render_str(FRONT, Format::Pdf, None).unwrap();
    assert!(out.bytes.starts_with(b"%PDF-"));
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn html_marks_page_format_and_column_break() {
    let x = render(FRONT, Format::Html);
    assert!(x.contains(r#"data-page-fmt="lowerRoman""#));
    assert!(x.contains(r#"data-page-fmt="upperLetter""#));
    assert!(x.contains(r#"data-page-fmt="chineseCounting""#));
    assert_eq!(count(&x, "data-page-fmt="), 3);
    assert!(x.contains(r#"<div class="fmtagno-colbreak" id="a-cb"></div>"#));
    assert!(x.contains(".fmtagno-colbreak + * { break-before: column; }"));
    let paged = fmtagno::render::render(
        &fmtagno::prepare(FRONT, &fmtagno::Env::default()).unwrap().0,
        &fmtagno::prepare(FRONT, &fmtagno::Env::default()).unwrap().1,
        Format::Html,
        &fmtagno::render::Options {
            paged: true,
            ..Default::default()
        },
    )
    .unwrap();
    let p = String::from_utf8(paged.bytes).unwrap();
    assert!(p.contains("fmtagnoNum(x.num, x.sec.fmt)"));
}

#[test]
fn column_break_not_allowed_in_header() {
    let cb = r#"{"meta":{"page":{"header":"h"}},"root":"d","elements":{"d":{"type":"Document","children":["t"]},"t":{"type":"Text","props":{"text":"a"}},"h":{"type":"Document","children":["c"]},"c":{"type":"ColumnBreak"}}}"#;
    assert!(err(cb).contains("页眉页脚里不能放 ColumnBreak"));
}
