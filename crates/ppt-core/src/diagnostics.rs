//! 解析诊断:记录"内容被静默丢失 / 降级"的结构化事实,供调用方(如 RAG 管线)判断
//! 解析结果是否完整。每条只含种类、部件路径、计数——**绝不含文档正文**。

/// 诊断种类。`#[non_exhaustive]`:后续可增补;稳定标识见 [`DiagnosticKind::code`]
/// (kebab-case,风格同渲染告警码)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// 部件 XML 中途损坏 / 被截断(读取错误,或 EOF 时标签仍未闭合):已解析的前缀保留,
    /// 之后的内容丢失。`count` = 解析停止处的字节偏移。
    XmlTruncated,
    /// 形状树 / `mc:AlternateContent` 嵌套超过上限,整棵子树被跳过。`count` = 被跳过的子树数。
    NestingTooDeep,
    /// `p:sldIdLst` 重复引用同一幻灯片部件,只保留首次。`part` = 被重复引用的 slide 部件,
    /// `count` = 被去掉的重复引用数。
    DuplicateSlideRef,
    /// 关系(`.rels`)指向包内不存在的部件。`part` = 持有该关系的源部件,`count` = 悬空关系数
    /// (外部链接不算)。
    MissingPart,
    /// SmartArt 没有可用的 drawing 部件(缺失 / 畸形 / 为空)而降级:渲染为占位框,
    /// 仅 data 部件文字保留。`part` = data 部件,`count` = 受影响的 frame 数。
    SmartArtDegraded,
    /// 图表部件存在但解析不出可用数据(XML 不良构或无系列)而降级。`part` = 图表部件,
    /// `count` = 受影响的 frame 数。
    ChartDegraded,
}

impl DiagnosticKind {
    /// 稳定的 kebab-case 标识(Python 侧 `kind` 字段)。
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            DiagnosticKind::XmlTruncated => "xml-truncated",
            DiagnosticKind::NestingTooDeep => "nesting-too-deep",
            DiagnosticKind::DuplicateSlideRef => "duplicate-slide-ref",
            DiagnosticKind::MissingPart => "missing-part",
            DiagnosticKind::SmartArtDegraded => "smartart-degraded",
            DiagnosticKind::ChartDegraded => "chart-degraded",
        }
    }
}

/// 一条解析诊断。同一 `(kind, part)` 只出现一次,重复发生累加进 `count`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    /// 所在部件路径(包内路径,如 `ppt/slides/slide1.xml`)。
    pub part: String,
    /// 计数,语义见各 [`DiagnosticKind`] 变体。
    pub count: usize,
}
