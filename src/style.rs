//! 样式模型：字段平铺，名字跟 CSS 走，取值以 OOXML 能表达的为准。完整说明见 docs/STYLE.md。

use std::borrow::Cow;

use anyhow::{Result, bail};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::units;

/// 给「从 JSON 值解析」的类型补上 serde 和 schemars 实现。
macro_rules! value_type {
    ($t:ty, $name:literal, $schema:tt) => {
        impl<'de> Deserialize<'de> for $t {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let v = Value::deserialize(d)?;
                <$t>::from_value(&v).map_err(serde::de::Error::custom)
            }
        }
        impl Serialize for $t {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                self.to_value().serialize(s)
            }
        }
        impl JsonSchema for $t {
            fn schema_name() -> Cow<'static, str> {
                $name.into()
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!($schema)
            }
        }
    };
}

/// 字符串枚举：JSON 里写 camelCase 字符串。
macro_rules! str_enum {
    ($(#[$m:meta])* $t:ident { $($v:ident = $s:literal),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        pub enum $t { $( #[serde(rename = $s)] $v ),* }
        impl $t {
            pub fn as_str(self) -> &'static str { match self { $( $t::$v => $s ),* } }
            #[allow(dead_code)]
            pub const NAMES: &'static [&'static str] = &[$($s),*];
        }
    };
}

// ---------- 长度 ----------

/// 长度：数字是 pt；字符串可带 pt、mm、cm、in、px、em、line、%。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Len {
    Pt(f32),
    /// 当前字号的倍数
    Em(f32),
    /// 行数，只用在段前段后
    Line(f32),
    /// 百分比，用在宽高
    Pct(f32),
}

impl Len {
    pub fn from_value(v: &Value) -> Result<Len> {
        match v {
            Value::Number(n) => Ok(Len::Pt(n.as_f64().unwrap_or(0.0) as f32)),
            Value::String(s) => Len::parse(s),
            _ => bail!("长度要写数字（pt）或带单位的字符串，如 \"4mm\""),
        }
    }

    pub fn parse(s: &str) -> Result<Len> {
        let t = s.trim();
        if let Some(n) = t.strip_suffix('%') {
            return Ok(Len::Pct(num(n, s)?));
        }
        if let Some(n) = t.strip_suffix("em") {
            return Ok(Len::Em(num(n, s)?));
        }
        if let Some(n) = t.strip_suffix("line") {
            return Ok(Len::Line(num(n, s)?));
        }
        units::parse_len_pt(t)
            .map(Len::Pt)
            .ok_or_else(|| anyhow::anyhow!("看不懂长度 `{s}`，单位支持 pt mm cm in px em line %"))
    }

    fn to_value(self) -> Value {
        match self {
            Len::Pt(v) => serde_json::json!(v),
            Len::Em(v) => format!("{v}em").into(),
            Len::Line(v) => format!("{v}line").into(),
            Len::Pct(v) => format!("{v}%").into(),
        }
    }

    /// 换成 pt。`font_size` 是当前字号；百分比相对 `whole`。
    pub fn pt(self, font_size: f32, whole: f32) -> f32 {
        match self {
            Len::Pt(v) => v,
            Len::Em(v) => v * font_size,
            Len::Line(v) => v * font_size * 1.2,
            Len::Pct(v) => v / 100.0 * whole,
        }
    }

    /// 不需要上下文的 pt（em 按 10.5pt 字号，百分比当 0）。
    pub fn abs_pt(self) -> f32 {
        self.pt(DEFAULT_FONT_SIZE, 0.0)
    }
}

fn num(n: &str, whole: &str) -> Result<f32> {
    n.trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("看不懂长度 `{whole}`"))
}

value_type!(Len, "Length", { "anyOf": [{ "type": "number" }, { "type": "string", "pattern": "^-?[0-9.]+(pt|mm|cm|in|px|em|line|%)?$" }] });

/// 表格一列的宽：数字是比例（分剩下的宽度），带单位的字符串是固定长度，`"auto"` 按内容撑开。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColWidth {
    Ratio(f32),
    Fixed(f32),
    Auto,
}

impl ColWidth {
    pub fn from_value(v: &Value) -> Result<ColWidth> {
        match v {
            Value::Number(n) => {
                let x = n.as_f64().unwrap_or(0.0) as f32;
                if x <= 0.0 {
                    bail!("列宽比例要大于 0");
                }
                Ok(ColWidth::Ratio(x))
            }
            Value::String(t) if t.trim() == "auto" => Ok(ColWidth::Auto),
            Value::String(t) => match Len::parse(t)? {
                Len::Pt(x) if x > 0.0 => Ok(ColWidth::Fixed(x)),
                _ => bail!("列宽 `{t}` 要写数字（比例）、绝对长度（如 \"20mm\"）或 \"auto\""),
            },
            _ => bail!("列宽要写数字（比例）、绝对长度（如 \"20mm\"）或 \"auto\""),
        }
    }

    fn to_value(self) -> Value {
        match self {
            ColWidth::Ratio(x) => serde_json::json!(x),
            ColWidth::Fixed(x) => format!("{x}pt").into(),
            ColWidth::Auto => "auto".into(),
        }
    }
}

value_type!(ColWidth, "ColWidth", { "anyOf": [{ "type": "number", "exclusiveMinimum": 0 }, { "const": "auto" }, { "type": "string", "pattern": "^[0-9.]+(pt|mm|cm|in|px)?$" }] });

