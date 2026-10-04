#![no_main]
//! 首字节选部件种类(layout / master / theme / chart / tableStyles / notes / comments /
//! diagram drawing / diagram data / presentation),其余字节当作该部件的 XML,其它部件取最小合法内容,
//! 打成 pptx 再解析,跑文本 / Markdown 导出器与渲染映射(`resolve`)并渲染 PDF——让 fuzzer 直接打到
//! `parse_slide_xml` 够不着的附属部件(整包变异几乎都止步于 zip CRC)。只有 panic/abort/OOM 算失败。

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&selector, xml)) = data.split_first() else {
        return;
    };
    if let Ok(parsed) = ppt_parse::parse_bytes(&pptspine_fuzz::pack_part(selector, xml)) {
        let resolved = pptspine_fuzz::exercise_exports(&parsed);
        let _ = ppt_render::render_pdf(
            &resolved,
            &parsed.media,
            &ppt_render::RenderOptions::default(),
        );
    }
});
