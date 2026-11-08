//! 从 Word 的 .docx 解析出 fmtagno 的 JSON：标题、段落和行内样式、列表、表格、图片、分节、
//! 页眉页脚、页码、分页、公式，Word 样式表转成 `styles`。解不了的内容给警告，不中断。
//!
//! 样式的思路：先按 Word 的规则算出每段、每个 run 实际生效的格式（docDefaults → 样式链 →
//! 编号 → 直接格式），再按 fmtagno 的层叠规则算出「不写时会是什么」，只写不一样的字段。
//! 这样 parse 出来的 JSON 再 render 回 docx，格式基本不变，JSON 也不会每段都带一大串样式。

mod omml;
mod table;
mod word;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;

use anyhow::{Context, Result, bail};
use roxmltree::{Document, Node};
use serde_json::{Map, Value, json};

use crate::cascade::builtin;
use crate::components::Field;
use crate::style::{self, Len, ListStyleType, Style};
use word::{
    A, Lvl, M, Numbering, PPr, R, Styles, Theme, V, W, WP, child, child_ns, is, isw, round2, wa,
    wval,
};

/// parse 的选项。
#[derive(Debug, Clone)]
pub struct Options {
    /// JSON 里图片路径的前缀，比如 `report.assets`；图片文件由调用方写进这个目录
    pub assets_dir: String,
}

/// parse 的结果。
#[derive(Debug)]
pub struct Parsed {
    pub json: Value,
    /// (文件名, 内容)：要写进 `assets_dir` 的图片
    pub assets: Vec<(String, Vec<u8>)>,
    pub warnings: Vec<String>,
}

/// 警告，同一件事只报一次。
#[derive(Debug, Default)]
pub struct Warns {
    pub list: Vec<String>,
    seen: HashSet<String>,
}

impl Warns {
    pub fn push(&mut self, m: String) {
        self.list.push(m);
    }
    pub fn once(&mut self, key: &str, m: String) {
        if self.seen.insert(key.to_string()) {
            self.list.push(m);
        }
    }
}

/// 解析一个 docx 文件的字节。
pub fn docx(bytes: &[u8], opts: &Options) -> Result<Parsed> {
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("不是 zip 文件，不像 docx")?;
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        if f.is_dir() {
            continue;
        }
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        files.insert(f.name().trim_start_matches('/').to_string(), buf);
    }
    let main = main_part(&files).unwrap_or_else(|| "word/document.xml".to_string());
    if !files.contains_key(&main) {
        bail!("docx 里没有 {main}，不是 Word 文档");
    }
    let mut ctx = Ctx::new(files, opts.clone(), &main)?;
    let json = ctx.run(&main)?;
    Ok(Parsed {
        json,
        assets: ctx.assets,
        warnings: ctx.warns.list,
    })
}

/// [Content_Types] 和 _rels/.rels 里找主文档。
fn main_part(files: &HashMap<String, Vec<u8>>) -> Option<String> {
    let rels = std::str::from_utf8(files.get("_rels/.rels")?).ok()?;
    let doc = Document::parse(rels).ok()?;
    doc.descendants()
        .find(|n| {
            n.tag_name().name() == "Relationship"
                && n.attribute("Type")
                    .is_some_and(|t| t.ends_with("/officeDocument"))
        })
        .and_then(|n| n.attribute("Target"))
        .map(|t| t.trim_start_matches('/').to_string())
}

#[derive(Debug, Clone)]
struct Rel {
    target: String,
    external: bool,
}

/// 读一个部件的关系表，目标路径换成 zip 里的完整路径。
fn read_rels(files: &HashMap<String, Vec<u8>>, part: &str) -> HashMap<String, Rel> {
    let (dir, name) = match part.rfind('/') {
        Some(i) => (&part[..i], &part[i + 1..]),
        None => ("", part),
    };
    let path = if dir.is_empty() {
        format!("_rels/{name}.rels")
    } else {
        format!("{dir}/_rels/{name}.rels")
    };
    let mut out = HashMap::new();
    let Some(text) = files.get(&path).and_then(|b| std::str::from_utf8(b).ok()) else {
        return out;
    };
    let Ok(doc) = Document::parse(text) else {
        return out;
    };
    for n in doc.descendants().filter(|n| n.tag_name().name() == "Relationship") {
        let (Some(id), Some(t)) = (n.attribute("Id"), n.attribute("Target")) else {
            continue;
        };
        let external = n.attribute("TargetMode") == Some("External");
        let target = if external {
            t.to_string()
        } else if let Some(abs) = t.strip_prefix('/') {
            abs.to_string()
        } else {
            normalize(&format!("{dir}/{t}"))
        };
        out.insert(id.to_string(), Rel { target, external });
    }
    out
}

/// 去掉路径里的 `..` 和 `.`。
fn normalize(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for s in p.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            x => parts.push(x),
        }
    }
    parts.join("/")
}

// ---------- 中间结构 ----------

/// 段落里的一段内容。
#[derive(Debug, Clone)]
enum Seg {
    Text {
        text: String,
        /// 实际生效的文字样式
        style: Style,
        href: Option<String>,
        code: bool,
        cstyle: Option<String>,
    },
    Field {
        field: Field,
        text: String,
        style: Style,
    },
    Image(Img),
    Math {
        latex: String,
        display: bool,
        jc: Option<String>,
        style: Style,
    },
    PageBreak,
}

#[derive(Debug, Clone)]
struct Img {
    src: String,
    w: f32,
    h: f32,
    alt: String,
    rotation: Option<f32>,
    anchor: Option<Anchor>,
}

#[derive(Debug, Clone, Default)]
struct Anchor {
    behind: bool,
    wrap: Option<&'static str>,
    h_rel: String,
    v_rel: String,
    h_align: Option<String>,
    x: f32,
    y: f32,
    dist: f32,
}

/// 一段解析好的段落。
#[derive(Debug, Clone)]
struct Para {
    ppr: PPr,
    run_base: Style,
    pstyle: Option<String>,
    segs: Vec<Seg>,
    list: Option<ListRef>,
    sect: Option<Sect>,
    toc: Option<u8>,
    in_toc: bool,
    /// 直接格式里写了缩进（列表段落用它代替编号定义里的缩进）
    direct_ind: bool,
}

#[derive(Debug, Clone)]
struct ListRef {
    num_id: i64,
    abs: i64,
    ilvl: usize,
    levels: Vec<Lvl>,
}

/// 一节的设置。
#[derive(Debug, Clone, Default)]
struct Sect {
    w: f32,
    h: f32,
    landscape: bool,
    /// 上右下左，pt
    margins: [f32; 4],
    header_d: f32,
    footer_d: f32,
    gutter: f32,
    cols: Option<(u8, f32)>,
    title_pg: bool,
    pg_start: Option<u32>,
    headers: Vec<(String, String)>,
    footers: Vec<(String, String)>,
}

impl Sect {
    fn content_width(&self) -> f32 {
        (self.w - self.margins[1] - self.margins[3] - self.gutter).max(72.0)
    }
}

/// 字段帧：Word 的复杂域（fldChar begin / separate / end）。
#[derive(Debug, Clone)]
struct Frame {
    instr: String,
    result: bool,
    kind: Option<FieldKind>,
    text: String,
    style: Option<Style>,
}

#[derive(Debug, Clone)]
enum FieldKind {
    Page(Field),
    Link(String),
    Toc(u8),
    Other(String),
}

fn classify_field(instr: &str) -> FieldKind {
    let t = instr.trim();
    let mut words = t.split_whitespace();
    let head = words.next().unwrap_or("").to_ascii_uppercase();
    match head.as_str() {
        "PAGE" => FieldKind::Page(Field::Page),
        "NUMPAGES" => FieldKind::Page(Field::Pages),
        "SECTIONPAGES" => FieldKind::Page(Field::SectionPages),
        "HYPERLINK" => {
            let rest = t[4.min(t.len())..].trim_start_matches(|c: char| c.is_ascii_alphabetic());
            let url = rest
                .split('"')
                .nth(1)
                .map(str::to_string)
                .unwrap_or_else(|| rest.split_whitespace().next().unwrap_or("").to_string());
            if rest.contains("\\l") {
                FieldKind::Other("HYPERLINK \\l".into())
            } else {
                FieldKind::Link(url)
            }
        }
        "TOC" => {
            // \o "1-3"
            let levels = t
                .split("\\o")
                .nth(1)
                .and_then(|r| r.split('"').nth(1))
                .and_then(|r| r.split('-').nth(1))
                .and_then(|x| x.trim().parse::<u8>().ok())
                .unwrap_or(3)
                .clamp(1, 6);
            FieldKind::Toc(levels)
        }
        other => FieldKind::Other(other.to_string()),
    }
}

/// 段落里的内容该怎么处理（看当前所在的域）。
enum Gate {
    Drop,
    Capture(usize),
    Emit(Option<String>),
}

// ---------- 样式 JSON 小工具 ----------

/// 段落级字段：不写时 Word 用默认值（0、false、左对齐），所以缺了也能补一个「复位值」。
const PARA_KEYS: &[&str] = &[
    "align",
    "spaceBefore",
    "spaceAfter",
    "lineHeight",
    "marginLeft",
    "marginRight",
    "textIndent",
    "hangingIndent",
    "keepWithNext",
    "keepTogether",
    "pageBreakBefore",
    "widowControl",
    "contextualSpacing",
    "direction",
    "outlineLevel",
    "borderTop",
    "borderRight",
    "borderBottom",
    "borderLeft",
    "borderBetween",
    "paddingTop",
    "paddingRight",
    "paddingBottom",
    "paddingLeft",
    "background",
];

/// 文字字段（不含 background：段落上的 background 是段落底纹）。
const TEXT_KEYS: &[&str] = &[
    "fontFamily",
    "fontSize",
    "color",
    "bold",
    "italic",
    "underline",
    "underlineColor",
    "strike",
    "verticalAlign",
    "caps",
    "letterSpacing",
    "highlight",
    "emphasisMark",
    "scale",
    "kerning",
    "lang",
    "hidden",
];

const LIST_KEYS: &[&str] = &[
    "listStyleType",
    "listMarkerFormat",
    "listMarkerColor",
    "listMarkerFont",
    "listMarkerSize",
    "listMarkerAlign",
    "listMarkerGap",
    "listIndent",
    "listHanging",
    "listStart",
];

/// 某个字段没写时等于什么。只有能写出来的才有。
fn reset(k: &str) -> Option<Value> {
    Some(match k {
        "spaceBefore" | "spaceAfter" | "marginLeft" | "marginRight" | "textIndent"
        | "letterSpacing" => json!(0),
        "bold" | "italic" | "keepWithNext" | "keepTogether" | "pageBreakBefore" | "hidden"
        | "contextualSpacing" | "strike" => json!(false),
        "underline" => json!("none"),
        "align" => json!("left"),
        "color" => json!("#000000"),
        "lineHeight" => json!(1),
        "fontSize" => json!(10),
        "scale" => json!(100),
        "direction" => json!("ltr"),
        "borderTop" | "borderRight" | "borderBottom" | "borderLeft" | "borderBetween" => {
            json!("none")
        }
        "paddingTop" | "paddingRight" | "paddingBottom" | "paddingLeft" => json!(0),
        _ => return None,
    })
}

fn to_map(s: &Style) -> Map<String, Value> {
    let mut m = match serde_json::to_value(s) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    };
    clean_map(&mut m);
    expand(&mut m);
    m
}

/// `border`、`padding` 拆成四边，方便逐个比较。
fn expand(m: &mut Map<String, Value>) {
    for (all, sides) in [
        (
            "border",
            ["borderTop", "borderRight", "borderBottom", "borderLeft"],
        ),
        (
            "padding",
            ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"],
        ),
    ] {
        if let Some(v) = m.remove(all) {
            for s in sides {
                m.entry(s.to_string()).or_insert_with(|| v.clone());
            }
        }
    }
}

