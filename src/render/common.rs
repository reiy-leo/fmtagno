//! 三个渲染器共用的计算：列表编号、表格部位、警告去重。

use std::collections::HashSet;

use crate::{
    components::{Cell, Inline, ListItem, RowHeight, Run, TableLook, TableProps},
    doc::Node,
    style::{Border, ColWidth, Edge, ListStyleType, Style},
};

/// 同一类警告只报一次。
#[derive(Default)]
pub struct Warn {
    pub list: Vec<String>,
    seen: HashSet<String>,
}

impl Warn {
    pub fn push(&mut self, msg: String) {
        self.list.push(msg);
    }
    /// 某个格式不支持某个字段：每个字段只报一次。
    pub fn unsupported(&mut self, format: &str, field: &str, id: &str) {
        let key = format!("{format}:{field}");
        if self.seen.insert(key) {
            self.list.push(format!(
                "{format} 表达不了样式 `{field}`（第一次出现在 `{id}`），已忽略"
            ));
        }
    }
}

// ---------- 列表 ----------

/// 一级列表的样式：元素样式叠上 `levelN` 部位；哪里都没写编号类型时，按层级轮换默认类型。
pub fn list_level_style(n: &Node, level: usize, ordered: bool) -> Style {
    let mut s = n.style.clone();
    s.merge(&n.part(&format!("level{level}")));
    if s.list_style_type.is_none() {
        s.list_style_type = Some(default_type(level, ordered));
    }
    s
}

pub fn default_type(level: usize, ordered: bool) -> ListStyleType {
    let i = (level - 1) % 3;
    if ordered {
        [
            ListStyleType::Decimal,
            ListStyleType::LowerAlpha,
            ListStyleType::LowerRoman,
        ][i]
            .clone()
    } else {
        [
            ListStyleType::Disc,
            ListStyleType::Circle,
            ListStyleType::Square,
        ][i]
            .clone()
    }
}

/// 列表展开成一行一项。
pub struct ListLine<'a> {
    pub level: usize,
    pub item: &'a ListItem,
    /// 编号或符号的文字（已按 listMarkerFormat 拼好）
    pub marker: String,
    pub style: Style,
}

/// 把嵌套列表摊平，算好每项的编号。
pub fn flatten_list<'a>(n: &Node, items: &'a [ListItem], ordered: bool) -> Vec<ListLine<'a>> {
    let mut out = Vec::new();
    let mut counters: Vec<u32> = Vec::new();
    let mut types: Vec<ListStyleType> = Vec::new();
    walk(n, items, ordered, 1, &mut counters, &mut types, &mut out);
    out
}

fn walk<'a>(
    n: &Node,
    items: &'a [ListItem],
    ordered: bool,
    level: usize,
    counters: &mut Vec<u32>,
    types: &mut Vec<ListStyleType>,
    out: &mut Vec<ListLine<'a>>,
) {
    let style = list_level_style(n, level, ordered);
    let ty = style.list_style_type.clone().unwrap();
    let start = style.list_start.unwrap_or(1);
    counters.truncate(level - 1);
    types.truncate(level - 1);
    counters.push(start.saturating_sub(1));
    types.push(ty.clone());
    for it in items {
        counters[level - 1] += 1;
        let marker = if let Some(b) = ty.bullet().filter(|_| !ty.ordered()) {
            match &style.list_marker_format {
                Some(f) => fill_format(f, counters, types, false),
                None => b,
            }
        } else {
            let f = style
                .list_marker_format
                .clone()
                .unwrap_or_else(|| format!("%{level}."));
            fill_format(&f, counters, types, legal(&ty))
        };
        out.push(ListLine {
            level,
            item: it,
            marker,
            style: style.clone(),
        });
        if !it.children().is_empty() {
            let sub_ordered = it.ordered().unwrap_or(ordered);
            walk(
                n,
                it.children(),
                sub_ordered,
                level + 1,
                counters,
                types,
                out,
            );
            counters.truncate(level);
            types.truncate(level);
        }
    }
}

/// 这一级用阿拉伯数字时，`%1.%2` 里上几级的编号也写成阿拉伯数字（Word 叫 isLgl），
/// 上一级是「一、」，这一级出来是「2.1」而不是「二.1」。
pub fn legal(ty: &ListStyleType) -> bool {
    matches!(
        ty,
        ListStyleType::Decimal | ListStyleType::DecimalLeadingZero
    )
}

