//! 内置组件和它们的 props。

use schemars::{JsonSchema, Schema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::style::{Style, Underline};

/// run 的种类：普通文字或行内代码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RunKind {
    InlineCode,
}

/// 域：渲染时才知道的值，Word 里是 PAGE、NUMPAGES、SECTIONPAGES。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Field {
    /// 当前页码
    Page,
    /// 全文总页数
    Pages,
    /// 本节页数
    SectionPages,
}

/// 一段文字里的一小段，可以单独设文字样式或加链接。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Run {
    /// 写了 field 时可以不写
    #[serde(default)]
    pub text: String,
    /// 页码这类域，写了就不用 text
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<Field>,
    /// 写 "InlineCode" 表示行内代码，外观由 InlineCode 组件样式决定
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<RunKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    /// 行内图片：本地路径或网址，写了就不显示 text
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// 行内图片的宽，pt；只写宽或高时按比例算另一边
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// 行内图片的高，pt
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    /// 常用文字样式的简写
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<Underline>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<crate::style::Color>,
    /// 样式名、样式名数组或样式对象，只能写文字字段
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<Value>")]
    pub style: Option<Value>,
    /// 算好的样式，渲染器用它，不从 JSON 读
    #[serde(skip)]
    #[schemars(skip)]
    pub computed: Style,
}

impl Run {
    pub fn plain(text: &str) -> Run {
        Run {
            text: text.to_string(),
            field: None,
            kind: None,
            href: None,
            image: None,
            width: None,
            height: None,
            bold: None,
            italic: None,
            underline: None,
            color: None,
            style: None,
            computed: Style::default(),
        }
    }

    pub fn is_code(&self) -> bool {
        self.kind == Some(RunKind::InlineCode)
    }
}

/// 文字类 props：纯字符串，或 runs 数组（数组里也能直接写字符串）。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Inline {
    Plain(String),
    Runs(Vec<RunOrText>),
}

// 一份文档里的 run 不多，不值得为大小差异装箱
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RunOrText {
    Text(String),
    Run(Run),
}

impl Default for Inline {
    fn default() -> Self {
        Inline::Plain(String::new())
    }
}

impl Inline {
    /// 统一成 runs，字符串变成普通 run。
    pub fn normalize(&mut self) {
        if let Inline::Plain(s) = self {
            *self = Inline::Runs(vec![RunOrText::Run(Run::plain(s))]);
        }
        if let Inline::Runs(rs) = self {
            for r in rs.iter_mut() {
                if let RunOrText::Text(s) = r {
                    *r = RunOrText::Run(Run::plain(s));
                }
            }
        }
    }

    /// 先 normalize 过才有意义。
    pub fn runs(&self) -> impl Iterator<Item = &Run> {
        let v: &[RunOrText] = match self {
            Inline::Runs(r) => r,
            Inline::Plain(_) => &[],
        };
        v.iter().filter_map(|r| match r {
            RunOrText::Run(r) => Some(r),
            RunOrText::Text(_) => None,
        })
    }

    pub fn runs_mut(&mut self) -> impl Iterator<Item = &mut Run> {
        let v: &mut [RunOrText] = match self {
            Inline::Runs(r) => r,
            Inline::Plain(_) => &mut [],
        };
        v.iter_mut().filter_map(|r| match r {
            RunOrText::Run(r) => Some(r),
            RunOrText::Text(_) => None,
        })
    }

