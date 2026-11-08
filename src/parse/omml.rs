//! OMML（Word 公式）转 LaTeX。认得的结构：分数、上下标、根号、n 元运算、括号、重音、上下划线、
//! 上下大括号、lim 这类下面带字的、函数名、矩阵、方程组、盒子。认不出的结构退成里面的文字，
//! 名字记进 `unsupported`，由调用方给警告。

use roxmltree::Node;

use super::word::{M, W, is};

pub struct Omml {
    pub unsupported: Vec<String>,
}

fn mval<'a>(n: Node<'a, '_>) -> Option<&'a str> {
    n.attribute((M, "val"))
}

fn mchild<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| is(*c, M, name))
}

/// xxxPr 里某个属性的 m:val。
fn prop<'a>(n: Node<'a, '_>, pr: &str, name: &str) -> Option<&'a str> {
    mchild(n, pr).and_then(|p| mchild(p, name)).and_then(mval)
}

fn on(v: Option<&str>) -> bool {
    matches!(v, Some("1" | "on" | "true"))
}

/// 函数名，写成 \sin 这类命令。
const FUNCS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh",
    "coth", "log", "ln", "lg", "exp", "lim", "liminf", "limsup", "max", "min", "sup", "inf", "det",
    "dim", "gcd", "ker", "deg", "arg", "hom", "Pr",
];

/// 单个字符对应的 LaTeX 命令。
fn symbol(c: char) -> Option<&'static str> {
    Some(match c {
        'α' => "\\alpha",
        'β' => "\\beta",
        'γ' => "\\gamma",
        'δ' => "\\delta",
        'ε' => "\\epsilon",
        'ϵ' => "\\epsilon",
        'ζ' => "\\zeta",
        'η' => "\\eta",
        'θ' => "\\theta",
        'ϑ' => "\\vartheta",
        'ι' => "\\iota",
        'κ' => "\\kappa",
        'λ' => "\\lambda",
        'μ' => "\\mu",
        'ν' => "\\nu",
        'ξ' => "\\xi",
        'π' => "\\pi",
        'ϖ' => "\\varpi",
        'ρ' => "\\rho",
        'ϱ' => "\\varrho",
        'σ' => "\\sigma",
        'ς' => "\\varsigma",
        'τ' => "\\tau",
        'υ' => "\\upsilon",
        'φ' => "\\phi",
        'ϕ' => "\\phi",
        'χ' => "\\chi",
        'ψ' => "\\psi",
        'ω' => "\\omega",
        'Γ' => "\\Gamma",
        'Δ' => "\\Delta",
        'Θ' => "\\Theta",
        'Λ' => "\\Lambda",
        'Ξ' => "\\Xi",
        'Π' => "\\Pi",
        'Σ' => "\\Sigma",
        'Υ' => "\\Upsilon",
        'Φ' => "\\Phi",
        'Ψ' => "\\Psi",
        'Ω' => "\\Omega",
        '±' => "\\pm",
        '∓' => "\\mp",
        '×' => "\\times",
        '÷' => "\\div",
        '·' | '⋅' => "\\cdot",
        '∗' => "\\ast",
        '∘' => "\\circ",
        '∙' => "\\bullet",
        '≤' => "\\le",
        '≥' => "\\ge",
        '≠' => "\\ne",
        '≈' => "\\approx",
        '≡' => "\\equiv",
        '∼' => "\\sim",
        '≃' => "\\simeq",
        '≅' => "\\cong",
        '∝' => "\\propto",
        '≪' => "\\ll",
        '≫' => "\\gg",
        '∞' => "\\infty",
        '∂' => "\\partial",
        '∇' => "\\nabla",
        '∈' => "\\in",
        '∉' => "\\notin",
        '∋' => "\\ni",
        '⊂' => "\\subset",
        '⊃' => "\\supset",
        '⊆' => "\\subseteq",
        '⊇' => "\\supseteq",
        '∪' => "\\cup",
        '∩' => "\\cap",
        '∅' => "\\emptyset",
        '∀' => "\\forall",
        '∃' => "\\exists",
        '¬' => "\\neg",
        '∧' => "\\wedge",
        '∨' => "\\vee",
        '⊕' => "\\oplus",
        '⊗' => "\\otimes",
        '→' => "\\to",
        '←' => "\\leftarrow",
        '↔' => "\\leftrightarrow",
        '⇒' => "\\Rightarrow",
        '⇐' => "\\Leftarrow",
        '⇔' => "\\Leftrightarrow",
        '↦' => "\\mapsto",
        '↑' => "\\uparrow",
        '↓' => "\\downarrow",
        '…' => "\\ldots",
        '⋯' => "\\cdots",
        '⋮' => "\\vdots",
        '⋱' => "\\ddots",
        '′' => "'",
        '″' => "''",
        '∠' => "\\angle",
        '⊥' => "\\perp",
        '∥' => "\\parallel",
        '°' => "^\\circ",
        'ℏ' => "\\hbar",
        'ℓ' => "\\ell",
        '∑' => "\\sum",
        '∏' => "\\prod",
        '∐' => "\\coprod",
        '∫' => "\\int",
        '∬' => "\\iint",
        '∭' => "\\iiint",
        '∮' => "\\oint",
        '⋃' => "\\bigcup",
        '⋂' => "\\bigcap",
        '⋁' => "\\bigvee",
        '⋀' => "\\bigwedge",
        '⨁' => "\\bigoplus",
        '⨂' => "\\bigotimes",
        '⟨' | '〈' => "\\langle",
        '⟩' | '〉' => "\\rangle",
        '⌊' => "\\lfloor",
        '⌋' => "\\rfloor",
        '⌈' => "\\lceil",
        '⌉' => "\\rceil",
        '\u{2061}' | '\u{2062}' | '\u{2063}' | '\u{200B}' => "",
        '−' => "-",
        '\u{2003}' => "\\quad",
        '\u{2009}' | '\u{2006}' => "\\,",
        '\u{2005}' => "\\:",
        '\u{2004}' => "\\;",
        _ => return None,
    })
}

