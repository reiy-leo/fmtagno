//! PDF 渲染器：先把元素树写成 Typst 标记，再用 typst 库编译成 PDF。
//!
//! 文字一律写成 Typst 字符串字面量（`#"..."`），不用转义标记语法。
//! 图片通过虚拟文件 `/assets/N.ext` 交给 typst。
//! 每个元素写成一个 block，样式用 block 里的 set 规则表达，只写和父元素不同的部分。

use std::{collections::HashMap, fmt::Write, sync::OnceLock};

use typst::{
    Library, LibraryExt, World,
    diag::{FileError, FileResult},
    foundations::{Bytes, Datetime, Duration},
    syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot},
    text::{Font, FontBook, FontInfo, FontWeight},
    utils::LazyHash,
};
use typst_kit::fonts::{self, FontStore};
use typst_layout::PagedDocument;

use crate::{
    assets,
    components::{CodeProps, Component, Field, ImageProps, Inline, ListProps, Run, TableProps},
    doc::{Doc, HfNodes, Node, SectionLayout},
    render::{
        Options, Output,
        common::{self, Grid, Warn},
    },
    style::{
        Align, AnchorTo, Background, Border, BorderStyle, Caps, ColWidth, Color, Edge, HAlign, Len,
        LineHeight, MarkerGap, Position, Side, Strike, Style, UnderlineStyle, VerticalAlign, Wrap,
        WrapMode,
    },
};

const FORMAT: &str = "pdf";

/// 默认字体顺序：西文无衬线在前，中文按 macOS、Linux、Windows 常见字体排，
/// 最后是 typst 自带字体兜底。typst 按字形逐个往后找。
const FONTS: &[&str] = &[
    "Helvetica Neue",
    "Arial",
    "Liberation Sans",
    "PingFang SC",
    "Hiragino Sans GB",
    "Noto Sans CJK SC",
    "Source Han Sans SC",
    "Microsoft YaHei",
    "Libertinus Serif",
];
const MONO_FONTS: &[&str] = &["Menlo", "Consolas", "DejaVu Sans Mono"];
/// 文字盒高度（ascender 到 descender）约等于字号的这么多倍，用来把行高倍数换成 typst 的 leading。
const GLYPH_BOX: f32 = 1.17;
/// 行盒基线以上、以下的部分（合起来是 GLYPH_BOX），按常见中文字体估的。
const ASCENT: f32 = 0.88;
const DESCENT: f32 = 0.29;

pub fn render(doc: &Doc, tree: &Node, opts: &Options) -> anyhow::Result<Output> {
    let (source, files, mut warnings) = markup(doc, tree, opts);
    let fallbacks = fallback_fonts(&source);
    if std::env::var_os("FMTAGNO_DEBUG_TYPST").is_some() {
        eprintln!("{source}");
    }
    let world = FmtagnoWorld::new(source, files);
    let warned = typst::compile::<PagedDocument>(&world);
    for w in warned.warnings.iter() {
        // 默认字体列表本来就是按平台挑着用的，找不到不算问题
        let msg = w.message.to_string();
        // 用户写成回退列表的字体（逗号分开的几个），找不到前面的也正常
        let default_font = msg.strip_prefix("unknown font family: ").is_some_and(|f| {
            FONTS
                .iter()
                .chain(MONO_FONTS)
                .any(|d| d.eq_ignore_ascii_case(f))
                || fallbacks.iter().any(|d| d.eq_ignore_ascii_case(f))
        });
        let w = format!("typst：{msg}");
        if !default_font && !warnings.contains(&w) {
            warnings.push(w);
        }
    }
    let document = warned.output.map_err(|errs| {
        let msgs: Vec<String> = errs.iter().map(|e| e.message.to_string()).collect();
        anyhow::anyhow!("typst 排版失败：{}", msgs.join("；"))
    })?;
    let bytes = typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default()).map_err(|errs| {
        let msgs: Vec<String> = errs.iter().map(|e| e.message.to_string()).collect();
        anyhow::anyhow!("生成 PDF 失败：{}", msgs.join("；"))
    })?;
    Ok(Output { bytes, warnings })
}

/// 源码里所有字体列表中的名字（只要列表里不止一个用户字体就算回退）。
fn fallback_fonts(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.split("font: (").skip(1) {
        let Some(list) = line.split_once(')') else {
            continue;
        };
        let names: Vec<String> = list
            .0
            .split("\", \"")
            .map(|x| {
                x.trim_matches(|c| c == '"' || c == ',' || c == ' ')
                    .to_string()
            })
            .filter(|x| !FONTS.contains(&x.as_str()) && !MONO_FONTS.contains(&x.as_str()))
            .collect();
        if names.len() > 1 {
            out.extend(names);
        }
    }
    out
}

/// 图片虚拟文件：路径和内容。
pub type Files = Vec<(String, Vec<u8>)>;

/// 生成 Typst 源码。返回源码、图片文件、警告。
/// 页码函数，见 preamble。
const FIELDS_FN: &str = r#"#let fmtagno-info() = {
  let p = here().page()
  let num = 1
  let prev = none
  let next = none
  for m in query(<fmtagno-sec>) {
    let sp = m.location().page()
    if sp > p { next = sp; break }
    if m.value != none { num = m.value } else if prev != none { num = num + sp - prev }
    prev = sp
  }
  if prev == none { prev = 1 }
  let end = locate(<fmtagno-end>).page()
  (
    page: num + p - prev,
    first: prev == p,
    total: end,
    section-pages: (if next == none { end + 1 } else { next }) - prev,
  )
}
"#;

pub fn markup(doc: &Doc, tree: &Node, opts: &Options) -> (String, Files, Vec<String>) {
    let mut m = Markup {
        out: String::new(),
        files: Vec::new(),
        warn: Warn::default(),
        opts,
        margins: [0.0; 4],
    };
    m.preamble(doc);
    let base = Style::default();
    m.node(tree, &base);
    m.out.push_str("#metadata(none) <fmtagno-end>\n");
    (m.out, m.files, m.warn.list)
}

