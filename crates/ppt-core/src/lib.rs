#![forbid(unsafe_code)]
//! `ppt-core` —— pptspine 的领域核:结构化 pptx 模型 + EMU 几何 + 类型化错误。
//!
//! 这里**没有任何 IO / zip / XML 逻辑**,只有纯数据类型,供 `ppt-parse` 填充、供
//! `py-bindings` 暴露。保持 domain-neutral、稳定、可测。

pub mod error;
pub mod geom;
pub mod model;

pub use error::{PptError, Result};
pub use geom::{emu_to_points, Emu, Point, Rect, EMU_PER_INCH, EMU_PER_POINT};
pub use model::{
    AutoShape, Cell, Color, Paragraph, Picture, Presentation, Row, Shape, Slide, Table, TextFrame,
    TextRun,
};
