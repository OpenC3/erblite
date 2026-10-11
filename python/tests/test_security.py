# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

"""Sandbox / escape-attempt tests."""

from __future__ import annotations

import os
import re
from pathlib import Path

import pytest

import erblite

PKG = Path(__file__).resolve().parents[1] / "erblite"


def err(src: str, **kw: object) -> erblite.Error:
    with pytest.raises(erblite.Error) as info:
        erblite.render(src, **kw)  # type: ignore[arg-type]
    return info.value


@pytest.mark.parametrize(
    "src",
    [
        "<%= ''.__class__ %>",
        "<%= ().__class__ %>",
        "<%= nil.__class__.__bases__ %>",
        "<%= [].__len__ %>",
        "<%= {}.__dict__ %>",
        "<%= 1.__add__(2) %>",
        "<%= 'x'.__getattribute__('upper') %>",
        "<%= __import__('os') %>",
        "<%= 1.send(:+, 2) %>",
        "<%= 1.public_send(:+, 2) %>",
        "<%= 1.__send__(:+, 2) %>",
        "<%= 'x'.instance_eval('upcase') %>",
        "<%= 'x'.instance_exec { 1 } %>",
        "<%= 1.method(:+) %>",
        "<%= 'x'.instance_variable_get(:@a) %>",
        "<%= 'x'.instance_variables %>",
        "<%= String.new('x') %>",
        "<%= Object.new %>",
        "<%= Object.const_get(:Kernel) %>",
        "<%= Integer.class_eval { 1 } %>",
        "<%= Integer.define_method(:x) { 1 } %>",
        "<%= 1.class.ancestors %>",
        "<%= eval('1 + 1') %>",
        "<%= system('ls') %>",
        "<%= exec('ls') %>",
        "<%= spawn('ls') %>",
        "<%= require 'socket' %>",
        "<%= load 'x.rb' %>",
        "<%= open('data/lines.txt') %>",
        "<%= proc { 1 } %>",
        "<%= lambda { 1 } %>",
        "<%= File.write('x.txt', 'y') %>",
        "<%= File.open('x.txt') %>",
        "<%= File.delete('x.txt') %>",
        "<%= Dir.mkdir('x') %>",
        "<%= Dir.entries('.') %>",
        "<%= JSON.load('1') %>",
        "<%= Math.send(:sqrt, 4) %>",
        "<%= :upcase.to_proc %>",
        "<%= [1].each.next %>",
    ],
)
def test_no_method_escapes(src: str) -> None:
    assert err(src).type == "NoMethodError"


@pytest.mark.parametrize(
    "src",
    [
        "<%= `ls` %>",
        "<%= %x(ls) %>",
        "<%= $stdout %>",
        "<%= $0 %>",
        "<%= @x %>",
        "<%= @@x %>",
        "<% def foo; end %>",
        "<% class Foo; end %>",
        "<% module Foo; end %>",
        "<%= ->(x) { x } %>",
        "<% begin; rescue; end %>",
        "<%= self %>",
        "<%= __FILE__ %>",
        "<%= __dir__ %>",
        "<%= defined?(x) %>",
        "<% return %>",
        "<%= yield %>",
        "<%= super %>",
        "<% alias a b %>",
        "<%= 'a' =~ /a/ %>",
        "<%= [*[1]] %>",
        "<% x = <<~EOS\nhi\nEOS\n%>",
        '<%= "#@x" %>',
        '<%= "#$0" %>',
        "<% X = 1 %>",
    ],
)
def test_syntax_level_escapes(src: str) -> None:
    assert err(src).type == "SyntaxError"


@pytest.mark.parametrize(
    "src",
    [
        "<%= binding %>",
        "<%= fork %>",
        "<%= exit %>",
        "<%= abort %>",
        "<%= __builtins__ %>",
        "<%= __class__ %>",
        "<%= Kernel.exit %>",
        "<%= ENV['HOME'] %>",
        "<%= ObjectSpace %>",
        "<%= Process.pid %>",
        "<%= IO %>",
    ],
)
def test_unknown_constants(src: str) -> None:
    assert err(src).type == "NameError"


