//! HTML 渲染器：一个自带 CSS 的完整页面，能直接打开，也能打印。

use std::fmt::Write;

use base64::Engine;

use crate::{
    assets,
    components::{CodeProps, Component, Field, ImageProps, Inline, ListProps, Run, TableProps},
    doc::{Doc, Node, SectionLayout},
    render::{
        Options, Output,
        common::{self, Grid, Warn},
    },
    style::{
        Align, AnchorTo, Background, Caps, ColWidth, Edge, EmphasisMark, Len, LineHeight,
        MarkerGap, Position, Side, Strike, Style, UnderlineStyle, VerticalAlign, Wrap, WrapMode,
    },
};

const FORMAT: &str = "html";
const DEFAULT_FONTS: &str = "-apple-system, \"Helvetica Neue\", Arial, \"PingFang SC\", \"Noto Sans CJK SC\", \"Noto Sans SC\", \"Microsoft YaHei\", sans-serif";

pub fn render(doc: &Doc, tree: &Node, opts: &Options) -> Output {
    let mut r = Html {
        out: String::new(),
        warn: Warn::default(),
        opts,
        sec_css: String::new(),
    };
    let lang = doc.meta.lang.as_deref().unwrap_or("zh-CN");
    let title = doc.meta.title.as_deref().unwrap_or("fmtagno");
    let page = &doc.meta.page;
    let mut ws = Vec::new();
    let (w, h) = page.dims_pt(&mut ws);
    for x in ws {
        r.warn.push(x);
    }
    let [mt, mr, mb, ml] = page.margins_pt();
    let gutter = page.gutter.map(|g| g.abs_pt()).unwrap_or(0.0);
    let _ = write!(
        r.out,
        "<!doctype html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"generator\" content=\"fmtagno {}\">\n",
        esc(lang),
        env!("CARGO_PKG_VERSION")
    );
    if let Some(a) = &doc.meta.author {
        let _ = writeln!(r.out, "<meta name=\"author\" content=\"{}\">", esc(a));
    }
    let cols = "/*fmtagno-sections*/\n";
    let _ = write!(
        r.out,
        "<title>{}</title>\n<style>\n@page {{ size: {w}pt {h}pt; margin: {mt}pt {mr}pt {mb}pt {}pt; }}\n\
         body {{ font-family: {DEFAULT_FONTS}; color: #1a1a1a; max-width: {}pt; margin: 2rem auto; padding: 0 1.5rem; }}\n\
         {cols}{}</style>\n{}</head>\n<body>\n",
        esc(title),
        ml + gutter,
        w - ml - mr - gutter,
        BASE_CSS,
        if opts.paged { PAGED_CSS } else { "" }
    );
    r.node(tree, None);
    if opts.paged {
        // 放在 body 末尾：Paged.js 加载完就把上面的内容按 @page 切成一页页
        r.out.push_str("<script>\n");
        r.out.push_str(PAGED_JS);
        r.out.push_str("\n</script>\n<script>\n");
        r.out.push_str(PAGED_REPEAT_THEAD);
        r.out.push('\n');
        r.out.push_str(PAGED_HF);
        r.out.push_str("\n</script>\n");
    }
    r.out.push_str("</body>\n</html>\n");
    r.out = r.out.replacen("/*fmtagno-sections*/\n", &r.sec_css, 1);
    Output {
        bytes: r.out.into_bytes(),
        warnings: r.warn.list,
    }
}

