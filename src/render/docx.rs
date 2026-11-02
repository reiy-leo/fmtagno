//! docx 渲染器，基于 docx-rs。样式按算好的结果直接写在段落和 run 上（直接格式）；
//! 标题、代码另外挂上同名的 Word 样式，方便在 Word 里找和改。

use std::str::FromStr;

use docx_rs::{
    AbstractNumbering, AlignmentType, BorderType, BreakType, Docx, DrawingPosition, FieldCharType,
    Footer, Header, Hyperlink, HyperlinkType, IndentLevel, InstrNUMPAGES, InstrPAGE, InstrText,
    Level, LevelJc, LevelSuffixType, LevelText, LineSpacing, LineSpacingType, NumberFormat,
    Numbering, NumberingId, PageMargin, PageNumType, PageOrientationType, Paragraph,
    ParagraphBorder, ParagraphBorderPosition, ParagraphBorders, Pic, PicAlign, RelativeFromHType,
    RelativeFromVType, Run, RunFonts, Shading, ShdType, SpecialIndentType, Start, Style, StyleType,
    Table, TableCell, TableCellBorder, TableCellBorderPosition, TableCellBorders, TableCellMargins,
    TableLayoutType, TableRow, TextDirectionType, VAlignType, VertAlignType, WidthType,
};

use crate::{
    assets,
    components::{
        CodeProps, Component, Field, ImageProps, Inline, ListProps, Run as PRun, TableProps,
    },
    doc::{Doc, Node, Page, SectionLayout},
    render::{
        Options, Output,
        common::{self, Grid, Warn},
    },
    style::{
        Align, AnchorTo, Background, Border, BorderStyle, Caps, Edge, HAlign, Len, LineHeight,
        ListStyleType, MarkerGap, Position, Side, Strike, Style as PStyle, TableLayout,
        UnderlineStyle, VerticalAlign, Wrap, WrapMode,
    },
    units,
};

const FORMAT: &str = "docx";

pub fn render(doc: &Doc, tree: &Node, opts: &Options) -> anyhow::Result<Output> {
    let warn = Warn::default();
    let root = &tree.style;
    let mut docx = Docx::new()
        .default_fonts(fonts(root).unwrap_or_else(|| {
            RunFonts::new()
                .ascii("Arial")
                .hi_ansi("Arial")
                .east_asia("PingFang SC")
                .cs("Arial")
        }))
        .default_size((root.size_pt() * 2.0).round() as usize);
    let _ = doc;
    for lvl in 1..=6 {
        docx = docx.add_style(
            Style::new(format!("Heading{lvl}"), StyleType::Paragraph)
                .name(format!("heading {lvl}"))
                .based_on("Normal")
                .next("Normal")
                .q_format(true)
                .outline_lvl(lvl - 1),
        );
    }
    docx = docx
        .add_style(
            Style::new("Code", StyleType::Paragraph)
                .name("Code")
                .based_on("Normal"),
        )
        .add_style(Style::new("InlineCode", StyleType::Character).name("InlineCode"));

    let mut r = Ctx {
        warn,
        opts,
        content_width_pt: 0.0,
        // docx-rs 自带一个 id 为 1 的默认编号，从 2 开始免得撞上
        next_num: 2,
        blocks: Vec::new(),
        numberings: Vec::new(),
        repeat_heads: Vec::new(),
        sections: Vec::new(),
    };
    r.node(tree);
    let sections = std::mem::take(&mut r.sections);
    // 只要有一节写了页眉页脚，每节都把三种引用写全（没有的写空的），
    // 不然 Word 会把缺的那种接到上一节
    let any_hf = sections
        .iter()
        .any(|(l, _)| !l.header.is_empty() || !l.footer.is_empty());
    let even_odd = sections.iter().any(|(l, _)| l.even_odd);
    let last = sections.len().saturating_sub(1);
    let cols: Vec<Option<(u8, f32)>> = sections
        .iter()
        .map(|(l, _)| {
            l.page
                .columns
                .as_ref()
                .filter(|c| c.count() > 1)
                .map(|c| (c.count(), c.gap_pt()))
        })
        .collect();
    for (i, (lay, blocks)) in sections.into_iter().enumerate() {
        let page = &lay.page;
        let (w, h) = page.dims_pt(&mut Vec::new());
        let size = docx_rs::PageSize::new()
            .size(units::twip(w) as u32, units::twip(h) as u32)
            .orient(if page.landscape {
                PageOrientationType::Landscape
            } else {
                PageOrientationType::Portrait
            });
        let margin = page_margin(page);
        let hf = if any_hf { Some(r.headers(&lay)) } else { None };
        if i < last {
            let mut sec = docx_rs::Section::new().page_size(size).page_margin(margin);
            if lay.title_page {
                sec = sec.title_pg();
            }
            if let Some(n) = lay.page_number_start {
                sec = sec.page_num_type(PageNumType::new().start(n));
            }
            for b in blocks {
                sec = match b {
                    Block::P(p) => sec.add_paragraph(p),
                    Block::T(t) => sec.add_table(t),
                };
            }
            docx = docx.add_section(sec);
            // docx-rs 的 add_section 一节只留得住一个页眉或页脚（处理完第一个就把其余的丢了），
            // 所以每个页眉页脚单独挂在一个占位节上，生成 XML 后再挪回真正的节（fix_sections）
            if let Some(hf) = hf {
                let dummy = || {
                    docx_rs::Section::new()
                        .page_size(docx_rs::PageSize::new().size(DUMMY_PAGE, DUMMY_PAGE))
                };
                docx = docx.add_section(dummy().header(hf.header));
                docx = docx.add_section(dummy().footer(hf.footer));
                if let Some(x) = hf.first_header {
                    docx = docx.add_section(dummy().first_header(x));
                }
                if let Some(x) = hf.first_footer {
                    docx = docx.add_section(dummy().first_footer(x));
                }
                if let Some(x) = hf.even_header {
                    docx = docx.add_section(dummy().even_header(x));
                }
                if let Some(x) = hf.even_footer {
                    docx = docx.add_section(dummy().even_footer(x));
                }
            }
        } else {
            for b in blocks {
                docx = match b {
                    Block::P(p) => docx.add_paragraph(p),
                    Block::T(t) => docx.add_table(t),
                };
            }
            docx.document.section_property.page_size = size;
            docx = docx.page_margin(margin);
            if lay.title_page {
                docx = docx.title_pg();
            }
            if let Some(n) = lay.page_number_start {
                docx = docx.page_num_type(PageNumType::new().start(n));
            }
            if let Some(hf) = hf {
                docx = docx.header(hf.header).footer(hf.footer);
                if let Some(x) = hf.first_header {
                    docx = docx.first_header(x);
                }
                if let Some(x) = hf.first_footer {
                    docx = docx.first_footer(x);
                }
                if let Some(x) = hf.even_header {
                    docx = docx.even_header(x);
                }
                if let Some(x) = hf.even_footer {
                    docx = docx.even_footer(x);
                }
            }
        }
    }
    if even_odd {
        docx.settings = docx.settings.even_and_odd_headers();
    }
    for (abs, num) in std::mem::take(&mut r.numberings) {
        docx = docx.add_abstract_numbering(abs).add_numbering(num);
    }
    let mut xml = docx.build();
    xml.document = mark_table_headers(&xml.document, &r.repeat_heads);
    xml.document = fix_sections(&xml.document, &cols);
    let mut buf = std::io::Cursor::new(Vec::new());
    xml.pack(&mut buf)?;
    Ok(Output {
        bytes: buf.into_inner(),
        warnings: r.warn.list,
    })
}