/// 四边一样时合回 `border`、`padding`；三个文种一样的字体写成一个名字。
fn compact(m: &mut Map<String, Value>) {
    for (all, sides) in [
        (
            "border",
            ["borderTop", "borderRight", "borderBottom", "borderLeft"],
        ),
        (
            "padding",
            ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"],
        ),
    ] {
        let vals: Vec<Option<&Value>> = sides.iter().map(|s| m.get(*s)).collect();
        if let Some(Some(first)) = vals.first()
            && vals.iter().all(|v| v.is_some_and(|x| same(x, first)))
            && !m.contains_key(all)
        {
            let v = (*first).clone();
            for s in sides {
                m.remove(s);
            }
            m.insert(all.to_string(), v);
        }
    }
    if let Some(Value::Object(f)) = m.get("fontFamily") {
        let l = f.get("latin");
        let e = f.get("eastAsia");
        let c = f.get("complex");
        if l.is_some() && l == e && (c.is_none() || c == l) {
            let v = l.unwrap().clone();
            m.insert("fontFamily".into(), v);
        }
    }
}

/// 两个值相等（数字差一点也算）。
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            (x.as_f64().unwrap_or(0.0) - y.as_f64().unwrap_or(0.0)).abs() < 0.011
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(v, w)| same(v, w))
        }
        _ => a == b,
    }
}

/// 只写 `eff` 和 `exp` 不一样的字段。`keys` 是要比的字段；`soft` 里的字段在 `eff` 里缺了就不管
/// （留给下一层），其余缺了就写复位值。
fn diff(
    eff: &Map<String, Value>,
    exp: &Map<String, Value>,
    keys: &[&str],
    soft: &[&str],
) -> Map<String, Value> {
    let mut out = Map::new();
    for k in keys {
        match (eff.get(*k), exp.get(*k)) {
            (Some(a), Some(b)) if same(a, b) => {}
            (Some(a), None) => {
                if reset(k).is_some_and(|r| same(&r, a)) {
                    continue;
                }
                out.insert(k.to_string(), a.clone());
            }
            (Some(a), Some(_)) => {
                out.insert(k.to_string(), a.clone());
            }
            (None, Some(b)) => {
                if soft.contains(k) {
                    continue;
                }
                if let Some(r) = reset(k)
                    && !same(&r, b)
                {
                    out.insert(k.to_string(), r);
                }
            }
            (None, None) => {}
        }
    }
    out
}

fn from_map(m: &Map<String, Value>) -> Style {
    let mut m = m.clone();
    // 合并前的四边写法能直接解析
    style::parse_object(&Value::Object(std::mem::take(&mut m))).unwrap_or_default()
}

/// 浮点数收拾成最多 4 位小数，整数写成整数。
fn clean(v: &mut Value) {
    match v {
        Value::Number(n) if n.is_f64() => {
            let x = n.as_f64().unwrap_or(0.0);
            let r = (x * 10000.0).round() / 10000.0;
            *v = if r.fract() == 0.0 && r.abs() < 1e15 {
                json!(r as i64)
            } else {
                json!(r)
            };
        }
        Value::Array(a) => a.iter_mut().for_each(clean),
        Value::Object(m) => clean_map(m),
        _ => {}
    }
}

fn clean_map(m: &mut Map<String, Value>) {
    for v in m.values_mut() {
        clean(v);
    }
}

/// 每个字段在所有样式里都一样时取出来。
fn hoist(list: &[Map<String, Value>], keys: &[&str]) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(first) = list.first() else {
        return out;
    };
    for k in keys {
        if let Some(v) = first.get(*k)
            && list.iter().all(|m| m.get(*k).is_some_and(|w| same(v, w)))
        {
            out.insert(k.to_string(), v.clone());
        }
    }
    out
}

fn merged(a: &Style, b: &Style) -> Style {
    let mut s = a.clone();
    s.merge(b);
    s
}

fn text_of(s: &Style) -> Style {
    let mut t = s.text_only();
    t.background = None;
    t
}

/// 渲染器按段落字号看 em：字号，pt。
fn size_of(m: &Map<String, Value>) -> Option<f32> {
    m.get("fontSize").and_then(|v| v.as_f64()).map(|x| x as f32)
}

// ---------- 主体 ----------

struct Ctx {
    files: HashMap<String, Vec<u8>>,
    opts: Options,
    theme: Theme,
    styles: Styles,
    numbering: Numbering,
    warns: Warns,
    elements: Map<String, Value>,
    counters: HashMap<&'static str, usize>,
    assets: Vec<(String, Vec<u8>)>,
    asset_by_target: HashMap<String, String>,
    /// 当前部件的关系表
    rels: HashMap<String, Rel>,
    /// 输出的 styles
    out_styles: Map<String, Value>,
    /// 组件样式（我们写进 styles 的），算「不写时是什么」用
    comp: BTreeMap<String, Style>,
    /// Word 段落样式 id → 命名样式名（None 表示没差别，不用引用）
    named: HashMap<String, Option<String>>,
    named_styles: HashMap<String, Style>,
    char_named: HashMap<String, Option<String>>,
    /// Document 算好以后往下传的样式
    doc_inh: Style,
    frames: Vec<Frame>,
    toc_started: Option<u8>,
    content_width: f32,
    sects: Vec<Sect>,
    sect_index: usize,
    /// 当前节已经收的块
    cur_blocks: Vec<String>,
    done_sects: Vec<(Vec<String>, Sect)>,
    last_table: Option<String>,
    hf_cache: HashMap<String, Option<String>>,
    even_odd: bool,
    in_table: usize,
    /// 当前表格样式的段落、文字属性
    tbl_base: Option<(PPr, Style)>,
    /// 预扫一遍正文段落的有效样式：(种类, 有效样式)，用来定 Document、Text、Heading.N 的样式
    stats: Vec<(Kind, Map<String, Value>)>,
}

/// 预扫时段落的种类。
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Text,
    Heading(u8),
    List,
}

/// 一组样式里某个字段最常见的值；没写的也算一种。
fn majority(list: &[&Map<String, Value>], key: &str) -> Option<Value> {
    let mut best: Option<(Option<&Value>, usize)> = None;
    for m in list {
        let v = m.get(key);
        let n = list
            .iter()
            .filter(|x| match (x.get(key), v) {
                (Some(a), Some(b)) => same(a, b),
                (None, None) => true,
                _ => false,
            })
            .count();
        if best.is_none_or(|(_, b)| n > b) {
            best = Some((v, n));
        }
    }
    best.and_then(|(v, _)| v.cloned())
}

/// 用多数段落的值改 `m` 里的这些字段；多数段落没写的字段从 `m` 里去掉。
fn apply_majority(m: &mut Map<String, Value>, list: &[&Map<String, Value>], keys: &[&str]) {
    if list.is_empty() {
        return;
    }
    for k in keys {
        match majority(list, k) {
            Some(v) => {
                m.insert(k.to_string(), v);
            }
            None => {
                m.remove(*k);
            }
        }
    }
}

impl Ctx {
    fn new(files: HashMap<String, Vec<u8>>, opts: Options, main: &str) -> Result<Ctx> {
        let mut warns = Warns::default();
        let rels = read_rels(&files, main);
        let text = |p: &str| {
            files
                .get(p)
                .map(|b| String::from_utf8_lossy(b).into_owned())
        };
        let find = |suffix: &str| {
            rels.values()
                .find(|r| !r.external && r.target.ends_with(suffix))
                .map(|r| r.target.clone())
        };
        let theme = find("theme1.xml")
            .or_else(|| Some("word/theme/theme1.xml".into()))
            .and_then(|p| text(&p))
            .map(|t| Theme::parse(&t))
            .unwrap_or_default();
        let styles = find("styles.xml")
            .and_then(|p| text(&p))
            .map(|t| Styles::parse(&t, &theme, &mut warns))
            .unwrap_or_default();
        let numbering = find("numbering.xml")
            .and_then(|p| text(&p))
            .map(|t| Numbering::parse(&t, &theme, &mut warns))
            .unwrap_or_default();
        let even_odd = find("settings.xml")
            .and_then(|p| text(&p))
            .is_some_and(|t| {
                Document::parse(&t).is_ok_and(|d| {
                    d.descendants()
                        .any(|n| isw(n, "evenAndOddHeaders") && word::on_off(n))
                })
            });
        Ok(Ctx {
            files,
            opts,
            theme,
            styles,
            numbering,
            warns,
            elements: Map::new(),
            counters: HashMap::new(),
            assets: Vec::new(),
            asset_by_target: HashMap::new(),
            rels,
            out_styles: Map::new(),
            comp: BTreeMap::new(),
            named: HashMap::new(),
            named_styles: HashMap::new(),
            char_named: HashMap::new(),
            doc_inh: Style::default(),
            frames: Vec::new(),
            toc_started: None,
            content_width: 451.0,
            sects: Vec::new(),
            sect_index: 0,
            cur_blocks: Vec::new(),
            done_sects: Vec::new(),
            last_table: None,
            hf_cache: HashMap::new(),
            even_odd,
            in_table: 0,
            tbl_base: None,
            stats: Vec::new(),
        })
    }

    fn id(&mut self, prefix: &'static str) -> String {
        let n = self.counters.entry(prefix).or_insert(0);
        *n += 1;
        format!("{prefix}{n}")
    }

    fn add(&mut self, id: &str, el: Value) {
        self.elements.insert(id.to_string(), el);
    }

    fn run(&mut self, main: &str) -> Result<Value> {
        let text = String::from_utf8_lossy(&self.files[main]).into_owned();
        let doc = Document::parse(&text).with_context(|| format!("{main} 不是合法的 XML"))?;
        let body = child(doc.root_element(), "body").context("document.xml 里没有 w:body")?;
        self.prepass(body);
        self.base_styles();
        // 先把所有节的设置读出来，处理内容时要知道版心宽度
        let sect_nodes: Vec<Node> = body
            .descendants()
            .filter(|n| isw(*n, "sectPr"))
            .filter(|n| {
                n.parent()
                    .is_some_and(|p| isw(p, "body") || (isw(p, "pPr")))
            })
            .collect();
        self.sects = sect_nodes.iter().map(|n| self.sect(*n)).collect();
        if self.sects.is_empty() {
            self.sects.push(Sect {
                w: 595.3,
                h: 841.9,
                margins: [72.0, 90.0, 72.0, 90.0],
                header_d: 36.0,
                footer_d: 36.0,
                ..Sect::default()
            });
        }
        self.content_width = self.sects[0].content_width();
        let mut blocks = Vec::new();
        self.blocks(body, &mut blocks, true);
        self.cur_blocks.extend(blocks);
        // 最后一节：body 下面的 sectPr
        let last = self
            .sects
            .get(self.sect_index)
            .cloned()
            .unwrap_or_else(|| self.sects.last().cloned().unwrap());
        let rest = std::mem::take(&mut self.cur_blocks);
        self.done_sects.push((rest, last));
        self.finish(&doc)
    }

    // ---------- 节 ----------

    fn sect(&mut self, n: Node) -> Sect {
        let f = |e: Option<Node>, a: &str| e.and_then(|x| wa(x, a)).and_then(|v| v.parse::<f32>().ok());
        let pg = child(n, "pgSz");
        let mar = child(n, "pgMar");
        let mut s = Sect {
            w: f(pg, "w").unwrap_or(11906.0) / 20.0,
            h: f(pg, "h").unwrap_or(16838.0) / 20.0,
            landscape: pg.and_then(|p| wa(p, "orient")) == Some("landscape"),
            margins: [
                f(mar, "top").unwrap_or(1440.0).abs() / 20.0,
                f(mar, "right").unwrap_or(1800.0) / 20.0,
                f(mar, "bottom").unwrap_or(1440.0).abs() / 20.0,
                f(mar, "left").unwrap_or(1800.0) / 20.0,
            ],
            header_d: f(mar, "header").unwrap_or(720.0) / 20.0,
            footer_d: f(mar, "footer").unwrap_or(720.0) / 20.0,
            gutter: f(mar, "gutter").unwrap_or(0.0) / 20.0,
            title_pg: child(n, "titlePg").is_some_and(word::on_off),
            ..Sect::default()
        };
        if let Some(c) = child(n, "cols") {
            let count = f(Some(c), "num").unwrap_or(1.0) as u8;
            if count > 1 {
                s.cols = Some((count, f(Some(c), "space").unwrap_or(425.0) / 20.0));
            }
            if c.children().any(|x| isw(x, "col")) {
                self.warns.once(
                    "cols",
                    "分栏里各栏宽度不同，只保留了栏数和栏距".into(),
                );
            }
        }
        if let Some(p) = child(n, "pgNumType") {
            s.pg_start = f(Some(p), "start").map(|x| x as u32);
            if let Some(fmt) = wa(p, "fmt").filter(|x| *x != "decimal") {
                self.warns.once(
                    "pgfmt",
                    format!("页码格式 `{fmt}` 没保留，按阿拉伯数字处理"),
                );
            }
        }
        if let Some(t) = child(n, "type").and_then(wval)
            && t != "nextPage"
        {
            self.warns.once(
                "secttype",
                format!("分节类型 `{t}` 没保留，按下一页分节处理"),
            );
        }
        if child(n, "vAlign").and_then(wval).is_some_and(|v| v != "top") {
            self.warns
                .once("vAlign", "页面的垂直对齐没保留".into());
        }
        for r in n.children() {
            let list = if isw(r, "headerReference") {
                &mut s.headers
            } else if isw(r, "footerReference") {
                &mut s.footers
            } else {
                continue;
            };
            if let (Some(t), Some(id)) = (
                wa(r, "type").or(Some("default")),
                r.attribute((R, "id")),
            ) {
                list.push((t.to_string(), id.to_string()));
            }
        }
        s
    }

