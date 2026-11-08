//! Word 的样式表、主题字体、编号定义，以及 w:rPr、w:pPr 换成 fmtagno 样式字段。

use std::collections::HashMap;

use roxmltree::{Document, Node};

use crate::style::{
    Align, Background, Border, BorderStyle, Caps, Color, EmphasisMark, FontFamily, Len, LineHeight,
    PATTERNS, Strike, Style, Underline, UnderlineStyle, VerticalAlign,
};

use super::Warns;

pub const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub const M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
pub const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
pub const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
pub const V: &str = "urn:schemas-microsoft-com:vml";

/// 是不是 `ns` 命名空间里叫 `name` 的元素。
pub fn is(n: Node, ns: &str, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(ns)
}

pub fn isw(n: Node, name: &str) -> bool {
    is(n, W, name)
}

/// w: 命名空间的子元素。
pub fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| isw(*c, name))
}

pub fn child_ns<'a, 'i>(n: Node<'a, 'i>, ns: &str, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| is(*c, ns, name))
}

/// w:xxx 属性。
pub fn wa<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attribute((W, name))
}

pub fn wval<'a>(n: Node<'a, '_>) -> Option<&'a str> {
    wa(n, "val")
}

fn num(s: Option<&str>) -> Option<f32> {
    s.and_then(|x| x.trim().parse::<f32>().ok())
}

/// 开关属性：没写 val、写 true / 1 / on 都是开。
pub fn on_off(n: Node) -> bool {
    !matches!(wval(n), Some("0" | "false" | "off" | "none"))
}

/// 小数保留两位，免得 15.749999 这种数。
pub fn round2(x: f32) -> f32 {
    (x * 100.0).round() / 100.0
}

pub fn pt(x: f32) -> Len {
    Len::Pt(round2(x))
}

/// 把一个 pt 值写成 em（字号的倍数），能整除 0.25 em 时才用。
pub fn nice_em(v: f32, fs: f32) -> Len {
    if fs > 0.0 && v != 0.0 {
        let k = v / fs;
        let q = (k * 4.0).round() / 4.0;
        if (k - q).abs() < 0.01 && (q * fs * 20.0).round() == (v * 20.0).round() {
            return Len::Em(q);
        }
    }
    pt(v)
}

// ---------- 颜色、字体 ----------

pub fn color_attr(v: Option<&str>) -> Option<Color> {
    let v = v?.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("auto") {
        return None;
    }
    Color::parse(&format!("#{v}")).ok()
}

/// w:highlight 的 16 种颜色。
pub fn highlight_color(name: &str) -> Option<Color> {
    let hex = match name {
        "yellow" => "FFFF00",
        "green" => "00FF00",
        "cyan" => "00FFFF",
        "magenta" => "FF00FF",
        "blue" => "0000FF",
        "red" => "FF0000",
        "darkBlue" => "000080",
        "darkCyan" => "008080",
        "darkGreen" => "008000",
        "darkMagenta" => "800080",
        "darkRed" => "800000",
        "darkYellow" => "808000",
        "darkGray" => "808080",
        "lightGray" => "C0C0C0",
        "black" => "000000",
        "white" => "FFFFFF",
        _ => return None,
    };
    Color::parse(&format!("#{hex}")).ok()
}

