//! docx 渲染器，基于 docx-rs。样式按算好的结果直接写在段落和 run 上（直接格式）；
//! 标题、代码另外挂上同名的 Word 样式，方便在 Word 里找和改。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::str::FromStr;

use docx_rs::{
    AbstractNumbering, AlignmentType, BorderType, BreakType, Docx, DrawingPosition, FieldCharType,
    Footer, Header, HeightRule, Hyperlink, HyperlinkType, IndentLevel, InstrNUMPAGES, InstrPAGE,
    InstrText, Level, LevelJc, LevelSuffixType, LevelText, LineSpacing, LineSpacingType,
    NumberFormat, Numbering, NumberingId, PageMargin, PageNumType, PageOrientationType, Paragraph,
    ParagraphBorder, ParagraphBorderPosition, ParagraphBorders, Pic, PicAlign, RelativeFromHType,
    RelativeFromVType, Run, RunFonts, Shading, ShdType, SpecialIndentType, Start, Style, StyleType,
    Table, TableCell, TableCellBorder, TableCellBorderPosition, TableCellBorders, TableCellMargins,
    TableLayoutType, TableRow, TextDirectionType, VAlignType, VMergeType, VertAlignType, WidthType,
};

use crate::{
    assets,
    components::{
        CodeProps, Component, Field, ImageProps, ListProps, Run as PRun, TableOfContentsProps,
        TableProps,
    },
    doc::{Doc, Node, Page, PageNumberFormat, SectionLayout},
    render::{
        Options, Output,
        common::{self, Grid, Warn},
    },
    style::{
        Align, AnchorTo, Background, Border, BorderStyle, Caps, ColWidth, Edge, HAlign, Len,
        LineHeight, ListStyleType, MarkerGap, Position, Side, Strike, Style as PStyle, TableLayout,
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
    // Word 更新目录域时用「目录 1–6」样式重写各项，按第一个目录的 entry 样式定义它们，
    // 更新后缩进、间距、字号才和更新前一样
    if let Some(toc) = find_toc(tree) {
        for lvl in 1..=6u8 {
            let es = common::toc_entry_style(toc, lvl);
            let fs = es.size_pt();
            let pt = |l: Option<Len>| l.map_or(0, |x| units::twip(x.pt(fs, 0.0)).max(0) as u32);
            let mut st = Style::new(format!("TOC{lvl}"), StyleType::Paragraph)
                .name(format!("toc {lvl}"))
                .based_on("Normal")
                .next("Normal")
                .size((fs * 2.0).round() as usize)
                .indent(Some(pt(es.margin_left) as i32), None, None, None)
                .line_spacing(
                    LineSpacing::new()
                        .before(pt(es.space_before))
                        .after(pt(es.space_after)),
                );
            if let Some(c) = &es.color {
                st = st.color(c.rgb());
            }
            docx = docx.add_style(st);
        }
    }

    let mut r = Ctx {
        warn,
        opts,
        content_width_pt: 0.0,
        // docx-rs 自带一个 id 为 1 的默认编号，从 2 开始免得撞上
        next_num: 2,
        blocks: Vec::new(),
        numberings: Vec::new(),
        repeat_heads: Vec::new(),
        ind_chars: Vec::new(),
        sections: Vec::new(),
        root: tree.clone(),
        maths: Vec::new(),
        alts: BTreeMap::new(),
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
    let fixes: Vec<SectFix> = sections
        .iter()
        .map(|(l, _)| SectFix {
            cols: l
                .page
                .columns
                .as_ref()
                .filter(|c| c.count() > 1)
                .map(|c| (c.count(), c.gap_pt())),
            fmt: l
                .page
                .page_number_format
                .filter(|f| *f != PageNumberFormat::Decimal),
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
            for b in merge_col_breaks(blocks) {
                sec = match b {
                    Block::P(p) => sec.add_paragraph(p),
                    Block::T(t) => sec.add_table(t),
                    Block::ColBreak => sec,
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
            for b in merge_col_breaks(blocks) {
                docx = match b {
                    Block::P(p) => docx.add_paragraph(p),
                    Block::T(t) => docx.add_table(t),
                    Block::ColBreak => docx,
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
    xml.document = fix_sections(&xml.document, &fixes);
    xml.document = mark_behind(&xml.document);
    // 每张图的 wp:docPr：id 在整份文档里不重复，有替代文字的写进 descr
    let mut pic_no = 0;
    // 正文的图先建，页眉页脚的图在各节收尾时才建，所以先正文、后页眉页脚
    let media: BTreeMap<String, u64> = xml
        .media
        .iter()
        .map(|(name, b)| (stem(name).to_string(), bytes_key(b)))
        .collect();
    let doc_rels = rel_targets(std::iter::once(&xml.document_rels));
    let hf_rels = rel_targets(xml.header_rels.iter().chain(xml.footer_rels.iter()));
    let key_of = |rels: &BTreeMap<String, String>, rid: &str| -> Option<u64> {
        let t = rels.get(rid)?;
        media.get(stem(t)).copied()
    };
    xml.document = fix_doc_pr(&xml.document, &mut r.alts, &mut pic_no, &|rid| {
        key_of(&doc_rels, rid)
    });
    for h in xml.headers.iter_mut().chain(xml.footers.iter_mut()) {
        *h = fix_doc_pr(h, &mut r.alts, &mut pic_no, &|rid| key_of(&hf_rels, rid));
    }
    xml.document = fix_ind_chars(&xml.document, &r.ind_chars);
    for h in xml.headers.iter_mut().chain(xml.footers.iter_mut()) {
        *h = mark_behind(h);
        *h = fix_ind_chars(h, &r.ind_chars);
    }
    if !r.maths.is_empty() {
        xml.document = inject_math(&xml.document, &r.maths, &mut r.warn);
        for h in xml.headers.iter_mut().chain(xml.footers.iter_mut()) {
            *h = inject_math(h, &r.maths, &mut r.warn);
        }
    }
    fix_part_rels(
        &xml.headers,
        &mut xml.header_rels,
        &xml.footers,
        &mut xml.footer_rels,
    );
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

/// 一个公式：先在文档里放一个写着记号的 run，生成 XML 后换成 OMML（docx-rs 不会写公式）。
struct MathSlot {
    id: String,
    latex: String,
    /// 块级公式（m:oMathPara），否则是行内（m:oMath）
    block: bool,
    /// 块级公式的对齐：left、center、right
    jc: &'static str,
}

const MATH_OPEN: char = '\u{E000}';
const MATH_CLOSE: char = '\u{E001}';
const MATH_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";

fn math_marker(i: usize) -> String {
    format!("{MATH_OPEN}fmtagno-math-{i}{MATH_CLOSE}")
}

/// 把写着公式记号的 run 换成 OMML。公式的字号、颜色从这个 run 的 w:rPr 里取，
/// 字体换成 Cambria Math。根元素上没有 m 命名空间时补上。
fn inject_math(xml: &[u8], slots: &[MathSlot], warn: &mut Warn) -> Vec<u8> {
    let s = String::from_utf8_lossy(xml);
    if !s.contains(MATH_OPEN) {
        return xml.to_vec();
    }
    let mut out = String::with_capacity(s.len() + 1024);
    let mut rest: &str = &s;
    let open_len = MATH_OPEN.len_utf8();
    while let Some(m) = rest.find(MATH_OPEN) {
        let Some(m_end) = rest[m..].find(MATH_CLOSE).map(|j| m + j) else {
            break;
        };
        let idx: Option<usize> = rest[m + open_len..m_end]
            .strip_prefix("fmtagno-math-")
            .and_then(|x| x.parse().ok());
        let start = [rest[..m].rfind("<w:r>"), rest[..m].rfind("<w:r ")]
            .into_iter()
            .flatten()
            .max();
        let end = rest[m_end..]
            .find("</w:r>")
            .map(|j| m_end + j + "</w:r>".len());
        let (Some(idx), Some(start), Some(end)) = (idx, start, end) else {
            break;
        };
        let Some(slot) = slots.get(idx) else { break };
        let run = &rest[start..end];
        out.push_str(&rest[..start]);
        let rpr = run
            .find("<w:rPr>")
            .and_then(|i| run[i..].find("</w:rPr>").map(|j| &run[i..i + j]))
            .unwrap_or("");
        match crate::math::parse(&slot.latex, slot.block) {
            Ok(root) => {
                let mrpr = math_rpr(rpr);
                let mut o = crate::math::Omml::new(&mrpr);
                let x = if slot.block {
                    o.block(&root, slot.jc)
                } else {
                    o.inline(&root)
                };
                warn.math_partial(FORMAT, &slot.id, &slot.latex, &o.unsupported);
                out.push_str(&x);
            }
            Err(e) => {
                warn.math_failed(FORMAT, &slot.id, &slot.latex, &e);
                let marker = math_marker(idx);
                out.push_str(&run.replace(&marker, &xml_text(&slot.latex)));
            }
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    // 根元素补上 m 命名空间
    if !out.contains("xmlns:m=")
        && let Some(i) = ["<w:document", "<w:hdr", "<w:ftr"]
            .iter()
            .find_map(|t| out.find(t).map(|i| i + t.len()))
    {
        out.insert_str(i, &format!(" xmlns:m=\"{MATH_NS}\""));
    }
    out.into_bytes()
}

fn xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 公式里每个 m:r 用的 w:rPr：只留粗体、颜色、字号，字体用 Cambria Math。
fn math_rpr(rpr: &str) -> String {
    let take = |name: &str| -> Option<String> {
        let k = format!("<w:{name}");
        let mut from = 0;
        while let Some(i) = rpr[from..].find(&k).map(|i| from + i) {
            let after = &rpr[i + k.len()..];
            if after.starts_with([' ', '/', '>']) {
                let j = after.find("/>")?;
                return Some(rpr[i..i + k.len() + j + 2].to_string());
            }
            from = i + k.len();
        }
        None
    };
    let mut o = String::from(
        "<w:rPr><w:rFonts w:ascii=\"Cambria Math\" w:hAnsi=\"Cambria Math\" w:cs=\"Cambria Math\" />",
    );
    for name in ["b", "color", "sz", "szCs"] {
        if let Some(e) = take(name) {
            o.push_str(&e);
        }
    }
    o.push_str("</w:rPr>");
    o
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

/// docx-rs 只按三个固定位置给页眉页脚挂图片关系，页眉页脚多了（每节一个、加占位节）
/// 图片就挂错文件，Word 里不显示。这里按每个页眉页脚里实际引用的 r:embed / r:id 重排关系。
fn fix_part_rels(
    headers: &[Vec<u8>],
    header_rels: &mut Vec<Vec<u8>>,
    footers: &[Vec<u8>],
    footer_rels: &mut Vec<Vec<u8>>,
) {
    let mut all: BTreeMap<String, String> = BTreeMap::new();
    for r in header_rels.iter().chain(footer_rels.iter()) {
        let r = String::from_utf8_lossy(r);
        for part in r.split("<Relationship ").skip(1) {
            let Some(end) = part.find("/>") else { continue };
            let el = format!("<Relationship {}/>", &part[..end]);
            if let Some(id) = attr(&el, "Id") {
                all.insert(id, el);
            }
        }
    }
    if all.is_empty() {
        return;
    }
    let rebuild = |parts: &[Vec<u8>], rels: &mut Vec<Vec<u8>>| {
        rels.resize(parts.len().max(rels.len()), Vec::new());
        for (i, p) in parts.iter().enumerate() {
            let p = String::from_utf8_lossy(p);
            let mut out = String::from(
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
            );
            for (id, el) in &all {
                if p.contains(&format!("r:embed=\"{id}\"")) || p.contains(&format!("r:id=\"{id}\""))
                {
                    out.push_str(el);
                }
            }
            out.push_str("</Relationships>");
            rels[i] = out.into_bytes();
        }
    };
    rebuild(headers, header_rels);
    rebuild(footers, footer_rels);
}

fn attr(el: &str, name: &str) -> Option<String> {
    let k = format!(" {name}=\"");
    let i = el.find(&k)? + k.len();
    let j = el[i..].find('"')?;
    Some(el[i..i + j].to_string())
}

/// 收拾 docx-rs 生成的分节：
/// 1. 把占位节（纸张 1×1）上的页眉页脚引用挪到它前面那个真正的节上，删掉占位节的段落；
/// 2. sectPr 里的子元素按 OOXML 规定的顺序排（headerReference 在最前，titlePg 在 cols 后面）；
/// 3. 按 `fixes` 写分栏和页码格式（docx-rs 的 Section 不让设）。
fn fix_sections(doc: &[u8], fixes: &[SectFix]) -> Vec<u8> {
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
            let fix = fixes.get(n);
            if let Some((count, gap)) = fix.and_then(|f| f.cols) {
                let c = format!(
                    "<w:cols w:space=\"{}\" w:num=\"{count}\" />",
                    units::twip(gap)
                );
                match els.iter_mut().find(|e| e.starts_with("<w:cols ")) {
                    Some(e) => *e = c,
                    None => els.push(c),
                }
            }
            if let Some(fmt) = fix.and_then(|f| f.fmt) {
                let a = format!(" w:fmt=\"{}\"", fmt.word());
                match els.iter_mut().find(|e| e.starts_with("<w:pgNumType")) {
                    Some(e) => e.insert_str("<w:pgNumType".len(), &a),
                    None => els.push(format!("<w:pgNumType{a} />")),
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

/// 一节里 docx-rs 写不了、要在生成 XML 后补上的设置。
struct SectFix {
    /// 栏数和栏距（pt），只在多于一栏时有
    cols: Option<(u8, f32)>,
    /// 页码格式，阿拉伯数字时不写
    fmt: Option<PageNumberFormat>,
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
/// 衬于文字下方的图片在 relativeHeight 上的记号，见 `mark_behind`。
const BEHIND_MARK: u32 = 190_499;

/// 一段按字符计的缩进，单位是百分之一字符（Word 的 leftChars 等）。
#[derive(Default)]
struct IndChars {
    /// 真正的右缩进（twip），w:right 先写成占位值
    right: i32,
    left_chars: Option<i32>,
    right_chars: Option<i32>,
    hanging_chars: Option<i32>,
    first_line_chars: Option<i32>,
}

/// w:right 写成 IND_MARK + 序号，生成 XML 后按序号换回真值并补上 *Chars。
const IND_MARK: i32 = 1_000_000_000;

/// docx-rs 只写 w:left / w:right / w:hanging / w:firstLine（hangingChars 等存了但不输出），
/// 这里把按字符计的缩进补进 <w:ind>，Word 改字号时缩进跟着变。
fn fix_ind_chars(xml: &[u8], list: &[IndChars]) -> Vec<u8> {
    if list.is_empty() {
        return xml.to_vec();
    }
    let s = String::from_utf8_lossy(xml);
    let mut out = String::with_capacity(s.len());
    let mut rest: &str = &s;
    const KEY: &str = "w:right=\"";
    while let Some(i) = rest.find("<w:ind ") {
        let end = rest[i..].find('>').map(|e| i + e).unwrap_or(rest.len());
        let tag = &rest[i..end];
        out.push_str(&rest[..i]);
        let hit = tag.find(KEY).and_then(|k| {
            let v = &tag[k + KEY.len()..];
            let q = v.find('"')?;
            let n: i32 = v[..q].parse().ok()?;
            let ic = list.get(usize::try_from(n.checked_sub(IND_MARK)?).ok()?)?;
            Some((k, k + KEY.len() + q + 1, ic))
        });
        match hit {
            Some((a, b, ic)) => {
                let t0 = format!("{}w:right=\"{}\"{}", &tag[..a], ic.right, &tag[b..]);
                // 自闭合标签的 / 要留在最后，先拿掉，补完属性再加回
                let (body, close) = match t0.trim_end().strip_suffix('/') {
                    Some(x) => (x.trim_end().to_string(), " /"),
                    None => (t0.clone(), ""),
                };
                let mut t = body;
                for (name, v) in [
                    ("leftChars", ic.left_chars),
                    ("rightChars", ic.right_chars),
                    ("hangingChars", ic.hanging_chars),
                    ("firstLineChars", ic.first_line_chars),
                ] {
                    if let Some(v) = v
                        && !t.contains(&format!("w:{name}="))
                    {
                        t.push_str(&format!(" w:{name}=\"{v}\""));
                    }
                }
                out.push_str(&t);
                out.push_str(close);
            }
            None => out.push_str(tag),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.into_bytes()
}

/// 图片内容的指纹，用来把建图时记下的替代文字和 XML 里的图对上。
fn bytes_key(b: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.hash(&mut h);
    h.finish()
}

/// 路径里的文件名去掉扩展名：media 里存的名字不带扩展名，rels 的 Target 带。
fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(a, _)| a)
}

/// .rels 里的关系 id → Target。
fn rel_targets<'a>(rels: impl Iterator<Item = &'a Vec<u8>>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for r in rels {
        let r = String::from_utf8_lossy(r);
        for part in r.split("<Relationship ").skip(1) {
            let el = &part[..part.find("/>").unwrap_or(part.len())];
            let el = format!(" {el}");
            if let (Some(id), Some(t)) = (attr(&el, "Id"), attr(&el, "Target")) {
                out.insert(id, t);
            }
        }
    }
    out
}

/// docx-rs 给每张图都写 `<wp:docPr id="1" name="Figure" />`：Word 要求 id 不重复，
/// 替代文字也没处写。这里按顺序换成不重复的 id，再按后面紧跟的 r:embed 找到图片内容，
/// 取出这张图的替代文字写进 descr。
fn fix_doc_pr(
    xml: &[u8],
    alts: &mut Alts,
    no: &mut u32,
    key_of: &dyn Fn(&str) -> Option<u64>,
) -> Vec<u8> {
    const OLD: &str = "<wp:docPr id=\"1\" name=\"Figure\" />";
    let s = String::from_utf8_lossy(xml);
    if !s.contains(OLD) {
        return xml.to_vec();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest: &str = &s;
    while let Some(i) = rest.find(OLD) {
        out.push_str(&rest[..i]);
        let after = &rest[i + OLD.len()..];
        // 这张图自己的 r:embed（在下一个 docPr 之前）
        let scope = &after[..after.find("<wp:docPr ").unwrap_or(after.len())];
        let alt = attr(scope, "r:embed")
            .and_then(|rid| key_of(&rid))
            .and_then(|k| alts.get_mut(&k))
            .and_then(|q| q.pop_front())
            .filter(|a| !a.is_empty());
        *no += 1;
        let _ = write!(out, "<wp:docPr id=\"{no}\" name=\"Picture {no}\"");
        if let Some(a) = alt {
            let _ = write!(out, " descr=\"{}\"", xml_attr(&a));
        }
        out.push_str(" />");
        rest = after;
    }
    out.push_str(rest);
    out.into_bytes()
}

type Alts = BTreeMap<u64, std::collections::VecDeque<String>>;

/// XML 属性值转义，换行也转成字符引用（不然读回来会变成空格）。
fn xml_attr(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\n' => o.push_str("&#10;"),
            '\r' => o.push_str("&#13;"),
            '\t' => o.push_str("&#9;"),
            c => o.push(c),
        }
    }
    o
}

/// 把带记号的浮动图片改成衬于文字下方（behindDoc="1"）。
fn mark_behind(xml: &[u8]) -> Vec<u8> {
    let s = String::from_utf8_lossy(xml);
    let mark = format!("relativeHeight=\"{BEHIND_MARK}\"");
    if !s.contains(&mark) {
        return xml.to_vec();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest: &str = &s;
    while let Some(i) = rest.find("<wp:anchor ") {
        let end = rest[i..].find('>').map(|e| i + e).unwrap_or(rest.len());
        let tag = &rest[i..end];
        out.push_str(&rest[..i]);
        if tag.contains(&mark) {
            out.push_str(&tag.replace("behindDoc=\"0\"", "behindDoc=\"1\""));
        } else {
            out.push_str(tag);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.into_bytes()
}

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
    /// 分栏符，写文件前并进下一段开头（见 merge_col_breaks）
    ColBreak,
}

fn col_break_run() -> Run {
    Run::new().add_break(BreakType::Column)
}

/// 分栏符单独占一段时，Word 在符号处换栏，但这一段的段落标记会跟到下一栏顶上，
/// 多出一个空行。所以把分栏符放进下一段第一个 run 前面；后面不是段落（表格或节尾）时才单独占一段。
fn merge_col_breaks(blocks: Vec<Block>) -> Vec<Block> {
    let mut out = Vec::with_capacity(blocks.len());
    let mut pending = false;
    for b in blocks {
        match b {
            Block::ColBreak => {
                if pending {
                    out.push(Block::P(Paragraph::new().add_run(col_break_run())));
                }
                pending = true;
            }
            Block::P(mut p) => {
                if std::mem::take(&mut pending) {
                    p.children
                        .insert(0, docx_rs::ParagraphChild::Run(Box::new(col_break_run())));
                }
                out.push(Block::P(p));
            }
            Block::T(t) => {
                if std::mem::take(&mut pending) {
                    out.push(Block::P(Paragraph::new().add_run(col_break_run())));
                }
                out.push(Block::T(t));
            }
        }
    }
    if pending {
        out.push(Block::P(Paragraph::new().add_run(col_break_run())));
    }
    out
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
    /// 按字符计的缩进（em），docx-rs 写不出来，生成 XML 后补上（fix_ind_chars）
    ind_chars: Vec<IndChars>,
    /// 每节的版面和内容
    sections: Vec<(SectionLayout, Vec<Block>)>,
    /// 整棵树，目录要从里面收标题
    root: Node,
    /// 公式，按记号里的序号排
    maths: Vec<MathSlot>,
    /// 图片的替代文字，生成 XML 后写进 wp:docPr。docx-rs 会把内容相同的图片合成一份、
    /// 改掉关系 id，所以按图片内容分组，每组按建图的顺序排（没写的也占一个位置）
    alts: Alts,
}

impl Ctx<'_> {
    /// 登记一个公式，返回占位 run 里要写的记号。
    fn math_slot(&mut self, id: &str, latex: &str, block: bool, jc: &'static str) -> String {
        self.maths.push(MathSlot {
            id: id.to_string(),
            latex: latex.to_string(),
            block,
            jc,
        });
        math_marker(self.maths.len() - 1)
    }

    /// 块级公式：单独一段。带编号时用居中制表位放公式、右对齐制表位放编号，
    /// 这时公式按行内公式排。
    fn math_block(&mut self, n: &Node, p: &crate::components::MathProps) {
        let s = &n.style;
        let ts = s.text_only_no_bg();
        match &p.number {
            None => {
                let jc = match s.align {
                    Some(Align::Left) => "left",
                    Some(Align::Right) => "right",
                    _ => "center",
                };
                let mark = self.math_slot(&n.id, &p.latex, true, jc);
                let para = self.para(Paragraph::new(), s, &n.id);
                let run = self.run_props(Run::new().add_text(&mark), &ts, &n.id);
                self.blocks.push(Block::P(para.add_run(run)));
            }
            Some(no) => {
                let mut ls = s.clone();
                ls.align = Some(Align::Left);
                let w = units::twip(self.content_width_pt).max(0) as usize;
                let mark = self.math_slot(&n.id, &p.latex, false, "center");
                let para = self
                    .para(Paragraph::new(), &ls, &n.id)
                    .add_tab(
                        docx_rs::Tab::new()
                            .val(docx_rs::TabValueType::Center)
                            .pos(w / 2),
                    )
                    .add_tab(docx_rs::Tab::new().val(docx_rs::TabValueType::Right).pos(w));
                let tab1 = self.run_props(Run::new().add_tab(), &ts, &n.id);
                let eq = self.run_props(Run::new().add_text(&mark), &ts, &n.id);
                let tab2 = self.run_props(Run::new().add_tab(), &ts, &n.id);
                let num = self.run_props(Run::new().add_text(format!("({no})")), &ts, &n.id);
                let para = para.add_run(tab1).add_run(eq).add_run(tab2).add_run(num);
                self.blocks.push(Block::P(para));
            }
        }
    }

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
            Component::Quote(p) => {
                let bs = boxed(s);
                let para = self.para(Paragraph::new(), &bs, &n.id);
                let para = self.inline(para, p.text.runs().cloned().collect(), &n.id);
                self.blocks.push(Block::P(para));
            }
            Component::Callout(p) => {
                let bs = boxed(s);
                let para = self.para(Paragraph::new(), &bs, &n.id);
                let para = self.inline(para, p.text.runs().cloned().collect(), &n.id);
                self.blocks.push(Block::P(para));
            }
            Component::Divider(_) => {
                // 空段落，行高按样式（默认固定 1pt），线画在段落下边
                let para = self.para(Paragraph::new(), s, &n.id);
                self.blocks.push(Block::P(para));
            }
            Component::TableOfContents(p) => self.toc(n, p),
            Component::Code(p) => self.code(n, p),
            Component::Math(p) => self.math_block(n, p),
            Component::List(p) => self.list(n, p),
            Component::Table(p) => self.table(n, p),
            Component::Image(p) => self.image(n, p),
            Component::PageBreak(_) => self.page_break(),
            Component::ColumnBreak(_) => self.blocks.push(Block::ColBreak),
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
                Block::ColBreak => h,
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
                Block::ColBreak => f,
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

    /// 目录：Word 的 TOC 域，先填好标题文字（没有页码），打开后更新域就有页码。
    fn toc(&mut self, n: &Node, p: &TableOfContentsProps) {
        let depth = p.depth();
        if let Some(t) = &p.title {
            let ts = common::toc_title_style(n);
            let para = self.para(Paragraph::new(), &ts, &n.id);
            let para = para.add_run(self.run_props(Run::new().add_text(t), &ts.text_only(), &n.id));
            self.blocks.push(Block::P(para));
        }
        let entries = common::toc_entries(&self.root, depth);
        let instr = format!("TOC \\o \"1-{depth}\" \\h \\z \\u");
        let begin = [
            Run::new().add_field_char(FieldCharType::Begin, true),
            Run::new().add_instr_text(InstrText::Unsupported(instr)),
            Run::new().add_field_char(FieldCharType::Separate, false),
        ];
        let after = n.style.space_after;
        if entries.is_empty() {
            let mut es = common::toc_entry_style(n, 1);
            es.space_after = after.or(es.space_after);
            let mut para = self.para(Paragraph::new(), &es, &n.id);
            for r in begin {
                para = para.add_run(r);
            }
            para = para.add_run(Run::new().add_field_char(FieldCharType::End, false));
            self.blocks.push(Block::P(para));
            return;
        }
        let last = entries.len() - 1;
        let tab = units::twip(self.content_width_pt).max(0) as usize;
        let mut begin = Some(begin);
        for (i, e) in entries.iter().enumerate() {
            let mut es = common::toc_entry_style(n, e.level);
            if i == last && after.is_some() {
                es.space_after = after;
            }
            let mut para = self.para(
                Paragraph::new().style(&format!("TOC{}", e.level)).add_tab(
                    docx_rs::Tab::new()
                        .val(docx_rs::TabValueType::Right)
                        .leader(docx_rs::TabLeaderType::Dot)
                        .pos(tab),
                ),
                &es,
                &n.id,
            );
            if let Some(b) = begin.take() {
                for r in b {
                    para = para.add_run(r);
                }
            }
            para =
                para.add_run(self.run_props(Run::new().add_text(&e.text), &es.text_only(), &n.id));
            if i == last {
                para = para.add_run(Run::new().add_field_char(FieldCharType::End, false));
            }
            self.blocks.push(Block::P(para));
        }
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
            // Word 的编号字号、颜色跟段落标记走，fmtagno 不写段落标记的 rPr，
            // 没指定时就用列表文字的字号和颜色，不然编号会是 Word 默认的五号黑字
            if ms.color.is_none() {
                ms.color = s.color.clone();
            }
            if let Some(c) = &ms.color {
                level = level.color(c.rgb());
            }
            let msz = if ms.font_size.is_some() {
                ms.size_pt()
            } else {
                fs
            };
            level = level.size((msz * 2.0).round() as usize);
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
        let pad_pt = s
            .cell_padding
            .map(|x| x.pt(s.size_pt(), 0.0))
            .unwrap_or(4.0);
        let pad = units::twip(pad_pt);
        let ws = common::col_widths(p, &g, &mut self.warn, &n.id);
        let nat = common::auto_natural(n, p, &g, &ws, pad_pt);
        let ws = common::resolve_auto(&ws, &nat, width_pt);
        let fills = common::fills_width(&ws);
        // auto 列按最宽的格子估一个初始宽度，Word 打开时按 autofit 自己再排
        let auto_pt: Vec<f32> = ws
            .iter()
            .zip(&nat)
            .map(|(w, a)| if *w == ColWidth::Auto { *a } else { 0.0 })
            .collect();
        let fixed_sum: f32 = ws
            .iter()
            .zip(&auto_pt)
            .map(|(w, a)| match w {
                ColWidth::Fixed(x) => *x,
                ColWidth::Auto => *a,
                ColWidth::Ratio(_) => 0.0,
            })
            .sum();
        let ratio_sum: f32 = ws
            .iter()
            .map(|w| if let ColWidth::Ratio(x) = w { *x } else { 0.0 })
            .sum();
        let rest = (width_pt - fixed_sum).max(0.0);
        let grid_pt: Vec<f32> = ws
            .iter()
            .zip(&auto_pt)
            .map(|(w, a)| match w {
                ColWidth::Fixed(x) => *x,
                ColWidth::Auto => *a,
                ColWidth::Ratio(x) => rest * x / ratio_sum,
            })
            .collect();
        let grid: Vec<usize> = grid_pt.iter().map(|x| units::twip(*x) as usize).collect();
        let total: usize = if fills || s.width.is_some() {
            units::twip(width_pt) as usize
        } else {
            grid.iter().sum()
        };
        g.warn_ragged(&mut self.warn, &n.id);
        let mut rows = Vec::new();
        for r in 0..g.rows {
            let mut tcs = Vec::new();
            for c in 0..g.cols {
                let sl = *g.slot(r, c);
                let (r0, c0) = sl.origin;
                // 横向被合并掉的位置不写；纵向合并在下面几行写 vMerge continue 的格子
                if c0 != c {
                    continue;
                }
                let cont = r0 != r;
                let width: usize = grid[c..c + sl.col_span].iter().sum();
                let width = &width;
                let part = common::cell_parts(n, &p.look, &g, r0, c0);
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
                let para = if cont {
                    para
                } else {
                    let runs = common::rebase_runs(sl.text(), s, &cs);
                    self.inline(para, runs, &n.id)
                };
                let mut cell = TableCell::new().add_paragraph(para);
                if sl.col_span > 1 {
                    cell = cell.grid_span(sl.col_span);
                }
                if sl.row_span > 1 {
                    cell = cell.vertical_merge(if cont {
                        VMergeType::Continue
                    } else {
                        VMergeType::Restart
                    });
                }
                cell = if sl.col_span == 1 && ws[c] == ColWidth::Auto {
                    cell.width(0, WidthType::Auto)
                } else {
                    cell.width(*width, WidthType::Dxa)
                };
                if let Some(bg) = part.bg() {
                    cell = cell.shading(shading(bg));
                }
                // 两格之间的线只写一次：上下之间写在上面一格的下边，左右之间写在左边一格的右边。
                // 两边都写的话 Word 会画两条（双线尤其明显）。
                // 合并格：上下边只在第一行、最后一行写，右边取最后一列
                let last_row = r == r0 + sl.row_span - 1;
                let cr = c + sl.col_span - 1;
                let mut b = common::cell_edges(n, &p.look, &g, r, c);
                b[1] = common::cell_edges(n, &p.look, &g, r, cr)[1].clone();
                if r + 1 < g.rows && last_row {
                    b[2] = common::cell_edges(n, &p.look, &g, r + 1, c)[0].clone();
                }
                if !last_row {
                    b[2] = None;
                }
                if cont {
                    b[0] = None;
                }
                if cr + 1 < g.cols {
                    b[1] = common::cell_edges(n, &p.look, &g, r, cr + 1)[3].clone();
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
                // 部位上写的 padding 落成这一格的 w:tcMar，没写的边沿用表格的 cellPadding
                for e in Edge::ALL {
                    if let Some(v) = part.padding_side(e) {
                        let v = units::twip(v) as usize;
                        let pr = cell.property;
                        cell.property = match e {
                            Edge::Top => pr.margin_top(v, WidthType::Dxa),
                            Edge::Right => pr.margin_right(v, WidthType::Dxa),
                            Edge::Bottom => pr.margin_bottom(v, WidthType::Dxa),
                            Edge::Left => pr.margin_left(v, WidthType::Dxa),
                        };
                    }
                }
                tcs.push(cell);
            }
            let mut row = TableRow::new(tcs);
            if s.row_break == Some(false) {
                row = row.cant_split();
            }
            if let Some(h) = g.row_height(p, r) {
                row = row
                    .row_height(units::twip(h.height) as f32)
                    .height_rule(if h.exact {
                        HeightRule::Exact
                    } else {
                        HeightRule::AtLeast
                    });
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
                Some(TableLayout::Fixed) => TableLayoutType::Fixed,
                // 有 auto 列时让 Word 按内容排
                None if ws.contains(&ColWidth::Auto) => TableLayoutType::Autofit,
                None => TableLayoutType::Fixed,
            })
            .margins(TableCellMargins::new().margin(
                pad as usize,
                pad as usize,
                pad as usize,
                pad as usize,
            ))
            // 按内容收窄的表格 tblW 写 auto，不然 Word 会被这个总宽卡住，auto 列撑不开
            .width(
                if fills || s.width.is_some() { total } else { 0 },
                if fills || s.width.is_some() {
                    WidthType::Dxa
                } else {
                    WidthType::Auto
                },
            );
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
        // 绝对定位的图片（比如整页背景）可以比版心宽，不缩
        if w > self.content_width_pt && s.position != Some(Position::Absolute) {
            h *= self.content_width_pt / w;
            w = self.content_width_pt;
        }
        let mut pic = Pic::new(&asset.bytes).size(units::emu(w), units::emu(h));
        self.alt(&pic, &p.alt);
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
            if s.position.unwrap_or(Position::Inline) == Position::Inline {
                self.warn.unsupported(
                    FORMAT,
                    "wrap: behind（只对浮动、绝对定位的图片有效）",
                    &n.id,
                );
            } else {
                // docx-rs 把 behindDoc 写死成 0：用 relativeHeight 做记号，生成 XML 后再改
                pic = pic.relative_height(BEHIND_MARK);
            }
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
        let mut para = self.para(Paragraph::new(), &ps, &n.id);
        // 按页或页边距定位的图片不该占正文的行：Word 的锚点得落在一个段落里，
        // 把这个段落压成 1pt 高、无段前段后，别把后面的内容往下推（PDF、HTML 里它本来就不占位）
        if s.position == Some(Position::Absolute)
            && !matches!(s.anchor_to, Some(AnchorTo::Paragraph))
        {
            para = para.line_spacing(
                LineSpacing::new()
                    .before(0)
                    .after(0)
                    .line(20)
                    .line_rule(LineSpacingType::Exact),
            );
        }
        let para = para.add_run(Run::new().add_image(pic).size(2));
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
            // 倍数按 CSS 理解：字号 × 倍数。Word 的 auto 是乘字体自己的单倍行高
            // （雅黑约 1.32 倍字号），会比 HTML、PDF 高出三四成，所以换成「至少」。
            Some(LineHeight::Multiple(x)) => {
                ls = ls
                    .line(units::twip(fs * x))
                    .line_rule(LineSpacingType::AtLeast);
                any = true;
            }
            Some(LineHeight::Exact(l)) => {
                ls = ls
                    .line(units::twip(l.pt(fs, 0.0)))
                    .line_rule(LineSpacingType::Exact);
                any = true;
            }
            // Word 自己的多倍行距：line 是倍数 × 240
            Some(LineHeight::FontMultiple(x)) => {
                ls = ls
                    .line((x * 240.0).round().max(1.0) as i32)
                    .line_rule(LineSpacingType::Auto);
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
        // Word 的 hanging 是首行往左伸出去；我们的 hangingIndent 是首行不动、其余行往里缩，
        // 和 PDF、HTML 一样，所以左缩进要加上悬挂量
        let left = match (s.margin_left, s.hanging_indent) {
            (ml, Some(h)) => Some(units::twip(
                ml.map(|l| l.pt(fs, 0.0)).unwrap_or(0.0) + h.pt(fs, 0.0),
            )),
            (Some(l), None) => Some(units::twip(l.pt(fs, 0.0))),
            (None, None) => None,
        };
        let right = s.margin_right.map(|l| units::twip(l.pt(fs, 0.0)));
        let special = match (s.hanging_indent, s.text_indent) {
            (Some(h), _) => Some(SpecialIndentType::Hanging(units::twip(h.pt(fs, 0.0)))),
            (None, Some(t)) => Some(SpecialIndentType::FirstLine(units::twip(t.pt(fs, 0.0)))),
            _ => None,
        };
        if left.is_some() || right.is_some() || special.is_some() {
            // 写成 em 的缩进同时写 Word 的字符单位，Word 里改字号时跟着变
            let em = |l: Option<Len>| match l {
                Some(Len::Em(x)) => Some((x * 100.0).round() as i32),
                _ => None,
            };
            let ic = IndChars {
                right: right.unwrap_or(0),
                left_chars: match (s.margin_left, s.hanging_indent) {
                    (Some(Len::Em(l)), Some(Len::Em(h))) => Some(((l + h) * 100.0).round() as i32),
                    (None, Some(Len::Em(h))) => Some((h * 100.0).round() as i32),
                    (l, None) => em(l),
                    _ => None,
                },
                right_chars: em(s.margin_right),
                hanging_chars: em(s.hanging_indent),
                first_line_chars: if s.hanging_indent.is_none() {
                    em(s.text_indent)
                } else {
                    None
                },
            };
            let right = if ic.left_chars.is_some()
                || ic.right_chars.is_some()
                || ic.hanging_chars.is_some()
                || ic.first_line_chars.is_some()
            {
                self.ind_chars.push(ic);
                Some(IND_MARK + self.ind_chars.len() as i32 - 1)
            } else {
                right
            };
            p = p.indent(left, special, right, None);
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
        if let Some(bg) = s.bg() {
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
        } else if Edge::ALL.iter().any(|e| s.padding_side(*e).is_some()) && s.bg().is_some() {
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
            if let Some(src) = &r.image {
                if let Some(pic) = self.inline_pic(id, src, r.width, r.height) {
                    // 行内图片的 text 就是替代文字
                    self.alt(&pic, &r.text);
                    p = p.add_run(Run::new().add_image(pic));
                }
                continue;
            }
            if r.is_math() {
                // 公式里放不了链接（OMML 不能套在 w:hyperlink 里），只放公式
                let mark = self.math_slot(id, &r.text, false, "center");
                let run = self.run_props(Run::new().add_text(&mark), s, id);
                p = p.add_run(run);
                continue;
            }
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

    /// 记下一张图的替代文字。每张图都要记，空的也记，好和 XML 里的顺序对上。
    fn alt(&mut self, pic: &Pic, alt: &str) {
        self.alts
            .entry(bytes_key(&pic.image))
            .or_default()
            .push_back(alt.to_string());
    }

    /// 行内图片（runs 里的 image）：按写的宽高，只写一边时按比例。
    fn inline_pic(&mut self, id: &str, src: &str, w: Option<f32>, h: Option<f32>) -> Option<Pic> {
        let asset = if assets::is_remote(src) {
            Err("docx 暂不支持远程图片".to_string())
        } else {
            assets::load(src, self.opts.base_dir.as_deref()).map_err(|e| format!("{e:#}"))
        };
        let got = asset.and_then(|a| match image::load_from_memory(&a.bytes) {
            Ok(img) => Ok((a, img.width() as f32 * 0.75, img.height() as f32 * 0.75)),
            Err(_) => Err(format!("{} 格式放不进 docx", a.mime)),
        });
        let (a, pw, ph) = match got {
            Ok(x) => x,
            Err(why) => {
                self.warn
                    .push(format!("`{id}` 里的行内图片 `{src}`：{why}"));
                return None;
            }
        };
        let (w, h) = match (w, h) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * ph / pw),
            (None, Some(h)) => (h * pw / ph, h),
            (None, None) => (pw, ph),
        };
        Some(Pic::new(&a.bytes).size(units::emu(w), units::emu(h)))
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
        if let Some(bg) = s.bg() {
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
        BorderStyle::Thick => BorderType::Thick,
        BorderStyle::DashDotDot => BorderType::DotDotDash,
        BorderStyle::DashSmallGap => BorderType::DashSmallGap,
        BorderStyle::DashDotStroked => BorderType::DashDotStroked,
        BorderStyle::Triple => BorderType::Triple,
        BorderStyle::ThinThickSmallGap => BorderType::ThinThickSmallGap,
        BorderStyle::ThickThinSmallGap => BorderType::ThickThinSmallGap,
        BorderStyle::ThinThickThinSmallGap => BorderType::ThinThickThinSmallGap,
        BorderStyle::ThinThickMediumGap => BorderType::ThinThickMediumGap,
        BorderStyle::ThickThinMediumGap => BorderType::ThickThinMediumGap,
        BorderStyle::ThinThickThinMediumGap => BorderType::ThinThickThinMediumGap,
        BorderStyle::ThinThickLargeGap => BorderType::ThinThickLargeGap,
        BorderStyle::ThickThinLargeGap => BorderType::ThickThinLargeGap,
        BorderStyle::ThinThickThinLargeGap => BorderType::ThinThickThinLargeGap,
        BorderStyle::DoubleWave => BorderType::DoubleWave,
        BorderStyle::ThreeDEmboss => BorderType::ThreeDEmboss,
        BorderStyle::ThreeDEngrave => BorderType::ThreeDEngrave,
        BorderStyle::Outset => BorderType::Outset,
        BorderStyle::Inset => BorderType::Inset,
    }
}

/// 线宽换成 1/8 pt，Word 只认 2 到 96。
fn eighths(pt: f32) -> usize {
    ((pt * 8.0).round() as usize).clamp(2, 96)
}

fn find_toc(n: &Node) -> Option<&Node> {
    if matches!(n.component, Component::TableOfContents(_)) {
        return Some(n);
    }
    n.children.iter().find_map(find_toc)
}

/// Word 把段落左右边框画在缩进外面（伸进页边），PDF、HTML 画在盒子边上、字往里缩。
/// 把内边距和线宽加进左右缩进，三种格式的字和线就对齐了。
fn boxed(s: &PStyle) -> PStyle {
    let mut b = s.clone();
    let fs = s.size_pt();
    for (e, m) in [
        (Edge::Left, &mut b.margin_left),
        (Edge::Right, &mut b.margin_right),
    ] {
        let Some(line) = s.border_side(e).filter(|x| x.width > 0.0) else {
            continue;
        };
        let add = s.padding_side(e).unwrap_or(0.0) + line.width;
        let base = m.map(|x| x.pt(fs, 0.0)).unwrap_or(0.0);
        *m = Some(Len::Pt(base + add));
    }
    b
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
