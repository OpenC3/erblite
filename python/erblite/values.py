"""Runtime value types and the core value protocol (truthiness, equality, inspect, to_s).

Template values are plain data:

* ``None`` (nil), ``True``/``False``, ``int`` (64-bit checked), ``float``, ``str``
* :class:`Symbol`, ``list`` (Array), :class:`RHash`, :class:`RRange`, :class:`RClass`,
  :class:`REnumerator`

None of these expose Python attributes to templates: every method call goes through the
explicit allowlist tables in :mod:`erblite.builtins`.
"""

from __future__ import annotations

import math
import re
from collections.abc import Iterator
from typing import Any

from .errors import RubyError

INT_MIN = -(2**63)
INT_MAX = 2**63 - 1
MAX_SIZE = 10_000_000  # [ERBLite] max String/Array/Hash size (spec §14)

Value = Any


def check_int(v: int) -> int:
    if v < INT_MIN or v > INT_MAX:
        raise RubyError("RangeError", "integer overflow (result exceeds 64-bit range)")
    return v


class Symbol:
    """A Ruby symbol. Compared and hashed by name (not interned, so templates cannot grow a
    process-wide symbol table)."""

    __slots__ = ("name",)

    def __init__(self, name: str) -> None:
        object.__setattr__(self, "name", str(name))

    def __setattr__(self, key: str, value: object) -> None:
        raise AttributeError("Symbol is immutable")

    def __eq__(self, other: object) -> bool:
        return type(other) is Symbol and other.name == self.name

    def __hash__(self) -> int:
        return hash(("Symbol", self.name))

    def __repr__(self) -> str:
        return f"Symbol({self.name!r})"


class FStr(str):
    """A frozen String (the result of ``String#freeze``). Behaves exactly like ``str``."""

    __slots__ = ()


class RRange:
    __slots__ = ("first", "last", "excl")

    def __init__(self, first: Value, last: Value, excl: bool) -> None:
        self.first = first
        self.last = last
        self.excl = excl


class RClass:
    """A built-in class or module constant (Integer, Math, JSON, ...)."""

    __slots__ = ("name", "is_module")

    def __init__(self, name: str, is_module: bool = False) -> None:
        self.name = name
        self.is_module = is_module


class REnumerator:
    """An enumerator: a receiver, an iterator method name and its arguments."""

    __slots__ = ("recv", "meth", "args")

    def __init__(self, recv: Value, meth: str, args: list) -> None:
        self.recv = recv
        self.meth = meth
        self.args = args


class RHash:
    """An insertion-ordered hash with Ruby ``eql?`` key semantics and a default value."""

    __slots__ = ("store", "default")

    def __init__(self, default: Value = None) -> None:
        self.store: dict[Any, tuple[Value, Value]] = {}
        self.default = default

    @staticmethod
    def from_pairs(pairs: list[tuple[Value, Value]]) -> RHash:
        h = RHash()
        for k, v in pairs:
            h.set(k, v)
        return h

    def get(self, key: Value, missing: Value = None) -> Value:
        e = self.store.get(hash_key(key))
        return missing if e is None else e[1]

    def has(self, key: Value) -> bool:
        return hash_key(key) in self.store

    def set(self, key: Value, value: Value) -> None:
        hk = hash_key(key)
        e = self.store.get(hk)
        if e is not None:
            self.store[hk] = (e[0], value)
        else:
            if len(self.store) >= MAX_SIZE:
                raise RubyError("LimitError", f"hash larger than {MAX_SIZE} entries")
            if type(key) is str:
                key = FStr(key)
            self.store[hk] = (key, value)

    def delete(self, key: Value) -> tuple[bool, Value]:
        e = self.store.pop(hash_key(key), None)
        if e is None:
            return False, None
        return True, e[1]

    def items(self) -> list[tuple[Value, Value]]:
        return list(self.store.values())

    def keys(self) -> list[Value]:
        return [k for k, _ in self.store.values()]

    def values(self) -> list[Value]:
        return [v for _, v in self.store.values()]

    def __len__(self) -> int:
        return len(self.store)

    def copy(self) -> RHash:
        h = RHash(self.default)
        h.store = dict(self.store)
        return h


class _Ident:
    """Hash key wrapper for values compared by identity."""

    __slots__ = ("obj",)

    def __init__(self, obj: object) -> None:
        self.obj = obj

    def __hash__(self) -> int:
        return id(self.obj)

    def __eq__(self, other: object) -> bool:
        return isinstance(other, _Ident) and other.obj is self.obj


