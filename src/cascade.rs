//! 样式层叠：内置默认样式、样式集、顶层 styles、继承、元素自己的样式，按 docs/STYLE.md 第十一、十二节的顺序叠起来。

use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    components::{self, Component, Run, Spec},
    doc::{Doc, Element},
    style::{self, Color, Style, Underline, UnderlineStyle},
};

/// 一个样式集。单独成文件时要写 `name`。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleSetDef {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub extends: Option<String>,
    /// 全局样式：只能写会继承的字段
    #[serde(default)]
    pub global: Option<Value>,
    /// 组件样式：键是组件名或「组件名.部位」
    #[serde(default)]
    pub components: BTreeMap<String, Value>,
    /// 命名样式
    #[serde(default)]
    pub styles: BTreeMap<String, Value>,
}

/// 从文件或目录读样式集（目录里的 .json，不进子目录）。
pub fn load_style_sets(path: &Path, into: &mut BTreeMap<String, StyleSetDef>) -> Result<()> {
    let files: Vec<PathBuf> = if path.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(path)
            .with_context(|| format!("读不了目录 {}", path.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
            })
            .collect();
        v.sort();
        v
    } else {
        vec![path.to_path_buf()]
    };
    for f in files {
        let text =
            std::fs::read_to_string(&f).with_context(|| format!("读不了 {}", f.display()))?;
        let def: StyleSetDef = serde_json::from_str(&text)
            .with_context(|| format!("{} 不是合法的样式集", f.display()))?;
        let name = def
            .name
            .clone()
            .ok_or_else(|| anyhow::anyhow!("{} 里的样式集要写 name", f.display()))?;
        if into.contains_key(&name) {
            bail!("样式集 `{name}` 定义了不止一次（{}）", f.display());
        }
        into.insert(name, def);
    }
    Ok(())
}

/// 算好的样式表：选定的样式集（含 extends）加顶层 styles。
#[derive(Debug, Default)]
pub struct Sheet {
    root: Style,
    /// 组件样式：样式集的在前，顶层 styles 的叠在后面
    comp: BTreeMap<String, Style>,
    named_set: BTreeMap<String, Style>,
    named_top: BTreeMap<String, Style>,
    /// 变量：样式集 global 里 `$名字` 定义，`${名字}` 引用
    vars: BTreeMap<String, Value>,
}

impl Sheet {
    /// `external` 是文件里读来的样式集；文档里同名的优先。`choose` 是命令行指定的样式集。
    pub fn build(
        doc: &Doc,
        external: &BTreeMap<String, StyleSetDef>,
        choose: Option<&str>,
    ) -> Result<Sheet> {
        let mut sets = external.clone();
        for (k, v) in &doc.style_sets {
            sets.insert(k.clone(), v.clone());
        }
        let names: HashSet<&str> = components::BUILTINS
            .iter()
            .map(|s| s.name)
            .chain(components::STYLE_ONLY.iter().copied())
            .chain(doc.custom_names.iter().map(String::as_str))
            .collect();

        let mut sheet = Sheet::default();
        let mut root = builtin("$global").cloned().unwrap_or_default();

        // 样式集，父在前
        if let Some(name) = choose.or(doc.style_set.as_deref()) {
            let mut chain = Vec::new();
            let mut cur = Some(name.to_string());
            while let Some(n) = cur {
                if chain.contains(&n) {
                    bail!("样式集的 extends 成环了：{} → {n}", chain.join(" → "));
                }
                let def = sets.get(&n).ok_or_else(|| {
                    anyhow::anyhow!(
                        "样式集 `{n}` 没定义；现有：{}",
                        sets.keys().cloned().collect::<Vec<_>>().join(" ")
                    )
                })?;
                cur = def.extends.clone();
                chain.push(n);
            }
            // 先收齐变量（子样式集覆盖父的），后面的样式值才能引用
            let mut globals = Vec::new();
            for n in chain.iter().rev() {
                let ctx = || format!("样式集 `{n}`");
                let g = match &sets[n].global {
                    Some(Value::Object(m)) => {
                        let mut rest = serde_json::Map::new();
                        for (k, v) in m {
                            if let Some(name) = k.strip_prefix('$') {
                                check_var_name(name).with_context(ctx)?;
                                sheet.vars.insert(name.to_string(), v.clone());
                            } else {
                                rest.insert(k.clone(), v.clone());
                            }
                        }
                        Some(Value::Object(rest))
                    }
                    Some(v) => Some(v.clone()),
                    None => None,
                };
                globals.push(g);
            }
            sheet.expand_vars()?;
            for (n, g) in chain.iter().rev().zip(globals) {
                let def = &sets[n];
                let ctx = || format!("样式集 `{n}`");
                if let Some(g) = &g {
                    let g = sheet.subst(g).with_context(ctx)?;
                    style::check_inherited_fields(&g, "样式集的 global ").with_context(ctx)?;
                    root.merge(&style::parse_object(&g).with_context(ctx)?);
                }
                for (k, v) in &def.components {
                    check_component_key(k, &names).with_context(ctx)?;
                    let s = sheet
                        .subst(v)
                        .and_then(|v| parse_obj(&v))
                        .with_context(|| format!("样式集 `{n}` 的 `{k}`"))?;
                    sheet.comp.entry(k.clone()).or_default().merge(&s);
                }
                for (k, v) in &def.styles {
                    let s = sheet
                        .subst(v)
                        .and_then(|v| parse_obj(&v))
                        .with_context(|| format!("样式集 `{n}` 的样式 `{k}`"))?;
                    sheet.named_set.insert(k.clone(), s);
                }
            }
        }

        // 顶层 styles：组件样式叠在样式集后面，其他是命名样式
        for (k, v) in &doc.styles {
            let s = sheet
                .subst(v)
                .and_then(|v| parse_obj(&v))
                .with_context(|| format!("styles 里的 `{k}`"))?;
            if names.contains(head(k)) {
                check_component_key(k, &names)?;
                sheet.comp.entry(k.clone()).or_default().merge(&s);
            } else {
                sheet.named_top.insert(k.clone(), s);
            }
        }
        sheet.root = root;
        Ok(sheet)
    }