struct Markup<'a> {
    out: String,
    files: Files,
    warn: Warn,
    opts: &'a Options,
    /// 上右下左页边距，pt
    margins: [f32; 4],
}

impl Markup<'_> {
    fn preamble(&mut self, doc: &Doc) {
        let o = &mut self.out;
        // 页码这类域都靠它算：每节开头放一个 <fmtagno-sec> 标记（值是重新开始的页码或 none），
        // 文末放 <fmtagno-end>。页码自己数，不用 counter(page)，这样首页判断、重排页码和
        // Word 的规则一致。
        o.push_str(FIELDS_FN);
        let mut docset = Vec::new();
        if let Some(t) = &doc.meta.title {
            docset.push(format!("title: {}", lit(t)));
        }
        if let Some(a) = &doc.meta.author {
            docset.push(format!("author: {}", lit(a)));
        }
        if !docset.is_empty() {
            let _ = writeln!(o, "#set document({})", docset.join(", "));
        }
        let lang = doc.meta.lang.as_deref().unwrap_or("zh");
        let _ = writeln!(
            o,
            "#set text(font: {}, lang: {}, top-edge: \"ascender\", bottom-edge: \"descender\")",
            lits(FONTS),
            lit(&lang_code(lang))
        );
        let _ = writeln!(o, "#set par(spacing: 0pt)");
        let _ = writeln!(o, "#set block(above: 0pt, below: 0pt)");
        let _ = writeln!(o, "#set figure(numbering: none)");
        // 标题的外观全由样式决定，heading 只留着生成书签
        let _ = writeln!(o, "#show heading: it => it.body");
    }

    fn node(&mut self, n: &Node, parent: &Style) {
        let s = &n.style;
        match &n.component {
            Component::Document(_) => {
                let body = self.children_src(n);
                let src = self.scope_src(body, s, parent, &n.id);
                self.out.push_str(&src);
            }
            Component::Slide(p) => {
                self.out.push_str("#pagebreak(weak: true)\n");
                let mut body = String::new();
                if let Some(t) = &p.title {
                    let mut ts = s.clone();
                    ts.merge(&n.part("title"));
                    let inner = format!("#{}", lit(t));
                    body.push_str(&self.block_src(
                        format!("#heading(level: 2)[{inner}]"),
                        &ts,
                        s,
                        &n.id,
                    ));
                }
                let mut bs = s.clone();
                bs.merge(&n.part("body"));
                let kids = {
                    let saved = std::mem::take(&mut self.out);
                    for c in &n.children {
                        self.node(c, &bs);
                    }
                    std::mem::replace(&mut self.out, saved)
                };
                body.push_str(&self.scope_src(kids, &bs, s, &n.id));
                let src = self.scope_src(body, s, parent, &n.id);
                self.out.push_str(&src);
                self.out.push_str("#pagebreak(weak: true)\n");
            }
            Component::Heading(p) => {
                let body = self.inline(&p.text, s);
                self.block(
                    format!("#heading(level: {})[{body}]", p.level),
                    s,
                    parent,
                    &n.id,
                );
            }
            Component::Text(p) => {
                let mut body = self.inline(&p.text, s);
                // 空段落也占一行（和 Word 一样），放一个零宽空格撑起行盒
                if body.trim().is_empty() {
                    body = "#sym.zws".into();
                }
                self.block(format!("#par[{body}]"), s, parent, &n.id);
            }
            Component::Code(p) => self.code(n, p, parent),
            Component::List(p) => self.list(n, p, parent),
            Component::Table(p) => self.table(n, p, parent),
            Component::Image(p) => self.image(n, p, parent),
            Component::PageBreak(_) => self.out.push_str("#pagebreak()\n"),
            Component::Section(p) => self.section(n, p.layout(), parent),
        }
    }

    /// 一节：换页面设置（typst 会从新的一页开始），放节标记，再排内容。
    fn section(&mut self, n: &Node, lay: &SectionLayout, parent: &Style) {
        let page = &lay.page;
        let mut ws = Vec::new();
        let (w, h) = page.dims_pt(&mut ws);
        for x in ws {
            self.warn.push(x);
        }
        let [mt, mr, mb, ml] = page.margins_pt();
        let gutter = page.gutter.map(|g| g.abs_pt()).unwrap_or(0.0);
        self.margins = [mt, mr, mb, ml + gutter];
        let cols = page.columns.as_ref().map_or(1, |c| c.count());
        let (hd, fd) = page.hf_distance_pt();
        let header = self.hf_src(&lay.header, lay, parent, true, hd);
        let footer = self.hf_src(&lay.footer, lay, parent, false, fd);
        let mut o = String::new();
        if lay.index > 0 {
            o.push_str("#pagebreak(weak: true)\n");
        }
        let _ = writeln!(
            o,
            "#set page(width: {w}pt, height: {h}pt, margin: (top: {mt}pt, right: {mr}pt, bottom: {mb}pt, left: {}pt), columns: {cols}, header: {header}, footer: {footer}, header-ascent: 0pt, footer-descent: 0pt)",
            ml + gutter
        );
        if let Some(c) = page.columns.as_ref().filter(|c| c.count() > 1) {
            let _ = writeln!(o, "#set columns(gutter: {}pt)", c.gap_pt());
        }
        let start = lay
            .page_number_start
            .map_or("none".to_string(), |x| x.to_string());
        let _ = writeln!(o, "#metadata({start}) <fmtagno-sec>");
        self.out.push_str(&o);
        let body = self.children_src(n);
        let src = self.scope_src(body, &n.style, parent, &n.id);
        self.out.push_str(&src);
    }

    /// 页眉或页脚的 typst 表达式：按本节首页、偶数页、其他页挑内容。
    /// 页眉顶端离纸边 `dist`，页脚底端离纸边 `dist`，跟 Word 一样。
    fn hf_src(
        &mut self,
        set: &HfNodes,
        lay: &SectionLayout,
        parent: &Style,
        top: bool,
        dist: f32,
    ) -> String {
        if set.is_empty() {
            return "none".into();
        }
        let one = |m: &mut Self, n: Option<&Node>| -> String {
            let Some(n) = n else {
                return "none".into();
            };
            let saved = std::mem::take(&mut m.out);
            m.node(n, parent);
            let body = std::mem::replace(&mut m.out, saved);
            let (inset, align) = if top {
                (format!("(top: {dist}pt)"), "top")
            } else {
                (format!("(bottom: {dist}pt)"), "bottom")
            };
            format!("block(width: 100%, height: 100%, inset: {inset}, align({align}, [\n{body}]))")
        };
        let default = one(self, set.default.as_ref());
        let mut src = String::from("context { let i = fmtagno-info(); ");
        if lay.title_page {
            let f = one(self, set.for_first());
            let _ = write!(src, "if i.first {{ {f} }} else ");
        }
        if lay.even_odd {
            let e = one(self, set.for_even());
            let _ = write!(src, "if calc.even(i.page) {{ {e} }} else ");
        }
        let _ = write!(src, "{{ {default} }} }}");
        src
    }

    fn children_src(&mut self, n: &Node) -> String {
        let saved = std::mem::take(&mut self.out);
        for c in &n.children {
            self.node(c, &n.style);
        }
        std::mem::replace(&mut self.out, saved)
    }

    fn block(&mut self, body: String, s: &Style, parent: &Style, id: &str) {
        let src = self.block_src(body, s, parent, id);
        self.out.push_str(&src);
    }

    /// 容器（Document、Slide）：里面可能有分页，不能包进 block。
    /// 没有盒子样式时只用一个内容块限定 set 规则的范围。
    fn scope_src(&mut self, body: String, s: &Style, parent: &Style, id: &str) -> String {
        if s.has_box() || s.width.is_some() || s.height.is_some() {
            return self.block_src(body, s, parent, id);
        }
        let src = self.block_src(String::new(), s, parent, id);
        // block_src 的形状是 `#block(...)[\n{sets}\n]`，取出 sets
        let sets = src
            .split_once("[\n")
            .and_then(|(_, rest)| rest.rsplit_once("\n]"))
            .map(|(x, _)| x.to_string())
            .unwrap_or_default();
        let mut out = String::new();
        if s.page_break_before == Some(true) {
            out.push_str("#pagebreak(weak: true)\n");
        }
        let _ = writeln!(out, "#[\n{sets}\n{body}]");
        out
    }

    /// 把一段内容包上段落和盒子样式。
    fn block_src(&mut self, body: String, s: &Style, parent: &Style, id: &str) -> String {
        let fs = s.size_pt();
        let mut sets = String::new();
        let text = self.text_args(s, parent, id);
        if !text.is_empty() {
            let _ = writeln!(sets, "#set text({text})");
        }
        let mut par = Vec::new();
        macro_rules! changed {
            ($f:ident) => {
                s.$f.is_some() && parent.$f != s.$f
            };
        }
        if (changed!(line_height) || changed!(font_size))
            && let Some(l) = s.line_height
        {
            // 像 CSS 那样把多出来的行距对半分到每行上下（half-leading），
            // 单行的段落、列表项也就有了行高，和 HTML、Word 的间距对得上。
            // 行间距交给行盒，leading 归零。
            let half = match l {
                LineHeight::Multiple(x) => format!("{:.3}em", ((x - GLYPH_BOX) / 2.0).max(0.0)),
                LineHeight::Exact(x) | LineHeight::AtLeast(x) => {
                    format!("{:.2}pt", ((x.pt(fs, 0.0) - GLYPH_BOX * fs) / 2.0).max(0.0))
                }
            };
            let _ = writeln!(
                sets,
                "#set text(top-edge: {ASCENT}em + {half}, bottom-edge: -{DESCENT}em - {half})"
            );
            par.push("leading: 0pt".into());
        }
        if changed!(align) {
            match s.align {
                Some(Align::Justify | Align::Distribute) => par.push("justify: true".into()),
                Some(_) => par.push("justify: false".into()),
                None => {}
            }
            if s.align == Some(Align::Distribute) {
                self.warn
                    .unsupported(FORMAT, "align: distribute（按 justify 处理）", id);
            }
        }
        if changed!(text_indent) || changed!(hanging_indent) || changed!(font_size) {
            if let Some(h) = s.hanging_indent {
                par.push(format!(
                    "hanging-indent: {}pt, first-line-indent: 0pt",
                    h.pt(fs, 0.0)
                ));
            } else if let Some(t) = s.text_indent {
                par.push(format!(
                    "first-line-indent: (amount: {}pt, all: true)",
                    t.pt(fs, 0.0)
                ));
            }
        }
        if !par.is_empty() {
            let _ = writeln!(sets, "#set par({})", par.join(", "));
        }
        if changed!(align)
            && let Some(a) = s.align
        {
            let a = match a {
                Align::Center => "center",
                Align::Right => "right",
                _ => "left",
            };
            let _ = writeln!(sets, "#set align({a})");
        }
        if changed!(widow_control)
            && let Some(w) = s.widow_control
        {
            let c = if w { "100%" } else { "0%" };
            let _ = writeln!(sets, "#set text(costs: (widow: {c}, orphan: {c}))");
        }
        if changed!(auto_space)
            && let Some(a) = s.auto_space
        {
            let _ = writeln!(
                sets,
                "#set text(cjk-latin-spacing: {})",
                if a { "auto" } else { "none" }
            );
        }
        for (f, on) in [
            ("borderBetween", s.border_between.is_some()),
            ("borderDiagonal", s.border_diagonal.is_some()),
            ("contextualSpacing", s.contextual_spacing == Some(true)),
            ("wrap: false", s.wrap == Some(Wrap::Text(false))),
            ("emphasisMark", s.emphasis_mark.is_some()),
            ("scale", s.scale.is_some()),
        ] {
            if on {
                self.warn.unsupported(FORMAT, f, id);
            }
        }

        let mut body = body;
        if let Some(r) = s.text_rotation.filter(|r| *r == 90 || *r == 270) {
            let deg = if r == 90 { 90 } else { -90 };
            body = format!("#rotate({deg}deg, reflow: true)[{body}]");
        }
        let mut args = Vec::new();
        if let Some(bg) = &s.background {
            args.push(format!("fill: {}", bg_src(bg)));
        }
        let strokes = strokes(s);
        if !strokes.is_empty() {
            args.push(format!("stroke: ({})", strokes.join(", ")));
        }
        let pads: Vec<String> = Edge::ALL
            .iter()
            .zip(["top", "right", "bottom", "left"])
            .filter_map(|(e, k)| s.padding_side(*e).map(|p| format!("{k}: {p}pt")))
            .collect();
        if !pads.is_empty() {
            args.push(format!("inset: ({})", pads.join(", ")));
        }
        if let Some(r) = s.radius {
            args.push(format!("radius: {}pt", r.pt(fs, 0.0)));
        }
        if let Some(b) = s.space_before {
            args.push(format!("above: {}pt", b.pt(fs, 0.0)));
        }
        if let Some(a) = s.space_after {
            args.push(format!("below: {}pt", a.pt(fs, 0.0)));
        }
        if s.keep_together == Some(true) {
            args.push("breakable: false".into());
        }
        if s.keep_with_next == Some(true) {
            args.push("sticky: true".into());
        }
        match s.width {
            Some(Len::Pct(p)) => args.push(format!("width: {p}%")),
            Some(w) => args.push(format!("width: {}pt", w.pt(fs, 0.0))),
            None => args.push("width: 100%".into()),
        }
        if let Some(h) = s.height {
            args.push(match h {
                Len::Pct(p) => format!("height: {p}%"),
                h => format!("height: {}pt", h.pt(fs, 0.0)),
            });
        }
        let mut src = String::new();
        if s.page_break_before == Some(true) {
            src.push_str("#pagebreak(weak: true)\n");
        }
        let ins: Vec<f32> = Edge::ALL
            .iter()
            .map(|e| s.padding_side(*e).unwrap_or(0.0))
            .collect();
        let dbl = double_lines(
            [
                s.border_side(Edge::Top),
                s.border_side(Edge::Right),
                s.border_side(Edge::Bottom),
                s.border_side(Edge::Left),
            ],
            [ins[0], ins[1], ins[2], ins[3]],
        );
        let inner = format!("#block({})[\n{dbl}{sets}{body}\n]", args.join(", "));
        let ml = s.margin_left.map(|x| x.pt(fs, 0.0)).unwrap_or(0.0);
        let mr = s.margin_right.map(|x| x.pt(fs, 0.0)).unwrap_or(0.0);
        if ml != 0.0 || mr != 0.0 {
            let _ = writeln!(src, "#pad(left: {ml}pt, right: {mr}pt)[{inner}]");
        } else {
            let _ = writeln!(src, "{inner}");
        }
        src
    }

    /// `set text(...)` 的参数：和父样式不同的字体、字号、颜色、粗斜体等。
    fn text_args(&mut self, s: &Style, parent: &Style, _id: &str) -> String {
        let mut a = Vec::new();
        if s.font_family.is_some() && s.font_family != parent.font_family {
            a.push(format!("font: {}", font_list(s, false)));
        }
        if s.font_size.is_some() && s.font_size != parent.font_size {
            a.push(format!("size: {}pt", s.size_pt()));
        }
        if let Some(c) = s.color.as_ref().filter(|_| s.color != parent.color) {
            a.push(format!("fill: {}", color(c)));
        }
        if s.bold.is_some() && s.bold != parent.bold {
            a.push(format!(
                "weight: {}",
                if s.bold == Some(true) {
                    "\"bold\""
                } else {
                    "\"regular\""
                }
            ));
        }
        if s.italic.is_some() && s.italic != parent.italic {
            a.push(format!(
                "style: {}",
                if s.italic == Some(true) {
                    "\"italic\""
                } else {
                    "\"normal\""
                }
            ));
        }
        if let Some(l) = s
            .letter_spacing
            .filter(|_| s.letter_spacing != parent.letter_spacing)
        {
            a.push(format!("tracking: {}pt", l.pt(s.size_pt(), 0.0)));
        }
        if let Some(k) = s.kerning.filter(|_| s.kerning != parent.kerning) {
            a.push(format!("kerning: {k}"));
        }
        if let Some(l) = s.lang.as_ref().filter(|_| s.lang != parent.lang) {
            a.push(format!("lang: {}", lit(&lang_code(l))));
        }
        if let Some(d) = s.direction.filter(|_| s.direction != parent.direction) {
            a.push(format!("dir: {}", d.as_str()));
        }
        a.join(", ")
    }

    fn inline(&mut self, t: &Inline, para: &Style) -> String {
        let mut out = String::new();
        for r in t.runs() {
            out.push_str(&self.run(r, para));
        }
        out
    }

    /// 文字换了样式环境（格子、列表项）：run 样式按新环境重算。
    fn inline_in(&mut self, t: &Inline, old: &Style, new: &Style) -> String {
        let mut out = String::new();
        for r in common::rebase_runs(t, old, new) {
            out.push_str(&self.run(&r, new));
        }
        out
    }

    fn run(&mut self, r: &Run, para: &Style) -> String {
        let s = &r.computed;
        if s.hidden == Some(true) {
            return String::new();
        }
        let mut piece = String::new();
        if let Some(src) = &r.image {
            let Some(path) = self.asset_path(&r.text, src) else {
                return String::new();
            };
            let mut args = vec![lit(&path)];
            if let Some(w) = r.width {
                args.push(format!("width: {w}pt"));
            }
            if let Some(h) = r.height {
                args.push(format!("height: {h}pt"));
            }
            return format!("#box(image({}))", args.join(", "));
        }
        if let Some(f) = r.field {
            let key = match f {
                Field::Page => "page",
                Field::Pages => "total",
                Field::SectionPages => "section-pages",
            };
            let _ = write!(piece, "#context str(fmtagno-info().{key})");
        }
        for (i, line) in r.text.split('\n').enumerate().filter(|_| r.field.is_none()) {
            if i > 0 {
                piece.push_str("#linebreak()");
            }
            if !line.is_empty() {
                let _ = write!(piece, "#{}", lit(line));
            }
        }
        let args = self.text_args(s, para, "");
        if !args.is_empty() {
            piece = format!("#text({args})[{piece}]");
        }
        match s.caps {
            Some(Caps::Small) => piece = format!("#smallcaps[{piece}]"),
            Some(Caps::All) => piece = format!("#upper[{piece}]"),
            None => {}
        }
        match s.vertical_align {
            Some(VerticalAlign::Super) => piece = format!("#super[{piece}]"),
            Some(VerticalAlign::Sub) => piece = format!("#sub[{piece}]"),
            _ => {}
        }
        if let Some(u) = s.underline.filter(|u| u.on()) {
            let mut st = Vec::new();
            let thick = match u.0 {
                UnderlineStyle::Thick => "1.2pt",
                _ => "0.6pt",
            };
            st.push(format!("thickness: {thick}"));
            if let Some(c) = &s.underline_color {
                st.push(format!("paint: {}", color(c)));
            }
            match u.0 {
                UnderlineStyle::Dotted => st.push("dash: \"dotted\"".into()),
                UnderlineStyle::Dashed => st.push("dash: \"dashed\"".into()),
                UnderlineStyle::Double | UnderlineStyle::Wavy => {
                    self.warn
                        .unsupported(FORMAT, "underline: double/wavy", &r.text)
                }
                _ => {}
            }
            piece = format!("#underline(stroke: ({}))[{piece}]", st.join(", "));
        }
        match s.strike {
            Some(Strike::Single) => piece = format!("#strike[{piece}]"),
            Some(Strike::Double) => {
                self.warn.unsupported(FORMAT, "strike: double", &r.text);
                piece = format!("#strike[{piece}]");
            }
            _ => {}
        }
        if let Some(h) = &s.highlight {
            piece = format!("#highlight(fill: {})[{piece}]", color(h));
        }
        if let Some(bg) = &s.background {
            piece = format!(
                "#box(fill: {}, inset: (x: 2pt), outset: (y: 2pt), radius: 2pt)[#set text(top-edge: \"ascender\", bottom-edge: \"descender\");{piece}]",
                bg_src(bg)
            );
        }
        if let Some(h) = &r.href {
            piece = format!("#link({})[{piece}]", lit(h));
        }
        piece
    }

    fn code(&mut self, n: &Node, p: &CodeProps, parent: &Style) {
        let s = &n.style;
        let mut body = format!(
            "#show raw: set text(font: {}, size: {}pt)\n",
            font_list(s, true),
            s.size_pt()
        );
        if p.line_numbers {
            body.push_str(
                "#show raw.line: it => [#box(width: 2.5em)[#text(fill: rgb(\"#999999\"))[#it.number]]#it.body]\n",
            );
        }
        let _ = write!(body, "#raw({}, block: true)", lit(&p.code));
        // 字体已经给 raw 了，外层不再设
        let mut outer = s.clone();
        outer.font_family = parent.font_family.clone();
        outer.font_size = parent.font_size;
        self.block(body, &outer, parent, &n.id);
    }

    fn list(&mut self, n: &Node, p: &ListProps, parent: &Style) {
        let lines = common::flatten_list(n, &p.items, p.ordered);
        let marker_part = n.part("marker");
        let item_part = n.part("item");
        let mut body = String::new();
        for l in &lines {
            let s = &l.style;
            let fs = s.size_pt();
            let indent = s.list_indent.map(|x| x.pt(fs, 0.0)).unwrap_or(18.0) * l.level as f32;
            let hanging = s.list_hanging.map(|x| x.pt(fs, 0.0)).unwrap_or(14.0);
            let mut item = s.clone();
            item.merge(&item_part);
            let mut ms = item.text_only();
            ms.background = None;
            if let Some(c) = &s.list_marker_color {
                ms.color = Some(c.clone());
            }
            if let Some(f) = &s.list_marker_font {
                ms.font_family = Some(crate::style::FontFamily {
                    latin: Some(f.clone()),
                    east_asia: Some(f.clone()),
                    complex: None,
                });
            }
            if let Some(pct) = s.list_marker_size {
                ms.font_size = Some(Len::Pt(fs * pct / 100.0));
            }
            ms.merge(&marker_part);
            let margs = self.text_args(&ms, &item.text_only(), &n.id);
            let mut marker = match &s.list_marker_image {
                Some(src) => match self.asset_path(&n.id, src) {
                    Some(path) => format!("#box(image({}, height: 0.8em))", lit(&path)),
                    None => format!("#{}", lit("•")),
                },
                None => format!("#{}", lit(&l.marker)),
            };
            if !margs.is_empty() {
                marker = format!("#text({margs})[{marker}]");
            }
            let gap = match s.list_marker_gap {
                Some(MarkerGap::Len(g)) => g.pt(fs, 0.0),
                Some(MarkerGap::Space) => fs * 0.3,
                Some(MarkerGap::None) => 0.0,
                _ => fs * 0.5,
            };
            // 编号在编号框里的位置；右对齐时空隙留在右边
            let fill = match s.list_marker_align {
                Some(HAlign::Center) => "[#h(1fr)#m#h(1fr)]".to_string(),
                Some(HAlign::Right) => format!("[#h(1fr)#m#h({gap}pt)]"),
                _ => "[#m#h(1fr)]".to_string(),
            };
            let text = self.inline_in(l.item.text(), &n.style, &item);
            // 编号比悬挂缩进宽时（如「十二、」），编号框跟着变宽，像 Word 跳到下一个制表位
            let line = format!(
                "#set par(hanging-indent: {hanging}pt, first-line-indent: 0pt)\n#par[#context {{ let m = [{marker}]; box(width: calc.max({hanging}pt, measure([#m#h({gap}pt)#h(0pt)]).width + 1pt), {fill}) }}{text}]"
            );
            let mut ls = item.clone();
            ls.margin_left = Some(Len::Pt(indent - hanging));
            ls.text_indent = None;
            ls.hanging_indent = None;
            ls.space_before = None;
            ls.space_after = None;
            ls.background = None;
            ls.border = None;
            ls.padding = None;
            body.push_str(&self.block_src(line, &ls, &n.style, &n.id));
        }
        self.block(body, &n.style, parent, &n.id);
    }

    fn table(&mut self, n: &Node, p: &TableProps, parent: &Style) {
        let g = Grid::of(p);
        let s = &n.style;
        let widths: Vec<String> = common::col_widths(p, &g, &mut self.warn, &n.id)
            .iter()
            .map(|w| match w {
                ColWidth::Ratio(x) => format!("{x}fr"),
                ColWidth::Fixed(x) => format!("{x}pt"),
                ColWidth::Auto => "auto".to_string(),
            })
            .collect();
        let pad = s
            .cell_padding
            .map(|x| x.pt(s.size_pt(), 0.0))
            .unwrap_or(4.0);
        let mut header = Vec::new();
        let mut body = Vec::new();
        g.warn_ragged(&mut self.warn, &n.id);
        let mut exact_rows = Vec::new();
        for r in 0..g.rows {
            let rh = g.row_height(p, r);
            exact_rows.push(match rh {
                Some(h) if h.exact => format!("{}pt", h.height),
                _ => "auto".to_string(),
            });
            for c in 0..g.cols {
                if !g.is_origin(r, c) {
                    continue;
                }
                let sl = *g.slot(r, c);
                let part = common::cell_parts(n, &p.look, &g, r, c);
                let mut cs = s.inherited();
                cs.merge(&part);
                let mut text = self.inline_in(sl.text(), s, &cs);
                let mut sets = self.text_args(&cs, s, &n.id);
                if !sets.is_empty() {
                    sets = format!("#set text({sets})\n");
                }
                if let Some(a) = cs.align.filter(|a| s.align != Some(*a)) {
                    let a = match a {
                        Align::Center => "center",
                        Align::Right => "right",
                        _ => "left",
                    };
                    let _ = writeln!(sets, "#set align({a})");
                }
                let mut args = Vec::new();
                if let Some(bg) = &part.background {
                    args.push(format!("fill: {}", bg_src(bg)));
                }
                let b = common::span_edges(n, &p.look, &g, r, c);
                if sl.row_span > 1 {
                    args.push(format!("rowspan: {}", sl.row_span));
                }
                if sl.col_span > 1 {
                    args.push(format!("colspan: {}", sl.col_span));
                }
                let st: Vec<String> = ["top", "right", "bottom", "left"]
                    .iter()
                    .zip(b.iter())
                    .map(|(k, b)| format!("{k}: {}", stroke(b.as_ref())))
                    .collect();
                args.push(format!("stroke: ({})", st.join(", ")));
                let pads: Vec<String> = Edge::ALL
                    .iter()
                    .zip(["top", "right", "bottom", "left"])
                    .map(|(e, k)| format!("{k}: {}pt", part.padding_side(*e).unwrap_or(pad)))
                    .collect();
                args.push(format!("inset: ({})", pads.join(", ")));
                if let Some(v) = part.vertical_align {
                    args.push(format!(
                        "align: {}",
                        match v {
                            VerticalAlign::Middle => "horizon",
                            VerticalAlign::Bottom => "bottom",
                            _ => "top",
                        }
                    ));
                }
                let ins: Vec<f32> = Edge::ALL
                    .iter()
                    .map(|e| part.padding_side(*e).unwrap_or(pad))
                    .collect();
                let dbl = double_lines(
                    [b[0].as_ref(), b[1].as_ref(), b[2].as_ref(), b[3].as_ref()],
                    [ins[0], ins[1], ins[2], ins[3]],
                );
                // 最小行高：Typst 的行只有固定高度，最小高度靠把内容撑到那么高（只看单行的格子）
                if let Some(h) = rh.filter(|h| !h.exact && sl.row_span == 1) {
                    let min = (h.height - ins[0] - ins[2]).max(0.0);
                    let va = match part.vertical_align {
                        Some(VerticalAlign::Middle) => "horizon",
                        Some(VerticalAlign::Bottom) => "bottom",
                        _ => "top",
                    };
                    text = format!(
                        "#layout(sz => {{ let c = [{sets}{text}]; let h = measure(block(width: sz.width, c)).height; block(height: calc.max(h, {min}pt), width: 100%, align({va}, c)) }})"
                    );
                    sets = String::new();
                }
                let cell = format!("table.cell({})[{dbl}{sets}{text}]", args.join(", "));
                if g.has_header && r == 0 {
                    header.push(cell);
                } else {
                    body.push(cell);
                }
            }
        }
        if s.row_break == Some(false) {
            self.warn.unsupported(FORMAT, "rowBreak", &n.id);
        }
        let mut all = Vec::new();
        if !header.is_empty() {
            all.push(format!(
                "table.header(repeat: {}, {})",
                s.repeat_header.unwrap_or(true),
                header.join(", ")
            ));
        }
        all.extend(body);
        let src = format!(
            "#table(columns: ({},), rows: ({},), stroke: none, inset: 0pt, {})",
            widths.join(", "),
            exact_rows.join(", "),
            all.join(", ")
        );
        // 边框和背景已经落到格子上
        let mut outer = s.clone();
        outer.border = None;
        outer.border_top = None;
        outer.border_right = None;
        outer.border_bottom = None;
        outer.border_left = None;
        outer.border_inside_h = None;
        outer.border_inside_v = None;
        outer.padding = None;
        self.block(src, &outer, parent, &n.id);
    }

    /// 本地图片放进虚拟文件，返回路径；失败时给警告。
    fn asset_path(&mut self, id: &str, src: &str) -> Option<String> {
        let loaded = if assets::is_remote(src) {
            Err("pdf 暂不支持远程图片".to_string())
        } else {
            assets::load(src, self.opts.base_dir.as_deref()).map_err(|e| format!("{e:#}"))
        };
        match loaded {
            Ok(a) => {
                let ext = match a.mime {
                    "image/png" => "png",
                    "image/jpeg" => "jpg",
                    "image/gif" => "gif",
                    "image/svg+xml" => "svg",
                    "image/webp" => "webp",
                    _ => "bin",
                };
                let path = format!("/assets/{}.{ext}", self.files.len());
                self.files.push((path.clone(), a.bytes));
                Some(path)
            }
            Err(why) => {
                self.warn
                    .push(format!("图片 `{id}`：{why}，改成显示 alt 文字"));
                None
            }
        }
    }

    fn image(&mut self, n: &Node, p: &ImageProps, parent: &Style) {
        let s = &n.style;
        let Some(path) = self.asset_path(&n.id, &p.src) else {
            let alt = if p.alt.is_empty() { &p.src } else { &p.alt };
            self.block(
                format!("#emph[#{}]", lit(&format!("[{alt}]"))),
                s,
                parent,
                &n.id,
            );
            return;
        };
        let mut args = vec![lit(&path)];
        let fs = s.size_pt();
        match (s.width, p.width) {
            (Some(Len::Pct(x)), _) => args.push(format!("width: {x}%")),
            (Some(w), _) => args.push(format!("width: {}pt", w.pt(fs, 0.0))),
            (None, Some(w)) => args.push(format!("width: {w}pt")),
            _ => {}
        }
        match (s.height, p.height) {
            (Some(Len::Pct(x)), _) => args.push(format!("height: {x}%")),
            (Some(h), _) => args.push(format!("height: {}pt", h.pt(fs, 0.0))),
            (None, Some(h)) => args.push(format!("height: {h}pt")),
            _ => {}
        }
        if let Some(f) = s.object_fit {
            args.push(format!(
                "fit: \"{}\"",
                match f {
                    crate::style::ObjectFit::Fill => "stretch",
                    f => f.as_str(),
                }
            ));
        }
        if !p.alt.is_empty() {
            args.push(format!("alt: {}", lit(&p.alt)));
        }
        let mut img = format!("image({})", args.join(", "));
        if let Some(r) = s.rotation {
            img = format!("rotate({r}deg, reflow: true)[#{img}]");
        }
        if s.opacity.is_some() {
            self.warn.unsupported(FORMAT, "opacity", &n.id);
        }
        let mut fig = match &p.caption {
            Some(c) => {
                let mut cs = s.inherited();
                cs.merge(&n.part("caption"));
                let cap = format!("#{}", lit(c));
                let args = self.text_args(&cs, s, &n.id);
                let cap = if args.is_empty() {
                    cap
                } else {
                    format!("#text({args})[{cap}]")
                };
                format!("#figure({img}, caption: [{cap}], gap: 4pt)")
            }
            None => format!("#box({img})"),
        };
        let mut st = s.clone();
        st.width = None;
        st.height = None;
        match s.position.unwrap_or(Position::Inline) {
            Position::Inline => {
                if let Some(a) = s.horizontal_align {
                    fig = format!("#align({})[{fig}]", a.as_str());
                }
                self.block(fig, &st, parent, &n.id);
            }
            Position::Float => {
                // typst 不能让文字绕排图片，按左右对齐的独立块处理
                if !matches!(s.wrap, Some(Wrap::Image(WrapMode::TopAndBottom))) {
                    self.warn
                        .unsupported(FORMAT, "文字绕排（position: float）", &n.id);
                }
                let side = match s.float.unwrap_or(Side::Left) {
                    Side::Left => "left",
                    Side::Right => "right",
                };
                fig = format!("#align({side})[{fig}]");
                self.block(fig, &st, parent, &n.id);
            }
            Position::Absolute => {
                let mut x = s.x.map(|v| v.abs_pt()).unwrap_or(0.0);
                let mut y = s.y.map(|v| v.abs_pt()).unwrap_or(0.0);
                if s.anchor_to == Some(AnchorTo::Page) {
                    x -= self.margins[3];
                    y -= self.margins[0];
                }
                let al = match (s.horizontal_align, s.vertical_align) {
                    (Some(h), _) => format!("top + {}", h.as_str()),
                    (_, Some(VerticalAlign::Bottom)) => "bottom + left".into(),
                    _ => "top + left".into(),
                };
                let _ = writeln!(self.out, "#place({al}, dx: {x}pt, dy: {y}pt)[{fig}]");
            }
        }
    }
}