def hash_key(v: Value) -> Any:
    """Returns a Python-hashable key implementing Ruby ``eql?``/``hash``."""
    t = type(v)
    if v is None:
        return ("n",)
    if t is bool:
        return ("b", v)
    if t is int:
        return ("i", v)
    if t is float:
        if v != v:
            return ("f", "nan", id(v))
        return ("f", v)
    if t is str or t is FStr:
        return ("s", str(v))
    if t is Symbol:
        return ("y", v.name)
    if t is list:
        return ("a", tuple(hash_key(x) for x in v))
    if t is RHash:
        return ("h", frozenset((k, hash_key(val)) for k, (_, val) in v.store.items()))
    if t is RRange:
        return ("r", hash_key(v.first), hash_key(v.last), v.excl)
    if t is RClass:
        return ("c", v.name)
    return ("o", _Ident(v))


# ---------------------------------------------------------------------------------------
# Classes


def class_name(v: Value) -> str:
    t = type(v)
    if v is None:
        return "NilClass"
    if v is True:
        return "TrueClass"
    if v is False:
        return "FalseClass"
    if t is int:
        return "Integer"
    if t is float:
        return "Float"
    if t is str or t is FStr:
        return "String"
    if t is Symbol:
        return "Symbol"
    if t is list:
        return "Array"
    if t is RHash:
        return "Hash"
    if t is RRange:
        return "Range"
    if t is RClass:
        return "Module" if v.is_module else "Class"
    if t is REnumerator:
        return "Enumerator"
    return "Object"


ANCESTORS = {
    "NilClass": ["NilClass", "Object"],
    "TrueClass": ["TrueClass", "Object"],
    "FalseClass": ["FalseClass", "Object"],
    "Integer": ["Integer", "Numeric", "Comparable", "Object"],
    "Float": ["Float", "Numeric", "Comparable", "Object"],
    "String": ["String", "Comparable", "Object"],
    "Symbol": ["Symbol", "Comparable", "Object"],
    "Array": ["Array", "Enumerable", "Object"],
    "Hash": ["Hash", "Enumerable", "Object"],
    "Range": ["Range", "Enumerable", "Object"],
    "Enumerator": ["Enumerator", "Enumerable", "Object"],
    "Class": ["Class", "Module", "Object"],
    "Module": ["Module", "Object"],
}


def is_a(v: Value, cls: RClass) -> bool:
    return cls.name in ANCESTORS.get(class_name(v), ["Object"])


# ---------------------------------------------------------------------------------------
# Truthiness and equality


def truthy(v: Value) -> bool:
    return v is not None and v is not False


def is_num(v: Value) -> bool:
    t = type(v)
    return t is int or t is float


def req(a: Value, b: Value) -> bool:
    """Ruby ``==``."""
    ta, tb = type(a), type(b)
    if ta is int or ta is float:
        if tb is int or tb is float:
            return a == b
        return False
    if ta is FStr:
        ta = str
    if tb is FStr:
        tb = str
    if ta is not tb:
        return False
    if ta is str:
        return a == b
    if ta is list:
        if a is b:
            return True
        if len(a) != len(b):
            return False
        return all(req(x, y) for x, y in zip(a, b, strict=True))
    if ta is RHash:
        if a is b:
            return True
        if len(a) != len(b):
            return False
        for hk, (_, va) in a.store.items():
            e = b.store.get(hk)
            if e is None or not req(va, e[1]):
                return False
        return True
    if ta is RRange:
        return a.excl == b.excl and req(a.first, b.first) and req(a.last, b.last)
    if ta is RClass:
        return a.name == b.name
    if ta is bool or a is None:
        return a is b
    if ta is Symbol:
        return a.name == b.name
    return a is b


def eql(a: Value, b: Value) -> bool:
    """Ruby ``eql?``."""
    return hash_key(a) == hash_key(b)


def cmp(a: Value, b: Value) -> int | None:
    """Ruby ``<=>``; ``None`` when not comparable."""
    ta, tb = type(a), type(b)
    if (ta is int or ta is float) and (tb is int or tb is float):
        if a != a or b != b:
            return None
        return -1 if a < b else (1 if a > b else 0)
    if isinstance(a, str) and isinstance(b, str):
        return -1 if a < b else (1 if a > b else 0)
    if ta is Symbol and tb is Symbol:
        return -1 if a.name < b.name else (1 if a.name > b.name else 0)
    if ta is list and tb is list:
        for x, y in zip(a, b, strict=False):
            c = cmp(x, y)
            if c is None:
                return None
            if c != 0:
                return c
        return -1 if len(a) < len(b) else (1 if len(a) > len(b) else 0)
    if req(a, b):
        return 0
    return None


def cmp_or_fail(a: Value, b: Value) -> int:
    c = cmp(a, b)
    if c is None:
        raise RubyError("ArgumentError", f"comparison of {class_name(a)} with {cmp_desc(b)} failed")
    return c


