"""内嵌图片字节闭环验收:解析 pptx -> 取某张内嵌图片字节 -> 喂给 ``ocr_image`` 端到端跑通。"""

from __future__ import annotations

import pptspine


def test_picture_shape_carries_media_and_rel_id(image_pptx):
    pptx_bytes, media_name, _png = image_pptx
    pres = pptspine.open_bytes(pptx_bytes)
    shapes = pres.slides()[0].shapes()
    pics = [s for s in shapes if s["kind"] == "picture"]
    assert len(pics) == 1
    pic = pics[0]
    # picture dict 保留 media / rel_id,便于查回字节。
    assert pic["media"] == media_name
    assert pic["rel_id"] == "rId1"
    assert pic["image_bytes_len"] > 0


def test_image_bytes_roundtrip(image_pptx):
    pptx_bytes, media_name, png = image_pptx
    pres = pptspine.open_bytes(pptx_bytes)

    # media 名可枚举。
    assert media_name in pres.media_names()

    # image_bytes 取回的字节与原始 PNG 完全一致。
    got = pres.image_bytes(media_name)
    assert isinstance(got, bytes)
    assert got == png

    # 不存在的名字返回 None,绝不 panic。
    assert pres.image_bytes("nope.png") is None


def test_ocr_closure_end_to_end(image_pptx):
    """解析 pptx -> image_bytes -> ocr_image 闭环:识别出已知参考行。"""
    pptx_bytes, media_name, _png = image_pptx
    pres = pptspine.open_bytes(pptx_bytes)

    img = pres.image_bytes(media_name)
    assert img is not None

    items = pptspine.ocr_image(img)
    assert isinstance(items, list) and items

    joined = "".join(ch for it in items for ch in it["text"] if not ch.isspace())
    assert "pdfspineOCRtest2026" in joined
