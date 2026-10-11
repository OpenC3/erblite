# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

"""JSON parsing and generation (matching Ruby's ``json`` gem output)."""

from __future__ import annotations

import math
from typing import Any

from .errors import RubyError
from .values import (
    INT_MAX,
    INT_MIN,
    RHash,
    Symbol,
    Value,
    float_to_s,
    to_s,
)

MAX_NESTING = 100


def _perr(msg: str) -> RubyError:
    return RubyError("JSON::ParserError", msg)


class _Parser:
    def __init__(self, src: str, symbolize: bool) -> None:
        self.s = src
        self.i = 0
        self.symbolize = symbolize
        self.depth = 0

    def ws(self) -> None:
        s = self.s
        n = len(s)
        while self.i < n:
            c = s[self.i]
            if c in " \t\r\n":
                self.i += 1
            elif c == "/" and s.startswith("/*", self.i):
                end = s.find("*/", self.i + 2)
                if end < 0:
                    raise _perr("unexpected end of input, expected closing '*/'")
                self.i = end + 2
            elif c == "/" and s.startswith("//", self.i):
                end = s.find("\n", self.i)
                self.i = n if end < 0 else end + 1
            else:
                break

    def at(self) -> str:
        return f" at '{self.s[self.i : self.i + 20]}'"

    def parse(self) -> Value:
        self.ws()
        if self.i >= len(self.s):
            raise _perr("unexpected end of input")
        v = self.value()
        self.ws()
        if self.i < len(self.s):
            raise _perr("unexpected token at end of stream" + self.at())
        return v

    def value(self) -> Value:
        s = self.s
        if self.i >= len(s):
            raise _perr("unexpected end of input")
        c = s[self.i]
        if c == "{":
            return self.obj()
        if c == "[":
            return self.arr()
        if c == '"':
            return self.string()
        if c == "-" or c.isdigit():
            return self.number()
        for lit, val in (("true", True), ("false", False), ("null", None)):
            if s.startswith(lit, self.i):
                self.i += len(lit)
                return val
        raise _perr("unexpected character" + self.at())

    def enter(self) -> None:
        self.depth += 1
        if self.depth > MAX_NESTING + 1:  # the json gem accepts 101 levels when parsing
            raise _perr(f"nesting of {self.depth} is too deep")

    def obj(self) -> RHash:
        self.enter()
        self.i += 1
        h = RHash()
        self.ws()
        if self.i < len(self.s) and self.s[self.i] == "}":
            self.i += 1
            self.depth -= 1
            return h
        while True:
            self.ws()
            if self.i >= len(self.s) or self.s[self.i] != '"':
                raise _perr("expected object key" + self.at())
            k = self.string()
            self.ws()
            if self.i >= len(self.s) or self.s[self.i] != ":":
                raise _perr("expected ':' after object key" + self.at())
            self.i += 1
            self.ws()
            v = self.value()
            h.set(Symbol(k) if self.symbolize else k, v)
            self.ws()
            if self.i < len(self.s) and self.s[self.i] == ",":
                self.i += 1
                continue
            if self.i < len(self.s) and self.s[self.i] == "}":
                self.i += 1
                self.depth -= 1
                return h
            raise _perr("expected ',' or '}' after object value" + self.at())

    def arr(self) -> list:
        self.enter()
        self.i += 1
        out: list = []
        self.ws()
        if self.i < len(self.s) and self.s[self.i] == "]":
            self.i += 1
            self.depth -= 1
            return out
        while True:
            self.ws()
            out.append(self.value())
            self.ws()
            if self.i < len(self.s) and self.s[self.i] == ",":
                self.i += 1
                self.ws()
                if self.i < len(self.s) and self.s[self.i] == "]":
                    raise _perr("unexpected character" + self.at())
                continue
            if self.i < len(self.s) and self.s[self.i] == "]":
                self.i += 1
                self.depth -= 1
                return out
            raise _perr("expected ',' or ']' after array value" + self.at())

    def number(self) -> Value:
        s = self.s
        start = self.i
        n = len(s)
        if s[self.i] == "-":
            self.i += 1
        if self.i >= n or not s[self.i].isdigit():
            raise _perr("invalid number" + self.at())
        if s[self.i] == "0" and self.i + 1 < n and s[self.i + 1].isdigit():
            self.i = start
            raise _perr("invalid number" + self.at())
        while self.i < n and s[self.i].isdigit():
            self.i += 1
        is_float = False
        if self.i < n and s[self.i] == ".":
            self.i += 1
            if self.i >= n or not s[self.i].isdigit():
                self.i = start
                raise _perr("invalid number" + self.at())
            while self.i < n and s[self.i].isdigit():
                self.i += 1
            is_float = True
        if self.i < n and s[self.i] in "eE":
            self.i += 1
            if self.i < n and s[self.i] in "+-":
                self.i += 1
            if self.i >= n or not s[self.i].isdigit():
                self.i = start
                raise _perr("invalid number" + self.at())
            while self.i < n and s[self.i].isdigit():
                self.i += 1
            is_float = True
        text = s[start : self.i]
        if not text.isascii():
            raise _perr("invalid number" + self.at())
        if is_float:
            return float(text)
        v = int(text)
        if v < INT_MIN or v > INT_MAX:
            raise RubyError("RangeError", f"JSON integer {text} out of 64-bit range")
        return v

    def string(self) -> str:
        s = self.s
        n = len(s)
        self.i += 1
        out: list[str] = []
        while True:
            if self.i >= n:
                raise _perr('unexpected end of input, expected closing "')
            c = s[self.i]
            if c == '"':
                self.i += 1
                return "".join(out)
            if ord(c) < 0x20:
                raise _perr("invalid ASCII control character in string" + self.at())
            if c == "\\":
                if self.i + 1 >= n:
                    raise _perr("unexpected end of input")
                e = s[self.i + 1]
                simple = {'"': '"', "\\": "\\", "/": "/", "b": "\b", "f": "\f", "n": "\n", "r": "\r", "t": "\t"}
                if e in simple:
                    out.append(simple[e])
                    self.i += 2
                    continue
                if e == "u":
                    cp = self.hex4(self.i + 2)
                    self.i += 6
                    if 0xD800 <= cp <= 0xDBFF:
                        if s.startswith("\\u", self.i):
                            lo = self.hex4(self.i + 2)
                            if 0xDC00 <= lo <= 0xDFFF:
                                self.i += 6
                                out.append(chr(0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00)))
                                continue
                        raise _perr("incomplete surrogate pair" + self.at())
                    if 0xDC00 <= cp <= 0xDFFF:
                        raise _perr("incomplete surrogate pair" + self.at())
                    out.append(chr(cp))
                    continue
                raise _perr("invalid escape character in string" + self.at())
            out.append(c)
            self.i += 1

    def hex4(self, p: int) -> int:
        h = self.s[p : p + 4]
        if len(h) != 4 or any(ch not in "0123456789abcdefABCDEF" for ch in h):
            raise _perr("incomplete unicode character escape sequence" + self.at())
        return int(h, 16)