    /// 把值里的 `${名字}` 换成变量。整个字符串就是一个引用时换成变量原值（数字、对象都行），
    /// 夹在别的文字里时按文字拼进去；`$${` 写出字面的 `${`。
    pub fn subst(&self, v: &Value) -> Result<Value> {
        self.subst_in(v, &mut Vec::new())
    }

    fn subst_in(&self, v: &Value, stack: &mut Vec<String>) -> Result<Value> {
        Ok(match v {
            Value::String(s) if s.contains("${") => self.subst_str(s, stack)?,
            Value::Array(a) => Value::Array(
                a.iter()
                    .map(|x| self.subst_in(x, stack))
                    .collect::<Result<_>>()?,
            ),
            Value::Object(m) => {
                let mut out = serde_json::Map::new();
                for (k, x) in m {
                    out.insert(k.clone(), self.subst_in(x, stack)?);
                }
                Value::Object(out)
            }
            _ => v.clone(),
        })
    }

    fn var(&self, name: &str, stack: &mut Vec<String>) -> Result<Value> {
        if stack.iter().any(|n| n == name) {
            bail!("变量互相引用成环了：{} → {name}", stack.join(" → "));
        }
        let v = self.vars.get(name).ok_or_else(|| {
            anyhow::anyhow!(
                "变量 `${{{name}}}` 没定义；在样式集的 global 里写 \"${name}\": 值{}",
                if self.vars.is_empty() {
                    String::new()
                } else {
                    format!(
                        "。现有：{}",
                        self.vars.keys().cloned().collect::<Vec<_>>().join(" ")
                    )
                }
            )
        })?;
        stack.push(name.to_string());
        let out = self.subst_in(v, stack);
        stack.pop();
        out
    }

    fn subst_str(&self, s: &str, stack: &mut Vec<String>) -> Result<Value> {
        // 整个字符串就是 ${x}：保留变量原来的类型
        if let Some(name) = s.strip_prefix("${").and_then(|r| r.strip_suffix('}'))
            && !name.contains(['{', '}', '$'])
        {
            return self.var(name, stack);
        }
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find('$') {
            out.push_str(&rest[..i]);
            let after = &rest[i..];
            if let Some(r) = after.strip_prefix("$${") {
                out.push_str("${");
                rest = r;
            } else if let Some(r) = after.strip_prefix("${") {
                let end = r
                    .find('}')
                    .ok_or_else(|| anyhow::anyhow!("`{s}` 里的 ${{ 没有配对的 }}"))?;
                let name = &r[..end];
                match self.var(name, stack)? {
                    Value::String(x) => out.push_str(&x),
                    Value::Number(x) => out.push_str(&x.to_string()),
                    Value::Bool(x) => out.push_str(&x.to_string()),
                    _ => bail!(
                        "变量 `${{{name}}}` 不是文字或数字，只能单独整个引用，不能拼在 `{s}` 里"
                    ),
                }
                rest = &r[end + 1..];
            } else {
                out.push('$');
                rest = &after[1..];
            }
        }
        out.push_str(rest);
        Ok(Value::String(out))
    }

