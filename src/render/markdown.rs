//! Markdown 渲染器，按 GitHub Flavored Markdown 写。
//!
//! Markdown 没有样式，只留下结构：标题、段落、粗体斜体删除线、链接、列表、
//! 表格、代码、引用、分割线、图片。提示框写成 GitHub 的 `> [!NOTE]`。
//! 有合并格或没有表头的表格退回内嵌 HTML；格子里的换行写成 `<br>`。
//! 分页、页眉页脚、页码域、字号颜色这些丢掉，第一次遇到时给警告。

use std::collections::HashSet;
use std::fmt::Write as _;

use super::common::{self, Warn};
use super::{Options, Output};
use crate::{
    components::{
        CalloutProps, Cell, CodeProps, Component, ImageProps, Inline, ListItem, Run,
        TableOfContentsProps, TableProps,
    },
    doc::{Doc, Node},
    style::{Strike, Style, VerticalAlign},
};

pub fn render(doc: &Doc, tree: &Node, _opts: &Options) -> Output {
    let mut r = Md {
        blocks: Vec::new(),
        warn: Warn::default(),
        root: tree,
        anchors: HashSet::new(),
    };
    // 有目录时，被目录收进去的标题前面放一个锚点，目录链接才跳得过去
    collect_anchors(tree, &mut r.anchors);
    if doc.meta.page.header.is_some() || doc.meta.page.footer.is_some() {
        r.drop_once("页眉页脚", &tree.id);
    }
    r.node(tree);
    let mut out = r.blocks.join("\n\n");
    out.push('\n');
    Output {
        bytes: out.into_bytes(),
        warnings: r.warn.list,
    }
}

fn collect_anchors(root: &Node, out: &mut HashSet<String>) {
    fn find_depth(n: &Node, best: &mut u8) {
        if let Component::TableOfContents(p) = &n.component {
            *best = (*best).max(p.depth());
        }
        for c in &n.children {
            find_depth(c, best);
        }
    }
    let mut depth = 0;
    find_depth(root, &mut depth);
    if depth > 0 {
        for e in common::toc_entries(root, depth) {
            out.insert(e.id);
        }
    }
}

struct Md<'a> {
    /// 一块一个字符串，最后用空行连起来
    blocks: Vec<String>,
    warn: Warn,
    root: &'a Node,
    anchors: HashSet<String>,
}

