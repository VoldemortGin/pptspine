#![no_main]
//! 字节当 `ppt/slides/slide1.xml`,现场包进最小 pptx(含 layout / master / theme / 图片 / 超链接 rels),
//! 绕开 zip CRC 直达 XML 层;解析成功再跑导出器与渲染映射。`Err` 都可接受;只有 panic/abort/OOM 算失败。

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(parsed) = ppt_parse::parse_bytes(&pptspine_fuzz::pack_slide_xml(data)) {
        pptspine_fuzz::exercise_exports(&parsed);
    }
});
