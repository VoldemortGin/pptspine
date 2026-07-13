"""自渲染 SSIM 回归门(PRD-PDF-EXPORT §8 B-11 门(4))。

与 LibreOffice oracle(``scripts/lo_oracle_ssim.py``,**local-only、never-CI**)不同:
本门是**自渲染回归门**——把每个合成 fixture 经 ``pptspine`` ``to_pdf()`` 渲染、``pdfspine``
栅格化,和 git 里 committed 的灰度参考(``python/tests/ssim_refs/*.ssimref``)逐页算 SSIM,
要求 ≥ ``--min-ssim``(缺省 0.97)。参考在渲染稳定后一次性入库;渲染**有意**变更后用
``scripts/ssim_baseline.py --make-references`` 重生成,把有意更新与无意漂移分开。

门逻辑与 ``.ssimref`` 读写都在 ``scripts/ssim_baseline.py``(单一真相),本测试只做薄断言。

字体环境敏感(PRD §9 风险 5):只在**固定字体的 runner** 上强制,故默认 skip;
设环境变量 ``PPTSPINE_SSIM_GATE=1`` 启用(见 ``.github/workflows/ci.yml`` 的 ssim-gate step)。
"""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path

import pytest

pytest.importorskip("pdfspine")
pytest.importorskip("pptspine")

REPO_ROOT = Path(__file__).resolve().parents[2]

pytestmark = pytest.mark.skipif(
    not os.environ.get("PPTSPINE_SSIM_GATE"),
    reason="自渲染 SSIM 门仅在固定字体 runner 上强制;设 PPTSPINE_SSIM_GATE=1 启用",
)


def _load_baseline():
    path = REPO_ROOT / "scripts" / "ssim_baseline.py"
    spec = importlib.util.spec_from_file_location("ssim_baseline", path)
    assert spec is not None and spec.loader is not None, f"cannot load {path}"
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_self_render_ssim_gate():
    """全 fixture 矩阵逐页 SSIM(当前渲染 vs committed 参考)≥ 阈值,尺寸一致、无空白回归。"""
    mod = _load_baseline()
    payload = mod.check_references(min_ssim=mod.MIN_SSIM_DEFAULT)
    fails = [d for d in payload["docs"] if not d["passed"]]
    detail = "\n".join(
        f"  {d['name']}__p{d['page']}: ssim={d['ssim']} err={d['error']}" for d in fails
    )
    assert payload["passed"], (
        f"自渲染 SSIM 门失败 ({len(fails)}/{payload['n']} 页 < {mod.MIN_SSIM_DEFAULT} "
        f"或尺寸/空白回归):\n{detail}\n"
        "若为**有意**渲染变更,跑 `python scripts/ssim_baseline.py --make-references` 重生成基线。"
    )