/// 页边距，含页眉页脚距离；装订线算在左边。
fn page_margin(page: &Page) -> PageMargin {
    let [mt, mr, mb, ml] = page.margins_pt();
    let gutter = page.gutter.map(|g| g.abs_pt()).unwrap_or(0.0);
    let (hd, fd) = page.hf_distance_pt();
    PageMargin::new()
        .top(units::twip(mt))
        .bottom(units::twip(mb))
        .left(units::twip(ml + gutter))
        .right(units::twip(mr))
        .header(units::twip(hd))
        .footer(units::twip(fd))
        .gutter(0)
}

/// 一节要写的页眉页脚。
struct HeaderSet {
    header: Header,
    footer: Footer,
    first_header: Option<Header>,
    first_footer: Option<Footer>,
    even_header: Option<Header>,
    even_footer: Option<Footer>,
}

/// 占位节的纸张边长（twip），用来在 XML 里认出占位节。
const DUMMY_PAGE: u32 = 1;

/// 收拾 docx-rs 生成的分节：
/// 1. 把占位节（纸张 1×1）上的页眉页脚引用挪到它前面那个真正的节上，删掉占位节的段落；
/// 2. sectPr 里的子元素按 OOXML 规定的顺序排（headerReference 在最前，titlePg 在 cols 后面）；
/// 3. 按 `cols` 写分栏（docx-rs 的 Section 不让设）。
fn fix_sections(doc: &[u8], cols: &[Option<(u8, f32)>]) -> Vec<u8> {
    let Ok(xml) = std::str::from_utf8(doc) else {
        return doc.to_vec();
    };
    const OPEN: &str = "<w:sectPr>";
    const CLOSE: &str = "</w:sectPr>";
    let dummy_size = format!("<w:pgSz w:w=\"{DUMMY_PAGE}\" w:h=\"{DUMMY_PAGE}\"");
    // 第一遍：挪引用、删占位节
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    // 上一个真正的节在 out 里的 <w:sectPr> 之后的位置
    let mut last_real: Option<usize> = None;
    while let Some(i) = rest.find(OPEN) {
        let Some(j) = rest[i..].find(CLOSE).map(|j| i + j) else {
            break;
        };
        let body = &rest[i + OPEN.len()..j];
        if body.contains(&dummy_size)
            && let Some(at) = last_real
            && let Some(p) = rest[..i].rfind("<w:p ")
            && let Some(after) = rest[j..].strip_prefix(CLOSE)
            && let Some(after) = after.strip_prefix("</w:pPr></w:p>")
        {
            out.push_str(&rest[..p]);
            let refs: String = elements(body)
                .into_iter()
                .filter(|e| {
                    e.starts_with("<w:headerReference") || e.starts_with("<w:footerReference")
                })
                .collect();
            out.insert_str(at, &refs);
            rest = after;
            continue;
        }
        out.push_str(&rest[..i + OPEN.len()]);
        last_real = Some(out.len());
        out.push_str(&rest[i + OPEN.len()..j]);
        rest = &rest[j..];
    }
    out.push_str(rest);
    // 第二遍：排顺序、写分栏
    const ORDER: &[&str] = &[
        "headerReference",
        "footerReference",
        "footnotePr",
        "endnotePr",
        "type",
        "pgSz",
        "pgMar",
        "paperSrc",
        "pgBorders",
        "lnNumType",
        "pgNumType",
        "cols",
        "formProt",
        "vAlign",
        "noEndnote",
        "titlePg",
        "textDirection",
        "bidi",
        "rtlGutter",
        "docGrid",
    ];
    let mut done = String::with_capacity(out.len());
    let mut rest = out.as_str();
    let mut n = 0;
    while let Some(i) = rest.find(OPEN) {
        let Some(j) = rest[i..].find(CLOSE).map(|j| i + j) else {
            break;
        };
        done.push_str(&rest[..i + OPEN.len()]);
        let body = &rest[i + OPEN.len()..j];
        let mut els = elements(body);
        if els.concat().len() == body.len() {
            if let Some(Some((count, gap))) = cols.get(n) {
                let c = format!(
                    "<w:cols w:space=\"{}\" w:num=\"{count}\" />",
                    units::twip(*gap)
                );
                match els.iter_mut().find(|e| e.starts_with("<w:cols ")) {
                    Some(e) => *e = c,
                    None => els.push(c),
                }
            }
            let rank = |e: &String| {
                let name = e[3..].split([' ', '/', '>']).next().unwrap_or("");
                ORDER.iter().position(|x| *x == name).unwrap_or(ORDER.len())
            };
            els.sort_by_key(rank);
            done.push_str(&els.concat());
        } else {
            // 有不认识的写法（带子元素的），原样留着
            done.push_str(body);
        }
        n += 1;
        rest = &rest[j..];
    }
    done.push_str(rest);
    done.into_bytes()
}

