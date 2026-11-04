# 样式模型（草案）

> 状态：已实现（2026-10-28），代码见 `src/style.rs`（字段）、`src/cascade.rs`（层叠）、`src/render/`（各格式）。各格式表达不了的字段，渲染时给一次警告并忽略。
>
> 已定（Rei 2026-10-28）：字段平铺在 `style` 下；字段名跟 CSS 走；不做主题色。

fmtagno 的样式以 OOXML 为准：先把 Word、PowerPoint、Excel 能配的属性列全，再归并成一套字段。HTML 和 PDF（Typst）按这套字段去对应，表达不了的就忽略并给警告。

依据：ECMA-376 第 5 版 Part 1（WordprocessingML 第 17 章、SpreadsheetML 第 18 章、DrawingML 第 20–21 章）。

## 一、写法约定

- 字段名用 camelCase，和现在一致。
- 按 OOXML 的层级分组：文字（rPr）、段落（pPr）、边框、背景、盒子（表格单元格、文本框）、表格、页面。分组只用来写文档；JSON 里字段平铺在 `style` 下，不嵌套。
- 长度：数字就是 pt；字符串可以带单位：`pt`、`mm`、`cm`、`in`、`px`（按 96 dpi 折算），还有两个排版单位：
  - `em`：当前字号的倍数，`"2em"` 就是首行缩进两个字。Word 里对应 `firstLineChars` 这类按字符计的属性。
  - `line`：行数，只用在段前段后，`"0.5line"` 对应 Word 的 `beforeLines`。
- 颜色：`#RRGGBB`，以后加 `#RRGGBBAA`（只有 PPT、HTML、PDF 认透明度）。不做主题色名。
- 同一个属性分「全部」和「单边」两种写法，单边覆盖全部：`border` 和 `borderTop`，`padding` 和 `paddingLeft`。

### 单位换算（实现时用）

| OOXML 单位 | 用在哪 | 1pt 等于 |
|---|---|---|
| twip（1/20 pt） | w:spacing、w:ind、页边距、表格宽 | 20 |
| 半磅 | w:sz（字号） | 2 |
| 1/8 pt | w:pBdr、w:tcBorders 的 sz | 8 |
| 1/100 pt | a:rPr sz、a:spcPts | 100 |
| EMU | DrawingML 的长度、线宽 | 12700 |
| 1/240 行 | w:spacing line（lineRule=auto） | — |
| 1/1000 % | a:spcPct、a:lnSpc | 100% = 100000 |

## 二、文字（对应 w:rPr、a:rPr、xlsx font）

| 字段 | 值 | Word w:rPr | PPT a:rPr | Excel font | HTML | PDF |
|---|---|---|---|---|---|---|
| `fontFamily` | 字符串，或按文种分开的对象（见下） | w:rFonts | a:latin / a:ea / a:cs | name（只有一个） | font-family | font |
| `fontSize` | 长度 | w:sz + w:szCs | sz | sz | font-size | size |
| `color` | 颜色 | w:color | a:solidFill | color | color | fill |
| `bold` | 布尔 | w:b + w:bCs | b | b | font-weight | weight |
| `italic` | 布尔 | w:i + w:iCs | i | i | font-style | style |
| `underline` | 布尔，或 `single` `double` `thick` `dotted` `dashed` `wavy` | w:u val | u | u（single、double，另有会计用） | text-decoration-style | underline（只有实线） |
| `underlineColor` | 颜色 | w:u color | a:uFill | — | text-decoration-color | stroke |
| `strike` | 布尔，或 `double` | w:strike / w:dstrike | strike（sngStrike / dblStrike） | strike（只有单线） | line-through | strike |
| `verticalAlign` | `super` `sub` | w:vertAlign | baseline（±30000） | vertAlign | vertical-align | super / sub |
| `caps` | `all` `small` | w:caps / w:smallCaps | cap | — | text-transform / font-variant | smallcaps / upper |
| `letterSpacing` | 长度 | w:spacing（twip） | spc（1/100 pt） | — | letter-spacing | tracking |
| `highlight` | 颜色 | w:highlight（只有 16 种固定色）或 w:shd | a:highlight | — | background | highlight |
| `emphasisMark` | `dot` `comma` `circle` `underDot` | w:em（着重号） | — | — | text-emphasis | — |
| `scale` | 百分比数字，如 `80` | w:w | — | — | transform（不准） | — |
| `kerning` | 布尔 | w:kern | kern | — | font-kerning | kerning |
| `lang` | BCP 47，如 `zh-CN` | w:lang val / eastAsia | lang / altLang | — | lang | lang |
| `hidden` | 布尔 | w:vanish | — | — | display:none | — |