/// 数学字母区（𝐀 𝔸 𝒜 …）换回普通字母和对应的字体命令。
fn math_alpha(c: char) -> Option<(&'static str, char)> {
    let u = c as u32;
    // 特例：零散放在别处的几个
    let special = match c {
        'ℝ' => Some(("mathbb", 'R')),
        'ℕ' => Some(("mathbb", 'N')),
        'ℤ' => Some(("mathbb", 'Z')),
        'ℚ' => Some(("mathbb", 'Q')),
        'ℂ' => Some(("mathbb", 'C')),
        'ℍ' => Some(("mathbb", 'H')),
        'ℙ' => Some(("mathbb", 'P')),
        'ℎ' => Some(("", 'h')),
        'ℬ' => Some(("mathcal", 'B')),
        'ℰ' => Some(("mathcal", 'E')),
        'ℱ' => Some(("mathcal", 'F')),
        'ℋ' => Some(("mathcal", 'H')),
        'ℐ' => Some(("mathcal", 'I')),
        'ℒ' => Some(("mathcal", 'L')),
        'ℳ' => Some(("mathcal", 'M')),
        'ℛ' => Some(("mathcal", 'R')),
        'ℭ' => Some(("mathfrak", 'C')),
        'ℌ' => Some(("mathfrak", 'H')),
        'ℑ' => Some(("mathfrak", 'I')),
        'ℜ' => Some(("mathfrak", 'R')),
        'ℨ' => Some(("mathfrak", 'Z')),
        _ => None,
    };
    if special.is_some() {
        return special;
    }
    // 每种字体 52 个（A–Z a–z）
    const BLOCKS: &[(u32, &str)] = &[
        (0x1D400, "mathbf"),
        (0x1D434, ""),
        (0x1D468, "boldsymbol"),
        (0x1D49C, "mathcal"),
        (0x1D4D0, "mathcal"),
        (0x1D504, "mathfrak"),
        (0x1D538, "mathbb"),
        (0x1D56C, "mathfrak"),
        (0x1D5A0, "mathsf"),
        (0x1D5D4, "mathsf"),
        (0x1D608, "mathsf"),
        (0x1D63C, "mathsf"),
        (0x1D670, "mathtt"),
    ];
    for (base, cmd) in BLOCKS {
        if (*base..base + 52).contains(&u) {
            let i = u - base;
            let ch = if i < 26 {
                (b'A' + i as u8) as char
            } else {
                (b'a' + (i - 26) as u8) as char
            };
            return Some((cmd, ch));
        }
    }
    // 粗体数字、双线数字等
    for (base, cmd) in [
        (0x1D7CE, "mathbf"),
        (0x1D7D8, "mathbb"),
        (0x1D7E2, "mathsf"),
        (0x1D7EC, "mathsf"),
        (0x1D7F6, "mathtt"),
    ] {
        if (base..base + 10).contains(&u) {
            return Some((cmd, (b'0' + (u - base) as u8) as char));
        }
    }
    None
}

