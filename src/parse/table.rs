//! 表格：w:tbl → Table。合并格（gridSpan、vMerge）、列宽、行高、单格样式、边框。

use roxmltree::Node;
use serde_json::{Map, Value, json};

use super::word::{self, TblPr, child, isw, margins, round2, wa, wval};
use super::{Ctx, PARA_KEYS, Para, Seg, TEXT_KEYS, compact, diff, from_map, hoist, merged, same, size_of, text_of, to_map};
use crate::style::{Border, Style};

/// 一个 w:tc。
struct TCell {
    r: usize,
    c: usize,
    span: usize,
    /// Some(true) 是 vMerge restart，Some(false) 是 continue
    vmerge: Option<bool>,
    auto_w: bool,
    /// 上右下左
    borders: [Option<Border>; 4],
    shd: Option<crate::style::Background>,
    valign: Option<&'static str>,
    rotation: Option<u16>,
    mar: [Option<f32>; 4],
    paras: Vec<Para>,
}

struct TRow {
    height: Option<(f32, bool)>,
    header: bool,
    cant_split: bool,
}

/// 行、格子可能包在 sdt、customXml 里，摊平。
fn flat<'a, 'i>(n: Node<'a, 'i>, name: &str, out: &mut Vec<Node<'a, 'i>>) {
    for c in n.children().filter(|c| c.is_element()) {
        if isw(c, name) {
            out.push(c);
        } else if isw(c, "sdt") {
            if let Some(sc) = child(c, "sdtContent") {
                flat(sc, name, out);
            }
        } else if isw(c, "customXml") {
            flat(c, name, out);
        }
    }
}

fn bval(b: &Option<Border>) -> Value {
    let b = b.clone().unwrap_or_else(Border::none);
    serde_json::to_value(b).unwrap_or(json!("none"))
}