/// 中文字体名换成英文族名。认识的换掉，不认识但带非 ASCII 字符的原样留着并给警告。
pub fn english_font(name: &str, warns: &mut Warns) -> String {
    let n = name.trim();
    const MAP: &[(&str, &str)] = &[
        ("宋体", "SimSun"),
        ("新宋体", "NSimSun"),
        ("黑体", "SimHei"),
        ("微软雅黑", "Microsoft YaHei"),
        ("微软雅黑 Light", "Microsoft YaHei Light"),
        ("楷体", "KaiTi"),
        ("楷体_GB2312", "KaiTi_GB2312"),
        ("仿宋", "FangSong"),
        ("仿宋_GB2312", "FangSong_GB2312"),
        ("等线", "DengXian"),
        ("等线 Light", "DengXian Light"),
        ("华文黑体", "STHeiti"),
        ("华文楷体", "STKaiti"),
        ("华文宋体", "STSong"),
        ("华文仿宋", "STFangsong"),
        ("华文中宋", "STZhongsong"),
        ("华文细黑", "STXihei"),
        ("华文行楷", "STXingkai"),
        ("华文新魏", "STXinwei"),
        ("华文隶书", "STLiti"),
        ("华文琥珀", "STHupo"),
        ("华文彩云", "STCaiyun"),
        ("幼圆", "YouYuan"),
        ("隶书", "LiSu"),
        ("方正舒体", "FZShuTi"),
        ("方正姚体", "FZYaoti"),
        ("方正小标宋简体", "FZXiaoBiaoSong-B05S"),
        ("方正小标宋_GBK", "FZXiaoBiaoSong-B05"),
        ("方正仿宋_GBK", "FZFangSong-Z02"),
        ("方正仿宋简体", "FZFangSong-Z02S"),
        ("方正黑体_GBK", "FZHei-B01"),
        ("方正黑体简体", "FZHei-B01S"),
        ("方正楷体_GBK", "FZKai-Z03"),
        ("方正楷体简体", "FZKai-Z03S"),
        ("方正书宋_GBK", "FZShuSong-Z01"),
        ("苹方", "PingFang SC"),
        ("苹方-简", "PingFang SC"),
        ("苹方-繁", "PingFang TC"),
        ("冬青黑体简体中文", "Hiragino Sans GB"),
        ("思源黑体", "Source Han Sans SC"),
        ("思源宋体", "Source Han Serif SC"),
        ("思源黑体 CN", "Source Han Sans CN"),
        ("思源宋体 CN", "Source Han Serif CN"),
        ("细明体", "MingLiU"),
        ("新细明体", "PMingLiU"),
        ("細明體", "MingLiU"),
        ("新細明體", "PMingLiU"),
        ("标楷体", "DFKai-SB"),
        ("標楷體", "DFKai-SB"),
        ("微軟正黑體", "Microsoft JhengHei"),
        ("微软正黑体", "Microsoft JhengHei"),
        ("ＭＳ 明朝", "MS Mincho"),
        ("ＭＳ ゴシック", "MS Gothic"),
        ("ＭＳ Ｐ明朝", "MS PMincho"),
        ("ＭＳ Ｐゴシック", "MS PGothic"),
        ("游ゴシック", "Yu Gothic"),
        ("游明朝", "Yu Mincho"),
        ("メイリオ", "Meiryo"),
        ("맑은 고딕", "Malgun Gothic"),
        ("바탕", "Batang"),
        ("굴림", "Gulim"),
        ("돋움", "Dotum"),
    ];
    if let Some((_, e)) = MAP.iter().find(|(c, _)| *c == n) {
        return e.to_string();
    }
    if !n.is_ascii() {
        warns.once(
            &format!("font:{n}"),
            format!("字体 `{n}` 没找到英文名，原样保留"),
        );
    }
    n.to_string()
}

/// 主题字体（theme1.xml 的 a:fontScheme）。
#[derive(Debug, Default, Clone)]
pub struct Theme {
    pub major_latin: Option<String>,
    pub major_ea: Option<String>,
    pub minor_latin: Option<String>,
    pub minor_ea: Option<String>,
}

impl Theme {
    pub fn parse(xml: &str) -> Theme {
        let mut t = Theme::default();
        let Ok(doc) = Document::parse(xml) else {
            return t;
        };
        let pick = |font: Node| -> (Option<String>, Option<String>) {
            let face = |n: Option<Node>| {
                n.and_then(|x| x.attribute("typeface"))
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            };
            let latin = face(child_ns(font, A, "latin"));
            let ea = face(child_ns(font, A, "ea")).or_else(|| {
                face(
                    font.children()
                        .find(|c| is(*c, A, "font") && c.attribute("script") == Some("Hans")),
                )
            });
            (latin, ea)
        };
        for n in doc.descendants() {
            if is(n, A, "majorFont") {
                (t.major_latin, t.major_ea) = pick(n);
            } else if is(n, A, "minorFont") {
                (t.minor_latin, t.minor_ea) = pick(n);
            }
        }
        t
    }

    fn resolve(&self, theme: &str) -> Option<String> {
        match theme {
            "majorHAnsi" | "majorAscii" | "majorBidi" => self.major_latin.clone(),
            "majorEastAsia" => self.major_ea.clone(),
            "minorHAnsi" | "minorAscii" | "minorBidi" => self.minor_latin.clone(),
            "minorEastAsia" => self.minor_ea.clone(),
            _ => None,
        }
    }
}

// ---------- 边框、底纹 ----------

/// Word 的线型名换成 fmtagno 的。艺术边框和不认识的按实线，给警告。
pub fn border_style(val: &str, warns: &mut Warns) -> BorderStyle {
    match val {
        "nil" | "none" => BorderStyle::None,
        "single" => BorderStyle::Solid,
        "dotDash" => BorderStyle::DashDot,
        "dotDotDash" => BorderStyle::DashDotDot,
        other => serde_json::from_value::<BorderStyle>(other.into()).unwrap_or_else(|_| {
            warns.once(
                &format!("border:{other}"),
                format!("边框线型 `{other}` 没有对应的写法（艺术边框不支持），按实线处理"),
            );
            BorderStyle::Solid
        }),
    }
}

