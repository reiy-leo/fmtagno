//! 自定义组件：用 JSON 定义文件把内置组件组合起来。
//!
//! 渲染前先把自定义组件展开成内置组件，渲染器只认内置组件。
//!
//! 展开规则：
//! - 实例的 id 留给模板的根元素，父元素的 children 不用改；模板里其他元素的 id 变成 `实例id.模板id`。
//! - 字符串正好是 `{{prop}}` 时，整个换成 prop 的值（可以是数组、对象，比如 runs）；
//!   字符串里夹着 `{{prop}}` 时，按文字替换，这时 prop 只能是字符串、数字或布尔。
//! - 容器组件在模板某个元素的 children 里写 `$children`，实例的子元素放到那里。
//! - 模板里的 style、parts 是组件默认值：样式集和顶层 styles 能覆盖它，实例上写的 style、parts 优先级最高。

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    components,
    doc::{Doc, Element, Origin},
};

/// 展开最多套几层，超过就当成环。
const MAX_DEPTH: usize = 32;
const CHILDREN_SLOT: &str = "$children";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Def {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub props: BTreeMap<String, PropDef>,
    /// 能不能有 children；能的话模板里要有 `$children`
    #[serde(default)]
    pub container: bool,
    pub template: Template,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PropDef {
    #[serde(rename = "type", default)]
    pub kind: PropType,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PropType {
    #[default]
    String,
    Number,
    Integer,
    Boolean,
    Array,
    Object,
    /// 纯字符串或 runs 数组，和内置组件的文字 props 一样
    Inline,
    Any,
}

impl PropType {
    fn accepts(self, v: &Value) -> bool {
        match self {
            PropType::String => v.is_string(),
            PropType::Number => v.is_number(),
            PropType::Integer => v.is_i64() || v.is_u64(),
            PropType::Boolean => v.is_boolean(),
            PropType::Array => v.is_array(),
            PropType::Object => v.is_object(),
            PropType::Inline => v.is_string() || v.is_array(),
            PropType::Any => true,
        }
    }

    fn schema(self) -> Value {
        match self {
            PropType::String => json!({ "type": "string" }),
            PropType::Number => json!({ "type": "number" }),
            PropType::Integer => json!({ "type": "integer" }),
            PropType::Boolean => json!({ "type": "boolean" }),
            PropType::Array => json!({ "type": "array" }),
            PropType::Object => json!({ "type": "object" }),
            PropType::Inline => json!({ "anyOf": [{ "type": "string" }, { "type": "array" }] }),
            PropType::Any => json!({}),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub root: String,
    /// 和文档的 elements 一样；先按 JSON 值存，展开时替换完再解析
    pub elements: BTreeMap<String, Value>,
}

impl Def {
    /// 定义本身的检查：名字、模板结构、`{{prop}}` 引用、`$children`。
    pub fn validate(&self) -> Result<()> {
        let n = &self.name;
        if n.is_empty() || !n.chars().next().unwrap().is_ascii_uppercase() {
            bail!("组件名 `{n}` 要以大写字母开头");
        }
        if !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            bail!("组件名 `{n}` 只能用字母、数字和下划线");
        }
        if components::spec(n).is_some() {
            bail!("`{n}` 和内置组件重名");
        }
        if !self.template.elements.contains_key(&self.template.root) {
            bail!("`{n}` 的模板里没有根元素 `{}`", self.template.root);
        }
        for (k, p) in &self.props {
            if let Some(d) = &p.default
                && !p.kind.accepts(d)
            {
                bail!("`{n}` 的 prop `{k}`：默认值类型和 type 对不上");
            }
        }
        let mut slots = 0;
        for (id, el) in &self.template.elements {
            if id.contains('.') {
                bail!("`{n}` 的模板元素 id `{id}` 不能带点号");
            }
            for name in placeholders(el) {
                if !self.props.contains_key(&name) {
                    bail!("`{n}` 的模板元素 `{id}` 引用了没定义的 prop `{name}`");
                }
            }
            if let Some(ch) = el.get("children").and_then(Value::as_array) {
                for c in ch {
                    match c.as_str() {
                        Some(CHILDREN_SLOT) => slots += 1,
                        Some(c) if self.template.elements.contains_key(c) => {}
                        Some(c) => bail!("`{n}` 的模板元素 `{id}` 的子元素 `{c}` 不在模板里"),
                        None => bail!("`{n}` 的模板元素 `{id}` 的 children 只能写字符串"),
                    }
                }
            }
        }
        match (self.container, slots) {
            (true, 1) | (false, 0) => Ok(()),
            (true, 0) => bail!("`{n}` 是容器，模板里要在某个 children 里写一次 `$children`"),
            (true, _) => bail!("`{n}` 的 `$children` 只能写一次"),
            (false, _) => {
                bail!("`{n}` 不是容器，模板里不能写 `$children`；要用就加 `container: true`")
            }
        }
    }

    /// 给 `fmtagno catalog --json` 用的 props JSON Schema。
    pub fn props_schema(&self) -> Value {
        let mut props = serde_json::Map::new();
        let mut required = Vec::new();
        for (k, p) in &self.props {
            let mut s = p.kind.schema();
            if let Some(d) = &p.description {
                s["description"] = json!(d);
            }
            if let Some(d) = &p.default {
                s["default"] = d.clone();
            }
            if p.required {
                required.push(json!(k));
            }
            props.insert(k.clone(), s);
        }
        json!({
            "type": "object",
            "properties": props,
            "required": required,
            "additionalProperties": false,
        })
    }

    /// 检查实例的 props，补上默认值。
    fn resolve_props(&self, id: &str, props: &Value) -> Result<BTreeMap<String, Value>> {
        let given = match props {
            Value::Null => serde_json::Map::new(),
            Value::Object(m) => m.clone(),
            _ => bail!("元素 `{id}` 的 props 要是对象"),
        };
        for k in given.keys() {
            if !self.props.contains_key(k) {
                bail!("元素 `{id}`：{} 没有 prop `{k}`", self.name);
            }
        }
        let mut out = BTreeMap::new();
        for (k, p) in &self.props {
            match given.get(k).or(p.default.as_ref()) {
                Some(v) => {
                    if !p.kind.accepts(v) {
                        bail!("元素 `{id}`：prop `{k}` 要是 {:?} 类型", p.kind);
                    }
                    out.insert(k.clone(), v.clone());
                }
                None if p.required => bail!("元素 `{id}`：缺少必填的 prop `{k}`"),
                None => {}
            }
        }
        Ok(out)
    }
}

/// 一组自定义组件。
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    defs: BTreeMap<String, Def>,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn defs(&self) -> impl Iterator<Item = &Def> {
        self.defs.values()
    }

    pub fn get(&self, name: &str) -> Option<&Def> {
        self.defs.get(name)
    }

    pub fn add(&mut self, def: Def) -> Result<()> {
        def.validate()?;
        if self.defs.contains_key(&def.name) {
            bail!("组件 `{}` 定义了不止一次", def.name);
        }
        self.defs.insert(def.name.clone(), def);
        Ok(())
    }

    /// 读一个文件或目录（目录里的 .json，不进子目录）。
    pub fn load_path(&mut self, path: &Path) -> Result<()> {
        if path.is_dir() {
            let mut files: Vec<PathBuf> = std::fs::read_dir(path)
                .with_context(|| format!("读不了目录 {}", path.display()))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_file() && is_json(p))
                .collect();
            files.sort();
            for f in files {
                self.load_file(&f)?;
            }
            Ok(())
        } else {
            self.load_file(path)
        }
    }

    fn load_file(&mut self, path: &Path) -> Result<()> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("读不了 {}", path.display()))?;
        let def =
            parse_def(&text).with_context(|| format!("{} 不是合法的组件定义", path.display()))?;
        self.add(def)
            .with_context(|| format!("{} 里的组件定义有问题", path.display()))
    }

    /// 把文档里所有自定义组件展开成内置组件。
    pub fn expand(&self, doc: &mut Doc) -> Result<()> {
        for depth in 0.. {
            let pending: Vec<String> = doc
                .elements
                .iter()
                .filter(|(_, el)| self.defs.contains_key(&el.kind))
                .map(|(id, _)| id.clone())
                .collect();
            if pending.is_empty() {
                return Ok(());
            }
            if depth >= MAX_DEPTH {
                bail!("自定义组件套了 {MAX_DEPTH} 层还没展开完，可能互相引用成环了：{pending:?}");
            }
            for id in pending {
                self.expand_one(doc, &id)?;
            }
        }
        unreachable!()
    }

    fn expand_one(&self, doc: &mut Doc, id: &str) -> Result<()> {
        let el = doc.elements.remove(id).expect("pending 里的 id 一定存在");
        let def = &self.defs[&el.kind];
        let props = def
            .resolve_props(id, &el.props)
            .with_context(|| format!("展开 `{id}`（{}）时出错", def.name))?;
        if !def.container && !el.children.is_empty() {
            bail!("元素 `{id}`：{} 不能有 children", def.name);
        }
        let tpl = &def.template;
        let new_id = |t: &str| {
            if t == tpl.root {
                id.to_string()
            } else {
                format!("{id}.{t}")
            }
        };
        for (tid, raw) in &tpl.elements {
            let mut v = substitute(raw, &props)
                .with_context(|| format!("展开 `{id}`（{}）的模板元素 `{tid}` 时出错", def.name))?;
            if let Some(ch) = v.get_mut("children").and_then(Value::as_array_mut) {
                let mut out = Vec::new();
                for c in ch.iter() {
                    match c.as_str() {
                        Some(CHILDREN_SLOT) => out.extend(el.children.iter().map(|c| json!(c))),
                        Some(c) => out.push(json!(new_id(c))),
                        None => unreachable!("定义检查过 children 都是字符串"),
                    }
                }
                *ch = out;
            }
            // 模板里写的 style、parts 算组件默认值，优先级低于样式集；实例上写的才是元素自己的
            let tpl_style = v.as_object_mut().and_then(|m| m.remove("style"));
            let tpl_parts = v.as_object_mut().and_then(|m| m.remove("parts"));
            let mut element: Element = serde_json::from_value(v).with_context(|| {
                format!("展开 `{id}`（{}）后，模板元素 `{tid}` 的结构不对", def.name)
            })?;
            element
                .template_styles
                .extend(tpl_style.filter(|x| !x.is_null()));
            if let Some(Value::Object(m)) = tpl_parts {
                element.template_parts.push(m.into_iter().collect());
            }
            element.origins.push(Origin {
                component: def.name.clone(),
                element: tid.clone(),
                root: *tid == tpl.root,
            });
            if *tid == tpl.root {
                // 外层模板给这个实例写的样式，排在内层模板之后
                element
                    .template_styles
                    .extend(el.template_styles.iter().cloned());
                element
                    .template_parts
                    .extend(el.template_parts.iter().cloned());
                element.origins.extend(el.origins.iter().cloned());
                element.style = el.style.clone();
                element.parts = el.parts.clone();
            }
            let nid = new_id(tid);
            if doc.elements.contains_key(&nid) {
                bail!("展开 `{id}` 时生成的元素 id `{nid}` 和文档里已有的重复了");
            }
            doc.elements.insert(nid, element);
        }
        Ok(())
    }
}