pub const DEFAULT_FONT_SIZE: f32 = 10.5;

// ---------- 颜色 ----------

/// 颜色：存成不带 # 的大写十六进制，6 位或 8 位（带透明度）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Color(String);

impl Color {
    pub fn from_value(v: &Value) -> Result<Color> {
        match v {
            Value::String(s) => Color::parse(s),
            _ => bail!("颜色要写成 \"#RRGGBB\""),
        }
    }

    pub fn parse(s: &str) -> Result<Color> {
        let h = s
            .trim()
            .strip_prefix('#')
            .ok_or_else(|| anyhow::anyhow!("颜色 `{s}` 要以 # 开头，写成 #RRGGBB"))?;
        if !h.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("颜色 `{s}` 不是十六进制");
        }
        let full = match h.len() {
            3 | 4 => h.chars().flat_map(|c| [c, c]).collect(),
            6 | 8 => h.to_string(),
            _ => bail!("颜色 `{s}` 要写成 #RGB、#RRGGBB 或 #RRGGBBAA"),
        };
        Ok(Color(full.to_ascii_uppercase()))
    }

    fn to_value(&self) -> Value {
        format!("#{}", self.0).into()
    }

    /// RRGGBB，不带透明度。
    pub fn rgb(&self) -> &str {
        &self.0[..6]
    }

    /// 透明度 0–1，没写就是 1。
    pub fn alpha(&self) -> f32 {
        if self.0.len() == 8 {
            u8::from_str_radix(&self.0[6..], 16).unwrap_or(255) as f32 / 255.0
        } else {
            1.0
        }
    }

    /// `#RRGGBB` 或 `#RRGGBBAA`。
    pub fn css(&self) -> String {
        format!("#{}", self.0)
    }
}

value_type!(Color, "Color", { "type": "string", "pattern": "^#([0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$" });

// ---------- 字体 ----------

/// 字体：一个名字，或按文种分开。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FontFamily {
    /// 西文
    pub latin: Option<String>,
    /// 中日韩
    pub east_asia: Option<String>,
    /// 阿拉伯文等复杂文种
    pub complex: Option<String>,
}

impl FontFamily {
    pub fn from_value(v: &Value) -> Result<FontFamily> {
        match v {
            Value::String(s) => Ok(FontFamily {
                latin: Some(s.clone()),
                east_asia: Some(s.clone()),
                complex: Some(s.clone()),
            }),
            Value::Object(m) => {
                let mut f = FontFamily::default();
                for (k, x) in m {
                    let s = x
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("fontFamily.{k} 要是字符串"))?
                        .to_string();
                    match k.as_str() {
                        "latin" => f.latin = Some(s),
                        "eastAsia" => f.east_asia = Some(s),
                        "complex" => f.complex = Some(s),
                        _ => bail!("fontFamily 只认 latin、eastAsia、complex，没有 `{k}`"),
                    }
                }
                Ok(f)
            }
            _ => bail!("fontFamily 要是字体名，或 {{ latin, eastAsia, complex }}"),
        }
    }

    fn to_value(&self) -> Value {
        let mut m = serde_json::Map::new();
        for (k, v) in [
            ("latin", &self.latin),
            ("eastAsia", &self.east_asia),
            ("complex", &self.complex),
        ] {
            if let Some(v) = v {
                m.insert(k.into(), v.clone().into());
            }
        }
        Value::Object(m)
    }

    /// 后写的文种覆盖前面的，没写的保留。
    pub fn merge(&mut self, o: &FontFamily) {
        if o.latin.is_some() {
            self.latin = o.latin.clone();
        }
        if o.east_asia.is_some() {
            self.east_asia = o.east_asia.clone();
        }
        if o.complex.is_some() {
            self.complex = o.complex.clone();
        }
    }

    /// 回退列表：西文在前，中文在后，去重。
    pub fn fallback(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for f in [&self.latin, &self.east_asia, &self.complex]
            .into_iter()
            .flatten()
        {
            if !v.contains(&f.as_str()) {
                v.push(f);
            }
        }
        v
    }
}

value_type!(FontFamily, "FontFamily", { "anyOf": [
    { "type": "string" },
    { "type": "object", "properties": { "latin": { "type": "string" }, "eastAsia": { "type": "string" }, "complex": { "type": "string" } }, "additionalProperties": false }
] });

// ---------- 小枚举 ----------

str_enum!(UnderlineStyle {
    None = "none",
    Single = "single",
    Double = "double",
    Thick = "thick",
    Dotted = "dotted",
    Dashed = "dashed",
    Wavy = "wavy",
});

/// 下划线：布尔或线型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Underline(pub UnderlineStyle);

impl Underline {
    fn from_value(v: &Value) -> Result<Underline> {
        Ok(Underline(match v {
            Value::Bool(true) => UnderlineStyle::Single,
            Value::Bool(false) => UnderlineStyle::None,
            _ => serde_json::from_value(v.clone()).map_err(|_| {
                anyhow::anyhow!(
                    "underline 要是布尔，或 single double thick dotted dashed wavy none"
                )
            })?,
        }))
    }
    fn to_value(self) -> Value {
        self.0.as_str().into()
    }
    pub fn on(self) -> bool {
        self.0 != UnderlineStyle::None
    }
}

value_type!(Underline, "Underline", { "anyOf": [{ "type": "boolean" }, { "enum": ["none", "single", "double", "thick", "dotted", "dashed", "wavy"] }] });