/// 某一边的 typst stroke。
fn stroke(b: Option<&Border>) -> String {
    match b {
        None => "none".into(),
        Some(b) if b.is_none() => "none".into(),
        Some(b) => {
            let dash = match b.style {
                BorderStyle::Dashed => ", dash: \"dashed\"",
                BorderStyle::Dotted => ", dash: \"dotted\"",
                BorderStyle::DashDot => ", dash: \"dash-dotted\"",
                _ => "",
            };
            // 双线：Typst 只有单线，这里画外面那条（线宽 1/3），里面那条由 double_lines 补
            let w = if b.style == BorderStyle::Double {
                b.width / 3.0
            } else {
                b.width
            };
            format!("(paint: {}, thickness: {w}pt{dash})", color(&b.color))
        }
    }
}

/// 双线边框里面那条线：按 CSS 的画法，两条线和中间空隙各占线宽的 1/3。
/// 外线由 stroke 画在边上，这里在盒子内容里用 place 往里挪 2/3 线宽再画一条。
/// `sides` 按上右下左，`inset` 是盒子的内边距（place 以内容区为准，要先退回边上）。
fn double_lines(sides: [Option<&Border>; 4], inset: [f32; 4]) -> String {
    let [t, r, b, l] = inset;
    let mut o = String::new();
    for (i, side) in sides.iter().enumerate() {
        let Some(bd) = side.filter(|x| x.style == BorderStyle::Double && !x.is_none()) else {
            continue;
        };
        let w = bd.width / 3.0;
        let off = bd.width * 2.0 / 3.0;
        let st = format!("(paint: {}, thickness: {w}pt)", color(&bd.color));
        let _ = match i {
            0 => write!(
                o,
                "#place(top + left, dx: {}pt, dy: {}pt, line(length: 100% + {}pt, stroke: {st}))",
                -l,
                off - t,
                l + r
            ),
            2 => write!(
                o,
                "#place(bottom + left, dx: {}pt, dy: {}pt, line(length: 100% + {}pt, stroke: {st}))",
                -l,
                b - off,
                l + r
            ),
            1 => write!(
                o,
                "#place(top + right, dx: {}pt, dy: {}pt, line(angle: 90deg, length: 100% + {}pt, stroke: {st}))",
                r - off,
                -t,
                t + b
            ),
            _ => write!(
                o,
                "#place(top + left, dx: {}pt, dy: {}pt, line(angle: 90deg, length: 100% + {}pt, stroke: {st}))",
                off - l,
                -t,
                t + b
            ),
        };
    }
    o
}