    /// 变量值里也能引用别的变量：先全部展开，顺便查出成环和未定义。
    fn expand_vars(&mut self) -> Result<()> {
        let mut done = BTreeMap::new();
        for k in self.vars.keys() {
            done.insert(k.clone(), self.var(k, &mut Vec::new())?);
        }
        self.vars = done;
        Ok(())
    }

    /// 根元素的「父样式」：内置全局默认值加样式集 global。
    pub fn root_style(&self) -> &Style {
        &self.root
    }

    fn comp(&self, key: &str) -> Option<&Style> {
        self.comp.get(key)
    }

    /// 解析 `style` 的值：样式名、样式名数组或样式对象。
    pub fn resolve(&self, v: &Value) -> Result<Style> {
        let v = &self.subst(v)?;
        let mut out = Style::default();
        match v {
            Value::Null => {}
            Value::String(name) => out.merge(self.lookup(name)?),
            Value::Array(items) => {
                for it in items {
                    out.merge(&self.resolve(it)?);
                }
            }
            Value::Object(_) => out.merge(&style::parse_object(v)?),
            _ => bail!("style 只能是样式名、样式名数组或样式对象"),
        }
        Ok(out)
    }

    fn lookup(&self, name: &str) -> Result<&Style> {
        self.named_set
            .get(name)
            .or_else(|| self.named_top.get(name))
            .ok_or_else(|| anyhow::anyhow!("样式 `{name}` 没定义"))
    }

    /// 算一个元素的完整样式和部位样式。
    pub fn element(
        &self,
        el: &Element,
        spec: &Spec,
        component: &Component,
        parent: &Style,
    ) -> Result<(Style, BTreeMap<String, Style>)> {
        let kind = spec.name;
        let variant = match component {
            Component::Heading(h) => Some(h.level.to_string()),
            _ => None,
        };
        let mut s = parent.inherited();
        let mut keys = vec![kind.to_string()];
        if let Some(v) = &variant {
            keys.push(format!("{kind}.{v}"));
        }
        for k in &keys {
            if let Some(b) = builtin(k) {
                s.merge(b);
            }
        }
        for k in &keys {
            if let Some(c) = self.comp(k) {
                s.merge(c);
            }
        }
        for t in &el.template_styles {
            s.merge(&self.resolve(t).context("自定义组件模板里的 style")?);
        }
        for o in &el.origins {
            if o.root
                && let Some(c) = self.comp(&o.component)
            {
                s.merge(c);
            }
            if let Some(c) = self.comp(&format!("{}:{}", o.component, o.element)) {
                s.merge(c);
            }
        }
        if let Some(v) = &el.style {
            s.merge(&self.resolve(v)?);
        }

        // 部位
        for k in el.parts.keys() {
            if !spec.parts.contains(&k.as_str()) {
                bail!(
                    "{kind} 没有部位 `{k}`；可选：{}",
                    if spec.parts.is_empty() {
                        "（无）".to_string()
                    } else {
                        spec.parts.join(" ")
                    }
                );
            }
        }
        let mut parts = BTreeMap::new();
        for p in spec.parts {
            let mut ps = Style::default();
            let mut any = false;
            let mut add = |x: &Style| {
                ps.merge(x);
                any = true;
            };
            if let Some(b) = builtin(&format!("{kind}.{p}")) {
                add(b);
            }
            if let Some(c) = self.comp(&format!("{kind}.{p}")) {
                add(c);
            }
            for tp in &el.template_parts {
                if let Some(v) = tp.get(*p) {
                    add(&self.resolve(v).context("自定义组件模板里的 parts")?);
                }
            }
            for o in &el.origins {
                if let Some(c) = self.comp(&format!("{}:{}.{p}", o.component, o.element)) {
                    add(c);
                }
            }
            if let Some(v) = el.parts.get(*p) {
                add(&self.resolve(v).with_context(|| format!("部位 `{p}`"))?);
            }
            if any {
                parts.insert(p.to_string(), ps);
            }
        }
        Ok((s, parts))
    }

    /// 算一个 run 的文字样式：段落样式 → 行内代码（行内公式）样式 → 链接默认 → 简写 → run 的 style。
    pub fn run(&self, r: &Run, para: &Style) -> Result<Style> {
        let mut s = para.text_only();
        s.background = None;
        if r.is_code() {
            if let Some(b) = builtin("InlineCode") {
                s.merge(b);
            }
            if let Some(c) = self.comp("InlineCode") {
                s.merge(c);
            }
        }
        if r.is_math()
            && let Some(c) = self.comp("InlineMath")
        {
            s.merge(c);
        }
        if r.href.is_some() {
            if r.color.is_none() {
                s.color = Some(Color::parse("#0563C1").unwrap());
            }
            if r.underline.is_none() {
                s.underline = Some(Underline(UnderlineStyle::Single));
            }
        }
        if let Some(b) = r.bold {
            s.bold = Some(b);
        }
        if let Some(i) = r.italic {
            s.italic = Some(i);
        }
        if let Some(u) = r.underline {
            s.underline = Some(u);
        }
        if let Some(c) = &r.color {
            s.color = Some(c.clone());
        }
        if let Some(v) = &r.style {
            if v.is_object() {
                style::check_text_fields(v, "run 的 style ")?;
            }
            s.merge(&self.resolve(v)?.text_only());
        }
        Ok(s)
    }
}