def cmp_desc(v: Value) -> str:
    if v is None or type(v) in (int, float, bool):
        return inspect(v)
    return class_name(v)


# ---------------------------------------------------------------------------------------
# Float formatting


def float_digits(f: float) -> tuple[str, int]:
    """Shortest round-trip digits D and exponent e with |f| = 0.D * 10**e."""
    r = repr(abs(f))
    if "e" in r:
        mant, exp_s = r.split("e")
        exp = int(exp_s)
    else:
        mant, exp = r, 0
    if "." in mant:
        ip, fp = mant.split(".")
    else:
        ip, fp = mant, ""
    digits = ip + fp
    e = len(ip) + exp
    stripped = digits.lstrip("0")
    e -= len(digits) - len(stripped)
    stripped = stripped.rstrip("0")
    if not stripped:
        return "0", 1
    return stripped, e


def float_to_s(f: float) -> str:
    if f != f:
        return "NaN"
    if f == math.inf:
        return "Infinity"
    if f == -math.inf:
        return "-Infinity"
    if f == 0.0:
        return "-0.0" if math.copysign(1.0, f) < 0 else "0.0"
    sign = "-" if f < 0 else ""
    d, e = float_digits(f)
    n = len(d)
    if 0 < e <= 16 and n > e:
        return sign + d[:e] + "." + d[e:]
    if 0 < e <= 15:
        return sign + d + "0" * (e - n) + ".0"
    if -4 < e <= 0:
        return sign + "0." + "0" * (-e) + d
    x = e - 1
    return sign + d[0] + "." + (d[1:] or "0") + "e" + ("+" if x >= 0 else "-") + f"{abs(x):02d}"


# ---------------------------------------------------------------------------------------
# to_s / inspect

_SIMPLE_SYM = re.compile(r"\A(?:[A-Za-z_]|[^\x00-\x7f])(?:[A-Za-z0-9_]|[^\x00-\x7f])*[?!=]?\Z")
_KEY_SYM = re.compile(r"\A(?:[A-Za-z_]|[^\x00-\x7f])(?:[A-Za-z0-9_]|[^\x00-\x7f])*[?!]?\Z")
_VAR_SYM = re.compile(
    r"\A(?:(?:@@?|\$)(?:[A-Za-z_]|[^\x00-\x7f])(?:[A-Za-z0-9_]|[^\x00-\x7f])*|\$[0-9]+|\$[~*$?!@/\\;,.=:<>\"&`'+0_])\Z"
)
OPERATOR_SYMS = frozenset("+ - * / % ** == != < > <= >= <=> ! [] []= << >> & | ^ ~ +@ -@ === =~ !~".split())

_STR_ESC = {
    '"': '\\"',
    "\\": "\\\\",
    "\n": "\\n",
    "\t": "\\t",
    "\r": "\\r",
    "\x1b": "\\e",
    "\x07": "\\a",
    "\x08": "\\b",
    "\x0c": "\\f",
    "\x0b": "\\v",
}


def _needs_uescape(c: str) -> bool:
    o = ord(c)
    return o < 0x20 or 0x7F <= o <= 0x9F or o == 0x2028 or o == 0x2029


def inspect_str(s: str) -> str:
    out = ['"']
    n = len(s)
    i = 0
    while i < n:
        c = s[i]
        e = _STR_ESC.get(c)
        if e is not None:
            out.append(e)
        elif c == "#" and i + 1 < n and s[i + 1] in "{$@":
            out.append("\\#")
        elif _needs_uescape(c):
            out.append(f"\\u{ord(c):04X}")
        else:
            out.append(c)
        i += 1
    out.append('"')
    return "".join(out)


def sym_is_simple(name: str) -> bool:
    return bool(_SIMPLE_SYM.match(name)) or name in OPERATOR_SYMS or bool(_VAR_SYM.match(name))


def inspect_sym(name: str) -> str:
    if sym_is_simple(name):
        return ":" + name
    return ":" + inspect_str(name)


def inspect(v: Value) -> str:
    return _inspect(v, set())