`fontFamily` 按文种分开写：

```json
{ "fontFamily": { "latin": "Inter", "eastAsia": "思源黑体", "complex": "Noto Naskh" } }
```

- `latin`：西文：w:rFonts ascii + hAnsi，a:latin
- `eastAsia`：中日韩：w:rFonts eastAsia，a:ea
- `complex`：阿拉伯文等：w:rFonts cs，a:cs

写成字符串时三项都用它。代码用什么字体不在这里定，写在 `InlineCode` 和 `Code` 组件的样式里（见第十三节）。Excel 一个单元格只能有一种字体：有中文内容取 `eastAsia`，否则取 `latin`。HTML、PDF 拼成回退列表：`latin, eastAsia`。

## 三、段落（对应 w:pPr、a:pPr、xlsx alignment）

| 字段 | 值 | Word w:pPr | PPT a:pPr | Excel alignment | HTML | PDF |
|---|---|---|---|---|---|---|
| `align` | `left` `center` `right` `justify` `distribute` | w:jc（start、center、end、both、distribute） | algn（l、ctr、r、just、dist） | horizontal | text-align | align / justify |
| `spaceBefore` | 长度或 `"Nline"` | w:spacing before / beforeLines | a:spcBef | — | margin-top | block above |
| `spaceAfter` | 同上 | w:spacing after / afterLines | a:spcAft | — | margin-bottom | block below |
| `lineHeight` | 数字是倍数，按 CSS 算（字号 × 倍数）；长度是固定值；`{ atLeast: 长度 }` 是最小值 | w:spacing line + lineRule（倍数写成 atLeast 字号 × 倍数，不用 auto：Word 的 auto 乘的是字体自己的单倍行高，雅黑约 1.32 倍字号；固定值 exact；最小值 atLeast） | a:lnSpc（spcPct / spcPts） | — | line-height | par leading（近似） |
| `marginLeft` | 长度，整段左缩进 | w:ind start | marL | indent（按字符数，只有整数） | margin-left | pad |
| `marginRight` | 长度，整段右缩进 | w:ind end | — | — | margin-right | pad |
| `textIndent` | 长度，首行缩进 | w:ind firstLine（`em` 时用 firstLineChars） | indent（正数） | — | text-indent | first-line-indent |
| `hangingIndent` | 长度，悬挂缩进：除首行外其他行缩进，和 `textIndent` 只能写一个 | w:ind hanging / hangingChars | indent（负数）+ marL | — | padding-left + 负 text-indent | hanging-indent |
| `keepWithNext` | 布尔 | w:keepNext | — | — | break-after: avoid | 用 block(breakable) 近似 |
| `keepTogether` | 布尔 | w:keepLines | — | — | break-inside: avoid | block(breakable: false) |
| `pageBreakBefore` | 布尔 | w:pageBreakBefore | — | — | break-before | pagebreak |
| `widowControl` | 布尔，默认开 | w:widowControl | — | — | orphans / widows | — |
| `contextualSpacing` | 布尔：同样式段落之间不加段距 | w:contextualSpacing | — | — | — | — |
| `autoSpace` | 布尔：中英文、中文和数字之间自动加空隙 | w:autoSpaceDE + w:autoSpaceDN | — | — | text-autospace（新） | — |
| `kinsoku` | 布尔：中文避头尾 | w:kinsoku | — | — | line-break: strict | 默认就有 |
| `direction` | `ltr` `rtl` | w:bidi | rtl | readingOrder | direction | dir |
| `wrap` | 布尔：自动换行 | — | a:bodyPr wrap | wrapText | white-space | — |
| `outlineLevel` | 1–9 | w:outlineLvl | — | — | — | — |

## 四、边框（对应 w:pBdr、w:tblBorders、w:tcBorders、a:ln、xlsx border）

一条边写成对象，也能用简写字符串 `"1pt solid #CCCCCC"`：

```json
{
  "border": { "width": 1, "style": "solid", "color": "#CCCCCC" },
  "borderBottom": { "width": 2, "style": "double", "color": "#1A73E8" },
  "borderBetween": "none"
}
```

- `borderBetween`：段落之间的线，w:pBdr between

| 字段 | 用在哪 | Word | PPT | Excel | HTML | PDF |
|---|---|---|---|---|---|---|
| `border` / `borderTop` / `borderRight` / `borderBottom` / `borderLeft` | 段落、单元格、表格、文本框 | w:top … w:start、w:end | 形状只有一条 a:ln，四边不能分开 | left right top bottom | border-* | stroke |
| `borderBetween` | 相邻同样式段落之间 | w:pBdr between | — | — | — | — |
| `borderInsideH` / `borderInsideV` | 表格内部横线、竖线 | w:tblBorders insideH / insideV | a:tcPr 的 lnB 等逐格写 | 逐格写 | 逐格写 | table stroke |
| `borderDiagonal` | 单元格斜线 | w:tl2br / w:tr2bl | a:lnTlToBr | diagonal | — | — |