/// 键的组件名部分：`Heading.1` → `Heading`，`PriceTable:t.header` → `PriceTable`。
fn head(k: &str) -> &str {
    k.split(['.', ':']).next().unwrap_or(k)
}

fn check_component_key(k: &str, names: &HashSet<&str>) -> Result<()> {
    let h = head(k);
    if !names.contains(h) {
        bail!("`{k}` 不是组件样式的键：组件 `{h}` 不存在");
    }
    if k.contains(':') {
        // 自定义组件内部元素，展开前不好校验
        return Ok(());
    }
    if let Some(sub) = k.strip_prefix(h).and_then(|x| x.strip_prefix('.')) {
        match components::spec(h) {
            Some(s) if s.name == "Heading" => {
                if !matches!(sub, "1" | "2" | "3" | "4" | "5" | "6") {
                    bail!("`{k}`：标题级别只有 1 到 6");
                }
            }
            Some(s) => {
                if !s.parts.contains(&sub) {
                    bail!("`{k}`：{h} 没有部位 `{sub}`");
                }
            }
            None => bail!("`{k}`：只有内置组件能用「组件名.部位」"),
        }
    }
    Ok(())
}

fn check_var_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        bail!("变量名 `${name}` 只能用字母、数字、_ 和 -");
    }
    Ok(())
}

fn parse_obj(v: &Value) -> Result<Style> {
    if !v.is_object() {
        bail!("要写样式对象");
    }
    style::parse_object(v)
}

/// 内置默认样式。`$global` 是根元素的起点。
pub(crate) fn builtin(key: &str) -> Option<&'static Style> {
    static B: OnceLock<BTreeMap<String, Style>> = OnceLock::new();
    B.get_or_init(|| {
        let mono = json!({ "latin": "Menlo, Consolas, DejaVu Sans Mono" });
        let raw = json!({
            "$global": { "fontSize": 10.5, "lineHeight": 1.5 },
            "Heading": { "bold": true, "keepWithNext": true, "spaceBefore": 12, "spaceAfter": 6, "lineHeight": 1.25 },
            "Heading.1": { "fontSize": 20, "spaceBefore": 18 },
            "Heading.2": { "fontSize": 16 },
            "Heading.3": { "fontSize": 14 },
            "Heading.4": { "fontSize": 12 },
            "Heading.5": { "fontSize": 11 },
            "Heading.6": { "fontSize": 11 },
            "Text": { "spaceAfter": 6 },
            "Code": { "fontFamily": mono, "fontSize": 9, "lineHeight": 1.4, "background": "#F6F8FA",
                      "padding": 8, "spaceAfter": 8, "keepTogether": true },
            "InlineCode": { "fontFamily": mono, "background": "#F0F1F3" },
            "List": { "spaceAfter": 6, "listIndent": "2em", "listHanging": "2em" },
            "Table": { "border": "0.5pt solid #999999", "cellPadding": 4, "repeatHeader": true, "spaceAfter": 8 },
            "Table.header": { "bold": true, "background": "#F3F3F3" },
            "Image": { "spaceAfter": 6 },
            "Image.caption": { "fontSize": 9, "color": "#666666" },
            "Slide.title": { "fontSize": 20, "bold": true },
            "Divider": { "borderBottom": "0.5pt solid #D0D7DE", "lineHeight": "1pt", "spaceBefore": 6, "spaceAfter": 12 },
            "Quote": { "borderLeft": "3pt solid #D0D7DE", "paddingLeft": 10, "color": "#57606A", "spaceAfter": 8 },
            "Math": { "align": "center", "spaceBefore": 6, "spaceAfter": 6 },
            "Callout": { "background": "#F6F8FA", "border": "0.5pt solid #E1E4E8", "padding": 8, "spaceAfter": 8 },
            "TableOfContents": { "spaceAfter": 12 },
            "TableOfContents.title": { "fontSize": 14, "bold": true, "spaceAfter": 6 },
            "TableOfContents.entry": { "spaceAfter": 2, "listIndent": "1.5em" }
        });
        raw.as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), style::parse_object(v).expect("内置样式应当合法")))
            .collect()
    })
    .get(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_parse() {
        assert!(builtin("Code").is_some());
        assert!(builtin("$global").is_some());
    }
}