/// 删除线：布尔或 "double"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strike {
    None,
    Single,
    Double,
}

impl Strike {
    fn from_value(v: &Value) -> Result<Strike> {
        Ok(match v {
            Value::Bool(true) => Strike::Single,
            Value::Bool(false) => Strike::None,
            Value::String(s) if s == "double" => Strike::Double,
            Value::String(s) if s == "single" => Strike::Single,
            _ => bail!("strike 要是布尔或 \"double\""),
        })
    }
    fn to_value(self) -> Value {
        match self {
            Strike::None => false.into(),
            Strike::Single => true.into(),
            Strike::Double => "double".into(),
        }
    }
}

value_type!(Strike, "Strike", { "anyOf": [{ "type": "boolean" }, { "enum": ["single", "double"] }] });

str_enum!(Align {
    Left = "left",
    Center = "center",
    Right = "right",
    Justify = "justify",
    Distribute = "distribute",
});

str_enum!(
    /// 文字的上下标，也是盒子和图片的竖直对齐。
    VerticalAlign {
        Super = "super",
        Sub = "sub",
        Top = "top",
        Middle = "middle",
        Bottom = "bottom",
    }
);

str_enum!(Caps {
    All = "all",
    Small = "small",
});

str_enum!(EmphasisMark {
    Dot = "dot",
    Comma = "comma",
    Circle = "circle",
    UnderDot = "underDot",
});

str_enum!(Direction {
    Ltr = "ltr",
    Rtl = "rtl",
});

str_enum!(HAlign {
    Left = "left",
    Center = "center",
    Right = "right",
});

str_enum!(Position {
    Inline = "inline",
    Float = "float",
    Absolute = "absolute",
});

str_enum!(Side {
    Left = "left",
    Right = "right",
});

str_enum!(AnchorTo {
    Paragraph = "paragraph",
    Margin = "margin",
    Page = "page",
    Slide = "slide",
});

str_enum!(ObjectFit {
    Contain = "contain",
    Cover = "cover",
    Fill = "fill",
});

str_enum!(TableLayout {
    Fixed = "fixed",
    Auto = "auto",
});

str_enum!(WrapMode {
    Square = "square",
    Tight = "tight",
    TopAndBottom = "topAndBottom",
    None = "none",
    Behind = "behind",
    InFront = "inFront",
});

/// `wrap`：段落里是布尔（要不要自动换行），图片上是绕排方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    Text(bool),
    Image(WrapMode),
}

impl Wrap {
    fn from_value(v: &Value) -> Result<Wrap> {
        Ok(match v {
            Value::Bool(b) => Wrap::Text(*b),
            _ => Wrap::Image(serde_json::from_value(v.clone()).map_err(|_| {
                anyhow::anyhow!("wrap 要是布尔，或 square tight topAndBottom none behind inFront")
            })?),
        })
    }
    fn to_value(self) -> Value {
        match self {
            Wrap::Text(b) => b.into(),
            Wrap::Image(m) => m.as_str().into(),
        }
    }
}

value_type!(Wrap, "Wrap", { "anyOf": [{ "type": "boolean" }, { "enum": ["square", "tight", "topAndBottom", "none", "behind", "inFront"] }] });

// ---------- 行高 ----------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineHeight {
    /// 倍数
    Multiple(f32),
    /// 固定值
    Exact(Len),
    /// 最小值
    AtLeast(Len),
}

impl LineHeight {
    fn from_value(v: &Value) -> Result<LineHeight> {
        Ok(match v {
            Value::Number(n) => LineHeight::Multiple(n.as_f64().unwrap_or(1.0) as f32),
            Value::String(_) => match Len::from_value(v)? {
                Len::Em(x) => LineHeight::Multiple(x),
                Len::Pct(x) => LineHeight::Multiple(x / 100.0),
                l => LineHeight::Exact(l),
            },
            Value::Object(m) if m.len() == 1 && m.contains_key("atLeast") => {
                LineHeight::AtLeast(Len::from_value(&m["atLeast"])?)
            }
            _ => {
                bail!("lineHeight 写倍数（1.5）、固定值（\"20pt\"）或 {{ \"atLeast\": \"18pt\" }}")
            }
        })
    }
    fn to_value(self) -> Value {
        match self {
            LineHeight::Multiple(x) => serde_json::json!(x),
            // 固定值的 pt 要带单位写，光写数字读回来会当成倍数
            LineHeight::Exact(Len::Pt(v)) => format!("{v}pt").into(),
            LineHeight::Exact(l) => l.to_value(),
            LineHeight::AtLeast(l) => serde_json::json!({ "atLeast": l.to_value() }),
        }
    }
}

value_type!(LineHeight, "LineHeight", { "anyOf": [
    { "type": "number" }, { "type": "string" },
    { "type": "object", "properties": { "atLeast": { "type": ["number", "string"] } }, "required": ["atLeast"], "additionalProperties": false }
] });

// ---------- 边框 ----------