const BASE_CSS: &str = "\
* { box-sizing: border-box; }
h1, h2, h3, h4, h5, h6, p, pre, ul, ol, figure, table { margin: 0; }
table { border-collapse: collapse; }
th, td { text-align: inherit; vertical-align: top; font-weight: inherit; }
figure img { max-width: 100%; height: auto; display: block; }
pre { white-space: pre-wrap; }
code { font: inherit; }
.fmtagno-list { list-style: none; padding: 0; }
.fmtagno-marker { display: inline-block; white-space: nowrap; }
.fmtagno-ln { display: inline-block; min-width: 2.5em; color: #999; user-select: none; }
.fmtagno-pagebreak { break-after: page; }
.fmtagno-section + .fmtagno-section { break-before: page; }
.fmtagno-slide { aspect-ratio: 16 / 9; border: 1px solid #ddd; border-radius: 6px; padding: 2rem 3rem; \
margin: 1.5rem 0; overflow: hidden; break-inside: avoid; break-after: page; position: relative; }
@media screen { .fmtagno-pagebreak { border-top: 1px dashed #bbb; margin: 2rem 0; } \
.fmtagno-section + .fmtagno-section { border-top: 1px dashed #bbb; margin-top: 2rem; padding-top: 2rem; } }
@media print { body { max-width: none; margin: 0; padding: 0; } .fmtagno-slide { border: none; } }
";

/// Paged.js 0.4.3（MIT，许可证见 assets/pagedjs-LICENSE.md），`--paged` 时内嵌进 HTML。
const PAGED_JS: &str = include_str!("../../assets/paged.polyfill.min.js");

/// Paged.js 自己不会在续页重复 thead：表格被切开后，给续上的那半张表补一份表头。
/// repeatHeader: false 时表头不放进 thead，这里自然不会补。
const PAGED_REPEAT_THEAD: &str = "\
class FmtagnoRepeatThead extends Paged.Handler {
  // 续页上放第一行之前补表头，排版时就把表头的高度算进去，不会挤掉最后一行
  renderNode(clone, node) {
    if (clone.nodeType !== 1 || clone.tagName !== 'TR') return;
    const t = clone.closest('table[data-split-from]');
    if (!t || t.querySelector(':scope > thead')) return;
    const th = node.closest('table')?.querySelector(':scope > thead');
    if (!th) return;
    const copy = th.cloneNode(true);
    for (const e of [copy, ...copy.querySelectorAll('[data-ref]')]) e.removeAttribute('data-ref');
    t.insertBefore(copy, t.querySelector(':scope > tbody') || t.firstChild);
  }
}
Paged.registerHandlers(FmtagnoRepeatThead);";

/// 按页显示时的屏幕样式：灰底、白纸、阴影；正文不再限宽，屏幕上的分页虚线也不要了。
/// 带 data-pagedjs-ignore，Paged.js 不去改写它。
const PAGED_CSS: &str = "<style data-pagedjs-ignore>
@media screen {
  body { max-width: none; margin: 0; padding: 0; background: #e8eaed; }
  .pagedjs_pages { display: flex; flex-direction: column; align-items: center; padding: 24px 0; }
  .pagedjs_page { background: #fff; box-shadow: 0 1px 4px rgba(0,0,0,.25); margin-bottom: 24px; }
  .fmtagno-pagebreak { border-top: none; margin: 0; }
  .fmtagno-section + .fmtagno-section { border-top: none; margin-top: 0; padding-top: 0; }
}
.pagedjs_pagebox { position: relative; isolation: isolate; }
.pagedjs_area { position: relative; }
.fmtagno-hf-box { position: absolute; }
/* 排版时先放在原地不占位，免得 Paged.js 以为它溢出了页面；排完再挪回坐标上 */
.fmtagno-abs:not(.fmtagno-placed) { top: 0 !important; left: 0 !important; visibility: hidden; }
@media print { body { background: none; } }
</style>
<style>
/* Paged.js 不认空 div 上的 break-after，改成让下一个元素从新页开始 */
.fmtagno-pagebreak + * { break-before: page; }
</style>
";

/// 页眉页脚和页码：Paged.js 排完所有页后，按每页属于哪节、是不是本节首页、页码奇偶，
/// 把对应的 <template> 内容放进页面的边距里，再填好页码域。
/// 页码按 Word 的规则自己数：每节接着上一节数，写了 data-page-start 就从那个数重新开始。
const PAGED_HF: &str = r#"class FmtagnoHeaders extends Paged.Handler {
  beforeParsed(content) {
    this.tpls = {};
    for (const t of content.querySelectorAll('template.fmtagno-hf')) {
      this.tpls[t.dataset.section + ':' + t.dataset.kind] = t.content;
      t.remove();
    }
  }
  afterRendered(pages) {
    const info = [];
    let cur = null;
    pages.forEach((p, i) => {
      const s = p.element.querySelector('.fmtagno-section[data-section]');
      const id = s ? s.dataset.section : (cur ? cur.id : '0');
      let first = false;
      if (!cur || id !== cur.id) {
        const start = s && s.dataset.pageStart ? +s.dataset.pageStart
          : (cur ? cur.num + (i - cur.page) : 1);
        cur = { id, page: i, num: start, el: s, count: 0 };
        first = true;
      }
      cur.count++;
      info.push({ p, sec: cur, first, num: cur.num + (i - cur.page) });
    });
    const fill = (root, x) => {
      for (const f of root.querySelectorAll('.fmtagno-field')) {
        const k = f.dataset.field;
        f.textContent = k === 'page' ? x.num : k === 'pages' ? pages.length : x.sec.count;
      }
    };
    for (const x of info) {
      const s = x.sec.el;
      if (s) {
        const [hd, fd, left, width] = s.dataset.hf.split(' ');
        const kind = x.first && 'titlePage' in s.dataset ? 'first'
          : 'evenOdd' in s.dataset && x.num % 2 === 0 ? 'even' : 'default';
        const box = x.p.element.querySelector('.pagedjs_pagebox');
        for (const [part, pos, d] of [['header', 'top', hd], ['footer', 'bottom', fd]]) {
          const t = this.tpls[x.sec.id + ':' + part + '-' + kind];
          if (!t || !box) continue;
          const div = document.createElement('div');
          div.className = 'fmtagno-hf-box fmtagno-' + part;
          div.style.cssText = `${pos}:${d}pt;left:${left}pt;width:${width}pt`;
          div.appendChild(t.cloneNode(true));
          box.appendChild(div);
        }
      }
      fill(x.p.element, x);
    }
    // 绝对定位的图挪到它所在页的纸面（anchorTo: page）或版心（margin）上，用原来写的坐标
    for (const x of info) {
      const box = x.p.element.querySelector('.pagedjs_pagebox');
      const area = x.p.element.querySelector('.pagedjs_area');
      for (const f of x.p.element.querySelectorAll('.fmtagno-abs')) {
        const to = f.dataset.anchor === 'page' ? box : area;
        if (!to) continue;
        f.classList.add('fmtagno-placed');
        to.appendChild(f);
      }
    }
    // 排完的标记，截图、打印脚本可以等它
    document.documentElement.dataset.fmtagnoReady = '1';
  }
}
Paged.registerHandlers(FmtagnoHeaders);"#;

struct Html<'a> {
    out: String,
    warn: Warn,
    opts: &'a Options,
    /// 各节的 @page 和分栏规则，最后填进 <style>
    sec_css: String,
}

impl Html<'_> {
    fn node(&mut self, n: &Node, parent: Option<&Style>) {
        let id = esc(&n.id);
        let st = self.attr(&n.style, parent, &n.id, Ctx::Block);
        let lang = n
            .style
            .lang
            .as_ref()
            .filter(|l| parent.and_then(|p| p.lang.as_ref()) != Some(l))
            .map(|l| format!(" lang=\"{}\"", esc(l)))
            .unwrap_or_default();
        let st = format!("{st}{lang}");
        match &n.component {
            Component::Document(_) => {
                let cls = if parent.is_none() {
                    " class=\"fmtagno-root\""
                } else {
                    ""
                };
                let _ = writeln!(self.out, "<main{cls} id=\"{id}\"{st}>");
                self.children(n);
                self.out.push_str("</main>\n");
            }
            Component::Slide(p) => {
                let _ = writeln!(
                    self.out,
                    "<section class=\"fmtagno-slide\" id=\"{id}\"{st}>"
                );
                if let Some(t) = &p.title {
                    let mut ts = n.style.clone();
                    ts.merge(&n.part("title"));
                    let a = self.attr(&ts, Some(&n.style), &n.id, Ctx::Block);
                    let _ = writeln!(self.out, "<h2{a}>{}</h2>", esc(t));
                }
                let body = n.part("body");
                if body.is_empty() {
                    self.children(n);
                } else {
                    let mut bs = n.style.clone();
                    bs.merge(&body);
                    let a = self.attr(&bs, Some(&n.style), &n.id, Ctx::Block);
                    let _ = writeln!(self.out, "<div{a}>");
                    for c in &n.children {
                        self.node(c, Some(&bs));
                    }
                    self.out.push_str("</div>\n");
                }
                self.out.push_str("</section>\n");
            }
            Component::Heading(p) => {
                let _ = write!(self.out, "<h{l} id=\"{id}\"{st}>", l = p.level);
                self.inline(&p.text, &n.style, &n.id);
                let _ = writeln!(self.out, "</h{}>", p.level);
            }
            Component::Text(p) => {
                let _ = write!(self.out, "<p id=\"{id}\"{st}>");
                self.inline(&p.text, &n.style, &n.id);
                self.out.push_str("</p>\n");
            }
            Component::Code(p) => self.code(n, p, &st),
            Component::List(p) => self.list(n, p, &st),
            Component::Table(p) => self.table(n, p, parent),
            Component::Image(p) => self.image(n, p, parent),
            Component::PageBreak(_) => {
                let _ = writeln!(
                    self.out,
                    "<div class=\"fmtagno-pagebreak\" id=\"{id}\"></div>"
                );
            }
            Component::Section(p) => self.section(n, p.layout(), parent, &st),
        }
    }

    /// 一节：每节一个具名页面（@page fmtagno-sN），页眉页脚写成 <template>，
    /// 只在 --paged 时由脚本放进页边距；直接打开或打印时不显示。
    fn section(&mut self, n: &Node, lay: &SectionLayout, parent: Option<&Style>, st: &str) {
        let page = &lay.page;
        let mut ws = Vec::new();
        let (w, h) = page.dims_pt(&mut ws);
        for x in ws {
            self.warn.push(x);
        }
        let [mt, mr, mb, ml] = page.margins_pt();
        let gutter = page.gutter.map(|g| g.abs_pt()).unwrap_or(0.0);
        let (hd, fd) = page.hf_distance_pt();
        let i = lay.index;
        let _ = writeln!(
            self.sec_css,
            "@page fmtagno-s{i} {{ size: {w}pt {h}pt; margin: {mt}pt {mr}pt {mb}pt {}pt; }}",
            ml + gutter
        );
        let cols = page
            .columns
            .as_ref()
            .filter(|c| c.count() > 1)
            .map(|c| {
                format!(
                    " column-count: {}; column-gap: {}pt;",
                    c.count(),
                    c.gap_pt()
                )
            })
            .unwrap_or_default();
        let _ = writeln!(
            self.sec_css,
            ".fmtagno-section[data-section=\"{i}\"] {{ page: fmtagno-s{i};{cols} }}"
        );
        let mut attrs = format!(
            " data-section=\"{i}\" data-hf=\"{hd} {fd} {} {}\"",
            ml + gutter,
            w - ml - mr - gutter
        );
        if let Some(x) = lay.page_number_start {
            let _ = write!(attrs, " data-page-start=\"{x}\"");
        }
        if lay.title_page {
            attrs.push_str(" data-title-page");
        }
        if lay.even_odd {
            attrs.push_str(" data-even-odd");
        }
        let _ = writeln!(
            self.out,
            "<section class=\"fmtagno-section\" id=\"{}\"{attrs}{st}>",
            esc(&n.id)
        );
        for (part, set) in [("header", &lay.header), ("footer", &lay.footer)] {
            let kinds = [
                ("default", set.default.as_ref(), true),
                ("first", set.for_first(), lay.title_page),
                ("even", set.for_even(), lay.even_odd),
            ];
            for (kind, node, on) in kinds {
                let Some(node) = node.filter(|_| on) else {
                    continue;
                };
                let _ = writeln!(
                    self.out,
                    "<template class=\"fmtagno-hf\" data-section=\"{i}\" data-kind=\"{part}-{kind}\">"
                );
                self.node(node, parent);
                self.out.push_str("</template>\n");
            }
        }
        self.children(n);
        self.out.push_str("</section>\n");
    }

    fn children(&mut self, n: &Node) {
        for c in &n.children {
            self.node(c, Some(&n.style));
        }
    }

    /// ` style="..."`，没有内容时是空串。
    fn attr(&mut self, s: &Style, parent: Option<&Style>, id: &str, ctx: Ctx) -> String {
        let c = self.css(s, parent, id, ctx);
        if c.is_empty() {
            String::new()
        } else {
            format!(" style=\"{}\"", esc(&c))
        }
    }

    fn code(&mut self, n: &Node, p: &CodeProps, st: &str) {
        let class = p
            .language
            .as_ref()
            .map(|l| format!(" class=\"language-{}\"", esc(l)))
            .unwrap_or_default();
        let _ = write!(self.out, "<pre id=\"{}\"{st}><code{class}>", esc(&n.id));
        if p.line_numbers {
            for (i, line) in p.code.lines().enumerate() {
                if i > 0 {
                    self.out.push('\n');
                }
                let _ = write!(
                    self.out,
                    "<span class=\"fmtagno-ln\">{}</span>{}",
                    i + 1,
                    esc(line)
                );
            }
        } else {
            self.out.push_str(&esc(&p.code));
        }
        self.out.push_str("</code></pre>\n");
    }

    /// 列表：编号自己算好写出来，这样 listMarkerFormat、对齐、间距在各格式一致。
    fn list(&mut self, n: &Node, p: &ListProps, st: &str) {
        let lines = common::flatten_list(n, &p.items, p.ordered);
        let _ = writeln!(
            self.out,
            "<div class=\"fmtagno-list\" id=\"{}\"{st}>",
            esc(&n.id)
        );
        let marker_part = n.part("marker");
        let item_part = n.part("item");
        for l in &lines {
            let s = &l.style;
            let fs = s.size_pt();
            let indent = s.list_indent.map(|x| x.pt(fs, 0.0)).unwrap_or(18.0) * l.level as f32;
            let hanging = s.list_hanging.map(|x| x.pt(fs, 0.0)).unwrap_or(14.0);
            let mut item = s.clone();
            // 段前段后、边框背景在外层容器上，列表项只管自己写的
            item.space_before = None;
            item.space_after = None;
            item.background = None;
            item.border = None;
            item.padding = None;
            item.merge(&item_part);
            let mut css = self.css(&item, Some(&n.style), &n.id, Ctx::ListItem);
            let _ = write!(css, "padding-left:{indent}pt;text-indent:-{hanging}pt;");
            let _ = write!(self.out, "<div style=\"{}\">", esc(&css));
            // 编号
            let mut ms = Style::default();
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
                ms.font_size = Some(Len::Em(pct / 100.0));
            }
            ms.merge(&marker_part);
            let mut mcss = self.css(&ms, None, &n.id, Ctx::Inline);
            let gap = match s.list_marker_gap {
                Some(MarkerGap::Len(g)) => g.pt(fs, 0.0),
                Some(MarkerGap::Space) => fs * 0.3,
                Some(MarkerGap::None) => 0.0,
                Some(MarkerGap::Tab) | None => fs * 0.5,
            };
            let align = s.list_marker_align.map(|a| a.as_str()).unwrap_or("left");
            let _ = write!(
                mcss,
                "text-indent:0;min-width:{hanging}pt;padding-right:{gap}pt;text-align:{align};"
            );
            let _ = write!(
                self.out,
                "<span class=\"fmtagno-marker\" style=\"{}\">",
                esc(&mcss)
            );
            match &s.list_marker_image {
                Some(src) => {
                    let src = self.image_src(&n.id, src);
                    let _ = write!(
                        self.out,
                        "<img src=\"{}\" alt=\"\" style=\"height:0.8em;vertical-align:middle\">",
                        esc(&src)
                    );
                }
                None => self.out.push_str(&esc(&l.marker)),
            }
            self.out.push_str("</span>");
            self.inline_in(l.item.text(), &n.style, &item, &n.id);
            self.out.push_str("</div>\n");
        }
        self.out.push_str("</div>\n");
    }

    fn table(&mut self, n: &Node, p: &TableProps, parent: Option<&Style>) {
        let g = Grid::of(p);
        let s = &n.style;
        let mut ts = s.clone();
        // 表格的边框画在格子上
        ts.border = None;
        ts.border_top = None;
        ts.border_right = None;
        ts.border_bottom = None;
        ts.border_left = None;
        ts.padding = None;
        let mut css = self.css(&ts, parent, &n.id, Ctx::Table);
        let ws = common::col_widths(p, &g, &mut self.warn, &n.id);
        let has_auto = ws.contains(&ColWidth::Auto);
        if s.width.is_none() && common::fills_width(&ws) {
            css.push_str("width:100%;");
        }
        // 有 auto 列时用自动布局，浏览器才会按内容撑开
        let layout =
            s.table_layout
                .map(|l| l.as_str())
                .unwrap_or(if has_auto { "auto" } else { "fixed" });
        let _ = write!(css, "table-layout:{layout};");
        let _ = writeln!(
            self.out,
            "<table id=\"{}\" style=\"{}\">",
            esc(&n.id),
            esc(&css)
        );
        if p.widths.is_some() {
            let fixed: f32 = ws
                .iter()
                .map(|w| if let ColWidth::Fixed(x) = w { *x } else { 0.0 })
                .sum();
            let ratio: f32 = ws
                .iter()
                .map(|w| if let ColWidth::Ratio(x) = w { *x } else { 0.0 })
                .sum();
            self.out.push_str("<colgroup>");
            for w in &ws {
                match w {
                    ColWidth::Fixed(x) => {
                        let _ = write!(self.out, "<col style=\"width:{x}pt\">");
                    }
                    ColWidth::Ratio(x) if fixed > 0.0 || has_auto => {
                        let _ = write!(
                            self.out,
                            "<col style=\"width:calc((100% - {fixed}pt) * {:.4})\">",
                            x / ratio
                        );
                    }
                    ColWidth::Ratio(x) => {
                        let _ = write!(self.out, "<col style=\"width:{:.2}%\">", x / ratio * 100.0);
                    }
                    ColWidth::Auto => self.out.push_str("<col>"),
                }
            }
            self.out.push_str("</colgroup>\n");
        }
        let pad = s
            .cell_padding
            .map(|x| x.pt(s.size_pt(), 0.0))
            .unwrap_or(4.0);
        let empty = Inline::Plain(String::new());
        let head_sep = g.has_header && s.repeat_header.unwrap_or(true);
        if !head_sep {
            self.out.push_str("<tbody>\n");
        }
        for r in 0..g.rows {
            let (cells, header): (&[Inline], bool) = if g.has_header && r == 0 {
                (&p.columns, true)
            } else {
                let i = r - usize::from(g.has_header);
                if p.rows[i].len() != g.cols {
                    self.warn.push(format!(
                        "表格 `{}` 第 {} 行有 {} 格，按 {} 列补齐",
                        n.id,
                        i + 1,
                        p.rows[i].len(),
                        g.cols
                    ));
                }
                (&p.rows[i], false)
            };
            if head_sep && r == 0 {
                self.out.push_str("<thead>");
            }
            let tr = if s.row_break == Some(false) {
                " style=\"break-inside:avoid\""
            } else {
                ""
            };
            let _ = write!(self.out, "<tr{tr}>");
            for c in 0..g.cols {
                let part = common::cell_parts(n, &p.look, &g, r, c);
                let mut cs = s.inherited();
                cs.merge(&part);
                // 先写表格的 cellPadding，部位上写的 padding 跟在后面盖掉对应的边
                let mut css = format!("padding:{pad}pt;");
                css.push_str(&self.css(&part, Some(s), &n.id, Ctx::Cell));
                let borders = common::cell_edges(n, &p.look, &g, r, c);
                for (e, b) in ["top", "right", "bottom", "left"]
                    .iter()
                    .zip(borders.iter())
                {
                    if let Some(b) = b {
                        let _ = write!(css, "border-{e}:{};", b.css());
                    }
                }
                let tag = if header { "th" } else { "td" };
                let a = if css.is_empty() {
                    String::new()
                } else {
                    format!(" style=\"{}\"", esc(&css))
                };
                let _ = write!(self.out, "<{tag}{a}>");
                self.inline_in(cells.get(c).unwrap_or(&empty), s, &cs, &n.id);
                let _ = write!(self.out, "</{tag}>");
            }
            self.out.push_str("</tr>\n");
            if head_sep && r == 0 {
                self.out.push_str("</thead>\n<tbody>\n");
            }
        }
        self.out.push_str("</tbody>\n</table>\n");
    }

    fn image(&mut self, n: &Node, p: &ImageProps, parent: Option<&Style>) {
        let s = &n.style;
        let mut fig = s.clone();
        // 宽高、旋转这些给图片本身
        fig.width = None;
        fig.height = None;
        fig.rotation = None;
        fig.opacity = None;
        fig.object_fit = None;
        let mut css = self.css(&fig, parent, &n.id, Ctx::Block);
        let dist = s.wrap_distance.map(|d| d.abs_pt()).unwrap_or(6.0);
        match s.position.unwrap_or(Position::Inline) {
            Position::Inline => {
                if let Some(a) = s.horizontal_align {
                    let _ = write!(css, "text-align:{};", a.as_str());
                }
            }
            Position::Float => {
                let side = s.float.unwrap_or(Side::Left);
                let (m1, m2) = match side {
                    Side::Left => ("right", "left"),
                    Side::Right => ("left", "right"),
                };
                let _ = write!(
                    css,
                    "float:{};margin-{m1}:{dist}pt;margin-{m2}:0;",
                    side.as_str()
                );
                match s.wrap {
                    Some(Wrap::Image(WrapMode::TopAndBottom)) => {
                        css = css.replace(&format!("float:{};", side.as_str()), "");
                    }
                    Some(Wrap::Image(WrapMode::Behind | WrapMode::InFront | WrapMode::None)) => {
                        self.warn
                            .unsupported(FORMAT, "wrap（浮动时的 behind/inFront/none）", &n.id)
                    }
                    _ => {}
                }
            }
            Position::Absolute => {
                css.push_str("position:absolute;margin:0;");
                if let Some(x) = s.x {
                    let _ = write!(css, "left:{};", len_css(x));
                }
                if let Some(y) = s.y {
                    let _ = write!(css, "top:{};", len_css(y));
                }
                match s.wrap {
                    Some(Wrap::Image(WrapMode::Behind)) => css.push_str("z-index:-1;"),
                    Some(Wrap::Image(WrapMode::InFront)) => css.push_str("z-index:10;"),
                    _ => {}
                }
                if s.anchor_to == Some(AnchorTo::Paragraph) {
                    self.warn.unsupported(FORMAT, "anchorTo: paragraph", &n.id);
                }
            }
        }
        // 相对页面或版心定位的图：--paged 时由 PAGED_ABS 挪到所在页上再按坐标放
        let page_abs = match (s.position, s.anchor_to) {
            (Some(Position::Absolute), Some(AnchorTo::Page)) => Some("page"),
            (Some(Position::Absolute), Some(AnchorTo::Margin) | None) => Some("margin"),
            _ => None,
        };
        let mut img = String::new();
        if let Some(w) = s.width {
            let _ = write!(img, "width:{};", len_css(w));
        } else if let Some(w) = p.width {
            let _ = write!(img, "width:{w}pt;");
        }
        if let Some(h) = s.height {
            let _ = write!(img, "height:{};", len_css(h));
        } else if let Some(h) = p.height {
            let _ = write!(img, "height:{h}pt;");
        }
        if let Some(f) = s.object_fit {
            let _ = write!(img, "object-fit:{};", f.as_str());
        }
        if let Some(r) = s.rotation {
            let _ = write!(img, "transform:rotate({r}deg);");
        }
        if let Some(o) = s.opacity {
            let _ = write!(img, "opacity:{o};");
        }
        if s.horizontal_align.is_some() || s.position == Some(Position::Inline) {
            img.push_str("display:inline-block;");
        }
        let src = self.image_src(&n.id, &p.src);
        let _ = write!(
            self.out,
            "<figure id=\"{}\"{} style=\"{}\"><img src=\"{}\" alt=\"{}\" style=\"{}\">",
            esc(&n.id),
            page_abs
                .map(|a| format!(" class=\"fmtagno-abs\" data-anchor=\"{a}\""))
                .unwrap_or_default(),
            esc(&css),
            esc(&src),
            esc(&p.alt),
            esc(&img)
        );
        if let Some(c) = &p.caption {
            let mut cs = s.inherited();
            cs.merge(&n.part("caption"));
            let a = self.attr(&cs, Some(s), &n.id, Ctx::Block);
            let _ = write!(self.out, "<figcaption{a}>{}</figcaption>", esc(c));
        }
        self.out.push_str("</figure>\n");
    }

    /// 本地图片直接内嵌成 data URI，生成的 HTML 单独拿走也能看。
    fn image_src(&mut self, id: &str, src: &str) -> String {
        if assets::is_remote(src) {
            return src.to_string();
        }
        match assets::load(src, self.opts.base_dir.as_deref()) {
            Ok(a) => format!(
                "data:{};base64,{}",
                a.mime,
                base64::engine::general_purpose::STANDARD.encode(&a.bytes)
            ),
            Err(e) => {
                self.warn.push(format!("图片 `{id}`：{e:#}，保留原路径"));
                src.to_string()
            }
        }
    }

    fn inline(&mut self, t: &Inline, para: &Style, id: &str) {
        let base = para.text_only();
        for r in t.runs() {
            self.run(r, &base, id);
        }
    }

    /// 文字换了样式环境（格子、列表项）：run 样式按新环境重算后再写。
    fn inline_in(&mut self, t: &Inline, old: &Style, new: &Style, id: &str) {
        let base = new.text_only();
        for r in common::rebase_runs(t, old, new) {
            self.run(&r, &base, id);
        }
    }

    fn run(&mut self, r: &Run, base: &Style, id: &str) {
        let s = &r.computed;
        if s.hidden == Some(true) {
            return;
        }
        let mut b = base.clone();
        b.background = None;
        let mut css = self.css(s, Some(&b), id, Ctx::Inline);
        // 段落上的 highlight 也要落到每一段文字上
        if let Some(h) = &s.highlight
            && !css.contains("background")
        {
            let _ = write!(css, "background-color:{};", h.css());
        }
        let mut text = esc(&r.text).replace('\n', "<br>");
        if let Some(f) = r.field {
            let k = match f {
                Field::Page => "page",
                Field::Pages => "pages",
                Field::SectionPages => "sectionPages",
            };
            text = format!("<span class=\"fmtagno-field\" data-field=\"{k}\">{text}</span>");
        }
        let (open, close) = if r.is_code() {
            ("<code", "</code>")
        } else {
            ("<span", "</span>")
        };
        let body = if css.is_empty() && !r.is_code() {
            text
        } else if css.is_empty() {
            format!("{open}>{text}{close}")
        } else {
            format!("{open} style=\"{}\">{text}{close}", esc(&css))
        };
        match &r.href {
            Some(h) => {
                let _ = write!(self.out, "<a href=\"{}\">{body}</a>", esc(h));
            }
            None => self.out.push_str(&body),
        }
    }

    /// 样式转 CSS。会继承的字段和父样式一样时不写。
    fn css(&mut self, s: &Style, parent: Option<&Style>, id: &str, ctx: Ctx) -> String {
        let mut o = String::new();
        // 和父元素相同的继承字段跳过
        macro_rules! changed {
            ($f:ident) => {
                s.$f.is_some() && parent.map(|p| p.$f != s.$f).unwrap_or(true)
            };
        }
        let fs = s.size_pt();
        if changed!(font_family)
            && let Some(f) = &s.font_family
        {
            let list: Vec<String> = f
                .fallback()
                .iter()
                .flat_map(|x| x.split(','))
                .map(|x| format!("\"{}\"", safe(x.trim())))
                .collect();
            let _ = write!(o, "font-family:{}, {DEFAULT_FONTS};", list.join(", "));
        }
        if changed!(font_size)
            && let Some(v) = s.font_size
        {
            let _ = write!(o, "font-size:{};", len_css(v));
        }
        if changed!(color)
            && let Some(c) = &s.color
        {
            let _ = write!(o, "color:{};", c.css());
        }
        if changed!(bold)
            && let Some(v) = s.bold
        {
            let _ = write!(o, "font-weight:{};", if v { 700 } else { 400 });
        }
        if changed!(italic)
            && let Some(v) = s.italic
        {
            let _ = write!(o, "font-style:{};", if v { "italic" } else { "normal" });
        }
        if changed!(underline) || changed!(strike) || changed!(underline_color) {
            let mut lines = Vec::new();
            let mut style = "solid";
            let mut thick = false;
            if let Some(u) = s.underline.filter(|u| u.on()) {
                lines.push("underline");
                match u.0 {
                    UnderlineStyle::Double => style = "double",
                    UnderlineStyle::Dotted => style = "dotted",
                    UnderlineStyle::Dashed => style = "dashed",
                    UnderlineStyle::Wavy => style = "wavy",
                    UnderlineStyle::Thick => thick = true,
                    _ => {}
                }
            }
            match s.strike {
                Some(Strike::Single) => lines.push("line-through"),
                Some(Strike::Double) => {
                    lines.push("line-through");
                    style = "double";
                }
                _ => {}
            }
            if lines.is_empty() {
                o.push_str("text-decoration:none;");
            } else {
                let _ = write!(o, "text-decoration:{} {style}", lines.join(" "));
                if let Some(c) = &s.underline_color {
                    let _ = write!(o, " {}", c.css());
                }
                o.push(';');
                if thick {
                    o.push_str("text-decoration-thickness:2px;");
                }
            }
        }
        if let Some(v) = s.vertical_align {
            match (v, ctx) {
                (VerticalAlign::Super | VerticalAlign::Sub, _) => {
                    let _ = write!(o, "vertical-align:{};font-size:0.75em;", v.as_str());
                }
                (_, Ctx::Cell) => {
                    let _ = write!(
                        o,
                        "vertical-align:{};",
                        if v == VerticalAlign::Middle {
                            "middle"
                        } else {
                            v.as_str()
                        }
                    );
                }
                _ => {}
            }
        }
        if changed!(caps)
            && let Some(c) = s.caps
        {
            o.push_str(match c {
                Caps::All => "text-transform:uppercase;",
                Caps::Small => "font-variant:small-caps;",
            });
        }
        if changed!(letter_spacing)
            && let Some(v) = s.letter_spacing
        {
            let _ = write!(o, "letter-spacing:{};", len_css(v));
        }
        if ctx == Ctx::Inline
            && let Some(h) = &s.highlight
        {
            let _ = write!(o, "background-color:{};", h.css());
        }
        if changed!(emphasis_mark)
            && let Some(e) = s.emphasis_mark
        {
            o.push_str(match e {
                EmphasisMark::Dot => "text-emphasis:filled dot;",
                EmphasisMark::Comma => "text-emphasis:filled sesame;",
                EmphasisMark::Circle => "text-emphasis:open circle;",
                EmphasisMark::UnderDot => "text-emphasis:filled dot;text-emphasis-position:under;",
            });
        }
        if s.scale.is_some() && changed!(scale) {
            self.warn.unsupported(FORMAT, "scale", id);
        }
        if changed!(kerning)
            && let Some(k) = s.kerning
        {
            let _ = write!(o, "font-kerning:{};", if k { "normal" } else { "none" });
        }
        if changed!(hidden) && s.hidden == Some(true) {
            o.push_str("display:none;");
        }
        if ctx == Ctx::Inline {
            if let Some(bg) = &s.background {
                let _ = write!(
                    o,
                    "background:{};border-radius:2px;padding:0 0.2em;",
                    bg_css(bg)
                );
            }
            return o;
        }

        // 段落
        if changed!(align)
            && let Some(a) = s.align
        {
            match a {
                Align::Distribute => o.push_str("text-align:justify;text-align-last:justify;"),
                a => {
                    let _ = write!(o, "text-align:{};", a.as_str());
                }
            }
        }
        if let Some(v) = s.space_before {
            let _ = write!(o, "margin-top:{};", len_css_line(v, fs));
        }
        if let Some(v) = s.space_after {
            let _ = write!(o, "margin-bottom:{};", len_css_line(v, fs));
        }
        if changed!(line_height)
            && let Some(l) = s.line_height
        {
            match l {
                LineHeight::Multiple(x) => {
                    let _ = write!(o, "line-height:{x};");
                }
                LineHeight::Exact(x) | LineHeight::AtLeast(x) => {
                    let _ = write!(o, "line-height:{};", len_css(x));
                }
            }
        }
        if let Some(v) = s.margin_left {
            let _ = write!(o, "margin-left:{};", len_css(v));
        }
        if let Some(v) = s.margin_right {
            let _ = write!(o, "margin-right:{};", len_css(v));
        }
        if s.keep_with_next == Some(true) {
            o.push_str("break-after:avoid;");
        }
        if s.keep_together == Some(true) {
            o.push_str("break-inside:avoid;");
        }
        if s.page_break_before == Some(true) {
            o.push_str("break-before:page;");
        }
        if changed!(widow_control)
            && let Some(w) = s.widow_control
        {
            let n = if w { 2 } else { 1 };
            let _ = write!(o, "orphans:{n};widows:{n};");
        }
        if changed!(auto_space)
            && let Some(a) = s.auto_space
        {
            let _ = write!(
                o,
                "text-autospace:{};",
                if a { "normal" } else { "no-autospace" }
            );
        }
        if changed!(kinsoku)
            && let Some(k) = s.kinsoku
        {
            let _ = write!(o, "line-break:{};", if k { "strict" } else { "anywhere" });
        }
        if changed!(direction)
            && let Some(d) = s.direction
        {
            let _ = write!(o, "direction:{};", d.as_str());
        }
        if let Some(Wrap::Text(false)) = s.wrap {
            o.push_str("white-space:nowrap;");
        }
        if s.border_between.is_some() {
            self.warn.unsupported(FORMAT, "borderBetween", id);
        }
        if s.border_diagonal.is_some() {
            self.warn.unsupported(FORMAT, "borderDiagonal", id);
        }
        if s.contextual_spacing == Some(true) {
            self.warn.unsupported(FORMAT, "contextualSpacing", id);
        }

        // 盒子
        if ctx != Ctx::Table && ctx != Ctx::Cell {
            for (e, name) in Edge::ALL.iter().zip(["top", "right", "bottom", "left"]) {
                if let Some(b) = s.border_side(*e) {
                    let _ = write!(o, "border-{name}:{};", b.css());
                }
            }
        }
        if let Some(bg) = &s.background {
            let _ = write!(o, "background:{};", bg_css(bg));
        }
        if ctx != Ctx::Table {
            for (e, name) in Edge::ALL.iter().zip(["top", "right", "bottom", "left"]) {
                if let Some(p) = s.padding_side(*e) {
                    let _ = write!(o, "padding-{name}:{p}pt;");
                }
            }
        }
        if let Some(w) = s.width {
            let _ = write!(o, "width:{};", len_css(w));
        }
        if let Some(h) = s.height {
            let _ = write!(o, "height:{};", len_css(h));
        }
        if let Some(r) = s.text_rotation {
            match r {
                90 => o.push_str("writing-mode:vertical-rl;"),
                270 => o.push_str("writing-mode:vertical-rl;transform:rotate(180deg);"),
                _ => {}
            }
        }
        if let Some(r) = s.radius {
            let _ = write!(o, "border-radius:{};", len_css(r));
        }
        // 缩进放最后：悬挂缩进要改 padding-left
        if ctx != Ctx::ListItem {
            if let Some(h) = s.hanging_indent.filter(|_| changed!(hanging_indent)) {
                let pad = s.padding_side(Edge::Left).unwrap_or(0.0);
                let _ = write!(
                    o,
                    "padding-left:calc({} + {pad}pt);text-indent:-{};",
                    len_css(h),
                    len_css(h)
                );
            } else if changed!(text_indent)
                && let Some(t) = s.text_indent
            {
                let _ = write!(o, "text-indent:{};", len_css(t));
            }
        }
        o
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ctx {
    Block,
    Inline,
    ListItem,
    Table,
    Cell,
}

fn len_css(l: Len) -> String {
    match l {
        Len::Pt(v) => format!("{v}pt"),
        Len::Em(v) => format!("{v}em"),
        Len::Line(v) => format!("{}em", v * 1.2),
        Len::Pct(v) => format!("{v}%"),
    }
}

fn len_css_line(l: Len, fs: f32) -> String {
    match l {
        Len::Line(v) => format!("{}pt", v * fs * 1.2),
        l => len_css(l),
    }
}

fn bg_css(b: &Background) -> String {
    match b {
        Background::Gradient { angle, stops } => format!(
            "linear-gradient({}deg, {})",
            angle,
            stops.iter().map(|c| c.css()).collect::<Vec<_>>().join(", ")
        ),
        b => b.solid().css(),
    }
}

/// 去掉能跳出 CSS 声明的字符。
fn safe(v: &str) -> String {
    v.chars()
        .filter(|c| !matches!(c, ';' | '{' | '}' | '<' | '>' | '"' | '\\'))
        .collect()
}

pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            _ => o.push(c),
        }
    }
    o
}
