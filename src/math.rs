//! 公式：LaTeX 先用 math-core 转成 MathML（html 直接用），再把 MathML 读成一棵小树，
//! 从树出 OMML（docx）和 Typst 公式（pdf）。三个格式走同一个解析器，认得的写法一样多。

use std::fmt::Write;

use math_core::{LatexToMathML, MathCoreConfig, MathDisplay};

/// LaTeX 转 MathML，返回 `<math>…</math>`（块级带 `display="block"`）。
pub fn to_mathml(latex: &str, block: bool) -> Result<String, String> {
    let conv =
        LatexToMathML::new(MathCoreConfig::default(), Vec::new()).map_err(|e| e.0.to_string())?;
    let display = if block {
        MathDisplay::Block
    } else {
        MathDisplay::Inline
    };
    conv.convert_with_local_state(latex, display)
        .map(|r| r.mathml)
        .map_err(|e| e.to_string())
}

/// MathML 里的一个元素。mi、mn、mo、mtext 这类叶子的文字放在 `text`。
#[derive(Debug, Clone, Default)]
pub struct MNode {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub kids: Vec<MNode>,
    pub text: String,
}

impl MNode {
    fn attr(&self, k: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(a, _)| a == k)
            .map(|(_, v)| v.as_str())
    }

    /// 叶子（mi、mo……）里只有这一个字符时返回它。
    fn single(&self) -> Option<char> {
        let mut cs = self.text.chars();
        let c = cs.next()?;
        cs.next().is_none().then_some(c)
    }

    /// 去掉只起占位作用的外壳：只剩一个有内容的子元素的 mrow。
    fn core(&self) -> &MNode {
        if self.tag == "mrow" {
            let real: Vec<&MNode> = self.kids.iter().filter(|k| !k.is_blank()).collect();
            if real.len() == 1 {
                return real[0].core();
            }
        }
        self
    }

    /// 不占地方的东西：没有宽度的 mspace、看不见的运算符（函数应用、不可见乘号）。
    fn is_blank(&self) -> bool {
        match self.tag.as_str() {
            "mspace" => em(self.attr("width")).unwrap_or(0.0) == 0.0,
            "mo" => invisible(&self.text),
            _ => false,
        }
    }

    /// 所有文字连起来，退化成纯文字时用。
    pub fn all_text(&self) -> String {
        let mut s = self.text.clone();
        for k in &self.kids {
            s.push_str(&k.all_text());
        }
        s
    }
}

/// LaTeX 读成 MathML 树，根是 `<math>`。
pub fn parse(latex: &str, block: bool) -> Result<MNode, String> {
    let ml = to_mathml(latex, block)?;
    parse_mathml(&ml)
}

/// 读 math-core 出的 MathML。它会写 `&ApplyFunction;` 这类 HTML 实体，XML 解析器不认，先换掉。
pub fn parse_mathml(ml: &str) -> Result<MNode, String> {
    let fixed = entities(ml);
    let doc = roxmltree::Document::parse(&fixed).map_err(|e| format!("MathML 读不了：{e}"))?;
    Ok(read(doc.root_element()))
}

