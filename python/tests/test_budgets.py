"""字节预算与共享在 Python 边界的表现:共享字符串在一次转换里是同一个 ``str`` 对象(不被放大回
N 份);模型字节 / 导出输出 / 渲染 op 预算可经关键字参数调整,超出时有诊断或 ``warnings``。"""

from __future__ import annotations

import warnings

import pytest
from pptx_synth import REL_BASE, SlideSpec, build_pptx

import pptspine


def _text_box(inner: str) -> str:
    return (
        '<p:sp><p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>'
        '<p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm></p:spPr>'
        f"<p:txBody><a:bodyPr/>{inner}</p:txBody></p:sp>"
    )


def _author_deck(n: int) -> bytes:
    authors = (
        '<p:cmAuthorLst xmlns:p="urn:p"><p:cmAuthor id="0" name="'
        + "A" * 3000
        + '" initials="AB"/></p:cmAuthorLst>'
    )
    cm = '<p:cmLst xmlns:p="urn:p">' + '<p:cm authorId="0"/>' * n + "</p:cmLst>"
    spec = SlideSpec("", [("rId1", f"{REL_BASE}/comments", "../comments/comment1.xml")])
    return build_pptx(
        [spec], {"ppt/comments/comment1.xml": cm, "ppt/commentAuthors.xml": authors}
    )


def test_shared_author_is_one_python_object_per_call():
    cs = pptspine.open_bytes(_author_deck(50)).slide(0).comments()
    assert len(cs) == 50
    assert all(c["author"] is cs[0]["author"] for c in cs)
    assert cs[0]["author"] == "A" * 3000


def test_shared_hyperlink_target_is_one_python_object_per_call():
    run = '<a:r><a:rPr><a:hlinkClick r:id="rId7"/></a:rPr><a:t>x</a:t></a:r>'
    spec = SlideSpec(
        _text_box(f"<a:p>{run * 20}</a:p>"),
        [("rId7", f"{REL_BASE}/hyperlink", 'https://example.com/a" TargetMode="External')],
    )
    runs = pptspine.open_bytes(build_pptx([spec])).slide(0).shapes()[0]["paragraphs"][0]["runs"]
    urls = [r["hyperlink"]["url"] for r in runs]
    assert urls[0] == "https://example.com/a"
    assert all(u is urls[0] for u in urls)


def test_max_model_bytes_truncates_with_diagnostic():
    body = "<a:p><a:r><a:t>" + "t" * 1000 + "</a:t></a:r></a:p>"
    data = build_pptx([SlideSpec(_text_box(body * 200))])
    full = pptspine.open_bytes(data)
    assert full.diagnostics() == []
    tight = pptspine.open_bytes(data, max_model_bytes=64 * 1024)
    kinds = {d["kind"] for d in tight.diagnostics()}
    assert kinds & {"content-truncated", "value-truncated"}
    assert len(tight.to_text()) < len(full.to_text())
    with pytest.raises(ValueError):
        pptspine.open_bytes(data, max_model_bytes=0)


def test_text_export_output_budget_warns_and_marks():
    body = "<a:p><a:r><a:t>" + "w" * 500 + "</a:t></a:r></a:p>"
    pres = pptspine.open_bytes(build_pptx([SlideSpec(_text_box(body * 40))]))
    assert "[pptspine: output truncated" not in pres.to_text()
    for fn in (pres.to_text, pres.to_markdown):
        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always")
            out = fn(max_output_bytes=2_000)
        assert any("output truncated" in str(x.message) for x in w)
        body_part, marker = out.rsplit("\n", 1)
        assert len(body_part.encode()) <= 2_000
        assert marker.startswith("[pptspine: output truncated")
    with pytest.raises(ValueError):
        pres.to_text(max_output_bytes=-1)


def test_pdf_render_budget_warns():
    shapes = "".join(
        _text_box(f"<a:p><a:r><a:t>box {i}</a:t></a:r></a:p>") for i in range(200)
    )
    pres = pptspine.open_bytes(build_pptx([SlideSpec(shapes)]))
    with warnings.catch_warnings(record=True) as w:
        warnings.simplefilter("always")
        small = pres.to_pdf(max_page_ops=20)
    assert any("render-budget" in str(x.message) for x in w)
    with warnings.catch_warnings(record=True) as w:
        warnings.simplefilter("always")
        full = pres.to_pdf()
    assert not any("render-budget" in str(x.message) for x in w)
    assert len(small) < len(full)


def test_truncated_flag_and_parse_report():
    one = "<a:p><a:r><a:t>x</a:t></a:r></a:p>"
    clean = pptspine.open_bytes(build_pptx([SlideSpec(_text_box(one))]))
    assert clean.truncated is False
    r = clean.parse_report()
    assert r["truncated"] is False and r["truncated_parts"] == []
    assert r["items_used"] == 2 and r["shapes_used"] == 1 and r["model_bytes"] > 0

    tight = pptspine.open_bytes(
        build_pptx([SlideSpec(_text_box(one * 50)) for _ in range(3)]), max_part_items=10
    )
    assert tight.truncated is True
    r = tight.parse_report()
    assert r["truncated_parts"] == [f"ppt/slides/slide{i}.xml" for i in (1, 2, 3)]
    # 每页 50 段 × (段 + run) = 100 个节点,保留 10 个;被跳过段落里的 run 也计入。
    assert r["dropped_items"] == 3 * 90


def test_limit_kwargs_report_accurately_and_are_documented():
    data = build_pptx([SlideSpec(_text_box("<a:p><a:r><a:t>x</a:t></a:r></a:p>"))])
    with pytest.raises(ValueError, match="too large"):
        pptspine.open_bytes(data, max_entries=2**70)
    with pytest.raises(ValueError, match="positive integer"):
        pptspine.open_bytes(data, max_entries=-(2**70))
    for name in (
        "max_slides", "max_comments", "max_part_shapes", "max_total_shapes",
        "max_part_items", "max_total_items", "max_model_bytes",
    ):
        assert name in pptspine.open.__doc__, name
        assert name in pptspine.open_bytes.__doc__, name