    /// 段落带的 sectPr：这一节到这里结束。
    fn end_section(&mut self, out: &mut Vec<String>) {
        self.cur_blocks.append(out);
        let blocks = std::mem::take(&mut self.cur_blocks);
        let s = self.sects[self.sect_index.min(self.sects.len() - 1)].clone();
        self.done_sects.push((blocks, s));
        self.sect_index += 1;
        if let Some(n) = self.sects.get(self.sect_index) {
            self.content_width = n.content_width();
        }
    }

    // ---------- 块 ----------

    /// 处理一串块级内容（body、页眉页脚、sdtContent），元素 id 追加到 `out`。
    fn blocks(&mut self, parent: Node, out: &mut Vec<String>, body: bool) {
        let mut list: Option<(ListRef, Vec<Para>)> = None;
        let kids: Vec<Node> = parent.children().filter(|c| c.is_element()).collect();
        for n in kids {
            if isw(n, "p") {
                let para = self.para(n);
                if para.list.is_some() && !para.in_toc && para.toc.is_none() {
                    let lr = para.list.clone().unwrap();
                    match &mut list {
                        Some((cur, items)) if cur.abs == lr.abs && cur.num_id == lr.num_id => {
                            items.push(para.clone())
                        }
                        _ => {
                            if let Some(l) = list.take() {
                                out.push(self.list(l.0, l.1));
                            }
                            list = Some((lr, vec![para.clone()]));
                        }
                    }
                    self.last_table = None;
                    if let Some(s) = para.sect.clone() {
                        let _ = s;
                        if let Some(l) = list.take() {
                            out.push(self.list(l.0, l.1));
                        }
                        if body {
                            self.end_section(out);
                        }
                    }
                    continue;
                }
                if let Some(l) = list.take() {
                    out.push(self.list(l.0, l.1));
                }
                let has_sect = para.sect.is_some();
                self.paragraph(para, out);
                if has_sect && body {
                    self.end_section(out);
                }
            } else if isw(n, "tbl") {
                if let Some(l) = list.take() {
                    out.push(self.list(l.0, l.1));
                }
                if self.in_toc() {
                    continue;
                }
                let id = self.table(n);
                out.push(id.clone());
                self.last_table = Some(id);
            } else if isw(n, "sdt") {
                if let Some(l) = list.take() {
                    out.push(self.list(l.0, l.1));
                }
                if let Some(c) = child(n, "sdtContent") {
                    self.blocks(c, out, body);
                }
            } else if isw(n, "customXml") || isw(n, "ins") || isw(n, "moveTo") {
                if let Some(l) = list.take() {
                    out.push(self.list(l.0, l.1));
                }
                self.blocks(n, out, body);
            } else if isw(n, "altChunk") {
                self.warns
                    .once("altChunk", "嵌入的外部文档（altChunk）没解析".into());
            }
        }
        if let Some(l) = list.take() {
            out.push(self.list(l.0, l.1));
        }
    }

    fn in_toc(&self) -> bool {
        self.frames
            .iter()
            .any(|f| matches!(f.kind, Some(FieldKind::Toc(_))))
    }

    /// 读一段：算好段落属性、文字基准，收集内容。
    fn para(&mut self, p: Node) -> Para {
        let in_toc_start = self.in_toc();
        let direct = child(p, "pPr")
            .map(|x| PPr::parse(x, &self.theme, &mut self.warns))
            .unwrap_or_default();
        let pstyle = direct
            .style
            .clone()
            .or_else(|| self.styles.default_para.clone());
        let (sp, sr) = pstyle
            .as_deref()
            .map(|s| self.styles.para(s))
            .unwrap_or_default();
        let mut ppr = self.styles.default_ppr.clone();
        let mut run_base = self.styles.default_rpr.clone();
        if let Some((tp, tr)) = &self.tbl_base {
            ppr.merge(tp);
            run_base.merge(tr);
        }
        ppr.merge(&sp);
        run_base.merge(&sr);
        let mut probe = ppr.clone();
        probe.merge(&direct);
        let mut list = None;
        if let Some(num_id) = probe.num_id.filter(|n| *n > 0) {
            match self.numbering.levels(num_id, &self.styles) {
                Some((abs, levels)) => {
                    let ilvl = probe.ilvl.unwrap_or(0).min(8);
                    let mut lp = PPr::default();
                    lp.ind = levels[ilvl].ind;
                    ppr.merge(&lp);
                    list = Some(ListRef {
                        num_id,
                        abs,
                        ilvl,
                        levels,
                    });
                }
                None => self.warns.once(
                    &format!("num:{num_id}"),
                    format!("编号 {num_id} 在 numbering.xml 里找不到，按普通段落处理"),
                ),
            }
        }
        ppr.merge(&direct);
        let sect = child(p, "pPr")
            .and_then(|x| child(x, "sectPr"))
            .map(|_| Sect::default());
        let mut segs = Vec::new();
        self.toc_started = None;
        self.inline(p, &run_base, None, &mut segs);
        let toc = self.toc_started.take();
        let in_toc = in_toc_start || self.in_toc() || toc.is_some();
        Para {
            ppr,
            run_base,
            pstyle,
            segs,
            list,
            sect,
            toc,
            in_toc,
            direct_ind: !direct.ind.is_empty(),
        }
    }

    /// 段落里能直接出元素的部分：普通段落、标题、图片、公式、分页、目录。
    fn paragraph(&mut self, para: Para, out: &mut Vec<String>) {
        if let Some(levels) = para.toc {
            let mut props = Map::new();
            if levels != 3 {
                props.insert("levels".into(), json!(levels));
            }
            let id = self.id("toc");
            self.add(&id, json!({ "type": "TableOfContents", "props": props }));
            out.push(id);
            self.last_table = None;
            return;
        }
        if para.in_toc {
            return;
        }
        let empty = para.segs.is_empty();
        // 表格后面那个 1pt 高的空段落：渲染器用它隔开两张表，当表格的段后间距
        if empty
            && let Some(t) = self.last_table.take()
            && para.ppr.line_rule.as_deref() == Some("exact")
            && para.ppr.line.is_some_and(|l| l <= 40.0)
        {
            let after = para.ppr.after.unwrap_or(0.0) / 20.0;
            if let Some(Value::Object(el)) = self.elements.get_mut(&t) {
                let st = el
                    .entry("style")
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Value::Object(m) = st {
                    let builtin_after = builtin("Table")
                        .and_then(|b| b.space_after)
                        .map(|l| l.abs_pt())
                        .unwrap_or(0.0);
                    if (after - builtin_after).abs() < 0.01 {
                        m.remove("spaceAfter");
                    } else {
                        m.insert("spaceAfter".into(), json!(round2(after)));
                    }
                }
            }
            return;
        }
        self.last_table = None;
        // 分节的那个空段落不出元素（Word 的节末段落）
        if empty && para.sect.is_some() {
            return;
        }
        // 按分页拆开
        let has_break = para.segs.iter().any(|s| matches!(s, Seg::PageBreak));
        let mut chunks: Vec<Vec<Seg>> = vec![Vec::new()];
        for s in &para.segs {
            if matches!(s, Seg::PageBreak) {
                chunks.push(Vec::new());
            } else {
                chunks.last_mut().unwrap().push(s.clone());
            }
        }
        for (i, chunk) in chunks.into_iter().enumerate() {
            if i > 0 {
                let id = self.id("pagebreak");
                self.add(&id, json!({ "type": "PageBreak" }));
                out.push(id);
            }
            if chunk.is_empty() && has_break {
                continue;
            }
            let id = self.block_of(&para, chunk);
            out.push(id);
        }
    }

    /// 一段内容出一个元素。
    fn block_of(&mut self, para: &Para, segs: Vec<Seg>) -> String {
        let visible_text = |s: &Seg| match s {
            Seg::Text { text, .. } => !text.trim().is_empty(),
            Seg::Field { .. } => true,
            _ => false,
        };
        let images: Vec<&Img> = segs
            .iter()
            .filter_map(|s| if let Seg::Image(i) = s { Some(i) } else { None })
            .collect();
        let maths: Vec<&Seg> = segs
            .iter()
            .filter(|s| matches!(s, Seg::Math { .. }))
            .collect();
        let texts = segs.iter().any(visible_text);
        if images.len() == 1 && maths.is_empty() && !texts {
            let img = images[0].clone();
            return self.image_block(para, &img);
        }
        if maths.len() == 1 && images.is_empty() {
            let Seg::Math {
                latex,
                display,
                jc,
                style,
            } = maths[0].clone()
            else {
                unreachable!()
            };
            if !texts && display {
                return self.math_block(para, &latex, None, jc.as_deref(), &style);
            }
            // 带编号的块级公式：制表符、公式、制表符、(1)
            let mut before = String::new();
            let mut after = String::new();
            let mut seen = false;
            for s in &segs {
                match s {
                    Seg::Math { .. } => seen = true,
                    Seg::Text { text, .. } if seen => after.push_str(text),
                    Seg::Text { text, .. } => before.push_str(text),
                    _ => {}
                }
            }
            let a = after.trim();
            if before.trim().is_empty()
                && before.contains('\t')
                && after.contains('\t')
                && a.starts_with('(')
                && a.ends_with(')')
            {
                let no = a[1..a.len() - 1].trim().to_string();
                let number = no
                    .parse::<u32>()
                    .map(|x| json!(x))
                    .unwrap_or_else(|_| json!(no));
                return self.math_block(para, &latex, Some(number), Some("center"), &style);
            }
            if !texts && !display {
                // 段落里只有一个行内公式，也当块级公式
                let jc = para.ppr.jc.clone();
                return self.math_block(para, &latex, None, jc.as_deref(), &style);
            }
        }
        let level = para
            .pstyle
            .as_deref()
            .and_then(|s| self.styles.heading_level(s));
        if para
            .ppr
            .tabs
            .as_ref()
            .is_some_and(|t| t.iter().any(|(k, _)| k != "clear"))
            && segs
                .iter()
                .any(|s| matches!(s, Seg::Text { text, .. } if text.contains('\t')))
        {
            self.warns.once(
                "tabs",
                "段落里的制表位没保留（fmtagno 还没有 tabStops），制表符照原样留在文字里".into(),
            );
        }
        match level {
            Some(l) => self.text_block("Heading", Some(l), para, &segs),
            None => self.text_block("Text", None, para, &segs),
        }
    }

    /// 「不写样式时」元素的样式：Document 往下传的、内置组件样式、我们写的组件样式、命名样式。
    fn expected(&self, kind: &str, level: Option<u8>, named: Option<&str>) -> Style {
        let mut s = self.doc_inh.clone();
        let mut keys = vec![kind.to_string()];
        if let Some(l) = level {
            keys.push(format!("{kind}.{l}"));
        }
        for k in &keys {
            if let Some(b) = builtin(k) {
                s.merge(b);
            }
        }
        for k in &keys {
            if let Some(c) = self.comp.get(k) {
                s.merge(c);
            }
        }
        if let Some(n) = named.and_then(|n| self.named_styles.get(n)) {
            s.merge(n);
        }
        s
    }