impl Md<'_> {
    /// Markdown 表达不了的东西，每种只报一次。
    fn drop_once(&mut self, what: &str, id: &str) {
        self.warn.unsupported("Markdown", what, id);
    }

    fn node(&mut self, n: &Node) {
        if n.style.hidden == Some(true) {
            return;
        }
        match &n.component {
            Component::Document(_) => self.children(n),
            Component::Section(p) => {
                let lay = p.layout();
                if !lay.header.is_empty() || !lay.footer.is_empty() {
                    self.drop_once("页眉页脚", &n.id);
                }
                self.children(n);
            }
            Component::Slide(p) => {
                if let Some(t) = &p.title {
                    self.blocks.push(format!("## {}", esc(t)));
                }
                self.children(n);
            }
            Component::Heading(p) => {
                let level = p.level.clamp(1, 6) as usize;
                let text = self.inline(&p.text, &n.style, &n.id).replace('\n', " ");
                let mut b = String::new();
                if self.anchors.contains(&n.id) {
                    let _ = writeln!(b, "<a id=\"{}\"></a>", attr(&n.id));
                }
                let _ = write!(b, "{} {}", "#".repeat(level), text.trim());
                self.blocks.push(b);
            }
            Component::Text(p) => {
                let t = self.inline(&p.text, &n.style, &n.id);
                if !t.trim().is_empty() {
                    self.blocks.push(hard_breaks(&t));
                }
            }
            Component::Quote(p) => {
                let t = self.inline(&p.text, &n.style, &n.id);
                self.blocks.push(quote(&hard_breaks(&t)));
            }
            Component::Callout(p) => self.callout(n, p),
            Component::Divider(_) => self.blocks.push("---".into()),
            Component::TableOfContents(p) => self.toc(p),
            Component::Code(p) => self.code(p),
            Component::Math(p) => {
                let mut b = format!("$$\n{}", p.latex.trim());
                if let Some(no) = &p.number {
                    let _ = write!(b, " \\tag{{{no}}}");
                }
                b.push_str("\n$$");
                self.blocks.push(b);
            }
            Component::List(p) => {
                let mut s = String::new();
                self.list(n, &p.items, p.ordered, &mut s);
                self.blocks.push(s.trim_end().to_string());
            }
            Component::Table(p) => self.table(n, p),
            Component::Image(p) => self.image(p),
            Component::PageBreak(_) => self.drop_once("PageBreak（分页）", &n.id),
        }
    }

    fn children(&mut self, n: &Node) {
        for c in &n.children {
            self.node(c);
        }
    }

    fn callout(&mut self, n: &Node, p: &CalloutProps) {
        let t = hard_breaks(&self.inline(&p.text, &n.style, &n.id));
        // GitHub 的提示框会自己画图标，常见的 emoji 换成对应的种类
        let (kind, keep_icon) = match p.icon.as_deref().map(|s| s.trim_end_matches('\u{fe0f}')) {
            Some("💡") => ("TIP", false),
            Some("⚠" | "🚧") => ("WARNING", false),
            Some("❗" | "‼" | "📌") => ("IMPORTANT", false),
            Some("⛔" | "🚫" | "🛑" | "❌") => ("CAUTION", false),
            Some("ℹ" | "📝") | None => ("NOTE", false),
            Some(_) => ("NOTE", true),
        };
        // 解析时图标已经插到文字最前面（图标 + en 空格）。换成了种类就去掉，认不出就留着
        let body = match p.icon.as_deref() {
            Some(i) if !keep_icon => t
                .strip_prefix(&format!("{i}\u{2002}"))
                .map(str::to_string)
                .unwrap_or(t),
            Some(i) => t.replacen(&format!("{i}\u{2002}"), &format!("{i} "), 1),
            None => t,
        };
        self.blocks.push(format!("> [!{kind}]\n{}", quote(&body)));
    }

    fn toc(&mut self, p: &TableOfContentsProps) {
        let entries = common::toc_entries(self.root, p.depth());
        let mut b = String::new();
        if let Some(t) = &p.title {
            let _ = write!(b, "**{}**\n\n", esc(t));
        }
        if entries.is_empty() {
            if !b.is_empty() {
                self.blocks.push(b.trim_end().to_string());
            }
            return;
        }
        let min = entries.iter().map(|e| e.level).min().unwrap_or(1);
        for e in &entries {
            let indent = "  ".repeat((e.level - min) as usize);
            let _ = writeln!(b, "{indent}- [{}](#{})", esc(&e.text), e.id);
        }
        self.blocks.push(b.trim_end().to_string());
    }

    fn code(&mut self, p: &CodeProps) {
        // 围栏要比代码里最长的一串反引号更长
        let longest = longest_run(&p.code, '`');
        let fence = "`".repeat(longest.max(2) + 1);
        let lang = p.language.as_deref().unwrap_or("");
        self.blocks.push(format!(
            "{fence}{lang}\n{}\n{fence}",
            p.code.trim_end_matches('\n')
        ));
    }

    fn list(&mut self, n: &Node, items: &[ListItem], ordered: bool, out: &mut String) {
        for (i, it) in items.iter().enumerate() {
            let marker = if ordered {
                format!("{}.", i + 1)
            } else {
                "-".to_string()
            };
            let text = self.inline(it.text(), &n.style, &n.id);
            let cont = " ".repeat(marker.len() + 1);
            let text = hard_breaks(&text).replace('\n', &format!("\n{cont}"));
            let _ = writeln!(out, "{marker} {text}");
            let kids = it.children();
            if !kids.is_empty() {
                let o = it.ordered().unwrap_or(ordered);
                // 子列表缩进到父项文字对齐的位置
                let mut sub = String::new();
                self.list(n, kids, o, &mut sub);
                for line in sub.lines() {
                    if line.is_empty() {
                        out.push('\n');
                    } else {
                        let _ = writeln!(out, "{cont}{line}");
                    }
                }
            }
        }
    }

    fn table(&mut self, n: &Node, p: &TableProps) {
        let all = p.columns.iter().chain(p.rows.iter().flatten());
        let spans = all
            .into_iter()
            .any(|c| c.row_span() > 1 || c.col_span() > 1);
        let ncols = p
            .rows
            .iter()
            .map(|r| r.len())
            .chain(std::iter::once(p.columns.len()))
            .max()
            .unwrap_or(0);
        if p.columns.is_empty() || spans || ncols == 0 {
            self.html_table(n, p);
            return;
        }
        let base = &n.style;
        let cell = |me: &mut Self, c: &Cell| -> String {
            me.inline(c.text(), base, &n.id)
                .trim()
                .replace('\n', "<br>")
                .replace('|', "\\|")
        };
        let mut b = String::new();
        let head: Vec<String> = (0..ncols)
            .map(|i| p.columns.get(i).map(|c| cell(self, c)).unwrap_or_default())
            .collect();
        let _ = writeln!(b, "| {} |", head.join(" | "));
        let _ = writeln!(b, "|{}|", vec![" --- "; ncols].join("|"));
        for r in &p.rows {
            let row: Vec<String> = (0..ncols)
                .map(|i| r.get(i).map(|c| cell(self, c)).unwrap_or_default())
                .collect();
            let _ = writeln!(b, "| {} |", row.join(" | "));
        }
        self.blocks.push(b.trim_end().to_string());
    }

    /// 合并格、没有表头：GFM 表格写不出来，写 HTML。
    fn html_table(&mut self, n: &Node, p: &TableProps) {
        let mut b = String::from("<table>\n");
        let cell = |me: &mut Self, tag: &str, c: &Cell, b: &mut String| {
            let mut a = String::new();
            if c.row_span() > 1 {
                let _ = write!(a, " rowspan=\"{}\"", c.row_span());
            }
            if c.col_span() > 1 {
                let _ = write!(a, " colspan=\"{}\"", c.col_span());
            }
            let t = me.inline_html(c.text(), &n.style, &n.id);
            let _ = writeln!(b, "<{tag}{a}>{t}</{tag}>");
        };
        if !p.columns.is_empty() {
            b.push_str("<thead>\n<tr>\n");
            for c in &p.columns {
                cell(self, "th", c, &mut b);
            }
            b.push_str("</tr>\n</thead>\n");
        }
        b.push_str("<tbody>\n");
        for r in &p.rows {
            b.push_str("<tr>\n");
            for c in r {
                cell(self, "td", c, &mut b);
            }
            b.push_str("</tr>\n");
        }
        b.push_str("</tbody>\n</table>");
        self.blocks.push(b);
    }

    fn image(&mut self, p: &ImageProps) {
        let mut b = format!("![{}]({})", esc(&p.alt), url(&p.src));
        if let Some(c) = &p.caption {
            let _ = write!(b, "\n\n*{}*", esc(c));
        }
        self.blocks.push(b);
    }

    /// 一段文字转 Markdown。粗体、斜体这些只在和段落本身不同时才标出来，
    /// 免得整个标题都套上 `**`。
    fn inline(&mut self, t: &Inline, base: &Style, id: &str) -> String {
        let mut out = String::new();
        let mut t = t.clone();
        t.normalize();
        for r in t.runs() {
            self.run(r, base, id, &mut out);
        }
        out
    }

    fn run(&mut self, r: &Run, base: &Style, id: &str, out: &mut String) {
        let s = &r.computed;
        if s.hidden == Some(true) {
            return;
        }
        if r.field.is_some() {
            self.drop_once("页码域", id);
            return;
        }
        if let Some(src) = &r.image {
            let _ = write!(out, "![{}]({})", esc(&r.text), url(src));
            return;
        }
        if r.text.is_empty() {
            return;
        }
        let mut body = if r.is_math() {
            format!("${}$", r.text.trim().replace('\n', " "))
        } else if r.is_code() {
            let n = longest_run(&r.text, '`') + 1;
            let f = "`".repeat(n);
            let pad = if r.text.starts_with('`') || r.text.ends_with('`') {
                " "
            } else {
                ""
            };
            format!("{f}{pad}{}{pad}{f}", r.text.replace('\n', " "))
        } else {
            esc(&r.text)
        };
        let on = |v: Option<bool>| v == Some(true);
        let mut marks: Vec<(&str, &str)> = Vec::new();
        if on(s.bold) && !on(base.bold) {
            marks.push(("**", "**"));
        }
        if on(s.italic) && !on(base.italic) {
            marks.push(("*", "*"));
        }
        if matches!(s.strike, Some(Strike::Single | Strike::Double))
            && !matches!(base.strike, Some(Strike::Single | Strike::Double))
        {
            marks.push(("~~", "~~"));
        }
        match s.vertical_align {
            Some(VerticalAlign::Super) => marks.push(("<sup>", "</sup>")),
            Some(VerticalAlign::Sub) => marks.push(("<sub>", "</sub>")),
            _ => {}
        }
        if !marks.is_empty() && !r.is_code() && !r.is_math() {
            // 标记不能贴着空白，把首尾空白挪到标记外面
            let lead: String = body.chars().take_while(|c| c.is_whitespace()).collect();
            let trail: String = body
                .chars()
                .rev()
                .take_while(|c| c.is_whitespace())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let core = body.trim().to_string();
            if !core.is_empty() {
                let open: String = marks.iter().map(|m| m.0).collect();
                let close: String = marks.iter().rev().map(|m| m.1).collect();
                body = format!("{lead}{open}{core}{close}{trail}");
            }
        }
        match &r.href {
            Some(h) => {
                let _ = write!(out, "[{}]({})", body, url(h));
            }
            None => out.push_str(&body),
        }
    }

    /// HTML 表格格子里的文字：Markdown 标记在 HTML 块里不生效，写成标签。
    fn inline_html(&mut self, t: &Inline, base: &Style, id: &str) -> String {
        let mut t = t.clone();
        t.normalize();
        let mut out = String::new();
        let on = |v: Option<bool>| v == Some(true);
        for r in t.runs() {
            let s = &r.computed;
            if s.hidden == Some(true) {
                continue;
            }
            if r.field.is_some() {
                self.drop_once("页码域", id);
                continue;
            }
            if let Some(src) = &r.image {
                let _ = write!(out, "<img src=\"{}\" alt=\"{}\">", attr(src), attr(&r.text));
                continue;
            }
            let mut body = html_esc(&r.text).replace('\n', "<br>");
            if r.is_math() {
                body = format!("${}$", html_esc(r.text.trim()).replace('\n', " "));
            } else if r.is_code() {
                body = format!("<code>{body}</code>");
            }
            if on(s.bold) && !on(base.bold) {
                body = format!("<strong>{body}</strong>");
            }
            if on(s.italic) && !on(base.italic) {
                body = format!("<em>{body}</em>");
            }
            if matches!(s.strike, Some(Strike::Single | Strike::Double)) {
                body = format!("<del>{body}</del>");
            }
            match s.vertical_align {
                Some(VerticalAlign::Super) => body = format!("<sup>{body}</sup>"),
                Some(VerticalAlign::Sub) => body = format!("<sub>{body}</sub>"),
                _ => {}
            }
            if let Some(h) = &r.href {
                body = format!("<a href=\"{}\">{body}</a>", attr(h));
            }
            out.push_str(&body);
        }
        out
    }
}

