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
    /// SmartArt 没有可用的 drawing 部件(缺失 / 畸形 / 为空)或超出展开预算而降级:渲染为占位框,
    /// 仅 data 部件文字保留。`part` = data 部件(data 部件不存在 / 关系找不到时为持有该关系的
    /// 源 slide 部件),`count` = 受影响的 frame 数。
    SmartArtDegraded,
    /// 图表部件存在但解析不出可用数据(XML 不良构或无系列)或超出数据点预算而降级。`part` = 图表部件,
    /// `count` = 受影响的 frame 数。
    ChartDegraded,
    /// `a:custGeom` 超过参考线 / 路径 / 命令数预算而降级:渲染按包围盒近似。`part` = 所在部件,
    /// `count` = 被降级的 `custGeom` 数。
    CustomGeometryDegraded,
    /// 同一张幻灯片的 `.rels` 里有多条 `comments` 关系指向同一批注部件,只保留首次。
    /// `part` = 被重复引用的批注部件,`count` = 被去掉的重复引用数。
    DuplicateCommentRef,
    /// 批注总条数(含回复)超过 `ZipLimits::max_comments`,超出部分被截断。`part` = 被截断的
    /// 批注部件,`count` = 发生截断的次数(每张受影响的幻灯片记一次;解析在上限处提前停止,
    /// 被丢弃的确切条数不可知)。
    CommentsTruncated,
    /// 单个部件的形状数超过 `ZipLimits::max_part_shapes`,或整个演示文稿累计超过
    /// `ZipLimits::max_total_shapes`:该部件的形状解析提前停止(已解析的保留),其余形状被丢弃。
    /// `part` = 被截断的部件,`count` = 被丢弃的形状元素数(组合内被整体跳过的后代不另计)。
    ShapesTruncated,
    /// 单个部件的文本 / 表格节点(段落、run、表格行 / 单元格 / 网格列、渐变停靠点、颜色变换、
    /// 形状调节值)超过 `ZipLimits::max_part_items`,或整个演示文稿累计超过
    /// `ZipLimits::max_total_items`:超出的节点被丢弃。`part` = 被截断的部件,`count` = 被丢弃
    /// 的节点数。
    ContentTruncated,
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
            DiagnosticKind::CustomGeometryDegraded => "custom-geometry-degraded",
            DiagnosticKind::DuplicateCommentRef => "duplicate-comment-ref",
            DiagnosticKind::CommentsTruncated => "comments-truncated",
            DiagnosticKind::ShapesTruncated => "shapes-truncated",
            DiagnosticKind::ContentTruncated => "content-truncated",
        }
    }
}

/// 一条解析诊断。同一 `(kind, part)` 只出现一次,重复发生累加进 `count`。不同条目总数有上限
/// (10 000):超出后新条目并入每种 kind 一条的汇总条目,其 `part` 为空串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    /// 所在部件路径(**包内真实存在**的部件,如 `ppt/slides/slide1.xml`;绝不是文件里写的任意
    /// 目标串)。空串 = 汇总条目(条数超限后的合并,或没有可用的真实部件路径)。
    pub part: String,
    /// 计数,语义见各 [`DiagnosticKind`] 变体。
    pub count: usize,
}