/// 一条边：w:top 这类元素。返回边和 space（pt）。
pub fn border_of(n: Node, warns: &mut Warns) -> (Border, f32) {
    let style = border_style(wval(n).unwrap_or("single"), warns);
    if style == BorderStyle::None {
        return (Border::none(), 0.0);
    }
    let width = num(wa(n, "sz")).map(|s| s / 8.0).unwrap_or(0.5);
    let color = color_attr(wa(n, "color")).unwrap_or_else(|| Color::parse("#000000").unwrap());
    let space = num(wa(n, "space")).unwrap_or(0.0);
    (
        Border {
            width: round2(width),
            style,
            color,
        },
        space,
    )
}

/// w:shd 换成背景。
pub fn shading(n: Node) -> Option<Background> {
    let val = wval(n).unwrap_or("clear");
    let fill = color_attr(wa(n, "fill"));
    let color = color_attr(wa(n, "color"));
    match val {
        "nil" => None,
        "clear" => fill.map(Background::Color),
        "solid" => color
            .or(Some(Color::parse("#000000").unwrap()))
            .map(Background::Color),
        p if PATTERNS.contains(&p) => Some(Background::Pattern {
            pattern: p.to_string(),
            color: color.unwrap_or_else(|| Color::parse("#000000").unwrap()),
            fill: fill.unwrap_or_else(|| Color::parse("#FFFFFF").unwrap()),
        }),
        _ => fill.map(Background::Color),
    }
}

// ---------- w:rPr ----------

