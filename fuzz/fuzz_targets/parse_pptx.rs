#![no_main]
//! 任意字节 -> `parse_bytes`(缺省 `ZipLimits`);解析成功再跑导出器与渲染映射。`Err` 都可接受;
//! 只有 panic/abort/OOM 算失败。

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(parsed) = ppt_parse::parse_bytes(data) {
        pptspine_fuzz::exercise_exports(&parsed);
    }
});