/// sectPr 里自闭合的子元素，按原顺序。
fn elements(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find("<w:") {
        let Some(j) = rest[i..].find("/>").map(|j| i + j + 2) else {
            break;
        };
        let e = &rest[i..j];
        if e[1..].contains('<') {
            // 不是自闭合的，交给调用方原样处理
            break;
        }
        out.push(e.to_string());
        rest = &rest[j..];
    }
    out
}

/// 给要重复的表头行补上 `<w:tblHeader />`：Word 跨页时会在新页顶上再画一遍这一行。
/// `heads` 和文档里 `<w:tbl>` 出现的顺序一一对应（表格不嵌套）。
fn mark_table_headers(doc: &[u8], heads: &[bool]) -> Vec<u8> {
    if !heads.contains(&true) {
        return doc.to_vec();
    }
    let Ok(xml) = std::str::from_utf8(doc) else {
        return doc.to_vec();
    };
    let mut out = String::with_capacity(xml.len() + heads.len() * 32);
    let mut rest = xml;
    for &repeat in heads {
        let Some(t) = rest.find("<w:tbl>") else { break };
        let Some(tr) = rest[t..].find("<w:trPr").map(|i| t + i) else {
            break;
        };
        out.push_str(&rest[..tr]);
        rest = &rest[tr..];
        if repeat {
            if let Some(r) = rest.strip_prefix("<w:trPr />") {
                out.push_str("<w:trPr><w:tblHeader /></w:trPr>");
                rest = r;
            } else if let Some(r) = rest.strip_prefix("<w:trPr>") {
                out.push_str("<w:trPr><w:tblHeader />");
                rest = r;
            }
        }
        // 跳过这张表，下一轮找下一张
        let end = rest
            .find("</w:tbl>")
            .map_or(rest.len(), |i| i + "</w:tbl>".len());
        out.push_str(&rest[..end]);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.into_bytes()
}

// 段落和表格都只活到 render 结束，不值得为大小差异装箱
#[allow(clippy::large_enum_variant)]
enum Block {
    P(Paragraph),
    T(Table),
}

struct Ctx<'a> {
    warn: Warn,
    opts: &'a Options,
    content_width_pt: f32,
    next_num: usize,
    blocks: Vec<Block>,
    numberings: Vec<(AbstractNumbering, Numbering)>,
    /// 按出现顺序，每张表的表头要不要跨页重复
    repeat_heads: Vec<bool>,
    /// 每节的版面和内容
    sections: Vec<(SectionLayout, Vec<Block>)>,
}

