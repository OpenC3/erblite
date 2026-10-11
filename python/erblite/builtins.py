"""Built-in method tables.

Every method a template can call is listed explicitly in one of the dictionaries below,
keyed by method name. :func:`find_method` looks a name up in the tables for the
receiver's type; any name that is not present raises ``NoMethodError``. ``respond_to?``
consults the very same tables.
"""

from __future__ import annotations

import math
from collections.abc import Callable
from fractions import Fraction
from typing import Any

from .errors import RubyError, argc_error
from .values import (
    INT_MAX,
    INT_MIN,
    FStr,
    RClass,
    REnumerator,
    RHash,
    RRange,
    Symbol,
    Value,
    check_int,
    class_name,
    cmp,
    cmp_or_fail,
    eql,
    float_to_s,
    inspect,
    is_a,
    req,
    to_s,
    truthy,
)

Handler = Callable[[Any, Value, list, Any], Value]
Table = dict[str, Handler]

COMMON: Table = {}
NIL: Table = {}
BOOL: Table = {}
NUMERIC: Table = {}
INTEGER: Table = {}
FLOAT: Table = {}
STRING: Table = {}
SYMBOL: Table = {}
ARRAY: Table = {}
HASH: Table = {}
RANGE: Table = {}
ENUMERABLE: Table = {}
ENUMERATOR: Table = {}
CLASS: Table = {}
SINGLETON: dict[str, Table] = {}
FUNCTIONS: dict[str, Callable[[Any, list, Any], Value]] = {}


def method(table: Table, *names: str) -> Callable[[Handler], Handler]:
    def deco(fn: Handler) -> Handler:
        for n in names:
            table[n] = fn
        return fn

    return deco


def function(*names: str) -> Callable:
    def deco(fn: Callable) -> Callable:
        for n in names:
            FUNCTIONS[n] = fn
        return fn

    return deco


def singleton(cls: str, *names: str) -> Callable[[Handler], Handler]:
    return method(SINGLETON.setdefault(cls, {}), *names)


# ---------------------------------------------------------------------------------------
# Dispatch


def tables_for(v: Value) -> tuple[Table, ...]:
    t = type(v)
    if t is str or t is FStr:
        return (STRING, COMMON)
    if t is int:
        return (INTEGER, NUMERIC, COMMON)
    if t is list:
        return (ARRAY, ENUMERABLE, COMMON)
    if t is RHash:
        return (HASH, ENUMERABLE, COMMON)
    if v is None:
        return (NIL, COMMON)
    if t is bool:
        return (BOOL, COMMON)
    if t is float:
        return (FLOAT, NUMERIC, COMMON)
    if t is Symbol:
        return (SYMBOL, COMMON)
    if t is RRange:
        return (RANGE, ENUMERABLE, COMMON)
    if t is REnumerator:
        return (ENUMERATOR, ENUMERABLE, COMMON)
    if t is RClass:
        return (SINGLETON.get(v.name, {}), CLASS, COMMON)
    return (COMMON,)


def find_method(v: Value, name: str) -> Handler | None:
    for table in tables_for(v):
        h = table.get(name)
        if h is not None:
            return h
    return None


def describe_recv(v: Value) -> str:
    if v is None:
        return "nil"
    if v is True:
        return "true"
    if v is False:
        return "false"
    if type(v) is RClass:
        return ("module " if v.is_module else "class ") + v.name
    return "an instance of " + class_name(v)


def no_method(v: Value, name: str) -> RubyError:
    return RubyError("NoMethodError", f"undefined method '{name}' for {describe_recv(v)}")


# ---------------------------------------------------------------------------------------
# Argument helpers


def argc(args: list, lo: int, hi: int | None = None) -> None:
    n = len(args)
    if hi is None:
        hi = lo
    if n < lo or (hi >= 0 and n > hi):
        if lo == hi:
            exp = str(lo)
        elif hi < 0:
            exp = f"{lo}+"
        else:
            exp = f"{lo}..{hi}"
        raise argc_error(n, exp)


def no_implicit(v: Value, into: str) -> RubyError:
    if v is None:
        return RubyError("TypeError", f"no implicit conversion from nil to {into.lower()}")
    if v is True or v is False:
        return RubyError("TypeError", f"no implicit conversion of {'true' if v else 'false'} into {into}")
    return RubyError("TypeError", f"no implicit conversion of {class_name(v)} into {into}")


def to_int(v: Value) -> int:
    t = type(v)
    if t is int:
        return v
    if t is float:
        if v != v or v in (math.inf, -math.inf):
            raise RubyError("FloatDomainError", float_to_s(v))
        r = int(v)
        if r < INT_MIN or r > INT_MAX:
            raise RubyError("RangeError", f"float {float_to_s(v)} out of range of integer")
        return r
    raise no_implicit(v, "Integer")