/// 普通文字转 LaTeX：特殊字符转义，符号换命令，数学字母区换字体命令。
fn latex_text(s: &str, plain: bool) -> String {
    let mut out = String::new();
    let mut font: Option<(&str, String)> = None;
    let flush = |out: &mut String, font: &mut Option<(&str, String)>| {
        if let Some((cmd, buf)) = font.take() {
            if cmd.is_empty() {
                out.push_str(&buf);
            } else {
                out.push_str(&format!("\\{cmd}{{{buf}}}"));
            }
        }
    };
    for c in s.chars() {
        if let Some((cmd, ch)) = math_alpha(c) {
            match &mut font {
                Some((f, buf)) if *f == cmd => buf.push(ch),
                _ => {
                    flush(&mut out, &mut font);
                    font = Some((cmd, ch.to_string()));
                }
            }
            continue;
        }
        flush(&mut out, &mut font);
        if let Some(cmd) = symbol(c) {
            if cmd.starts_with('\\') && cmd[1..].chars().all(|x| x.is_ascii_alphabetic()) {
                out.push_str(cmd);
                out.push(' ');
            } else {
                out.push_str(cmd);
            }
            continue;
        }
        match c {
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '%' | '&' | '#' | '$' | '_' => {
                out.push('\\');
                out.push(c);
            }
            '\\' => out.push_str("\\backslash "),
            '~' => out.push_str("\\sim "),
            '^' => out.push_str("\\hat{}"),
            ' ' if plain => out.push_str("\\ "),
            '\u{a0}' => out.push('~'),
            c => out.push(c),
        }
    }
    flush(&mut out, &mut font);
    out
}

/// 包成 LaTeX 参数：一个字符或一个命令时不用括号也行，这里一律加括号，最稳。
fn group(s: &str) -> String {
    format!("{{{}}}", s.trim())
}

/// 括号字符换成 \left 后面能跟的写法。
fn delim(c: &str, left: bool) -> String {
    match c {
        "" => ".".into(),
        "(" | ")" | "[" | "]" | "|" | "/" => c.into(),
        "{" => "\\{".into(),
        "}" => "\\}".into(),
        "‖" | "∥" => "\\|".into(),
        "⟨" | "〈" => "\\langle".into(),
        "⟩" | "〉" => "\\rangle".into(),
        "⌊" => "\\lfloor".into(),
        "⌋" => "\\rfloor".into(),
        "⌈" => "\\lceil".into(),
        "⌉" => "\\rceil".into(),
        _ => {
            if left {
                "(".into()
            } else {
                ")".into()
            }
        }
    }
}

impl Omml {
    pub fn new() -> Omml {
        Omml {
            unsupported: Vec::new(),
        }
    }

    fn miss(&mut self, what: &str) {
        if !self.unsupported.iter().any(|x| x == what) {
            self.unsupported.push(what.to_string());
        }
    }

    /// m:oMath 或 m:oMathPara 转 LaTeX。
    pub fn convert(&mut self, n: Node) -> String {
        let s = if is(n, M, "oMathPara") {
            let parts: Vec<String> = n
                .children()
                .filter(|c| is(*c, M, "oMath"))
                .map(|c| self.seq(c))
                .collect();
            parts.join(" \\\\ ")
        } else {
            self.seq(n)
        };
        tidy(&s)
    }

    /// 子元素按顺序拼起来。
    fn seq(&mut self, n: Node) -> String {
        let mut out = String::new();
        for c in n.children().filter(|c| c.is_element()) {
            let s = self.node(c);
            push(&mut out, &s);
        }
        out
    }