impl Ctx<'_> {
    fn node(&mut self, n: &Node) {
        let s = &n.style;
        match &n.component {
            Component::Document(_) => self.children(n),
            Component::Slide(p) => {
                if let Some(t) = &p.title {
                    let mut ts = s.clone();
                    ts.merge(&n.part("title"));
                    let para = self.para(Paragraph::new().style("Heading2"), &ts, &n.id);
                    let para = para.add_run(self.run_props(Run::new().add_text(t), &ts, &n.id));
                    self.blocks.push(Block::P(para));
                }
                if !n.part("body").is_empty() {
                    self.warn.unsupported(FORMAT, "Slide.body", &n.id);
                }
                self.children(n);
                self.page_break();
            }
            Component::Heading(p) => {
                let para = self.para(
                    Paragraph::new().style(&format!("Heading{}", p.level)),
                    s,
                    &n.id,
                );
                let para = self.inline(para, p.text.runs().cloned().collect(), &n.id);
                self.blocks.push(Block::P(para));
            }
            Component::Text(p) => {
                let para = self.para(Paragraph::new(), s, &n.id);
                let para = self.inline(para, p.text.runs().cloned().collect(), &n.id);
                self.blocks.push(Block::P(para));
            }
            Component::Code(p) => self.code(n, p),
            Component::List(p) => self.list(n, p),
            Component::Table(p) => self.table(n, p),
            Component::Image(p) => self.image(n, p),
            Component::PageBreak(_) => self.page_break(),
            Component::Section(p) => {
                let lay = p.layout();
                let page = &lay.page;
                let (w, _) = page.dims_pt(&mut Vec::new());
                let [_, mr, _, ml] = page.margins_pt();
                let gutter = page.gutter.map(|g| g.abs_pt()).unwrap_or(0.0);
                self.content_width_pt = w - ml - mr - gutter;
                let saved = std::mem::take(&mut self.blocks);
                self.children(n);
                let blocks = std::mem::replace(&mut self.blocks, saved);
                self.sections.push((lay.clone(), blocks));
            }
        }
    }

    /// 把一个元素渲染成段落和表格，给页眉页脚用。
    fn blocks_of(&mut self, n: Option<&Node>) -> Vec<Block> {
        let saved = std::mem::take(&mut self.blocks);
        if let Some(n) = n {
            self.node(n);
        }
        std::mem::replace(&mut self.blocks, saved)
    }

    fn header_of(&mut self, n: Option<&Node>) -> Header {
        let mut h = Header::new();
        for b in self.blocks_of(n) {
            h = match b {
                Block::P(p) => h.add_paragraph(p),
                Block::T(t) => h.add_table(t),
            };
        }
        h
    }

    fn footer_of(&mut self, n: Option<&Node>) -> Footer {
        let mut f = Footer::new();
        for b in self.blocks_of(n) {
            f = match b {
                Block::P(p) => f.add_paragraph(p),
                Block::T(t) => f.add_table(t),
            };
        }
        f
    }

    fn headers(&mut self, lay: &SectionLayout) -> HeaderSet {
        // 表头重复只认正文里的表
        let heads = self.repeat_heads.len();
        let set = HeaderSet {
            header: self.header_of(lay.header.default.as_ref()),
            footer: self.footer_of(lay.footer.default.as_ref()),
            first_header: lay
                .title_page
                .then(|| self.header_of(lay.header.for_first())),
            first_footer: lay
                .title_page
                .then(|| self.footer_of(lay.footer.for_first())),
            even_header: lay.even_odd.then(|| self.header_of(lay.header.for_even())),
            even_footer: lay.even_odd.then(|| self.footer_of(lay.footer.for_even())),
        };
        self.repeat_heads.truncate(heads);
        set
    }

    fn children(&mut self, n: &Node) {
        for c in &n.children {
            self.node(c);
        }
    }

    fn page_break(&mut self) {
        self.blocks.push(Block::P(
            Paragraph::new().add_run(Run::new().add_break(BreakType::Page)),
        ));
    }

    fn code(&mut self, n: &Node, p: &CodeProps) {
        let s = &n.style;
        let mut para = self.para(Paragraph::new().style("Code"), s, &n.id);
        let width = p.code.lines().count().to_string().len();
        for (i, line) in p.code.split('\n').enumerate() {
            let mut run = Run::new();
            if i > 0 {
                run = run.add_break(BreakType::TextWrapping);
            }
            let text = if p.line_numbers {
                format!("{:>width$}  {line}", i + 1)
            } else {
                line.to_string()
            };
            run = run.add_text(text);
            para = para.add_run(self.run_props(run, &s.text_only_no_bg(), &n.id));
        }
        self.blocks.push(Block::P(para));
    }

    fn list(&mut self, n: &Node, p: &ListProps) {
        let lines = common::flatten_list(n, &p.items, p.ordered);
        let id = self.next_num;
        self.next_num += 1;
        // 每一级的定义取这一级第一次出现时的样式
        let mut abs = AbstractNumbering::new(id);
        let mut done = [false; 9];
        let marker_part = n.part("marker");
        for l in &lines {
            let lv = (l.level - 1).min(8);
            if done[lv] {
                continue;
            }
            done[lv] = true;
            let s = &l.style;
            let fs = s.size_pt();
            let ty = s.list_style_type.clone().unwrap_or(ListStyleType::Disc);
            let fmt = num_format(&ty);
            let indent = s.list_indent.map(|x| x.pt(fs, 0.0)).unwrap_or(18.0) * l.level as f32;
            let hanging = s.list_hanging.map(|x| x.pt(fs, 0.0)).unwrap_or(14.0);
            let jc = match s.list_marker_align {
                Some(HAlign::Center) => "center",
                Some(HAlign::Right) => "right",
                _ => "left",
            };
            let mut level = Level::new(
                lv,
                Start::new(s.list_start.unwrap_or(1) as usize),
                NumberFormat::new(fmt),
                LevelText::new(common::word_level_text(s, l.level)),
                LevelJc::new(jc),
            )
            .indent(
                Some(units::twip(indent)),
                // Word 里右对齐、居中的编号是以编号位置为右边、中线排的；
                // 换算一下，让效果和 html、pdf 里「编号在悬挂区里对齐」一致
                Some(SpecialIndentType::Hanging(units::twip(
                    match s.list_marker_align {
                        Some(HAlign::Right) => fs * 0.5,
                        Some(HAlign::Center) => hanging / 2.0,
                        _ => hanging,
                    },
                ))),
                None,
                None,
            )
            .suffix(match s.list_marker_gap {
                Some(MarkerGap::Space) => LevelSuffixType::Space,
                Some(MarkerGap::None) => LevelSuffixType::Nothing,
                _ => LevelSuffixType::Tab,
            });
            let mut ms = PStyle::default();
            if let Some(c) = &s.list_marker_color {
                ms.color = Some(c.clone());
            }
            if let Some(pct) = s.list_marker_size {
                ms.font_size = Some(Len::Pt(fs * pct / 100.0));
            }
            ms.merge(&marker_part);
            if let Some(c) = &ms.color {
                level = level.color(c.rgb());
            }
            if ms.font_size.is_some() {
                level = level.size((ms.size_pt() * 2.0).round() as usize);
            }
            if ms.bold == Some(true) {
                level = level.bold();
            }
            if ms.italic == Some(true) {
                level = level.italic();
            }
            if let Some(f) = &s.list_marker_font {
                level = level.fonts(RunFonts::new().ascii(f).hi_ansi(f).east_asia(f).cs(f));
            }
            if s.list_marker_image.is_some() {
                self.warn.unsupported(FORMAT, "listMarkerImage", &n.id);
            }
            if matches!(s.list_marker_gap, Some(MarkerGap::Len(_))) {
                self.warn
                    .unsupported(FORMAT, "listMarkerGap（长度，按制表符处理）", &n.id);
            }
            if common::legal(&ty) && l.level > 1 {
                level = level.is_lgl();
            }
            abs = abs.add_level(level);
        }
        self.numberings.push((abs, Numbering::new(id, id)));
        let item_part = n.part("item");
        let last = lines.len().saturating_sub(1);
        for (i, l) in lines.iter().enumerate() {
            let mut item = l.style.clone();
            item.merge(&item_part);
            // 段前只给第一项，段后只给最后一项，列表内部不留空
            if i > 0 {
                item.space_before = item_part.space_before;
            }
            if i < last {
                item.space_after = item_part.space_after;
            }
            // 缩进由编号定义管，段落上不再写
            let mut ps = item.clone();
            ps.margin_left = None;
            ps.text_indent = None;
            ps.hanging_indent = None;
            let para = Paragraph::new()
                .numbering(NumberingId::new(id), IndentLevel::new((l.level - 1).min(8)));
            let para = self.para(para, &ps, &n.id);
            let runs = common::rebase_runs(l.item.text(), &n.style, &item);
            let para = self.inline(para, runs, &n.id);
            self.blocks.push(Block::P(para));
        }
    }

    fn table(&mut self, n: &Node, p: &TableProps) {
        let g = Grid::of(p);
        let s = &n.style;
        let width_pt = match s.width {
            Some(Len::Pct(x)) => self.content_width_pt * x / 100.0,
            Some(w) => w.abs_pt(),
            None => self.content_width_pt,
        };
        let total = units::twip(width_pt) as f32;
        let even = vec![(total / g.cols as f32) as usize; g.cols];
        let grid: Vec<usize> = match &p.widths {
            Some(ws) if ws.len() == g.cols && ws.iter().sum::<f32>() > 0.0 => {
                let sum: f32 = ws.iter().sum();
                ws.iter().map(|x| (total * x / sum) as usize).collect()
            }
            Some(_) => {
                self.warn.push(format!(
                    "表格 `{}` 的 widths 个数和列数对不上，改成平均分",
                    n.id
                ));
                even
            }
            None => even,
        };
        let pad = units::twip(
            s.cell_padding
                .map(|x| x.pt(s.size_pt(), 0.0))
                .unwrap_or(4.0),
        );
        let empty = Inline::Plain(String::new());
        let mut rows = Vec::new();
        for r in 0..g.rows {
            let cells: &[Inline] = if g.has_header && r == 0 {
                &p.columns
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
                &p.rows[i]
            };
            let mut tcs = Vec::new();
            for (c, width) in grid.iter().enumerate() {
                let part = common::cell_parts(n, &p.look, &g, r, c);
                let mut cs = s.inherited();
                cs.merge(&part);
                let mut ps = cs.clone();
                ps.background = None;
                // 部位的边框归格子画，格子里的段落不再带一份（不然 Word 会在格子里多画一条）
                ps.border = None;
                ps.border_top = None;
                ps.border_right = None;
                ps.border_bottom = None;
                ps.border_left = None;
                ps.border_between = None;
                ps.border_diagonal = None;
                ps.space_before = part.space_before;
                ps.space_after = part.space_after;
                let para = self.para(Paragraph::new(), &ps, &n.id);
                let runs = common::rebase_runs(cells.get(c).unwrap_or(&empty), s, &cs);
                let para = self.inline(para, runs, &n.id);
                let mut cell = TableCell::new()
                    .add_paragraph(para)
                    .width(*width, WidthType::Dxa);
                if let Some(bg) = &part.background {
                    cell = cell.shading(shading(bg));
                }
                // 两格之间的线只写一次：上下之间写在上面一格的下边，左右之间写在左边一格的右边。
                // 两边都写的话 Word 会画两条（双线尤其明显）。
                let mut b = common::cell_edges(n, &p.look, &g, r, c);
                if r + 1 < g.rows {
                    b[2] = common::cell_edges(n, &p.look, &g, r + 1, c)[0].clone();
                }
                if c + 1 < g.cols {
                    b[1] = common::cell_edges(n, &p.look, &g, r, c + 1)[3].clone();
                }
                let mut borders = TableCellBorders::with_empty();
                for (i, (pos, b)) in [
                    TableCellBorderPosition::Top,
                    TableCellBorderPosition::Right,
                    TableCellBorderPosition::Bottom,
                    TableCellBorderPosition::Left,
                ]
                .into_iter()
                .zip(b.iter())
                .enumerate()
                {
                    let shared = (i == 0 && r > 0) || (i == 3 && c > 0);
                    if !shared {
                        borders = borders.set(cell_border(pos, b.as_ref()));
                    }
                }
                cell = cell.set_borders(borders);
                if let Some(v) = part.vertical_align {
                    cell = cell.vertical_align(match v {
                        VerticalAlign::Middle => VAlignType::Center,
                        VerticalAlign::Bottom => VAlignType::Bottom,
                        _ => VAlignType::Top,
                    });
                }
                if let Some(rot) = part.text_rotation {
                    cell = cell.text_direction(match rot {
                        90 => TextDirectionType::TbRl,
                        270 => TextDirectionType::BtLr,
                        _ => TextDirectionType::Lr,
                    });
                }
                if Edge::ALL.iter().any(|e| part.padding_side(*e).is_some()) {
                    self.warn
                        .unsupported(FORMAT, "单个格子的 padding（用 cellPadding）", &n.id);
                }
                tcs.push(cell);
            }
            let mut row = TableRow::new(tcs);
            if s.row_break == Some(false) {
                row = row.cant_split();
            }
            rows.push(row);
        }
        // docx-rs 没有 w:tblHeader，生成 XML 后再补（见 mark_table_headers）
        self.repeat_heads
            .push(g.has_header && s.repeat_header.unwrap_or(true));
        let t = Table::without_borders(rows)
            .set_grid(grid)
            .layout(match s.table_layout {
                Some(TableLayout::Auto) => TableLayoutType::Autofit,
                _ => TableLayoutType::Fixed,
            })
            .margins(TableCellMargins::new().margin(
                pad as usize,
                pad as usize,
                pad as usize,
                pad as usize,
            ))
            .width(total as usize, WidthType::Dxa);
        if s.page_break_before == Some(true) {
            self.page_break();
        }
        self.blocks.push(Block::T(t));
        // Word 两个表格紧挨着会合并，后面放一个空段落，用 spaceAfter 当间距
        let gap = PStyle {
            space_after: s.space_after,
            line_height: Some(LineHeight::Exact(Len::Pt(1.0))),
            ..PStyle::default()
        };
        let para = self.para(Paragraph::new(), &gap, &n.id);
        self.blocks.push(Block::P(para));
    }

    fn image(&mut self, n: &Node, p: &ImageProps) {
        let s = &n.style;
        let asset = if assets::is_remote(&p.src) {
            Err("docx 暂不支持远程图片".to_string())
        } else {
            assets::load(&p.src, self.opts.base_dir.as_deref()).map_err(|e| format!("{e:#}"))
        };
        let decoded = asset.and_then(|a| match image::load_from_memory(&a.bytes) {
            Ok(img) => Ok((a, img.width() as f32, img.height() as f32)),
            Err(_) => Err(format!(
                "{} 格式放不进 docx（支持 png、jpeg、gif、webp）",
                a.mime
            )),
        });
        let (asset, px_w, px_h) = match decoded {
            Ok(x) => x,
            Err(why) => {
                self.warn
                    .push(format!("图片 `{}`：{why}，改成显示 alt 文字", n.id));
                let alt = if p.alt.is_empty() { &p.src } else { &p.alt };
                let para = self.para(Paragraph::new(), s, &n.id);
                let run = self.run_props(
                    Run::new().add_text(format!("[{alt}]")),
                    &s.text_only_no_bg(),
                    &n.id,
                );
                self.blocks.push(Block::P(para.add_run(run.italic())));
                return;
            }
        };
        let (pw, ph) = (px_w * 0.75, px_h * 0.75); // px → pt
        let sw = s
            .width
            .map(|l| l.pt(s.size_pt(), self.content_width_pt))
            .or(p.width);
        let sh = s.height.map(|l| l.pt(s.size_pt(), 0.0)).or(p.height);
        let (mut w, mut h) = match (sw, sh) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * ph / pw),
            (None, Some(h)) => (h * pw / ph, h),
            (None, None) => (pw, ph),
        };
        if w > self.content_width_pt {
            h *= self.content_width_pt / w;
            w = self.content_width_pt;
        }
        let mut pic = Pic::new(&asset.bytes).size(units::emu(w), units::emu(h));
        if let Some(r) = s.rotation {
            pic = pic.rotate((r.rem_euclid(360.0)) as u16);
        }
        let dist = units::emu(s.wrap_distance.map(|d| d.abs_pt()).unwrap_or(6.0)) as i32;
        match s.position.unwrap_or(Position::Inline) {
            Position::Inline => {}
            Position::Float => {
                pic = pic
                    .floating()
                    .relative_from_h(RelativeFromHType::Margin)
                    .relative_from_v(RelativeFromVType::Paragraph)
                    .position_h(DrawingPosition::Align(
                        match s.float.unwrap_or(Side::Left) {
                            Side::Left => PicAlign::Left,
                            Side::Right => PicAlign::Right,
                        },
                    ))
                    .position_v(DrawingPosition::Offset(0))
                    .dist_t(dist)
                    .dist_b(dist)
                    .dist_l(dist)
                    .dist_r(dist);
            }
            Position::Absolute => {
                let (rh, rv) = match s.anchor_to.unwrap_or(AnchorTo::Page) {
                    AnchorTo::Page | AnchorTo::Slide => {
                        (RelativeFromHType::Page, RelativeFromVType::Page)
                    }
                    AnchorTo::Margin => (RelativeFromHType::Margin, RelativeFromVType::Margin),
                    AnchorTo::Paragraph => {
                        (RelativeFromHType::Column, RelativeFromVType::Paragraph)
                    }
                };
                pic = pic
                    .floating()
                    .relative_from_h(rh)
                    .relative_from_v(rv)
                    .position_h(match s.horizontal_align {
                        Some(HAlign::Left) => DrawingPosition::Align(PicAlign::Left),
                        Some(HAlign::Center) => DrawingPosition::Align(PicAlign::Center),
                        Some(HAlign::Right) => DrawingPosition::Align(PicAlign::Right),
                        None => DrawingPosition::Offset(units::emu(
                            s.x.map(|v| v.abs_pt()).unwrap_or(0.0),
                        ) as i32),
                    })
                    .position_v(DrawingPosition::Offset(units::emu(
                        s.y.map(|v| v.abs_pt()).unwrap_or(0.0),
                    ) as i32));
                if matches!(
                    s.wrap,
                    Some(Wrap::Image(
                        WrapMode::InFront | WrapMode::Behind | WrapMode::None
                    )) | None
                ) {
                    pic = pic.overlapping();
                }
            }
        }
        if matches!(s.wrap, Some(Wrap::Image(WrapMode::Behind))) {
            self.warn
                .unsupported(FORMAT, "wrap: behind（按 inFront 处理）", &n.id);
        }
        for (f, on) in [
            ("opacity", s.opacity.is_some()),
            ("objectFit", s.object_fit.is_some()),
        ] {
            if on {
                self.warn.unsupported(FORMAT, f, &n.id);
            }
        }
        let mut ps = s.clone();
        if s.position.unwrap_or(Position::Inline) == Position::Inline
            && let Some(a) = s.horizontal_align
        {
            ps.align = Some(match a {
                HAlign::Left => Align::Left,
                HAlign::Center => Align::Center,
                HAlign::Right => Align::Right,
            });
        }
        ps.width = None;
        ps.height = None;
        let para = self
            .para(Paragraph::new(), &ps, &n.id)
            .add_run(Run::new().add_image(pic));
        self.blocks.push(Block::P(para));
        if let Some(c) = &p.caption {
            let mut cs = s.inherited();
            cs.merge(&n.part("caption"));
            let para = self.para(Paragraph::new(), &cs, &n.id);
            let run = self.run_props(Run::new().add_text(c), &cs.text_only_no_bg(), &n.id);
            self.blocks.push(Block::P(para.add_run(run)));
        }
    }

    /// 段落属性。
    fn para(&mut self, mut p: Paragraph, s: &PStyle, id: &str) -> Paragraph {
        let fs = s.size_pt();
        if let Some(a) = s.align {
            p = p.align(match a {
                Align::Left => AlignmentType::Left,
                Align::Center => AlignmentType::Center,
                Align::Right => AlignmentType::Right,
                Align::Justify => AlignmentType::Both,
                Align::Distribute => AlignmentType::Distribute,
            });
        }
        let mut ls = LineSpacing::new();
        let mut any = false;
        match s.space_before {
            Some(Len::Line(x)) => {
                ls = ls.before_lines((x * 100.0) as u32);
                any = true;
            }
            Some(l) => {
                ls = ls.before(units::twip(l.pt(fs, 0.0)).max(0) as u32);
                any = true;
            }
            None => {}
        }
        match s.space_after {
            Some(Len::Line(x)) => {
                ls = ls.after_lines((x * 100.0) as u32);
                any = true;
            }
            Some(l) => {
                ls = ls.after(units::twip(l.pt(fs, 0.0)).max(0) as u32);
                any = true;
            }
            None => {}
        }
        match s.line_height {
            Some(LineHeight::Multiple(x)) => {
                ls = ls.line((x * 240.0) as i32).line_rule(LineSpacingType::Auto);
                any = true;
            }
            Some(LineHeight::Exact(l)) => {
                ls = ls
                    .line(units::twip(l.pt(fs, 0.0)))
                    .line_rule(LineSpacingType::Exact);
                any = true;
            }
            Some(LineHeight::AtLeast(l)) => {
                ls = ls
                    .line(units::twip(l.pt(fs, 0.0)))
                    .line_rule(LineSpacingType::AtLeast);
                any = true;
            }
            None => {}
        }
        if any {
            p = p.line_spacing(ls);
        }
        let left = s.margin_left.map(|l| units::twip(l.pt(fs, 0.0)));
        let right = s.margin_right.map(|l| units::twip(l.pt(fs, 0.0)));
        let special = match (s.hanging_indent, s.text_indent) {
            (Some(h), _) => Some(SpecialIndentType::Hanging(units::twip(h.pt(fs, 0.0)))),
            (None, Some(t)) => Some(SpecialIndentType::FirstLine(units::twip(t.pt(fs, 0.0)))),
            _ => None,
        };
        if left.is_some() || right.is_some() || special.is_some() {
            p = p.indent(left, special, right, None);
            match (s.hanging_indent, s.text_indent) {
                (Some(Len::Em(x)), _) => p = p.hanging_chars((x * 100.0) as i32),
                (None, Some(Len::Em(x))) => p = p.first_line_chars((x * 100.0) as i32),
                _ => {}
            }
        }
        if let Some(v) = s.keep_with_next {
            p = p.keep_next(v);
        }
        if let Some(v) = s.keep_together {
            p = p.keep_lines(v);
        }
        if let Some(v) = s.page_break_before {
            p = p.page_break_before(v);
        }
        if let Some(v) = s.widow_control {
            p = p.widow_control(v);
        }
        if let Some(l) = s.outline_level {
            p = p.outline_lvl((l.clamp(1, 9) - 1) as usize);
        }
        if let Some(d) = s.direction {
            p.property = p.property.bidi(d == crate::style::Direction::Rtl);
        }
        if let Some(bg) = &s.background {
            p.property = p.property.shading(shading(bg));
        }
        let mut borders = ParagraphBorders::with_empty();
        let mut has_border = false;
        for (e, pos) in Edge::ALL.iter().zip([
            ParagraphBorderPosition::Top,
            ParagraphBorderPosition::Right,
            ParagraphBorderPosition::Bottom,
            ParagraphBorderPosition::Left,
        ]) {
            if let Some(b) = s.border_side(*e) {
                let space = s.padding_side(*e).unwrap_or(0.0);
                borders = borders.set(para_border(pos, b, space));
                has_border = true;
            }
        }
        if let Some(b) = &s.border_between {
            borders = borders.set(para_border(ParagraphBorderPosition::Between, b, 0.0));
            has_border = true;
        }
        if has_border {
            p.property = p.property.set_borders(borders);
        } else if Edge::ALL.iter().any(|e| s.padding_side(*e).is_some()) && s.background.is_some() {
            self.warn
                .unsupported(FORMAT, "padding（段落只在有边框时生效）", id);
        }
        for (f, on) in [
            ("contextualSpacing", s.contextual_spacing.is_some()),
            ("autoSpace", s.auto_space.is_some()),
            ("kinsoku", s.kinsoku.is_some()),
            ("radius", s.radius.is_some()),
            ("wrap: false", s.wrap == Some(Wrap::Text(false))),
            ("borderDiagonal", s.border_diagonal.is_some()),
        ] {
            if on {
                self.warn.unsupported(FORMAT, f, id);
            }
        }
        p
    }

    fn inline(&mut self, mut p: Paragraph, runs: Vec<PRun>, id: &str) -> Paragraph {
        for r in runs {
            let s = &r.computed;
            if s.hidden == Some(true) && r.text.is_empty() {
                continue;
            }
            let mut run = Run::new();
            if let Some(f) = r.field {
                let instr = match f {
                    Field::Page => InstrText::PAGE(InstrPAGE::new()),
                    Field::Pages => InstrText::NUMPAGES(InstrNUMPAGES::new()),
                    Field::SectionPages => InstrText::Unsupported("SECTIONPAGES".into()),
                };
                let shown = if r.text.is_empty() { "1" } else { &r.text };
                // 每一部分单独一个 run 并带上文字属性，域结果才会用这些属性显示
                let parts = [
                    Run::new().add_field_char(FieldCharType::Begin, true),
                    Run::new().add_instr_text(instr),
                    Run::new().add_field_char(FieldCharType::Separate, false),
                    Run::new().add_text(shown),
                    Run::new().add_field_char(FieldCharType::End, false),
                ];
                for part in parts {
                    let part = self.run_props(part, s, id);
                    p = p.add_run(part);
                }
                continue;
            }
            for (i, line) in r.text.split('\n').enumerate() {
                if i > 0 {
                    run = run.add_break(BreakType::TextWrapping);
                }
                run = run.add_text(line);
            }
            if r.is_code() {
                run.run_property = run.run_property.style("InlineCode");
            }
            let run = self.run_props(run, s, id);
            p = match &r.href {
                Some(h) => p.add_hyperlink(Hyperlink::new(h, HyperlinkType::External).add_run(run)),
                None => p.add_run(run),
            };
        }
        p
    }

    /// 文字属性。
    fn run_props(&mut self, mut run: Run, s: &PStyle, id: &str) -> Run {
        if let Some(f) = fonts(s) {
            run = run.fonts(f);
        }
        if s.font_size.is_some() {
            run = run.size((s.size_pt() * 2.0).round() as usize);
        }
        if let Some(c) = &s.color {
            run = run.color(c.rgb());
        }
        match s.bold {
            Some(true) => run = run.bold(),
            Some(false) => run = run.disable_bold(),
            None => {}
        }
        match s.italic {
            Some(true) => run = run.italic(),
            Some(false) => run = run.disable_italic(),
            None => {}
        }
        if let Some(u) = s.underline {
            run = run.underline(match u.0 {
                UnderlineStyle::None => "none",
                UnderlineStyle::Single => "single",
                UnderlineStyle::Double => "double",
                UnderlineStyle::Thick => "thick",
                UnderlineStyle::Dotted => "dotted",
                UnderlineStyle::Dashed => "dash",
                UnderlineStyle::Wavy => "wave",
            });
        }
        if s.underline_color.is_some() {
            self.warn.unsupported(FORMAT, "underlineColor", id);
        }
        match s.strike {
            Some(Strike::Single) => run = run.strike(),
            Some(Strike::Double) => run = run.dstrike(),
            _ => {}
        }
        match s.vertical_align {
            Some(VerticalAlign::Super) => {
                run.run_property = run.run_property.vert_align(VertAlignType::SuperScript)
            }
            Some(VerticalAlign::Sub) => {
                run.run_property = run.run_property.vert_align(VertAlignType::SubScript)
            }
            _ => {}
        }
        match s.caps {
            Some(Caps::All) => run.run_property = run.run_property.caps(),
            Some(Caps::Small) => self.warn.unsupported(FORMAT, "caps: small", id),
            None => {}
        }
        if let Some(l) = s.letter_spacing {
            run = run.character_spacing(units::twip(l.pt(s.size_pt(), 0.0)));
        }
        if let Some(x) = s.scale {
            run = run.stretch(x.round() as i32);
        }
        if s.hidden == Some(true) {
            run = run.vanish();
        }
        // 任意颜色的高亮和行内背景都用底纹表达（w:highlight 只有 16 种固定色）
        if let Some(bg) = s.background.as_ref() {
            run = run.shading(shading(bg));
        } else if let Some(h) = &s.highlight {
            run = run.shading(Shading::new().shd_type(ShdType::Clear).fill(h.rgb()));
        }
        for (f, on) in [
            ("emphasisMark", s.emphasis_mark.is_some()),
            ("kerning", s.kerning.is_some()),
        ] {
            if on {
                self.warn.unsupported(FORMAT, f, id);
            }
        }
        run
    }
}