/// 段落里的换行：Markdown 里单个换行会被并成空格，行尾加反斜杠才是硬换行。
fn hard_breaks(s: &str) -> String {
    let s = s.trim_end_matches('\n');
    s.split('\n').collect::<Vec<_>>().join("\\\n")
}

/// 每行前面加 `> `。
fn quote(s: &str) -> String {
    s.lines()
        .map(|l| {
            if l.is_empty() {
                ">".to_string()
            } else {
                format!("> {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn longest_run(s: &str, ch: char) -> usize {
    let mut best = 0;
    let mut cur = 0;
    for c in s.chars() {
        if c == ch {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

/// 转义 Markdown 的特殊字符。行首的 `#`、`>`、`-`、`+`、`1.` 也要转，免得变成标题、引用、列表。
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, line) in s.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut start = true;
        let chars: Vec<char> = line.chars().collect();
        for (j, &c) in chars.iter().enumerate() {
            match c {
                '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' => {
                    out.push('\\');
                    out.push(c);
                }
                '#' | '-' | '+' | '=' if start => {
                    out.push('\\');
                    out.push(c);
                }
                '.' | ')'
                    if start_number(&chars[..j])
                        && chars.get(j + 1).is_none_or(|n| n.is_whitespace()) =>
                {
                    out.push('\\');
                    out.push(c);
                }
                _ => out.push(c),
            }
            if !c.is_whitespace() && !c.is_ascii_digit() {
                start = false;
            }
        }
    }
    out
}

/// 这一行到这里为止是不是只有空白加数字（`12. ` 会被当成有序列表，`1.1` 不会）。
fn start_number(prefix: &[char]) -> bool {
    let t: Vec<&char> = prefix.iter().skip_while(|c| c.is_whitespace()).collect();
    !t.is_empty() && t.iter().all(|c| c.is_ascii_digit())
}

/// 链接和图片地址：有空格或括号时用尖括号包起来。
fn url(s: &str) -> String {
    if s.contains([' ', '(', ')', '<', '>']) {
        format!("<{}>", s.replace('<', "%3C").replace('>', "%3E"))
    } else {
        s.to_string()
    }
}

fn html_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn attr(s: &str) -> String {
    html_esc(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes() {
        assert_eq!(esc("a*b_c"), "a\\*b\\_c");
        assert_eq!(esc("# 不是标题"), "\\# 不是标题");
        assert_eq!(esc("1. 不是列表"), "1\\. 不是列表");
        assert_eq!(esc("版本 1.2"), "版本 1.2");
        assert_eq!(esc("a-b"), "a-b");
    }

    #[test]
    fn breaks_and_quotes() {
        assert_eq!(hard_breaks("a\nb\n"), "a\\\nb");
        assert_eq!(quote("a\\\nb"), "> a\\\n> b");
    }
}
