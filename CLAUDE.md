# CLAUDE.md — pptspine(宪章)

> 家族关系与依赖：先读 [`docs/spine-family.md`](docs/spine-family.md)（每仓副本相同，真源在家族根目录）。

Spine 家族成员之一:**纯 Rust 的 PowerPoint(.pptx / OOXML)结构化解析器 + 本地图片 OCR**。
先读家族 `../README.md`,本文件是 pptspine 的操作指南,风格对齐 `../corespine/CLAUDE.md`。

## 这是什么

`.pptx` 本质是 OOXML —— 一个装着 XML 部件的 zip 包。pptspine **直接走 XML**,把它解析成
**信息无损**的结构化模型:幻灯片、文本框(段落 + 带样式的 run)、表格(单元格、合并、填充)、
图片、自选图形(autoshape)。嵌入的图片还可以经**离线、确定性**的姊妹 crate
[`ocrspine`](../ocrspine)(PP-OCRv5 / `tract-onnx`)做本地 OCR —— **无云端、无网络**。

## 宪章(不可违背)

- **零网络、零云 LLM。** OCR 一律走本地 `ocrspine`(tract-onnx),确定性输出。任何联网/云推理
  的代码**不准进**。
- **容错解析,绝不 panic。** 未知元素跳过、缺失属性 → `None`、畸形输入 → 类型化 `PptError`。
  解析层对脏输入必须健壮。
  - **解压限额(`ppt_parse::ZipLimits`)。** 读 zip 包不信任头字段声明大小(不按它预分配,
    `take(limit + 1)` 截断读取),超限返回 `PptError::LimitExceeded { kind: LimitKind, limit, actual }`
    (Python 侧为 `PptZipError`,信息含限额种类)。默认:条目数 10 000、单条目 256 MiB、总解压量
    1 GiB、压缩比 10 000(仅对解压量 > 1 MiB 的条目判定)、条目名 1024 字节、幻灯片数 5 000(`p:sldIdLst` 边读边去重,
    收集到 `max_slides + 1` 个不同部件就停;重复引用记 `duplicate-slide-ref`);绝对路径 / 盘符形式(`C:`)/ 含 `..`
    的条目名直接拒绝(`PptError::Zip`)。`parse_bytes` / `parse_path` 用默认值,`parse_*_with_limits` 可自定(Python:
    `open` / `open_bytes` 的仅关键字参数,非法值 `ValueError`,超出 64 位报 "too large")。
  - **内存按字节记账(`ZipLimits::max_model_bytes`,缺省 2 GiB)。** 每产生一个非共享节点按结构体大小扣、每个字符串按
    实际字节扣(`xml/budget.rs` 线程局部状态;`attr_string` / `read_text` 统一记账),所有部件(slide / layout / master /
    主题 / 表格样式 / 备注 / 批注 / 图表 / SmartArt data 与 drawing)都经 `Package::budgeted`(同一套预算、同一套
    诊断);逐 frame / 逐 slide 的克隆(SmartArt 展开、批注拷贝)每份都扣。被多处引用的值**共享**而不是克隆:批注
    作者 / 缩写、超链接目标是 `Arc<str>`,图表是 `Arc<Chart>`(只在首次解析时扣),终态 IR 的图表与链接同样共享;
    py-bindings 每次转换按指针缓存,同一共享值只建一个 Python `str`。单值上限:属性 64 KiB、文本节点 1 MiB、
    标签(批注作者 / 图表类别 / 系列 / 标题)4 KiB、URL 16 KiB、OMML 定界符 / 运算符 2 个字符,超长截断记
    `value-truncated`。估算口径见 `ppt_core::model_bytes`(`estimated_model_bytes` ≤ 预算是测试性质)。合法基准
    (`tests/legit_benchmarks.rs`)实测最大 0.76 GB 记账(RSS 约 1.5 GB)。
  - **个数预算是第二道闸**(缺省 ≥ 基准实测需求 4 倍,合法大文稿零截断零诊断):SmartArt 展开形状 100 000 / 文字
    8 MiB(单 drawing ≤ 10 000 形状)、图表数据点(按 frame)2 000 000、批注含回复 100 000(批注部件按**剩余**额度
    解析,额度为 0 直接跳过)、单部件 / 全文形状 20 000 / 1 000 000、单部件 / 全文节点 200 000 / 8 000 000;
    终态 IR 的母版 / 版式继承形状实例全文 ≤ `MAX_INHERITED_SHAPES`(400 000)。超出后提前停止(已解析的保留),
    记 `shapes-truncated` / `content-truncated` / `comments-truncated` / `smartart-degraded` / `chart-degraded`;
    `count` = 实际丢失量(被跳过的行 / 单元格 / 段落 / 组合的后代都计,`skip_dropped`)。**截断公平**:先解析
    layout / master / theme / 表格样式,再逐页解析(每页的图表 / SmartArt / 备注 / 批注紧随其后),解析任何部件前先为
    剩余幻灯片每页预留保底 `min(节点 2 000 / 形状 100 / 字节 2 MiB, 剩余 / 页数)`——耗尽时"每页长尾被截"。
    `Presentation.report`(`ParseReport`;Python `truncated` / `parse_report()`)给出被截断部件全集(不受诊断
    10 000 条上限影响)、丢弃计数与用量。诊断 `part` 只准是包内真实部件路径,不同条目 ≤ 10 000,超出并入每种
    kind 一条 `part=""` 的汇总。
  - **导出 / 渲染有界。** `ExportOptions::max_output_bytes`(缺省 64 MiB,写满即停、带截断标记行)、
    `RenderOptions::max_page_ops` / `max_total_ops`(缺省 100 000 / 10 000 000 op 权重,表格 / 组合按单元格 /
    子形状粒度停下,`render-budget` 告警)。阅读顺序 XY-cut 的切分步数预算 `4·n·⌈log₂(n+1)⌉ + 20 000`,超出退回
    (top, left) 行排序(正常版式结果不变,有固化语料摘要)。
  - 组合 / `mc:AlternateContent` 嵌套超过 64 层的子树整体跳过(防递归下降爆栈)。`mc:AlternateContent` 按**信息量**
    选分支(`info_score`:可见文字字符数 + 有媒体图片 / 表格 / 图表 / SmartArt / 有几何形状各 1;空文本框、无媒体
    图片、无几何形状、裸 OLE 占位框、空组合为 0):第一个 > 0 的 Choice → 否则 > 0 的 Fallback → 都为 0 取叶子
    多的、相同取 Choice;落选分支的预算与诊断计数退还(段落层:第一个有文字的 Choice,否则 Fallback)。`a14:m` 公式
    线性化为 `RunKind::Math` run(括号只看槽位线性化结果:非原子才加括号;`m:d` / `m:nary` / `m:sSubSup` / `m:sPre` /
    `m:limLow|Upp` / `m:func` / `m:bar|acc` / `m:m` / `m:eqArr` 各有确定形式,自闭合 `<m:e/>` 是空槽位,空结构输出
    空串,`m:subHide` / `m:supHide` 隐藏上下限,见 `xml/slide.rs` 公式段注释;与 docspine 各自独立实现)。