    /// 纯文字，给 alt、标题这类地方用。
    pub fn plain_text(&self) -> String {
        match self {
            Inline::Plain(s) => s.clone(),
            Inline::Runs(rs) => rs
                .iter()
                .map(|r| match r {
                    RunOrText::Text(s) => s.as_str(),
                    RunOrText::Run(r) => r.text.as_str(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentProps {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HeadingProps {
    #[schemars(range(min = 1, max = 6))]
    pub level: u8,
    pub text: Inline,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextProps {
    pub text: Inline,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CodeProps {
    pub code: String,
    /// 语言名，只做标记
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default)]
    pub line_numbers: bool,
}

/// 列表项：文字，或带子列表的 { text, items, ordered }。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ListItem {
    Plain(Inline),
    Nested(NestedItem),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NestedItem {
    pub text: Inline,
    #[serde(default)]
    pub items: Vec<ListItem>,
    /// 子列表要不要编号，不写跟上一级一样
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordered: Option<bool>,
}

impl ListItem {
    pub fn text(&self) -> &Inline {
        match self {
            ListItem::Plain(t) => t,
            ListItem::Nested(n) => &n.text,
        }
    }
    pub fn text_mut(&mut self) -> &mut Inline {
        match self {
            ListItem::Plain(t) => t,
            ListItem::Nested(n) => &mut n.text,
        }
    }
    pub fn children(&self) -> &[ListItem] {
        match self {
            ListItem::Plain(_) => &[],
            ListItem::Nested(n) => &n.items,
        }
    }
    pub fn children_mut(&mut self) -> &mut [ListItem] {
        match self {
            ListItem::Plain(_) => &mut [],
            ListItem::Nested(n) => &mut n.items,
        }
    }
    pub fn ordered(&self) -> Option<bool> {
        match self {
            ListItem::Plain(_) => None,
            ListItem::Nested(n) => n.ordered,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListProps {
    #[serde(default)]
    pub ordered: bool,
    pub items: Vec<ListItem>,
}

/// 哪些表格部位生效，对应 Word 的 w:tblLook。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TableLook {
    #[serde(default = "yes")]
    pub first_row: bool,
    #[serde(default)]
    pub last_row: bool,
    #[serde(default = "yes")]
    pub first_column: bool,
    #[serde(default)]
    pub last_column: bool,
    #[serde(default = "yes")]
    pub banded_rows: bool,
    #[serde(default)]
    pub banded_columns: bool,
}

fn yes() -> bool {
    true
}

impl Default for TableLook {
    fn default() -> Self {
        TableLook {
            first_row: true,
            last_row: false,
            first_column: true,
            last_column: false,
            banded_rows: true,
            banded_columns: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TableProps {
    /// 表头；有表头时它就是 firstRow
    #[serde(default)]
    pub columns: Vec<Cell>,
    /// 每行只写从这一行开始的格子，被上面的格子 rowSpan 占掉的位置不写（同 HTML）
    pub rows: Vec<Vec<Cell>>,
    /// 各列宽度：数字是比例，分掉固定列和 auto 列剩下的宽度；
    /// 带单位的字符串是固定宽度（"5.65pt"、"20mm"）；"auto" 按内容撑开。
    /// 没有比例列时表格不撑满，宽度等于各列相加。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widths: Option<Vec<crate::style::ColWidth>>,
    #[serde(default)]
    pub look: TableLook,
    /// 各行高度，按行算（有表头时表头是第一项）：长度是最小高度，
    /// `{"height": "20pt", "rule": "exact"}` 是固定高度，null 不限
    #[serde(
        rename = "rowHeights",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub row_heights: Option<Vec<Option<RowHeight>>>,
}

/// 表格的一格：文字（字符串或 runs），或写成对象带上合并和单格样式。
// 一张表格的格子不多，不值得为大小差异装箱
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Cell {
    Text(Inline),
    Full(CellObj),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellObj {
    #[serde(default)]
    pub text: Inline,
    /// 往下合并几行（含自己），默认 1
    #[serde(rename = "rowSpan", default, skip_serializing_if = "Option::is_none")]
    pub row_span: Option<u32>,
    /// 往右合并几列（含自己），默认 1
    #[serde(rename = "colSpan", default, skip_serializing_if = "Option::is_none")]
    pub col_span: Option<u32>,
    /// 只给这一格的样式：样式名、样式名数组或样式对象，叠在部位样式之后
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<Value>")]
    pub style: Option<Value>,
    /// 算好的样式，渲染器用它，不从 JSON 读
    #[serde(skip)]
    #[schemars(skip)]
    pub computed: Style,
}

impl Cell {
    pub fn text(&self) -> &Inline {
        match self {
            Cell::Text(t) => t,
            Cell::Full(c) => &c.text,
        }
    }

    pub fn text_mut(&mut self) -> &mut Inline {
        match self {
            Cell::Text(t) => t,
            Cell::Full(c) => &mut c.text,
        }
    }

    pub fn row_span(&self) -> usize {
        match self {
            Cell::Full(c) => c.row_span.unwrap_or(1).max(1) as usize,
            Cell::Text(_) => 1,
        }
    }

    pub fn col_span(&self) -> usize {
        match self {
            Cell::Full(c) => c.col_span.unwrap_or(1).max(1) as usize,
            Cell::Text(_) => 1,
        }
    }

    /// 单格样式（算好的），没写是空样式
    pub fn style(&self) -> Option<&Style> {
        match self {
            Cell::Full(c) if c.style.is_some() => Some(&c.computed),
            _ => None,
        }
    }
}

/// 行高：最小高度或固定高度，pt。JSON 里写长度（最小高度），
/// 或 `{"height": "20pt", "rule": "exact"}`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowHeight {
    pub height: f32,
    pub exact: bool,
}

impl<'de> Deserialize<'de> for RowHeight {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let v = Value::deserialize(d)?;
        let len = |v: &Value| -> Result<f32, D::Error> {
            match crate::style::Len::from_value(v).map_err(D::Error::custom)? {
                crate::style::Len::Pt(x) if x >= 0.0 => Ok(x),
                _ => Err(D::Error::custom("行高要写绝对长度（如 \"20pt\"、\"8mm\"）")),
            }
        };
        match &v {
            Value::Object(m) => {
                for k in m.keys() {
                    if k != "height" && k != "rule" {
                        return Err(D::Error::custom(format!("行高里没有 `{k}`")));
                    }
                }
                let h = len(m
                    .get("height")
                    .ok_or_else(|| D::Error::custom("行高要写 height"))?)?;
                let exact = match m.get("rule").and_then(Value::as_str) {
                    None | Some("atLeast") => false,
                    Some("exact") => true,
                    Some(o) => {
                        return Err(D::Error::custom(format!(
                            "行高 rule 只能是 atLeast 或 exact，不是 `{o}`"
                        )));
                    }
                };
                Ok(RowHeight { height: h, exact })
            }
            _ => Ok(RowHeight {
                height: len(&v)?,
                exact: false,
            }),
        }
    }
}

impl Serialize for RowHeight {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let h = format!("{}pt", self.height);
        if self.exact {
            serde_json::json!({"height": h, "rule": "exact"}).serialize(s)
        } else {
            h.serialize(s)
        }
    }
}

impl JsonSchema for RowHeight {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "RowHeight".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> Schema {
        schemars::json_schema!({ "anyOf": [
            { "type": "number" },
            { "type": "string" },
            { "type": "object", "properties": {
                "height": { "anyOf": [{ "type": "number" }, { "type": "string" }] },
                "rule": { "enum": ["atLeast", "exact"] }
            }, "required": ["height"], "additionalProperties": false }
        ] })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageProps {
    pub src: String,
    #[serde(default)]
    pub alt: String,
    /// 宽，pt
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// 高，pt
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageBreakProps {}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlideProps {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// 一节：可以换纸张、页边距、分栏、页眉页脚，重排页码。没写的字段沿用上一节。
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SectionProps {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<crate::doc::PageSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landscape: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin: Option<crate::style::Len>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_top: Option<crate::style::Len>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_right: Option<crate::style::Len>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_bottom: Option<crate::style::Len>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_left: Option<crate::style::Len>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gutter: Option<crate::style::Len>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<crate::doc::Columns>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_distance: Option<crate::style::Len>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer_distance: Option<crate::style::Len>,
    /// 页眉：元素 id，或 { default, first, even }；写 null 清掉，不写沿用上一节
    #[serde(
        default,
        deserialize_with = "crate::doc::patch",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<crate::doc::HfDef>")]
    pub header: Option<Option<crate::doc::HfDef>>,
    #[serde(
        default,
        deserialize_with = "crate::doc::patch",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<crate::doc::HfDef>")]
    pub footer: Option<Option<crate::doc::HfDef>>,
    /// 本节页码从几重新开始
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_number_start: Option<u32>,
    /// 算好的版面，建树时填
    #[serde(skip)]
    #[schemars(skip)]
    pub layout: Option<Box<crate::doc::SectionLayout>>,
}

impl SectionProps {
    /// 建好树以后一定有。
    pub fn layout(&self) -> &crate::doc::SectionLayout {
        self.layout.as_deref().expect("Section 的版面没算")
    }
}

#[derive(Debug, Clone)]
pub enum Component {
    Document(DocumentProps),
    Heading(HeadingProps),
    Text(TextProps),
    Code(CodeProps),
    List(ListProps),
    Table(TableProps),
    Image(ImageProps),
    PageBreak(PageBreakProps),
    Slide(SlideProps),
    Section(SectionProps),
}

impl Component {
    pub fn name(&self) -> &'static str {
        match self {
            Component::Document(_) => "Document",
            Component::Heading(_) => "Heading",
            Component::Text(_) => "Text",
            Component::Code(_) => "Code",
            Component::List(_) => "List",
            Component::Table(_) => "Table",
            Component::Image(_) => "Image",
            Component::PageBreak(_) => "PageBreak",
            Component::Slide(_) => "Slide",
            Component::Section(_) => "Section",
        }
    }

    /// 组件里所有文字类 props，按出现顺序。
    pub fn inlines_mut(&mut self) -> Vec<&mut Inline> {
        fn list<'a>(items: &'a mut [ListItem], out: &mut Vec<&'a mut Inline>) {
            for it in items {
                match it {
                    ListItem::Plain(t) => out.push(t),
                    ListItem::Nested(n) => {
                        out.push(&mut n.text);
                        list(&mut n.items, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        match self {
            Component::Heading(p) => out.push(&mut p.text),
            Component::Text(p) => out.push(&mut p.text),
            Component::List(p) => list(&mut p.items, &mut out),
            Component::Table(p) => {
                out.extend(p.columns.iter_mut().map(Cell::text_mut));
                for r in &mut p.rows {
                    out.extend(r.iter_mut().map(Cell::text_mut));
                }
            }
            _ => {}
        }
        out
    }
}

pub struct Spec {
    pub name: &'static str,
    pub description: &'static str,
    /// 能不能有 children
    pub container: bool,
    /// 能单独设样式的部位
    pub parts: &'static [&'static str],
    pub schema: fn() -> Schema,
}

pub const TABLE_PARTS: &[&str] = &[
    "table",
    "cell",
    "header",
    "firstRow",
    "lastRow",
    "firstColumn",
    "lastColumn",
    "oddRow",
    "evenRow",
    "oddColumn",
    "evenColumn",
    "topLeftCell",
    "topRightCell",
    "bottomLeftCell",
    "bottomRightCell",
];

pub const LIST_PARTS: &[&str] = &[
    "level1", "level2", "level3", "level4", "level5", "level6", "level7", "level8", "level9",
    "marker", "item",
];

pub const BUILTINS: &[Spec] = &[
    Spec {
        name: "Document",
        description: "根容器，按顺序排列子元素",
        container: true,
        parts: &[],
        schema: || schema_for!(DocumentProps),
    },
    Spec {
        name: "Heading",
        description: "标题，level 1 到 6",
        container: false,
        parts: &[],
        schema: || schema_for!(HeadingProps),
    },
    Spec {
        name: "Text",
        description: "一段正文，可以是纯文本或富文本 runs（runs 里可以放 InlineCode）",
        container: false,
        parts: &[],
        schema: || schema_for!(TextProps),
    },
    Spec {
        name: "Code",
        description: "整段代码，保留空格和换行",
        container: false,
        parts: &[],
        schema: || schema_for!(CodeProps),
    },
    Spec {
        name: "List",
        description: "列表，ordered 为 true 时带编号；项可以带子列表",
        container: false,
        parts: LIST_PARTS,
        schema: || schema_for!(ListProps),
    },
    Spec {
        name: "Table",
        description: "表格，columns 是表头，rows 是数据行",
        container: false,
        parts: TABLE_PARTS,
        schema: || schema_for!(TableProps),
    },
    Spec {
        name: "Image",
        description: "图片，src 是本地路径或链接",
        container: false,
        parts: &["caption"],
        schema: || schema_for!(ImageProps),
    },
    Spec {
        name: "PageBreak",
        description: "分页",
        container: false,
        parts: &[],
        schema: || schema_for!(PageBreakProps),
    },
    Spec {
        name: "Section",
        description: "一节，只能直接放在 root 下面：可以换纸张、方向、页边距、分栏、页眉页脚，页码可以重新开始；没写的沿用上一节",
        container: true,
        parts: &[],
        schema: || schema_for!(SectionProps),
    },
    Spec {
        name: "Slide",
        description: "一页幻灯片（16:9），子元素排在里面",
        container: true,
        parts: &["title", "body"],
        schema: || schema_for!(SlideProps),
    },
];

/// 不能当元素、只用来挂样式的组件名。
pub const STYLE_ONLY: &[&str] = &["InlineCode"];

pub fn spec(name: &str) -> Option<&'static Spec> {
    BUILTINS.iter().find(|s| s.name == name)
}

pub fn parse(kind: &str, props: &serde_json::Value) -> anyhow::Result<Component> {
    let p = if props.is_null() {
        serde_json::json!({})
    } else {
        props.clone()
    };
    let mut c = match kind {
        "Document" => Component::Document(serde_json::from_value(p)?),
        "Heading" => {
            let h: HeadingProps = serde_json::from_value(p)?;
            if !(1..=6).contains(&h.level) {
                anyhow::bail!("level 要在 1 到 6 之间，现在是 {}", h.level);
            }
            Component::Heading(h)
        }
        "Text" => Component::Text(serde_json::from_value(p)?),
        "Code" => Component::Code(serde_json::from_value(p)?),
        "List" => Component::List(serde_json::from_value(p)?),
        "Table" => Component::Table(serde_json::from_value(p)?),
        "Image" => Component::Image(serde_json::from_value(p)?),
        "PageBreak" => Component::PageBreak(serde_json::from_value(p)?),
        "Slide" => Component::Slide(serde_json::from_value(p)?),
        "Section" => Component::Section(serde_json::from_value(p)?),
        "InlineCode" => anyhow::bail!("InlineCode 只能写在文字的 runs 里，不能单独当元素"),
        other => anyhow::bail!("组件 `{other}` 没注册"),
    };
    for t in c.inlines_mut() {
        t.normalize();
    }
    Ok(c)
}
