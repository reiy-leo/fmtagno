//! fmtagno：一份 JSON，按组件目录渲染成 docx / pdf / html / Markdown。

pub mod assets;
pub mod cascade;
pub mod components;
pub mod custom;
pub mod doc;
pub mod math;
pub mod render;
pub mod style;
pub mod units;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

/// 文档以外的东西：自定义组件、样式集文件、命令行选的样式集。
#[derive(Debug, Default, Clone)]
pub struct Env {
    pub catalog: custom::Catalog,
    pub style_sets: BTreeMap<String, cascade::StyleSetDef>,
    /// 命令行 --style-set，优先于文档的 styleSet
    pub style_set: Option<String>,
}

/// 解析、展开自定义组件、算样式、校验，得到可以渲染的文档和元素树。
/// 文档自己 `components` 里写的组件会合进 `env.catalog`。
pub fn prepare(json: &str, env: &Env) -> Result<(doc::Doc, doc::Node)> {
    let mut d = doc::parse(json)?;
    let inline = std::mem::take(&mut d.components);
    let mut all = env.catalog.clone();
    for def in inline {
        let name = def.name.clone();
        all.add(def)
            .with_context(|| format!("文档里的组件定义 `{name}` 有问题"))?;
    }
    all.expand(&mut d)?;
    d.custom_names = all.defs().map(|x| x.name.clone()).collect();
    let sheet = cascade::Sheet::build(&d, &env.style_sets, env.style_set.as_deref())?;
    let tree = d.tree_with(&sheet)?;
    Ok((d, tree))
}

/// 检查文档里的公式（Math 组件和 runs 里的 InlineMath）能不能解析，返回警告。
/// 解析不了的公式渲染时会改成显示 LaTeX 原文，不算错误。
pub fn check_math(tree: &doc::Node) -> Vec<String> {
    fn walk(n: &doc::Node, out: &mut Vec<String>) {
        let mut c = n.component.clone();
        let mut all: Vec<(String, bool)> = Vec::new();
        if let components::Component::Math(p) = &c {
            all.push((p.latex.clone(), true));
        }
        for t in c.inlines_mut() {
            all.extend(
                t.runs()
                    .filter(|r| r.is_math())
                    .map(|r| (r.text.clone(), false)),
            );
        }
        for (latex, block) in all {
            if let Err(e) = math::to_mathml(&latex, block) {
                out.push(format!(
                    "元素 `{}` 里的公式 `{latex}` 解析不了（{e}），渲染时会显示 LaTeX 原文",
                    n.id
                ));
            }
        }
        for k in &n.children {
            walk(k, out);
        }
    }
    let mut out = Vec::new();
    walk(tree, &mut out);
    out
}

/// 从 JSON 文本直接渲染，命令行和 MCP 都走这里。
/// `base_dir` 是文档所在目录，用来找相对路径的图片；没有就按当前目录找。
pub fn render_with(
    json: &str,
    format: render::Format,
    base_dir: Option<&Path>,
    env: &Env,
) -> Result<render::Output> {
    let (d, tree) = prepare(json, env)?;
    render::render(
        &d,
        &tree,
        format,
        &render::Options {
            base_dir: base_dir.map(Into::into),
            ..Default::default()
        },
    )
}

/// 不带外部组件和样式集的 [`render_with`]。
pub fn render_str(
    json: &str,
    format: render::Format,
    base_dir: Option<&Path>,
) -> Result<render::Output> {
    render_with(json, format, base_dir, &Env::default())
}

/// 读文档以外的定义：文档目录下的 `catalog/`、`styles/`（有的话），再读命令行指定的。
pub fn load_env(
    doc_dir: Option<&Path>,
    catalogs: &[PathBuf],
    style_sets: &[PathBuf],
    style_set: Option<String>,
) -> Result<Env> {
    let mut env = Env {
        style_set,
        ..Env::default()
    };
    if let Some(dir) = doc_dir {
        let c = dir.join("catalog");
        if c.is_dir() {
            env.catalog.load_path(&c)?;
        }
        let s = dir.join("styles");
        if s.is_dir() {
            cascade::load_style_sets(&s, &mut env.style_sets)?;
        }
    }
    for p in catalogs {
        env.catalog.load_path(p)?;
    }
    for p in style_sets {
        cascade::load_style_sets(p, &mut env.style_sets)?;
    }
    Ok(env)
}