fn entities(s: &str) -> String {
    const MAP: &[(&str, &str)] = &[
        ("ApplyFunction", "\u{2061}"),
        ("af", "\u{2061}"),
        ("InvisibleTimes", "\u{2062}"),
        ("it", "\u{2062}"),
        ("InvisibleComma", "\u{2063}"),
        ("ic", "\u{2063}"),
        ("nbsp", "\u{a0}"),
        ("NoBreak", "\u{2060}"),
    ];
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let name_end = rest[1..]
            .find(|c: char| !c.is_ascii_alphanumeric())
            .map(|j| j + 1);
        match name_end {
            Some(j) if rest[j..].starts_with(';') => {
                let name = &rest[1..j];
                if let Some((_, v)) = MAP.iter().find(|(k, _)| *k == name) {
                    out.push_str(v);
                } else {
                    // amp、lt、gt、quot、apos 和数字实体留给 XML 解析器
                    out.push_str(&rest[..=j]);
                }
                rest = &rest[j + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn read(n: roxmltree::Node) -> MNode {
    let tag = n.tag_name().name().to_string();
    let attrs = n
        .attributes()
        .map(|a| (a.name().to_string(), a.value().to_string()))
        .collect();
    let leaf = matches!(tag.as_str(), "mi" | "mn" | "mo" | "mtext" | "ms");
    if leaf {
        // mtext 里可能还套着 <code>、<span>，只要文字
        let text: String = n
            .descendants()
            .filter(|d| d.is_text())
            .filter_map(|d| d.text())
            .collect();
        return MNode {
            tag,
            attrs,
            kids: Vec::new(),
            text,
        };
    }
    let kids = n.children().filter(|c| c.is_element()).map(read).collect();
    MNode {
        tag,
        attrs,
        kids,
        text: String::new(),
    }
}

/// U+2061–U+2064：函数应用、不可见乘号、不可见逗号、不可见加号。
fn invisible(s: &str) -> bool {
    s.chars()
        .all(|c| ('\u{2061}'..='\u{2064}').contains(&c) || c == '\u{200b}')
}

/// "0.16666667em" 这类宽度换成 em；ex 按半个 em 算。
fn em(w: Option<&str>) -> Option<f32> {
    let w = w?.trim();
    if let Some(x) = w.strip_suffix("em") {
        return x.trim().parse().ok();
    }
    if let Some(x) = w.strip_suffix("ex") {
        return x.trim().parse::<f32>().ok().map(|v| v / 2.0);
    }
    if let Some(x) = w.strip_suffix("pt") {
        return x.trim().parse::<f32>().ok().map(|v| v / 10.0);
    }
    w.parse().ok()
}

/// 大运算符：求和、连乘、积分这类，带上下限时要排成 n 元运算符。
const BIG_OPS: &str = "∑∏∐∫∬∭∮∯∰∱∲∳⋀⋁⋂⋃⨀⨁⨂⨄⨆⨌";

fn big_op(n: &MNode) -> Option<char> {
    let n = n.core();
    if n.tag != "mo" {
        return None;
    }
    n.single().filter(|c| BIG_OPS.contains(*c))
}

/// lim、max 这类多字母的函数名，带下限时下限放正下方。
fn func_name(n: &MNode) -> bool {
    let n = n.core();
    n.tag == "mi" && n.text.chars().count() > 1
}

const OPEN: &str = "([{⟨⌈⌊|‖⟦⦃";
const CLOSE: &str = ")]}⟩⌉⌋|‖⟧⦄";

/// 能自动拉伸的一对括号：`\left( … \right)`、pmatrix、binom 这类。
/// 返回左括号、里面的元素、右括号；看不见的一边是空字符串。
fn fence(n: &MNode) -> Option<(String, &[MNode], String)> {
    if n.tag != "mrow" || n.kids.len() < 2 {
        return None;
    }
    let first = &n.kids[0];
    let last = &n.kids[n.kids.len() - 1];
    if first.tag != "mo" || last.tag != "mo" {
        return None;
    }
    if first.attr("stretchy") == Some("false") || last.attr("stretchy") == Some("false") {
        return None;
    }
    let side = |m: &MNode, set: &str| -> Option<String> {
        if m.text.is_empty() || invisible(&m.text) {
            Some(String::new())
        } else {
            m.single().filter(|c| set.contains(*c)).map(String::from)
        }
    };
    let open = side(first, OPEN)?;
    let close = side(last, CLOSE)?;
    if open.is_empty() && close.is_empty() {
        return None;
    }
    Some((open, &n.kids[1..n.kids.len() - 1], close))
}

/// 上面或下面那个记号（mover、munder 的第二个子元素）是哪类。
enum Mark {
    /// 普通重音：帽子、波浪、点、箭头，值是组合字符
    Accent(char),
    /// 上划线、下划线
    Bar,
    /// 上、下大括号这类
    Group(char),
}

fn mark(over: &MNode, stretchy_default: bool) -> Option<Mark> {
    let m = over.core();
    if m.tag != "mo" {
        return None;
    }
    let c = m.single()?;
    let stretchy = m.attr("stretchy").map_or(stretchy_default, |v| v == "true");
    match c {
        '\u{332}' | '\u{305}' | '‾' | '_' | '\u{203e}' if stretchy => Some(Mark::Bar),
        '¯' if stretchy => Some(Mark::Bar),
        '⏞' | '⏟' | '⎴' | '⎵' | '⏜' | '⏝' | '⏠' | '⏡' => Some(Mark::Group(c)),
        _ => combining(c).map(Mark::Accent),
    }
}

/// 重音符号换成组合字符（OMML 的 m:acc 和 Typst 的 accent 都要这种）。
fn combining(c: char) -> Option<char> {
    Some(match c {
        'ˆ' | '^' | '\u{302}' => '\u{302}',
        '˜' | '~' | '\u{303}' => '\u{303}',
        '¯' | '\u{304}' | '\u{305}' | '‾' => '\u{304}',
        '˙' | '\u{307}' => '\u{307}',
        '¨' | '\u{308}' => '\u{308}',
        '´' | '\u{301}' => '\u{301}',
        '`' | '\u{300}' => '\u{300}',
        'ˇ' | '\u{30c}' => '\u{30c}',
        '˘' | '\u{306}' => '\u{306}',
        '˚' | '\u{30a}' => '\u{30a}',
        '⃗' | '→' => '\u{20d7}',
        '←' | '\u{20d6}' => '\u{20d6}',
        '↔' | '\u{20e1}' => '\u{20e1}',
        _ => return None,
    })
}

/// n 元运算符后面，遇到这些就不再算进被积（被求和）的部分。
fn stops_operand(n: &MNode) -> bool {
    n.tag == "mo"
        && n.single()
            .is_some_and(|c| "=<>≤≥≠≈≡∼≃≅∝→←⇒⇔↔⟹⟺+-−±∓,;:∈∉⊂⊃⊆⊇≪≫≺≻".contains(c))
}

/// mtable 的一格是不是靠左（cases、aligned 这类）。
fn cell_left(td: &MNode) -> bool {
    td.attr("style")
        .is_some_and(|s| s.contains("text-align: left") || s.contains("text-align:left"))
        || td.attr("columnalign") == Some("left")
}

fn rows(table: &MNode) -> Vec<Vec<&MNode>> {
    table
        .kids
        .iter()
        .filter(|r| r.tag == "mtr" || r.tag == "mlabeledtr")
        .map(|r| r.kids.iter().filter(|c| c.tag == "mtd").collect())
        .collect()
}

// ---------- OMML ----------

fn xml_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[derive(Clone, Copy)]
enum RunKind {
    /// 按 Word 公式的默认规则：字母斜体
    Math,
    /// 正体（函数名、\mathrm）
    Plain,
    /// 普通文字（\text），不用公式字体
    Text,
}

/// MathML 转 OMML。`rpr` 是要放进每个 m:r 的 `<w:rPr>…</w:rPr>`（字号、颜色）。
pub struct Omml<'a> {
    rpr: &'a str,
    /// 认不出、只能退成纯文字的写法
    pub unsupported: Vec<String>,
}

impl<'a> Omml<'a> {
    pub fn new(rpr: &'a str) -> Self {
        Omml {
            rpr,
            unsupported: Vec::new(),
        }
    }

    /// 行内公式：`<m:oMath>`。
    pub fn inline(&mut self, root: &MNode) -> String {
        format!("<m:oMath>{}</m:oMath>", self.seq(&root.kids))
    }

    /// 块级公式：`<m:oMathPara>`，`jc` 是 left、center、right。
    pub fn block(&mut self, root: &MNode, jc: &str) -> String {
        format!(
            "<m:oMathPara><m:oMathParaPr><m:jc m:val=\"{jc}\"/></m:oMathParaPr><m:oMath>{}</m:oMath></m:oMathPara>",
            self.seq(&root.kids)
        )
    }

    fn ctrl(&self) -> String {
        format!("<m:ctrlPr>{}</m:ctrlPr>", self.rpr)
    }

    fn run(&self, text: &str, kind: RunKind) -> String {
        if text.is_empty() {
            return String::new();
        }
        let mrpr = match kind {
            RunKind::Math => "",
            RunKind::Plain => "<m:rPr><m:sty m:val=\"p\"/></m:rPr>",
            RunKind::Text => "<m:rPr><m:nor/></m:rPr>",
        };
        format!(
            "<m:r>{mrpr}{}<m:t xml:space=\"preserve\">{}</m:t></m:r>",
            self.rpr,
            xml_esc(text)
        )
    }

    fn seq(&mut self, kids: &[MNode]) -> String {
        let mut out = String::new();
        let mut i = 0;
        while i < kids.len() {
            let k = &kids[i];
            if let Some(parts) = nary_parts(k) {
                let mut j = i + 1;
                while j < kids.len() && !stops_operand(&kids[j]) {
                    j += 1;
                }
                let e = self.seq(&kids[i + 1..j]);
                out.push_str(&self.nary(parts, &e));
                i = j;
                continue;
            }
            out.push_str(&self.node(k));
            i += 1;
        }
        out
    }

    fn nary(&mut self, p: Nary, e: &str) -> String {
        let mut pr = format!(
            "<m:chr m:val=\"{}\"/><m:limLoc m:val=\"{}\"/>",
            xml_esc(&p.op.to_string()),
            if p.limits { "undOvr" } else { "subSup" }
        );
        if p.sub.is_none() {
            pr.push_str("<m:subHide m:val=\"1\"/>");
        }
        if p.sup.is_none() {
            pr.push_str("<m:supHide m:val=\"1\"/>");
        }
        let sub = p.sub.map(|s| self.node(s)).unwrap_or_default();
        let sup = p.sup.map(|s| self.node(s)).unwrap_or_default();
        format!(
            "<m:nary><m:naryPr>{pr}{}</m:naryPr><m:sub>{sub}</m:sub><m:sup>{sup}</m:sup><m:e>{e}</m:e></m:nary>",
            self.ctrl()
        )
    }

    fn degrade(&mut self, n: &MNode) -> String {
        if !self.unsupported.contains(&n.tag) {
            self.unsupported.push(n.tag.clone());
        }
        self.run(&n.all_text(), RunKind::Math)
    }

    fn node(&mut self, n: &MNode) -> String {
        let kids = &n.kids;
        match n.tag.as_str() {
            "math" | "mstyle" | "mpadded" | "merror" => self.seq(kids),
            "semantics" => kids.first().map(|k| self.node(k)).unwrap_or_default(),
            "mrow" => match fence(n) {
                Some((open, inner, close)) => format!(
                    "<m:d><m:dPr><m:begChr m:val=\"{}\"/><m:endChr m:val=\"{}\"/>{}</m:dPr><m:e>{}</m:e></m:d>",
                    xml_esc(&open),
                    xml_esc(&close),
                    self.ctrl(),
                    self.seq(inner)
                ),
                None => self.seq(kids),
            },
            "mfenced" => {
                let open = n.attr("open").unwrap_or("(");
                let close = n.attr("close").unwrap_or(")");
                let sep = n.attr("separators").unwrap_or(",").trim();
                let es: String = kids
                    .iter()
                    .map(|k| format!("<m:e>{}</m:e>", self.node(k)))
                    .collect();
                let sep = sep
                    .chars()
                    .next()
                    .map(|c| format!("<m:sepChr m:val=\"{}\"/>", xml_esc(&c.to_string())))
                    .unwrap_or_default();
                format!(
                    "<m:d><m:dPr><m:begChr m:val=\"{}\"/>{sep}<m:endChr m:val=\"{}\"/>{}</m:dPr>{es}</m:d>",
                    xml_esc(open),
                    xml_esc(close),
                    self.ctrl()
                )
            }
            "mphantom" => format!(
                "<m:phant><m:phantPr><m:show m:val=\"0\"/>{}</m:phantPr><m:e>{}</m:e></m:phant>",
                self.ctrl(),
                self.seq(kids)
            ),
            "mi" => {
                let kind = if n.attr("mathvariant") == Some("normal") || n.text.chars().count() > 1
                {
                    RunKind::Plain
                } else {
                    RunKind::Math
                };
                self.run(&n.text, kind)
            }
            "mn" => self.run(&n.text, RunKind::Plain),
            "mo" => {
                if invisible(&n.text) {
                    String::new()
                } else {
                    self.run(&n.text, RunKind::Plain)
                }
            }
            "mtext" | "ms" => self.run(&n.text, RunKind::Text),
            "mspace" => {
                let w = em(n.attr("width")).unwrap_or(0.0);
                let sp = if w <= 0.0 {
                    ""
                } else if w >= 0.9 {
                    "\u{2003}"
                } else if w >= 0.25 {
                    "\u{2004}"
                } else {
                    "\u{2009}"
                };
                self.run(sp, RunKind::Plain)
            }
            "msup" | "msub" | "msubsup" => {
                if nary_parts(n).is_some() {
                    return self.seq(std::slice::from_ref(n));
                }
                let want = if n.tag == "msubsup" { 3 } else { 2 };
                if kids.len() != want {
                    return self.degrade(n);
                }
                let e = self.node(&kids[0]);
                let a = self.node(&kids[1]);
                let c = self.ctrl();
                match n.tag.as_str() {
                    "msup" => format!(
                        "<m:sSup><m:sSupPr>{c}</m:sSupPr><m:e>{e}</m:e><m:sup>{a}</m:sup></m:sSup>"
                    ),
                    "msub" => format!(
                        "<m:sSub><m:sSubPr>{c}</m:sSubPr><m:e>{e}</m:e><m:sub>{a}</m:sub></m:sSub>"
                    ),
                    _ => {
                        let b = self.node(&kids[2]);
                        format!(
                            "<m:sSubSup><m:sSubSupPr>{c}</m:sSubSupPr><m:e>{e}</m:e><m:sub>{a}</m:sub><m:sup>{b}</m:sup></m:sSubSup>"
                        )
                    }
                }
            }
            "mfrac" => {
                if kids.len() != 2 {
                    return self.degrade(n);
                }
                let ty = if no_bar(n) {
                    "<m:type m:val=\"noBar\"/>"
                } else {
                    ""
                };
                format!(
                    "<m:f><m:fPr>{ty}{}</m:fPr><m:num>{}</m:num><m:den>{}</m:den></m:f>",
                    self.ctrl(),
                    self.node(&kids[0]),
                    self.node(&kids[1])
                )
            }
            "msqrt" => format!(
                "<m:rad><m:radPr><m:degHide m:val=\"1\"/>{}</m:radPr><m:deg/><m:e>{}</m:e></m:rad>",
                self.ctrl(),
                self.seq(kids)
            ),
            "mroot" => {
                if kids.len() != 2 {
                    return self.degrade(n);
                }
                format!(
                    "<m:rad><m:radPr>{}</m:radPr><m:deg>{}</m:deg><m:e>{}</m:e></m:rad>",
                    self.ctrl(),
                    self.node(&kids[1]),
                    self.node(&kids[0])
                )
            }
            "munder" | "mover" | "munderover" => self.under_over(n),
            "mtable" => self.table(n),
            "menclose" => {
                let notation = n.attr("notation").unwrap_or("longdiv");
                if notation.contains("box") || notation.contains("roundedbox") {
                    format!(
                        "<m:borderBox><m:borderBoxPr>{}</m:borderBoxPr><m:e>{}</m:e></m:borderBox>",
                        self.ctrl(),
                        self.seq(kids)
                    )
                } else if notation.contains("strike") {
                    format!(
                        "<m:borderBox><m:borderBoxPr><m:hideTop m:val=\"1\"/><m:hideBot m:val=\"1\"/><m:hideLeft m:val=\"1\"/><m:hideRight m:val=\"1\"/><m:strikeBLTR m:val=\"1\"/>{}</m:borderBoxPr><m:e>{}</m:e></m:borderBox>",
                        self.ctrl(),
                        self.seq(kids)
                    )
                } else {
                    self.degrade(n)
                }
            }
            _ => self.degrade(n),
        }
    }

    fn under_over(&mut self, n: &MNode) -> String {
        let kids = &n.kids;
        let want = if n.tag == "munderover" { 3 } else { 2 };
        if kids.len() != want {
            return self.degrade(n);
        }
        if nary_parts(n).is_some() {
            return self.seq(std::slice::from_ref(n));
        }
        let c = self.ctrl();
        let base = self.node(&kids[0]);
        if n.tag != "munderover" {
            let top = n.tag == "mover";
            let accent_attr = if top { "accent" } else { "accentunder" };
            let accent = n.attr(accent_attr) == Some("true");
            match mark(&kids[1], false) {
                Some(Mark::Bar) => {
                    return format!(
                        "<m:bar><m:barPr><m:pos m:val=\"{}\"/>{c}</m:barPr><m:e>{base}</m:e></m:bar>",
                        if top { "top" } else { "bot" }
                    );
                }
                Some(Mark::Group(ch)) => {
                    return format!(
                        "<m:groupChr><m:groupChrPr><m:chr m:val=\"{}\"/><m:pos m:val=\"{}\"/><m:vertJc m:val=\"{}\"/>{c}</m:groupChrPr><m:e>{base}</m:e></m:groupChr>",
                        xml_esc(&ch.to_string()),
                        if top { "top" } else { "bot" },
                        if top { "bot" } else { "top" }
                    );
                }
                Some(Mark::Accent(ch)) if top && accent => {
                    return format!(
                        "<m:acc><m:accPr><m:chr m:val=\"{}\"/>{c}</m:accPr><m:e>{base}</m:e></m:acc>",
                        xml_esc(&ch.to_string())
                    );
                }
                _ => {}
            }
            let lim = self.node(&kids[1]);
            let el = if top { "limUpp" } else { "limLow" };
            return format!(
                "<m:{el}><m:{el}Pr>{c}</m:{el}Pr><m:e>{base}</m:e><m:lim>{lim}</m:lim></m:{el}>"
            );
        }
        let low = self.node(&kids[1]);
        let up = self.node(&kids[2]);
        format!(
            "<m:limUpp><m:limUppPr>{c}</m:limUppPr><m:e><m:limLow><m:limLowPr>{c}</m:limLowPr><m:e>{base}</m:e><m:lim>{low}</m:lim></m:limLow></m:e><m:lim>{up}</m:lim></m:limUpp>"
        )
    }

    fn table(&mut self, n: &MNode) -> String {
        let rs = rows(n);
        let cols = rs.iter().map(Vec::len).max().unwrap_or(0).max(1);
        let left = rs.iter().flatten().next().is_some_and(|td| cell_left(td));
        let mut out = format!(
            "<m:m><m:mPr><m:mcs><m:mc><m:mcPr><m:count m:val=\"{cols}\"/><m:mcJc m:val=\"{}\"/></m:mcPr></m:mc></m:mcs>{}</m:mPr>",
            if left { "left" } else { "center" },
            self.ctrl()
        );
        for r in rs {
            out.push_str("<m:mr>");
            for i in 0..cols {
                let cell = r.get(i).map(|td| self.seq(&td.kids)).unwrap_or_default();
                let _ = write!(out, "<m:e>{cell}</m:e>");
            }
            out.push_str("</m:mr>");
        }
        out.push_str("</m:m>");
        out
    }
}

/// 一个 n 元运算符（求和、积分……）的各部分。
struct Nary<'a> {
    op: char,
    sub: Option<&'a MNode>,
    sup: Option<&'a MNode>,
    /// 上下限放在正上正下（求和），否则放在右边（积分）
    limits: bool,
}

fn nary_parts(n: &MNode) -> Option<Nary<'_>> {
    let k = &n.kids;
    let (base, sub, sup, limits) = match (n.tag.as_str(), k.len()) {
        ("mo", _) => (n, None, None, false),
        ("msub", 2) => (&k[0], Some(&k[1]), None, false),
        ("msup", 2) => (&k[0], None, Some(&k[1]), false),
        ("msubsup", 3) => (&k[0], Some(&k[1]), Some(&k[2]), false),
        ("munder", 2) => (&k[0], Some(&k[1]), None, true),
        ("mover", 2) => (&k[0], None, Some(&k[1]), true),
        ("munderover", 3) => (&k[0], Some(&k[1]), Some(&k[2]), true),
        _ => return None,
    };
    let op = big_op(base)?;
    Some(Nary {
        op,
        sub,
        sup,
        limits,
    })
}

// ---------- Typst ----------

/// MathML 转 Typst 公式源码（放进 `$…$` 里）。每个字符都写成 `\u{…}` 转义，
/// 这样相邻的字母不会拼成变量名，`->` 这类也不会被当成简写；文字用字符串，结构用函数。
pub struct Typst {
    pub unsupported: Vec<String>,
}

fn esc_chars(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let _ = write!(out, "\\u{{{:x}}}", c as u32);
    }
    out
}

