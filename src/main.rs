use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use fmtagno::{components, render};

#[derive(Parser)]
#[command(
    name = "fmtagno",
    version,
    about = "把一份 JSON 渲染成 docx / pdf / html / Markdown"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// 文档以外的定义。
#[derive(clap::Args)]
struct Defs {
    /// 自定义组件的定义文件或目录（JSON），可写多次；文档目录下的 catalog/ 会自动读
    #[arg(long)]
    catalog: Vec<PathBuf>,
    /// 样式集文件或目录（JSON），可写多次；文档目录下的 styles/ 会自动读
    #[arg(long)]
    style_sets: Vec<PathBuf>,
    /// 用哪个样式集，优先于文档里的 styleSet
    #[arg(long)]
    style_set: Option<String>,
}

impl Defs {
    fn env(self, input: &Path) -> Result<fmtagno::Env> {
        fmtagno::load_env(
            doc_dir(input).as_deref(),
            &self.catalog,
            &self.style_sets,
            self.style_set,
        )
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// 渲染文档：fmtagno render doc.json -o out.html（输入写 - 表示从标准输入读）
    Render {
        input: PathBuf,
        /// 输出文件，按扩展名选格式；写 - 表示输出到标准输出，这时要配 --format
        #[arg(short, long)]
        output: PathBuf,
        /// 指定格式（html、docx、pdf、md），不写就看输出文件的扩展名
        #[arg(short, long)]
        format: Option<String>,
        /// 只对 HTML 有用：内嵌 Paged.js，打开就按页显示（分页、表头重复不用打印也看得到）
        #[arg(long)]
        paged: bool,
        #[command(flatten)]
        defs: Defs,
    },
    /// 只校验，不出文件
    Check {
        input: PathBuf,
        #[command(flatten)]
        defs: Defs,
    },
    /// 列出可用组件；加 --json 输出每个组件 props 的 JSON Schema
    Catalog {
        #[arg(long)]
        json: bool,
        /// 一起列出这些定义文件或目录里的自定义组件
        #[arg(long)]
        catalog: Vec<PathBuf>,
    },
    /// 以 MCP server 方式运行（stdio），还没实现
    Mcp,
}

fn read_input(p: &Path) -> Result<String> {
    if p == Path::new("-") {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        Ok(s)
    } else {
        std::fs::read_to_string(p).with_context(|| format!("读不了 {}", p.display()))
    }
}

/// 文档所在目录；从标准输入读时没有。
fn doc_dir(input: &Path) -> Option<PathBuf> {
    if input == Path::new("-") {
        None
    } else {
        input.parent().map(Path::to_path_buf)
    }
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Render {
            input,
            output,
            format,
            paged,
            defs,
        } => {
            let fmt = match &format {
                Some(f) => render::Format::from_name(&f.to_ascii_lowercase())?,
                None if output == Path::new("-") => bail!("输出到标准输出时要用 --format 指定格式"),
                None => render::Format::from_path(&output)?,
            };
            if paged && !matches!(fmt, render::Format::Html) {
                eprintln!("警告：--paged 只对 HTML 有用，已忽略");
            }
            let text = read_input(&input)?;
            let base_dir = doc_dir(&input);
            let env = defs.env(&input)?;
            let (d, tree) = fmtagno::prepare(&text, &env)
                .with_context(|| format!("{} 不是合法的 fmtagno 文档", input.display()))?;
            let out = render::render(&d, &tree, fmt, &render::Options { base_dir, paged })?;
            for w in &out.warnings {
                eprintln!("警告：{w}");
            }
            if output == Path::new("-") {
                std::io::stdout().write_all(&out.bytes)?;
            } else {
                std::fs::write(&output, &out.bytes)
                    .with_context(|| format!("写不了 {}", output.display()))?;
                eprintln!("已生成 {}", output.display());
            }
            Ok(())
        }
        Cmd::Check { input, defs } => {
            let text = read_input(&input)?;
            let env = defs.env(&input)?;
            let (d, _) = fmtagno::prepare(&text, &env)
                .with_context(|| format!("{} 校验没通过", input.display()))?;
            println!("ok：展开后 {} 个元素", d.elements.len());
            Ok(())
        }
        Cmd::Catalog { json, catalog } => {
            let cat = fmtagno::load_env(None, &catalog, &[], None)?.catalog;
            if json {
                let mut m = serde_json::Map::new();
                for s in components::BUILTINS {
                    m.insert(
                        s.name.into(),
                        serde_json::json!({
                            "description": s.description,
                            "container": s.container,
                            "props": (s.schema)(),
                        }),
                    );
                }
                for d in cat.defs() {
                    m.insert(
                        d.name.clone(),
                        serde_json::json!({
                            "description": d.description,
                            "container": d.container,
                            "custom": true,
                            "props": d.props_schema(),
                        }),
                    );
                }
                println!("{}", serde_json::to_string_pretty(&m)?);
            } else {
                for s in components::BUILTINS {
                    println!(
                        "{:<10} {}{}",
                        s.name,
                        s.description,
                        if s.container { "（容器）" } else { "" }
                    );
                }
                for d in cat.defs() {
                    println!(
                        "{:<10} {}（自定义{}）",
                        d.name,
                        d.description,
                        if d.container { "，容器" } else { "" }
                    );
                }
            }
            Ok(())
        }
        Cmd::Mcp => bail!("MCP server 还没实现"),
    }
}