str_enum!(
    /// 线型：Word 的全部线型（ST_Border），艺术边框除外。名字照 Word，`single` 叫 `solid`。
    BorderStyle {
    None = "none",
    Solid = "solid",
    Thick = "thick",
    Dashed = "dashed",
    Dotted = "dotted",
    DashDot = "dashDot",
    DashDotDot = "dashDotDot",
    DashSmallGap = "dashSmallGap",
    DashDotStroked = "dashDotStroked",
    Double = "double",
    Triple = "triple",
    ThinThickSmallGap = "thinThickSmallGap",
    ThickThinSmallGap = "thickThinSmallGap",
    ThinThickThinSmallGap = "thinThickThinSmallGap",
    ThinThickMediumGap = "thinThickMediumGap",
    ThickThinMediumGap = "thickThinMediumGap",
    ThinThickThinMediumGap = "thinThickThinMediumGap",
    ThinThickLargeGap = "thinThickLargeGap",
    ThickThinLargeGap = "thickThinLargeGap",
    ThinThickThinLargeGap = "thinThickThinLargeGap",
    Wave = "wave",
    DoubleWave = "doubleWave",
    ThreeDEmboss = "threeDEmboss",
    ThreeDEngrave = "threeDEngrave",
    Outset = "outset",
    Inset = "inset",
});

/// 一条边：`{ width, style, color }`，或简写 `"1pt solid #CCCCCC"`，或 `"none"`。
#[derive(Debug, Clone, PartialEq)]
pub struct Border {
    /// 线宽，pt
    pub width: f32,
    pub style: BorderStyle,
    pub color: Color,
}

impl Border {
    pub fn none() -> Border {
        Border {
            width: 0.0,
            style: BorderStyle::None,
            color: Color("000000".into()),
        }
    }

    pub fn is_none(&self) -> bool {
        self.style == BorderStyle::None || self.width <= 0.0
    }

    fn from_value(v: &Value) -> Result<Border> {
        let mut b = Border {
            width: 1.0,
            style: BorderStyle::Solid,
            color: Color("000000".into()),
        };
        match v {
            Value::String(s) if s.trim() == "none" => return Ok(Border::none()),
            Value::String(s) => {
                for tok in s.split_whitespace() {
                    if tok.starts_with('#') {
                        b.color = Color::parse(tok)?;
                    } else if let Ok(st) = serde_json::from_value::<BorderStyle>(tok.into()) {
                        b.style = st;
                    } else {
                        b.width = Len::parse(tok)
                            .map_err(|_| {
                                anyhow::anyhow!("看不懂边框 `{s}`，写成 \"1pt solid #CCCCCC\"")
                            })?
                            .abs_pt();
                    }
                }
            }
            Value::Object(m) => {
                for (k, x) in m {
                    match k.as_str() {
                        "width" => b.width = Len::from_value(x)?.abs_pt(),
                        "style" => {
                            b.style = serde_json::from_value(x.clone()).map_err(|_| {
                                anyhow::anyhow!("边框 style 只认 {}", BorderStyle::NAMES.join(" "))
                            })?
                        }
                        "color" => b.color = Color::from_value(x)?,
                        _ => bail!("边框只认 width、style、color，没有 `{k}`"),
                    }
                }
            }
            _ => bail!("边框写成 {{ width, style, color }} 或 \"1pt solid #CCCCCC\""),
        }
        Ok(b)
    }

    fn to_value(&self) -> Value {
        if self.is_none() {
            return "none".into();
        }
        format!(
            "{}pt {} {}",
            self.width,
            self.style.as_str(),
            self.color.css()
        )
        .into()
    }

    /// CSS 的 border 值。CSS 没有的线型（见 `common::border_tile`）先写成透明边，
    /// 留出位置，图案另外叠上去。
    pub fn css(&self) -> String {
        if self.is_none() {
            return "none".into();
        }
        let st = match self.style {
            BorderStyle::Solid | BorderStyle::Thick => "solid",
            BorderStyle::Dashed => "dashed",
            BorderStyle::Dotted => "dotted",
            BorderStyle::Double => "double",
            BorderStyle::ThreeDEmboss => "ridge",
            BorderStyle::ThreeDEngrave => "groove",
            BorderStyle::Outset => "outset",
            BorderStyle::Inset => "inset",
            _ => return format!("{}pt solid transparent", self.width),
        };
        format!("{}pt {} {}", self.width, st, self.color.css())
    }
}

value_type!(Border, "Border", { "anyOf": [
    { "type": "string" },
    { "type": "object", "properties": {
        "width": { "type": ["number", "string"] },
        "style": { "enum": ["none", "solid", "thick", "dashed", "dotted", "dashDot", "dashDotDot",
            "dashSmallGap", "dashDotStroked", "double", "triple",
            "thinThickSmallGap", "thickThinSmallGap", "thinThickThinSmallGap",
            "thinThickMediumGap", "thickThinMediumGap", "thinThickThinMediumGap",
            "thinThickLargeGap", "thickThinLargeGap", "thinThickThinLargeGap",
            "wave", "doubleWave", "threeDEmboss", "threeDEngrave", "outset", "inset"] },
        "color": { "type": "string" } }, "additionalProperties": false }
] });

// ---------- 背景 ----------

/// 底纹名字：Word 和 Excel 都有的那些。
pub const PATTERNS: &[&str] = &[
    "pct5",
    "pct10",
    "pct20",
    "pct25",
    "pct30",
    "pct40",
    "pct50",
    "pct60",
    "pct70",
    "pct75",
    "pct80",
    "pct90",
    "horzStripe",
    "vertStripe",
    "diagStripe",
    "reverseDiagStripe",
    "horzCross",
    "diagCross",
    "thinHorzStripe",
    "thinVertStripe",
    "thinDiagStripe",
    "thinReverseDiagStripe",
    "thinHorzCross",
    "thinDiagCross",
];

