# fmtagno 产品说明

fmtagno 把一份 JSON 文档渲染成 docx、pdf、html，也能出 Markdown。JSON 里不写排版细节，只引用提前注册好的组件，再带一点样式。思路来自 [vercel-labs/json-render](https://github.com/vercel-labs/json-render)。

它是本地命令行工具，单个 Rust 二进制，另带一个 MCP server 给 AI 调用。通过 Homebrew 安装。

## 给谁用

- 想用一份数据同时出 Word、PDF 和网页的人。
- 想让 AI 直接产出办公文档的人：AI 只管写 JSON，fmtagno 负责排版。

## 第一版（v0.1）

- 格式：docx、pdf、html，外加 Markdown（GFM，只留结构不留样式，有合并格的表格写成内嵌 HTML，提示框写成 `> [!NOTE]`）。
- 内置组件：Heading、Text、Code、List、Table、Image、PageBreak、Divider（分割线）、Quote（引用）、Callout（提示框）、TableOfContents（目录）、Slide、Section（分节），外加根组件 Document；行内有 InlineCode（写在 runs 里）。
- 命令行：
  - `fmtagno render doc.json -o out.docx`：按输出文件的扩展名选格式。
  - `fmtagno check doc.json`：只校验，不出文件。
  - `fmtagno catalog`：列出能用的组件，加 `--json` 输出 schema。
  - 输入写 `-` 从标准输入读；输出写 `-` 时配 `--format` 写到标准输出。
  - `fmtagno mcp`：以 MCP server 方式运行（stdio）。
- MCP 工具：`render`（JSON 进，文件出）、`check`、`list_components`（带每个组件的 props 说明，方便 AI 写对）。
- 自定义组件：JSON 定义文件，用内置组件拼。

## 第二版

- xlsx、pptx。
- Slide 组件在 html 和 pdf 里按“一页一张”显示，到第二版再出真正的 pptx。

## 不做

- 不把 docx、pdf 解析回 JSON。
- 不做 Rust 插件。自定义组件只能拼内置组件。
- 不做图形界面，不做网站。
- 不依赖 LibreOffice 或 Word。

## 文档格式

结构沿用 json-render：所有元素平铺在 `elements` 里，用 id 互相引用，`root` 指向根元素。好处是 AI 可以一条一条地流式输出元素，也好做增量修改。

```json
{
  "version": "0.1",
  "meta": { "title": "季度报告", "lang": "zh-CN", "page": { "size": "A4", "margin": "20mm" } },
  "styles": { "muted": { "color": "#666666", "fontSize": 10 } },
  "root": "doc",
  "elements": {
    "doc":   { "type": "Document", "children": ["title", "intro"] },
    "title": { "type": "Heading", "props": { "level": 1, "text": "季度报告" } },
    "intro": { "type": "Text", "props": { "text": "一份 JSON，出三种格式。" }, "style": "muted" }
  }
}
```

- `type`：组件名，必须在组件目录里。`fmtagno catalog --json` 能看到每个组件 props 的 JSON Schema。
- `props`：组件自己的参数，多写、少写、写错类型都会报错。
- `style`：三种写法都行：样式名 `"muted"`；样式名数组 `["muted", "center"]`，后面的覆盖前面的；样式对象 `{ "fontSize": 11 }`。数组里也能混写对象。
- `parts`：部位样式，比如表格的 `header`、`evenRow`、`lastRow`，列表的 `level2`、`marker`。
- `children`：子元素 id 列表，只有容器组件（Document、Slide、Section）能用。每个元素只能被引用一次。

文字类的 props（Heading 的 `text`、Text 的 `text`、List 的每一项、表格的每一格）都能写成纯字符串，也能写成 runs 数组。数组里可以直接写字符串，也可以写对象：`bold`、`italic`、`underline`、`color`、`href` 是简写，`style` 能写全部文字样式；`"type": "InlineCode"` 表示行内代码。列表项可以写成 `{ "text": …, "items": [...] }` 带子列表。

`meta.page` 管纸张：`size`（默认 A4，也可以写 `{ "width", "height" }`）、`landscape`、`margin` 和四个单边、`gutter`、`columns`，还有页眉页脚 `header`、`footer` 和页码起始 `pageNumberStart`。要分节就在 root 下面放 `Section`，每节能单独改这些设置；runs 里写 `{ "field": "page" }` 显示页码。细节见 [STYLE.md](STYLE.md) 第十节。

样式字段、样式集（`styleSet`、`styleSets`）、部位样式和层叠顺序都在 [STYLE.md](STYLE.md)。字段名跟 CSS 走，取值以 OOXML 为准；某个格式表达不了的样式，渲染时忽略并给一次警告，不报错。

完整例子见 `examples/showcase.json`、`examples/styles.json`，分节和页眉页脚见 `examples/sections.json`。

## 各格式的差别

同一份 JSON 在三种格式里尽量长得一样。docx 有这些不同：

- 图片只收 png、jpeg、gif、webp。SVG 和远程图片放不进去，改成显示 alt 文字，并给警告。
- `padding` 只在有 `border` 时生效，用作边框和文字的间距。
- Slide 变成一个二级标题加正文，后面分页。
- 纸张、方向、页边距、分栏按每节的设置写；表格默认铺满版心；`widths` 可以写比例、固定宽度或 `"auto"`，没有比例列时表格按内容收窄（见 docs/STYLE.md 第九节）。
- 每节末尾会多一个空段落，用来放这一节的设置（Word 自己分节也是这样）。

pdf 用 Typst 排版，有这些不同：

- 中文用系统字体：macOS 上是苹方，Linux 上是 Noto Sans CJK。都没有时退回 Typst 自带字体，中文可能显示不全。
- 中文字体没有斜体，`italic` 对中文不起作用。
- 支持 SVG；远程图片暂不支持，改成显示 alt 文字。
- 颜色只认十六进制写法，比如 `#c00`、`#1a73e8`。

html 默认是一长条网页，分页和表头重复只在打印时出现。加 `--paged` 会把 Paged.js（MIT，0.4.3，已打包进二进制，离线能用）嵌进页面，打开就像 Google Docs 那样一页页显示：

- 按 `meta.page` 的纸张和页边距切页，PageBreak 后面的内容从新页开始。
- 表格跨页时续页顶上会再出现表头（`repeatHeader: false` 时不出现）。
- 显示页眉页脚，填好页码。排完版后 `<html>` 上会有 `data-fmtagno-ready="1"`，截图或打印的脚本可以等它。
- 屏幕上横竖混排没问题，打印时只用第一节的纸张。
- 文件会大 500 KB 左右，打开时要等脚本排完版。

## 自定义组件

自定义组件把内置组件组合起来，JSON 里像内置组件一样引用。渲染前先展开成内置组件，所以三个渲染器只认内置组件。

定义放在三个地方，都能用：

- 文档旁边的 `catalog/` 目录，自动读取。
- `--catalog` 指定的文件或目录，可以写多次。`render`、`check`、`catalog` 三个命令都认。
- 文档 JSON 里的 `components` 数组，适合 AI 通过 MCP 一次传完。

一个文件一个组件，用 JSON 写：

```json
{
  "name": "Notice",
  "description": "带标题的提示框",
  "container": true,
  "props": {
    "title": { "type": "inline", "required": true },
    "tone": { "type": "string", "default": "#1a73e8" }
  },
  "template": {
    "root": "box",
    "elements": {
      "box": { "type": "Document", "children": ["head", "$children"] },
      "head": {
        "type": "Text",
        "props": { "text": "{{title}}" },
        "style": { "bold": true, "color": "{{tone}}" }
      }
    }
  }
}
```

- `name`：大写字母开头，不能和内置组件重名
- `container`：能放子元素；模板里要写一次 $children

规则：

- prop 的 `type`：`string`（默认）、`number`、`integer`、`boolean`、`array`、`object`、`inline`（纯字符串或 runs 数组）、`any`。
- 字符串正好是 `"{{title}}"` 时，整个换成 prop 的值，数组、对象也行；夹在文字里时按文字替换，比如 `"第 {{n}} 章"`。没给值的可选 prop 换成空。
- 实例的 id 留给模板根元素，其他模板元素的 id 变成 `实例id.模板id`，比如 `tip.head`。
- 实例的 `style` 叠在模板根元素的 `style` 后面。
- 模板里可以用别的自定义组件，最多套 32 层，超过就当成环报错。
- 少写必填 prop、多写没定义的 prop、类型不对，都会报错。

完整例子见 `examples/custom.json` 和 `examples/catalog/`。

## 技术选型

版本在 2026-10-28 核对过 crates.io，动手加依赖前再看一次：

| 用途 | crate | 当时最新版 |
| --- | --- | --- |
| docx | docx-rs | 0.4.22 |
| pdf | typst（当库用） | 0.15.1 |
| xlsx（第二版） | rust_xlsxwriter | 0.101.0 |
| pptx（第二版） | ppt-rs | 0.2.37 |
| MCP | rmcp | 3.8.0 |
| 命令行 | clap | 4.6.7 |
| JSON Schema | schemars | 1.2.3 |

- PDF 走 Typst：先把文档转成 Typst 源码，再编译成 PDF。中文排版好，不用装 LibreOffice。字体要打包进二进制，或者读系统字体。
- HTML 自己拼字符串，不引模板引擎。
- 只支持 JSON，不支持 YAML（Rei 2026-10-28 定）：AI 生成 JSON 最稳，也少一个依赖。

## 发布

- cargo-dist 出 GitHub Release，同时生成 Homebrew formula，推到自建 tap `reiy-leo/homebrew-tap`。
- 预编译 macOS Intel、macOS Apple Silicon、Linux x86_64。Rei 的 Mac 是 2018 年的 Intel 机器，不能让它从源码编译。
- 每次发版前先问 Rei。