fn typ_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            // 字符串首尾的空格排版时会被吃掉，换成不换行空格
            ' ' => out.push('\u{a0}'),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn arg(s: String) -> String {
    if s.trim().is_empty() {
        "\"\"".into()
    } else {
        s
    }
}

impl Default for Typst {
    fn default() -> Self {
        Self::new()
    }
}

impl Typst {
    pub fn new() -> Self {
        Typst {
            unsupported: Vec::new(),
        }
    }

    /// 整个公式（`<math>` 的内容）。
    pub fn math(&mut self, root: &MNode) -> String {
        self.seq(&root.kids)
    }

    fn seq(&mut self, kids: &[MNode]) -> String {
        kids.iter().map(|k| self.node(k)).collect()
    }

    fn degrade(&mut self, n: &MNode) -> String {
        if !self.unsupported.contains(&n.tag) {
            self.unsupported.push(n.tag.clone());
        }
        typ_str(&n.all_text())
    }

    fn node(&mut self, n: &MNode) -> String {
        let kids = &n.kids;
        match n.tag.as_str() {
            "math" | "mstyle" | "mpadded" | "merror" => self.seq(kids),
            "semantics" => kids.first().map(|k| self.node(k)).unwrap_or_default(),
            "mrow" => match fence(n) {
                Some((open, inner, close)) => self.fenced(&open, inner, &close),
                None => self.seq(kids),
            },
            "mfenced" => {
                let open = n.attr("open").unwrap_or("(");
                let close = n.attr("close").unwrap_or(")");
                let sep = n.attr("separators").unwrap_or(",").trim().to_string();
                let sep = esc_chars(&sep.chars().next().map(String::from).unwrap_or_default());
                let parts: Vec<String> = kids.iter().map(|k| self.node(k)).collect();
                format!(
                    "lr({}{}{})",
                    esc_chars(open),
                    parts.join(&sep),
                    esc_chars(close)
                )
            }
            "mphantom" => format!("hide({})", arg(self.seq(kids))),
            "mi" => {
                if n.text.chars().count() > 1 {
                    format!("op({})", typ_str(&n.text))
                } else if n.attr("mathvariant") == Some("normal") {
                    format!("upright({})", arg(esc_chars(&n.text)))
                } else {
                    esc_chars(&n.text)
                }
            }
            "mn" | "mo" => {
                if invisible(&n.text) {
                    String::new()
                } else {
                    esc_chars(&n.text)
                }
            }
            "mtext" | "ms" => {
                if n.text.is_empty() {
                    String::new()
                } else {
                    typ_str(&n.text)
                }
            }
            "mspace" => match em(n.attr("width")) {
                Some(w) if w != 0.0 => format!("#h({w}em)"),
                _ => String::new(),
            },
            "msup" | "msub" | "msubsup" => {
                let want = if n.tag == "msubsup" { 3 } else { 2 };
                if kids.len() != want {
                    return self.degrade(n);
                }
                let mut base = arg(self.node(&kids[0]));
                if big_op(&kids[0]).is_some() {
                    base = format!("scripts({base})");
                }
                let a = arg(self.node(&kids[1]));
                match n.tag.as_str() {
                    "msup" => format!("attach({base}, t: {a})"),
                    "msub" => format!("attach({base}, b: {a})"),
                    _ => {
                        let b = arg(self.node(&kids[2]));
                        format!("attach({base}, b: {a}, t: {b})")
                    }
                }
            }
            "mfrac" => {
                if kids.len() != 2 {
                    return self.degrade(n);
                }
                let a = arg(self.node(&kids[0]));
                let b = arg(self.node(&kids[1]));
                if no_bar(n) {
                    format!("mat(delim: #none, {a}; {b})")
                } else {
                    format!("frac({a}, {b})")
                }
            }
            "msqrt" => format!("sqrt({})", arg(self.seq(kids))),
            "mroot" => {
                if kids.len() != 2 {
                    return self.degrade(n);
                }
                let b = arg(self.node(&kids[0]));
                let i = arg(self.node(&kids[1]));
                format!("root({i}, {b})")
            }
            "munder" | "mover" | "munderover" => self.under_over(n),
            "mtable" => self.table(n, "#none"),
            "menclose" => {
                let notation = n.attr("notation").unwrap_or("");
                let body = arg(self.seq(kids));
                if notation.contains("strike") {
                    format!("cancel({body})")
                } else if notation.contains("box") {
                    format!("#box(stroke: 0.5pt, inset: 2pt, ${body}$)")
                } else {
                    self.degrade(n)
                }
            }
            _ => self.degrade(n),
        }
    }