#[derive(Debug, Clone, PartialEq)]
pub enum Background {
    Color(Color),
    /// 底纹：`pattern` 用 `color` 画在 `fill` 上
    Pattern {
        pattern: String,
        color: Color,
        fill: Color,
    },
    /// 线性渐变：角度（度），颜色均匀分布
    Gradient {
        angle: f32,
        stops: Vec<Color>,
    },
}

impl Background {
    fn from_value(v: &Value) -> Result<Background> {
        match v {
            Value::String(s) => Ok(Background::Color(Color::parse(s)?)),
            Value::Object(m) if m.contains_key("pattern") => {
                let pattern = m["pattern"].as_str().unwrap_or("").to_string();
                if !PATTERNS.contains(&pattern.as_str()) {
                    bail!("不认识的底纹 `{pattern}`，可选：{}", PATTERNS.join(" "));
                }
                for k in m.keys() {
                    if !matches!(k.as_str(), "pattern" | "color" | "fill") {
                        bail!("底纹只认 pattern、color、fill，没有 `{k}`");
                    }
                }
                Ok(Background::Pattern {
                    pattern,
                    color: m
                        .get("color")
                        .map(Color::from_value)
                        .transpose()?
                        .unwrap_or(Color("000000".into())),
                    fill: m
                        .get("fill")
                        .map(Color::from_value)
                        .transpose()?
                        .unwrap_or(Color("FFFFFF".into())),
                })
            }
            Value::Object(m) if m.contains_key("gradient") => {
                if m["gradient"] != "linear" {
                    bail!("渐变现在只支持 \"linear\"");
                }
                for k in m.keys() {
                    if !matches!(k.as_str(), "gradient" | "angle" | "stops") {
                        bail!("渐变只认 gradient、angle、stops，没有 `{k}`");
                    }
                }
                let stops = m
                    .get("stops")
                    .and_then(Value::as_array)
                    .ok_or_else(|| anyhow::anyhow!("渐变要有 stops 颜色数组"))?
                    .iter()
                    .map(Color::from_value)
                    .collect::<Result<Vec<_>>>()?;
                if stops.len() < 2 {
                    bail!("渐变至少要两个颜色");
                }
                Ok(Background::Gradient {
                    angle: m.get("angle").and_then(Value::as_f64).unwrap_or(90.0) as f32,
                    stops,
                })
            }
            _ => bail!(
                "background 写颜色、{{ pattern, color, fill }} 或 {{ gradient, angle, stops }}"
            ),
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Background::Color(c) => c.to_value(),
            Background::Pattern {
                pattern,
                color,
                fill,
            } => {
                serde_json::json!({ "pattern": pattern, "color": color.to_value(), "fill": fill.to_value() })
            }
            Background::Gradient { angle, stops } => serde_json::json!({
                "gradient": "linear", "angle": angle,
                "stops": stops.iter().map(Color::to_value).collect::<Vec<_>>()
            }),
        }
    }

    /// 只能用纯色的格式：底纹按深浅混色，渐变取第一个颜色。
    pub fn solid(&self) -> Color {
        match self {
            Background::Color(c) => c.clone(),
            Background::Pattern {
                pattern,
                color,
                fill,
            } => {
                let pct = pattern
                    .strip_prefix("pct")
                    .and_then(|x| x.parse::<f32>().ok())
                    .unwrap_or(25.0)
                    / 100.0;
                mix(color, fill, pct)
            }
            Background::Gradient { stops, .. } => stops[0].clone(),
        }
    }
}

/// 按比例 `t` 把 `a` 混进 `b`。
fn mix(a: &Color, b: &Color, t: f32) -> Color {
    let ch = |c: &Color, i: usize| u8::from_str_radix(&c.0[i..i + 2], 16).unwrap_or(0) as f32;
    let mut s = String::new();
    for i in [0, 2, 4] {
        let v = ch(a, i) * t + ch(b, i) * (1.0 - t);
        s.push_str(&format!("{:02X}", v.round() as u8));
    }
    Color(s)
}

value_type!(Background, "Background", { "anyOf": [
    { "type": "string" },
    { "type": "object", "properties": { "pattern": { "type": "string" }, "color": { "type": "string" }, "fill": { "type": "string" } }, "required": ["pattern"], "additionalProperties": false },
    { "type": "object", "properties": { "gradient": { "const": "linear" }, "angle": { "type": "number" }, "stops": { "type": "array", "items": { "type": "string" } } }, "required": ["gradient", "stops"], "additionalProperties": false }
] });

// ---------- 列表 ----------

/// 列表编号或符号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListStyleType {
    Disc,
    Circle,
    Square,
    Dash,
    None,
    Decimal,
    DecimalLeadingZero,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
    /// 一、二、三
    CjkDecimal,
    /// 甲、乙、丙
    CjkIdeographic,
    /// ①②③
    CircledDecimal,
    /// 任意字符当项目符号
    Char(String),
}

