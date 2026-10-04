#![forbid(unsafe_code)]
//! `ppt-core` —— pptspine 的领域核:结构化 pptx 模型 + EMU 几何 + 类型化错误。
//!
//! 这里**没有任何 IO / zip / XML 逻辑**,只有纯数据类型,供 `ppt-parse` 填充、供
//! `py-bindings` 暴露。保持 domain-neutral、稳定、可测。

pub mod color;
pub mod custgeom;
pub mod diagnostics;
pub mod error;
pub mod export;
pub mod geom;
pub mod model;
pub mod model_bytes;
pub mod resolved;
pub mod style;
pub mod theme;

pub use color::{apply_transforms, ColorSpec, ColorTransform, ResolvedColor};
pub use diagnostics::{Diagnostic, DiagnosticKind};
pub use error::{LimitKind, PptError, Result};
pub use export::{
    presentation_markdown, presentation_markdown_with, presentation_text, presentation_text_with,
    slide_text, slide_text_with, ExportOptions, TextOrder,
};
pub use geom::{emu_to_points, Emu, Point, Rect, EMU_PER_INCH, EMU_PER_POINT};
pub use model::{
    AutoShape, BlipFill, Cell, Chart, ChartKind, ChartSeries, Color, DocProperties, Fill,
    GroupShape, Hyperlink, Paragraph, Picture, Presentation, RelRect, Row, Section, Shape, Slide,
    Table, TextFrame, TextRun, Xfrm,
};
pub use resolved::{ResolvedPresentation, ResolvedShape, ResolvedSlide};
pub use style::{
    Bullet, FontRef, PlaceholderRef, RunStyle, ShapeStyle, StyleMatrixRef, TextLevelStyle,
    TextStyleLevels, TxStyles,
};
pub use theme::{ClrMap, ColorScheme, FontScheme, FontSet, Theme, ThemeLine};