def test_respond_to_uses_allowlist() -> None:
    out = erblite.render(
        "<%= 'x'.respond_to?(:upcase) %>,<%= 'x'.respond_to?(:__class__) %>,<%= 1.respond_to?(:send) %>,"
        "<%= 1.respond_to?('instance_eval') %>,<%= [].respond_to?(:each) %>"
    )
    assert out == "true,false,false,false,true"


def test_no_base_dir_means_no_file_access() -> None:
    for src in [
        "<%= File.read('x') %>",
        "<%= File.exist?('x') %>",
        "<%= Dir.glob('*') %>",
        "<%= render('x') %>",
    ]:
        assert err(src).type == "SecurityError"
    # pure path functions still work
    assert erblite.render("<%= File.join('a', 'b') %>,<%= File.basename('/x/y.rb', '.rb') %>") == "a/b,y"


@pytest.fixture()
def sandbox(tmp_path: Path) -> Path:
    base = tmp_path / "base"
    (base / "sub").mkdir(parents=True)
    (base / "ok.txt").write_text("inside")
    (base / "sub" / "deep.txt").write_text("deep")
    (tmp_path / "secret.txt").write_text("SECRET")
    (tmp_path / "outdir").mkdir()
    (tmp_path / "outdir" / "o.txt").write_text("SECRET")
    os.symlink(tmp_path / "secret.txt", base / "link.txt")
    os.symlink(tmp_path / "outdir", base / "linkdir")
    os.symlink(base / "ok.txt", base / "goodlink.txt")
    return base


@pytest.mark.parametrize(
    "src",
    [
        "<%= File.read('../secret.txt') %>",
        "<%= File.read('sub/../../secret.txt') %>",
        "<%= File.read('link.txt') %>",
        "<%= File.read('linkdir/o.txt') %>",
        "<%= File.read(File.join('..', 'secret.txt')) %>",
        "<%= File.read('/etc/passwd') %>",
        "<%= render('../secret.txt') %>",
        "<%= render('link.txt') %>",
        "<%= Dir.glob('../*') %>",
        "<%= Dir.glob('/etc/*') %>",
        "<%= Dir.glob('**/../../*') %>",
        "<%= Dir.glob('{ok.txt,../*}') %>",
    ],
)
def test_path_escapes(sandbox: Path, src: str) -> None:
    e = err(src, base_dir=str(sandbox))
    assert e.type == "SecurityError", str(e)
    assert "SECRET" not in e.message


def test_sandbox_allowed_paths(sandbox: Path) -> None:
    out = erblite.render(
        "<%= File.read('ok.txt') %>,<%= File.read('sub/../ok.txt') %>,<%= File.read('goodlink.txt') %>,"
        f"<%= File.read('{sandbox}/sub/deep.txt') %>,<%= File.exist?('link.txt') %>,"
        "<%= File.exist?('../secret.txt') %>,"
        "<%= File.file?('ok.txt') %>,<%= File.directory?('sub') %>",
        base_dir=str(sandbox),
    )
    assert out == "inside,inside,inside,deep,false,false,true,true"


def test_glob_does_not_leak_symlinked_content(sandbox: Path) -> None:
    out = erblite.render("<%= Dir.glob('**/*') %>|<%= Dir.glob('linkdir/*') %>", base_dir=str(sandbox))
    assert "o.txt" not in out
    assert '"link.txt"' not in out
    assert out.startswith('["goodlink.txt", "ok.txt", "sub", "sub/deep.txt"]')