impl ListStyleType {
    const NAMES: &'static [(&'static str, ListStyleType)] = &[
        ("disc", ListStyleType::Disc),
        ("circle", ListStyleType::Circle),
        ("square", ListStyleType::Square),
        ("dash", ListStyleType::Dash),
        ("none", ListStyleType::None),
        ("decimal", ListStyleType::Decimal),
        ("decimal-leading-zero", ListStyleType::DecimalLeadingZero),
        ("lower-alpha", ListStyleType::LowerAlpha),
        ("upper-alpha", ListStyleType::UpperAlpha),
        ("lower-roman", ListStyleType::LowerRoman),
        ("upper-roman", ListStyleType::UpperRoman),
        ("cjk-decimal", ListStyleType::CjkDecimal),
        ("cjk-ideographic", ListStyleType::CjkIdeographic),
        ("circled-decimal", ListStyleType::CircledDecimal),
    ];

    fn from_value(v: &Value) -> Result<ListStyleType> {
        let s = v
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("listStyleType 要是字符串"))?;
        if let Some((_, t)) = Self::NAMES.iter().find(|(n, _)| *n == s) {
            return Ok(t.clone());
        }
        if !s.is_empty() && s.chars().count() <= 3 {
            return Ok(ListStyleType::Char(s.to_string()));
        }
        bail!(
            "不认识的 listStyleType `{s}`；可选 {}，或写 1 到 3 个字符当符号",
            Self::NAMES
                .iter()
                .map(|x| x.0)
                .collect::<Vec<_>>()
                .join(" ")
        )
    }

    fn to_value(&self) -> Value {
        match self {
            ListStyleType::Char(c) => c.clone().into(),
            t => Self::NAMES
                .iter()
                .find(|(_, x)| x == t)
                .map(|x| x.0)
                .unwrap_or("disc")
                .into(),
        }
    }

    pub fn ordered(&self) -> bool {
        !matches!(
            self,
            ListStyleType::Disc
                | ListStyleType::Circle
                | ListStyleType::Square
                | ListStyleType::Dash
                | ListStyleType::None
                | ListStyleType::Char(_)
        )
    }

    /// 符号类的字符。
    pub fn bullet(&self) -> Option<String> {
        Some(match self {
            ListStyleType::Disc => "•".into(),
            ListStyleType::Circle => "◦".into(),
            ListStyleType::Square => "▪".into(),
            ListStyleType::Dash => "–".into(),
            ListStyleType::None => "".into(),
            ListStyleType::Char(c) => c.clone(),
            _ => return None,
        })
    }

    /// 第 n 个（从 1 起）编号的文字。
    pub fn number(&self, n: u32) -> String {
        match self {
            ListStyleType::Decimal => n.to_string(),
            ListStyleType::DecimalLeadingZero => format!("{n:02}"),
            ListStyleType::LowerAlpha => alpha(n).to_lowercase(),
            ListStyleType::UpperAlpha => alpha(n),
            ListStyleType::LowerRoman => roman(n).to_lowercase(),
            ListStyleType::UpperRoman => roman(n),
            ListStyleType::CjkDecimal => cjk(n),
            ListStyleType::CjkIdeographic => {
                const G: [&str; 10] = ["甲", "乙", "丙", "丁", "戊", "己", "庚", "辛", "壬", "癸"];
                G[((n.max(1) - 1) % 10) as usize].into()
            }
            ListStyleType::CircledDecimal => match n {
                1..=20 => char::from_u32(0x2460 + n - 1).unwrap().to_string(),
                _ => format!("({n})"),
            },
            _ => self.bullet().unwrap_or_default(),
        }
    }
}

fn alpha(mut n: u32) -> String {
    let mut s = Vec::new();
    while n > 0 {
        n -= 1;
        s.push((b'A' + (n % 26) as u8) as char);
        n /= 26;
    }
    s.iter().rev().collect()
}

fn roman(mut n: u32) -> String {
    const T: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for (v, r) in T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

fn cjk(n: u32) -> String {
    const D: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    match n {
        0..=9 => D[n as usize].into(),
        10..=19 => format!("十{}", if n > 10 { D[(n % 10) as usize] } else { "" }),
        20..=99 => format!(
            "{}十{}",
            D[(n / 10) as usize],
            if !n.is_multiple_of(10) {
                D[(n % 10) as usize]
            } else {
                ""
            }
        ),
        _ => n.to_string(),
    }
}

value_type!(ListStyleType, "ListStyleType", { "type": "string" });

/// 编号和正文之间：长度，或 tab、space、none。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarkerGap {
    Len(Len),
    Tab,
    Space,
    None,
}

impl MarkerGap {
    fn from_value(v: &Value) -> Result<MarkerGap> {
        Ok(match v.as_str() {
            Some("tab") => MarkerGap::Tab,
            Some("space") => MarkerGap::Space,
            Some("none") => MarkerGap::None,
            _ => MarkerGap::Len(Len::from_value(v)?),
        })
    }
    fn to_value(self) -> Value {
        match self {
            MarkerGap::Len(l) => l.to_value(),
            MarkerGap::Tab => "tab".into(),
            MarkerGap::Space => "space".into(),
            MarkerGap::None => "none".into(),
        }
    }
}

value_type!(MarkerGap, "MarkerGap", { "anyOf": [{ "type": "number" }, { "type": "string" }] });

// ---------- 样式 ----------