线型 `style` 只收各格式都有的几种，再按格式换：

| fmtagno | Word val | PPT prstDash / cmpd | Excel style | CSS |
|---|---|---|---|---|
| `none` | nil | a:noFill | 不写 | none |
| `solid` | single | solid | 按粗细：hair（≤0.25pt）、thin（≤1）、medium（≤2）、thick | solid |
| `dashed` | dashed | dash | dashed / mediumDashed | dashed |
| `dotted` | dotted | sysDot | dotted | dotted |
| `dashDot` | dotDash | dashDot | dashDot / mediumDashDot | dashed（近似） |
| `double` | double | cmpd="dbl" | double | double |
| `wave` | wave | — | — | wavy 只有下划线有，边框退成 solid |

Word 的线宽只能在 0.25pt 到 12pt 之间（sz 2–96），Excel 只有四档，换算时就近取。

## 五、背景（对应 w:shd、a:solidFill / a:gradFill / a:pattFill、xlsx fill）

```json
{ "background": "#F5F7FA" }
{ "background": { "pattern": "pct20", "color": "#000000", "fill": "#FFFFFF" } }
{ "background": { "gradient": "linear", "angle": 90, "stops": ["#FFFFFF", "#E8F0FE"] } }
```

三行分别是纯色、底纹、渐变。

| 写法 | Word | PPT | Excel | HTML | PDF |
|---|---|---|---|---|---|
| 纯色 | w:shd val=clear fill | a:solidFill | patternFill solid | background-color | fill |
| 底纹 `pattern` | w:shd val（pct5…pct95、horzStripe 等） | a:pattFill prst | patternFill patternType（gray125、darkGrid 等） | 只能近似成纯色 | 近似成纯色 |
| 渐变 | 段落和单元格不支持，退成第一个色 | a:gradFill | gradientFill | linear-gradient | gradient.linear |
| 图片 | 不支持 | a:blipFill | 不支持 | background-image | 不支持 |

`pattern` 的名字取 Word 和 Excel 的交集，具体列表实现时定。

## 六、盒子（单元格、文本框、提示框这类容器）

| 字段 | 值 | Word | PPT | Excel | HTML | PDF |
|---|---|---|---|---|---|---|
| `padding` / `paddingTop` … | 长度 | w:tcMar；段落用边框 space 近似 | a:bodyPr lIns tIns rIns bIns | — | padding | inset |
| `verticalAlign` | `top` `middle` `bottom` | w:vAlign | a:bodyPr anchor | vertical | vertical-align | align |
| `width` / `height` | 长度或百分比 | w:tcW、w:trHeight | a:xfrm ext | 列宽、行高 | width / height | width / height |
| `textRotation` | `0` `90` `270` | w:textDirection | a:bodyPr vert | textRotation | writing-mode | rotate |
| `radius` | 长度 | — | prstGeom roundRect | — | border-radius | radius |

## 七、列表（对应 w:numbering、a:pPr 的 bu*、CSS list-style）

Word 的列表定义在 numbering.xml：`w:abstractNum` 下每一级一个 `w:lvl`，段落用 `w:numPr` 引用。PPT 把项目符号直接写在段落的 a:pPr 上。