impl Ctx {
    pub(super) fn table(&mut self, tbl: Node) -> String {
        let tblpr = child(tbl, "tblPr");
        let style_id = tblpr
            .and_then(|p| child(p, "tblStyle"))
            .and_then(wval)
            .map(str::to_string)
            .or_else(|| self.styles.default_table.clone());
        let (mut tp, tppr, trpr, cond) = style_id
            .as_deref()
            .map(|s| self.styles.table(s))
            .unwrap_or_default();
        if cond {
            self.warns.once(
                "tblStylePr",
                "表格样式里的条件格式（首行、隔行这些）没解析，只用了整表的格式".into(),
            );
        }
        if let Some(p) = tblpr {
            tp.merge(&TblPr::parse(p, &mut self.warns));
        }
        let tblw = tblpr.and_then(|p| child(p, "tblW")).map(|w| {
            (
                wa(w, "w").unwrap_or("0").trim_end_matches('%').parse::<f32>().unwrap_or(0.0),
                wa(w, "type").unwrap_or("dxa").to_string(),
                wa(w, "w").is_some_and(|x| x.ends_with('%')),
            )
        });
        let layout = tblpr
            .and_then(|p| child(p, "tblLayout"))
            .and_then(|l| wa(l, "type"))
            .map(str::to_string);
        if let Some(j) = tblpr.and_then(|p| child(p, "jc")).and_then(wval)
            && matches!(j, "center" | "right" | "end")
        {
            self.warns.once(
                "tbljc",
                "表格整体居中、靠右没保留（fmtagno 的表格没有对齐字段）".into(),
            );
        }
        let grid: Vec<f32> = child(tbl, "tblGrid")
            .map(|g| {
                g.children()
                    .filter(|c| isw(*c, "gridCol"))
                    .map(|c| wa(c, "w").and_then(|w| w.parse::<f32>().ok()).unwrap_or(0.0) / 20.0)
                    .collect()
            })
            .unwrap_or_default();

        // 读行和格子
        let saved = self.tbl_base.replace((tppr, trpr));
        self.in_table += 1;
        let mut rows_n = Vec::new();
        flat(tbl, "tr", &mut rows_n);
        let mut rows: Vec<TRow> = Vec::new();
        let mut cells: Vec<TCell> = Vec::new();
        let mut ncols = grid.len();
        for (r, tr) in rows_n.iter().enumerate() {
            let trpr = child(*tr, "trPr");
            let num = |n: Option<Node>, a: &str| n.and_then(|x| wa(x, a)).and_then(|v| v.parse::<f32>().ok());
            let height = trpr.and_then(|p| child(p, "trHeight")).and_then(|h| {
                let v = num(Some(h), "val")?;
                Some((round2(v / 20.0), wa(h, "hRule") == Some("exact")))
            });
            let before = num(trpr.and_then(|p| child(p, "gridBefore")), "val").unwrap_or(0.0) as usize;
            if before > 0 || trpr.and_then(|p| child(p, "gridAfter")).is_some() {
                self.warns.once(
                    "gridBefore",
                    "表格行前后空着的格子（gridBefore、gridAfter）补成了空格子".into(),
                );
            }
            rows.push(TRow {
                height,
                header: trpr.and_then(|p| child(p, "tblHeader")).is_some_and(word::on_off),
                cant_split: trpr.and_then(|p| child(p, "cantSplit")).is_some_and(word::on_off),
            });
            let mut c = 0;
            for _ in 0..before {
                cells.push(TCell {
                    r,
                    c,
                    span: 1,
                    vmerge: None,
                    auto_w: false,
                    borders: [None, None, None, None],
                    shd: None,
                    valign: None,
                    rotation: None,
                    mar: [None; 4],
                    paras: Vec::new(),
                });
                c += 1;
            }
            let mut tcs = Vec::new();
            flat(*tr, "tc", &mut tcs);
            for tc in tcs {
                let pr = child(tc, "tcPr");
                let span = pr
                    .and_then(|p| child(p, "gridSpan"))
                    .and_then(|g| wval(g))
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                let vmerge = pr
                    .and_then(|p| child(p, "vMerge"))
                    .map(|v| wval(v) == Some("restart"));
                let auto_w = pr
                    .and_then(|p| child(p, "tcW"))
                    .is_some_and(|w| wa(w, "type") == Some("auto"));
                let mut borders: [Option<Border>; 4] = [None, None, None, None];
                if let Some(b) = pr.and_then(|p| child(p, "tcBorders")) {
                    for e in b.children().filter(|e| e.is_element()) {
                        let i = match e.tag_name().name() {
                            "top" => 0,
                            "right" | "end" => 1,
                            "bottom" => 2,
                            "left" | "start" => 3,
                            "tl2br" | "tr2bl" => {
                                self.warns.once(
                                    "diag",
                                    "格子里的斜线没保留".into(),
                                );
                                continue;
                            }
                            _ => continue,
                        };
                        borders[i] = Some(word::border_of(e, &mut self.warns).0);
                    }
                }
                let shd = pr.and_then(|p| child(p, "shd")).and_then(word::shading);
                let valign = pr.and_then(|p| child(p, "vAlign")).and_then(wval).and_then(|v| match v {
                    "center" => Some("middle"),
                    "bottom" => Some("bottom"),
                    "top" => Some("top"),
                    _ => None,
                });
                let rotation = pr
                    .and_then(|p| child(p, "textDirection"))
                    .and_then(wval)
                    .and_then(|v| match v {
                        "tbRl" | "tbRlV" => Some(90),
                        "btLr" => Some(270),
                        _ => None,
                    });
                let mar = pr.and_then(|p| child(p, "tcMar")).map(margins).unwrap_or([None; 4]);
                let mut paras = Vec::new();
                for k in tc.children().filter(|k| k.is_element()) {
                    if isw(k, "p") {
                        paras.push(self.para(k));
                    } else if isw(k, "tbl") || isw(k, "sdt") || isw(k, "customXml") {
                        if isw(k, "tbl") {
                            self.warns.once(
                                "nested",
                                "表格里套的表格没保留结构，文字按段落接在格子里".into(),
                            );
                        }
                        for p in k.descendants().filter(|d| isw(*d, "p")) {
                            paras.push(self.para(p));
                        }
                    }
                }
                cells.push(TCell {
                    r,
                    c,
                    span,
                    vmerge,
                    auto_w,
                    borders,
                    shd,
                    valign,
                    rotation,
                    mar,
                    paras,
                });
                c += span;
            }
            ncols = ncols.max(c);
        }
        self.tbl_base = saved;
        self.in_table -= 1;
        let nrows = rows.len();
        if nrows == 0 || ncols == 0 {
            let id = self.id("p");
            self.add(&id, json!({ "type": "Text", "props": { "text": "" } }));
            return id;
        }

        // 每个网格位置属于哪个 w:tc
        let mut at: Vec<Vec<Option<usize>>> = vec![vec![None; ncols]; nrows];
        for (i, cl) in cells.iter().enumerate() {
            for x in cl.c..(cl.c + cl.span).min(ncols) {
                at[cl.r][x] = Some(i);
            }
        }
        // vMerge：continue 的格子归到上面的起始格
        let mut origin: Vec<usize> = (0..cells.len()).collect();
        let mut row_span: Vec<usize> = vec![1; cells.len()];
        for i in 0..cells.len() {
            if cells[i].vmerge == Some(false) && cells[i].r > 0 {
                if let Some(up) = at[cells[i].r - 1][cells[i].c]
                    && cells[up].vmerge.is_some()
                {
                    origin[i] = origin[up];
                    row_span[origin[i]] += 1;
                }
            }
        }

        // Word 实际画出来的四条边：格子自己写的 → 邻格写的 → 表格的外框、内线
        let own = |i: usize, k: usize| cells[i].borders[k].clone();
        let mut edges: Vec<[Option<Border>; 4]> = Vec::with_capacity(cells.len());
        for (i, cl) in cells.iter().enumerate() {
            let last_col = cl.c + cl.span >= ncols;
            let top = own(i, 0)
                .or_else(|| (cl.r > 0).then(|| at[cl.r - 1][cl.c].and_then(|u| own(u, 2))).flatten())
                .or_else(|| if cl.r == 0 { tp.borders[0].clone() } else { tp.borders[4].clone() });
            let bottom = own(i, 2)
                .or_else(|| (cl.r + 1 < nrows).then(|| at[cl.r + 1][cl.c].and_then(|d| own(d, 0))).flatten())
                .or_else(|| if cl.r + 1 == nrows { tp.borders[2].clone() } else { tp.borders[4].clone() });
            let left = own(i, 3)
                .or_else(|| (cl.c > 0).then(|| at[cl.r][cl.c - 1].and_then(|l| own(l, 1))).flatten())
                .or_else(|| if cl.c == 0 { tp.borders[3].clone() } else { tp.borders[5].clone() });
            let right = own(i, 1)
                .or_else(|| (!last_col).then(|| at[cl.r][cl.c + cl.span].and_then(|x| own(x, 3))).flatten())
                .or_else(|| if last_col { tp.borders[1].clone() } else { tp.borders[5].clone() });
            edges.push([top, right, bottom, left]);
        }
        // 合并块的边：上、左、右取起始格，下取最后一行
        let blocks: Vec<usize> = (0..cells.len()).filter(|i| origin[*i] == *i).collect();
        let block_edges = |b: usize| -> [Value; 4] {
            let cl = &cells[b];
            let last_r = cl.r + row_span[b] - 1;
            let bottom = at[last_r][cl.c].map(|x| edges[x][2].clone()).unwrap_or(None);
            [bval(&edges[b][0]), bval(&edges[b][1]), bval(&bottom), bval(&edges[b][3])]
        };
        // 各位置最常见的线当表格的边框
        let mut pools: [Vec<Value>; 6] = Default::default();
        for &b in &blocks {
            let cl = &cells[b];
            let e = block_edges(b);
            let last_r = cl.r + row_span[b] >= nrows;
            let last_c = cl.c + cl.span >= ncols;
            pools[if cl.r == 0 { 0 } else { 4 }].push(e[0].clone());
            pools[if last_c { 1 } else { 5 }].push(e[1].clone());
            pools[if last_r { 2 } else { 4 }].push(e[2].clone());
            pools[if cl.c == 0 { 3 } else { 5 }].push(e[3].clone());
        }
        // 同样多时取全表最常见的那种线
        let everything: Vec<Value> = pools.iter().flatten().cloned().collect();
        let common = |v: &Vec<Value>| -> Option<Value> {
            let mut best: Option<(&Value, usize, usize)> = None;
            for x in v {
                let n = v.iter().filter(|y| same(x, y)).count();
                let g = everything.iter().filter(|y| same(x, y)).count();
                if best.is_none_or(|(_, m, h)| n > m || (n == m && g > h)) {
                    best = Some((x, n, g));
                }
            }
            best.map(|(x, _, _)| x.clone())
        };
        let base: Vec<Value> = pools
            .iter()
            .map(|p| common(p).unwrap_or(json!("none")))
            .collect();
        let mut tstyle = Map::new();
        let names = [
            "borderTop",
            "borderRight",
            "borderBottom",
            "borderLeft",
            "borderInsideH",
            "borderInsideV",
        ];
        if base.iter().all(|b| same(b, &base[0])) {
            tstyle.insert("border".into(), base[0].clone());
        } else {
            for (k, v) in names.iter().zip(&base) {
                tstyle.insert(k.to_string(), v.clone());
            }
        }

        // 格子边距
        let mar_tw = |i: usize, d: f32| tp.cell_mar[i].unwrap_or(d);
        let pad = [mar_tw(0, 0.0), mar_tw(1, 108.0), mar_tw(2, 0.0), mar_tw(3, 108.0)].map(|x| round2(x / 20.0));
        tstyle.insert("cellPadding".into(), json!(pad[3]));
        let mut cell_part = Map::new();
        for (i, k) in ["paddingTop", "paddingRight", "paddingBottom"].iter().enumerate() {
            if (pad[i] - pad[3]).abs() > 0.01 {
                cell_part.insert(k.to_string(), json!(pad[i]));
            }
        }
        tstyle.insert("spaceAfter".into(), json!(0));
        if !rows.is_empty() && rows.iter().all(|r| r.cant_split) {
            tstyle.insert("rowBreak".into(), json!(false));
        }

        // 每块的文字和段落字段
        let block_info: Vec<(Map<String, Value>, Vec<String>, Vec<Seg>)> = blocks
            .iter()
            .map(|&b| {
                let paras: Vec<&Para> = (0..cells.len())
                    .filter(|i| origin[*i] == b)
                    .flat_map(|i| cells[i].paras.iter())
                    .collect();
                let mut segs: Vec<Seg> = Vec::new();
                for (k, p) in paras.iter().enumerate() {
                    if k > 0 {
                        match segs.last_mut() {
                            Some(Seg::Text { text, .. }) => text.push('\n'),
                            _ => segs.push(Seg::Text {
                                text: "\n".into(),
                                style: text_of(&p.run_base),
                                href: None,
                                code: false,
                                cstyle: None,
                            }),
                        }
                    }
                    for s in &p.segs {
                        match s {
                            Seg::PageBreak => {}
                            Seg::Math { latex, style, jc, .. } => segs.push(Seg::Math {
                                latex: latex.clone(),
                                display: false,
                                jc: jc.clone(),
                                style: style.clone(),
                            }),
                            x => segs.push(x.clone()),
                        }
                    }
                }
                let (eff, hoisted) = match paras.first() {
                    Some(p) => {
                        let mut fake = (*p).clone();
                        fake.ppr.after = paras.last().and_then(|l| l.ppr.after);
                        fake.ppr.after_lines = paras.last().and_then(|l| l.ppr.after_lines);
                        self.para_eff(&fake, &segs)
                    }
                    None => (Map::new(), Vec::new()),
                };
                (eff, hoisted, segs)
            })
            .collect();
        if cells.iter().any(|c| c.paras.iter().any(|p| p.list.is_some())) {
            self.warns.once(
                "tbllist",
                "表格里的列表编号没保留，只留了文字".into(),
            );
        }
        let inh_keys: Vec<&str> = TEXT_KEYS
            .iter()
            .copied()
            .chain(["align", "lineHeight", "textIndent", "hangingIndent"])
            .collect();
        let pool: Vec<(Map<String, Value>, usize)> = block_info
            .iter()
            .filter(|(_, h, s)| !h.is_empty() && !s.is_empty())
            .map(|(e, h, _)| {
                let mut m = e.clone();
                m.retain(|k, _| {
                    h.iter().any(|x| x == k) || !TEXT_KEYS.contains(&k.as_str())
                });
                (m, 1)
            })
            .collect();
        let thoist = hoist(&pool, &inh_keys);
        let exp_t = self.expected("Table", None, None);
        let soft: Vec<&str> = inh_keys
            .iter()
            .copied()
            .filter(|k| !thoist.contains_key(*k))
            .collect();
        let td = diff(&thoist, &to_map(&exp_t), &inh_keys, &soft);
        tstyle.extend(td);
        let tcomputed = merged(&exp_t, &from_map(&tstyle));
        // 和内置表格样式一样的字段不用写
        if let Ok(Value::Object(raw)) = serde_json::to_value(&exp_t) {
            let mut raw = raw;
            super::clean_map(&mut raw);
            tstyle.retain(|k, v| !raw.get(k).is_some_and(|e| same(e, v)));
        }
        let cell_base = tcomputed.inherited();

        // 表头：第一行写了 tblHeader，并且每格都有底纹（内置表头底纹去不掉）
        let header_ok = rows[0].header
            && blocks
                .iter()
                .filter(|b| cells[**b].r == 0)
                .all(|b| cells[*b].shd.is_some());
        if rows[0].header && !header_ok {
            self.warns.once(
                "tblheader",
                "表头行有格子没底纹，按普通行处理（fmtagno 的表头自带灰色底纹），跨页重复表头没保留".into(),
            );
        }
        let header_part: Style = if header_ok {
            let mut h = builtin_part("Table.header");
            h.merge(&builtin_part("Table.firstRow"));
            h
        } else {
            Style::default()
        };

        // 输出格子
        let mut out_rows: Vec<Vec<Value>> = vec![Vec::new(); nrows];
        for (bi, &b) in blocks.iter().enumerate() {
            let cl = &cells[b];
            let (eff, hoisted, segs) = &block_info[bi];
            let mut base = cell_base.clone();
            if header_ok && cl.r == 0 {
                base.merge(&header_part);
            }
            let soft: Vec<&str> = TEXT_KEYS
                .iter()
                .copied()
                .filter(|k| !hoisted.iter().any(|h| h == k))
                .collect();
            let mut keys: Vec<&str> = PARA_KEYS
                .iter()
                .copied()
                .filter(|k| !k.starts_with("border") && !k.starts_with("padding") && *k != "background")
                .collect();
            keys.extend(TEXT_KEYS.iter().copied());
            let mut d = if segs.is_empty() && hoisted.is_empty() {
                Map::new()
            } else {
                diff(eff, &to_map(&base), &keys, &soft)
            };
            d.remove("outlineLevel");
            if let Some(bg) = &cl.shd {
                let v = serde_json::to_value(bg).unwrap_or(Value::Null);
                if !base.background.as_ref().is_some_and(|x| x == bg) {
                    d.insert("background".into(), v);
                }
            }
            if let Some(v) = cl.valign
                && v != "top"
            {
                d.insert("verticalAlign".into(), json!(v));
            }
            if let Some(rot) = cl.rotation {
                d.insert("textRotation".into(), json!(rot));
            }
            for (i, k) in ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"].iter().enumerate() {
                if let Some(m) = cl.mar[i] {
                    let v = round2(m / 20.0);
                    if (v - pad[i]).abs() > 0.01 {
                        d.insert(k.to_string(), json!(v));
                    }
                }
            }
            // 边框：和表格同位置的不一样才写
            let e = block_edges(b);
            let last_r = cl.r + row_span[b] >= nrows;
            let last_c = cl.c + cl.span >= ncols;
            let pos = [
                if cl.r == 0 { 0 } else { 4 },
                if last_c { 1 } else { 5 },
                if last_r { 2 } else { 4 },
                if cl.c == 0 { 3 } else { 5 },
            ];
            for (k, side) in ["borderTop", "borderRight", "borderBottom", "borderLeft"].iter().enumerate() {
                if !same(&e[k], &base_edge(&base, &tstyle, pos[k])) {
                    d.insert(side.to_string(), e[k].clone());
                }
            }
            let computed = merged(&base, &from_map(&d));
            let text = self.inline_json(segs, &computed);
            compact(&mut d);
            let rs = row_span[b];
            let cs = cl.span.min(ncols - cl.c);
            let v = if d.is_empty() && rs == 1 && cs == 1 {
                text
            } else {
                let mut m = Map::new();
                m.insert("text".into(), text);
                if rs > 1 {
                    m.insert("rowSpan".into(), json!(rs));
                }
                if cs > 1 {
                    m.insert("colSpan".into(), json!(cs));
                }
                if !d.is_empty() {
                    m.insert("style".into(), Value::Object(d));
                }
                Value::Object(m)
            };
            out_rows[cl.r].push(v);
        }