    fn arg(&mut self, n: Node, name: &str) -> String {
        mchild(n, name).map(|c| self.seq(c)).unwrap_or_default()
    }

    fn node(&mut self, n: Node) -> String {
        if n.tag_name().namespace() == Some(W) {
            // 公式里夹着的 w:r（少见），按文字取
            return n
                .descendants()
                .filter(|d| is(*d, W, "t"))
                .filter_map(|d| d.text())
                .map(|t| latex_text(t, false))
                .collect();
        }
        if n.tag_name().namespace() != Some(M) {
            return String::new();
        }
        match n.tag_name().name() {
            "oMath" | "e" | "num" | "den" | "sub" | "sup" | "deg" | "lim" | "fName" => self.seq(n),
            "r" => self.run(n),
            "f" => {
                let a = self.arg(n, "num");
                let b = self.arg(n, "den");
                match prop(n, "fPr", "type") {
                    Some("lin") => format!("{a}/{b}"),
                    Some("skw") => format!("{{}}^{}/{{}}_{}", group(&a), group(&b)),
                    Some("noBar") => {
                        format!("\\genfrac{{}}{{}}{{0pt}}{{}}{}{}", group(&a), group(&b))
                    }
                    _ => format!("\\frac{}{}", group(&a), group(&b)),
                }
            }
            "sSup" => {
                let e = self.arg(n, "e");
                let a = self.arg(n, "sup");
                format!("{}^{}", base(&e), group(&a))
            }
            "sSub" => {
                let e = self.arg(n, "e");
                let a = self.arg(n, "sub");
                format!("{}_{}", base(&e), group(&a))
            }
            "sSubSup" => {
                let e = self.arg(n, "e");
                let a = self.arg(n, "sub");
                let b = self.arg(n, "sup");
                format!("{}_{}^{}", base(&e), group(&a), group(&b))
            }
            "sPre" => {
                let e = self.arg(n, "e");
                let a = self.arg(n, "sub");
                let b = self.arg(n, "sup");
                format!("{{}}_{}^{}{}", group(&a), group(&b), e)
            }
            "rad" => {
                let e = self.arg(n, "e");
                let d = self.arg(n, "deg");
                if on(prop(n, "radPr", "degHide")) || d.trim().is_empty() {
                    format!("\\sqrt{}", group(&e))
                } else {
                    format!("\\sqrt[{}]{}", d.trim(), group(&e))
                }
            }
            "nary" => self.nary(n),
            "d" => self.delimiter(n),
            "acc" => {
                let e = self.arg(n, "e");
                let ch = prop(n, "accPr", "chr").unwrap_or("\u{302}");
                let cmd = match ch {
                    "\u{302}" | "^" | "ˆ" => "hat",
                    "\u{303}" | "~" | "˜" => "tilde",
                    "\u{304}" | "\u{305}" | "¯" | "‾" => "bar",
                    "\u{307}" | "˙" => "dot",
                    "\u{308}" | "¨" => "ddot",
                    "\u{30C}" | "ˇ" => "check",
                    "\u{306}" | "˘" => "breve",
                    "\u{301}" | "´" => "acute",
                    "\u{300}" | "`" => "grave",
                    "\u{20D7}" | "→" | "\u{20D1}" => "vec",
                    "\u{20E1}" | "↔" => "overleftrightarrow",
                    "\u{30A}" | "˚" => "mathring",
                    _ => {
                        self.miss("重音符号");
                        "hat"
                    }
                };
                format!("\\{cmd}{}", group(&e))
            }
            "bar" => {
                let e = self.arg(n, "e");
                if prop(n, "barPr", "pos") == Some("top") {
                    format!("\\overline{}", group(&e))
                } else {
                    format!("\\underline{}", group(&e))
                }
            }
            "groupChr" => {
                let e = self.arg(n, "e");
                let top = prop(n, "groupChrPr", "pos") == Some("top");
                let ch = prop(n, "groupChrPr", "chr").unwrap_or("\u{23DF}");
                match (ch, top) {
                    ("\u{23DE}", _) | ("\u{FE37}", _) => format!("\\overbrace{}", group(&e)),
                    ("\u{23DF}", _) | ("\u{FE38}", _) => format!("\\underbrace{}", group(&e)),
                    ("→", true) => format!("\\overrightarrow{}", group(&e)),
                    ("←", true) => format!("\\overleftarrow{}", group(&e)),
                    (_, true) => format!("\\overbrace{}", group(&e)),
                    (_, false) => format!("\\underbrace{}", group(&e)),
                }
            }
            "limLow" | "limUpp" => {
                let low = n.tag_name().name() == "limLow";
                let em = mchild(n, "e");
                let lim = self.arg(n, "lim");
                // \underbrace{x}_{y}、\overbrace{x}^{y}
                if let Some(e) = em {
                    let kids: Vec<Node> = e.children().filter(|c| c.is_element()).collect();
                    if kids.len() == 1 && is(kids[0], M, "groupChr") {
                        let inner = self.node(kids[0]);
                        return format!("{inner}{}{}", if low { "_" } else { "^" }, group(&lim));
                    }
                    // \lim 的 limUpp 套 limLow（上下都有）
                    if !low && kids.len() == 1 && is(kids[0], M, "limLow") {
                        let inner = self.node(kids[0]);
                        return format!("{inner}^{}", group(&lim));
                    }
                }
                let e = em.map(|c| self.seq(c)).unwrap_or_default();
                let t = e.trim();
                if FUNCS.contains(&t.trim_start_matches('\\')) || t.starts_with("\\operatorname") {
                    let cmd = if t.starts_with('\\') {
                        t.to_string()
                    } else {
                        format!("\\{t}")
                    };
                    format!("{cmd}{}{}", if low { "_" } else { "^" }, group(&lim))
                } else if low {
                    format!("\\underset{}{}", group(&lim), group(&e))
                } else {
                    format!("\\overset{}{}", group(&lim), group(&e))
                }
            }
            "func" => {
                let name = mchild(n, "fName").map(|c| self.seq(c)).unwrap_or_default();
                let e = self.arg(n, "e");
                let t = name.trim();
                let name = if FUNCS.contains(&t) {
                    format!("\\{t}")
                } else if t.chars().all(|c| c.is_ascii_alphabetic()) && !t.is_empty() {
                    format!("\\operatorname{{{t}}}")
                } else {
                    t.to_string()
                };
                format!("{name} {e}")
            }
            "m" => self.matrix(n, "matrix"),
            "eqArr" => {
                let rows: Vec<String> = n
                    .children()
                    .filter(|c| is(*c, M, "e"))
                    .map(|c| self.seq(c))
                    .collect();
                format!("\\begin{{aligned}}{}\\end{{aligned}}", rows.join(" \\\\ "))
            }
            "box" | "phant" => {
                if n.tag_name().name() == "phant" && prop(n, "phantPr", "show") == Some("0") {
                    format!("\\phantom{}", group(&self.arg(n, "e")))
                } else {
                    self.arg(n, "e")
                }
            }
            "borderBox" => {
                let e = self.arg(n, "e");
                let pr = mchild(n, "borderBoxPr");
                let hidden_all = pr.is_some_and(|p| {
                    ["hideTop", "hideBot", "hideLeft", "hideRight"]
                        .iter()
                        .all(|h| on(mchild(p, h).and_then(mval)))
                });
                if hidden_all && pr.is_some_and(|p| on(mchild(p, "strikeBLTR").and_then(mval))) {
                    format!("\\cancel{}", group(&e))
                } else {
                    format!("\\boxed{}", group(&e))
                }
            }
            // 属性元素不出字
            name if name.ends_with("Pr") => String::new(),
            other => {
                self.miss(&format!("m:{other}"));
                n.descendants()
                    .filter(|d| is(*d, M, "t"))
                    .filter_map(|d| d.text())
                    .map(|t| latex_text(t, false))
                    .collect()
            }
        }
    }