def _inspect(v: Value, seen: set[int]) -> str:
    t = type(v)
    if v is None:
        return "nil"
    if v is True:
        return "true"
    if v is False:
        return "false"
    if t is int:
        return str(v)
    if t is float:
        return float_to_s(v)
    if t is str or t is FStr:
        return inspect_str(v)
    if t is Symbol:
        return inspect_sym(v.name)
    if t is list:
        if id(v) in seen:
            return "[...]"
        seen.add(id(v))
        try:
            return "[" + ", ".join(_inspect(x, seen) for x in v) + "]"
        finally:
            seen.discard(id(v))
    if t is RHash:
        if not v.store:
            return "{}"
        if id(v) in seen:
            return "{...}"
        seen.add(id(v))
        try:
            parts = []
            for k, val in v.store.values():
                if type(k) is Symbol:
                    if _KEY_SYM.match(k.name):
                        parts.append(f"{k.name}: {_inspect(val, seen)}")
                    else:
                        parts.append(f"{inspect_str(k.name)}: {_inspect(val, seen)}")
                else:
                    parts.append(f"{_inspect(k, seen)} => {_inspect(val, seen)}")
            return "{" + ", ".join(parts) + "}"
        finally:
            seen.discard(id(v))
    if t is RRange:
        a = "" if v.first is None else _inspect(v.first, seen)
        b = "" if v.last is None else _inspect(v.last, seen)
        if v.first is None and v.last is None:
            a = "nil"
        return a + ("..." if v.excl else "..") + b
    if t is RClass:
        return v.name
    if t is REnumerator:
        if v.meth == "step" and type(v.recv) in (int, float, RRange):
            # Ruby's Enumerator::ArithmeticSequence
            args = ", ".join(_inspect(a, seen) for a in v.args)
            recv = _inspect(v.recv, seen)
            if type(v.recv) is RRange:
                recv = f"({recv})"
            return f"({recv}.step" + (f"({args}))" if v.args else ")")
        s = f"#<Enumerator: {_inspect(v.recv, seen)}:{v.meth}"
        if v.args:
            s += "(" + ", ".join(_inspect(a, seen) for a in v.args) + ")"
        return s + ">"
    return "#<Object>"


def to_s(v: Value) -> str:
    t = type(v)
    if t is str:
        return v
    if t is FStr:
        return str(v)
    if v is None:
        return ""
    if t is int:
        return str(v)
    if t is float:
        return float_to_s(v)
    if t is Symbol:
        return v.name
    if t is RRange:
        return to_s(v.first) + ("..." if v.excl else "..") + to_s(v.last)
    return inspect(v)


# ---------------------------------------------------------------------------------------
# Misc helpers


def range_int_bounds(r: RRange) -> tuple[int, int | None]:
    """Integer start and inclusive end (None = endless) for iteration."""
    a, b = r.first, r.last
    if type(a) is not int:
        raise RubyError("TypeError", f"can't iterate from {class_name(a)}")
    if b is None:
        return a, None
    if type(b) is float:
        if b == math.inf:
            return a, None
        end = math.floor(b)
        if r.excl and end == b:
            end -= 1
        return a, int(end)
    if type(b) is not int:
        raise RubyError("TypeError", f"can't iterate from {class_name(a)}")
    return a, (b - 1 if r.excl else b)


def range_iter(r: RRange) -> Iterator[Value]:
    a, b = r.first, r.last
    if isinstance(a, str) and isinstance(b, str):
        yield from str_range(str(a), str(b), r.excl)
        return
    start, end = range_int_bounds(r)
    i = start
    while end is None or i <= end:
        yield i
        i += 1


def str_succ(s: str) -> str:
    if not s:
        return ""
    chars = list(s)
    has_alnum = any(c.isascii() and c.isalnum() for c in chars)
    i = len(chars) - 1
    if not has_alnum:
        while i >= 0:
            o = ord(chars[i])
            if o < 0x10FFFF:
                chars[i] = chr(o + 1)
                return "".join(chars)
            chars[i] = chr(0)
            i -= 1
        return chr(1) + "".join(chars)
    last_alnum = None
    while i >= 0:
        c = chars[i]
        if not (c.isascii() and c.isalnum()):
            i -= 1
            continue
        last_alnum = i
        if c == "z":
            chars[i] = "a"
        elif c == "Z":
            chars[i] = "A"
        elif c == "9":
            chars[i] = "0"
        else:
            chars[i] = chr(ord(c) + 1)
            return "".join(chars)
        i -= 1
        # carry: continue to the next alnum to the left
        while i >= 0 and not (chars[i].isascii() and chars[i].isalnum()):
            i -= 1
        if i < 0:
            break
    c = s[last_alnum]
    ins = "a" if c == "z" else ("A" if c == "Z" else "1")
    chars.insert(last_alnum, ins)
    return "".join(chars)


def str_range(a: str, b: str, excl: bool) -> Iterator[str]:
    if len(a) > len(b):
        return
    cur = a
    count = 0
    while True:
        if cur == b:
            if not excl:
                yield cur
            return
        if len(cur) > len(b):
            return
        yield cur
        cur = str_succ(cur)
        count += 1
        if count > 10_000_000:
            return