/// `%1.%2` 里的 %k 换成第 k 级的编号。`legal` 为真时上几级一律用这一级的数字写法。
fn fill_format(f: &str, counters: &[u32], types: &[ListStyleType], legal: bool) -> String {
    let mut out = f.to_string();
    let own = types.last();
    for k in (1..=counters.len()).rev() {
        let pat = format!("%{k}");
        if out.contains(&pat) {
            let ty = match own {
                Some(t) if legal => t,
                _ => &types[k - 1],
            };
            out = out.replace(&pat, &ty.number(counters[k - 1]));
        }
    }
    out
}

/// Word lvlText 的写法：`%1.` 原样，符号直接写。
pub fn word_level_text(style: &Style, level: usize) -> String {
    let ty = style.list_style_type.clone().unwrap_or(ListStyleType::Disc);
    match (
        &style.list_marker_format,
        ty.bullet().filter(|_| !ty.ordered()),
    ) {
        (Some(f), _) => f.clone(),
        (None, Some(b)) => b,
        (None, None) => format!("%{level}."),
    }
}

// ---------- 表格 ----------

/// 网格上的一个位置：属于哪一格（左上角的行列号）。被合并掉的位置 origin 指向左上角那格。
#[derive(Clone, Copy)]
pub struct Slot<'a> {
    pub origin: (usize, usize),
    pub row_span: usize,
    pub col_span: usize,
    /// None 是补齐出来的空格
    pub cell: Option<&'a Cell>,
}

impl Slot<'_> {
    pub fn text(&self) -> &Inline {
        static EMPTY: Inline = Inline::Plain(String::new());
        self.cell.map(Cell::text).unwrap_or(&EMPTY)
    }
}

/// 表格展开成网格：合并单元格按 HTML 的规则摆（每行只写从这一行开始的格子，
/// 被上面 rowSpan 占掉的位置跳过），行数按写的行数，列数取最宽的一行。
pub struct Grid<'a> {
    pub rows: usize,
    pub cols: usize,
    pub has_header: bool,
    pub slots: Vec<Vec<Slot<'a>>>,
    /// (行号, 实际占的列数)：和列数对不上、补了空格的行，给渲染器报警告
    pub ragged: Vec<(usize, usize)>,
}

impl<'a> Grid<'a> {
    pub fn of(p: &'a TableProps) -> Grid<'a> {
        let has_header = !p.columns.is_empty();
        let src: Vec<&'a [Cell]> = std::iter::once(p.columns.as_slice())
            .filter(|_| has_header)
            .chain(p.rows.iter().map(Vec::as_slice))
            .collect();
        let rows = src.len();
        let mut grid: Vec<Vec<Option<Slot<'a>>>> = vec![Vec::new(); rows];
        for (r, cells) in src.iter().enumerate() {
            let mut c = 0;
            for cell in cells.iter() {
                while grid[r].get(c).is_some_and(Option::is_some) {
                    c += 1;
                }
                let rs = cell.row_span().min(rows - r);
                let cs = cell.col_span();
                let slot = Slot {
                    origin: (r, c),
                    row_span: rs,
                    col_span: cs,
                    cell: Some(cell),
                };
                for row in grid.iter_mut().skip(r).take(rs) {
                    if row.len() < c + cs {
                        row.resize(c + cs, None);
                    }
                    for x in row.iter_mut().skip(c).take(cs) {
                        // 两格重叠时先写的算数
                        if x.is_none() {
                            *x = Some(slot);
                        }
                    }
                }
                c += cs;
            }
        }
        let cols = grid.iter().map(Vec::len).max().unwrap_or(0).max(1);
        let mut ragged = Vec::new();
        let slots = grid
            .into_iter()
            .enumerate()
            .map(|(r, row)| {
                let used = row.iter().filter(|x| x.is_some()).count();
                if used != cols && rows > 0 {
                    ragged.push((r, used));
                }
                (0..cols)
                    .map(|c| {
                        row.get(c).copied().flatten().unwrap_or(Slot {
                            origin: (r, c),
                            row_span: 1,
                            col_span: 1,
                            cell: None,
                        })
                    })
                    .collect()
            })
            .collect();
        Grid {
            rows,
            cols,
            has_header,
            slots,
            ragged,
        }
    }

    pub fn slot(&self, r: usize, c: usize) -> &Slot<'a> {
        &self.slots[r][c]
    }