def to_str(v: Value) -> str:
    if isinstance(v, str):
        return v
    raise no_implicit(v, "String")


def to_float(v: Value) -> float:
    t = type(v)
    if t is float:
        return v
    if t is int:
        return float(v)
    if v is None:
        raise RubyError("TypeError", "can't convert nil into Float")
    raise RubyError("TypeError", f"can't convert {class_name(v)} into Float")


def opt_block_enum(it: Any, recv: Value, name: str, args: list, blk: Any) -> REnumerator | None:
    if blk is None:
        return REnumerator(recv, name, list(args))
    return None


def require_class(v: Value) -> RClass:
    if type(v) is not RClass:
        raise RubyError("TypeError", "class or module required")
    return v


def case_eq(it: Any, pat: Value, v: Value) -> bool:
    t = type(pat)
    if t is RClass:
        return is_a(v, pat)
    if t is RRange:
        return range_cover(pat, v)
    return req(pat, v)


def range_cover(r: RRange, v: Value) -> bool:
    if r.first is not None:
        c = cmp(r.first, v)
        if c is None or c > 0:
            return False
    if r.last is not None:
        c = cmp(v, r.last)
        if c is None:
            return False
        if r.excl:
            return c < 0
        return c <= 0
    return True


# ---------------------------------------------------------------------------------------
# All values


