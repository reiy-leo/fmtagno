<p align="center"><img src="docs/logo.png" width="128" alt="fmtagno"></p>

# fmtagno

一份 JSON，出 Word、PDF 和网页。

JSON 里只引用注册好的组件（标题、正文、列表、表格、图片……），再带一点样式。fmtagno 负责把它排成 docx、pdf 或 html。AI 也能通过 MCP 直接调用。

> 还在开发，第一版没发布。

```sh
fmtagno render report.json -o report.docx
fmtagno render report.json -o report.pdf
fmtagno render report.json -o report.html --paged   # 屏幕上按页显示
fmtagno check report.json
fmtagno render report.json -o report.html --catalog my-components/ --style-sets themes/ --style-set formal
fmtagno mcp
```

文档格式和计划见 [docs/PRODUCT.md](docs/PRODUCT.md)。

## 许可证

MIT
