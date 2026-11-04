//! 样式模型：样式集、继承顺序、部位、列表、代码。

use fmtagno::{Env, render::Format};

fn html(json: &str) -> String {
    let out = fmtagno::render_with(json, Format::Html, None, &Env::default()).unwrap();
    String::from_utf8(out.bytes).unwrap()
}

fn err(json: &str) -> String {
    format!("{:#}", fmtagno::prepare(json, &Env::default()).unwrap_err())
}

fn all_formats(json: &str) {
    for f in [Format::Html, Format::Docx, Format::Pdf] {
        let out = fmtagno::render_str(json, f, None).unwrap_or_else(|e| panic!("{f:?}: {e:#}"));
        assert!(!out.bytes.is_empty());
    }
}

const SETS: &str = r##"{
  "version": "0.1",
  "styleSet": "modern",
  "styleSets": {
    "formal": {
      "global": { "fontFamily": { "latin": "Times New Roman", "eastAsia": "宋体" }, "fontSize": 12 },
      "components": { "Heading": { "color": "#111111" }, "Heading.1": { "fontSize": 22 } },
      "styles": { "muted": { "color": "#888888" } }
    },
    "modern": {
      "extends": "formal",
      "global": { "fontFamily": { "latin": "Inter" } },
      "components": { "Heading.1": { "color": "#1A73E8" } }
    }
  },
  "styles": { "Heading.1": { "align": "center" }, "muted": { "color": "#000000" } },
  "root": "d",
  "elements": {
    "d": { "type": "Document", "children": ["h", "t"] },
    "h": { "type": "Heading", "props": { "level": 1, "text": "标题" } },
    "t": { "type": "Text", "props": { "text": "正文" }, "style": "muted" }
  }
}"##;

#[test]
fn style_sets_cascade() {
    let (_, tree) = fmtagno::prepare(SETS, &Env::default()).unwrap();
    let h = &tree.children[0].children[0].style;
    // extends：formal 的 fontSize 22，modern 的颜色，顶层 styles 的居中
    assert_eq!(h.size_pt(), 22.0);
    assert_eq!(h.color.as_ref().unwrap().rgb(), "1A73E8");
    assert_eq!(h.align.map(|a| a.as_str()), Some("center"));
    let f = h.font_family.as_ref().unwrap();
    assert_eq!(f.latin.as_deref(), Some("Inter"));
    assert_eq!(f.east_asia.as_deref(), Some("宋体"));
    // 命名样式先找样式集里的
    let t = &tree.children[0].children[1].style;
    assert_eq!(t.color.as_ref().unwrap().rgb(), "888888");
    assert_eq!(t.size_pt(), 12.0);
}

#[test]
fn command_line_style_set_wins() {
    let env = Env {
        style_set: Some("formal".into()),
        ..Env::default()
    };
    let (_, tree) = fmtagno::prepare(SETS, &env).unwrap();
    assert_eq!(
        tree.children[0].children[0]
            .style
            .color
            .as_ref()
            .unwrap()
            .rgb(),
        "111111"
    );
}

#[test]
fn rejects_bad_styles() {
    let base = |extra: &str, el: &str| {
        format!(
            r#"{{{extra}"root":"d","elements":{{"d":{{"type":"Document","children":["x"]}},"x":{el}}}}}"#
        )
    };
    assert!(
        err(&base(
            r##""styleSet":"a","styleSets":{"a":{"global":{"border":"1pt solid #000"}}},"##,
            r#"{"type":"Text","props":{"text":"a"}}"#
        ))
        .contains("border")
    );
    assert!(
        err(&base(
            "",
            r#"{"type":"Table","props":{"rows":[]},"parts":{"nope":{}}}"#
        ))
        .contains("没有部位")
    );
    assert!(
        err(&base(
            "",
            r#"{"type":"Text","props":{"text":[{"text":"a","style":{"padding":3}}]}}"#
        ))
        .contains("padding")
    );
    assert!(
        err(&base(
            r#""styles":{"Heading.7":{}},"#,
            r#"{"type":"Text","props":{"text":"a"}}"#
        ))
        .contains("1 到 6")
    );
    assert!(
        err(&base(
            r#""styleSet":"zz","#,
            r#"{"type":"Text","props":{"text":"a"}}"#
        ))
        .contains("zz")
    );
    assert!(
        err(&base(
            "",
            r#"{"type":"Text","props":{"text":"a"},"style":{"color":"red"}}"#
        ))
        .contains("颜色")
    );
}