    /// 一对能拉伸的括号。括号里正好是矩阵时写成 mat 的 delim，cases 写成 cases，
    /// binom（不带横线的分数）写成 binom，其他用 lr。
    fn fenced(&mut self, open: &str, inner: &[MNode], close: &str) -> String {
        let real: Vec<&MNode> = inner.iter().filter(|k| !k.is_blank()).collect();
        if let [only] = real.as_slice() {
            let only = only.core();
            if only.tag == "mtable" {
                if open == "{" && close.is_empty() {
                    return self.cases(only);
                }
                let pair = matches!(
                    (open, close),
                    ("(", ")") | ("[", "]") | ("{", "}") | ("|", "|") | ("‖", "‖") | ("⟨", "⟩")
                );
                if pair {
                    let d = if open == "‖" { "||" } else { open };
                    return self.table(only, &typ_str(d));
                }
            }
            if only.tag == "mfrac" && no_bar(only) && open == "(" && close == ")" {
                let a = arg(self.node(&only.kids[0]));
                let b = arg(self.node(&only.kids[1]));
                return format!("binom({a}, {b})");
            }
        }
        format!(
            "lr({}{}{})",
            esc_chars(open),
            arg(self.seq(inner)),
            esc_chars(close)
        )
    }

    fn cases(&mut self, table: &MNode) -> String {
        let mut out = Vec::new();
        for r in rows(table) {
            // typst 的 & 是右对齐、左对齐轮着来：每格前面写一个 &（空的右对齐列）让格子都靠左，
            // 格子之间再空 1em，和 LaTeX 的 cases 一样
            let cells: Vec<String> = r.iter().map(|td| self.seq(&td.kids)).collect();
            let row: Vec<String> = cells
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    if i == 0 {
                        format!("&{c}")
                    } else {
                        format!("&&#h(1em){c}")
                    }
                })
                .collect();
            out.push(row.concat());
        }
        format!("cases({})", out.join(", "))
    }

    fn table(&mut self, table: &MNode, delim: &str) -> String {
        let rs = rows(table);
        let cols = rs.iter().map(Vec::len).max().unwrap_or(0).max(1);
        let left = rs.iter().flatten().next().is_some_and(|td| cell_left(td));
        let mut lines = Vec::new();
        for r in rs {
            let cells: Vec<String> = (0..cols)
                .map(|i| arg(r.get(i).map(|td| self.seq(&td.kids)).unwrap_or_default()))
                .collect();
            lines.push(cells.join(", "));
        }
        let align = if left { "align: #left, " } else { "" };
        format!("mat(delim: {delim}, {align}{})", lines.join("; "))
    }

    fn under_over(&mut self, n: &MNode) -> String {
        let kids = &n.kids;
        let want = if n.tag == "munderover" { 3 } else { 2 };
        if kids.len() != want {
            return self.degrade(n);
        }
        let base_src = arg(self.node(&kids[0]));
        if n.tag != "munderover" {
            let top = n.tag == "mover";
            match mark(&kids[1], false) {
                Some(Mark::Bar) => {
                    return format!("{}({base_src})", if top { "overline" } else { "underline" });
                }
                Some(Mark::Group(c)) => {
                    let f = match c {
                        '⏞' => "overbrace",
                        '⏟' => "underbrace",
                        '⎴' => "overbracket",
                        '⎵' => "underbracket",
                        '⏜' => "overparen",
                        '⏝' => "underparen",
                        '⏠' => "overshell",
                        _ => "undershell",
                    };
                    return format!("{f}({base_src})");
                }
                Some(Mark::Accent(c)) if top => {
                    return format!("accent({base_src}, {})", esc_chars(&c.to_string()));
                }
                _ => {}
            }
        }
        let base = if big_op(&kids[0]).is_some() || func_name(&kids[0]) {
            format!("limits({base_src}, inline: #false)")
        } else {
            format!("limits({base_src})")
        };
        let a = arg(self.node(&kids[1]));
        match n.tag.as_str() {
            "munder" => format!("attach({base}, b: {a})"),
            "mover" => format!("attach({base}, t: {a})"),
            _ => {
                let b = arg(self.node(&kids[2]));
                format!("attach({base}, b: {a}, t: {b})")
            }
        }
    }
}

