# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

"""Host API and CLI tests."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

import erblite


def test_render_variables() -> None:
    out = erblite.render(
        "<%= s %>|<%= i %>|<%= f %>|<%= b %>|<%= n.inspect %>|<%= a.inspect %>|"
        "<%= t.inspect %>|<%= h.inspect %>|<%= y.inspect %>",
        {
            "s": "str",
            "i": 1,
            "f": 1.5,
            "b": False,
            "n": None,
            "a": [1, "x"],
            "t": (1, 2),
            "h": {"k": {"z": [1]}},
            "y": erblite.Symbol("sym"),
        },
    )
    assert out == 'str|1|1.5|false|nil|[1, "x"]|[1, 2]|{"k" => {"z" => [1]}}|:sym'


def test_host_variables_are_locals() -> None:
    # Host variables behave like Ruby locals: `x -1` is subtraction, blocks can assign them.
    assert erblite.render("<%= x -1 %>", {"x": 5}) == "4"
    assert erblite.render("<% [1, 2].each { |i| t = t + i } %><%= t %>", {"t": 10}) == "13"
    assert erblite.render("<%= defined_later %>", {"defined_later": "v"}) == "v"


def test_template_reuse_with_different_variables() -> None:
    t = erblite.Template("<%= x -1 %>")
    assert t.render({"x": 3}) == "2"
    with pytest.raises(erblite.Error) as info:
        t.render()  # without a local `x`, Ruby parses `x -1` as the call x(-1)
    assert info.value.type == "NoMethodError"


def test_error_attributes() -> None:
    with pytest.raises(erblite.Error) as info:
        erblite.render("a\nb\n<%= nil.zork %>", filename="page.erb")
    e = info.value
    assert (e.type, e.filename, e.line) == ("NoMethodError", "page.erb", 3)
    assert "zork" in e.message
    assert str(e) == f"page.erb:3: NoMethodError: {e.message}"


def test_syntax_error_raised_at_construction() -> None:
    with pytest.raises(erblite.Error) as info:
        erblite.Template("x\n<%= 1 ", filename="t.erb")
    assert (info.value.type, info.value.line) == ("SyntaxError", 2)


def test_error_in_partial_reports_partial_location(tmp_path: Path) -> None:
    (tmp_path / "p.erb").write_text("ok\n<%= nope %>")
    with pytest.raises(erblite.Error) as info:
        erblite.render("<%= render 'p.erb' %>", base_dir=str(tmp_path), filename="main.erb")
    assert (info.value.type, info.value.filename, info.value.line) == ("NameError", "p.erb", 2)


def test_render_file_defaults_base_dir(tmp_path: Path) -> None:
    (tmp_path / "data.json").write_text('{"n": [1, 2]}')
    (tmp_path / "_p.erb").write_text("P<%= v %>")
    tpl = tmp_path / "t.erb"
    tpl.write_text("<%= JSON.parse(File.read('data.json'))['n'].sum %><%= render('_p.erb', v: 1) %>")
    assert erblite.render_file(tpl) == "3P1"
    assert erblite.render_file(str(tpl), {}) == "3P1"


def test_max_steps() -> None:
    with pytest.raises(erblite.Error) as info:
        erblite.render("<% 1000.times { } %>", max_steps=100)
    assert info.value.type == "LimitError"
    assert erblite.render("<% 10.times { } %>ok", max_steps=1000) == "ok"


def test_cli(tmp_path: Path) -> None:
    tpl = tmp_path / "t.erb"
    tpl.write_text("<%= name %>:<%= File.read('x.txt') %>")
    (tmp_path / "x.txt").write_text("X")
    vars_file = tmp_path / "v.json"
    vars_file.write_text(json.dumps({"name": "N"}))
    r = subprocess.run(
        [sys.executable, "-m", "erblite", str(tpl), "--vars", str(vars_file)],
        capture_output=True,
        text=True,
        cwd=str(Path(__file__).resolve().parents[1]),
    )
    assert r.returncode == 0, r.stderr
    assert r.stdout == "N:X"
    bad = tmp_path / "bad.erb"
    bad.write_text("<%= nil.x %>")
    r = subprocess.run(
        [sys.executable, "-m", "erblite", str(bad)],
        capture_output=True,
        text=True,
        cwd=str(Path(__file__).resolve().parents[1]),
    )
    assert r.returncode == 1
    assert "NoMethodError" in r.stderr