- **缝的元模式(家族统一)。** 唯一外部能力(OCR)经 Protocol seam 接入:`OcrEngine`(来自
  `ocrspine`)是协议,`PaddleOcr` 是确定性默认实现。core 只依赖协议,**绝不**直接 import 任何
  推理 SDK。
- **最小、优雅,不过度设计。** 文本 + 表格是**必须项**;图形/颜色尽力而为。痛了再抽,带证据抽。

## 铁律(本仓特有)

- **`../pdfspine/` 只读。** 另一个 agent 正在改它。可读它学模式(PyO3 chokepoint / 工作区布局),
  但**绝不**写入或修改 pdfspine 里的**任何**文件。
- **姊妹 crate 走 git dep(非 path)。** 在 `[workspace.dependencies]` 里一次性声明
  `ocrspine = { git = "https://github.com/VoldemortGin/ocrspine", rev = "041958aa…" }`
  (pdf-typeset 同理:`{ git = "https://github.com/VoldemortGin/pdfspine", rev = "…" }`);
  `ppt-ocr` 用 `ocrspine.workspace = true`、`ppt-render` 用 `pdf-typeset.workspace = true`,
  避免逐 crate 算相对路径。

## 模块地图(按 crate 定位)

```
crates/
  ppt-core/    领域模型 + 几何(EMU) + 类型化 PptError。无 IO / zip / XML。#![forbid(unsafe_code)]
    src/error.rs   PptError(thiserror):Zip/Xml/Unsupported/InvalidArgument/Io/Ocr + kind() + Result<T>
    src/diagnostics.rs Diagnostic / DiagnosticKind(#[non_exhaustive]):解析诊断(种类 + 部件路径 + 计数,绝不含正文),挂在 Presentation.diagnostics
    src/custgeom.rs a:custGeom 纯数据模型(CustGeom/Guide/CustPath/PathCmd)+ 预算常量(参考线 1024 / path 256 / 命令 20 000)
    src/geom.rs    Emu(i64,914400/inch) + to_points + Rect/Point
    src/model_bytes.rs 模型字节估算(Estimator:结构体大小 + 字符串字节,共享 Arc 只计一次)
    src/model.rs   Presentation/Slide/Shape/TextFrame/Paragraph/TextRun/Table/Row/Cell/Picture/AutoShape/Color
                   + Chart/ChartKind/ChartSeries(挂在 GraphicPlaceholder.chart)
    src/export/    reading_order.rs(XY-cut 视觉阅读顺序,展平组合几何) view.rs(导出选项 + 每页有序形状视图 + 纯文本)
                   markdown.rs(语义 Markdown:标题占位符 / 列表标记 / 图片 alt / 超链接 / 图表表格)
  ppt-parse/   OOXML 读取:zip 解包 + quick-xml 遍历 -> Presentation。本轮核心。#![forbid(unsafe_code)]
    src/lib.rs     parse_path / parse_bytes -> ParsedPptx { presentation, media }
    src/zip_pkg.rs zip 读 API:主部件(经 `_rels/.rels` 的 officeDocument 定位,缺失回退 presentation.xml)/ slides / _rels / media / layouts / masters
    src/links.rs   超链接后处理:rels 回填外链 url,页内跳转折成目标幻灯片序号
    src/charts.rs  图表后处理:占位的 c:chart@r:id 经 slide rels 读 ppt/charts/chartN.xml 回填 chart
    src/diagrams.rs SmartArt 后处理:dgm:relIds@r:dm → data 部件 → drawing 部件(优先,包成 frame 变换的组合)/ 退回 data 文字
    src/xml/       quick-xml walker:budget.rs(解析期线程局部预算:节点 / 模型字节 + 单值上限)
                   presentation.rs(尺寸+顺序) slide.rs(spTree -> Shape)
                   comments.rs(批注部件 + 作者部件:旧式 p:cmLst / 新式 p188:cmLst 含回复 -> Comment;审阅元数据,不进导出)
                   diagram.rs(SmartArt data 部件:内容点文字 + dataModelExt 的 drawing 关系 id)
                   doc_props.rs(docProps/core.xml + app.xml -> DocProperties)
                   chart.rs(c:chartSpace 缓存 -> Chart:种类/标题/类别/系列,稀疏 pt 补空,不读外部工作簿)
                   custgeom.rs(a:custGeom -> CustGeom:avLst/gdLst 参考线 + pathLst 路径命令,带预算,超限整体丢弃并记 custom-geometry-degraded 诊断)
                   table_style.rs(ppt/tableStyles.xml -> styleId -> TableStyle:九部件填充/边框/文字色)
  ppt-ocr/     图片 OCR 桥:把 ocrspine 套到嵌入图片上。本轮薄但可用。#![forbid(unsafe_code)]
    src/lib.rs     ocr_image_bytes / PptOcr{engine}
    src/table.rs   图片表格几何重建:OCR 词框 → 行列网格(reconstruct_table_from_image / reconstruct_from_words;移植自 docspine,家族第三份同源实现)
  ppt-render/  终态 IR -> PDF:逐 slide 一页,经共享 pdf-typeset 引擎(pdfspine Phase A)。#![forbid(unsafe_code)]
    src/lib.rs       render_pdf(pres,media,opts)->ExportResult:逐 slide 装配 / 背景 / 表格网格 / font_map 应用
    src/text.rs      ResolvedTextFrame 段落/run -> TS-5 绝对定位文本框(锚定/内边距/换行/项目符号/行距)
    src/shapes.rs    自选图形 / 连接线 / 图片 / 图表占位 -> 引擎 op
    src/chart.rs     图表矢量渲染:纯函数几何(柱/条/折线/饼 + 刻度/图例/标签)-> 引擎 op;不支持降级占位框
    src/custgeom.rs  a:custGeom 求值:参考线公式求值器(纯函数)+ 路径构建(缩放 / arcTo→三次贝塞尔)→ PathSeg;失败返回 None 由 shapes.rs 退回包围盒
    src/shapes/line_ends.rs 线端装饰 headEnd/tailEnd(triangle/stealth/diamond/oval/arrow)
    src/transform.rs 组合仿射(chOff/chExt 重映射,B-5)
  py-bindings/ PyO3 _core 扩展。唯一用 unsafe(经 PyO3)的 crate。#![deny(unsafe_op_in_unsafe_fn)]
    src/lib.rs     open -> Presentation handle;Slide.shapes() / Slide.comments() / Presentation.diagnostics() -> list[dict];ocr_image / reconstruct_image_table;异常层级
```

