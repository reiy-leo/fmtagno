use std::path::Path;

use fmtagno::render::Format;

#[test]
fn showcase_renders_to_pdf() {
    let out = fmtagno::render_str(
        include_str!("../examples/showcase.json"),
        Format::Pdf,
        Some(Path::new("examples")),
    )
    .unwrap();
    assert!(out.bytes.starts_with(b"%PDF-"));
    // SVG 在 pdf 里能直接用，不该有图片警告
    assert!(
        !out.warnings.iter().any(|w| w.contains("图片")),
        "{:?}",
        out.warnings
    );
}

#[test]
fn quotes_and_markup_chars_survive() {
    let json = r##"{"root":"d","elements":{"d":{"type":"Document","children":["t"]},"t":{"type":"Text","props":{"text":"a \"b\" \\ #c *d* _e_ $f$ <g> @h [i]"}}}}"##;
    let out = fmtagno::render_str(json, Format::Pdf, None).unwrap();
    assert!(out.bytes.starts_with(b"%PDF-"));
}

#[test]
fn styled_blocks_span_full_width() {
    let d = fmtagno::doc::parse(r##"{"root":"d","elements":{"d":{"type":"Document","children":["s"]},"s":{"type":"Text","props":{"text":"x"},"style":{"align":"right","color":"#666"}}}}"##).unwrap();
    let tree = d.tree().unwrap();
    let (src, _, _) = fmtagno::render::pdf::markup(&d, &tree, &Default::default());
    assert!(src.contains("width: 100%"), "{src}");
    assert!(src.contains("#set align(right)"), "{src}");
}

#[test]
fn table_column_widths_mix_ratio_fixed_auto() {
    let d = fmtagno::doc::parse(r##"{"root":"d","elements":{"d":{"type":"Document","children":["t"]},"t":{"type":"Table","props":{"rows":[["a","b","c"]],"widths":["20mm","auto",2]}}}}"##).unwrap();
    let tree = d.tree().unwrap();
    let (src, _, _) = fmtagno::render::pdf::markup(&d, &tree, &Default::default());
    assert!(src.contains("columns: (56.69"), "{src}");
    // auto 列按内容量出宽度，写成固定 pt
    let cols = src
        .split("columns: (")
        .nth(1)
        .unwrap()
        .split(')')
        .next()
        .unwrap();
    let parts: Vec<_> = cols.split(", ").collect();
    assert!(parts[1].ends_with("pt") && parts[2] == "2fr,", "{cols}");
}

#[test]
fn all_border_styles_render() {
    let json = include_str!("../examples/borders.json");
    let out = fmtagno::render_str(json, fmtagno::render::Format::Pdf, None).unwrap();
    assert!(out.bytes.starts_with(b"%PDF"));
}

/// 双线、粗细线不用图案铺：块套几层 stroke，表格格子用硬边渐变；虚线、波浪线才用 tiling。
#[test]
fn multi_line_borders_draw_solid_lines() {
    let one = |border: &str| {
        let json = format!(
            r##"{{"root":"d","elements":{{"d":{{"type":"Document","children":["a"]}},
              "a":{{"type":"Text","props":{{"text":"x"}},"style":{{"border":"{border}","padding":6}}}}}}}}"##
        );
        let d = fmtagno::doc::parse(&json).unwrap();
        let tree = d.tree().unwrap();
        fmtagno::render::pdf::markup(&d, &tree, &Default::default()).0
    };
    // thinThickSmallGap 是 1:1:3：细线 0.6pt，空 0.6pt，粗线 1.8pt；两层 stroke
    let src = one("3pt thinThickSmallGap #1A73E8");
    assert_eq!(src.matches("stroke: (top: (paint").count(), 2, "{src}");
    assert!(src.contains("thickness: 1.800pt"), "{src}");
    assert!(!src.contains("tiling("), "{src}");
    // 波浪线还是图案
    let src = one("3pt wave #1A73E8");
    assert!(src.contains("fill: tiling("), "{src}");

    let json = include_str!("../examples/borders.json");
    let d = fmtagno::doc::parse(json).unwrap();
    let tree = d.tree().unwrap();
    let (src, _, _) = fmtagno::render::pdf::markup(&d, &tree, &Default::default());
    assert!(src.contains("paint: gradient.linear("), "表格双线该用渐变");
}