/// 解析一份组件定义（JSON）。
pub fn parse_def(text: &str) -> Result<Def> {
    Ok(serde_json::from_str(text)?)
}

fn is_json(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
}

/// 找出值里所有 `{{name}}` 的 name。
fn placeholders(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    walk_strings(v, &mut |s| {
        let mut rest = s;
        while let Some(start) = rest.find("{{") {
            let after = &rest[start + 2..];
            match after.find("}}") {
                Some(end) => {
                    out.push(after[..end].trim().to_string());
                    rest = &after[end + 2..];
                }
                None => break,
            }
        }
    });
    out
}

fn walk_strings(v: &Value, f: &mut impl FnMut(&str)) {
    match v {
        Value::String(s) => f(s),
        Value::Array(a) => a.iter().for_each(|x| walk_strings(x, f)),
        Value::Object(m) => m.values().for_each(|x| walk_strings(x, f)),
        _ => {}
    }
}

/// 把 `{{prop}}` 换成实例的值。没给值的可选 prop 整个替换时变成 null，文字替换时变成空串。
fn substitute(v: &Value, props: &BTreeMap<String, Value>) -> Result<Value> {
    Ok(match v {
        Value::String(s) => {
            let t = s.trim();
            if let Some(name) = t.strip_prefix("{{").and_then(|x| x.strip_suffix("}}"))
                && !name.contains("{{")
                && !name.contains("}}")
            {
                return Ok(props.get(name.trim()).cloned().unwrap_or(Value::Null));
            }
            let mut out = String::new();
            let mut rest = s.as_str();
            while let Some(start) = rest.find("{{") {
                out.push_str(&rest[..start]);
                let after = &rest[start + 2..];
                let Some(end) = after.find("}}") else {
                    out.push_str(&rest[start..]);
                    rest = "";
                    break;
                };
                let name = after[..end].trim();
                match props.get(name) {
                    None | Some(Value::Null) => {}
                    Some(Value::String(x)) => out.push_str(x),
                    Some(x @ (Value::Number(_) | Value::Bool(_))) => out.push_str(&x.to_string()),
                    Some(_) => bail!(
                        "prop `{name}` 是数组或对象，不能夹在文字里，要单独写成 \"{{{{{name}}}}}\""
                    ),
                }
                rest = &after[end + 2..];
            }
            out.push_str(rest);
            Value::String(out)
        }
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|x| substitute(x, props))
                .collect::<Result<_>>()?,
        ),
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, x)| Ok((k.clone(), substitute(x, props)?)))
                .collect::<Result<_>>()?,
        ),
        other => other.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_whole_and_inline() {
        let mut p = BTreeMap::new();
        p.insert("t".to_string(), json!("标题"));
        p.insert("n".to_string(), json!(3));
        p.insert("runs".to_string(), json!([{ "text": "a", "bold": true }]));
        assert_eq!(
            substitute(&json!("{{runs}}"), &p).unwrap(),
            json!([{ "text": "a", "bold": true }])
        );
        assert_eq!(
            substitute(&json!("第 {{n}} 章：{{t}}"), &p).unwrap(),
            json!("第 3 章：标题")
        );
        assert_eq!(substitute(&json!("{{missing}}"), &p).unwrap(), Value::Null);
        assert!(substitute(&json!("x {{runs}}"), &p).is_err());
    }

    #[test]
    fn finds_placeholders() {
        let v = json!({ "a": "{{x}} and {{ y }}", "b": ["{{z}}"] });
        let mut got = placeholders(&v);
        got.sort();
        assert_eq!(got, ["x", "y", "z"]);
    }
}