    /// 这个位置是不是一格的左上角（要输出的格子）
    pub fn is_origin(&self, r: usize, c: usize) -> bool {
        self.slots[r][c].origin == (r, c)
    }

    /// 补齐的行报警告
    pub fn warn_ragged(&self, warn: &mut Warn, id: &str) {
        for (r, used) in &self.ragged {
            warn.push(format!(
                "表格 `{id}` 第 {} 行占 {used} 列，按 {} 列补齐",
                r + 1,
                self.cols
            ));
        }
    }

    /// 第 r 行的行高
    pub fn row_height(&self, p: &TableProps, r: usize) -> Option<RowHeight> {
        p.row_heights
            .as_ref()
            .and_then(|v| v.get(r).copied().flatten())
    }
}

/// 合并后的一格四条边：上、左取左上角，下取最后一行，右取最后一列。返回 上右下左。
pub fn span_edges(n: &Node, look: &TableLook, g: &Grid, r: usize, c: usize) -> [Option<Border>; 4] {
    let sl = g.slot(r, c);
    let (r1, c1) = (r + sl.row_span - 1, c + sl.col_span - 1);
    let first = cell_edges(n, look, g, r, c);
    if (r1, c1) == (r, c) {
        return first;
    }
    [
        first[0].clone(),
        cell_edges(n, look, g, r, c1)[1].clone(),
        cell_edges(n, look, g, r1, c)[2].clone(),
        first[3].clone(),
    ]
}

/// 各列宽度；没写或个数不对时平均分（个数不对会报警告）。
pub fn col_widths(p: &TableProps, g: &Grid, warn: &mut Warn, id: &str) -> Vec<ColWidth> {
    match &p.widths {
        Some(ws) if ws.len() == g.cols => ws.clone(),
        Some(_) => {
            warn.push(format!(
                "表格 `{id}` 的 widths 个数和列数对不上，改成平均分"
            ));
            vec![ColWidth::Ratio(1.0); g.cols]
        }
        None => vec![ColWidth::Ratio(1.0); g.cols],
    }
}

/// 有比例列时表格撑满（或撑到 style.width）；全是固定列和 auto 列时表格按内容收窄。
pub fn fills_width(ws: &[ColWidth]) -> bool {
    ws.iter().any(|w| matches!(w, ColWidth::Ratio(_)))
}

/// auto 列的自然宽度（pt）：按最宽的格子粗估，加左右 padding。非 auto 列是 0。
pub fn auto_natural(n: &Node, p: &TableProps, g: &Grid, ws: &[ColWidth], pad_pt: f32) -> Vec<f32> {
    let s = &n.style;
    (0..g.cols)
        .map(|c| {
            if ws[c] != ColWidth::Auto {
                return 0.0;
            }
            (0..g.rows)
                .filter(|r| g.is_origin(*r, c) && g.slot(*r, c).col_span == 1)
                .map(|r| {
                    let part = cell_parts(n, &p.look, g, r, c);
                    let mut cs = s.inherited();
                    cs.merge(&part);
                    let side = |e| part.padding_side(e).unwrap_or(pad_pt);
                    estimate_text_width(g.slot(r, c).text(), &cs)
                        + side(Edge::Left)
                        + side(Edge::Right)
                        + 3.0
                })
                .fold(0.0, f32::max)
        })
        .collect()
}

/// 比例列和 auto 列混写时，auto 列按内容撑开，但最多占一份 1fr：
/// 把每个 auto 列当成 1fr 算出一份的宽度，内容比这窄就用内容宽（变成固定列），
/// 比这宽就变成 1fr。这样比例列不会被长内容挤没。没有混写时原样返回。
/// `avail` 是表格可用总宽（pt）。三种格式用同一个结果，排出来的列宽一致。
pub fn resolve_auto(ws: &[ColWidth], natural: &[f32], avail: f32) -> Vec<ColWidth> {
    let ratio: f32 = ws
        .iter()
        .map(|w| if let ColWidth::Ratio(x) = w { *x } else { 0.0 })
        .sum();
    let autos = ws.iter().filter(|w| **w == ColWidth::Auto).count();
    if ratio <= 0.0 || autos == 0 {
        return ws.to_vec();
    }
    let fixed: f32 = ws
        .iter()
        .map(|w| if let ColWidth::Fixed(x) = w { *x } else { 0.0 })
        .sum();
    let share = ((avail - fixed) / (ratio + autos as f32)).max(0.0);
    ws.iter()
        .zip(natural)
        .map(|(w, a)| match w {
            ColWidth::Auto if *a < share => ColWidth::Fixed(*a),
            ColWidth::Auto => ColWidth::Ratio(1.0),
            w => w.clone(),
        })
        .collect()
}

