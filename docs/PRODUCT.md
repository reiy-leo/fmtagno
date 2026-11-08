# fmtagno 产品说明

fmtagno 把一份 JSON 文档渲染成 docx、pdf、html，也能出 Markdown。JSON 里不写排版细节，只引用提前注册好的组件，再带一点样式。思路来自 [vercel-labs/json-render](https://github.com/vercel-labs/json-render)。

它是本地命令行工具，单个 Rust 二进制，另带一个 MCP server 给 AI 调用。通过 Homebrew 安装。

## 给谁用

- 想用一份数据同时出 Word、PDF 和网页的人。
- 想让 AI 直接产出办公文档的人：AI 只管写 JSON，fmtagno 负责排版。

## 第一版（v0.1）

- 格式：docx、pdf、html，外加 Markdown（GFM，只留结构不留样式，有合并格的表格写成内嵌 HTML，提示框写成 `> [!NOTE]`）。
- 内置组件：Heading、Text、Code、Math（块级公式）、List、Table、Image、PageBreak、ColumnBreak（分栏符）、Divider（分割线）、Quote（引用）、Callout（提示框）、TableOfContents（目录）、Slide、Section（分节），外加根组件 Document；行内有 InlineCode 和 InlineMath（行内公式），都写在 runs 里。
- 命令行：
  - `fmtagno render doc.json -o out.docx`：按输出文件的扩展名选格式。
  - `fmtagno check doc.json`：只校验，不出文件。
  - `fmtagno catalog`：列出能用的组件，加 `--json` 输出 schema。
  - `fmtagno parse doc.docx -o doc.json`：反过来，从 Word 文档解析出 JSON，图片导到 `doc.assets/`。见下面「从 docx 解析」。
  - 输入写 `-` 从标准输入读；输出写 `-` 时配 `--format` 写到标准输出。
  - `fmtagno mcp`：以 MCP server 方式运行（stdio）。
- MCP 工具：`render`（JSON 进，文件出）、`check`、`list_components`（带每个组件的 props 说明，方便 AI 写对）。
- 自定义组件：JSON 定义文件，用内置组件拼。

## 第二版

- xlsx、pptx。
- Slide 组件在 html 和 pdf 里按“一页一张”显示，到第二版再出真正的 pptx。

## 不做

- 不把 pdf 解析回 JSON。docx 能解析，范围见「从 docx 解析」。
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

文字类的 props（Heading 的 `text`、Text 的 `text`、List 的每一项、表格的每一格）都能写成纯字符串，也能写成 runs 数组。数组里可以直接写字符串，也可以写对象：`bold`、`italic`、`underline`、`color`、`href` 是简写，`style` 能写全部文字样式；`"type": "InlineCode"` 表示行内代码，`"type": "InlineMath"` 表示行内公式（text 写 LaTeX）。块级公式用 Math 组件，写法见 [STYLE.md](STYLE.md) 第十四节。列表项可以写成 `{ "text": …, "items": [...] }` 带子列表。

`meta.page` 管纸张：`size`（默认 A4，也可以写 `{ "width", "height" }`）、`landscape`、`margin` 和四个单边、`gutter`、`columns`，还有页眉页脚 `header`、`footer`，页码起始 `pageNumberStart` 和页码格式 `pageNumberFormat`（罗马数字、字母、中文数字）。要分节就在 root 下面放 `Section`，每节能单独改这些设置；runs 里写 `{ "field": "page" }` 显示页码。细节见 [STYLE.md](STYLE.md) 第十节。

样式字段、样式集（`styleSet`、`styleSets`）、部位样式和层叠顺序都在 [STYLE.md](STYLE.md)。字段名跟 CSS 走，取值以 OOXML 为准；某个格式表达不了的样式，渲染时忽略并给一次警告，不报错。

完整例子见 `examples/showcase.json`、`examples/styles.json`，分节和页眉页脚见 `examples/sections.json`，页码格式和分栏符见 `examples/frontmatter.json`。

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

公式在四种格式里的写法（细节见 STYLE.md 第十四节）：

- docx 写成 Word 自己的公式（OMML），在 Word 里能直接改。带编号的块级公式用制表位排，公式按行内大小显示。
- pdf 交给 Typst 排，字体用 Typst 自带的 New Computer Modern Math。
- html 写成 MathML，不用脚本也不连网。括号拉伸、大号求和符号要靠系统里的公式字体（macOS、Windows 自带；Linux 可能要装）。
- Markdown 写成 `$…$` 和 `$$…$$`，GitHub 能显示。
- LaTeX 写错了不会让整个文件出不来：给一条警告，这个公式改成显示 LaTeX 原文。

html 默认是一长条网页，分页和表头重复只在打印时出现。加 `--paged` 会把 Paged.js（MIT，0.4.3，已打包进二进制，离线能用）嵌进页面，打开就像 Google Docs 那样一页页显示：

- 按 `meta.page` 的纸张和页边距切页，PageBreak 后面的内容从新页开始。
- 表格跨页时续页顶上会再出现表头（`repeatHeader: false` 时不出现）。
- 显示页眉页脚，填好页码。排完版后 `<html>` 上会有 `data-fmtagno-ready="1"`，截图或打印的脚本可以等它。
- 屏幕上横竖混排没问题，打印时只用第一节的纸张。
- 文件会大 500 KB 左右，打开时要等脚本排完版。

## 从 docx 解析

`fmtagno parse` 把一份 .docx 读成 fmtagno 的 JSON，格式和 render 的输入一样（扁平 elements 加 root）。拿到 JSON 后可以改内容、换样式集，再 render 成 docx、pdf、html。

```sh
fmtagno parse report.docx -o report.json      # 图片导到 report.assets/
fmtagno parse report.docx > report.json       # 不写 -o 时输出到标准输出，图片导到当前目录的 report.assets/
fmtagno parse report.docx -o out/report.json --assets img   # 图片导到 out/img/
```

目标是 parse 完再 render 回 docx，看起来基本一样。认不出、表达不了的内容在标准错误里给警告，不会中断。

能解析的：

- 标题：段落样式是「标题 1」到「标题 6」（或样式里写了大纲级别）的段落，变成 Heading。
- 正文和行内样式：字体、字号、颜色、粗体、斜体、下划线、删除线、上下标、大小写、字间距、缩放、高亮、底纹、着重号、隐藏；链接（超链接和 HYPERLINK 域）；行内代码（字符样式叫 InlineCode 的）。
- 段落格式：对齐、段前段后、行距、缩进（按字符计的写成 em）、与下段同页、段中不分页、段前分页、孤行控制、边框和底纹。
- 列表：numbering.xml 里的编号变成 List，有序无序、嵌套、每一级的编号类型、编号模板、起始编号、缩进、编号颜色和字号。
- 表格：合并格（gridSpan、vMerge）、列宽、行高、每格的底纹、边框、对齐、竖排、边距；表头按 Word 的「重复标题行」（w:tblHeader）认：第一行设了它就变成表头 columns，原来没底纹的写成 `parts.header` 的 `"background": "none"`，有底纹的照写；没设的第一行当普通行。好几行都设了时只有第一行当表头，给警告。
- 图片：行内图片和单独一段的图片（Image），图片文件导到 JSON 旁边的目录，JSON 里写相对路径；浮动图片按位置写成 float 或 absolute。替代文字从 wp:docPr 的 descr 读，没有 descr 时用 title；Image 写成 `alt`，行内图片写成 `text`。
- 分节：纸张大小和方向、页边距、页眉页脚距离、分栏、首页不同、奇偶页不同、页码起始值、页码格式（decimal、罗马数字、字母、chineseCounting；别的格式给警告，按阿拉伯数字处理）。第一节写在 `meta.page`，后面每节一个 Section。
- 页眉页脚、页码（PAGE、NUMPAGES、SECTIONPAGES 域）、分页符、分栏符（变回 ColumnBreak）、目录（TOC 域变回 TableOfContents）。
- 边框：Word 的 25 种线型都换回 STYLE.md 里的名字，`single` 写成 `solid`。
- 公式：OMML 转成 LaTeX，单独一段的变成 Math（带编号的也认），夹在文字里的变成 InlineMath。认不出的结构给警告，只留文字。

样式是这样处理的：

- 先按 Word 的规则算出每段、每个 run 实际用的格式，再只写和「不写时会是什么」不一样的字段，所以 JSON 不会每段都带一大串样式。
- 大多数正文段落共用的字体、字号、行距写进 `styles` 的 `Document`；正文的段前段后、首行缩进写进 `Text`；各级标题写进 `Heading.1` 这些键。
- 其他用到的 Word 段落样式和字符样式写成同名的命名样式（和组件名重名时前面加 `Word `），元素用 `style` 引用。
- 字体一律写英文族名：宋体写成 SimSun，微软雅黑写成 Microsoft YaHei，主题字体换成主题里的实际字体。

还不支持、会给警告的：

- 文本框、形状、艺术字、图表、SmartArt；嵌入对象（比如 MathType 公式）只留预览图。
- 脚注、尾注、批注、修订（已删除的文字不要，插入的文字照常保留）。
- 制表位（制表符原样留在文字里）、页面垂直对齐、连续分节；页码格式只认上面那六种。
- 表格样式里的条件格式（首行、隔行这些），表格整体居中，格子斜线，表格里套的表格（文字接在格子里）。
- Word 的「多倍行距」按 fmtagno 的倍数（字号 × 倍数）理解，渲染回去行距会比原文档紧一点。
- 渲染器本来就表达不了的东西读回来也一样：比如同一级列表在 Word 里只有一种编号定义。

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
| LaTeX 转 MathML | math-core | 0.9.0（2026-11-09 核对） |
| 读 MathML | roxmltree | 0.21 |

- PDF 走 Typst：先把文档转成 Typst 源码，再编译成 PDF。中文排版好，不用装 LibreOffice。字体要打包进二进制，或者读系统字体。
- HTML 自己拼字符串，不引模板引擎。
- 公式只用一个解析器：math-core（MIT）把 LaTeX 转成 MathML，html 直接用；docx 和 pdf 再从 MathML 转成 OMML 和 Typst 公式，这两步是自己写的。没用 mitex（LaTeX 转 Typst）：它转出来的东西要配 mitex 的 Typst 包才能排，离线用不了，而且 2024 年以后没再更新。
- 只支持 JSON，不支持 YAML（Rei 2026-10-28 定）：AI 生成 JSON 最稳，也少一个依赖。

## 发布

- cargo-dist 出 GitHub Release，同时生成 Homebrew formula，推到自建 tap `reiy-leo/homebrew-tap`。
- 预编译 macOS Intel、macOS Apple Silicon、Linux x86_64。Rei 的 Mac 是 2018 年的 Intel 机器，不能让它从源码编译。
- 每次发版前先问 Rei。