fn strokes(s: &Style) -> Vec<String> {
    Edge::ALL
        .iter()
        .zip(["top", "right", "bottom", "left"])
        .filter_map(|(e, k)| {
            s.border_side(*e)
                .map(|b| format!("{k}: {}", stroke(Some(b))))
        })
        .collect()
}

fn color(c: &Color) -> String {
    format!("rgb(\"{}\")", c.css())
}

fn bg_src(b: &Background) -> String {
    match b {
        Background::Gradient { angle, stops } => format!(
            "gradient.linear({}, angle: {}deg)",
            stops.iter().map(color).collect::<Vec<_>>().join(", "),
            angle - 90.0
        ),
        b => color(&b.solid()),
    }
}

fn lang_code(l: &str) -> String {
    l.split(['-', '_'])
        .next()
        .unwrap_or("zh")
        .to_ascii_lowercase()
}

/// Typst 字符串字面量。
fn lit(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn lits(xs: &[&str]) -> String {
    let v: Vec<String> = xs.iter().map(|x| lit(x)).collect();
    format!("({},)", v.join(", "))
}

/// 样式里的字体（逗号分开的也拆开），后面接默认字体兜底。
fn font_list(s: &Style, mono: bool) -> String {
    let mut v: Vec<String> = Vec::new();
    if let Some(f) = &s.font_family {
        for x in f.fallback() {
            for y in x.split(',') {
                let y = y.trim().to_string();
                if !y.is_empty() && !v.contains(&y) {
                    v.push(y);
                }
            }
        }
    }
    if mono {
        v.extend(MONO_FONTS.iter().map(|x| x.to_string()));
    }
    v.extend(FONTS.iter().map(|x| x.to_string()));
    let refs: Vec<&str> = v.iter().map(String::as_str).collect();
    lits(&refs)
}

/// 字体只扫一次：typst 自带字体加系统字体。
fn font_store() -> &'static FontStore {
    static STORE: OnceLock<FontStore> = OnceLock::new();
    STORE.get_or_init(|| {
        let mut s = FontStore::new();
        s.extend(fonts::embedded());
        let mut sys: Vec<_> = fonts::system().collect();
        // 一族里只要有一个字重不是整百，整族都按名字归一，免得 Heavy 正好是 700 抢了 Bold
        let odd: std::collections::HashSet<String> = sys
            .iter()
            .filter(|(_, i)| i.variant.weight.to_number() % 100 != 0)
            .map(|(_, i)| i.family.clone())
            .collect();
        for (path, info) in sys.iter_mut() {
            if odd.contains(&info.family) {
                normalize_weight(info, &path.path, path.index);
            }
        }
        s.extend(sys);
        s
    })
}