/// w:rPr 换成文字字段。
pub fn rpr(n: Node, theme: &Theme, warns: &mut Warns) -> Style {
    let mut s = Style::default();
    for c in n
        .children()
        .filter(|c| c.is_element() && c.tag_name().namespace() == Some(W))
    {
        match c.tag_name().name() {
            "rFonts" => {
                let get = |plain: &str, th: &str| -> Option<String> {
                    wa(c, th)
                        .and_then(|t| theme.resolve(t))
                        .or_else(|| wa(c, plain).map(str::to_string))
                };
                let latin = get("ascii", "asciiTheme").or_else(|| get("hAnsi", "hAnsiTheme"));
                let ea = get("eastAsia", "eastAsiaTheme");
                let cs = get("cs", "cstheme");
                let mut f = FontFamily {
                    latin: latin.map(|x| english_font(&x, warns)),
                    east_asia: ea.map(|x| english_font(&x, warns)),
                    complex: cs.map(|x| english_font(&x, warns)),
                };
                if f.latin.is_some() || f.east_asia.is_some() || f.complex.is_some() {
                    if let Some(prev) = &s.font_family {
                        let mut p = prev.clone();
                        p.merge(&f);
                        f = p;
                    }
                    s.font_family = Some(f);
                }
            }
            "sz" => {
                if let Some(v) = num(wval(c)) {
                    s.font_size = Some(pt(v / 2.0));
                }
            }
            "color" => {
                if let Some(col) = color_attr(wval(c)) {
                    s.color = Some(col);
                } else if wval(c) == Some("auto") {
                    s.color = Some(Color::parse("#000000").unwrap());
                }
            }
            "b" => s.bold = Some(on_off(c)),
            "i" => s.italic = Some(on_off(c)),
            "u" => {
                let st = match wval(c).unwrap_or("single") {
                    "none" => UnderlineStyle::None,
                    "double" => UnderlineStyle::Double,
                    "thick" => UnderlineStyle::Thick,
                    "dotted" | "dottedHeavy" => UnderlineStyle::Dotted,
                    "dash" | "dashedHeavy" | "dashLong" | "dashLongHeavy" | "dotDash"
                    | "dashDotHeavy" | "dotDotDash" | "dashDotDotHeavy" => UnderlineStyle::Dashed,
                    "wave" | "wavyHeavy" | "wavyDouble" => UnderlineStyle::Wavy,
                    _ => UnderlineStyle::Single,
                };
                s.underline = Some(Underline(st));
            }
            "strike" => {
                if on_off(c) {
                    s.strike = Some(Strike::Single);
                } else if s.strike != Some(Strike::Double) {
                    s.strike = Some(Strike::None);
                }
            }
            "dstrike" => {
                if on_off(c) {
                    s.strike = Some(Strike::Double);
                }
            }
            "vertAlign" => match wval(c) {
                Some("superscript") => s.vertical_align = Some(VerticalAlign::Super),
                Some("subscript") => s.vertical_align = Some(VerticalAlign::Sub),
                _ => {}
            },
            "caps" => {
                if on_off(c) {
                    s.caps = Some(Caps::All);
                }
            }
            "smallCaps" => {
                if on_off(c) {
                    s.caps = Some(Caps::Small);
                }
            }
            "spacing" => {
                if let Some(v) = num(wval(c)) {
                    s.letter_spacing = Some(pt(v / 20.0));
                }
            }
            "w" => {
                if let Some(v) = num(wval(c)) {
                    s.scale = Some(v);
                }
            }
            "kern" => {
                if let Some(v) = num(wval(c)) {
                    s.kerning = Some(v > 0.0);
                }
            }
            "lang" => {
                if let Some(v) = wval(c).or_else(|| wa(c, "eastAsia")) {
                    s.lang = Some(v.to_string());
                }
            }
            "vanish" => s.hidden = Some(on_off(c)),
            "shd" => {
                if let Some(b) = shading(c) {
                    s.background = Some(b);
                }
            }
            "highlight" => {
                if let Some(h) = wval(c).and_then(highlight_color) {
                    s.highlight = Some(h);
                }
            }
            "em" => {
                s.emphasis_mark = match wval(c) {
                    Some("dot") => Some(EmphasisMark::Dot),
                    Some("comma") => Some(EmphasisMark::Comma),
                    Some("circle") => Some(EmphasisMark::Circle),
                    Some("underDot") => Some(EmphasisMark::UnderDot),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    s
}

// ---------- w:pPr ----------

/// 段落属性的原始值，按属性逐个合并（样式链、编号、直接格式），最后再换成样式字段。
#[derive(Debug, Default, Clone)]
pub struct PPr {
    pub style: Option<String>,
    pub jc: Option<String>,
    pub before: Option<f32>,
    pub after: Option<f32>,
    pub before_lines: Option<f32>,
    pub after_lines: Option<f32>,
    pub line: Option<f32>,
    pub line_rule: Option<String>,
    pub ind: Ind,
    pub keep_next: Option<bool>,
    pub keep_lines: Option<bool>,
    pub page_break_before: Option<bool>,
    pub widow: Option<bool>,
    pub outline: Option<u8>,
    pub bidi: Option<bool>,
    pub shd: Option<Option<Background>>,
    /// 上右下左、段间
    pub borders: [Option<(Border, f32)>; 5],
    pub contextual: Option<bool>,
    /// (numId, ilvl)
    pub num_id: Option<i64>,
    pub ilvl: Option<usize>,
    /// 制表位：(类型, 位置 twip)
    pub tabs: Option<Vec<(String, f32)>>,
    /// 段落标记的文字样式
    pub mark: Option<Style>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Ind {
    pub left: Option<f32>,
    pub right: Option<f32>,
    pub first_line: Option<f32>,
    pub hanging: Option<f32>,
    pub left_chars: Option<f32>,
    pub right_chars: Option<f32>,
    pub first_line_chars: Option<f32>,
    pub hanging_chars: Option<f32>,
}

impl Ind {
    pub fn merge(&mut self, o: &Ind) {
        macro_rules! m {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f; } )* };
        }
        m!(left, right, left_chars, right_chars);
        // 首行缩进和悬挂缩进互斥：后写的一种盖掉前面的两种
        if o.first_line.is_some() || o.hanging.is_some() {
            self.first_line = o.first_line;
            self.hanging = o.hanging;
        }
        if o.first_line_chars.is_some() || o.hanging_chars.is_some() {
            self.first_line_chars = o.first_line_chars;
            self.hanging_chars = o.hanging_chars;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_none()
            && self.right.is_none()
            && self.first_line.is_none()
            && self.hanging.is_none()
            && self.left_chars.is_none()
            && self.right_chars.is_none()
            && self.first_line_chars.is_none()
            && self.hanging_chars.is_none()
    }

    pub fn parse(n: Node) -> Ind {
        let f = |a: &str| num(wa(n, a));
        Ind {
            left: f("left").or_else(|| f("start")),
            right: f("right").or_else(|| f("end")),
            first_line: f("firstLine"),
            hanging: f("hanging"),
            left_chars: f("leftChars").or_else(|| f("startChars")),
            right_chars: f("rightChars").or_else(|| f("endChars")),
            first_line_chars: f("firstLineChars"),
            hanging_chars: f("hangingChars"),
        }
    }

    /// 换成 marginLeft、marginRight、textIndent、hangingIndent。Word 的 left 含悬挂量，
    /// fmtagno 的 marginLeft 不含（见渲染器 para()）。按字符计的优先，写成 em。
    pub fn apply(&self, s: &mut Style) {
        let tw = |x: f32| pt(x / 20.0);
        let em = |x: f32| Len::Em(round2(x / 100.0));
        let hanging_chars = self.hanging_chars.filter(|x| *x > 0.0);
        let hanging = self.hanging.filter(|x| *x > 0.0);
        if let Some(h) = hanging_chars {
            s.hanging_indent = Some(em(h));
            s.text_indent = None;
            match self.left_chars {
                Some(l) if l - h != 0.0 => s.margin_left = Some(em(l - h)),
                Some(_) => s.margin_left = None,
                None => {
                    let l = self.left.unwrap_or(0.0) - hanging.unwrap_or(0.0);
                    s.margin_left = (l != 0.0).then(|| tw(l));
                }
            }
        } else if let Some(h) = hanging {
            s.hanging_indent = Some(tw(h));
            s.text_indent = None;
            let l = self.left.unwrap_or(0.0) - h;
            s.margin_left = match self.left_chars.filter(|x| *x != 0.0) {
                Some(lc) => Some(em(lc)),
                None => (l != 0.0).then(|| tw(l)),
            };
        } else {
            s.margin_left = match self.left_chars.filter(|x| *x != 0.0) {
                Some(lc) => Some(em(lc)),
                None => self.left.filter(|x| *x != 0.0).map(tw),
            };
            s.text_indent = match self.first_line_chars.filter(|x| *x != 0.0) {
                Some(fc) => Some(em(fc)),
                None => self.first_line.filter(|x| *x != 0.0).map(tw),
            };
        }
        s.margin_right = match self.right_chars.filter(|x| *x != 0.0) {
            Some(rc) => Some(em(rc)),
            None => self.right.filter(|x| *x != 0.0).map(tw),
        };
    }
}

impl PPr {
    pub fn parse(n: Node, theme: &Theme, warns: &mut Warns) -> PPr {
        let mut p = PPr::default();
        for c in n
            .children()
            .filter(|c| c.is_element() && c.tag_name().namespace() == Some(W))
        {
            match c.tag_name().name() {
                "pStyle" => p.style = wval(c).map(str::to_string),
                "jc" => p.jc = wval(c).map(str::to_string),
                "spacing" => {
                    p.before = num(wa(c, "before")).or(p.before);
                    p.after = num(wa(c, "after")).or(p.after);
                    p.before_lines = num(wa(c, "beforeLines")).or(p.before_lines);
                    p.after_lines = num(wa(c, "afterLines")).or(p.after_lines);
                    if let Some(l) = num(wa(c, "line")) {
                        p.line = Some(l);
                        p.line_rule = Some(wa(c, "lineRule").unwrap_or("auto").to_string());
                    }
                }
                "ind" => p.ind.merge(&Ind::parse(c)),
                "keepNext" => p.keep_next = Some(on_off(c)),
                "keepLines" => p.keep_lines = Some(on_off(c)),
                "pageBreakBefore" => p.page_break_before = Some(on_off(c)),
                "widowControl" => p.widow = Some(on_off(c)),
                "outlineLvl" => {
                    p.outline = num(wval(c)).map(|x| x as u8);
                }
                "bidi" => p.bidi = Some(on_off(c)),
                "shd" => p.shd = Some(shading(c)),
                "contextualSpacing" => p.contextual = Some(on_off(c)),
                "pBdr" => {
                    for b in c.children().filter(|b| b.is_element()) {
                        let i = match b.tag_name().name() {
                            "top" => 0,
                            "right" | "end" => 1,
                            "bottom" => 2,
                            "left" | "start" => 3,
                            "between" => 4,
                            _ => continue,
                        };
                        p.borders[i] = Some(border_of(b, warns));
                    }
                }
                "numPr" => {
                    if let Some(id) = child(c, "numId").and_then(|x| num(wval(x))) {
                        p.num_id = Some(id as i64);
                    }
                    if let Some(l) = child(c, "ilvl").and_then(|x| num(wval(x))) {
                        p.ilvl = Some(l as usize);
                    }
                }
                "tabs" => {
                    let v: Vec<(String, f32)> = c
                        .children()
                        .filter(|t| isw(*t, "tab"))
                        .filter_map(|t| Some((wval(t)?.to_string(), num(wa(t, "pos"))?)))
                        .collect();
                    p.tabs = Some(v);
                }
                "rPr" => p.mark = Some(rpr(c, theme, warns)),
                _ => {}
            }
        }
        p
    }

    /// 用 `o` 里写了的属性盖掉自己的。
    pub fn merge(&mut self, o: &PPr) {
        macro_rules! m {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f.clone(); } )* };
        }
        m!(
            style,
            jc,
            keep_next,
            keep_lines,
            page_break_before,
            widow,
            outline,
            bidi,
            shd,
            contextual,
            num_id,
            ilvl,
            tabs
        );
        // 段前段后：按行数的写法优先，所以写了磅值时把同一边的行数清掉
        if o.before.is_some() {
            self.before = o.before;
            if o.before_lines.is_none() {
                self.before_lines = None;
            }
        }
        if o.after.is_some() {
            self.after = o.after;
            if o.after_lines.is_none() {
                self.after_lines = None;
            }
        }
        if o.before_lines.is_some() {
            self.before_lines = o.before_lines;
        }
        if o.after_lines.is_some() {
            self.after_lines = o.after_lines;
        }
        if o.line.is_some() {
            self.line = o.line;
            self.line_rule = o.line_rule.clone();
        }
        self.ind.merge(&o.ind);
        for i in 0..5 {
            if o.borders[i].is_some() {
                self.borders[i] = o.borders[i].clone();
            }
        }
        if let Some(m) = &o.mark {
            let mut x = self.mark.clone().unwrap_or_default();
            x.merge(m);
            self.mark = Some(x);
        }
    }

    /// 换成段落字段。`fs` 是这段的字号，用来把行距写回倍数。
    pub fn to_style(&self, fs: f32) -> Style {
        let mut s = Style {
            align: self.jc.as_deref().and_then(|j| match j {
                "left" | "start" => Some(Align::Left),
                "center" => Some(Align::Center),
                "right" | "end" => Some(Align::Right),
                "both" | "lowKashida" | "mediumKashida" | "highKashida" => Some(Align::Justify),
                "distribute" | "thaiDistribute" => Some(Align::Distribute),
                _ => None,
            }),
            ..Style::default()
        };
        s.space_before = match (self.before_lines, self.before) {
            (Some(l), _) if l != 0.0 => Some(Len::Line(round2(l / 100.0))),
            (_, Some(b)) => Some(pt(b / 20.0)),
            (Some(_), None) => Some(Len::Pt(0.0)),
            _ => None,
        };
        s.space_after = match (self.after_lines, self.after) {
            (Some(l), _) if l != 0.0 => Some(Len::Line(round2(l / 100.0))),
            (_, Some(a)) => Some(pt(a / 20.0)),
            (Some(_), None) => Some(Len::Pt(0.0)),
            _ => None,
        };
        if let Some(line) = self.line {
            s.line_height = Some(match self.line_rule.as_deref() {
                Some("exact") => LineHeight::Exact(pt(line / 20.0)),
                Some("atLeast") => {
                    let v = line / 20.0;
                    // fmtagno 把倍数写成「至少 字号 × 倍数」，这里反过来认
                    let k = v / fs;
                    let q = (k * 20.0).round() / 20.0;
                    if fs > 0.0 && (k - q).abs() < 0.004 && ((fs * q) * 20.0).round() == line {
                        LineHeight::Multiple(round2(q))
                    } else {
                        LineHeight::AtLeast(pt(v))
                    }
                }
                _ => LineHeight::Multiple(round2(line / 240.0)),
            });
        }
        self.ind.apply(&mut s);
        s.keep_with_next = self.keep_next;
        s.keep_together = self.keep_lines;
        s.page_break_before = self.page_break_before;
        s.widow_control = self.widow;
        s.outline_level = self.outline.filter(|x| *x < 9).map(|x| x + 1);
        s.direction = match self.bidi {
            Some(true) => Some(crate::style::Direction::Rtl),
            _ => None,
        };
        if let Some(Some(b)) = &self.shd {
            s.background = Some(b.clone());
        }
        s.contextual_spacing = self.contextual;
        let slots = [
            (&mut s.border_top, &mut s.padding_top),
            (&mut s.border_right, &mut s.padding_right),
            (&mut s.border_bottom, &mut s.padding_bottom),
            (&mut s.border_left, &mut s.padding_left),
        ];
        for (i, (b, p)) in slots.into_iter().enumerate() {
            if let Some((bd, space)) = &self.borders[i] {
                *b = Some(bd.clone());
                if !bd.is_none() && *space > 0.0 {
                    *p = Some(pt(*space));
                }
            }
        }
        if let Some((bd, _)) = &self.borders[4] {
            s.border_between = Some(bd.clone());
        }
        s
    }
}