    /// 先把正文段落（不含表格里的）读一遍，记下各类段落的有效样式。
    fn prepass(&mut self, body: Node) {
        let warns = std::mem::take(&mut self.warns);
        let frames = std::mem::take(&mut self.frames);
        let paras: Vec<Node> = body
            .descendants()
            .filter(|n| isw(*n, "p"))
            .filter(|n| !n.ancestors().skip(1).any(|a| isw(a, "tbl") || isw(a, "txbxContent")))
            .collect();
        let mut stats = Vec::new();
        for p in paras {
            let para = self.para(p);
            if para.in_toc || para.toc.is_some() {
                continue;
            }
            let visible = para.segs.iter().any(|s| match s {
                Seg::Text { text, .. } => !text.trim().is_empty(),
                Seg::Field { .. } => true,
                _ => false,
            });
            if !visible {
                continue;
            }
            let kind = if para.list.is_some() {
                Kind::List
            } else if let Some(l) = para.pstyle.as_deref().and_then(|s| self.styles.heading_level(s)) {
                Kind::Heading(l)
            } else {
                Kind::Text
            };
            let (eff, _) = self.para_eff(&para, &para.segs);
            stats.push((kind, eff));
        }
        self.stats = stats;
        self.warns = warns;
        self.frames = frames;
        self.toc_started = None;
    }

    /// Document、Text 的组件样式：取 Word 的 docDefaults 和默认段落样式（Normal），
    /// 再按多数正文段落实际用的值改（fmtagno 生成的 docx 每段都写了直接格式）。
    fn base_styles(&mut self) {
        let (np, nr) = self
            .styles
            .default_para
            .clone()
            .map(|s| self.styles.para(&s))
            .unwrap_or_default();
        let mut ppr = self.styles.default_ppr.clone();
        ppr.merge(&np);
        let rpr = merged(&self.styles.default_rpr, &nr);
        let fs = rpr.font_size.map(|l| l.abs_pt()).unwrap_or(10.0);
        let mut full = ppr.to_style(fs);
        full.merge(&text_of(&rpr));
        let mut eff = to_map(&full.inherited());
        {
            let body: Vec<&Map<String, Value>> = self
                .stats
                .iter()
                .filter(|(k, _)| matches!(k, Kind::Text | Kind::List))
                .map(|(_, m)| m)
                .collect();
            for k in ["fontFamily", "fontSize", "color", "lineHeight"] {
                if let Some(v) = majority(&body, k) {
                    eff.insert(k.to_string(), v);
                }
            }
        }
        if !eff.contains_key("fontSize") {
            eff.insert("fontSize".into(), json!(10));
        }
        if !eff.contains_key("fontFamily") {
            let latin = self
                .theme
                .minor_latin
                .clone()
                .unwrap_or_else(|| "Times New Roman".into());
            let mut f = json!({ "latin": latin });
            if let Some(ea) = &self.theme.minor_ea {
                f["eastAsia"] = json!(ea);
            }
            eff.insert("fontFamily".into(), f);
        }
        let base = to_map(&builtin("$global").cloned().unwrap_or_default());
        let keys: Vec<&str> = TEXT_KEYS
            .iter()
            .chain(PARA_KEYS.iter())
            .copied()
            .collect();
        let mut doc = diff(&eff, &base, &keys, &[]);
        // Document 只放会继承的字段
        doc.retain(|k, _| {
            !matches!(
                k.as_str(),
                "spaceBefore" | "spaceAfter" | "marginLeft" | "marginRight" | "keepWithNext"
                    | "keepTogether" | "pageBreakBefore" | "outlineLevel" | "background"
            ) && !k.starts_with("border")
                && !k.starts_with("padding")
        });
        let doc_style = from_map(&doc);
        let mut root = builtin("$global").cloned().unwrap_or_default();
        root.merge(&doc_style);
        self.doc_inh = root.inherited();
        if !doc.is_empty() {
            let mut d = doc.clone();
            compact(&mut d);
            self.out_styles.insert("Document".into(), Value::Object(d));
            self.comp.insert("Document".into(), doc_style);
        }
        // Text：Normal 的段落间距这些，按多数正文段落改
        let mut local = to_map(&full);
        {
            let texts: Vec<&Map<String, Value>> = self
                .stats
                .iter()
                .filter(|(k, _)| *k == Kind::Text)
                .map(|(_, m)| m)
                .collect();
            apply_majority(&mut local, &texts, &["spaceBefore", "spaceAfter", "textIndent", "keepWithNext"]);
        }
        let exp = to_map(&self.expected("Text", None, None));
        let tkeys = [
            "spaceBefore",
            "spaceAfter",
            "keepWithNext",
            "keepTogether",
            "marginLeft",
            "marginRight",
            "textIndent",
            "hangingIndent",
        ];
        let t = diff(&local, &exp, &tkeys, &[]);
        if !t.is_empty() {
            self.comp.insert("Text".into(), from_map(&t));
            self.out_styles.insert("Text".into(), Value::Object(t));
        }
    }

    /// Word 段落样式 → fmtagno 命名样式（只给正文段落用）。
    fn named_style(&mut self, id: &str) -> Option<String> {
        if Some(id) == self.styles.default_para.as_deref() {
            return None;
        }
        if let Some(x) = self.named.get(id) {
            return x.clone();
        }
        let (sp, sr) = self.styles.para(id);
        let mut ppr = self.styles.default_ppr.clone();
        ppr.merge(&sp);
        let rpr = merged(&self.styles.default_rpr, &sr);
        let fs = rpr.font_size.map(|l| l.abs_pt()).unwrap_or(10.0);
        let mut full = ppr.to_style(fs);
        full.merge(&text_of(&rpr));
        let eff = to_map(&full);
        let exp = to_map(&self.expected("Text", None, None));
        let keys: Vec<&str> = PARA_KEYS.iter().chain(TEXT_KEYS.iter()).copied().collect();
        let mut d = diff(&eff, &exp, &keys, &[]);
        d.remove("outlineLevel");
        let out = if d.is_empty() {
            None
        } else {
            let name = self.style_key(id);
            self.named_styles.insert(name.clone(), from_map(&d));
            compact(&mut d);
            self.out_styles.insert(name.clone(), Value::Object(d));
            Some(name)
        };
        self.named.insert(id.to_string(), out.clone());
        out
    }

    /// Word 字符样式 → 命名样式（只有文字字段）。
    fn char_style(&mut self, id: &str) -> Option<String> {
        if let Some(x) = self.char_named.get(id) {
            return x.clone();
        }
        let r = self.styles.chars(id);
        let mut m = to_map(&r.text_only());
        m.retain(|k, _| TEXT_KEYS.contains(&k.as_str()) || k == "background");
        let out = if m.is_empty() {
            None
        } else {
            let name = self.style_key(id);
            self.named_styles.insert(name.clone(), from_map(&m));
            compact(&mut m);
            self.out_styles.insert(name.clone(), Value::Object(m));
            Some(name)
        };
        self.char_named.insert(id.to_string(), out.clone());
        out
    }

    /// 命名样式的键：Word 样式名，避开组件名和已有的键。
    fn style_key(&self, id: &str) -> String {
        let name = self
            .styles
            .map
            .get(id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| id.to_string())
            .replace(['.', ':'], "-");
        let head = name.split(['.', ':']).next().unwrap_or("");
        let clash = crate::components::BUILTINS.iter().any(|s| s.name == head)
            || crate::components::STYLE_ONLY.contains(&head);
        let mut key = if clash { format!("Word {name}") } else { name };
        if self.out_styles.contains_key(&key) {
            key = format!("{key} ({id})");
        }
        key
    }

    /// 标题的组件样式 Heading.N，按这一级第一次用到的 Word 样式算。
    fn heading_style(&mut self, level: u8, pstyle: Option<&str>) {
        let key = format!("Heading.{level}");
        if self.comp.contains_key(&key) || self.out_styles.contains_key(&key) {
            return;
        }
        let (sp, sr) = pstyle.map(|s| self.styles.para(s)).unwrap_or_default();
        let mut ppr = self.styles.default_ppr.clone();
        ppr.merge(&sp);
        let rpr = merged(&self.styles.default_rpr, &sr);
        let fs = rpr.font_size.map(|l| l.abs_pt()).unwrap_or(10.0);
        let mut full = ppr.to_style(fs);
        full.merge(&text_of(&rpr));
        let mut eff = to_map(&full);
        {
            let hs: Vec<&Map<String, Value>> = self
                .stats
                .iter()
                .filter(|(k, _)| *k == Kind::Heading(level))
                .map(|(_, m)| m)
                .collect();
            let keys: Vec<&str> = PARA_KEYS.iter().chain(TEXT_KEYS.iter()).copied().collect();
            apply_majority(&mut eff, &hs, &keys);
        }
        let exp = to_map(&self.expected("Heading", Some(level), None));
        let keys: Vec<&str> = PARA_KEYS.iter().chain(TEXT_KEYS.iter()).copied().collect();
        let mut d = diff(&eff, &exp, &keys, &[]);
        d.remove("outlineLevel");
        // 内置 Heading.N 写了字号，这里一定要写，不然会被盖掉
        if let Some(v) = eff.get("fontSize") {
            d.insert("fontSize".into(), v.clone());
        }
        self.comp.insert(key.clone(), from_map(&d));
        compact(&mut d);
        self.out_styles.insert(key, Value::Object(d));
    }

    /// 一个段落的有效样式（段落字段 + 各 run 共同的文字字段），和共同字段的键。
    fn para_eff(&self, para: &Para, segs: &[Seg]) -> (Map<String, Value>, Vec<String>) {
        let runs: Vec<Map<String, Value>> = segs
            .iter()
            .filter_map(|s| match s {
                Seg::Text { text, style, .. } if !text.is_empty() => Some(to_map(&text_of(style))),
                Seg::Field { style, .. } => Some(to_map(&text_of(style))),
                _ => None,
            })
            .collect();
        let hoisted = if runs.is_empty() {
            let mut base = para.run_base.clone();
            if let Some(m) = &para.ppr.mark {
                base.merge(m);
            }
            to_map(&text_of(&base))
        } else {
            hoist(&runs, TEXT_KEYS)
        };
        let fs = size_of(&hoisted)
            .or_else(|| para.run_base.font_size.map(|l| l.abs_pt()))
            .unwrap_or(10.0);
        let mut eff = to_map(&para.ppr.to_style(fs));
        for (k, v) in &hoisted {
            eff.insert(k.clone(), v.clone());
        }
        (eff, hoisted.keys().cloned().collect())
    }

    /// 标题、正文。
    fn text_block(&mut self, kind: &'static str, level: Option<u8>, para: &Para, segs: &[Seg]) -> String {
        if let Some(l) = level {
            self.heading_style(l, para.pstyle.as_deref());
        }
        let named = if kind == "Text" {
            para.pstyle.clone().and_then(|s| self.named_style(&s))
        } else {
            None
        };
        let exp = self.expected(kind, level, named.as_deref());
        let (eff, hoisted) = self.para_eff(para, segs);
        let soft: Vec<&str> = TEXT_KEYS
            .iter()
            .copied()
            .filter(|k| !hoisted.iter().any(|h| h == k))
            .collect();
        let keys: Vec<&str> = PARA_KEYS.iter().chain(TEXT_KEYS.iter()).copied().collect();
        let mut d = diff(&eff, &to_map(&exp), &keys, &soft);
        if kind == "Heading" {
            d.remove("outlineLevel");
        }
        let computed = merged(&exp, &from_map(&d));
        let text = self.inline_json(segs, &computed);
        let id = self.id(if kind == "Heading" { "h" } else { "p" });
        let mut el = Map::new();
        el.insert("type".into(), json!(kind));
        let mut props = Map::new();
        if let Some(l) = level {
            props.insert("level".into(), json!(l));
        }
        props.insert("text".into(), text);
        el.insert("props".into(), Value::Object(props));
        compact(&mut d);
        if let Some(st) = style_value(named, d) {
            el.insert("style".into(), st);
        }
        self.add(&id, Value::Object(el));
        id
    }

