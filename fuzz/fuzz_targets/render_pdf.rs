#![no_main]
//! 任意字节 -> `parse_bytes`;解析成功再 `resolve` + `ppt_render::render_pdf`。只有 panic/abort/OOM 算失败。
//! 输入以 `PK` 开头按完整 pptx 解析;否则当作 `slide1.xml` 现场打包(zip 的 CRC 让变异几乎都止步于
//! 容器层,这样 fuzzer 才能真正打到排版引擎)。
//! 渲染器没有"确定性字体"开关:`Typesetter::with_system_fonts` 只用 pdf-typeset 内置的 Liberation 字体,
//! 缺省 `RenderOptions`(空 `font_map`)不读任何外部字体文件,已是确定且跨机器一致的。

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let parsed = if data.starts_with(b"PK") {
        ppt_parse::parse_bytes(data)
    } else {
        ppt_parse::parse_bytes(&pptspine_fuzz::pack_slide_xml(data))
    };
    if let Ok(parsed) = parsed {
        let resolved = pptspine_fuzz::exercise_exports(&parsed);
        let _ = ppt_render::render_pdf(
            &resolved,
            &parsed.media,
            &ppt_render::RenderOptions::default(),
        );
    }
});