// ---------- 样式表 ----------

#[derive(Debug, Clone, Default)]
pub struct WStyle {
    pub id: String,
    pub name: String,
    pub based_on: Option<String>,
    pub ppr: PPr,
    pub rpr: Style,
    /// 表格样式的边框和格子边距（tblPr）
    pub tbl: Option<TblPr>,
    pub has_conditional: bool,
}

/// 表格级属性里 fmtagno 用得上的部分。
#[derive(Debug, Clone, Default)]
pub struct TblPr {
    /// 上右下左、内横、内竖
    pub borders: [Option<Border>; 6],
    /// 上右下左，twip
    pub cell_mar: [Option<f32>; 4],
}

impl TblPr {
    pub fn parse(n: Node, warns: &mut Warns) -> TblPr {
        let mut t = TblPr::default();
        if let Some(b) = child(n, "tblBorders") {
            for e in b.children().filter(|e| e.is_element()) {
                let i = match e.tag_name().name() {
                    "top" => 0,
                    "right" | "end" => 1,
                    "bottom" => 2,
                    "left" | "start" => 3,
                    "insideH" => 4,
                    "insideV" => 5,
                    _ => continue,
                };
                t.borders[i] = Some(border_of(e, warns).0);
            }
        }
        if let Some(m) = child(n, "tblCellMar") {
            t.cell_mar = margins(m);
        }
        t
    }