/// 粗估一段文字排成一行要多宽（pt），给 docx 的 auto 列算初始列宽用：
/// 全角字算 1em，半角算 0.6em（宁宽勿窄，窄了 Word 会折行），再加字间距。有换行时取最长的一行。
pub fn estimate_text_width(t: &Inline, cs: &Style) -> f32 {
    let size = cs.size_pt();
    let spacing = cs.letter_spacing.map(|l| l.pt(size, 0.0)).unwrap_or(0.0);
    t.plain_text()
        .split('\n')
        .map(|line| {
            line.chars()
                .map(|c| if (c as u32) < 0x2E80 { 0.6 * size } else { size } + spacing)
                .sum::<f32>()
        })
        .fold(0.0, f32::max)
}

/// 第 r 行第 c 列（都从 0 起，表头算第 0 行）的部位样式，按 Word 的顺序叠：
/// table → cell → 奇偶列 → 奇偶行 → 首末列 → 首末行 → 四角。
pub fn cell_parts(n: &Node, look: &TableLook, g: &Grid, r: usize, c: usize) -> Style {
    let band = n.style.band_size.unwrap_or(1).max(1) as usize;
    let first_row = r == 0 && (look.first_row || g.has_header);
    let last_row = look.last_row && r + 1 == g.rows && g.rows > 1;
    let first_col = look.first_column && c == 0;
    let last_col = look.last_column && c + 1 == g.cols && g.cols > 1;
    let mut s = Style::default();
    s.merge(&n.part("table"));
    s.merge(&n.part("cell"));
    if look.banded_columns && !first_col && !last_col {
        let i = c - usize::from(look.first_column);
        s.merge(&n.part(if (i / band).is_multiple_of(2) {
            "oddColumn"
        } else {
            "evenColumn"
        }));
    }
    if look.banded_rows && !first_row && !last_row {
        let i = r - usize::from(g.has_header || look.first_row);
        s.merge(&n.part(if (i / band).is_multiple_of(2) {
            "oddRow"
        } else {
            "evenRow"
        }));
    }
    if first_col {
        s.merge(&n.part("firstColumn"));
    }
    if last_col {
        s.merge(&n.part("lastColumn"));
    }
    if first_row {
        s.merge(&n.part("firstRow"));
        if g.has_header {
            s.merge(&n.part("header"));
        }
    }
    if last_row {
        s.merge(&n.part("lastRow"));
    }
    let top = r == 0;
    let bottom = r + 1 == g.rows;
    let left = c == 0;
    let right = c + 1 == g.cols;
    for (on, k) in [
        (top && left, "topLeftCell"),
        (top && right, "topRightCell"),
        (bottom && left, "bottomLeftCell"),
        (bottom && right, "bottomRightCell"),
    ] {
        if on {
            s.merge(&n.part(k));
        }
    }
    // 单格自己写的样式最后叠；合并掉的位置用左上角那格的
    if let Some(cs) = g
        .slots
        .get(r)
        .and_then(|x| x.get(c))
        .and_then(|x| x.cell)
        .and_then(Cell::style)
    {
        s.merge(cs);
    }
    s
}

/// 一格四条边，连同相邻格子写的边一起考虑：下面一格写了 borderTop，这一格的下边也用它，
/// 免得两条线叠在一起。返回 上右下左。
pub fn cell_edges(n: &Node, look: &TableLook, g: &Grid, r: usize, c: usize) -> [Option<Border>; 4] {
    let part = cell_parts(n, look, g, r, c);
    let mut b = cell_borders(&n.style, &part, g, r, c);
    // (这一格的边, 邻格位置, 邻格的哪条边)
    let neighbours = [
        (0, r.checked_sub(1).map(|r| (r, c)), Edge::Bottom),
        (1, (c + 1 < g.cols).then_some((r, c + 1)), Edge::Left),
        (2, (r + 1 < g.rows).then_some((r + 1, c)), Edge::Top),
        (3, c.checked_sub(1).map(|c| (r, c)), Edge::Right),
    ];
    for (i, at, theirs) in neighbours {
        let Some((nr, nc)) = at else { continue };
        if part.border_side(Edge::ALL[i]).is_some() {
            continue;
        }
        if let Some(t) = cell_parts(n, look, g, nr, nc).border_side(theirs) {
            b[i] = Some(t.clone());
        }
    }
    b
}