impl PStyle {
    /// 文字字段，去掉背景（背景已经画在段落上）。
    fn text_only_no_bg(&self) -> PStyle {
        let mut t = self.text_only();
        t.background = None;
        t
    }
}

/// 样式里的字体换成 w:rFonts；逗号分开的列表只取第一个。
fn fonts(s: &PStyle) -> Option<RunFonts> {
    let f = s.font_family.as_ref()?;
    let first = |x: &Option<String>| {
        x.as_ref()
            .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
    };
    let mut r = RunFonts::new();
    if let Some(l) = first(&f.latin) {
        r = r.ascii(&l).hi_ansi(&l);
    }
    if let Some(e) = first(&f.east_asia) {
        r = r.east_asia(&e);
    }
    if let Some(c) = first(&f.complex).or_else(|| first(&f.latin)) {
        r = r.cs(&c);
    }
    Some(r)
}

fn num_format(t: &ListStyleType) -> &'static str {
    match t {
        ListStyleType::Decimal => "decimal",
        ListStyleType::DecimalLeadingZero => "decimalZero",
        ListStyleType::LowerAlpha => "lowerLetter",
        ListStyleType::UpperAlpha => "upperLetter",
        ListStyleType::LowerRoman => "lowerRoman",
        ListStyleType::UpperRoman => "upperRoman",
        ListStyleType::CjkDecimal => "chineseCounting",
        ListStyleType::CjkIdeographic => "ideographTraditional",
        ListStyleType::CircledDecimal => "decimalEnclosedCircle",
        _ => "bullet",
    }
}