## 跑(始终从包根)

```bash
uv venv .venv
VIRTUAL_ENV="$(pwd)/.venv" uv pip install maturin pytest
cargo build --workspace --release      # 期望编译干净(ocrspine + pdf-typeset 均 git dep,一并编译,首次较慢)
OCRSPINE_MODELS="$(cd ../ocrspine && pwd)/models" \
  VIRTUAL_ENV="$(pwd)/.venv" .venv/bin/maturin develop --release
OCRSPINE_MODELS="$(cd ../ocrspine && pwd)/models" \
  .venv/bin/python -m pytest python/tests -q   # 解析测试必过;OCR 测试需 models env;PDF 导出读回测试需 venv 里 `pip install pdfspine`
```

## Fuzzing(cargo-fuzz,把"绝不 panic"变成可证明)

`fuzz/` 是**独立 package + 独立 workspace**(根 `Cargo.toml` 里 `exclude = ["fuzz"]`),需要 nightly +
`cargo install cargo-fuzz`;不进 `ci.yml`,由 `.github/workflows/fuzz.yml` 每日跑(也可手动触发)。
只有 panic / abort / OOM(`-rss_limit_mb=2048`)算失败,任何 `Err` 都可接受。
**所有 cargo-fuzz 命令都显式写 `cargo +nightly`**:仓库 `rust-toolchain.toml` 钉住稳定版,不带 `+nightly`
会被它覆盖,报 "the option `Z` is only accepted on the nightly compiler"(CI 里同理)。