def parse(src: str, symbolize: bool = False) -> Value:
    return _Parser(src, symbolize).parse()


# ---------------------------------------------------------------------------------------
# Generation


def json_float(f: float) -> str:
    """[ERBLite] JSON floats are written exactly like Float#to_s (spec §14)."""
    if f != f or math.isinf(f):
        word = "NaN" if f != f else ("Infinity" if f > 0 else "-Infinity")
        raise RubyError("JSON::GeneratorError", f"{word} not allowed in JSON")
    return float_to_s(f)


_ESC = {'"': '\\"', "\\": "\\\\", "\b": "\\b", "\f": "\\f", "\n": "\\n", "\r": "\\r", "\t": "\\t"}


def json_string(s: str) -> str:
    out = ['"']
    for c in s:
        e = _ESC.get(c)
        if e is not None:
            out.append(e)
        elif ord(c) < 0x20:
            out.append(f"\\u{ord(c):04x}")
        else:
            out.append(c)
    out.append('"')
    return "".join(out)


def _gen(v: Value, pretty: bool, indent: str, depth: int) -> str:
    t = type(v)
    if v is None:
        return "null"
    if v is True:
        return "true"
    if v is False:
        return "false"
    if t is int:
        return str(v)
    if t is float:
        return json_float(v)
    if isinstance(v, str):
        return json_string(v)
    if t is Symbol:
        return json_string(v.name)
    if t is list or t is RHash:
        depth += 1
        if depth > MAX_NESTING:
            raise RubyError("JSON::GeneratorError", f"nesting of {depth} is too deep")
        if t is list:
            if not v:
                return "[]"
            if pretty:
                inner = indent + "  "
                items = [inner + _gen(x, pretty, inner, depth) for x in v]
                return "[\n" + ",\n".join(items) + "\n" + indent + "]"
            return "[" + ",".join(_gen(x, pretty, indent, depth) for x in v) + "]"
        if not len(v):
            return "{}"
        if pretty:
            inner = indent + "  "
            items = [inner + json_string(to_s(k)) + ": " + _gen(x, pretty, inner, depth) for k, x in v.items()]
            return "{\n" + ",\n".join(items) + "\n" + indent + "}"
        return "{" + ",".join(json_string(to_s(k)) + ":" + _gen(x, pretty, indent, depth) for k, x in v.items()) + "}"
    return json_string(to_s(v))


def generate(v: Value, pretty: bool = False) -> str:
    return _gen(v, pretty, "", 0)


def register() -> None:
    from .builtins import argc, singleton, to_str

    @singleton("JSON", "parse")
    def j_parse(it: Any, recv: Value, args: list, blk: Any) -> Value:
        argc(args, 1, 2)
        src = to_str(args[0])
        symbolize = False
        if len(args) > 1:
            opts = args[1]
            if type(opts) is not RHash:
                raise RubyError("TypeError", "options must be a Hash")
            sv = opts.get(Symbol("symbolize_names"))
            symbolize = sv is not None and sv is not False
        return parse(str(src), symbolize)

    @singleton("JSON", "generate")
    def j_generate(it: Any, recv: Value, args: list, blk: Any) -> Value:
        argc(args, 1, 2)
        return generate(args[0])

    @singleton("JSON", "pretty_generate")
    def j_pretty(it: Any, recv: Value, args: list, blk: Any) -> Value:
        argc(args, 1, 2)
        return generate(args[0], pretty=True)