| 字段 | 值 | Word w:lvl | PPT a:pPr | HTML | PDF |
|---|---|---|---|---|---|
| `listStyleType` | 无序：`disc` `circle` `square` `dash` `none`，或任意字符如 `"▸"`；有序：`decimal` `decimal-leading-zero` `lower-alpha` `upper-alpha` `lower-roman` `upper-roman` `cjk-decimal`（一、二、三） `cjk-ideographic`（甲、乙、丙） `circled-decimal`（①②③） | w:numFmt（bullet、decimal、decimalZero、lowerLetter、upperLetter、lowerRoman、upperRoman、chineseCounting、ideographTraditional、decimalEnclosedCircle） + w:lvlText | a:buChar / a:buAutoNum type（arabicPeriod、alphaLcParenR、romanUcPeriod、circleNumDbPlain、ea1ChsPeriod …） | list-style-type | list marker / enum numbering |
| `listMarkerFormat` | 编号的模板，`%1` 是本级编号：`"%1."` `"(%1)"` `"第%1条"`；多级 `"%1.%2"`；本级是 `decimal` 时上几级也写成阿拉伯数字（上级「一、」，本级出「2.1」） | w:lvlText（+ w:isLgl） | 只能从 buAutoNum 的固定几种里选，选不到就退成 `%1.` | ::marker content / counters() | enum numbering 函数 |
| `listMarkerImage` | 图片路径，当项目符号 | w:lvlPicBulletId | a:buBlip | list-style-image | marker 里放 image |
| `listMarkerColor` | 颜色 | w:lvl 的 w:rPr color | a:buClr | ::marker color | marker 用 text fill |
| `listMarkerFont` | 字体名（Wingdings 这类符号字体） | w:lvl 的 w:rFonts | a:buFont | ::marker font-family | — |
| `listMarkerSize` | 百分比数字，相对正文 | w:lvl 的 w:sz | a:buSzPct | ::marker font-size | — |
| `listMarkerAlign` | `left` `center` `right`：编号在编号区里怎么对齐（1. 到 10. 右对齐时小数点能对齐） | w:lvlJc | — | ::marker text-align（支持差） | — |
| `listMarkerGap` | 编号和正文之间：长度，或 `tab` `space` `none` | w:suff + 制表位 | indent 和 marL 的差 | ::marker padding / gap | body-indent |
| `listIndent` | 每一级缩进多少（正文左边到哪） | w:ind left | marL（逐级） | padding-left | indent |
| `listHanging` | 编号区宽度，就是悬挂缩进 | w:ind hanging | indent（负数） | 负 text-indent | 用 body-indent 近似 |
| `listStart` | 有序列表从几开始 | w:start | buAutoNum startAt | start 属性 | enum start |

多级列表：List 的 props 里 items 可以嵌套；每一级的样式写在 `List.level2`、`List.level3` 这样的组件样式键下（见第十一节），没写的级别沿用上一级，`listIndent` 每级自动累加。

## 八、图片位置（对应 wp:inline / wp:anchor、p:pic 的 a:xfrm、xlsx 的 xdr anchor）

| 字段 | 值 | Word | PPT | Excel | HTML | PDF |
|---|---|---|---|---|---|---|
| `position` | `inline`（默认，跟文字走）`float`（浮动，文字绕排）`absolute`（按页面或幻灯片定位，不占位置） | wp:inline / wp:anchor | 都是绝对定位，inline 时按版式顺排 | twoCellAnchor / oneCellAnchor | 正常流 / float / position:absolute | 正常 / place(float: true) / place |
| `float` | `left` `right` | wp:anchor + positionH align | — | — | float | place(left / right) |
| `wrap` | `square` `tight` `topAndBottom` `none` `behind` `inFront` | wrapSquare / wrapTight / wrapTopAndBottom / wrapNone + behindDoc | — | — | float + shape-outside（tight 近似 square） | float: true 只有 topAndBottom 和 square 近似 |
| `wrapDistance` | 长度，或四边 `{ top, right, bottom, left }` | distT / distR / distB / distL | — | — | margin | clearance |
| `anchorTo` | `paragraph` `margin` `page`，PPT 里是 `slide` | positionH / positionV relativeFrom（column、margin、page） | 幻灯片 | 单元格 | 包含块 | place 的参照 |
| `x` / `y` | 长度：相对 `anchorTo` 的偏移 | wp:posOffset（EMU） | a:off（EMU） | xdr:from 列行 + 偏移 | left / top | dx / dy |
| `horizontalAlign` | `left` `center` `right` | positionH align | 按幻灯片宽算 | — | margin auto | place(center) |
| `verticalAlign` | `top` `middle` `bottom` | positionV align | 按幻灯片高算 | — | — | place(top / bottom) |
| `objectFit` | `contain` `cover` `fill`：宽高都给时怎么缩放 | 按比例算 extent；cover 用 a:srcRect 裁 | a:stretch / a:srcRect | 按比例算 | object-fit | fit |
| `rotation` | 角度 | a:xfrm rot | a:xfrm rot | — | transform | rotate |
| `opacity` | 0–1 | a:alphaModFix | a:alphaModFix | — | opacity | — |

图片的 `width`、`height` 继续放在 Image 的 props 里（它们是内容尺寸，不是样式），上面这些是样式。

## 九、表格（对应 w:tblPr、w:trPr、xlsx 工作表）

这些是表格组件的样式，不是每个元素都能用。

