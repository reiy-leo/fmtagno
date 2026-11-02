//! 文档模型：沿用 json-render 的扁平 elements + root。

use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

use anyhow::{Context, Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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

/// 纸张，对应 Word 的 w:sectPr。也是第一节的设置，后面的 Section 在它上面改。
#[derive(Debug, Clone, Deserialize)]
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
    /// 页眉：元素 id（每页一样），或 { default, first, even }
    #[serde(default)]
    pub header: Option<HfDef>,
    #[serde(default)]
    pub footer: Option<HfDef>,
    /// 页眉顶端离纸张上边的距离，默认 0.5in（Word 的默认值）
    #[serde(default)]
    pub header_distance: Option<Len>,
    /// 页脚底端离纸张下边的距离
    #[serde(default)]
    pub footer_distance: Option<Len>,
    /// 页码从几开始
    #[serde(default)]
    pub page_number_start: Option<u32>,
}

/// 页眉或页脚：一个元素 id 表示每页都用它（只改 default），
/// 或者分开写 default、first（本节第一页）、even（偶数页）。
/// 分开写时，写 null 表示这一种留空，不写表示沿用上一节。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum HfDef {
    Id(String),
    Parts(HfParts),
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HfParts {
    #[serde(default, deserialize_with = "patch", skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    pub default: Option<Option<String>>,
    #[serde(default, deserialize_with = "patch", skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    pub first: Option<Option<String>>,
    #[serde(default, deserialize_with = "patch", skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    pub even: Option<Option<String>>,
}

/// 区分「没写」（None）和「写了 null」（Some(None)）。
pub fn patch<'de, D, T>(d: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// 一节里页眉（或页脚）三种页各用哪个元素，按节往下累积。
/// 外层 None 是没写过；Some(None) 是写明留空。
#[derive(Debug, Clone, Default)]
struct HfSet {
    default: Option<Option<String>>,
    first: Option<Option<String>>,
    even: Option<Option<String>>,
}

impl HfSet {
    fn apply(&mut self, d: &HfDef) {
        match d {
            HfDef::Id(id) => self.default = Some(Some(id.clone())),
            HfDef::Parts(p) => {
                if let Some(v) = &p.default {
                    self.default = Some(v.clone());
                }
                if let Some(v) = &p.first {
                    self.first = Some(v.clone());
                }
                if let Some(v) = &p.even {
                    self.even = Some(v.clone());
                }
            }
        }
    }
    fn ids(&self) -> impl Iterator<Item = &String> {
        [&self.default, &self.first, &self.even]
            .into_iter()
            .filter_map(|x| x.as_ref().and_then(|x| x.as_ref()))
    }
}

/// 一节的页眉或页脚，已经建好元素树。
/// `first` 有值说明本节首页单独设置（Word 的 titlePg），值为 None 表示首页留空；
/// `even` 只在文档开了奇偶页不同时有意义。
#[derive(Debug, Clone, Default)]
pub struct HfNodes {
    pub default: Option<Node>,
    pub first: Option<Option<Node>>,
    pub even: Option<Option<Node>>,
}

impl HfNodes {
    pub fn is_empty(&self) -> bool {
        self.default.is_none()
            && self.first.as_ref().is_none_or(|x| x.is_none())
            && self.even.as_ref().is_none_or(|x| x.is_none())
    }
    /// 本节首页用的：首页单独设置就用它，否则跟普通页一样（奇数页）。
    pub fn for_first(&self) -> Option<&Node> {
        match &self.first {
            Some(f) => f.as_ref(),
            None => self.default.as_ref(),
        }
    }
    /// 偶数页用的。
    pub fn for_even(&self) -> Option<&Node> {
        match &self.even {
            Some(e) => e.as_ref(),
            None => self.default.as_ref(),
        }
    }
}