const TABLE: &str = r##"{
  "root": "d",
  "elements": {
    "d": { "type": "Document", "children": ["t"] },
    "t": {
      "type": "Table",
      "props": { "columns": ["品名", "单价"], "rows": [["苹果", "3.5"], ["梨", "4.0"], ["桃", "5.0"], ["合计", "12.5"]], "look": { "lastRow": true, "lastColumn": true } },
      "style": { "border": "0.5pt solid #CCCCCC" },
      "parts": {
        "header": { "background": "#1A73E8", "color": "#FFFFFF" },
        "evenRow": { "background": "#F5F7FA" },
        "lastRow": { "bold": true, "borderTop": "1pt solid #333333" },
        "lastColumn": { "align": "right" }
      }
    }
  }
}"##;

#[test]
fn table_parts() {
    let s = html(TABLE);
    assert!(s.contains("background:#1A73E8"));
    // 第 2 行数据（梨）是偶数行，合计行是 lastRow 不算带
    assert_eq!(s.matches("background:#F5F7FA").count(), 2);
    assert!(s.contains("border-top:1pt solid #333333"));
    assert!(s.contains("text-align:right"));
    all_formats(TABLE);
}

const LIST: &str = r##"{
  "root": "d",
  "elements": {
    "d": { "type": "Document", "children": ["l"] },
    "l": {
      "type": "List",
      "props": { "ordered": true, "items": ["一", { "text": "二", "items": ["二点一", "二点二"] }, "三"] },
      "style": { "listStyleType": "cjk-decimal", "listMarkerFormat": "%1、" },
      "parts": { "level2": { "listStyleType": "decimal", "listMarkerFormat": "%1.%2", "listMarkerColor": "#CF222E" } }
    }
  }
}"##;

#[test]
fn nested_list_markers() {
    let s = html(LIST);
    assert!(s.contains(">一、</span>"));
    assert!(s.contains(">三、</span>"));
    assert!(
        s.contains(">2.2</span>"),
        "本级是 decimal，上级编号也写成阿拉伯数字"
    );
    assert!(s.contains("color:#CF222E"));
    all_formats(LIST);
}

const CODE: &str = r##"{
  "styles": { "InlineCode": { "color": "#CF222E" } },
  "root": "d",
  "elements": {
    "d": { "type": "Document", "children": ["t", "c"] },
    "t": { "type": "Text", "props": { "text": ["运行 ", { "type": "InlineCode", "text": "fmtagno render" }, " 生成文件"] } },
    "c": { "type": "Code", "props": { "language": "rust", "code": "fn main() {\n    println!(\"你好\");\n}", "lineNumbers": true } }
  }
}"##;

