"""The ``Math`` module."""

from __future__ import annotations

import math
import sys
from collections.abc import Callable
from typing import Any

from .errors import RubyError
from .values import Value


def _f(v: Value) -> float:
    from .builtins import to_float

    return to_float(v)


def _domain(name: str) -> RubyError:
    return RubyError("Math::DomainError", f'Numerical argument is out of domain - "{name}"')


def _cbrt(x: float) -> float:
    if sys.version_info >= (3, 11):
        return math.cbrt(x)
    if x == 0.0 or not math.isfinite(x):
        return x
    y = math.copysign(abs(x) ** (1.0 / 3.0), x)
    y = y - (y * y * y - x) / (3.0 * y * y)
    return y


def _sqrt(x: float) -> float:
    if x < 0:
        raise _domain("sqrt")
    return math.sqrt(x)


def _log_one(x: float, name: str) -> float:
    if x < 0:
        raise _domain(name)
    if x == 0:
        return -math.inf
    if math.isinf(x):
        return math.inf
    return math.log(x)


def _log2(x: float) -> float:
    if x < 0:
        raise _domain("log2")
    if x == 0:
        return -math.inf
    return math.log2(x)


def _log10(x: float) -> float:
    if x < 0:
        raise _domain("log10")
    if x == 0:
        return -math.inf
    return math.log10(x)


def _asin(x: float) -> float:
    if x < -1 or x > 1:
        raise _domain("asin")
    return math.asin(x)


def _acos(x: float) -> float:
    if x < -1 or x > 1:
        raise _domain("acos")
    return math.acos(x)


def _exp(x: float) -> float:
    try:
        return math.exp(x)
    except OverflowError:
        return math.inf


def _trig(fn: Callable[[float], float], name: str) -> Callable[[float], float]:
    def g(x: float) -> float:
        if math.isinf(x):
            raise _domain(name)
        return fn(x)

    return g


ONE_ARG: dict[str, Callable[[float], float]] = {
    "sqrt": _sqrt,
    "cbrt": _cbrt,
    "sin": _trig(math.sin, "sin"),
    "cos": _trig(math.cos, "cos"),
    "tan": _trig(math.tan, "tan"),
    "asin": _asin,
    "acos": _acos,
    "atan": math.atan,
    "exp": _exp,
    "log2": _log2,
    "log10": _log10,
}


def register() -> None:
    from .builtins import argc, singleton

    def make(name: str, fn: Callable[[float], float]) -> None:
        def h(it: Any, v: Value, args: list, blk: Any) -> Value:
            argc(args, 1)
            x = _f(args[0])
            if x != x:
                return math.nan
            return fn(x)

        singleton("Math", name)(h)

    for name, fn in ONE_ARG.items():
        make(name, fn)

    @singleton("Math", "log")
    def m_log(it: Any, v: Value, args: list, blk: Any) -> Value:
        argc(args, 1, 2)
        x = _f(args[0])
        if x != x:
            return math.nan
        d = _log_one(x, "log")
        if len(args) > 1:
            b = _f(args[1])
            if b != b:
                return math.nan
            lb = _log_one(b, "log")
            d = d / lb if lb != 0 else (math.nan if d == 0 else math.copysign(math.inf, d))
        return d

    @singleton("Math", "atan2")
    def m_atan2(it: Any, v: Value, args: list, blk: Any) -> Value:
        argc(args, 2)
        return math.atan2(_f(args[0]), _f(args[1]))

    @singleton("Math", "hypot")
    def m_hypot(it: Any, v: Value, args: list, blk: Any) -> Value:
        argc(args, 2)
        return math.hypot(_f(args[0]), _f(args[1]))