/// 一组样式字段，全部可选。字段分组和各格式的对应见 docs/STYLE.md。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Style {
    // 文字
    /// 字体名，或 { latin, eastAsia, complex }
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<FontFamily>,
    /// 字号
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<Underline>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline_color: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strike: Option<Strike>,
    /// 文字：super、sub；盒子和图片：top、middle、bottom
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_align: Option<VerticalAlign>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caps: Option<Caps>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub letter_spacing: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<Color>,
    /// 着重号
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emphasis_mark: Option<EmphasisMark>,
    /// 字符缩放，百分比
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kerning: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,

    // 段落
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_before: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_after: Option<Len>,
    /// 倍数、固定值或 { atLeast }
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height: Option<LineHeight>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_left: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_right: Option<Len>,
    /// 首行缩进
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_indent: Option<Len>,
    /// 悬挂缩进
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hanging_indent: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_with_next: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_together: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_break_before: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub widow_control: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contextual_spacing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_space: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinsoku: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    /// 段落：要不要自动换行；图片：绕排方式
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap: Option<Wrap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 9))]
    pub outline_level: Option<u8>,

    // 边框
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_top: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_right: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_bottom: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_left: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_between: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_inside_h: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_inside_v: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_diagonal: Option<Border>,

    // 背景
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<Background>,

    // 盒子
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding_top: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding_right: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding_bottom: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding_left: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<Len>,
    /// 0、90、270
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_rotation: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<Len>,

    // 列表
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_style_type: Option<ListStyleType>,
    /// 编号模板，%1 是本级编号
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_marker_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_marker_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_marker_color: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_marker_font: Option<String>,
    /// 相对正文的百分比
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_marker_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_marker_align: Option<HAlign>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_marker_gap: Option<MarkerGap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_indent: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_hanging: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_start: Option<u32>,

    // 图片位置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub float: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap_distance: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_to: Option<AnchorTo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizontal_align: Option<HAlign>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_fit: Option<ObjectFit>,
    /// 角度
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub opacity: Option<f32>,

    // 表格
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_layout: Option<TableLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_padding: Option<Len>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_header: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_break: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band_size: Option<u32>,
}

/// 对字段列表做同一件事。第一组会往子元素继承（文字和段落、列表），第二组不继承。
macro_rules! with_fields {
    ($m:ident) => {
        $m!(
            inherit: font_family, font_size, color, bold, italic, underline, underline_color, strike,
            caps, letter_spacing, highlight, emphasis_mark, scale, kerning, lang, hidden,
            align, line_height, text_indent, hanging_indent, widow_control, contextual_spacing,
            auto_space, kinsoku, direction,
            list_style_type, list_marker_format, list_marker_image, list_marker_color, list_marker_font,
            list_marker_size, list_marker_align, list_marker_gap, list_indent, list_hanging;
            local: vertical_align, space_before, space_after, margin_left, margin_right, keep_with_next,
            keep_together, page_break_before, wrap, outline_level,
            border, border_top, border_right, border_bottom, border_left, border_between,
            border_inside_h, border_inside_v, border_diagonal, background,
            padding, padding_top, padding_right, padding_bottom, padding_left, width, height,
            text_rotation, radius, list_start,
            position, float, wrap_distance, anchor_to, x, y, horizontal_align, object_fit, rotation,
            opacity, table_layout, cell_padding, repeat_header, row_break, band_size
        )
    };
}

/// 文字字段：runs 上只能写这些。
const TEXT_FIELDS: &[&str] = &[
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
    "background",
];

/// 会继承的字段：样式集的 global 里只能写这些。
const INHERITED_FIELDS: &[&str] = &[
    "fontFamily",
    "fontSize",
    "color",
    "bold",
    "italic",
    "underline",
    "underlineColor",
    "strike",
    "caps",
    "letterSpacing",
    "highlight",
    "emphasisMark",
    "scale",
    "kerning",
    "lang",
    "hidden",
    "align",
    "lineHeight",
    "textIndent",
    "hangingIndent",
    "widowControl",
    "contextualSpacing",
    "autoSpace",
    "kinsoku",
    "direction",
    "listStyleType",
    "listMarkerFormat",
    "listMarkerImage",
    "listMarkerColor",
    "listMarkerFont",
    "listMarkerSize",
    "listMarkerAlign",
    "listMarkerGap",
    "listIndent",
    "listHanging",
];

impl Style {
    /// 用 `o` 里有值的字段覆盖自己；字体按文种合并。
    pub fn merge(&mut self, o: &Style) {
        macro_rules! go {
            (inherit: $($a:ident),*; local: $($b:ident),*) => {
                $( if o.$a.is_some() { self.$a = o.$a.clone(); } )*
                $( if o.$b.is_some() { self.$b = o.$b.clone(); } )*
            };
        }
        let fonts = match (&self.font_family, &o.font_family) {
            (Some(a), Some(b)) => {
                let mut f = a.clone();
                f.merge(b);
                Some(f)
            }
            _ => None,
        };
        with_fields!(go);
        if fonts.is_some() {
            self.font_family = fonts;
        }
    }

    /// 只留会继承的字段，给子元素当起点。
    pub fn inherited(&self) -> Style {
        let mut s = Style::default();
        macro_rules! go {
            (inherit: $($a:ident),*; local: $($b:ident),*) => {
                $( s.$a = self.$a.clone(); )*
            };
        }
        with_fields!(go);
        s
    }

