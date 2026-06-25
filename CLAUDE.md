# CLAUDE.md — pptspine(宪章)

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
- **缝的元模式(家族统一)。** 唯一外部能力(OCR)经 Protocol seam 接入:`OcrEngine`(来自
  `ocrspine`)是协议,`PaddleOcr` 是确定性默认实现。core 只依赖协议,**绝不**直接 import 任何
  推理 SDK。
- **最小、优雅,不过度设计。** 文本 + 表格是**必须项**;图形/颜色尽力而为。痛了再抽,带证据抽。

## 铁律(本仓特有)

- **`../pdfspine/` 只读。** 另一个 agent 正在改它。可读它学模式(PyO3 chokepoint / 工作区布局),
  但**绝不**写入或修改 pdfspine 里的**任何**文件。
- **依赖 `../ocrspine`(path 依赖)。** 在 `[workspace.dependencies]` 里一次性声明
  `ocrspine = { path = "../ocrspine" }`,`ppt-ocr` 用 `ocrspine.workspace = true`,避免逐 crate
  算相对路径。

## 模块地图(按 crate 定位)

```
crates/
  ppt-core/    领域模型 + 几何(EMU) + 类型化 PptError。无 IO / zip / XML。#![forbid(unsafe_code)]
    src/error.rs   PptError(thiserror):Zip/Xml/Unsupported/InvalidArgument/Io/Ocr + kind() + Result<T>
    src/geom.rs    Emu(i64,914400/inch) + to_points + Rect/Point
    src/model.rs   Presentation/Slide/Shape/TextFrame/Paragraph/TextRun/Table/Row/Cell/Picture/AutoShape/Color
  ppt-parse/   OOXML 读取:zip 解包 + quick-xml 遍历 -> Presentation。本轮核心。#![forbid(unsafe_code)]
    src/lib.rs     parse_path / parse_bytes -> ParsedPptx { presentation, media }
    src/zip_pkg.rs zip 读 API:presentation.xml / slides / _rels / media / layouts / masters
    src/xml/       quick-xml walker:presentation.rs(尺寸+顺序) slide.rs(spTree -> Shape)
  ppt-ocr/     图片 OCR 桥:把 ocrspine 套到嵌入图片上。本轮薄但可用。#![forbid(unsafe_code)]
    src/lib.rs     ocr_image_bytes / PptOcr{engine} + reconstruct_table_from_image(stub)
  py-bindings/ PyO3 _core 扩展。唯一用 unsafe(经 PyO3)的 crate。#![deny(unsafe_op_in_unsafe_fn)]
    src/lib.rs     open -> Presentation handle;Slide.shapes() -> list[dict];ocr_image;异常层级
```

## 跑(始终从包根)

```bash
uv venv .venv
VIRTUAL_ENV="$(pwd)/.venv" uv pip install maturin pytest
cargo build --workspace --release      # 期望编译干净(ocrspine 一并编译,首次较慢)
OCRSPINE_MODELS="$(cd ../ocrspine && pwd)/models" \
  VIRTUAL_ENV="$(pwd)/.venv" .venv/bin/maturin develop --release
OCRSPINE_MODELS="$(cd ../ocrspine && pwd)/models" \
  .venv/bin/python -m pytest python/tests -q   # 解析测试必过;OCR 测试需 models env
```

## 约定

- Python **3.11+**;Rust **2021** 边缘;import 顺序 **stdlib > 三方 > 本地**;简体中文 docstring/注释,
  匹配家族风格。
- **TDD**——测试即规格(`python/tests/conftest.py` 用纯 Python `zipfile` 合成最小 .pptx,不落二进制 fixture)。
- **最小改动**——只改需求要求的部分。
- **深层、按职责分组**的布局:crate / 文件路径先定位职责,再读文件名。
- 每个 crate `#![forbid(unsafe_code)]`,**唯独** `py-bindings` 用 `#![deny(unsafe_op_in_unsafe_fn)]`
  (PyO3 需要 unsafe FFI glue)。