#[test]
fn code_and_inline_code() {
    let s = html(CODE);
    assert!(s.contains("<code style=\""));
    assert!(s.contains("color:#CF222E"));
    assert!(s.contains("class=\"language-rust\""));
    assert!(s.contains("<span class=\"fmtagno-ln\">2</span>    println!"));
    assert!(err(r#"{"root":"x","elements":{"x":{"type":"InlineCode"}}}"#).contains("runs"));
    all_formats(CODE);
}

#[test]
fn custom_component_parts_and_overrides() {
    let json = r##"{
      "components": [{
        "name": "PriceTable",
        "props": { "rows": { "type": "array", "required": true } },
        "template": { "root": "t", "elements": {
          "t": { "type": "Table", "props": { "columns": ["品名", "金额"], "rows": "{{rows}}" },
                 "style": { "fontSize": 9 }, "parts": { "header": { "background": "#1A73E8" } } }
        } }
      }],
      "styles": { "PriceTable:t.header": { "background": "#333333" }, "Table": { "fontSize": 12 } },
      "root": "d",
      "elements": {
        "d": { "type": "Document", "children": ["p"] },
        "p": { "type": "PriceTable", "props": { "rows": [["苹果", "3"]] } }
      }
    }"##;
    let (_, tree) = fmtagno::prepare(json, &Env::default()).unwrap();
    let t = &tree.children[0].children[0];
    // 模板样式盖住通用的 Table 样式（12 → 9），PriceTable:t.header 再盖住模板的 parts
    assert_eq!(t.style.size_pt(), 9.0);
    assert_eq!(t.part("header").background.unwrap().solid().rgb(), "333333");
}

#[test]
fn page_and_paragraph_fields() {
    let json = r##"{
      "meta": { "page": { "size": { "width": "100mm", "height": "150mm" }, "marginLeft": "30mm", "columns": 2 } },
      "root": "d",
      "elements": {
        "d": { "type": "Document", "children": ["t", "i"] },
        "t": { "type": "Text", "props": { "text": "首行缩进两个字，段前半行，下划线虚线。" },
               "style": { "textIndent": "2em", "spaceBefore": "0.5line", "underline": "dashed",
                          "lineHeight": { "atLeast": 18 }, "background": { "gradient": "linear", "angle": 90, "stops": ["#FFFFFF", "#E8F0FE"] } } },
        "i": { "type": "Image", "props": { "src": "examples/logo.svg", "alt": "logo", "width": 40 },
               "style": { "position": "float", "float": "right", "wrapDistance": 4 } }
      }
    }"##;
    let s = html(json);
    assert!(s.contains("size: 283.46"), "{s}");
    assert!(s.contains("text-indent:2em"));
    assert!(s.contains("linear-gradient(90deg, #FFFFFF, #E8F0FE)"));
    assert!(s.contains("float:right"));
    all_formats(json);
}

const VARS: &str = r##"{
  "version": "0.1",
  "styleSet": "child",
  "styleSets": {
    "base": {
      "global": { "$color1": "#8DC63F", "$gap": 6, "$line": "1pt solid ${color1}", "fontSize": 11 },
      "components": { "Heading": { "color": "${color1}", "spaceAfter": "${gap}" } }
    },
    "child": {
      "extends": "base",
      "global": { "$color2": "#123456" },
      "styles": { "box": { "borderBottom": "${line}", "background": "${color2}" } }
    }
  },
  "root": "d",
  "elements": {
    "d": { "type": "Document", "children": ["h", "t"] },
    "h": { "type": "Heading", "props": { "level": 1, "text": "标题" } },
    "t": { "type": "Text", "props": { "runs": [{ "text": "绿", "color": "${color2}" }, "价 $${x}"] }, "style": ["box", { "color": "${color1}" }] }
  }
}"##;

/// global 里 `$名字` 定义变量，样式、props 里 `${名字}` 引用；父样式集的变量子样式集也能用。
#[test]
fn style_set_variables() {
    let h = html(VARS);
    assert!(h.to_lowercase().contains("#8dc63f"), "{h}");
    assert!(h.to_lowercase().contains("#123456"));
    assert!(h.contains("价 ${x}"));
    all_formats(VARS);

    let e = err(&VARS.replace(r#""${color2}" }]"#, r#""${nope}" }]"#));
    assert!(e.contains("${nope}") && e.contains("没定义"), "{e}");
    let e = err(&VARS
        .replace(r#""$gap": 6"#, r#""$gap": "${line}""#)
        .replace(r#""$line": "1pt solid ${color1}""#, r#""$line": "${gap}""#));
    assert!(e.contains("成环"), "{e}");
    let e = err(&VARS.replace(r#""$color2""#, r#""$bad name""#));
    assert!(e.contains("变量名"), "{e}");
}