    fn run(&mut self, n: Node) -> String {
        let text: String = n
            .children()
            .filter(|c| is(*c, M, "t") || is(*c, W, "t"))
            .filter_map(|c| c.text())
            .collect();
        if text.is_empty() {
            return String::new();
        }
        let rpr = mchild(n, "rPr");
        let nor = rpr.is_some_and(|p| mchild(p, "nor").is_some());
        let sty = rpr.and_then(|p| mchild(p, "sty")).and_then(mval);
        let scr = rpr.and_then(|p| mchild(p, "scr")).and_then(mval);
        if nor {
            return format!("\\text{{{}}}", text_escape(&text));
        }
        let t = text.trim();
        // 函数名（正体的 sin、lim）
        if sty == Some("p") && FUNCS.contains(&t) {
            return format!("\\{t} ");
        }
        let body = latex_text(&text, false);
        let font = match (scr, sty) {
            (Some("double-struck"), _) => Some("mathbb"),
            (Some("script"), _) => Some("mathcal"),
            (Some("fraktur"), _) => Some("mathfrak"),
            (Some("sans-serif"), Some("b")) => Some("mathbf"),
            (Some("sans-serif"), _) => Some("mathsf"),
            (Some("monospace"), _) => Some("mathtt"),
            (_, Some("b")) => Some("mathbf"),
            (_, Some("bi")) => Some("boldsymbol"),
            (_, Some("p")) if t.chars().any(|c| c.is_alphabetic()) => Some("mathrm"),
            _ => None,
        };
        match font {
            Some(f) => format!("\\{f}{}", group(&body)),
            None => body,
        }
    }