    pub fn merge(&mut self, o: &TblPr) {
        for i in 0..6 {
            if o.borders[i].is_some() {
                self.borders[i] = o.borders[i].clone();
            }
        }
        for i in 0..4 {
            if o.cell_mar[i].is_some() {
                self.cell_mar[i] = o.cell_mar[i];
            }
        }
    }
}

/// w:tblCellMar、w:tcMar：上右下左，twip（只认 dxa）。
pub fn margins(n: Node) -> [Option<f32>; 4] {
    let mut out = [None; 4];
    for e in n.children().filter(|e| e.is_element()) {
        let i = match e.tag_name().name() {
            "top" => 0,
            "right" | "end" => 1,
            "bottom" => 2,
            "left" | "start" => 3,
            _ => continue,
        };
        if matches!(wa(e, "type"), None | Some("dxa")) {
            out[i] = num(wa(e, "w"));
        }
    }
    out
}

#[derive(Debug, Default)]
pub struct Styles {
    pub default_ppr: PPr,
    pub default_rpr: Style,
    pub map: HashMap<String, WStyle>,
    pub default_para: Option<String>,
    pub default_table: Option<String>,
}

impl Styles {
    pub fn parse(xml: &str, theme: &Theme, warns: &mut Warns) -> Styles {
        let mut st = Styles::default();
        let Ok(doc) = Document::parse(xml) else {
            warns.push("styles.xml 读不了，样式表跳过".into());
            return st;
        };
        let root = doc.root_element();
        if let Some(dd) = child(root, "docDefaults") {
            if let Some(r) = child(dd, "rPrDefault").and_then(|x| child(x, "rPr")) {
                st.default_rpr = rpr(r, theme, warns);
            }
            if let Some(p) = child(dd, "pPrDefault").and_then(|x| child(x, "pPr")) {
                st.default_ppr = PPr::parse(p, theme, warns);
            }
        }
        for s in root.children().filter(|c| isw(*c, "style")) {
            let Some(id) = wa(s, "styleId") else { continue };
            let kind = wa(s, "type").unwrap_or("paragraph").to_string();
            let mut ws = WStyle {
                id: id.to_string(),
                name: child(s, "name").and_then(wval).unwrap_or(id).to_string(),
                based_on: child(s, "basedOn").and_then(wval).map(str::to_string),
                ..WStyle::default()
            };
            if let Some(p) = child(s, "pPr") {
                ws.ppr = PPr::parse(p, theme, warns);
            }
            if let Some(r) = child(s, "rPr") {
                ws.rpr = rpr(r, theme, warns);
            }
            if let Some(t) = child(s, "tblPr") {
                ws.tbl = Some(TblPr::parse(t, warns));
            }
            ws.has_conditional = s.children().any(|c| isw(c, "tblStylePr"));
            if matches!(wa(s, "default"), Some("1" | "true")) {
                match kind.as_str() {
                    "paragraph" => st.default_para = Some(id.to_string()),
                    "table" => st.default_table = Some(id.to_string()),
                    _ => {}
                }
            }
            st.map.insert(id.to_string(), ws);
        }
        st
    }