/// 有的字体 OS/2 里的字重不是整百，比如 MiSans：Regular 330、Medium 380、Bold 630。
/// typst 按数值找最近的字重，要 400 会拿到 Medium，要 700 会拿到 Heavy。
/// 这种字体整族按各自的字重名（Regular、Bold……）改成标准数值；认不出的名字不动。
fn normalize_weight(info: &mut FontInfo, path: &std::path::Path, index: u32) {
    let Ok(data) = std::fs::read(path) else {
        return;
    };
    let Ok(face) = ttf_parser::Face::parse(&data, index) else {
        return;
    };
    let name = |id: u16| {
        face.names()
            .into_iter()
            .filter(|n| n.name_id == id && n.is_unicode())
            .find_map(|n| n.to_string())
    };
    let Some(sub) = name(ttf_parser::name_id::TYPOGRAPHIC_SUBFAMILY)
        .or_else(|| name(ttf_parser::name_id::SUBFAMILY))
    else {
        return;
    };
    let key: String = sub
        .to_lowercase()
        .replace("italic", "")
        .replace("oblique", "")
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect();
    let w = match key.as_str() {
        "thin" | "hairline" => 100,
        "extralight" | "ultralight" => 200,
        "light" => 300,
        "regular" | "book" | "" => 400,
        "medium" => 500,
        "semibold" | "demibold" => 600,
        "bold" => 700,
        "extrabold" | "ultrabold" => 800,
        "heavy" | "black" => 900,
        _ => return,
    };
    info.variant.weight = FontWeight::from_number(w);
}

fn library() -> &'static LazyHash<Library> {
    static LIB: OnceLock<LazyHash<Library>> = OnceLock::new();
    LIB.get_or_init(|| LazyHash::new(Library::default()))
}

struct FmtagnoWorld {
    main: Source,
    files: HashMap<FileId, Bytes>,
}

fn file_id(path: &str) -> FileId {
    let vpath = VirtualPath::new(path).expect("内部生成的虚拟路径应当合法");
    RootedPath::new(VirtualRoot::Project, vpath).intern()
}

impl FmtagnoWorld {
    fn new(source: String, files: Files) -> Self {
        let main = Source::new(file_id("/main.typ"), source);
        let files = files
            .into_iter()
            .map(|(p, b)| (file_id(&p), Bytes::new(b)))
            .collect();
        Self { main, files }
    }
}

impl World for FmtagnoWorld {
    fn library(&self) -> &LazyHash<Library> {
        library()
    }

    fn book(&self) -> &LazyHash<FontBook> {
        font_store().book()
    }

    fn main(&self) -> FileId {
        self.main.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.get(&id).cloned().ok_or(FileError::AccessDenied)
    }

    fn font(&self, index: usize) -> Option<Font> {
        font_store().font(index)
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}