    fn nary(&mut self, n: Node) -> String {
        let ch = prop(n, "naryPr", "chr").unwrap_or("∫");
        let op = ch
            .chars()
            .next()
            .and_then(symbol)
            .map(str::to_string)
            .unwrap_or_else(|| latex_text(ch, false));
        let sub_hide = on(prop(n, "naryPr", "subHide"));
        let sup_hide = on(prop(n, "naryPr", "supHide"));
        let und_ovr = prop(n, "naryPr", "limLoc") == Some("undOvr");
        let sub = self.arg(n, "sub");
        let sup = self.arg(n, "sup");
        let e = self.arg(n, "e");
        let mut out = op;
        // 积分默认把上下限放右边，求和默认放上下；和默认不一样时写 \limits / \nolimits
        let is_int = matches!(ch, "∫" | "∬" | "∭" | "∮" | "∯" | "∰");
        if (!sub_hide && !sub.trim().is_empty()) || (!sup_hide && !sup.trim().is_empty()) {
            if is_int && und_ovr {
                out.push_str("\\limits");
            } else if !is_int && !und_ovr {
                out.push_str("\\nolimits");
            }
        }
        if !sub_hide && !sub.trim().is_empty() {
            out.push_str(&format!("_{}", group(&sub)));
        }
        if !sup_hide && !sup.trim().is_empty() {
            out.push_str(&format!("^{}", group(&sup)));
        }
        out.push(' ');
        out.push_str(&e);
        out
    }

    fn delimiter(&mut self, n: Node) -> String {
        let beg = prop(n, "dPr", "begChr").unwrap_or("(");
        let end = prop(n, "dPr", "endChr").unwrap_or(")");
        let sep = prop(n, "dPr", "sepChr").unwrap_or("|");
        let es: Vec<Node> = n.children().filter(|c| is(*c, M, "e")).collect();
        // 只包着一个矩阵、方程组：pmatrix、cases 这些环境
        if es.len() == 1 {
            let kids: Vec<Node> = es[0].children().filter(|c| c.is_element()).collect();
            if kids.len() == 1 {
                let k = kids[0];
                if is(k, M, "m") {
                    let env = match (beg, end) {
                        ("(", ")") => Some("pmatrix"),
                        ("[", "]") => Some("bmatrix"),
                        ("{", "}") => Some("Bmatrix"),
                        ("|", "|") => Some("vmatrix"),
                        ("‖", "‖") | ("∥", "∥") => Some("Vmatrix"),
                        ("{", "") => Some("cases"),
                        _ => None,
                    };
                    if let Some(env) = env {
                        return self.matrix(k, env);
                    }
                }
                if is(k, M, "eqArr") && beg == "{" && end.is_empty() {
                    let rows: Vec<String> = k
                        .children()
                        .filter(|c| is(*c, M, "e"))
                        .map(|c| self.seq(c))
                        .collect();
                    return format!("\\begin{{cases}}{}\\end{{cases}}", rows.join(" \\\\ "));
                }
                // \binom：noBar 分数外面套圆括号
                if is(k, M, "f")
                    && prop(k, "fPr", "type") == Some("noBar")
                    && beg == "("
                    && end == ")"
                {
                    let a = self.arg(k, "num");
                    let b = self.arg(k, "den");
                    return format!("\\binom{}{}", group(&a), group(&b));
                }
            }
        }
        let inner: Vec<String> = es.iter().map(|e| self.seq(*e)).collect();
        let sep = if sep == "|" {
            "\\mid ".to_string()
        } else {
            latex_text(sep, false)
        };
        format!(
            "\\left{} {} \\right{}",
            delim(beg, true),
            inner.join(&sep),
            delim(end, false)
        )
    }