    /// 样式链，从最底层的父样式到自己。
    pub fn chain(&self, id: &str) -> Vec<&WStyle> {
        let mut out = Vec::new();
        let mut cur = Some(id.to_string());
        while let Some(c) = cur {
            let Some(s) = self.map.get(&c) else { break };
            if out.iter().any(|x: &&WStyle| x.id == s.id) || out.len() > 20 {
                break;
            }
            out.push(s);
            cur = s.based_on.clone();
        }
        out.reverse();
        out
    }

    /// 段落样式叠好后的段落属性和文字属性（不含 docDefaults）。
    pub fn para(&self, id: &str) -> (PPr, Style) {
        let mut p = PPr::default();
        let mut r = Style::default();
        for s in self.chain(id) {
            p.merge(&s.ppr);
            r.merge(&s.rpr);
        }
        p.style = None;
        (p, r)
    }

    /// 字符样式叠好后的文字属性。
    pub fn chars(&self, id: &str) -> Style {
        let mut r = Style::default();
        for s in self.chain(id) {
            r.merge(&s.rpr);
        }
        r
    }

    /// 表格样式叠好后的表格属性、段落属性、文字属性。
    pub fn table(&self, id: &str) -> (TblPr, PPr, Style, bool) {
        let mut t = TblPr::default();
        let mut p = PPr::default();
        let mut r = Style::default();
        let mut cond = false;
        for s in self.chain(id) {
            if let Some(x) = &s.tbl {
                t.merge(x);
            }
            p.merge(&s.ppr);
            r.merge(&s.rpr);
            cond |= s.has_conditional;
        }
        (t, p, r, cond)
    }

    /// 段落样式是几级标题：名字是 heading N，或样式链里写了 outlineLvl。
    pub fn heading_level(&self, id: &str) -> Option<u8> {
        let chain = self.chain(id);
        for s in chain.iter().rev() {
            let n = s.name.to_ascii_lowercase();
            if let Some(x) = n.strip_prefix("heading ")
                && let Ok(l) = x.trim().parse::<u8>()
                && (1..=9).contains(&l)
            {
                return Some(l.min(6));
            }
            if let Some(x) = s.id.strip_prefix("Heading")
                && let Ok(l) = x.parse::<u8>()
                && (1..=9).contains(&l)
            {
                return Some(l.min(6));
            }
        }
        let (p, _) = self.para(id);
        p.outline.filter(|o| *o < 6).map(|o| o + 1)
    }
}