| 字段 | 值 | Word | PPT | Excel | HTML | PDF |
|---|---|---|---|---|---|---|
| `tableLayout` | `fixed` `auto` | w:tblLayout | 固定 | 固定 | table-layout | 列宽 |
| `cellPadding` | 长度 | w:tblCellMar | 逐格 | — | td padding | inset |
| `repeatHeader` | 布尔，默认开 | w:tblHeader | — | 打印标题行 | thead | table.header(repeat) |
| `rowBreak` | 布尔：行内能不能跨页 | w:cantSplit | — | — | break-inside | — |
| `bandSize` | 隔几行算一条带，默认 1 | w:tblStyleRowBandSize | — | — | nth-child(an+b) | fill 函数 |

表头、首行末行、奇偶行这些部位的样式见第十二节。

部位上可以写 `padding`、`paddingLeft` 这些，只改落在这个部位里的格子，没写的边还用 `cellPadding`。docx 写成这一格的 w:tcMar，HTML 是 td 的 padding，PDF 是这一格的 inset。

### 列宽

列宽写在 Table 的 props `widths` 里，一列一个值，三种写法可以混用：

- 数字：比例。固定列和 auto 列占完以后，剩下的宽度按比例分。
- 带单位的字符串：固定宽度，比如 `"5.65pt"`、`"20mm"`。不带单位的字符串（`"5"`）当 pt。
- `"auto"`：按格子里的文字撑开。

只要有一列是比例，表格就撑满版心（或撑到样式里的 `width`）。全是固定列和 auto 列时，表格宽度就是各列加起来，不撑满。比如章节标题条：`["5.65pt", "auto"]`，左边一条细色块，右边按标题长短变宽。

各格式怎么做：HTML 用自动布局，auto 列不写宽度；PDF 是 Typst 的 `auto` 列；docx 的 auto 列写 `w:tcW w:type="auto"`，表格宽度写 auto、布局改成 autofit，gridCol 按字数估一个初始宽度（全角字 1em、半角 0.6em，加字间距、左右 padding 和 3pt 余量），Word 打开后按内容重排。有 auto 列又没写 `tableLayout` 时，三种格式都按 `auto` 布局。

合并单元格（w:gridSpan、w:vMerge）放在 Table 的 props 里，不算样式。

## 十、页面（`meta.page`，对应 w:sectPr）

| 字段 | 值 | Word | PPT | Excel | HTML | PDF |
|---|---|---|---|---|---|---|
| `size` | `A4` `Letter` …，或 `{ width, height }` | w:pgSz | p:sldSz | pageSetup paperSize | @page size | page width / height |
| `landscape` | 布尔 | w:pgSz orient | — | orientation | @page | flipped |
| `margin` / `marginTop` … | 长度 | w:pgMar | — | pageMargins | @page margin | margin |
| `headerDistance` / `footerDistance` | 长度，默认 0.5in | w:pgMar header / footer | — | pageMargins header / footer | 页眉页脚框离纸边的距离（`--paged`） | 页眉页脚框的内边距 |
| `gutter` | 长度 | w:pgMar gutter | — | — | — | binding |
| `columns` | 数字，或 `{ count, gap }` | w:cols | — | — | columns | columns |
| `verticalAlign` | `top` `center` | w:vAlign | — | — | — | — |

`meta.page` 是第一节的设置。另外三个字段管页眉页脚和页码：

| 字段 | 值 | Word | HTML（`--paged`） | PDF |
|---|---|---|---|---|
| `header` / `footer` | 元素 id，或 `{ default, first, even }` | w:headerReference / w:footerReference | `<template>`，排完版由脚本放进页边距 | page header / footer |
| `pageNumberStart` | 数字 | w:pgNumType start | 脚本数页码时从这里重新数 | 自己数页码 |

### 分节

`Section` 是容器组件，只能直接放在 root 下面。每一节从新的一页开始，可以改纸张、方向、页边距、分栏、页眉页脚，也可以让页码重新开始。`Section` 的 props 就是上表的这些字段，都可以不写，不写的沿用上一节，跟 Word 的「链接到前一节」一样。

root 下面不写 `Section` 也行，整份文档就是一节，用 `meta.page` 的设置。也可以开头一段不放进 `Section`，算作第一节，后面全用 `Section`；但 `Section` 后面不能再跟不在 `Section` 里的内容。

```json
"elements": {
  "doc":   { "type": "Document", "children": ["cover", "body"] },
  "cover": { "type": "Section", "props": { "header": { "first": null } }, "children": ["title"] },
  "body":  {
    "type": "Section",
    "props": { "pageNumberStart": 1, "header": { "default": "hdr-odd", "even": "hdr-even" } },
    "children": ["ch1", "p1"]
  },
  "foot":  { "type": "Text", "props": { "text": ["第 ", { "field": "page" }, " 页，共 ", { "field": "pages" }, " 页"] } }
}
```

### 页眉页脚

