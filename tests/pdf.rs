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
    assert!(src.contains("pt, auto, 2fr,)"), "{src}");
}