    /// 段落里的 runs 换成 Inline 的 JSON：纯文字写字符串，带样式写对象。
    fn inline_json(&mut self, segs: &[Seg], elem: &Style) -> Value {
        let base = text_of(elem);
        let mut runs: Vec<Value> = Vec::new();
        for s in segs {
            let v = match s {
                Seg::Text {
                    text,
                    style,
                    href,
                    code,
                    cstyle,
                } => {
                    if text.is_empty() {
                        continue;
                    }
                    let named = cstyle.as_deref().and_then(|c| self.char_style(c));
                    let mut exp = base.clone();
                    if *code {
                        if let Some(b) = builtin("InlineCode") {
                            exp.merge(&text_of(b));
                        }
                    }
                    if let Some(n) = named.as_deref().and_then(|n| self.named_styles.get(n)) {
                        exp.merge(&n.text_only());
                    }
                    let eff = to_map(&text_of(style));
                    let mut d = diff(&eff, &to_map(&exp), TEXT_KEYS, &[]);
                    if let Some(bg) = &style.background {
                        let bgv = serde_json::to_value(bg).unwrap_or(Value::Null);
                        let same_named = named
                            .as_deref()
                            .and_then(|n| self.named_styles.get(n))
                            .and_then(|n| n.background.as_ref())
                            == Some(bg);
                        if !same_named && !(*code && is_code_bg(bg)) {
                            d.insert("background".into(), bgv);
                        }
                    }
                    run_value(text, d, href.as_deref(), *code, named, &eff)
                }
                Seg::Field { field, text, style } => {
                    let eff = to_map(&text_of(style));
                    let d = diff(&eff, &to_map(&base), TEXT_KEYS, &[]);
                    let mut v = run_value(text, d, None, false, None, &eff);
                    let f = serde_json::to_value(field).unwrap_or(Value::Null);
                    match &mut v {
                        Value::Object(m) => {
                            m.insert("field".into(), f);
                        }
                        _ => {
                            v = json!({ "text": text, "field": f });
                        }
                    }
                    v
                }
                Seg::Image(img) => {
                    if img.anchor.is_some() {
                        self.warns.once(
                            "anchor-inline",
                            "和文字混排的浮动图片改成了行内图片".into(),
                        );
                    }
                    json!({ "image": img.src, "width": round2(img.w), "height": round2(img.h) })
                }
                Seg::Math { latex, style, .. } => {
                    let eff = to_map(&text_of(style));
                    let mut d = diff(&eff, &to_map(&base), TEXT_KEYS, &["fontFamily"]);
                    d.remove("fontFamily");
                    let mut v = json!({ "type": "InlineMath", "text": latex });
                    if !d.is_empty() {
                        v["style"] = Value::Object(d);
                    }
                    v
                }
                Seg::PageBreak => continue,
            };
            // 和上一段样式一样就拼起来
            if let (Some(prev), Some(cur)) = (runs.last_mut(), Some(&v))
                && let Some(joined) = join_runs(prev, cur)
            {
                *prev = joined;
                continue;
            }
            runs.push(v);
        }
        match runs.len() {
            0 => json!(""),
            1 if runs[0].is_string() => runs.pop().unwrap(),
            _ => Value::Array(runs),
        }
    }

    // ---------- 段落内容 ----------

    fn gate(&self) -> Gate {
        let mut href = None;
        for (i, f) in self.frames.iter().enumerate().rev() {
            if !f.result {
                return Gate::Drop;
            }
            match &f.kind {
                Some(FieldKind::Toc(_)) => return Gate::Drop,
                Some(FieldKind::Page(_)) => {
                    if href.is_none() {
                        return Gate::Capture(i);
                    }
                }
                Some(FieldKind::Link(u)) => {
                    if href.is_none() {
                        href = Some(u.clone());
                    }
                }
                _ => {}
            }
        }
        Gate::Emit(href)
    }

    fn push_text(&mut self, out: &mut Vec<Seg>, text: &str, style: &Style, href: &Option<String>, code: bool, cstyle: &Option<String>) {
        match self.gate() {
            Gate::Drop => {}
            Gate::Capture(i) => {
                let f = &mut self.frames[i];
                f.text.push_str(text);
                if f.style.is_none() {
                    f.style = Some(style.clone());
                }
            }
            Gate::Emit(fh) => {
                let href = href.clone().or(fh);
                if let Some(Seg::Text {
                    text: t,
                    style: s,
                    href: h,
                    code: c,
                    cstyle: cs,
                }) = out.last_mut()
                    && s == style
                    && *h == href
                    && *c == code
                    && cs == cstyle
                {
                    t.push_str(text);
                    return;
                }
                out.push(Seg::Text {
                    text: text.to_string(),
                    style: style.clone(),
                    href,
                    code,
                    cstyle: cstyle.clone(),
                });
            }
        }
    }

    fn push_seg(&mut self, out: &mut Vec<Seg>, s: Seg) {
        if let Gate::Emit(_) = self.gate() {
            out.push(s);
        }
    }

    /// 段落、超链接这类容器里的内容。
    fn inline(&mut self, n: Node, base: &Style, href: Option<String>, out: &mut Vec<Seg>) {
        for c in n.children().filter(|c| c.is_element()) {
            let ns = c.tag_name().namespace();
            let name = c.tag_name().name();
            if ns == Some(M) {
                match name {
                    "oMath" | "oMathPara" => self.math(c, base, out),
                    _ => {}
                }
                continue;
            }
            if ns == Some("http://schemas.openxmlformats.org/markup-compatibility/2006")
                && name == "AlternateContent"
            {
                self.alternate(c, base, &href, out);
                continue;
            }
            if ns != Some(W) {
                continue;
            }
            match name {
                "r" => self.wrun(c, base, &href, out),
                "hyperlink" => {
                    let link = match c.attribute((R, "id")) {
                        Some(id) => self.rels.get(id).map(|r| r.target.clone()),
                        None => {
                            if wa(c, "anchor").is_some() {
                                self.warns.once(
                                    "anchor-link",
                                    "文档内部的链接（书签）没保留，只留了文字".into(),
                                );
                            }
                            None
                        }
                    };
                    self.inline(c, base, link.or(href.clone()), out);
                }
                "fldSimple" => {
                    let kind = classify_field(wa(c, "instr").unwrap_or(""));
                    match kind {
                        FieldKind::Page(f) => {
                            let mut tmp = Vec::new();
                            self.inline(c, base, None, &mut tmp);
                            let (text, style) = first_text(&tmp, base);
                            self.push_seg(out, Seg::Field { field: f, text, style });
                        }
                        FieldKind::Link(u) => self.inline(c, base, Some(u), out),
                        FieldKind::Toc(l) => {
                            self.toc_started = Some(l);
                        }
                        FieldKind::Other(o) => {
                            self.field_warn(&o);
                            self.inline(c, base, href.clone(), out);
                        }
                    }
                }
                "sdt" => {
                    if let Some(sc) = child(c, "sdtContent") {
                        self.inline(sc, base, href.clone(), out);
                    }
                }
                "ins" | "smartTag" | "customXml" | "dir" | "bdo" | "moveTo" => {
                    self.inline(c, base, href.clone(), out)
                }
                "del" | "moveFrom" => {}
                _ => {}
            }
        }
    }

    fn field_warn(&mut self, kind: &str) {
        if kind.is_empty() {
            return;
        }
        self.warns.once(
            &format!("field:{kind}"),
            format!("域 `{kind}` 没保留，只留了它显示的文字"),
        );
    }

    /// mc:AlternateContent：有图片就用 Choice 里的，其他（文本框、形状）不解析。
    fn alternate(&mut self, n: Node, base: &Style, href: &Option<String>, out: &mut Vec<Seg>) {
        const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
        let choice = n.children().find(|c| is(*c, MC, "Choice"));
        if let Some(ch) = choice {
            let has_pic = ch
                .descendants()
                .any(|d| d.tag_name().name() == "pic" && d.tag_name().namespace().is_some_and(|x| x.ends_with("/picture")));
            if has_pic {
                for d in ch.children().filter(|x| isw(*x, "drawing")) {
                    self.drawing(d, out);
                }
                return;
            }
            if ch.descendants().any(|d| d.tag_name().name() == "txbx") {
                self.warns.once(
                    "textbox",
                    "文本框没解析，里面的文字丢了".into(),
                );
            } else {
                self.warns
                    .once("shape", "形状、艺术字这类图形没解析".into());
            }
        }
        let _ = (base, href);
    }

