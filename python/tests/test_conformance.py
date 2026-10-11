# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

"""Runs the shared conformance suite in ../conformance/cases (see conformance/README.md)."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

import erblite

ROOT = Path(__file__).resolve().parents[2] / "conformance"
FIXTURES = ROOT / "fixtures"


def _cases() -> list:
    out = []
    for path in sorted((ROOT / "cases").glob("*.json")):
        for case in json.loads(path.read_text(encoding="utf-8")):
            out.append(pytest.param(case, id=f"{path.stem}/{case['name']}"))
    return out


@pytest.mark.parametrize("case", _cases())
def test_conformance(case: dict) -> None:
    kwargs = {"base_dir": str(FIXTURES), "filename": "test.erb", "max_steps": case.get("max_steps")}
    if "error" in case:
        with pytest.raises(erblite.Error) as info:
            erblite.render(case["template"], case.get("vars") or {}, **kwargs)
        assert info.value.type == case["error"], str(info.value)
        if "line" in case:
            assert info.value.line == case["line"], str(info.value)
    else:
        out = erblite.render(case["template"], case.get("vars") or {}, **kwargs)
        assert out == case["expected"]
