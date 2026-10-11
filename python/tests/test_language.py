# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

"""Focused tests for ERBLite-specific ([ERBLite]) behavior and implementation details."""

from __future__ import annotations

import pytest

import erblite


def r(src: str, **kw: object) -> str:
    return erblite.render(src, **kw)  # type: ignore[arg-type]


def err_type(src: str) -> str:
    with pytest.raises(erblite.Error) as info:
        erblite.render(src)
    return info.value.type


def test_integer_overflow_is_range_error() -> None:
    for src in [
        "<%= 9223372036854775807 + 1 %>",
        "<%= -9223372036854775808 - 1 %>",
        "<%= 3037000500 * 3037000500 %>",
        "<%= 2 ** 63 %>",
        "<%= 1 << 63 %>",
        "<%= -(-9223372036854775807 - 1) %>",
        "<%= (-9223372036854775807 - 1).abs %>",
        "<%= (-9223372036854775807 - 1) / -1 %>",
        "<%= 1e19.to_i %>",
        "<%= Integer('9223372036854775808') %>",
        "<%= '99999999999999999999'.to_i %>",
        "<%= 2 ** -1 %>",
    ]:
        assert err_type(src) == "RangeError", src
    assert r("<%= 9223372036854775807 %>,<%= -9223372036854775808 %>,<%= 2 ** 62 %>") == (
        "9223372036854775807,-9223372036854775808,4611686018427387904"
    )


def test_strings_are_values() -> None:
    assert r("<% s = 'a'; t = s; s << 'b' %><%= s %>,<%= t %>") == "ab,a"
    assert r("<% h = {k: 'x'} %><% h[:k] << 'y' << 'z' %><%= h[:k] %>") == "xyz"
    assert r("<% s = 'a' %><% s << 66 %><%= s %>") == "aB"
    assert r("<%= 'a' << 'b' %>") == "ab"


def test_string_count_is_substring_count() -> None:
    assert r("<%= 'abcabc'.count('bc') %>,<%= 'hello'.count('l') %>,<%= 'aaaa'.count('aa') %>") == "2,2,2"


def test_combination_family_returns_arrays() -> None:
    assert r("<%= [1, 2, 3].combination(2).length %>,<%= [1, 2].permutation.inspect %>") == "3,[[1, 2], [2, 1]]"


def test_negative_hex_format_is_error() -> None:
    assert err_type("<%= format('%x', -1) %>") == "ArgumentError"
    assert r("<%= format('%+x|% b', -255, -5) %>") == "-ff|-101"


def test_comment_ends_at_tag() -> None:
    assert r("<% a = 1 # note %>x<%= a %>") == "x1"


def test_break_and_next_outside_loop() -> None:
    assert err_type("<% break %>") == "SyntaxError"
    assert err_type("<% next %>") == "SyntaxError"


def test_ruby_float_formatting_quirks() -> None:
    # Ruby's sprintf rounds ties on its floating-point estimate (half-even).
    assert r("<%= format('%.1f|%.1f|%.1f|%.2f|%.2f|%.1f', 0.25, 0.35, 0.45, 1.005, 2.675, 0.95) %>") == (
        "0.2|0.4|0.4|1.00|2.68|1.0"
    )
    assert r("<%= [1e20, 1e-5, 100.0, 1e16].to_json %>") == "[1.0e+20,1.0e-05,100.0,1.0e+16]"
    assert r("<%= [1e20, 1e-5, 100.0, 1e16].inspect %>") == "[1.0e+20, 1.0e-05, 100.0, 1.0e+16]"


def test_hash_keys_use_eql() -> None:
    assert r("<% h = {1 => :i, 1.0 => :f, '1' => :s, :'1' => :y} %><%= h.size %>,<%= h[1] %>,<%= h[1.0] %>") == "4,i,f"
    assert r("<% h = {true => 1} %><%= h[1].inspect %>") == "nil"


def test_unknown_names() -> None:
    assert err_type("<%= puts %>") == "NameError"
    assert err_type("<%= puts(1) %>") == "NoMethodError"
    assert err_type("<%= undefined_name %>") == "NameError"


def test_partial_locals(tmp_path) -> None:  # noqa: ANN001
    (tmp_path / "_p.erb").write_text("<%= a %>-<%= b %>")
    assert r("<%= render '_p.erb', a: 1, 'b' => 2 %>", base_dir=str(tmp_path)) == "1-2"