```bash
cargo run --manifest-path fuzz/Cargo.toml --bin make_seeds       # 现场生成种子到 fuzz/corpus/(已 .gitignore,不落二进制 fixture)
cargo +nightly fuzz run parse_slide_xml -- -max_total_time=120 -rss_limit_mb=2048
cargo +nightly fuzz run parse_pptx      -- -max_total_time=120 -rss_limit_mb=2048
cargo +nightly fuzz run parse_parts     -- -max_total_time=120 -rss_limit_mb=2048
cargo +nightly fuzz run render_pdf      -- -max_total_time=120 -rss_limit_mb=2048
```

- target:`parse_pptx`(任意字节 → `parse_bytes`)、`parse_slide_xml`(字节当 `ppt/slides/slide1.xml`,
  现场打成最小 pptx,直达 XML 层,收益最大)、`parse_parts`(首字节选部件种类 layout / master / theme / chart /
  tableStyles / notes / comments / diagram drawing / diagram data / presentation,其余字节当该部件 XML,其它部件取
  最小合法内容;slide 引用全部附属部件)、`render_pdf`(`PK` 开头按 pptx,否则当 slide1.xml;解析成功再
  `resolve` + `render_pdf`;渲染器无"确定性字体"开关,`with_system_fonts` 本就只用内置 Liberation 字体)。
- 解析成功的 target 末尾都跑 `fuzz/src/lib.rs` 的 `exercise_exports`(`to_text` / `to_markdown` 两种顺序 ×
  含隐藏页 × 带 / 不带终态 IR,外加 `resolve` 渲染映射)。没有 nightly / cargo-fuzz 时的最小检查:
  `cargo check --manifest-path fuzz/Cargo.toml --all-targets` + `cargo test --manifest-path fuzz/Cargo.toml --lib`
  (含按部件截断的确定性冒烟,不是真 fuzz)。
- 复现 crash:`cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<crash-file>`;最小化:
  `cargo +nightly fuzz tmin <target> <crash-file>`;`RUST_BACKTRACE=1` 看 panic 位置。
- 修复流程:在对应 crate 最小改动修掉,并往 `crates/ppt-parse/tests/fuzz_regressions.rs`(已有;或所属 crate 的
  单测)加**现场构造**的回归测试(不提交 crash 二进制)。
- 新增 target:`fuzz/fuzz_targets/<name>.rs` + `fuzz/Cargo.toml` 加 `[[bin]]` + `fuzz.yml` 的 matrix 加名字
  + `fuzz/seed.rs` 补种子;共用的打包帮助函数放 `fuzz/src/lib.rs`。
- `fuzz/Cargo.lock` 由根 `Cargo.lock` 拷贝而来以钉住依赖版本;根 workspace 依赖(git rev 等)变更后重新拷贝。

## 约定

- Python **3.12+**;Rust **2021** 边缘;import 顺序 **stdlib > 三方 > 本地**;简体中文 docstring/注释,
  匹配家族风格。
- **TDD**——测试即规格(`python/tests/conftest.py` 用纯 Python `zipfile` 合成最小 .pptx,不落二进制 fixture)。
- **最小改动**——只改需求要求的部分。
- **深层、按职责分组**的布局:crate / 文件路径先定位职责,再读文件名。
- 每个 crate `#![forbid(unsafe_code)]`,**唯独** `py-bindings` 用 `#![deny(unsafe_op_in_unsafe_fn)]`
  (PyO3 需要 unsafe FFI glue)。