/// 一格四条边：表格的外框和内线，再被格子自己的边框覆盖。返回 上右下左。
pub fn cell_borders(
    table: &Style,
    cell: &Style,
    g: &Grid,
    r: usize,
    c: usize,
) -> [Option<Border>; 4] {
    let outer = |e: Edge| table.border_side(e).cloned();
    let inside_h = table
        .border_inside_h
        .clone()
        .or_else(|| table.border.clone());
    let inside_v = table
        .border_inside_v
        .clone()
        .or_else(|| table.border.clone());
    let base = [
        if r == 0 {
            outer(Edge::Top)
        } else {
            inside_h.clone()
        },
        if c + 1 == g.cols {
            outer(Edge::Right)
        } else {
            inside_v.clone()
        },
        if r + 1 == g.rows {
            outer(Edge::Bottom)
        } else {
            inside_h
        },
        if c == 0 { outer(Edge::Left) } else { inside_v },
    ];
    let mut out = base;
    for (i, e) in Edge::ALL.iter().enumerate() {
        if let Some(b) = cell.border_side(*e) {
            out[i] = Some(b.clone());
        }
    }
    out
}

/// 把文字放进另一个样式环境（表格格子、列表项）：每个 run 的样式按新环境重算。
pub fn rebase_runs(t: &Inline, old: &Style, new: &Style) -> Vec<Run> {
    t.runs()
        .map(|r| {
            let mut r = r.clone();
            r.computed = r.computed.rebase(old, new);
            r
        })
        .collect()
}

/// 目录里的一项。
pub struct TocEntry {
    pub level: u8,
    pub text: String,
    pub id: String,
}

/// 按文档顺序收 1 到 depth 级标题，给 docx、html 的目录用（PDF 交给 typst 的 outline）。
pub fn toc_entries(root: &Node, depth: u8) -> Vec<TocEntry> {
    fn walk(n: &Node, depth: u8, out: &mut Vec<TocEntry>) {
        if let crate::components::Component::Heading(h) = &n.component
            && h.level <= depth
        {
            out.push(TocEntry {
                level: h.level,
                text: h.text.plain_text(),
                id: n.id.clone(),
            });
        }
        for c in &n.children {
            walk(c, depth, out);
        }
    }
    let mut out = Vec::new();
    walk(root, depth, &mut out);
    out
}

/// 目录标题的样式：目录的可继承字段加 title 部位。
pub fn toc_title_style(n: &Node) -> Style {
    let mut s = n.style.inherited();
    s.merge(&n.part("title"));
    s
}

/// 目录项的样式：目录的可继承字段加 entry 部位，再按级缩进 listIndent × (级数 - 1)。
pub fn toc_entry_style(n: &Node, level: u8) -> Style {
    let mut s = n.style.inherited();
    s.merge(&n.part("entry"));
    let fs = s.size_pt();
    let step = s.list_indent.map(|x| x.pt(fs, 0.0)).unwrap_or(fs * 1.5);
    let base = s.margin_left.map(|x| x.pt(fs, 0.0)).unwrap_or(0.0);
    s.margin_left = Some(crate::style::Len::Pt(base + step * (level as f32 - 1.0)));
    s.list_indent = None;
    s
}

// ---------- 图案边框 ----------

/// CSS 和 Typst 画不出来的线型，用一小块 SVG 图案沿边铺开。HTML 当背景贴，PDF 当 tiling 填充，
/// 两边用的是同一份 SVG，长得一样。
///
/// 图案按「横边」画：x 沿着边，y 是线宽方向（0 到 `width`）。竖边把 x、y 对调。
/// y = 0 一律在上边或左边，不随边翻转，这样相邻两格共用的线叠在一起也是同一个图案。
pub struct Tile {
    /// 图案沿边方向的长度，pt
    pub period: f32,
    /// 线宽，pt
    pub width: f32,
    body: String,
    /// 沿边不断的线（双线、三线、粗细线、立体线）：每条是 (离上边或左边的距离, 粗细, 颜色)。
    /// PDF 按这个拆成几条实线画（块套几层 stroke，表格格子用硬边渐变），免得 tiling 拼缝；虚线、波浪线这里是空的。
    pub stripes: Vec<(f32, f32, String)>,
}