fn no_bar(n: &MNode) -> bool {
    n.attr("linethickness").is_some_and(|t| {
        t.trim()
            .trim_end_matches(char::is_alphabetic)
            .parse::<f32>()
            .is_ok_and(|v| v == 0.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn omml(latex: &str) -> String {
        let root = parse(latex, true).unwrap();
        Omml::new("").inline(&root)
    }

    fn typ(latex: &str) -> String {
        let root = parse(latex, true).unwrap();
        Typst::new().math(&root)
    }

    #[test]
    fn entities_replaced() {
        assert_eq!(entities("a&ApplyFunction;b&gt;c&"), "a\u{2061}b&gt;c&");
    }

    #[test]
    fn omml_structures() {
        assert!(omml(r"\frac{a}{b}").contains("<m:f>"));
        let s = omml(r"\sum_{i=1}^{n} i^2 = x");
        assert!(s.contains("<m:nary>"), "{s}");
        assert!(s.contains("m:val=\"∑\""), "{s}");
        assert!(s.contains("undOvr"), "{s}");
        // 等号不算进求和的部分
        assert!(s.contains("</m:e></m:nary><m:r>"), "{s}");
        let s = omml(r"\int_0^1 x\,dx");
        assert!(s.contains("subSup"), "{s}");
        assert!(omml(r"\sqrt{2}").contains("<m:degHide"));
        assert!(omml(r"\sqrt[3]{x}").contains("<m:deg><m:r>"));
        let s = omml(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}");
        assert!(s.contains("<m:d>") && s.contains("<m:m>"), "{s}");
        assert!(omml(r"\hat{x}").contains("<m:acc>"));
        assert!(omml(r"\overline{AB}").contains("<m:bar>"));
        assert!(omml(r"\lim_{n\to\infty} x").contains("<m:limLow>"));
        assert!(omml(r"\binom{n}{k}").contains("noBar"));
    }

    #[test]
    fn typst_structures() {
        assert_eq!(typ("x^2"), r"attach(\u{78}, t: \u{32})");
        assert!(typ(r"\frac{1}{2}").starts_with("frac("));
        assert!(
            typ(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}").starts_with("mat(delim: \"(\"")
        );
        assert!(typ(r"\begin{cases} 1 & x>0 \\ 0 & \text{else} \end{cases}").starts_with("cases("));
        assert!(typ(r"\binom{n}{k}").starts_with("binom("));
        assert!(typ(r"\sin x").contains("op(\"sin\")"));
    }
}