    fn wrun(&mut self, r: Node, base: &Style, href: &Option<String>, out: &mut Vec<Seg>) {
        let rpr = child(r, "rPr");
        let cstyle = rpr
            .and_then(|p| child(p, "rStyle"))
            .and_then(wval)
            .map(str::to_string);
        let mut style = base.clone();
        if let Some(cs) = &cstyle {
            style.merge(&self.styles.chars(cs));
        }
        if let Some(p) = rpr {
            style.merge(&word::rpr(p, &self.theme, &mut self.warns));
        }
        let code = cstyle.as_deref().is_some_and(|c| {
            c == "InlineCode"
                || self
                    .styles
                    .map
                    .get(c)
                    .is_some_and(|s| s.name == "InlineCode")
        });
        let cstyle = cstyle.filter(|_| !code);
        for c in r.children().filter(|c| c.is_element()) {
            if c.tag_name().namespace() == Some("http://schemas.openxmlformats.org/markup-compatibility/2006")
                && c.tag_name().name() == "AlternateContent"
            {
                self.alternate(c, base, href, out);
                continue;
            }
            if c.tag_name().namespace() != Some(W) {
                continue;
            }
            match c.tag_name().name() {
                "t" => {
                    let t = c.text().unwrap_or("");
                    if !t.is_empty() {
                        self.push_text(out, t, &style, href, code, &cstyle);
                    }
                }
                "tab" | "ptab" => self.push_text(out, "\t", &style, href, code, &cstyle),
                "br" | "cr" => match wa(c, "type") {
                    Some("page") => self.push_seg(out, Seg::PageBreak),
                    Some("column") => {
                        self.warns.once(
                            "colbreak",
                            "分栏符没保留，改成了换行".into(),
                        );
                        self.push_text(out, "\n", &style, href, code, &cstyle);
                    }
                    _ => self.push_text(out, "\n", &style, href, code, &cstyle),
                },
                "noBreakHyphen" => self.push_text(out, "\u{2011}", &style, href, code, &cstyle),
                "softHyphen" => self.push_text(out, "\u{AD}", &style, href, code, &cstyle),
                "sym" => {
                    let font = wa(c, "font").unwrap_or("");
                    let ch = wa(c, "char")
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        .map(|code| symbol_char(font, code));
                    if let Some(ch) = ch {
                        self.push_text(out, &ch, &style, href, code, &cstyle);
                    }
                }
                "fldChar" => match wa(c, "fldCharType") {
                    Some("begin") => self.frames.push(Frame {
                        instr: String::new(),
                        result: false,
                        kind: None,
                        text: String::new(),
                        style: None,
                    }),
                    Some("separate") => {
                        if let Some(f) = self.frames.last_mut() {
                            f.result = true;
                            let k = classify_field(&f.instr);
                            if let FieldKind::Toc(l) = k {
                                self.toc_started = Some(l);
                            }
                            f.kind = Some(k);
                        }
                    }
                    Some("end") => {
                        if let Some(mut f) = self.frames.pop() {
                            let kind = f.kind.take().unwrap_or_else(|| {
                                let k = classify_field(&f.instr);
                                if let FieldKind::Toc(l) = k {
                                    self.toc_started = Some(l);
                                }
                                k
                            });
                            match kind {
                                FieldKind::Page(field) => {
                                    let st = f.style.unwrap_or_else(|| style.clone());
                                    self.push_seg(
                                        out,
                                        Seg::Field {
                                            field,
                                            text: f.text,
                                            style: st,
                                        },
                                    );
                                }
                                FieldKind::Other(o) => self.field_warn(&o),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                },
                "instrText" => {
                    if let Some(f) = self.frames.last_mut()
                        && !f.result
                    {
                        f.instr.push_str(c.text().unwrap_or(""));
                    }
                }
                "drawing" => self.drawing(c, out),
                "pict" | "object" => self.vml(c, out),
                "footnoteReference" | "endnoteReference" => self.warns.once(
                    "footnote",
                    "脚注、尾注没解析".into(),
                ),
                _ => {}
            }
        }
    }

    fn math(&mut self, n: Node, base: &Style, out: &mut Vec<Seg>) {
        let mut conv = omml::Omml::new();
        let latex = conv.convert(n);
        let display = is(n, M, "oMathPara");
        let jc = if display {
            child_ns(n, M, "oMathParaPr")
                .and_then(|p| child_ns(p, M, "jc"))
                .and_then(|j| j.attribute((M, "val")))
                .map(str::to_string)
        } else {
            None
        };
        // 公式的字号、颜色、粗体取第一个 m:r 的 w:rPr
        let mut style = base.clone();
        if let Some(rpr) = n
            .descendants()
            .find(|d| is(*d, M, "r"))
            .and_then(|r| child(r, "rPr"))
        {
            let mut s = word::rpr(rpr, &self.theme, &mut self.warns);
            s.font_family = None;
            style.merge(&s);
        }
        style.italic = base.italic;
        if !conv.unsupported.is_empty() {
            self.warns.push(format!(
                "公式 `{latex}` 里有 {} 转不成 LaTeX，这部分只留了文字",
                conv.unsupported.join("、")
            ));
        } else if crate::math::to_mathml(&latex, display).is_err() {
            self.warns.push(format!(
                "公式转出来的 LaTeX `{latex}` 可能不完全对，渲染时会给警告"
            ));
        }
        self.push_seg(
            out,
            Seg::Math {
                latex,
                display,
                jc,
                style,
            },
        );
    }

    /// 图片：保存到 assets，返回 JSON 里写的路径。
    fn asset(&mut self, rid: &str) -> Option<String> {
        let rel = self.rels.get(rid)?.clone();
        if rel.external {
            self.warns.once(
                &format!("extimg:{}", rel.target),
                format!("链接的外部图片 `{}` 没下载，写成了网址", rel.target),
            );
            return Some(rel.target);
        }
        if let Some(p) = self.asset_by_target.get(&rel.target) {
            return Some(p.clone());
        }
        let bytes = self.files.get(&rel.target)?.clone();
        let base = rel.target.rsplit('/').next().unwrap_or("image").to_string();
        let mut name = base.clone();
        let mut k = 1;
        while self.assets.iter().any(|(n, _)| *n == name) {
            k += 1;
            name = format!("{k}-{base}");
        }
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg") {
            self.warns.once(
                &format!("imgfmt:{ext}"),
                format!("图片格式 {ext} 渲染时放不进 docx（只收 png、jpeg、gif、webp），文件照样导出了"),
            );
        }
        self.assets.push((name.clone(), bytes));
        let path = format!("{}/{name}", self.opts.assets_dir.trim_end_matches('/'));
        self.asset_by_target.insert(rel.target, path.clone());
        Some(path)
    }

    fn drawing(&mut self, d: Node, out: &mut Vec<Seg>) {
        let Some(box_) = d
            .children()
            .find(|c| is(*c, WP, "inline") || is(*c, WP, "anchor"))
        else {
            return;
        };
        let Some(blip) = box_.descendants().find(|x| is(*x, A, "blip")) else {
            if box_.descendants().any(|x| x.tag_name().name() == "chart") {
                self.warns.once("chart", "图表没解析".into());
            } else {
                self.warns
                    .once("shape", "形状、艺术字这类图形没解析".into());
            }
            return;
        };
        let Some(rid) = blip
            .attribute((R, "embed"))
            .or_else(|| blip.attribute((R, "link")))
        else {
            return;
        };
        let Some(src) = self.asset(rid) else {
            self.warns
                .push(format!("图片 {rid} 在 docx 里找不到，跳过"));
            return;
        };
        let ext = child_ns(box_, WP, "extent");
        let emu = |a: &str| {
            ext.and_then(|e| e.attribute(a))
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0)
                / 12700.0
        };
        let doc_pr = child_ns(box_, WP, "docPr");
        let alt = doc_pr
            .and_then(|p| p.attribute("descr").filter(|s| !s.is_empty()).or(p.attribute("title")))
            .unwrap_or("")
            .to_string();
        let rotation = box_
            .descendants()
            .find(|x| is(*x, A, "xfrm"))
            .and_then(|x| x.attribute("rot"))
            .and_then(|r| r.parse::<f32>().ok())
            .map(|r| round2(r / 60000.0))
            .filter(|r| *r != 0.0);
        let anchor = if is(box_, WP, "anchor") {
            let pos = |name: &str| child_ns(box_, WP, name);
            let rel_of = |n: Option<Node>| {
                n.and_then(|x| x.attribute("relativeFrom"))
                    .unwrap_or("column")
                    .to_string()
            };
            let off = |n: Option<Node>| {
                n.and_then(|x| child_ns(x, WP, "posOffset"))
                    .and_then(|o| o.text())
                    .and_then(|t| t.trim().parse::<f32>().ok())
                    .map(|v| v / 12700.0)
                    .unwrap_or(0.0)
            };
            let ph = pos("positionH");
            let pv = pos("positionV");
            let wrap = box_.children().find_map(|c| {
                if c.tag_name().namespace() != Some(WP) {
                    return None;
                }
                Some(match c.tag_name().name() {
                    "wrapNone" => "none",
                    "wrapSquare" => "square",
                    "wrapTight" | "wrapThrough" => "tight",
                    "wrapTopAndBottom" => "topAndBottom",
                    _ => return None,
                })
            });
            Some(Anchor {
                behind: matches!(box_.attribute("behindDoc"), Some("1" | "true")),
                wrap,
                h_rel: rel_of(ph),
                v_rel: rel_of(pv),
                h_align: ph
                    .and_then(|x| child_ns(x, WP, "align"))
                    .and_then(|a| a.text())
                    .map(str::to_string),
                x: off(ph),
                y: off(pv),
                dist: box_
                    .attribute("distL")
                    .and_then(|v| v.parse::<f32>().ok())
                    .map(|v| v / 12700.0)
                    .unwrap_or(0.0),
            })
        } else {
            None
        };
        self.push_seg(
            out,
            Seg::Image(Img {
                src,
                w: emu("cx"),
                h: emu("cy"),
                alt,
                rotation,
                anchor,
            }),
        );
    }

    /// 老式的 VML 图片（w:pict、w:object）。
    fn vml(&mut self, n: Node, out: &mut Vec<Seg>) {
        let Some(img) = n.descendants().find(|x| is(*x, V, "imagedata")) else {
            if n.descendants().any(|x| x.tag_name().name() == "textbox") {
                self.warns.once(
                    "textbox",
                    "文本框没解析，里面的文字丢了".into(),
                );
            }
            return;
        };
        if isw(n, "object") {
            self.warns.once(
                "ole",
                "嵌入对象（OLE，比如 MathType 公式）只保留了预览图".into(),
            );
        }
        let Some(rid) = img.attribute((R, "id")) else {
            return;
        };
        let Some(src) = self.asset(rid) else { return };
        let style = n
            .descendants()
            .find(|x| is(*x, V, "shape"))
            .and_then(|s| s.attribute("style"))
            .unwrap_or("");
        let dim = |k: &str| {
            style
                .split(';')
                .find_map(|kv| kv.trim().strip_prefix(&format!("{k}:")))
                .and_then(crate::units::parse_len_pt)
                .unwrap_or(0.0)
        };
        let (w, h) = (dim("width"), dim("height"));
        self.push_seg(
            out,
            Seg::Image(Img {
                src,
                w,
                h,
                alt: String::new(),
                rotation: None,
                anchor: None,
            }),
        );
    }

    // ---------- 图片、公式块 ----------

    fn image_block(&mut self, para: &Para, img: &Img) -> String {
        let mut st = Map::new();
        let ps = para.ppr.to_style(10.0);
        let pm = to_map(&ps);
        let exp = to_map(&self.expected("Image", None, None));
        for k in ["spaceBefore", "spaceAfter"] {
            let d = diff(&pm, &exp, &[k], &[]);
            st.extend(d);
        }
        if let Some(r) = img.rotation {
            st.insert("rotation".into(), json!(r));
        }
        match &img.anchor {
            None => match ps.align {
                Some(style::Align::Center) => {
                    st.insert("horizontalAlign".into(), json!("center"));
                }
                Some(style::Align::Right) => {
                    st.insert("horizontalAlign".into(), json!("right"));
                }
                _ => {}
            },
            Some(a) => {
                let float = a.h_rel == "margin"
                    && a.v_rel == "paragraph"
                    && matches!(a.h_align.as_deref(), Some("left" | "right"))
                    && matches!(a.wrap, Some("square" | "tight"))
                    && !a.behind;
                if float {
                    st.insert("position".into(), json!("float"));
                    st.insert("float".into(), json!(a.h_align.as_deref().unwrap()));
                    if (a.dist - 6.0).abs() > 0.1 {
                        st.insert("wrapDistance".into(), json!(round2(a.dist)));
                    }
                } else {
                    st.insert("position".into(), json!("absolute"));
                    let anchor = match (a.h_rel.as_str(), a.v_rel.as_str()) {
                        ("page", _) => "page",
                        ("margin", "margin") => "margin",
                        _ => "paragraph",
                    };
                    st.insert("anchorTo".into(), json!(anchor));
                    match a.h_align.as_deref() {
                        Some(al @ ("left" | "center" | "right")) => {
                            st.insert("horizontalAlign".into(), json!(al));
                        }
                        _ => {
                            st.insert("x".into(), json!(round2(a.x)));
                        }
                    }
                    st.insert("y".into(), json!(round2(a.y)));
                    let wrap = if a.behind {
                        "behind"
                    } else {
                        match a.wrap {
                            Some("none") | None => "inFront",
                            Some(w) => w,
                        }
                    };
                    st.insert("wrap".into(), json!(wrap));
                }
            }
        }
        let mut props = Map::new();
        props.insert("src".into(), json!(img.src));
        if !img.alt.is_empty() {
            props.insert("alt".into(), json!(img.alt));
        }
        if img.w > 0.0 {
            props.insert("width".into(), json!(round2(img.w)));
        }
        if img.h > 0.0 {
            props.insert("height".into(), json!(round2(img.h)));
        }
        let id = self.id("image");
        let mut el = json!({ "type": "Image", "props": props });
        if !st.is_empty() {
            el["style"] = Value::Object(st);
        }
        self.add(&id, el);
        id
    }

    fn math_block(
        &mut self,
        para: &Para,
        latex: &str,
        number: Option<Value>,
        jc: Option<&str>,
        mstyle: &Style,
    ) -> String {
        let fs = mstyle.font_size.map(|l| l.abs_pt()).unwrap_or(10.0);
        let mut ps = para.ppr.to_style(fs);
        ps.align = match jc {
            Some("left") => Some(style::Align::Left),
            Some("right") => Some(style::Align::Right),
            _ => Some(style::Align::Center),
        };
        if number.is_some() {
            ps.align = Some(style::Align::Center);
        }
        let mut eff = to_map(&ps);
        let mut ts = to_map(&text_of(mstyle));
        ts.remove("fontFamily");
        let tkeys: Vec<String> = ts.keys().cloned().collect();
        eff.extend(ts);
        let exp = to_map(&self.expected("Math", None, None));
        let mut keys: Vec<&str> = PARA_KEYS.to_vec();
        keys.retain(|k| !matches!(*k, "marginLeft" | "textIndent" | "hangingIndent"));
        keys.extend(["fontSize", "color", "bold"]);
        let soft: Vec<&str> = ["fontSize", "color", "bold"]
            .into_iter()
            .filter(|k| !tkeys.iter().any(|t| t == k))
            .collect();
        let mut d = diff(&eff, &exp, &keys, &soft);
        compact(&mut d);
        let mut props = json!({ "latex": latex });
        if let Some(n) = number {
            props["number"] = n;
        }
        let id = self.id("math");
        let mut el = json!({ "type": "Math", "props": props });
        if !d.is_empty() {
            el["style"] = Value::Object(d);
        }
        self.add(&id, el);
        id
    }

    // ---------- 列表 ----------

    fn list(&mut self, lr: ListRef, items: Vec<Para>) -> String {
        let levels = &lr.levels;
        let min_lvl = items.iter().filter_map(|p| p.list.as_ref().map(|l| l.ilvl)).min().unwrap_or(0);
        // 段落和文字
        let item_runs: Vec<Vec<Map<String, Value>>> = items
            .iter()
            .map(|p| {
                p.segs
                    .iter()
                    .filter_map(|s| match s {
                        Seg::Text { text, style, .. } if !text.is_empty() => {
                            Some(to_map(&text_of(style)))
                        }
                        _ => None,
                    })
                    .collect()
            })
            .collect();
        let all_runs: Vec<Map<String, Value>> = item_runs.iter().flatten().cloned().collect();
        let hoisted = if all_runs.is_empty() {
            to_map(&text_of(&items[0].run_base))
        } else {
            hoist(&all_runs, TEXT_KEYS)
        };
        let fs = size_of(&hoisted).unwrap_or(10.0);
        let first = &items[0];
        let last = items.last().unwrap();
        let mut ps = first.ppr.to_style(fs);
        ps.margin_left = None;
        ps.text_indent = None;
        ps.hanging_indent = None;
        ps.space_after = last.ppr.to_style(fs).space_after;
        let mut eff = to_map(&ps);
        for (k, v) in &hoisted {
            eff.insert(k.clone(), v.clone());
        }
        // 每一级的列表字段
        let lvl_ordered = |i: usize| -> bool { list_type(&levels[i]).0.ordered() };
        let ordered = lvl_ordered(min_lvl);
        let fields = |ctx: &mut Ctx, i: usize, level: usize, para: Option<&Para>| -> Map<String, Value> {
            let l = &levels[i];
            let (ty, font) = list_type(l);
            let mut m = Map::new();
            m.insert(
                "listStyleType".into(),
                serde_json::to_value(&ty).unwrap_or(Value::Null),
            );
            if ty.ordered() {
                let rt = renumber(&l.text, min_lvl);
                if rt != format!("%{level}.") {
                    m.insert("listMarkerFormat".into(), json!(rt));
                }
            } else if let ListStyleType::Char(_) = ty
                && let Some(f) = font
            {
                m.insert("listMarkerFont".into(), json!(f));
            }
            if let Some(s) = l.start.filter(|s| *s != 1) {
                m.insert("listStart".into(), json!(s));
            }
            let ind = match para {
                Some(p) if p.direct_ind => p.ppr.ind,
                _ => l.ind,
            };
            let left = ind.left.unwrap_or(0.0) / 20.0;
            let hanging = ind.hanging.unwrap_or(0.0) / 20.0;
            let li = word::nice_em(left / level as f32, fs);
            let lh = word::nice_em(hanging, fs);
            m.insert("listIndent".into(), serde_json::to_value(li).unwrap());
            m.insert("listHanging".into(), serde_json::to_value(lh).unwrap());
            match l.jc.as_deref() {
                Some("center") => {
                    m.insert("listMarkerAlign".into(), json!("center"));
                }
                Some("right" | "end") => {
                    m.insert("listMarkerAlign".into(), json!("right"));
                }
                _ => {}
            }
            match l.suff.as_deref() {
                Some("space") => {
                    m.insert("listMarkerGap".into(), json!("space"));
                }
                Some("nothing") => {
                    m.insert("listMarkerGap".into(), json!("none"));
                }
                _ => {}
            }
            if let Some(c) = &l.rpr.color {
                let text_c = hoisted.get("color").and_then(|v| v.as_str()).map(str::to_string);
                if Some(c.css()) != text_c {
                    m.insert("listMarkerColor".into(), json!(c.css()));
                }
            }
            if let Some(sz) = l.rpr.font_size.map(|x| x.abs_pt())
                && (sz - fs).abs() > 0.4
                && fs > 0.0
            {
                m.insert("listMarkerSize".into(), json!(round2(sz / fs * 100.0)));
            }
            clean_map(&mut m);
            let _ = ctx;
            m
        };
        let first_para_of = |i: usize| items.iter().find(|p| p.list.as_ref().is_some_and(|l| l.ilvl == i));
        let f1 = fields(self, min_lvl, 1, first_para_of(min_lvl));
        let exp_style = self.expected("List", None, None);
        let exp = to_map(&exp_style);
        let mut d = Map::new();
        // 段落字段和文字字段
        let soft: Vec<&str> = TEXT_KEYS
            .iter()
            .copied()
            .filter(|k| !hoisted.contains_key(*k))
            .collect();
        let mut keys: Vec<&str> = PARA_KEYS.to_vec();
        keys.retain(|k| !matches!(*k, "marginLeft" | "textIndent" | "hangingIndent"));
        keys.extend(TEXT_KEYS.iter().copied());
        d.extend(diff(&eff, &exp, &keys, &soft));
        // 第一级的列表字段：编号类型和默认一样时不写
        for (k, v) in &f1 {
            if k == "listStyleType" {
                let def = serde_json::to_value(crate::render::common::default_type(1, ordered)).unwrap();
                if same(v, &def) {
                    continue;
                }
            }
            if exp.get(k).is_some_and(|e| same(e, v)) {
                continue;
            }
            d.insert(k.clone(), v.clone());
        }
        let elem_style = merged(&exp_style, &from_map(&d));
        // 其他级：和第一级的有效值不一样的写进 levelN
        let mut parts = Map::new();
        let mut used: Vec<usize> = items
            .iter()
            .filter_map(|p| p.list.as_ref().map(|l| l.ilvl))
            .filter(|i| *i > min_lvl)
            .collect();
        used.sort();
        used.dedup();
        let base_list = {
            let mut m = to_map(&elem_style);
            m.retain(|k, _| LIST_KEYS.contains(&k.as_str()));
            m
        };
        let mut ordered_at: Vec<bool> = vec![ordered];
        for i in min_lvl + 1..=used.last().copied().unwrap_or(min_lvl) {
            ordered_at.push(lvl_ordered(i));
        }
        for i in used {
            let level = i - min_lvl + 1;
            let fi = fields(self, i, level, first_para_of(i));
            let mut base = base_list.clone();
            if !base.contains_key("listStyleType") {
                let o = ordered_at.get(level - 1).copied().unwrap_or(ordered);
                base.insert(
                    "listStyleType".into(),
                    serde_json::to_value(crate::render::common::default_type(level, o)).unwrap(),
                );
            }
            let mut pd = Map::new();
            for (k, v) in &fi {
                if !base.get(k).is_some_and(|b| same(b, v)) {
                    pd.insert(k.clone(), v.clone());
                }
            }
            for k in base.keys() {
                if !fi.contains_key(k) && matches!(k.as_str(), "listMarkerFormat" | "listStart" | "listMarkerAlign" | "listMarkerGap" | "listMarkerColor" | "listMarkerSize" | "listMarkerFont") {
                    let def = match k.as_str() {
                        "listMarkerFormat" => json!(format!("%{level}.")),
                        "listStart" => json!(1),
                        "listMarkerAlign" => json!("left"),
                        "listMarkerGap" => json!("tab"),
                        _ => continue,
                    };
                    pd.insert(k.clone(), def);
                }
            }
            if !pd.is_empty() {
                parts.insert(format!("level{level}"), Value::Object(pd));
            }
        }
        // 项与项之间的间距
        if items.len() > 1 {
            let afters: Vec<f32> = items[..items.len() - 1]
                .iter()
                .map(|p| p.ppr.to_style(fs).space_after.map(|l| l.pt(fs, 0.0)).unwrap_or(0.0))
                .collect();
            let befores: Vec<f32> = items[1..]
                .iter()
                .map(|p| p.ppr.to_style(fs).space_before.map(|l| l.pt(fs, 0.0)).unwrap_or(0.0))
                .collect();
            let mut ip = Map::new();
            if afters[0] > 0.0 {
                ip.insert("spaceAfter".into(), json!(round2(afters[0])));
            }
            if befores[0] > 0.0 {
                ip.insert("spaceBefore".into(), json!(round2(befores[0])));
            }
            if !ip.is_empty() {
                parts.insert("item".into(), Value::Object(ip));
            }
        }
        if levels[min_lvl].rpr.bold == Some(true) {
            parts.insert("marker".into(), json!({ "bold": true }));
        }
        if items.iter().any(|p| p.segs.iter().any(|s| matches!(s, Seg::PageBreak))) {
            self.warns.once(
                "listbreak",
                "列表项里的分页符没保留".into(),
            );
        }
        // 项：按层级嵌套
        struct It {
            text: Value,
            kids: Vec<It>,
            ordered: Option<bool>,
        }
        let mut root: Vec<It> = Vec::new();
        for p in &items {
            let l = p.list.as_ref().map(|x| x.ilvl).unwrap_or(min_lvl).max(min_lvl) - min_lvl;
            let segs: Vec<Seg> = p.segs.iter().filter(|s| !matches!(s, Seg::PageBreak)).cloned().collect();
            let text = self.inline_json(&segs, &elem_style);
            let mut cur = &mut root;
            for depth in 0..l {
                if cur.is_empty() {
                    cur.push(It { text: json!(""), kids: Vec::new(), ordered: None });
                }
                let parent = cur.last_mut().unwrap();
                let child_ordered = ordered_at.get(depth + 1).copied().unwrap_or(ordered);
                let parent_ordered = ordered_at.get(depth).copied().unwrap_or(ordered);
                if child_ordered != parent_ordered {
                    parent.ordered = Some(child_ordered);
                }
                cur = &mut parent.kids;
            }
            cur.push(It { text, kids: Vec::new(), ordered: None });
        }
        fn to_json(items: Vec<It>) -> Value {
            Value::Array(
                items
                    .into_iter()
                    .map(|it| {
                        if it.kids.is_empty() {
                            it.text
                        } else {
                            let mut m = json!({ "text": it.text, "items": to_json(it.kids) });
                            if let Some(o) = it.ordered {
                                m["ordered"] = json!(o);
                            }
                            m
                        }
                    })
                    .collect(),
            )
        }
        let mut props = Map::new();
        if ordered {
            props.insert("ordered".into(), json!(true));
        }
        props.insert("items".into(), to_json(root));
        let id = self.id("list");
        let mut el = json!({ "type": "List", "props": props });
        compact(&mut d);
        if !d.is_empty() {
            el["style"] = Value::Object(d);
        }
        if !parts.is_empty() {
            el["parts"] = Value::Object(parts);
        }
        self.add(&id, el);
        id
    }

    // ---------- 收尾 ----------

    fn finish(&mut self, doc: &Document) -> Result<Value> {
        let _ = doc;
        let sects = std::mem::take(&mut self.done_sects);
        let mut root_children: Vec<Value> = Vec::new();
        let mut meta = Map::new();
        if let Some(core) = self.files.get("docProps/core.xml") {
            let t = String::from_utf8_lossy(core).into_owned();
            if let Ok(d) = Document::parse(&t) {
                for n in d.descendants() {
                    match n.tag_name().name() {
                        "title" => {
                            if let Some(x) = n.text().filter(|s| !s.trim().is_empty()) {
                                meta.insert("title".into(), json!(x));
                            }
                        }
                        "creator" => {
                            if let Some(x) = n.text().filter(|s| !s.trim().is_empty() && *s != "unknown") {
                                meta.insert("author".into(), json!(x));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // 页眉页脚：逐节累积，判断哪些要写 null
        let mut seen_h = [false; 3];
        let mut seen_f = [false; 3];
        let n = sects.len();
        for (i, (blocks, s)) in sects.into_iter().enumerate() {
            let mut page = page_json(&s);
            let header = self.hf_def(&s, &s.headers, i, &mut seen_h);
            let footer = self.hf_def(&s, &s.footers, i, &mut seen_f);
            if let Some(h) = header {
                page.insert("header".into(), h);
            }
            if let Some(f) = footer {
                page.insert("footer".into(), f);
            }
            if let Some(st) = s.pg_start {
                page.insert("pageNumberStart".into(), json!(st));
            }
            if i == 0 {
                meta.insert("page".into(), Value::Object(page));
                root_children.extend(blocks.into_iter().map(Value::String));
            } else {
                let id = self.id("section");
                self.add(
                    &id,
                    json!({ "type": "Section", "props": page, "children": blocks }),
                );
                root_children.push(json!(id));
            }
        }
        let _ = n;
        self.add(
            "doc",
            json!({ "type": "Document", "children": root_children }),
        );
        let mut out = json!({
            "version": "0.1",
            "meta": meta,
            "root": "doc",
            "elements": self.elements,
        });
        if !self.out_styles.is_empty() {
            out["styles"] = Value::Object(self.out_styles.clone());
        }
        clean(&mut out);
        Ok(out)
    }

    /// 一节的页眉（或页脚）定义。
    fn hf_def(&mut self, s: &Sect, refs: &[(String, String)], i: usize, seen: &mut [bool; 3]) -> Option<Value> {
        let mut m = Map::new();
        for (slot, kind) in ["default", "first", "even"].iter().enumerate() {
            if *kind == "first" && !s.title_pg {
                continue;
            }
            if *kind == "even" && !self.even_odd {
                continue;
            }
            let r = refs.iter().find(|(t, _)| t == kind).map(|(_, id)| id.clone());
            let v = match r {
                Some(rid) => self.hf_element(&rid),
                None => {
                    if *kind == "first" && i == 0 {
                        None
                    } else {
                        continue;
                    }
                }
            };
            match v {
                Some(id) => {
                    seen[slot] = true;
                    m.insert(kind.to_string(), json!(id));
                }
                None => {
                    if seen[slot] || *kind == "first" {
                        m.insert(kind.to_string(), Value::Null);
                    }
                    seen[slot] = false;
                }
            }
        }
        if m.is_empty() {
            return None;
        }
        if m.len() == 1
            && let Some(Value::String(id)) = m.get("default")
        {
            return Some(json!(id));
        }
        Some(Value::Object(m))
    }

    /// 页眉页脚部件 → 元素 id；没内容时 None。
    fn hf_element(&mut self, rid: &str) -> Option<String> {
        let rel = self.rels.get(rid)?.clone();
        if let Some(x) = self.hf_cache.get(&rel.target) {
            return x.clone();
        }
        let text = String::from_utf8_lossy(self.files.get(&rel.target)?).into_owned();
        let doc = Document::parse(&text).ok()?;
        let saved_rels = std::mem::replace(&mut self.rels, read_rels(&self.files, &rel.target));
        let saved_frames = std::mem::take(&mut self.frames);
        let saved_last = self.last_table.take();
        let mut blocks = Vec::new();
        self.blocks(doc.root_element(), &mut blocks, false);
        self.rels = saved_rels;
        self.frames = saved_frames;
        self.last_table = saved_last;
        // 只有空段落的页眉当成空的
        let empty = blocks.iter().all(|id| {
            self.elements.get(id).is_some_and(|e| {
                e["type"] == "Text" && e["props"]["text"].as_str().is_some_and(|t| t.trim().is_empty())
            })
        });
        let out = if empty {
            for id in &blocks {
                self.elements.remove(id);
            }
            None
        } else if blocks.len() == 1 {
            Some(blocks[0].clone())
        } else {
            let kind = if rel.target.contains("footer") { "footer" } else { "header" };
            let id = self.id(if kind == "footer" { "footer" } else { "header" });
            self.add(&id, json!({ "type": "Document", "children": blocks }));
            Some(id)
        };
        self.hf_cache.insert(rel.target, out.clone());
        out
    }
}

fn is_code_bg(bg: &style::Background) -> bool {
    builtin("InlineCode").and_then(|b| b.background.as_ref()) == Some(bg)
}

/// 两个 run 的 JSON 除了文字都一样时拼起来。
fn join_runs(a: &Value, b: &Value) -> Option<Value> {
    match (a, b) {
        (Value::String(x), Value::String(y)) => Some(json!(format!("{x}{y}"))),
        (Value::Object(x), Value::Object(y)) => {
            if x.contains_key("field") || y.contains_key("field") || x.contains_key("image") || y.contains_key("image") {
                return None;
            }
            if x.get("type").is_some_and(|t| t == "InlineMath") || y.get("type").is_some_and(|t| t == "InlineMath") {
                return None;
            }
            let strip = |m: &Map<String, Value>| {
                let mut m = m.clone();
                m.remove("text");
                m
            };
            if strip(x) != strip(y) {
                return None;
            }
            let mut m = x.clone();
            let t = format!(
                "{}{}",
                x.get("text").and_then(Value::as_str).unwrap_or(""),
                y.get("text").and_then(Value::as_str).unwrap_or("")
            );
            m.insert("text".into(), json!(t));
            Some(Value::Object(m))
        }
        _ => None,
    }
}

/// 一个 run 的 JSON。粗斜体下划线颜色用简写，其他放 style；链接一定写颜色和下划线
/// （不然会套上链接的默认蓝色下划线）。
fn run_value(
    text: &str,
    mut d: Map<String, Value>,
    href: Option<&str>,
    code: bool,
    named: Option<String>,
    eff: &Map<String, Value>,
) -> Value {
    if d.is_empty() && href.is_none() && !code && named.is_none() {
        return json!(text);
    }
    let mut m = Map::new();
    if code {
        m.insert("type".into(), json!("InlineCode"));
    }
    m.insert("text".into(), json!(text));
    if let Some(h) = href {
        m.insert("href".into(), json!(h));
        let c = eff.get("color").cloned().unwrap_or(json!("#000000"));
        m.insert("color".into(), c);
        let u = eff.get("underline").cloned().unwrap_or(json!("none"));
        m.insert("underline".into(), shorthand_underline(u));
        d.remove("color");
        d.remove("underline");
    }
    if named.is_none() {
        for k in ["bold", "italic", "color"] {
            if let Some(v) = d.remove(k) {
                m.insert(k.into(), v);
            }
        }
        if let Some(u) = d.remove("underline") {
            m.insert("underline".into(), shorthand_underline(u));
        }
    }
    compact(&mut d);
    match (named, d.is_empty()) {
        (Some(n), true) => {
            m.insert("style".into(), json!(n));
        }
        (Some(n), false) => {
            m.insert("style".into(), json!([n, Value::Object(d)]));
        }
        (None, false) => {
            m.insert("style".into(), Value::Object(d));
        }
        (None, true) => {}
    }
    Value::Object(m)
}

fn shorthand_underline(u: Value) -> Value {
    match u.as_str() {
        Some("single") => json!(true),
        Some("none") => json!(false),
        _ => u,
    }
}

/// 元素的 style：命名样式加差别。
fn style_value(named: Option<String>, d: Map<String, Value>) -> Option<Value> {
    match (named, d.is_empty()) {
        (Some(n), true) => Some(json!(n)),
        (Some(n), false) => Some(json!([n, Value::Object(d)])),
        (None, false) => Some(Value::Object(d)),
        (None, true) => None,
    }
}

/// 一串内容里第一段文字和它的样式（域的显示值）。
fn first_text(segs: &[Seg], base: &Style) -> (String, Style) {
    let mut text = String::new();
    let mut style = None;
    for s in segs {
        if let Seg::Text { text: t, style: st, .. } = s {
            text.push_str(t);
            if style.is_none() {
                style = Some(st.clone());
            }
        }
    }
    (text, style.unwrap_or_else(|| base.clone()))
}

/// 符号字体（Symbol、Wingdings）里的字符换成 Unicode。
fn symbol_char(font: &str, code: u32) -> String {
    let c = if code >= 0xF000 { code - 0xF000 } else { code };
    let f = font.to_ascii_lowercase();
    if f.contains("symbol") {
        let m = match c {
            0x61 => 'α',
            0x62 => 'β',
            0x63 => 'χ',
            0x64 => 'δ',
            0x65 => 'ε',
            0x66 => 'φ',
            0x67 => 'γ',
            0x6C => 'λ',
            0x6D => 'μ',
            0x70 => 'π',
            0x71 => 'θ',
            0x72 => 'ρ',
            0x73 => 'σ',
            0x77 => 'ω',
            0xA3 => '≤',
            0xB3 => '≥',
            0xB1 => '±',
            0xB4 => '×',
            0xB7 => '•',
            0xB9 => '≠',
            0xBB => '≈',
            0xA5 => '∞',
            0xAE => '→',
            0xAC => '←',
            0xD6 => '√',
            0xE5 => '∑',
            _ => char::from_u32(c).unwrap_or('?'),
        };
        return m.to_string();
    }
    if f.contains("wingdings") {
        let m = match c {
            0xFC => '✓',
            0xFB => '✗',
            0xA7 => '▪',
            0x6C => '●',
            0x6E => '■',
            0x71 => '❑',
            0x76 => '❖',
            0xD8 => '➢',
            0xE0 => '→',
            0xA8 => '◻',
            _ => '•',
        };
        return m.to_string();
    }
    char::from_u32(code).map(|c| c.to_string()).unwrap_or_default()
}

/// 编号定义的一级换成 listStyleType，返回符号字体（Char 类型才有用）。
fn list_type(l: &Lvl) -> (ListStyleType, Option<String>) {
    let t = match l.fmt.as_str() {
        "decimal" | "decimalHalfWidth" | "decimalFullWidth" => ListStyleType::Decimal,
        "decimalZero" => ListStyleType::DecimalLeadingZero,
        "lowerLetter" => ListStyleType::LowerAlpha,
        "upperLetter" => ListStyleType::UpperAlpha,
        "lowerRoman" => ListStyleType::LowerRoman,
        "upperRoman" => ListStyleType::UpperRoman,
        "chineseCounting" | "chineseCountingThousand" | "japaneseCounting" | "chineseLegalSimplified"
        | "ideographDigital" | "taiwaneseCounting" | "taiwaneseCountingThousand" => {
            ListStyleType::CjkDecimal
        }
        "ideographTraditional" => ListStyleType::CjkIdeographic,
        "decimalEnclosedCircle" | "decimalEnclosedCircleChinese" => ListStyleType::CircledDecimal,
        "none" => ListStyleType::None,
        "bullet" => {
            let mut chars = l.text.chars();
            let c = chars.next();
            let font = l.font.clone().unwrap_or_default();
            let sym = font.to_ascii_lowercase();
            let t = match c {
                None => ListStyleType::None,
                Some('•' | '\u{F0B7}' | '·' | '●') => ListStyleType::Disc,
                Some('◦' | '○') => ListStyleType::Circle,
                Some('o') if sym.contains("courier") => ListStyleType::Circle,
                Some('▪' | '■' | '\u{F0A7}') => ListStyleType::Square,
                Some('–' | '-' | '—') => ListStyleType::Dash,
                Some(x) if (x as u32) >= 0xF000 && (x as u32) <= 0xF0FF => {
                    let s = symbol_char(&font, x as u32);
                    match s.as_str() {
                        "•" => ListStyleType::Disc,
                        "▪" => ListStyleType::Square,
                        other => ListStyleType::Char(other.to_string()),
                    }
                }
                Some(_) => {
                    let s: String = l.text.chars().take(3).collect();
                    ListStyleType::Char(s)
                }
            };
            return (t, None);
        }
        _ => ListStyleType::Decimal,
    };
    (t, None)
}

/// 列表从第 min 级开始时，lvlText 里的 %k 要往前挪。
fn renumber(text: &str, min: usize) -> String {
    if min == 0 {
        return text.to_string();
    }
    let mut out = text.to_string();
    for k in 1..=9usize {
        let from = format!("%{k}");
        if out.contains(&from) {
            let to = if k > min { format!("%{}", k - min) } else { String::new() };
            out = out.replace(&from, &to);
        }
    }
    out
}

/// 一节的 meta.page / Section props。
fn page_json(s: &Sect) -> Map<String, Value> {
    let mut m = Map::new();
    let (mut w, mut h) = (s.w, s.h);
    let landscape = s.landscape || w > h + 0.5;
    if landscape && w > h {
        std::mem::swap(&mut w, &mut h);
    }
    let name = ["A3", "A4", "A5", "B5", "Letter", "Legal"].into_iter().find(|n| {
        crate::units::page_size_pt(n).is_some_and(|(pw, ph)| (pw - w).abs() < 1.5 && (ph - h).abs() < 1.5)
    });
    match name {
        Some(n) => {
            m.insert("size".into(), json!(n));
        }
        None => {
            m.insert(
                "size".into(),
                json!({ "width": format!("{}pt", round2(w)), "height": format!("{}pt", round2(h)) }),
            );
        }
    }
    if landscape {
        m.insert("landscape".into(), json!(true));
    }
    let mar = s.margins.map(round2);
    let mut counts: Vec<(f32, usize)> = Vec::new();
    for x in mar {
        match counts.iter_mut().find(|(v, _)| (*v - x).abs() < 0.01) {
            Some(c) => c.1 += 1,
            None => counts.push((x, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    let main = counts[0].0;
    m.insert("margin".into(), json!(main));
    for (i, k) in ["marginTop", "marginRight", "marginBottom", "marginLeft"].iter().enumerate() {
        if (mar[i] - main).abs() > 0.01 {
            m.insert(k.to_string(), json!(mar[i]));
        }
    }
    if (s.header_d - 36.0).abs() > 0.01 {
        m.insert("headerDistance".into(), json!(round2(s.header_d)));
    }
    if (s.footer_d - 36.0).abs() > 0.01 {
        m.insert("footerDistance".into(), json!(round2(s.footer_d)));
    }
    if s.gutter > 0.0 {
        m.insert("gutter".into(), json!(round2(s.gutter)));
    }
    if let Some((c, g)) = s.cols {
        m.insert("columns".into(), json!({ "count": c, "gap": round2(g) }));
    }
    m
}

/// 给测试和命令行用：长度写成 JSON 值。
pub fn len_value(l: Len) -> Value {
    serde_json::to_value(l).unwrap_or(Value::Null)
}