    /// 只留文字字段（runs 上能写的那些，含 background）。
    pub fn text_only(&self) -> Style {
        Style {
            font_family: self.font_family.clone(),
            font_size: self.font_size,
            color: self.color.clone(),
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            underline_color: self.underline_color.clone(),
            strike: self.strike,
            vertical_align: self
                .vertical_align
                .filter(|v| matches!(v, VerticalAlign::Super | VerticalAlign::Sub)),
            caps: self.caps,
            letter_spacing: self.letter_spacing,
            highlight: self.highlight.clone(),
            emphasis_mark: self.emphasis_mark,
            scale: self.scale,
            kerning: self.kerning,
            lang: self.lang.clone(),
            hidden: self.hidden,
            background: self.background.clone(),
            ..Style::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Style::default()
    }

    /// run 原来按 `old` 算的样式，换到 `new` 这个环境里（比如放进表头格子）：
    /// 先用 `new` 的文字字段，run 自己改过的（和 `old` 不同的）再盖上去。
    pub fn rebase(&self, old: &Style, new: &Style) -> Style {
        let mut old = old.text_only();
        old.background = None;
        let mut out = new.text_only();
        out.background = None;
        macro_rules! go {
            ($($f:ident),*) => { $( if self.$f != old.$f { out.$f = self.$f.clone(); } )* };
        }
        go!(
            font_family,
            font_size,
            color,
            bold,
            italic,
            underline,
            underline_color,
            strike,
            vertical_align,
            caps,
            letter_spacing,
            highlight,
            emphasis_mark,
            scale,
            kerning,
            lang,
            hidden,
            background
        );
        out
    }

    /// 字号，pt。
    pub fn size_pt(&self) -> f32 {
        self.font_size
            .map(|l| l.pt(DEFAULT_FONT_SIZE, DEFAULT_FONT_SIZE))
            .unwrap_or(DEFAULT_FONT_SIZE)
    }

    /// 某一边的边框：单边覆盖 `border`。
    pub fn border_side(&self, side: Edge) -> Option<&Border> {
        let one = match side {
            Edge::Top => &self.border_top,
            Edge::Right => &self.border_right,
            Edge::Bottom => &self.border_bottom,
            Edge::Left => &self.border_left,
        };
        one.as_ref().or(self.border.as_ref())
    }

    /// 某一边的内边距，pt。
    pub fn padding_side(&self, side: Edge) -> Option<f32> {
        let one = match side {
            Edge::Top => self.padding_top,
            Edge::Right => self.padding_right,
            Edge::Bottom => self.padding_bottom,
            Edge::Left => self.padding_left,
        };
        one.or(self.padding).map(|l| l.pt(self.size_pt(), 0.0))
    }

    pub fn has_box(&self) -> bool {
        self.background.is_some()
            || Edge::ALL.iter().any(|e| self.border_side(*e).is_some())
            || Edge::ALL.iter().any(|e| self.padding_side(*e).is_some())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Right,
    Bottom,
    Left,
}

impl Edge {
    pub const ALL: [Edge; 4] = [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left];
}

/// 解析一个样式对象。
pub fn parse_object(v: &Value) -> Result<Style> {
    Ok(serde_json::from_value(v.clone())?)
}

/// 检查样式对象里只有 `allowed` 里的字段。
pub fn check_fields(v: &Value, allowed: &[&str], what: &str) -> Result<()> {
    if let Value::Object(m) = v {
        for k in m.keys() {
            if !allowed.contains(&k.as_str()) {
                bail!("{what}里不能写 `{k}`，只能写：{}", allowed.join(" "));
            }
        }
    }
    Ok(())
}

pub fn check_text_fields(v: &Value, what: &str) -> Result<()> {
    check_fields(v, TEXT_FIELDS, what)
}

pub fn check_inherited_fields(v: &Value, what: &str) -> Result<()> {
    check_fields(v, INHERITED_FIELDS, what)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_values() {
        let s: Style = serde_json::from_value(json!({
            "fontFamily": { "latin": "Inter", "eastAsia": "思源黑体" },
            "fontSize": "12pt",
            "textIndent": "2em",
            "spaceBefore": "0.5line",
            "lineHeight": { "atLeast": 18 },
            "border": "0.5pt dashed #ccc",
            "borderBottom": { "width": 2, "style": "double", "color": "#1A73E8" },
            "background": { "pattern": "pct20", "color": "#000000", "fill": "#FFFFFF" },
            "listStyleType": "cjk-decimal",
            "underline": "wavy",
            "wrap": "square"
        }))
        .unwrap();
        assert_eq!(s.text_indent, Some(Len::Em(2.0)));
        assert_eq!(s.space_before, Some(Len::Line(0.5)));
        assert_eq!(s.border.as_ref().unwrap().style, BorderStyle::Dashed);
        assert_eq!(s.border.as_ref().unwrap().color.rgb(), "CCCCCC");
        assert_eq!(s.background.as_ref().unwrap().solid().rgb(), "CCCCCC");
        assert_eq!(s.list_style_type.as_ref().unwrap().number(12), "十二");
        assert!(serde_json::from_value::<Style>(json!({ "color": "red" })).is_err());
        assert!(serde_json::from_value::<Style>(json!({ "nope": 1 })).is_err());
    }

    #[test]
    fn merges_fonts_by_script() {
        let mut a: Style =
            serde_json::from_value(json!({ "fontFamily": "Times New Roman" })).unwrap();
        a.merge(&serde_json::from_value(json!({ "fontFamily": { "eastAsia": "黑体" } })).unwrap());
        let f = a.font_family.unwrap();
        assert_eq!(f.latin.as_deref(), Some("Times New Roman"));
        assert_eq!(f.east_asia.as_deref(), Some("黑体"));
    }

    #[test]
    fn numbering() {
        assert_eq!(ListStyleType::UpperRoman.number(14), "XIV");
        assert_eq!(ListStyleType::LowerAlpha.number(28), "ab");
        assert_eq!(ListStyleType::CircledDecimal.number(3), "③");
    }
}