fn border_type(b: &Border) -> BorderType {
    if b.is_none() {
        return BorderType::Nil;
    }
    match b.style {
        BorderStyle::None => BorderType::Nil,
        BorderStyle::Solid => BorderType::Single,
        BorderStyle::Dashed => BorderType::Dashed,
        BorderStyle::Dotted => BorderType::Dotted,
        BorderStyle::DashDot => BorderType::DotDash,
        BorderStyle::Double => BorderType::Double,
        BorderStyle::Wave => BorderType::Wave,
    }
}

/// 线宽换成 1/8 pt，Word 只认 2 到 96。
fn eighths(pt: f32) -> usize {
    ((pt * 8.0).round() as usize).clamp(2, 96)
}

fn para_border(pos: ParagraphBorderPosition, b: &Border, space_pt: f32) -> ParagraphBorder {
    ParagraphBorder::new(pos)
        .val(border_type(b))
        .size(eighths(b.width))
        .space(space_pt.round().max(0.0) as usize)
        .color(b.color.rgb())
}

fn cell_border(pos: TableCellBorderPosition, b: Option<&Border>) -> TableCellBorder {
    match b {
        Some(b) if !b.is_none() => TableCellBorder::new(pos)
            .border_type(border_type(b))
            .size(eighths(b.width))
            .color(b.color.rgb()),
        _ => TableCellBorder::new(pos).border_type(BorderType::Nil),
    }
}

fn shading(b: &Background) -> Shading {
    match b {
        Background::Pattern {
            pattern,
            color,
            fill,
        } => Shading::new()
            .shd_type(ShdType::from_str(pattern).unwrap_or(ShdType::Clear))
            .color(color.rgb())
            .fill(fill.rgb()),
        b => Shading::new()
            .shd_type(ShdType::Clear)
            .fill(b.solid().rgb()),
    }
}