// ---------- 编号 ----------

#[derive(Debug, Clone, Default)]
pub struct Lvl {
    pub start: Option<u32>,
    pub fmt: String,
    pub text: String,
    pub jc: Option<String>,
    pub ind: Ind,
    pub rpr: Style,
    pub suff: Option<String>,
    pub font: Option<String>,
}

#[derive(Debug, Default)]
pub struct Numbering {
    /// numId → (abstractNumId, 各级的起始编号覆盖)
    pub nums: HashMap<i64, (i64, HashMap<usize, u32>)>,
    pub abs: HashMap<i64, Vec<Lvl>>,
    /// abstractNum 的 numStyleLink → 样式名
    links: HashMap<i64, String>,
}

impl Numbering {
    pub fn parse(xml: &str, theme: &Theme, warns: &mut Warns) -> Numbering {
        let mut nb = Numbering::default();
        let Ok(doc) = Document::parse(xml) else {
            warns.push("numbering.xml 读不了，列表编号跳过".into());
            return nb;
        };
        let root = doc.root_element();
        for a in root.children().filter(|c| isw(*c, "abstractNum")) {
            let Some(id) = num(wa(a, "abstractNumId")) else {
                continue;
            };
            let id = id as i64;
            if let Some(l) = child(a, "numStyleLink").and_then(wval) {
                nb.links.insert(id, l.to_string());
            }
            let mut lvls = vec![Lvl::default(); 9];
            for l in a.children().filter(|c| isw(*c, "lvl")) {
                let Some(i) = num(wa(l, "ilvl")).map(|x| x as usize).filter(|x| *x < 9) else {
                    continue;
                };
                let mut v = Lvl {
                    start: child(l, "start")
                        .and_then(|x| num(wval(x)))
                        .map(|x| x as u32),
                    fmt: child(l, "numFmt")
                        .and_then(wval)
                        .unwrap_or("decimal")
                        .to_string(),
                    text: child(l, "lvlText").and_then(wval).unwrap_or("").to_string(),
                    jc: child(l, "lvlJc").and_then(wval).map(str::to_string),
                    suff: child(l, "suff").and_then(wval).map(str::to_string),
                    ..Lvl::default()
                };
                if let Some(ind) = child(l, "pPr").and_then(|p| child(p, "ind")) {
                    v.ind = Ind::parse(ind);
                }
                if let Some(r) = child(l, "rPr") {
                    v.font = child(r, "rFonts")
                        .and_then(|f| wa(f, "ascii").or_else(|| wa(f, "hAnsi")))
                        .map(str::to_string);
                    v.rpr = rpr(r, theme, warns);
                }
                lvls[i] = v;
            }
            nb.abs.insert(id, lvls);
        }
        for n in root.children().filter(|c| isw(*c, "num")) {
            let Some(id) = num(wa(n, "numId")) else {
                continue;
            };
            let Some(abs) = child(n, "abstractNumId").and_then(|x| num(wval(x))) else {
                continue;
            };
            let mut ov = HashMap::new();
            for o in n.children().filter(|c| isw(*c, "lvlOverride")) {
                if let (Some(l), Some(s)) = (
                    num(wa(o, "ilvl")),
                    child(o, "startOverride").and_then(|x| num(wval(x))),
                ) {
                    ov.insert(l as usize, s as u32);
                }
                if child(o, "lvl").is_some() {
                    warns.once(
                        "lvlOverride",
                        "编号里有 lvlOverride 改写整级定义，没解析，按原定义处理".into(),
                    );
                }
            }
            nb.nums.insert(id as i64, (abs as i64, ov));
        }
        nb
    }

    /// numId 指向的 abstractNum 和各级定义（已套上起始编号覆盖）。
    /// numStyleLink 要靠样式表找到真正的定义。
    pub fn levels(&self, num_id: i64, styles: &Styles) -> Option<(i64, Vec<Lvl>)> {
        let (mut abs, ov) = self.nums.get(&num_id)?.clone();
        for _ in 0..4 {
            let Some(link) = self.links.get(&abs) else {
                break;
            };
            let target = styles
                .chain(link)
                .iter()
                .rev()
                .find_map(|s| s.ppr.num_id)
                .and_then(|n| self.nums.get(&n))
                .map(|x| x.0);
            match target {
                Some(t) if t != abs => abs = t,
                _ => break,
            }
        }
        let mut lv = self.abs.get(&abs)?.clone();
        for (i, s) in ov {
            if let Some(l) = lv.get_mut(i) {
                l.start = Some(s);
            }
        }
        Some((abs, lv))
    }
}
