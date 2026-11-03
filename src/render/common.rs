//! 三个渲染器共用的计算：列表编号、表格部位、警告去重。

use std::collections::HashSet;

use crate::{
    components::{Inline, ListItem, Run, TableLook, TableProps},
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

/// 表格展开后的一格：行列号和叠好的部位样式。
pub struct Grid {
    pub rows: usize,
    pub cols: usize,
    pub has_header: bool,
}

impl Grid {
    pub fn of(p: &TableProps) -> Grid {
        let cols = p
            .columns
            .len()
            .max(p.rows.iter().map(Vec::len).max().unwrap_or(0))
            .max(1);
        let has_header = !p.columns.is_empty();
        Grid {
            rows: p.rows.len() + usize::from(has_header),
            cols,
            has_header,
        }
    }
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

/// 粗估一段文字排成一行要多宽（pt），给 docx 的 auto 列算初始列宽用：
/// 全角字算 1em，半角算 0.55em，再加字间距。有换行时取最长的一行。
pub fn estimate_text_width(t: &Inline, cs: &Style) -> f32 {
    let size = cs.size_pt();
    let spacing = cs.letter_spacing.map(|l| l.pt(size, 0.0)).unwrap_or(0.0);
    t.plain_text()
        .split('\n')
        .map(|line| {
            line.chars()
                .map(|c| if (c as u32) < 0x2E80 { 0.55 * size } else { size } + spacing)
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
