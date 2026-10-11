# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

"""ERBLite: a sandboxed template engine using ERB syntax and a subset of Ruby.

>>> import erblite
>>> erblite.render("Hello <%= name.upcase %>!", {"name": "world"})
'Hello WORLD!'
"""

from __future__ import annotations

import os
import re
import sys
from collections.abc import Iterable, Mapping
from typing import Any

from . import nodes
from .errors import Error, RubyError
from .interpreter import Context, Interpreter
from .lexer import KEYWORDS, Token, tokenize_template
from .parser import Parser
from .sandbox import Sandbox
from .values import INT_MAX, INT_MIN, RHash, Symbol, Value

__all__ = ["Error", "Symbol", "Template", "render", "render_file"]
__version__ = "0.5.0"

_LOCAL_RE = re.compile(r"\A[a-z_][A-Za-z0-9_]*\Z")
_MIN_RECURSION = 10000


def _host_names(names: Iterable[str]) -> frozenset[str]:
    return frozenset(n for n in names if _LOCAL_RE.match(n) and n not in KEYWORDS)


def convert(value: Any, depth: int = 0) -> Value:
    """Converts a host (Python) value into a template value."""
    if depth > 200:
        raise Error("ArgumentError", "variables are nested too deeply")
    if value is None or isinstance(value, (bool, Symbol)):
        return value
    if isinstance(value, str):
        return str(value)
    if isinstance(value, float):
        return float(value)
    if isinstance(value, int):
        if value < INT_MIN or value > INT_MAX:
            raise Error("RangeError", f"integer {value} is out of 64-bit range")
        return int(value)
    if isinstance(value, Mapping):
        h = RHash()
        for k, v in value.items():
            h.set(convert(k, depth + 1), convert(v, depth + 1))
        return h
    if isinstance(value, (list, tuple)):
        return [convert(v, depth + 1) for v in value]
    raise Error("TypeError", f"unsupported variable type: {type(value).__name__}")


class Template:
    """A compiled template. Parsing is cached per set of host variable names, because Ruby
    decides at parse time whether a bare name is a local variable."""

    def __init__(self, source: str, filename: str = "(erblite)") -> None:
        self.source = source
        self.filename = filename
        try:
            self._tokens: list[Token] = tokenize_template(source)
        except RubyError as e:
            raise Error(e.type, e.message, filename, e.line or 1) from None
        self._trees: dict[frozenset[str], nodes.Seq] = {}

    def _tree(self, names: frozenset[str]) -> nodes.Seq:
        tree = self._trees.get(names)
        if tree is None:
            tree = _parse_tokens(self._tokens, names, self.filename)
            self._trees[names] = tree
        return tree

    def render(
        self,
        variables: Mapping[str, Any] | None = None,
        *,
        base_dir: str | os.PathLike[str] | None = None,
        max_steps: int | None = None,
    ) -> str:
        if sys.getrecursionlimit() < _MIN_RECURSION:
            sys.setrecursionlimit(_MIN_RECURSION)
        host: dict[str, Value] = {}
        for k, v in (variables or {}).items():
            if not isinstance(k, str):
                raise Error("TypeError", f"variable names must be strings, got {type(k).__name__}")
            host[k] = convert(v)
        names = _host_names(host.keys())
        tree = self._tree(names)
        ctx = Context(Sandbox(base_dir), max_steps, _parse_source)
        interp = Interpreter(ctx, self.filename)
        return interp.run(tree, {k: v for k, v in host.items() if k in names})


def _parse_tokens(tokens: list[Token], names: Iterable[str], filename: str) -> nodes.Seq:
    try:
        return Parser(tokens, names).parse_program()
    except RubyError as e:
        raise Error(e.type, e.message, filename, e.line or 1) from None
    except RecursionError:
        raise Error("LimitError", "template nesting too deep", filename, 1) from None


def _parse_source(source: str, filename: str, names: Iterable[str]) -> nodes.Seq:
    try:
        toks = tokenize_template(source)
    except RubyError as e:
        raise Error(e.type, e.message, filename, e.line or 1) from None
    return _parse_tokens(toks, _host_names(names), filename)


def render(
    source: str,
    variables: Mapping[str, Any] | None = None,
    *,
    base_dir: str | os.PathLike[str] | None = None,
    filename: str = "(erblite)",
    max_steps: int | None = None,
) -> str:
    """Renders ``source`` with ``variables`` and returns the output string."""
    return Template(source, filename=filename).render(variables, base_dir=base_dir, max_steps=max_steps)


def render_file(
    path: str | os.PathLike[str],
    variables: Mapping[str, Any] | None = None,
    *,
    base_dir: str | os.PathLike[str] | None = None,
    filename: str | None = None,
    max_steps: int | None = None,
) -> str:
    """Renders the template file at ``path``. ``base_dir`` defaults to the file's directory."""
    path = os.fspath(path)
    with open(path, encoding="utf-8") as f:
        source = f.read()
    if base_dir is None:
        base_dir = os.path.dirname(os.path.abspath(path))
    return Template(source, filename=filename or path).render(variables, base_dir=base_dir, max_steps=max_steps)