impl Tile {
    /// 铺这条边用的 SVG。`vertical` 为真时是左右两条竖边。
    pub fn svg(&self, vertical: bool) -> String {
        let (p, w) = (self.period, self.width);
        let (sw, sh, g) = if vertical {
            (w, p, "<g transform='matrix(0 1 1 0 0 0)'>")
        } else {
            (p, w, "<g>")
        };
        format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='{sw:.3}' height='{sh:.3}' viewBox='0 0 {sw:.3} {sh:.3}' preserveAspectRatio='none'>{g}{}</g></svg>",
            self.body
        )
    }

    /// 和 `svg` 一样，只是图案在线宽方向上挪半个线宽、首尾接上：给 Typst 的 stroke 当 paint 用，
    /// stroke 的图案以线的中线为原点。
    pub fn svg_centered(&self, vertical: bool) -> String {
        let (p, w) = (self.period, self.width);
        let h = w / 2.0;
        let (sw, sh) = if vertical { (w, p) } else { (p, w) };
        let g = |dy: f32| {
            if vertical {
                format!("<g transform='matrix(0 1 1 0 {dy:.3} 0)'>{}</g>", self.body)
            } else {
                format!("<g transform='translate(0 {dy:.3})'>{}</g>", self.body)
            }
        };
        format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='{sw:.3}' height='{sh:.3}' viewBox='0 0 {sw:.3} {sh:.3}' preserveAspectRatio='none'>{}{}</svg>",
            g(h),
            g(-h)
        )
    }

    /// 当 CSS url() 用的 data URI。
    pub fn data_uri(&self, vertical: bool) -> String {
        let mut o = String::from("data:image/svg+xml,");
        for ch in self.svg(vertical).chars() {
            match ch {
                '%' => o.push_str("%25"),
                '#' => o.push_str("%23"),
                '<' => o.push_str("%3C"),
                '>' => o.push_str("%3E"),
                '"' => o.push_str("%22"),
                c => o.push(c),
            }
        }
        o
    }
}