/// 一节算好的版面：纸张、页眉页脚、页码。由 `Doc::tree_with` 填进每个 Section 节点。
#[derive(Debug, Clone)]
pub struct SectionLayout {
    /// 第几节，从 0 开始
    pub index: usize,
    /// 已经叠加过前面各节的设置；header/footer 字段不用，看下面两个
    pub page: Page,
    pub header: HfNodes,
    pub footer: HfNodes,
    /// 首页单独设置（Word 的 titlePg）
    pub title_page: bool,
    /// 整份文档奇偶页不同（Word 的 evenAndOddHeaders）
    pub even_odd: bool,
    /// 本节页码从几重新开始
    pub page_number_start: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum PageSize {
    Named(String),
    Custom { width: Len, height: Len },
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
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
            header: None,
            footer: None,
            header_distance: None,
            footer_distance: None,
            page_number_start: None,
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

    /// 页眉、页脚离纸边的距离，pt。
    pub fn hf_distance_pt(&self) -> (f32, f32) {
        (
            self.header_distance.map(|l| l.abs_pt()).unwrap_or(36.0),
            self.footer_distance.map(|l| l.abs_pt()).unwrap_or(36.0),
        )
    }

    /// 在这份设置上叠加一节写的改动。
    pub fn apply(&mut self, o: &crate::components::SectionProps) {
        macro_rules! set {
            ($($f:ident),*) => { $( if let Some(v) = &o.$f { self.$f = v.clone().into(); } )* };
        }
        set!(size, landscape, margin);
        macro_rules! opt {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f.clone(); } )* };
        }
        opt!(
            margin_top,
            margin_right,
            margin_bottom,
            margin_left,
            gutter,
            columns,
            header_distance,
            footer_distance
        );
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
#[derive(Debug, Clone)]
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
    /// 建好后 root 下面一律是 Section：没写 Section 的内容归进一个隐含的节。
    pub fn tree_with(&self, sheet: &Sheet) -> Result<Node> {
        let mut seen = HashSet::new();
        let mut node = self.build(&self.root, sheet.root_style(), sheet, &mut seen, 0)?;
        self.sections(&mut node, sheet, &mut seen)?;
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

    /// 把 root 的子元素整理成一节节，算好每节的纸张和页眉页脚。
    fn sections<'a>(
        &'a self,
        root: &mut Node,
        sheet: &Sheet,
        seen: &mut HashSet<&'a str>,
    ) -> Result<()> {
        if !matches!(root.component, Component::Document(_)) {
            // root 不是 Document：外面补一个，让结构和平常一样
            let inner = std::mem::replace(
                root,
                Node {
                    id: format!("{}#document", self.root),
                    component: Component::Document(Default::default()),
                    style: sheet.root_style().clone(),
                    parts: BTreeMap::new(),
                    children: Vec::new(),
                },
            );
            root.children.push(inner);
        }
        // 开头不在 Section 里的内容算第一节
        let kids = std::mem::take(&mut root.children);
        let mut out: Vec<Node> = Vec::new();
        let mut loose: Vec<Node> = Vec::new();
        for k in kids {
            if matches!(k.component, Component::Section(_)) {
                if !loose.is_empty() {
                    if !out.is_empty() {
                        bail!(
                            "元素 `{}`：Section 后面的内容要放进 Section 里（root 下面要么全是 Section，要么只有开头一段不在 Section 里）",
                            loose[0].id
                        );
                    }
                    out.push(self.implicit_section(root, std::mem::take(&mut loose)));
                }
                out.push(k);
            } else {
                loose.push(k);
            }
        }
        if !loose.is_empty() || out.is_empty() {
            if !out.is_empty() {
                bail!(
                    "元素 `{}`：Section 后面的内容要放进 Section 里（root 下面要么全是 Section，要么只有开头一段不在 Section 里）",
                    loose[0].id
                );
            }
            out.push(self.implicit_section(root, loose));
        }

        // 逐节叠加纸张和页眉页脚设置
        let mut page = self.meta.page.clone();
        page.header = None;
        page.footer = None;
        page.page_number_start = None;
        let mut header = HfSet::default();
        let mut footer = HfSet::default();
        if let Some(h) = &self.meta.page.header {
            header.apply(h);
        }
        if let Some(f) = &self.meta.page.footer {
            footer.apply(f);
        }
        let mut sets = Vec::with_capacity(out.len());
        for (i, sec) in out.iter().enumerate() {
            let Component::Section(p) = &sec.component else {
                unreachable!()
            };
            let mut start = if i == 0 {
                self.meta.page.page_number_start
            } else {
                None
            };
            if p.page_number_start.is_some() {
                start = p.page_number_start;
            }
            page.apply(p);
            match &p.header {
                Some(Some(h)) => header.apply(h),
                Some(None) => header = HfSet::default(),
                None => {}
            }
            match &p.footer {
                Some(Some(f)) => footer.apply(f),
                Some(None) => footer = HfSet::default(),
                None => {}
            }
            sets.push((page.clone(), header.clone(), footer.clone(), start));
        }
        let even_odd = sets.iter().any(|(_, h, f, _)| {
            h.even.as_ref().is_some_and(|e| e.is_some()) || f.even.as_ref().is_some_and(|e| e.is_some())
        });

        // 页眉页脚里的元素：每个只建一次，各节共用
        let mut built: BTreeMap<String, Node> = BTreeMap::new();
        for (_, h, f, _) in &sets {
            for id in h.ids().chain(f.ids()) {
                if built.contains_key(id) {
                    continue;
                }
                let (key, _) = self
                    .elements
                    .get_key_value(id.as_str())
                    .ok_or_else(|| anyhow::anyhow!("页眉页脚用的元素 `{id}` 不存在"))?;
                let n = self
                    .build(key, &root.style, sheet, seen, 2)
                    .with_context(|| format!("在页眉页脚 `{id}` 里"))?;
                check_hf(&n).with_context(|| format!("在页眉页脚 `{id}` 里"))?;
                built.insert(id.clone(), n);
            }
        }
        let pick = |x: &Option<Option<String>>| -> Option<Option<Node>> {
            x.as_ref().map(|o| o.as_ref().map(|id| built[id].clone()))
        };
        for (i, (sec, (page, h, f, start))) in out.iter_mut().zip(sets).enumerate() {
            let title_page = h.first.is_some() || f.first.is_some();
            let nodes = |s: &HfSet| HfNodes {
                default: pick(&s.default).flatten(),
                first: if title_page {
                    Some(match &s.first {
                        Some(x) => x.as_ref().map(|id| built[id].clone()),
                        None => pick(&s.default).flatten(),
                    })
                } else {
                    None
                },
                even: if even_odd {
                    Some(match &s.even {
                        Some(x) => x.as_ref().map(|id| built[id].clone()),
                        None => pick(&s.default).flatten(),
                    })
                } else {
                    None
                },
            };
            let layout = SectionLayout {
                index: i,
                page,
                header: nodes(&h),
                footer: nodes(&f),
                title_page,
                even_odd,
                page_number_start: start,
            };
            if let Component::Section(p) = &mut sec.component {
                p.layout = Some(Box::new(layout));
            }
        }
        root.children = out;
        Ok(())
    }

    fn implicit_section(&self, root: &Node, children: Vec<Node>) -> Node {
        Node {
            id: format!("{}#section", root.id),
            component: Component::Section(Default::default()),
            style: root.style.clone(),
            parts: BTreeMap::new(),
            children,
        }
    }

    fn build<'a>(
        &'a self,
        id: &'a str,
        parent: &Style,
        sheet: &Sheet,
        seen: &mut HashSet<&'a str>,
        depth: usize,
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
        if el.kind == "Section" && depth != 1 {
            bail!("元素 `{id}`：Section 只能直接放在 root 下面");
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
                self.build(c, &style, sheet, seen, depth + 1)
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

/// 页眉页脚里不能分页、分节。
fn check_hf(n: &Node) -> Result<()> {
    match n.component {
        Component::PageBreak(_) | Component::Section(_) | Component::Slide(_) => {
            bail!("元素 `{}`：页眉页脚里不能放 {}", n.id, n.component.name())
        }
        _ => {}
    }
    for c in &n.children {
        check_hf(c)?;
    }
    Ok(())
}
