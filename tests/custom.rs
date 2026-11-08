use std::path::Path;

use fmtagno::{Env, render::Format};

fn examples_catalog() -> Env {
    fmtagno::load_env(Some(Path::new("examples")), &[], &[], None).unwrap()
}

#[test]
fn example_expands_and_renders_everywhere() {
    let cat = examples_catalog();
    let json = include_str!("../examples/custom.json");
    let (d, _) = fmtagno::prepare(json, &cat).unwrap();
    // 实例 id 留给根元素，其余加前缀
    assert_eq!(d.elements["tip"].kind, "Document");
    assert_eq!(
        d.elements["tip"].children,
        ["tip.head", "tip-body", "tip-list"]
    );
    assert_eq!(
        d.elements["ch1.title"].props["text"],
        "第 1 章：为什么要自定义组件"
    );
    assert!(d.elements["ch1.lead"].props["text"].is_array());
    for f in [Format::Html, Format::Docx, Format::Pdf, Format::Markdown] {
        let out = fmtagno::render_with(json, f, Some(Path::new("examples")), &cat).unwrap();
        assert!(!out.bytes.is_empty());
    }
    let html = fmtagno::render_with(json, Format::Html, None, &cat).unwrap();
    let html = String::from_utf8(html.bytes).unwrap();
    assert!(html.contains("fmtagno　2026-10-28"));
}

fn doc_with(components: &str, elements: &str) -> String {
    format!(
        r#"{{"components":[{components}],"root":"d","elements":{{"d":{{"type":"Document","children":["x"]}},{elements}}}}}"#
    )
}

fn err(json: &str) -> String {
    format!("{:#}", fmtagno::prepare(json, &Env::default()).unwrap_err())
}

#[test]
fn rejects_bad_usage() {
    let def = r#"{"name":"Box1","props":{"t":{"required":true}},"template":{"root":"a","elements":{"a":{"type":"Text","props":{"text":"{{t}}"}}}}}"#;
    assert!(err(&doc_with(def, r#""x":{"type":"Box1"}"#)).contains("缺少必填"));
    assert!(
        err(&doc_with(
            def,
            r#""x":{"type":"Box1","props":{"t":"a","u":1}}"#
        ))
        .contains("没有 prop `u`")
    );
    assert!(err(&doc_with(def, r#""x":{"type":"Box1","props":{"t":5}}"#)).contains("类型"));
    assert!(
        err(&doc_with(
            def,
            r#""x":{"type":"Box1","props":{"t":"a"},"children":["d"]}"#
        ))
        .contains("不能有 children")
    );
}

#[test]
fn rejects_bad_definitions() {
    let undefined = r#"{"name":"A1","template":{"root":"a","elements":{"a":{"type":"Text","props":{"text":"{{nope}}"}}}}}"#;
    assert!(err(&doc_with(undefined, r#""x":{"type":"A1"}"#)).contains("没定义的 prop"));
    let builtin = r#"{"name":"Text","template":{"root":"a","elements":{"a":{"type":"Text","props":{"text":"x"}}}}}"#;
    assert!(err(&doc_with(builtin, r#""x":{"type":"Text"}"#)).contains("内置组件重名"));
    let no_slot = r#"{"name":"C1","container":true,"template":{"root":"a","elements":{"a":{"type":"Document"}}}}"#;
    assert!(err(&doc_with(no_slot, r#""x":{"type":"C1"}"#)).contains("$children"));
}

#[test]
fn detects_recursion() {
    let a = r#"{"name":"Loop1","template":{"root":"a","elements":{"a":{"type":"Loop1"}}}}"#;
    assert!(err(&doc_with(a, r#""x":{"type":"Loop1"}"#)).contains("成环"));
}

#[test]
fn definition_file_parses() {
    let def = fmtagno::custom::parse_def(include_str!("../examples/catalog/notice.json")).unwrap();
    assert_eq!(def.name, "Notice");
    assert!(def.container);
    def.validate().unwrap();
}