/// 这条边要不要用图案画；实线、虚线、点线不用（两边都能原生画）。
/// HTML 另外还能原生画 double 和四种立体线，调用前自己挑掉。
pub fn border_tile(b: &Border, side: Edge) -> Option<Tile> {
    use crate::style::BorderStyle as S;
    if b.is_none() {
        return None;
    }
    let w = b.width;
    let c = b.color.css();
    let rect = |x: f32, y: f32, rw: f32, rh: f32, fill: &str| {
        format!("<rect x='{x:.3}' y='{y:.3}' width='{rw:.3}' height='{rh:.3}' fill='{fill}'/>")
    };
    // 横条纹：单位长度依次是 线、空、线……
    let stripes = |units: &[f32]| {
        let k = w / units.iter().sum::<f32>();
        let (mut y, mut o, mut st) = (0.0, String::new(), Vec::new());
        for (i, u) in units.iter().enumerate() {
            if i % 2 == 0 {
                o.push_str(&rect(0.0, y, 4.0, u * k, &c));
                st.push((y, u * k, c.clone()));
            }
            y += u * k;
        }
        Tile {
            period: 4.0,
            width: w,
            body: o,
            stripes: st,
        }
    };
    // 沿边的虚线段：长度依次是 画、空、画……
    let u = w.max(0.75);
    let dashes = |segs: &[f32]| {
        let (mut x, mut o) = (0.0, String::new());
        for (i, s) in segs.iter().enumerate() {
            if i % 2 == 0 {
                o.push_str(&rect(x, 0.0, s * u, w, &c));
            }
            x += s * u;
        }
        Tile {
            period: x,
            width: w,
            body: o,
            stripes: Vec::new(),
        }
    };
    let wave = |y0: f32, h: f32, p: f32| {
        let sw = h / 3.0;
        let mid = y0 + h / 2.0;
        format!(
            "<path d='M0 {mid:.3} Q{:.3} {:.3} {:.3} {mid:.3} T{p:.3} {mid:.3}' fill='none' stroke='{c}' stroke-width='{sw:.3}'/>",
            p / 4.0,
            // 二次曲线最高点只走到控制点的一半，控制点要放到两倍振幅处
            mid - (h - sw),
            p / 2.0
        )
    };
    let light = mix_white(&b.color, 0.6);
    let halves = |a: &str, z: &str| Tile {
        period: 4.0,
        width: w,
        body: format!(
            "{}{}",
            rect(0.0, 0.0, 4.0, w / 2.0, a),
            rect(0.0, w / 2.0, 4.0, w / 2.0, z)
        ),
        stripes: vec![
            (0.0, w / 2.0, a.to_string()),
            (w / 2.0, w / 2.0, z.to_string()),
        ],
    };
    let top_left = matches!(side, Edge::Top | Edge::Left);
    Some(match b.style {
        S::None | S::Solid | S::Thick | S::Dashed | S::Dotted => return None,
        S::Double => stripes(&[1.0, 1.0, 1.0]),
        S::Triple => stripes(&[1.0, 1.0, 1.0, 1.0, 1.0]),
        S::ThinThickSmallGap => stripes(&[1.0, 1.0, 3.0]),
        S::ThickThinSmallGap => stripes(&[3.0, 1.0, 1.0]),
        S::ThinThickThinSmallGap => stripes(&[1.0, 1.0, 3.0, 1.0, 1.0]),
        S::ThinThickMediumGap => stripes(&[1.0, 2.0, 3.0]),
        S::ThickThinMediumGap => stripes(&[3.0, 2.0, 1.0]),
        S::ThinThickThinMediumGap => stripes(&[1.0, 2.0, 3.0, 2.0, 1.0]),
        S::ThinThickLargeGap => stripes(&[1.0, 4.0, 3.0]),
        S::ThickThinLargeGap => stripes(&[3.0, 4.0, 1.0]),
        S::ThinThickThinLargeGap => stripes(&[1.0, 4.0, 3.0, 4.0, 1.0]),
        S::DashDot => dashes(&[3.0, 1.0, 1.0, 1.0]),
        S::DashDotDot => dashes(&[3.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
        S::DashSmallGap => dashes(&[3.0, 0.6]),
        S::DashDotStroked => {
            // 斜着的划和点：/// 样子
            let s = w / 2.0;
            let poly = |x: f32, len: f32| {
                format!(
                    "<polygon points='{:.3},{w:.3} {:.3},{w:.3} {:.3},0 {:.3},0' fill='{c}'/>",
                    x,
                    x + len,
                    x + len + s,
                    x + s
                )
            };
            Tile {
                period: 6.0 * u,
                width: w,
                body: format!("{}{}", poly(0.0, 3.0 * u), poly(4.0 * u, u)),
                stripes: Vec::new(),
            }
        }
        S::Wave => {
            let p = (w * 4.0).max(4.0);
            Tile {
                period: p,
                width: w,
                body: wave(0.0, w, p),
                stripes: Vec::new(),
            }
        }
        S::DoubleWave => {
            let p = (w * 3.0).max(4.0);
            Tile {
                period: p,
                width: w,
                body: format!("{}{}", wave(0.0, w / 2.0, p), wave(w / 2.0, w / 2.0, p)),
                stripes: Vec::new(),
            }
        }
        // 立体线：凸起是外亮内暗，凹陷反过来；y = 0 不随边翻转，所以四边都是同一个上下顺序
        S::ThreeDEmboss => halves(&light, &c),
        S::ThreeDEngrave => halves(&c, &light),
        S::Inset | S::Outset => {
            let dark = (b.style == S::Inset) == top_left;
            let fill = if dark { c.clone() } else { light.clone() };
            Tile {
                period: 4.0,
                width: w,
                body: rect(0.0, 0.0, 4.0, w, &fill),
                stripes: vec![(0.0, w, fill)],
            }
        }
    })
}

/// 颜色往白色里混 `t`（0–1）。
fn mix_white(c: &crate::style::Color, t: f32) -> String {
    let h = c.rgb();
    let ch = |i: usize| {
        let v = u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f32;
        (v + (255.0 - v) * t).round() as u8
    };
    format!("#{:02x}{:02x}{:02x}", ch(0), ch(2), ch(4))
}
