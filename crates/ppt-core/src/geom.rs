//! pptx 原生几何单位 EMU(English Metric Units)与简单几何体。
//!
//! OOXML 的位置/尺寸一律以 EMU 表示:`914400 EMU = 1 inch`,`12700 EMU = 1 point`。
//! 这里保持 domain-neutral —— 只放单位换算和裸几何,不掺任何业务语义。

/// 每英寸的 EMU 数。
pub const EMU_PER_INCH: i64 = 914_400;

/// 每磅(point)的 EMU 数。
pub const EMU_PER_POINT: f64 = 12_700.0;

/// English Metric Units —— pptx 的原生长度单位(i64)。
pub type Emu = i64;

/// 把 EMU 换算成磅(point,1/72 英寸)。
#[inline]
pub fn emu_to_points(emu: Emu) -> f64 {
    emu as f64 / EMU_PER_POINT
}

/// 一个二维点(EMU)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: Emu,
    pub y: Emu,
}

impl Point {
    pub const fn new(x: Emu, y: Emu) -> Self {
        Point { x, y }
    }
}

/// 一个形状的位置 + 尺寸(EMU)。`x`/`y` 是左上角偏移(`a:off`),`w`/`h` 是范围(`a:ext`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: Emu,
    pub y: Emu,
    pub w: Emu,
    pub h: Emu,
}

impl Rect {
    pub const fn new(x: Emu, y: Emu, w: Emu, h: Emu) -> Self {
        Rect { x, y, w, h }
    }

    /// 以磅为单位的 `(x, y, w, h)` 四元组。
    pub fn to_points(self) -> (f64, f64, f64, f64) {
        (
            emu_to_points(self.x),
            emu_to_points(self.y),
            emu_to_points(self.w),
            emu_to_points(self.h),
        )
    }
}