@method(COMMON, "==")
def m_eq(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    return req(v, args[0])


@method(COMMON, "!=")
def m_neq(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    return not req(v, args[0])


@method(COMMON, "!")
def m_not(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return not truthy(v)


@method(COMMON, "===")
def m_case_eq(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    return case_eq(it, v, args[0])


@method(COMMON, "eql?")
def m_eql(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    return eql(v, args[0])


@method(COMMON, "<=>")
def m_cmp(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    return cmp(v, args[0])


@method(COMMON, "nil?")
def m_nil_p(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v is None


@method(COMMON, "to_s")
def m_to_s(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return to_s(v)


@method(COMMON, "inspect")
def m_inspect(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return inspect(v)


@method(COMMON, "class")
def m_class(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    from .consts import class_value

    return class_value(class_name(v))


@method(COMMON, "is_a?", "kind_of?")
def m_is_a(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    return is_a(v, require_class(args[0]))


@method(COMMON, "instance_of?")
def m_instance_of(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    return class_name(v) == require_class(args[0]).name


@method(COMMON, "respond_to?")
def m_respond_to(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1, 2)
    name = args[0]
    if type(name) is Symbol:
        name = name.name
    elif not isinstance(name, str):
        raise RubyError("TypeError", f"{inspect(name)} is not a symbol nor a string")
    return find_method(v, name) is not None


@method(COMMON, "dup")
def m_dup(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    t = type(v)
    if t is list:
        return list(v)
    if t is RHash:
        return v.copy()
    if t is FStr:
        return str(v)
    return v


@method(COMMON, "freeze")
def m_freeze(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    if type(v) is str:
        return FStr(v)
    if type(v) in (list, RHash):
        it.ctx.frozen[id(v)] = v
    return v


@method(COMMON, "frozen?")
def m_frozen(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    t = type(v)
    if v is None or t in (bool, int, float, Symbol, RRange, RClass, FStr):
        return True
    if t in (list, RHash):
        return id(v) in it.ctx.frozen
    return False


@method(COMMON, "to_json")
def m_to_json(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    from .jsonlib import generate

    return generate(v)


@method(COMMON, "then")
def m_then(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    if blk is None:
        return REnumerator(v, "then", [])
    return it.yield_(blk, v)


@method(COMMON, "tap")
def m_tap(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    if blk is None:
        raise RubyError("LocalJumpError", "no block given (yield)")
    it.yield_(blk, v)
    return v


# ---------------------------------------------------------------------------------------
# nil / true / false


@method(NIL, "to_a")
def nil_to_a(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return []


@method(NIL, "to_i")
def nil_to_i(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return 0


@method(NIL, "to_f")
def nil_to_f(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return 0.0


@method(NIL, "to_h")
def nil_to_h(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return RHash()


@method(NIL, "&")
@method(BOOL, "&")
def bool_and(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    return bool(v) and truthy(args[0])


@method(NIL, "|")
@method(BOOL, "|")
def bool_or(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    return bool(v) or truthy(args[0])


@method(NIL, "^")
@method(BOOL, "^")
def bool_xor(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    return bool(v) != truthy(args[0])


# ---------------------------------------------------------------------------------------
# Numbers


def coerce_fail(a: Value, b: Value) -> RubyError:
    desc = "nil" if b is None else ("true" if b is True else ("false" if b is False else class_name(b)))
    return RubyError("TypeError", f"{desc} can't be coerced into {class_name(a)}")


def is_num(v: Value) -> bool:
    t = type(v)
    return t is int or t is float


def fdiv(a: float, b: float) -> float:
    a, b = float(a), float(b)
    if b == 0.0:
        if a != a or a == 0.0:
            return math.nan
        neg = (a < 0) != (math.copysign(1.0, b) < 0)
        return -math.inf if neg else math.inf
    return a / b


def flodivmod(x: float, y: float) -> tuple[float, float]:
    if y != y:
        return y, y
    if y == 0.0:
        raise RubyError("ZeroDivisionError", "divided by 0")
    if x == 0.0 or (math.isinf(y) and not math.isinf(x)):
        mod = x
    else:
        mod = math.fmod(x, y)
    if math.isinf(x) and not math.isinf(y):
        div = x
    else:
        div = (x - mod) / y
        div = c_round(div)
    if y * mod < 0:
        mod += y
        div -= 1.0
    return div, mod


def float_mod(x: float, y: float) -> float:
    return flodivmod(x, y)[1]


def fpow(a: float, b: float) -> float:
    a, b = float(a), float(b)
    if a == 0.0 and b < 0:
        if b == math.floor(b) and math.fmod(b, 2.0) != 0 and math.copysign(1.0, a) < 0:
            return -math.inf
        return math.inf
    if a < 0 and b != math.floor(b) and not math.isinf(b):
        raise RubyError("RangeError", "Complex results are not supported")
    try:
        return math.pow(a, b)
    except OverflowError:
        if a < 0 and b == math.floor(b) and math.fmod(b, 2.0) != 0:
            return -math.inf
        return math.inf
    except ValueError:
        return math.nan


def int_pow(a: int, b: int) -> int:
    if b < 0:
        raise RubyError("RangeError", "negative exponent is not supported (Ruby would return a Rational)")
    if a in (0, 1):
        return a if b > 0 else 1
    if a == -1:
        return 1 if b % 2 == 0 else -1
    if b > 64:
        raise RubyError("RangeError", "integer overflow (result exceeds 64-bit range)")
    return check_int(a**b)


def num_arith(op: str, a: Value, b: Value) -> Value:
    tb = type(b)
    if tb is not int and tb is not float:
        raise coerce_fail(a, b)
    if type(a) is int and tb is int:
        if op == "+":
            return check_int(a + b)
        if op == "-":
            return check_int(a - b)
        if op == "*":
            return check_int(a * b)
        if op == "/":
            if b == 0:
                raise RubyError("ZeroDivisionError", "divided by 0")
            return check_int(a // b)
        if op == "%":
            if b == 0:
                raise RubyError("ZeroDivisionError", "divided by 0")
            return a % b
        if op == "**":
            return int_pow(a, b)
    fa, fb = float(a), float(b)
    if op == "+":
        return fa + fb
    if op == "-":
        return fa - fb
    if op == "*":
        return fa * fb
    if op == "/":
        return fdiv(fa, fb)
    if op == "%":
        return float_mod(fa, fb)
    if op == "**":
        return fpow(fa, fb)
    raise AssertionError(op)


def _arith_handler(op: str) -> Handler:
    def h(it: Any, v: Value, args: list, blk: Any) -> Value:
        argc(args, 1)
        return num_arith(op, v, args[0])

    return h


for _op in ("+", "-", "*", "/", "%", "**"):
    NUMERIC[_op] = _arith_handler(_op)


def _cmp_handler(op: str) -> Handler:
    def h(it: Any, v: Value, args: list, blk: Any) -> Value:
        argc(args, 1)
        c = cmp_or_fail(v, args[0])
        if op == "<":
            return c < 0
        if op == "<=":
            return c <= 0
        if op == ">":
            return c > 0
        return c >= 0

    return h


for _op in ("<", "<=", ">", ">="):
    NUMERIC[_op] = _cmp_handler(_op)
    STRING[_op] = _cmp_handler(_op)


@method(NUMERIC, "-@")
def num_neg(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    if type(v) is int:
        return check_int(-v)
    return -v


@method(NUMERIC, "+@")
def num_pos(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v


@method(NUMERIC, "abs")
def num_abs(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    if type(v) is int:
        return check_int(abs(v))
    return abs(v)


@method(NUMERIC, "zero?")
def num_zero(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v == 0


@method(NUMERIC, "positive?")
def num_positive(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v > 0


@method(NUMERIC, "negative?")
def num_negative(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v < 0


@method(NUMERIC, "between?")
@method(STRING, "between?")
def cmp_between(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 2)
    return cmp_or_fail(v, args[0]) >= 0 and cmp_or_fail(v, args[1]) <= 0


@method(NUMERIC, "clamp")
@method(STRING, "clamp")
def cmp_clamp(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1, 2)
    if len(args) == 1:
        r = args[0]
        if type(r) is not RRange:
            raise no_implicit(r, "Range")
        if r.excl and r.last is not None:
            raise RubyError("ArgumentError", "cannot clamp with an exclusive range")
        lo, hi = r.first, r.last
    else:
        lo, hi = args
    if lo is not None and hi is not None and cmp_or_fail(lo, hi) > 0:
        raise RubyError("ArgumentError", "min argument must be less than or equal to max argument")
    if lo is not None and cmp_or_fail(v, lo) < 0:
        return lo
    if hi is not None and cmp_or_fail(v, hi) > 0:
        return hi
    return v


@method(NUMERIC, "divmod")
def num_divmod(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    b = args[0]
    if not is_num(b):
        raise coerce_fail(v, b)
    if type(v) is int and type(b) is int:
        if b == 0:
            raise RubyError("ZeroDivisionError", "divided by 0")
        return [check_int(v // b), v % b]
    d, m = flodivmod(float(v), float(b))
    return [float_to_int(d), m]


@method(NUMERIC, "fdiv")
def num_fdiv(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    b = args[0]
    if not is_num(b):
        raise coerce_fail(v, b)
    return fdiv(float(v), float(b))


@method(NUMERIC, "to_f")
def num_to_f(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return float(v)


@method(NUMERIC, "step")
def num_step(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0, 2)
    limit = args[0] if args else None
    stp = args[1] if len(args) > 1 else 1
    if blk is None:
        return REnumerator(v, "step", list(args))
    if not is_num(stp):
        raise coerce_fail(v, stp)
    if limit is not None and not is_num(limit):
        raise RubyError("ArgumentError", f"comparison of {class_name(v)} with {class_name(limit)} failed")
    if stp == 0:
        raise RubyError("ArgumentError", "step can't be 0")
    if type(v) is int and type(stp) is int and (limit is None or type(limit) is int):
        i = v
        while limit is None or (i <= limit if stp > 0 else i >= limit):
            it.step()
            it.yield_void(blk, i)
            i = check_int(i + stp)
        return v
    if limit is None:
        limit = math.inf if stp > 0 else -math.inf
    float_step(it, float(v), float(limit), float(stp), False, blk)
    return v


def float_step_size(beg: float, end: float, unit: float, excl: bool) -> float:
    if unit == 0:
        return math.inf
    if math.isinf(unit):
        if unit > 0:
            return 1.0 if beg <= end else 0.0
        return 1.0 if beg >= end else 0.0
    n = (end - beg) / unit
    err = (abs(beg) + abs(end) + abs(end - beg)) / abs(unit) * 2.220446049250313e-16
    if err > 0.5:
        err = 0.5
    if excl:
        if n <= 0:
            return 0
        if n < 1:
            n = 0
        else:
            n = math.floor(n - err)
        d = (n + 1) * unit + beg
        if beg < end:
            if d < end:
                n += 1
        elif beg > end:
            if d > end:
                n += 1
        return n
    if n < 0:
        return 0
    n = math.floor(n + err)
    return n + 1


def float_step(it: Any, beg: float, end: float, unit: float, excl: bool, blk: Any) -> None:
    n = float_step_size(beg, end, unit, excl)
    i = 0
    while i < n:
        it.step()
        d = i * unit + beg
        if (unit >= 0 and end < d) or (unit < 0 and d < end):
            d = end
        it.yield_void(blk, d)
        i += 1


# Integer ------------------------------------------------------------------------------


@method(INTEGER, "to_s", "inspect")
def int_to_s(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    base = to_int(args[0]) if args else 10
    if base < 2 or base > 36:
        raise RubyError("ArgumentError", f"invalid radix {base}")
    if base == 10:
        return str(v)
    digits = "0123456789abcdefghijklmnopqrstuvwxyz"
    n = abs(v)
    out = []
    while True:
        n, r = divmod(n, base)
        out.append(digits[r])
        if n == 0:
            break
    return ("-" if v < 0 else "") + "".join(reversed(out))


@method(INTEGER, "to_i", "ord")
def int_to_i(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v


@method(INTEGER, "succ", "next")
def int_succ(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return check_int(v + 1)


@method(INTEGER, "pred")
def int_pred(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return check_int(v - 1)


@method(INTEGER, "chr")
def int_chr(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    if v < 0 or v > 255:
        raise RubyError("RangeError", f"{v} out of char range")
    return chr(v)


@method(INTEGER, "times")
def int_times(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    if blk is None:
        return REnumerator(v, "times", [])
    i = 0
    while i < v:
        it.step()
        it.yield_void(blk, i)
        i += 1
    return v


@method(INTEGER, "upto")
def int_upto(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    if blk is None:
        return REnumerator(v, "upto", list(args))
    lim = args[0]
    if not is_num(lim):
        raise RubyError("ArgumentError", f"comparison of Integer with {class_name(lim)} failed")
    i = v
    while i <= lim:
        it.step()
        it.yield_void(blk, i)
        i += 1
    return v


@method(INTEGER, "downto")
def int_downto(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    if blk is None:
        return REnumerator(v, "downto", list(args))
    lim = args[0]
    if not is_num(lim):
        raise RubyError("ArgumentError", f"comparison of Integer with {class_name(lim)} failed")
    i = v
    while i >= lim:
        it.step()
        it.yield_void(blk, i)
        i -= 1
    return v


@method(INTEGER, "even?")
def int_even(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v % 2 == 0


@method(INTEGER, "odd?")
def int_odd(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v % 2 == 1


def _int_arg(v: Value, b: Value) -> int:
    if type(b) is not int:
        if type(b) is float:
            raise RubyError("TypeError", "not an integer")
        raise coerce_fail(v, b)
    return b


@method(INTEGER, "gcd")
def int_gcd(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    return check_int(math.gcd(v, _int_arg(v, args[0])))


@method(INTEGER, "lcm")
def int_lcm(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    b = _int_arg(v, args[0])
    if v == 0 or b == 0:
        return 0
    return check_int(abs(v * b) // math.gcd(v, b))


@method(INTEGER, "digits")
def int_digits(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    base = to_int(args[0]) if args else 10
    if v < 0:
        raise RubyError("Math::DomainError", "out of domain")
    if base < 2:
        raise RubyError("ArgumentError", f"invalid radix {base}")
    if v == 0:
        return [0]
    out = []
    while v > 0:
        v, r = divmod(v, base)
        out.append(r)
    return out


@method(INTEGER, "bit_length")
def int_bit_length(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v.bit_length() if v >= 0 else (~v).bit_length()


def int_round_digits(n: int, nd: int, mode: str) -> int:
    """Rounds an Integer to a multiple of 10**-nd (nd < 0)."""
    if -nd >= 20:
        if mode == "floor" and n < 0:
            raise RubyError("RangeError", "integer overflow (result exceeds 64-bit range)")
        if mode == "ceil" and n > 0:
            raise RubyError("RangeError", "integer overflow (result exceeds 64-bit range)")
        return 0
    m = 10 ** (-nd)
    if mode == "floor":
        return check_int(n - n % m)
    if mode == "ceil":
        return check_int(-((-n) - (-n) % m))
    if mode == "truncate":
        q = abs(n) // m * m
        return check_int(q if n >= 0 else -q)
    q, r = divmod(abs(n), m)
    if r * 2 >= m:
        q += 1
    res = q * m
    return check_int(res if n >= 0 else -res)


def _int_rounder(mode: str) -> Handler:
    def h(it: Any, v: Value, args: list, blk: Any) -> Value:
        nd = _nd_arg(args)
        if nd >= 0:
            return v
        return int_round_digits(v, nd, mode)

    return h


INTEGER["round"] = _int_rounder("round")
INTEGER["floor"] = _int_rounder("floor")
INTEGER["ceil"] = _int_rounder("ceil")
INTEGER["truncate"] = _int_rounder("truncate")


def _int_bitop(op: str) -> Handler:
    def h(it: Any, v: Value, args: list, blk: Any) -> Value:
        argc(args, 1)
        b = args[0]
        if type(b) is not int:
            if type(b) is float:
                raise coerce_fail(v, b)
            raise coerce_fail(v, b)
        if op == "&":
            return v & b
        if op == "|":
            return v | b
        if op == "^":
            return v ^ b
        if op == "<<":
            return int_shift(v, b)
        return int_shift(v, -b)

    return h


def int_shift(v: int, n: int) -> int:
    if n >= 0:
        if v == 0:
            return 0
        if n > 64:
            raise RubyError("RangeError", "integer overflow (result exceeds 64-bit range)")
        return check_int(v << n)
    n = -n
    if n > 64:
        return -1 if v < 0 else 0
    return v >> n


for _op in ("&", "|", "^", "<<", ">>"):
    INTEGER[_op] = _int_bitop(_op)


@method(INTEGER, "~")
def int_invert(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return ~v


# Float --------------------------------------------------------------------------------


def float_to_int(f: float) -> int:
    if f != f:
        raise RubyError("FloatDomainError", "NaN")
    if math.isinf(f):
        raise RubyError("FloatDomainError", "Infinity" if f > 0 else "-Infinity")
    r = int(f)
    if r < INT_MIN or r > INT_MAX:
        raise RubyError("RangeError", f"float {float_to_s(f)} out of range of integer")
    return r


def c_round(x: float) -> float:
    """C ``round()``: half away from zero."""
    if not math.isfinite(x):
        return x
    t = math.copysign(float(math.trunc(x)), x)
    if abs(x - t) >= 0.5:
        t += math.copysign(1.0, x)
    return t


def round_half_up(x: float, s: float) -> float:
    xs = x * s
    f = c_round(xs)
    if s == 1.0:
        return f
    if x > 0:
        if (f + 0.5) / s <= x:
            f += 1
        x = f
    else:
        if (f - 0.5) / s >= x:
            f -= 1
        x = f
    return x


def _frexp_exp(x: float) -> int:
    return math.frexp(x)[1]


def _c_div(a: int, b: int) -> int:
    q = abs(a) // abs(b)
    return q if (a >= 0) == (b > 0) else -q


def float_round_overflow(nd: int, binexp: int) -> bool:
    return nd >= 17 - (_c_div(binexp, 4) if binexp > 0 else _c_div(binexp, 3) - 1)


def float_round_underflow(nd: int, binexp: int) -> bool:
    return nd < -(_c_div(binexp, 3) + 1 if binexp > 0 else _c_div(binexp, 4))


def _round_by_rational(x: float, nd: int) -> float:
    """Ruby's rb_flo_round_by_rational: x.to_r.round(nd) (half away from zero), then
    Rational#to_f, which divides the reduced numerator and denominator as doubles."""
    exact = Fraction(x)
    scaled = abs(exact) * 10**nd
    q = math.floor(scaled)
    if scaled - q >= Fraction(1, 2):
        q += 1
    if x < 0:
        q = -q
    rat = Fraction(q, 10**nd)
    num, den = rat.numerator, rat.denominator
    if num == 0:
        return 0.0
    fixnum = 2**62
    if abs(num) < fixnum and den < 2**53:
        return float(num) / float(den)
    if abs(num) >= fixnum and den < fixnum:
        dx = _big2dbl(abs(num))
        r = _big_fdiv_int(abs(num), den) if math.isinf(dx) else dx / float(den)
        return -r if num < 0 else r
    r = _big_fdiv_int(abs(num), den)
    return -r if num < 0 else r


_BDIG = 32  # Ruby's BDIGIT is a 32-bit unsigned int (internal/bignum.h)
_BDMAX = (1 << _BDIG) - 1
_BIGRAD = float(1 << _BDIG)


def _big2dbl(x: int) -> float:
    """Port of bignum.c big2dbl (magnitude only)."""
    if x == 0:
        return 0.0
    ds = []
    t = x
    while t:
        ds.append(t & _BDMAX)
        t >>= _BDIG
    i = len(ds)
    bits = x.bit_length()
    if bits > 53 + 1024:
        return math.inf
    lo = 0
    if bits > 54:
        bits -= 54
        lo = bits // _BDIG
    else:
        bits = 0
    d = 0.0
    i -= 1
    while i > lo:
        d = ds[i] + _BIGRAD * d
        i -= 1
    dl = ds[i]
    if bits:
        bits %= _BDIG
        if dl & (1 << bits):
            carry = (dl & ~((_BDMAX << bits) & _BDMAX) & _BDMAX) != 0
            if not carry:
                j = i
                while j > 0:
                    j -= 1
                    if ds[j] != 0:
                        carry = True
                        break
            if carry:
                mask = (_BDMAX << bits) & _BDMAX
                bit = 1 << bits
                dl &= mask
                dl += bit
                dl &= _BDMAX
                if not dl:
                    d += 1
    d = dl + _BIGRAD * d
    if lo:
        d = math.ldexp(d, lo * _BDIG)
    return d


def _big_shift(x: int, n: int) -> int:
    return x >> n if n > 0 else x << -n


def _big_fdiv_int(x: int, y: int) -> float:
    """Port of bignum.c big_fdiv_int/big_fdiv for positive x, y (DBL_BIGDIG == 2)."""
    dbl_bigdig = (53 + _BDIG) // _BDIG
    ey = y.bit_length() - dbl_bigdig * _BDIG
    y = _big_shift(y, ey)
    ex = x.bit_length() - 2 * dbl_bigdig * _BDIG
    if ex > _BDIG:
        ex -= _BDIG
    elif ex > 0:
        ex = 0
    x = _big_shift(x, ex)
    z = x // y
    try:
        return math.ldexp(_big2dbl(z), ex - ey)
    except OverflowError:
        return math.inf


def float_round(x: float, nd: int) -> Value:
    if nd > 0:
        if x == 0.0 or not math.isfinite(x):
            return x
        binexp = _frexp_exp(x)
        if float_round_overflow(nd, binexp):
            return x
        if float_round_underflow(nd, binexp):
            return 0.0
        if nd > 14:
            return _round_by_rational(x, nd)
        s = 10.0**nd
        return round_half_up(x, s) / s
    if x == 0.0:
        return 0
    if nd == 0:
        return float_to_int(c_round(x))
    return int_round_digits(float_to_int(x), nd, "round")


def float_floor(x: float, nd: int) -> Value:
    if x == 0.0:
        return x if nd > 0 else 0
    if nd > 0:
        if not math.isfinite(x):
            return x
        binexp = _frexp_exp(x)
        if float_round_overflow(nd, binexp):
            return x
        if x > 0.0 and float_round_underflow(nd, binexp):
            return 0.0
        f = 10.0**nd
        mul = math.floor(x * f)
        res = (mul + 1) / f
        if res > x:
            res = mul / f
        return res
    r = float_to_int(float(math.floor(x)) if math.isfinite(x) else x)
    if nd < 0:
        r = int_round_digits(r, nd, "floor")
    return r


def float_ceil(x: float, nd: int) -> Value:
    if x == 0.0:
        return x if nd > 0 else 0
    if nd > 0:
        if not math.isfinite(x):
            return x
        binexp = _frexp_exp(x)
        if float_round_overflow(nd, binexp):
            return x
        if x < 0.0 and float_round_underflow(nd, binexp):
            return 0.0
        f = 10.0**nd
        c = math.copysign(float(math.ceil(x * f)), x)  # C ceil() keeps the sign of -0.0
        return c / f  # unlike floor, Ruby 3.4's ceil has no correction step
    r = float_to_int(float(math.ceil(x)) if math.isfinite(x) else x)
    if nd < 0:
        r = int_round_digits(r, nd, "ceil")
    return r


def _nd_arg(args: list) -> int:
    argc(args, 0, 1)
    if args and type(args[-1]) is RHash:
        raise RubyError("ArgumentError", "keyword options (such as half:) are not supported")
    return to_int(args[0]) if args else 0


@method(FLOAT, "round")
def flo_round(it: Any, v: Value, args: list, blk: Any) -> Value:
    return float_round(v, _nd_arg(args))


@method(FLOAT, "floor")
def flo_floor(it: Any, v: Value, args: list, blk: Any) -> Value:
    return float_floor(v, _nd_arg(args))


@method(FLOAT, "ceil")
def flo_ceil(it: Any, v: Value, args: list, blk: Any) -> Value:
    return float_ceil(v, _nd_arg(args))


@method(FLOAT, "to_i", "truncate")
def flo_to_i(it: Any, v: Value, args: list, blk: Any) -> Value:
    nd = _nd_arg(args)
    if nd > 0:
        return float_floor(v, nd) if v > 0 else float_ceil(v, nd)
    r = float_to_int(v)
    if nd < 0:
        r = int_round_digits(r, nd, "truncate")
    return r


@method(FLOAT, "to_s", "inspect")
def flo_to_s(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return float_to_s(v)


@method(FLOAT, "nan?")
def flo_nan(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v != v


@method(FLOAT, "finite?")
def flo_finite(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return math.isfinite(v)


@method(FLOAT, "infinite?")
def flo_infinite(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    if math.isinf(v):
        return 1 if v > 0 else -1
    return None


# ---------------------------------------------------------------------------------------
# Symbol


@method(SYMBOL, "to_s", "id2name", "name")
def sym_to_s(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v.name


@method(SYMBOL, "to_sym")
def sym_to_sym(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v


@method(SYMBOL, "length", "size")
def sym_length(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return len(v.name)


@method(SYMBOL, "empty?")
def sym_empty(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v.name == ""


def _sym_from_str(name: str) -> Handler:
    def h(it: Any, v: Value, args: list, blk: Any) -> Value:
        return Symbol(STRING[name](it, v.name, args, blk))

    return h


def _sym_pred_from_str(name: str) -> Handler:
    def h(it: Any, v: Value, args: list, blk: Any) -> Value:
        return STRING[name](it, v.name, args, blk)

    return h


# ---------------------------------------------------------------------------------------
# Class values


@method(CLASS, "name", "to_s", "inspect")
def cls_name(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v.name


@method(CLASS, "===")
def cls_case_eq(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    return is_a(args[0], v)


@singleton("Array", "new")
def array_new(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0, 2)
    if not args:
        return []
    if len(args) == 1 and type(args[0]) is list:
        return list(args[0])
    n = to_int(args[0])
    if n < 0:
        raise RubyError("ArgumentError", "negative array size")
    if blk is not None:
        out = []
        for i in range(n):
            it.step()
            out.append(it.yield_(blk, i))
        return out
    fill = args[1] if len(args) > 1 else None
    if n > 10_000_000:
        raise RubyError("LimitError", "array larger than 10000000 elements")
    return [fill] * n


@singleton("Hash", "new")
def hash_new(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    if blk is not None:
        raise RubyError("ArgumentError", "Hash.new with a block is not supported")
    return RHash(args[0] if args else None)


# ---------------------------------------------------------------------------------------
# Functions


@function("format", "sprintf")
def fn_format(it: Any, args: list, blk: Any) -> Value:
    argc(args, 1, -1)
    from .formatting import format_str

    return format_str(to_str(args[0]), args[1:])


@function("Integer")
def fn_integer(it: Any, args: list, blk: Any) -> Value:
    argc(args, 1, 2)
    from .builtins_str import parse_integer_strict

    x = args[0]
    base = args[1] if len(args) > 1 else None
    if type(base) is RHash:
        base = None
    if base is not None:
        base = to_int(base)
        if not isinstance(x, str):
            raise RubyError("ArgumentError", "base specified for non string value")
    if type(x) is int:
        return x
    if type(x) is float:
        return float_to_int(x)
    if isinstance(x, str):
        return parse_integer_strict(x, base)
    if x is None:
        raise RubyError("TypeError", "can't convert nil into Integer")
    raise RubyError("TypeError", f"can't convert {class_name(x)} into Integer")


@function("Float")
def fn_float(it: Any, args: list, blk: Any) -> Value:
    argc(args, 1, 2)
    from .builtins_str import parse_float_strict

    x = args[0]
    if type(x) is float:
        return x
    if type(x) is int:
        return float(x)
    if isinstance(x, str):
        return parse_float_strict(x)
    if x is None:
        raise RubyError("TypeError", "can't convert nil into Float")
    raise RubyError("TypeError", f"can't convert {class_name(x)} into Float")


@function("String")
def fn_string(it: Any, args: list, blk: Any) -> Value:
    argc(args, 1)
    return to_s(args[0])


@function("Array")
def fn_array(it: Any, args: list, blk: Any) -> Value:
    argc(args, 1)
    x = args[0]
    if x is None:
        return []
    if type(x) is list:
        return x
    if type(x) is RHash:
        return [[k, v] for k, v in x.items()]
    if type(x) is RRange or type(x) is REnumerator:
        return it.call_method(x, "to_a", [], None)
    return [x]


@function("raise")
def fn_raise(it: Any, args: list, blk: Any) -> Value:
    argc(args, 0, 2)
    if not args:
        raise RubyError("RuntimeError", "unhandled exception")
    msg = args[0]
    if not isinstance(msg, str):
        raise RubyError("TypeError", "exception class/object expected")
    raise RubyError("RuntimeError", msg)


@function("loop")
def fn_loop(it: Any, args: list, blk: Any) -> Value:
    argc(args, 0)
    if blk is None:
        raise RubyError("ArgumentError", "loop requires a block")
    while True:
        it.step()
        it.yield_void(blk)


@function("render")
def fn_render(it: Any, args: list, blk: Any) -> Value:
    argc(args, 1, 2)
    return it.render_partial(args[0], args[1] if len(args) > 1 else None)


def lookup_const(parent: Value, name: str, line: int) -> Value:
    from .consts import CONSTANTS, SCOPED

    if parent is None:
        v = CONSTANTS.get(name)
        if v is None:
            raise RubyError("NameError", f"uninitialized constant {name}", line)
        return v
    if type(parent) is not RClass:
        raise RubyError("TypeError", f"{inspect(parent)} is not a class/module", line)
    key = parent.name + "::" + name
    if key in SCOPED:
        return SCOPED[key]
    raise RubyError("NameError", f"uninitialized constant {key}", line)


def _register_late() -> None:
    from . import builtins_coll, builtins_str, sandbox  # noqa: F401
    from .jsonlib import register as json_register
    from .mathlib import register as math_register

    for n in ("upcase", "downcase", "capitalize"):
        SYMBOL[n] = _sym_from_str(n)
    for n in ("start_with?", "end_with?"):
        SYMBOL[n] = _sym_pred_from_str(n)
    json_register()
    math_register()


_register_late()