    fn matrix(&mut self, n: Node, env: &str) -> String {
        let rows: Vec<String> = n
            .children()
            .filter(|c| is(*c, M, "mr"))
            .map(|r| {
                r.children()
                    .filter(|c| is(*c, M, "e"))
                    .map(|c| self.seq(c))
                    .collect::<Vec<_>>()
                    .join(" & ")
            })
            .collect();
        format!("\\begin{{{env}}}{}\\end{{{env}}}", rows.join(" \\\\ "))
    }
}

/// 拼接时，前一段以命令名结尾、后一段以字母开头，中间补空格。
fn push(out: &mut String, s: &str) {
    if s.is_empty() {
        return;
    }
    let ends_cmd = {
        let t = out.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        t.ends_with('\\') && t.len() < out.len() && !out.ends_with(' ')
    };
    if ends_cmd && s.starts_with(|c: char| c.is_ascii_alphabetic()) {
        out.push(' ');
    }
    out.push_str(s);
}

/// 上下标的底：多于一个记号时加括号。
fn base(e: &str) -> String {
    let t = e.trim();
    let single = t.chars().count() == 1
        || (t.starts_with('\\') && t[1..].chars().all(|c| c.is_ascii_alphabetic()) && t.len() > 1)
        || (t.starts_with("\\left") && t.ends_with(|c: char| c != '}'))
        || t.starts_with("\\mathbf{") && t.matches('{').count() == 1;
    if single || t.is_empty() {
        if t.is_empty() {
            "{}".into()
        } else {
            t.to_string()
        }
    } else {
        group(t)
    }
}

fn text_escape(s: &str) -> String {
    s.replace('\\', "\\textbackslash ")
        .replace('{', "\\{")
        .replace('}', "\\}")
        .replace('%', "\\%")
        .replace('&', "\\&")
        .replace('#', "\\#")
        .replace('$', "\\$")
        .replace('_', "\\_")
}

/// 收拾空格：去掉多余的连续空格和首尾空格。
fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c == ' ' {
            if !prev_space {
                out.push(c);
            }
            prev_space = true;
        } else {
            prev_space = false;
            out.push(c);
        }
    }
    // 「{ 」「 }」里的空格没用
    let out = out.replace("{ ", "{").replace(" }", "}");
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(inner: &str) -> String {
        let xml = format!(r#"<m:oMath xmlns:m="{M}" xmlns:w="{W}">{inner}</m:oMath>"#);
        let doc = roxmltree::Document::parse(&xml).unwrap();
        Omml::new().convert(doc.root_element())
    }

    #[test]
    fn basics() {
        assert_eq!(
            conv(
                "<m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f>"
            ),
            "\\frac{a}{b}"
        );
        assert_eq!(
            conv(
                "<m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup>"
            ),
            "x^{2}"
        );
        assert_eq!(
            conv(
                "<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e><m:r><m:t>2</m:t></m:r></m:e></m:rad>"
            ),
            "\\sqrt{2}"
        );
        assert_eq!(
            conv("<m:r><m:t>α+β≤∞</m:t></m:r>"),
            "\\alpha +\\beta \\le \\infty"
        );
        assert_eq!(conv("<m:r><m:t>ℝ</m:t></m:r>"), "\\mathbb{R}");
    }
}
