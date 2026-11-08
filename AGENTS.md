# AGENTS.md

fmtagno：一份 JSON，按注册好的组件渲染成 docx、pdf、html（第二版加 xlsx、pptx）。本地 Rust 命令行加 MCP server，通过 Homebrew 发布。产品说明见 [docs/PRODUCT.md](docs/PRODUCT.md)。

## 边界

- 只做 JSON 到文档这一个方向，不解析回 JSON。
- 自定义组件只能用内置组件拼，不做 Rust 插件。
- 不依赖 LibreOffice、Word 等外部程序。

## 技术栈

- Rust 2024，单个二进制。
- docx-rs、typst（PDF）、自写 HTML；第二版加 rust_xlsxwriter、ppt-rs。
- clap 做命令行，rmcp 做 MCP。
- 发布用 cargo-dist，Homebrew tap 是 `reiy-leo/homebrew-tap`。

## 目录

```
src/main.rs         命令行入口
src/lib.rs          库入口，render_str 给命令行和 MCP 共用
src/doc.rs          文档模型、校验、建元素树
src/components.rs   内置组件的 props 和 schema
src/style.rs        样式字段和取值解析
src/cascade.rs      样式层叠：样式集、组件样式、部位、继承
src/custom.rs       自定义组件：定义、校验、展开
src/render/         各格式渲染器：html、docx、pdf（Typst）、markdown；common.rs 放列表编号、表格部位这类共用计算
tests/              集成测试
examples/       示例文档；examples/catalog/ 是示例组件定义
docs/           产品说明和设计文档
.agents/        给 AI 用的 skills、rules、prompts、memory
```

以后再加 `src/mcp.rs`。

## 构建环境

Hark 沙箱内存小，编译 typst、image 时会被杀。构建和测试放在 LightVela 服务器（`~/src/fmtagno`）上跑；看 docx 效果用那台的 WordCraft（`~/tools/wordcraft/wordcraft-0.15.0-linux-x86_64/bin/wordcraft-cli convert x.docx x.pdf --update-fields`，会重算页码等域），不用 LibreOffice；PDF 转图片用 poppler 的 `pdftoppm`。调 PDF 排版时设 `FMTAGNO_DEBUG_TYPST=1`，会把生成的 Typst 源码打到标准错误。

## 命令

```sh
cargo build
cargo run -- check examples/hello.json
cargo run -- render examples/showcase.json -o /tmp/showcase.html
cargo run -- catalog --json
cargo fmt && cargo clippy -- -D warnings
cargo test
```

## 规范

- 每加一个组件，同时写好三样：props 的 schema、三个渲染器的实现、`examples/` 里的用例。
- 渲染器只认内置组件。自定义组件在渲染前展开。
- 某个格式表达不了的样式，给警告，不报错。
- 报错信息用中文，带上出错的元素 id。
- 中文文档写得像人说话，不要翻译腔。
- 每轮工作做完就 commit 并 push 到 main。

## 禁止

- 不提交密钥、私钥、`.env`。
- 不自己发版、打 tag。发版前先问 Rei。
- 不在 Rei 的 Mac 上跑 `brew install` 或 `brew upgrade`，要先问。
- 不引入需要系统里另装程序的依赖。