- 写一个元素 id，等于只改 `default`（普通页用的那个）。
- 写对象时分开设：`default` 普通页，`first` 本节第一页，`even` 偶数页。写 `null` 表示这一种留空，不写表示沿用上一节。
- 只要设了 `first`（包括 `null`），本节首页就单独处理（Word 的「首页不同」）；页眉和页脚共用这个开关，只给页眉设了 `first` 时，首页的页脚用 `default`。
- 任何一节设了 `even`，整份文档就开「奇偶页不同」（Word 里这是整个文档的开关），没设 `even` 的节偶数页用 `default`。
- `"header": null` 写在 `Section` 上，表示清掉从上一节接过来的全部页眉。
- 页眉页脚里的元素跟正文一样写在 `elements` 里，但不能同时出现在正文里，也不能放 PageBreak、Section、Slide。样式继承根元素（Document）的样式。

### 页码

runs 里写 `{ "field": "page" }` 是当前页码，`"pages"` 是全文总页数，`"sectionPages"` 是本节页数。可以带文字样式；`text` 可以不写，写了就是 Word 打开时更新前显示的值。页码按 Word 的规则数：每节接着上一节往下数，节上写了 `pageNumberStart` 就从那个数重新开始；奇偶页按页码数，不按物理页数。

各格式的差别：

- docx：写成 Word 的域（PAGE、NUMPAGES、SECTIONPAGES），Word 打开后自己算。LibreOffice 不认 SECTIONPAGES，只显示 `text` 的值。
- pdf：Typst 排完版后按上面的规则算好。
- html：直接打开或打印时没有页眉页脚，页码显示 `text` 的值；加 `--paged` 后由脚本排完版再放页眉页脚、填页码。屏幕上每节按自己的纸张显示，但打印时 Paged.js 只用第一节的纸张，横竖混排的文档打出来横向那几页会被裁掉，要打印请用 pdf。

## 十一、样式集和继承

### 样式集

样式集是一整套外观：全局样式加各个组件的样式，组件样式继承全局样式。同一份文档换一个样式集，就换一套外观。

```json
{
  "styleSet": "formal",
  "styleSets": {
    "formal": {
      "global": {
        "fontFamily": { "latin": "Times New Roman", "eastAsia": "宋体" },
        "fontSize": 12,
        "lineHeight": 1.5
      },
      "components": {
        "Heading": { "fontFamily": { "eastAsia": "黑体" }, "bold": true },
        "Heading.1": { "fontSize": 22, "align": "center" },
        "Text": { "textIndent": "2em" },
        "Table.header": { "background": "#F2F2F2", "bold": true },
        "List.level2": { "listStyleType": "circle" }
      },
      "styles": { "muted": { "color": "#666666" } }
    },
    "modern": {
      "extends": "formal",
      "global": { "fontFamily": { "latin": "Inter", "eastAsia": "思源黑体" }, "fontSize": 11 }
    }
  },
  "styles": { "Heading.2": { "color": "#1A73E8" }, "warn": { "color": "#D93025" } }
}
```

- `styleSet`：这份文档用哪个样式集；命令行 --style-set 可以临时换
- `global`：全局样式：所有组件都继承
- `components`：组件样式：键是组件名，或「组件名。部位」
- `styles`：这个样式集里的命名样式，元素用 style: muted 引用
- `extends`：在 formal 基础上改
- `styles`：不属于任何样式集：哪个样式集都生效

- 样式集可以写在文档里，也可以单独成 JSON 文件（文件里要写 `name`），放在文档旁边的 `styles/` 目录会自动读，也可以用 `--style-sets` 指定文件或目录。这样一套公司模板能给很多文档用。文档里写的同名样式集优先。
- 顶层 `styles` 不属于任何样式集。键是组件名或「组件名。部位」时是组件样式，其他是命名样式。
- 样式集之间用 `extends` 继承，只能单继承，成环报错。
- `global` 里只放会继承的字段（文字和段落），写边框、背景这类字段会报错，因为它们不该铺到每个组件上。

组件样式的键：

| 键 | 管什么 |
|---|---|
| `Heading`、`Text`、`List`、`Table`、`Image`、`Slide` | 这类组件的全部元素 |
| `Heading.1` … `Heading.6` | 某一级标题 |
| `List.level1` … `List.level9`、`List.marker` | 某一级列表、编号本身 |
| `Table.header`、`Table.firstRow` … `Table.evenColumn` | 表格各部位，见第十二节 |
| `Image.caption` | 图片说明 |
| 自定义组件名 | 加在展开后的模板根元素上 |

### 继承顺序

一个元素的样式按下面的顺序叠，后面的覆盖前面的：