        // 列宽
        let mut gpt: Vec<f32> = grid.clone();
        gpt.resize(ncols, 0.0);
        if gpt.iter().any(|w| *w <= 0.0) {
            // 没有 tblGrid：按第一行格子的宽度均分
            let avg = self.content_width / ncols as f32;
            for w in gpt.iter_mut() {
                if *w <= 0.0 {
                    *w = avg;
                }
            }
        }
        let auto_col: Vec<bool> = (0..ncols)
            .map(|c| {
                let col: Vec<&TCell> = cells.iter().filter(|x| x.c == c && x.span == 1).collect();
                !col.is_empty() && col.iter().all(|x| x.auto_w)
            })
            .collect();
        let sum: f32 = gpt.iter().sum();
        let cw = self.content_width;
        let ratio = match &tblw {
            Some((_, t, pct)) if t == "pct" || *pct => true,
            Some((w, t, _)) if t == "dxa" && *w > 0.0 => ((w / 20.0) - cw).abs() <= cw * 0.01 + 1.0,
            _ => (sum - cw).abs() <= cw * 0.01 + 1.0,
        };
        if let Some((w, t, pct)) = &tblw
            && (t == "pct" || *pct)
        {
            let p = if *pct { *w } else { w / 50.0 };
            if (p - 100.0).abs() > 0.5 {
                tstyle.insert("width".into(), json!(format!("{}%", round2(p))));
            }
        }
        let widths: Vec<Value> = (0..ncols)
            .map(|c| {
                if auto_col[c] {
                    json!("auto")
                } else if ratio {
                    json!(round2(gpt[c]))
                } else {
                    json!(format!("{}pt", round2(gpt[c])))
                }
            })
            .collect();
        let all_equal_ratio = ratio
            && !auto_col.iter().any(|x| *x)
            && gpt.iter().all(|w| (w - gpt[0]).abs() < 0.5);
        let inferred_auto = auto_col.iter().any(|x| *x);
        match layout.as_deref() {
            Some("autofit") if !inferred_auto => {
                tstyle.insert("tableLayout".into(), json!("auto"));
            }
            Some("fixed") if inferred_auto => {
                tstyle.insert("tableLayout".into(), json!("fixed"));
            }
            _ => {}
        }
        let _ = size_of;

