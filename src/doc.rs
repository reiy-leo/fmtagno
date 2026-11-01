//! 文档模型：沿用 json-render 的扁平 elements + root。

use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    cascade::{Sheet, StyleSetDef},
    components::{self, Component},
    style::{Len, Style},
    units,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Doc {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub meta: Meta,
    /// 用哪个样式集
    #[serde(default)]
    pub style_set: Option<String>,
    #[serde(default)]
    pub style_sets: BTreeMap<String, StyleSetDef>,
    /// 不属于任何样式集的样式：键是组件名（如 `Heading.1`、`Table.header`）就是组件样式，否则是命名样式
    #[serde(default)]
    pub styles: BTreeMap<String, Value>,
    /// 文档里直接写的自定义组件定义，和定义文件写法一样
    #[serde(default)]
    pub components: Vec<crate::custom::Def>,
    pub root: String,
    pub elements: BTreeMap<String, Element>,
    /// 自定义组件名，展开时填，用来分辨 styles 的键
    #[serde(skip)]
    pub custom_names: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Meta {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub page: Page,
}

/// 纸张，对应 Word 的 w:sectPr。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Page {
    /// A4、A5、Letter 等，或 { width, height }
    #[serde(default = "default_size")]
    pub size: PageSize,
    #[serde(default)]
    pub landscape: bool,
    #[serde(default = "default_margin")]
    pub margin: Len,
    #[serde(default)]
    pub margin_top: Option<Len>,
    #[serde(default)]
    pub margin_right: Option<Len>,
    #[serde(default)]
    pub margin_bottom: Option<Len>,
    #[serde(default)]
    pub margin_left: Option<Len>,
    /// 装订线
    #[serde(default)]
    pub gutter: Option<Len>,
    /// 分栏：数字，或 { count, gap }
    #[serde(default)]
    pub columns: Option<Columns>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PageSize {
    Named(String),
    Custom { width: Len, height: Len },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Columns {
    Count(u8),
    Full {
        count: u8,
        #[serde(default)]
        gap: Option<Len>,
    },
}

impl Columns {
    pub fn count(&self) -> u8 {
        match self {
            Columns::Count(c) | Columns::Full { count: c, .. } => (*c).max(1),
        }
    }
    pub fn gap_pt(&self) -> f32 {
        match self {
            Columns::Full { gap: Some(g), .. } => g.abs_pt(),
            _ => 21.0,
        }
    }
}

fn default_size() -> PageSize {
    PageSize::Named("A4".into())
}
fn default_margin() -> Len {
    Len::parse("20mm").unwrap()
}
impl Default for Page {
    fn default() -> Self {
        Page {
            size: default_size(),
            landscape: false,
            margin: default_margin(),
            margin_top: None,
            margin_right: None,
            margin_bottom: None,
            margin_left: None,
            gutter: None,
            columns: None,
        }
    }
}

impl Page {
    /// 宽高，pt，已按横向翻转。纸张名不认识时给警告并按 A4。
    pub fn dims_pt(&self, warnings: &mut Vec<String>) -> (f32, f32) {
        let (w, h) = match &self.size {
            PageSize::Named(n) => units::page_size_pt(n).unwrap_or_else(|| {
                warnings.push(format!("不认识的纸张 `{n}`，按 A4 处理"));
                units::page_size_pt("A4").unwrap()
            }),
            PageSize::Custom { width, height } => (width.abs_pt(), height.abs_pt()),
        };
        if self.landscape { (h, w) } else { (w, h) }
    }

    /// 上右下左页边距，pt。
    pub fn margins_pt(&self) -> [f32; 4] {
        let m = self.margin.abs_pt();
        let f = |o: &Option<Len>| o.map(|l| l.abs_pt()).unwrap_or(m);
        [
            f(&self.margin_top),
            f(&self.margin_right),
            f(&self.margin_bottom),
            f(&self.margin_left),
        ]
    }

    /// 纸张名（认识的话）。
    pub fn named(&self) -> Option<&str> {
        match &self.size {
            PageSize::Named(n) => Some(n),
            PageSize::Custom { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Element {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub props: Value,
    /// 样式名、样式名数组或样式对象
    #[serde(default)]
    pub style: Option<Value>,
    /// 部位样式：键是部位名（如表格的 header、evenRow）
    #[serde(default)]
    pub parts: BTreeMap<String, Value>,
    #[serde(default)]
    pub children: Vec<String>,
    /// 来自自定义组件模板的样式，按先后叠加（内层模板在前）
    #[serde(skip)]
    pub template_styles: Vec<Value>,
    #[serde(skip)]
    pub template_parts: Vec<BTreeMap<String, Value>>,
    /// 这个元素是从哪些自定义组件展开出来的（内层在前）
    #[serde(skip)]
    pub origins: Vec<Origin>,
}

/// 展开来源：哪个自定义组件的哪个模板元素。
#[derive(Debug, Clone)]
pub struct Origin {
    pub component: String,
    pub element: String,
    /// 是不是模板根元素
    pub root: bool,
}

/// 校验并解析好的元素树，渲染器只读它。
#[derive(Debug)]
pub struct Node {
    pub id: String,
    pub component: Component,
    /// 算好的完整样式：已经含继承来的字段
    pub style: Style,
    /// 部位样式：只含为这个部位写的字段，渲染时叠在 `style` 上
    pub parts: BTreeMap<String, Style>,
    pub children: Vec<Node>,
}

impl Node {
    /// 部位样式；没写就是空的。
    pub fn part(&self, name: &str) -> Style {
        self.parts.get(name).cloned().unwrap_or_default()
    }
}

pub fn load(path: &Path) -> Result<Doc> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("读不了 {}", path.display()))?;
    parse(&text).with_context(|| format!("{} 不是合法的 fmtagno JSON", path.display()))
}

pub fn parse(text: &str) -> Result<Doc> {
    Ok(serde_json::from_str(text)?)
}

impl Doc {
    /// 用文档自带的样式算元素树。
    pub fn tree(&self) -> Result<Node> {
        let sheet = Sheet::build(self, &BTreeMap::new(), None)?;
        self.tree_with(&sheet)
    }

    /// 校验全部元素，按样式表算好样式，并从 root 建出元素树。
    pub fn tree_with(&self, sheet: &Sheet) -> Result<Node> {
        let mut seen = HashSet::new();
        let node = self.build(&self.root, sheet.root_style(), sheet, &mut seen)?;
        let unused: Vec<_> = self
            .elements
            .keys()
            .filter(|k| !seen.contains(k.as_str()))
            .collect();
        if !unused.is_empty() {
            eprintln!("警告：这些元素没被引用，不会渲染：{unused:?}");
        }
        Ok(node)
    }

    fn build<'a>(
        &'a self,
        id: &'a str,
        parent: &Style,
        sheet: &Sheet,
        seen: &mut HashSet<&'a str>,
    ) -> Result<Node> {
        let el = self
            .elements
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("元素 `{id}` 不存在"))?;
        if !seen.insert(id) {
            bail!("元素 `{id}` 被引用了不止一次（或者成环了）");
        }
        if components::STYLE_ONLY.contains(&el.kind.as_str()) {
            bail!(
                "元素 `{id}`：{} 只能写在文字的 runs 里，不能单独当元素",
                el.kind
            );
        }
        let spec = components::spec(&el.kind)
            .ok_or_else(|| anyhow::anyhow!("元素 `{id}` 的组件 `{}` 没注册", el.kind))?;
        if !spec.container && !el.children.is_empty() {
            bail!("元素 `{id}`：{} 不能有 children", el.kind);
        }
        let mut component = components::parse(&el.kind, &el.props)
            .with_context(|| format!("元素 `{id}`（{}）的 props 不对", el.kind))?;
        let (style, parts) = sheet
            .element(el, spec, &component, parent)
            .with_context(|| format!("元素 `{id}` 的样式不对"))?;
        for t in component.inlines_mut() {
            for r in t.runs_mut() {
                r.computed = sheet
                    .run(r, &style)
                    .with_context(|| format!("元素 `{id}` 里文字 `{}` 的样式不对", r.text))?;
            }
        }
        let mut children = Vec::with_capacity(el.children.len());
        for c in &el.children {
            children.push(
                self.build(c, &style, sheet, seen)
                    .with_context(|| format!("在 `{id}` 的子元素里"))?,
            );
        }
        Ok(Node {
            id: id.to_string(),
            component,
            style,
            parts,
            children,
        })
    }
}