1. 父元素传下来的文字、段落、列表字段（像 CSS 继承；边框、背景、盒子、图片位置字段不继承）。根元素的「父元素」是 fmtagno 内置的全局默认值，再叠上样式集的 `global`（有 `extends` 时先叠父样式集的）。
2. fmtagno 内置的组件默认样式：先 `Heading`，再 `Heading.1`。
3. 样式集的组件样式：先 `Heading`，再 `Heading.1`。
4. 顶层 `styles` 里的组件样式：同样先 `Heading` 再 `Heading.1`。
5. 自定义组件模板里写的 `style`（内层模板在前，外层模板给实例写的在后）。
6. 指名这个自定义组件的组件样式：`PriceTable`（只管模板根元素）和 `PriceTable:t`（管模板里的元素 `t`）。
7. 元素自己的 `style`：样式名、样式名数组、样式对象，后面的覆盖前面的。样式名先在当前样式集的 `styles` 里找，找不到再到顶层 `styles` 找。
8. runs 上的文字字段：先是元素的文字样式，行内代码再叠 `InlineCode` 组件样式，链接默认蓝色加下划线，然后是 `bold` 这类简写，最后是 run 自己的 `style`。

几点说明：

- 第 1 步排最低：组件样式总能盖过从父元素继承来的值，和 CSS 一样。
- 第 4 步排在第 3 步后面：样式集当底子，文档自己写的组件样式优先。
- 第 5 步排在第 3、4 步后面：自定义组件的设计不会被通用的 `Table` 样式冲掉；要改它，用第 6 步的键指名。
- 部位样式（第十二节）按同样的顺序叠：内置 → 样式集 → 顶层 styles → 模板 `parts` → `组件名:元素id.部位` → 元素自己的 `parts`。
- runs 上的 `style` 只能写文字字段，外加 `background`（行内底色）。

docx 现在把算好的样式直接写在段落和 run 上（直接格式）；标题另外挂 Word 的「标题 1」到「标题 6」样式（导航窗格和目录能用），代码挂 `Code`、`InlineCode` 样式名。把样式集整体写成 Word 样式表，留到以后。

## 十二、部位样式和组件结构

### 部位

一个元素可以分成几个部位，每个部位单独设样式。表格的部位照 Word 表格样式的条件格式（w:tblStylePr）来定：

| 部位 | 管什么 | Word w:tblStylePr type | Excel 表格样式 | HTML | PDF |
|---|---|---|---|---|---|
| `table` | 整张表 | 表格样式本身 | wholeTable | table | table |
| `header` | 表头（columns 那一行），等于 `firstRow` | firstRow | headerRow | thead tr | table.header |
| `firstRow` / `lastRow` | 第一行、最后一行（如合计行） | firstRow / lastRow | headerRow / totalRow | tr:first-child / :last-child | fill、stroke 函数按行号判断 |
| `firstColumn` / `lastColumn` | 第一列、最后一列 | firstCol / lastCol | firstColumn / lastColumn | td:first-child / :last-child | 同上 |
| `oddRow` / `evenRow` | 奇数行、偶数行（不算表头） | band1Horz / band2Horz | firstRowStripe / secondRowStripe | tbody tr:nth-child(odd / even) | 同上 |
| `oddColumn` / `evenColumn` | 奇数列、偶数列 | band1Vert / band2Vert | firstColumnStripe / secondColumnStripe | nth-child | 同上 |
| `topLeftCell` `topRightCell` `bottomLeftCell` `bottomRightCell` | 四个角的格子 | nwCell / neCell / swCell / seCell | — | 组合选择器 | 同上 |
| `cell` | 每个格子 | 表格样式的 w:tcPr | — | td | table.cell |

一个格子同时落在几个部位时，按 Word 的顺序叠：`table` → `cell` → 奇偶列 → 奇偶行 → 首末列 → 首末行 → 四角，后面的覆盖前面的。

哪些部位生效用 Table 的 props `look` 开关，对应 Word 的 w:tblLook：

```json
{
  "props": {
    "look": {
      "firstRow": true,
      "lastRow": true,
      "firstColumn": false,
      "bandedRows": true,
      "bandedColumns": false
    }
  }
}
```

docx 里各部位的样式现在逐格写成直接格式。docx-rs 还不支持 w:tblStylePr，所以还生成不了真正的 Word 表格样式，以后补。

其他组件的部位：

| 组件 | 部位 |
|---|---|
| List | `level1` … `level9`、`marker`、`item` |
| Image | `caption` |
| Slide | `title`、`body` |
| Heading | 用 `Heading.1` … 区分级别，不算部位 |

### 部位样式写在哪

三个地方都能写，写法一样，部位名做键：