        let mut props = Map::new();
        let mut body_rows = out_rows;
        if header_ok {
            let head = body_rows.remove(0);
            props.insert("columns".into(), Value::Array(head));
        }
        props.insert(
            "rows".into(),
            Value::Array(body_rows.into_iter().map(Value::Array).collect()),
        );
        if !all_equal_ratio {
            props.insert("widths".into(), Value::Array(widths));
        }
        if rows.iter().any(|r| r.height.is_some()) {
            let hs: Vec<Value> = rows
                .iter()
                .map(|r| match r.height {
                    None => Value::Null,
                    Some((h, false)) => json!(format!("{h}pt")),
                    Some((h, true)) => json!({ "height": format!("{h}pt"), "rule": "exact" }),
                })
                .collect();
            props.insert("rowHeights".into(), Value::Array(hs));
        }
        let id = self.id("table");
        let mut el = json!({ "type": "Table", "props": props });
        compact(&mut tstyle);
        el["style"] = Value::Object(tstyle);
        if !cell_part.is_empty() {
            el["parts"] = json!({ "cell": cell_part });
        }
        self.add(&id, el);
        id
    }
}

fn builtin_part(key: &str) -> Style {
    crate::cascade::builtin(key).cloned().unwrap_or_default()
}

/// 表格样式里某个位置（上右下左、内横、内竖）的边框，按 fmtagno 的规则取。
fn base_edge(_cell: &Style, t: &Map<String, Value>, pos: usize) -> Value {
    let names = [
        "borderTop",
        "borderRight",
        "borderBottom",
        "borderLeft",
        "borderInsideH",
        "borderInsideV",
    ];
    t.get(names[pos])
        .or_else(|| t.get("border"))
        .cloned()
        .unwrap_or(json!("none"))
}