def test_missing_file_and_null_bytes(sandbox: Path) -> None:
    assert err("<%= File.read('nope.txt') %>", base_dir=str(sandbox)).type == "IOError"
    assert err("<%= File.read('sub') %>", base_dir=str(sandbox)).type == "IOError"
    assert err('<%= File.read("ok.txt\\0") %>', base_dir=str(sandbox)).type == "ArgumentError"


def test_glob_pattern_is_not_regex(sandbox: Path) -> None:
    # Pathological wildcard patterns must run in polynomial time and not raise Python errors.
    (sandbox / ("a" * 200)).write_text("")
    out = erblite.render(
        "<%= Dir.glob('*a*a*a*a*a*a*a*a*a*a*a*a*b').size %>,<%= Dir.glob('[z-a]*') %>", base_dir=str(sandbox)
    )
    assert out == "0,[]"


def test_max_steps_stops_runaway_templates() -> None:
    for src in [
        "<% loop { } %>",
        "<% while true; end %>",
        "<% x = 0 %><% until false; x += 1; end %>",
        "<% (1..).each { } %>",
        "<% 1.step { } %>",
        "<% [1].cycle { } rescue 1 %>",
    ]:
        try:
            erblite.render(src, max_steps=5000)
        except erblite.Error as e:
            assert e.type in ("LimitError", "SyntaxError", "NoMethodError"), str(e)
        else:
            raise AssertionError(src)


def test_nesting_limits() -> None:
    deep = "<%= " + "(" * 400 + "1" + ")" * 400 + " %>"
    assert err(deep).type == "LimitError"
    deep_arr = "<%= " + "[" * 400 + "]" * 400 + " %>"
    assert err(deep_arr).type == "LimitError"
    ok = "<%= " + "(" * 100 + "1" + ")" * 100 + " %>"
    assert erblite.render(ok) == "1"


def test_recursive_render_limit(tmp_path: Path) -> None:
    (tmp_path / "r.erb").write_text("<%= render('r.erb') %>")
    assert err("<%= render('r.erb') %>", base_dir=str(tmp_path)).type == "LimitError"


def test_resource_guards() -> None:
    assert err("<%= 'x' * 1_000_000_000 %>").type == "LimitError"
    assert err("<%= [0] * 1_000_000_000 %>").type == "LimitError"
    assert err("<%= Array.new(1_000_000_000) %>").type == "LimitError"
    assert err("<%= 2 ** 1_000_000_000 %>").type == "RangeError"
    assert err("<%= 1 << 1_000_000_000 %>").type == "RangeError"
    assert err("<%= format('%999999999d', 1) %>").type == "LimitError"
    assert err("<% a = [1].freeze %><% a << 2 %>").type == "FrozenError"


def test_recursive_structures_do_not_crash() -> None:
    assert erblite.render("<% a = [] %><% a << a %><%= a.inspect %>") == "[[...]]"
    assert err("<% a = [] %><% a << a %><%= a.flatten %>").type == "ArgumentError"
    assert err("<% a = [] %><% a << a %><%= a.join %>").type == "ArgumentError"
    assert err("<% a = [] %><% a << a %><%= JSON.generate(a) %>").type == "JSON::GeneratorError"


def test_no_dynamic_code_in_source() -> None:
    """The implementation must not use dynamic evaluation or attribute dispatch."""
    banned = re.compile(r"(?<![.\w])(eval|exec|compile|getattr|setattr|__import__|importlib|pickle|subprocess)\s*\(")
    banned_mod = re.compile(r"^\s*(import|from)\s+(importlib|pickle|subprocess|marshal|ctypes)\b", re.M)
    for path in PKG.glob("*.py"):
        text = path.read_text()
        assert not banned.search(text), path.name
        assert not banned_mod.search(text), path.name


def test_values_are_plain_data() -> None:
    # Host objects cannot be passed in.
    class Evil:
        pass

    assert err("<%= x %>", variables={"x": Evil()}).type == "TypeError"
    assert err("<%= x %>", variables={"x": 2**70}).type == "RangeError"