```json
{
  "prices": {
    "type": "Table",
    "props": {
      "columns": ["品名", "单价"],
      "rows": [["苹果", "3.5"], ["梨", "4.0"], ["合计", "7.5"]]
    },
    "style": { "border": "0.5pt solid #CCCCCC" },
    "parts": {
      "header": { "background": "#1A73E8", "color": "#FFFFFF", "bold": true },
      "evenRow": { "background": "#F5F7FA" },
      "lastRow": { "bold": true, "borderTop": "1pt solid #333333" }
    }
  },
  "styles": {
    "Table.header": { "bold": true },
    "Table.oddRow": { "background": "#FAFAFA" }
  }
}
```

- `style`：CCCCCC" }
- `lastRow`：333333" }

### 组件自带结构和样式

自定义组件的模板本身就是结构：里面放一张表，表的部位样式直接写在模板元素的 `parts` 上，用这个组件的人只管传数据。

```json
{
  "name": "PriceTable",
  "description": "报价表：蓝色表头、斑马纹、合计行加粗",
  "props": { "rows": { "type": "array", "required": true } },
  "template": {
    "root": "t",
    "elements": {
      "t": {
        "type": "Table",
        "props": {
          "columns": ["品名", "数量", "单价", "金额"],
          "rows": "{{rows}}",
          "look": { "lastRow": true }
        },
        "style": { "border": "0.5pt solid #D0D7DE" },
        "parts": {
          "header": { "background": "#1A73E8", "color": "#FFFFFF", "bold": true },
          "evenRow": { "background": "#F5F7FA" },
          "lastRow": { "bold": true, "borderTop": "1pt solid #333333" },
          "lastColumn": { "align": "right" }
        }
      }
    }
  }
}
```

- `style`：D0D7DE" }
- `lastRow`：333333" }

样式集要改某个自定义组件里面的样式，用「组件名：模板元素 id」，再接部位：

```json
{
  "styleSets": {
    "dark": {
      "components": {
        "PriceTable": { "fontSize": 10 },
        "PriceTable:t.header": { "background": "#333333" }
      }
    }
  }
}
```

- `PriceTable`：组件根元素
- `PriceTable`：组件里表格 t 的表头

组件自带的样式排在通用组件样式之后（见第十一节第 5 步）：`Table` 这类通用样式改不动它，要改就用 `PriceTable`、`PriceTable:t.header` 这样指名的键，或者在元素上直接写。

## 十三、代码：InlineCode 和 Code

代码不靠猜，也不靠字体判断，用组件明确标出来：

- **InlineCode**：行内代码，写在文字的 runs 里，和普通 run 并列。取代现在 run 上的 `code: true`。
- **Code**：整段代码，是块级组件，和 Text 并列。

```json
{
  "intro": {
    "type": "Text",
    "props": {
      "text": [
        "运行",
        { "type": "InlineCode", "text": "fmtagno render doc.json -o out.pdf" },
        "生成 PDF。"
      ]
    }
  },
  "sample": {
    "type": "Code",
    "props": {
      "language": "rust",
      "code": "fn main() {\n    println!(\"你好\");   // 中文注释走 eastAsia 字体\n}\n",
      "lineNumbers": false
    }
  }
}
```

- `language`：只做标记，第一版不做语法高亮

字体和外观写在组件样式里，跟着样式集走：

```json
{
  "styleSets": {
    "formal": {
      "components": {
        "InlineCode": {
          "fontFamily": { "latin": "JetBrains Mono", "eastAsia": "思源黑体" },
          "background": "#F3F4F6"
        },
        "Code": {
          "fontFamily": { "latin": "JetBrains Mono", "eastAsia": "思源黑体" },
          "fontSize": 9,
          "background": "#F6F8FA",
          "padding": 8
        }
      }
    }
  }
}
```

| | Word | PPT | Excel | HTML | PDF |
|---|---|---|---|---|---|
| InlineCode | 字符样式（w:style type="character"，名为 InlineCode） | a:rPr 直接写字体 | 单元格富文本里的一段 | `<code>` | `raw(block: false)` |
| Code | 段落样式（名为 Code），保留空格和换行（xml:space="preserve"，换行用 w:br） | 文本框 | 不支持，退成文本 | `<pre><code>` | `raw(block: true)` |

- 内置默认样式给它们一套等宽字体回退：macOS 用 Menlo，Windows 用 Consolas，Linux 用 DejaVu Sans Mono。
- InlineCode 只能放文字类 props（Text、Heading、List 项、表格格子），不能单独当元素用。
- `language` 第一版只写进 HTML 的 `class="language-rust"`，以后再考虑高亮。
