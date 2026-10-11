# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

"""Array, Hash, Range, Enumerable and Enumerator methods (spec §8.7–8.10)."""

from __future__ import annotations

import functools
import itertools
import math
from collections.abc import Iterator
from typing import Any

from .builtins import (
    ARRAY,
    ENUMERABLE,
    ENUMERATOR,
    HASH,
    RANGE,
    argc,
    case_eq,
    find_method,
    float_step,
    method,
    no_implicit,
    range_cover,
    to_int,
    to_str,
)
from .builtins_str import range_span, seq_index
from .errors import RubyError
from .interpreter import NativeBlock
from .values import (
    REnumerator,
    RHash,
    RRange,
    Symbol,
    Value,
    check_int,
    class_name,
    cmp,
    cmp_or_fail,
    hash_key,
    inspect,
    range_int_bounds,
    range_iter,
    req,
    to_s,
    truthy,
)

MAX_ARRAY = 10_000_000

# ---------------------------------------------------------------------------------------
# Iteration


def enum_to_list(it: Any, e: REnumerator) -> list:
    out: list = []

    def collect(vals: list) -> Value:
        out.append(vals[0] if len(vals) == 1 else list(vals))
        return None

    it.call_method(e.recv, e.meth, e.args, NativeBlock(collect))
    return out


def each_items(it: Any, v: Value) -> Iterator[Value]:
    t = type(v)
    if t is list:
        i = 0
        while i < len(v):
            yield v[i]
            i += 1
    elif t is RHash:
        for k, val in v.items():
            yield [k, val]
    elif t is RRange:
        for x in range_iter(v):
            it.step()
            yield x
    elif t is REnumerator:
        yield from enum_to_list(it, v)
    else:
        raise RubyError("TypeError", f"can't iterate {class_name(v)}")


def to_list(it: Any, v: Value) -> list:
    t = type(v)
    if t is list:
        return list(v)
    if t is RRange and v.last is None and v.first is not None:
        raise RubyError("RangeError", "cannot convert endless range to an array")
    out = []
    for x in each_items(it, v):
        out.append(x)
        if len(out) > MAX_ARRAY:
            raise RubyError("LimitError", "collection too large")
    return out


def y1(it: Any, blk: Any, x: Value) -> Value:
    return it.yield_(blk, x)


def enum_or(it: Any, v: Value, name: str, args: list, blk: Any) -> REnumerator | None:
    if blk is None:
        return REnumerator(v, name, list(args))
    return None


def sort_values(it: Any, items: list, blk: Any = None) -> list:
    if blk is None:
        return sorted(items, key=functools.cmp_to_key(cmp_or_fail))

    def c(a: Value, b: Value) -> int:
        r = it.yield_(blk, a, b)
        if type(r) is int:
            return r
        if type(r) is float and r == r:
            return -1 if r < 0 else (1 if r > 0 else 0)
        raise RubyError("ArgumentError", f"comparison of {class_name(a)} with {class_name(b)} failed")

    return sorted(items, key=functools.cmp_to_key(c))


def sort_by_values(it: Any, items: list, blk: Any) -> list:
    keyed = [(it.yield_(blk, x), x) for x in items]
    keyed.sort(key=functools.cmp_to_key(lambda a, b: cmp_or_fail(a[0], b[0])))
    return [x for _, x in keyed]


def to_array_arg(it: Any, v: Value) -> list:
    if type(v) is list:
        return v
    if type(v) in (RHash, RRange, REnumerator):
        return to_list(it, v)
    raise RubyError("TypeError", f"wrong argument type {class_name(v)} (must respond to :each)")


# ---------------------------------------------------------------------------------------
# sum (Array#sum and Enumerable#sum, including Kahan-Babuska summation)


def ruby_sum(it: Any, items: Iterator[Value], init: Value, blk: Any, enum_mode: bool) -> Value:
    v = init
    n = 0
    float_mode = enum_mode and type(v) is float
    exact = type(v) is int and not float_mode
    f = float(v) if float_mode else 0.0
    c = 0.0
    generic = not exact and not float_mode
    for e in items:
        if blk is not None:
            e = it.yield_(blk, e)
        if generic:
            v = it.call_method(v, "+", [e], None)
            continue
        if exact:
            if type(e) is int:
                n += e
                continue
            v = v + n
            n = 0
            exact = False
            if type(e) is float:
                f = float(v)
                c = 0.0
                float_mode = True
            else:
                generic = True
                v = it.call_method(v, "+", [e], None)
                continue
        # float mode (Kahan-Babuska)
        if type(e) is float:
            x = e
        elif type(e) is int:
            x = float(e)
        else:
            v = f
            float_mode = False
            generic = True
            v = it.call_method(v, "+", [e], None)
            continue
        if f != f:
            continue
        if x != x:
            f = x
            continue
        if math.isinf(x):
            if math.isinf(f) and (x < 0) != (f < 0):
                f = math.nan
            else:
                f = x
            continue
        if math.isinf(f):
            continue
        t = f + x
        if abs(f) >= abs(x):
            c += (f - t) + x
        else:
            c += (x - t) + f
        f = t
    if float_mode:
        return f + c
    if exact:
        return check_int(v + n)
    return v


# ---------------------------------------------------------------------------------------
# Enumerable

E = ENUMERABLE


def _e(*names: str):  # noqa: ANN202
    return method(E, *names)


@_e("each_with_index")
def en_each_with_index(it: Any, v: Value, args: list, blk: Any) -> Value:
    e = enum_or(it, v, "each_with_index", args, blk)
    if e is not None:
        return e
    i = 0
    for x in each_items(it, v):
        it.yield_void(blk, x, i)
        i += 1
    return v


@_e("each_with_object")
def en_each_with_object(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    e = enum_or(it, v, "each_with_object", args, blk)
    if e is not None:
        return e
    memo = args[0]
    for x in each_items(it, v):
        it.yield_(blk, x, memo)
    return memo


@_e("each_slice")
def en_each_slice(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    n = to_int(args[0])
    if n <= 0:
        raise RubyError("ArgumentError", "invalid slice size")
    e = enum_or(it, v, "each_slice", args, blk)
    if e is not None:
        return e
    buf: list = []
    for x in each_items(it, v):
        buf.append(x)
        if len(buf) == n:
            it.yield_void(blk, buf)
            buf = []
    if buf:
        it.yield_void(blk, buf)
    return v


@_e("each_cons")
def en_each_cons(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    n = to_int(args[0])
    if n <= 0:
        raise RubyError("ArgumentError", "invalid size")
    e = enum_or(it, v, "each_cons", args, blk)
    if e is not None:
        return e
    buf: list = []
    for x in each_items(it, v):
        buf.append(x)
        if len(buf) > n:
            buf.pop(0)
        if len(buf) == n:
            it.yield_void(blk, list(buf))
    return v


@_e("each_entry")
def en_each_entry(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "each_entry", args, blk)
    if e is not None:
        return e
    for x in each_items(it, v):
        it.yield_void(blk, x)
    return v


@_e("map", "collect")
def en_map(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "map", args, blk)
    if e is not None:
        return e
    return [it.yield_(blk, x) for x in each_items(it, v)]


@_e("flat_map", "collect_concat")
def en_flat_map(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "flat_map", args, blk)
    if e is not None:
        return e
    out: list = []
    for x in each_items(it, v):
        r = it.yield_(blk, x)
        if type(r) is list:
            out.extend(r)
        else:
            out.append(r)
    return out


@_e("filter_map")
def en_filter_map(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "filter_map", args, blk)
    if e is not None:
        return e
    out = []
    for x in each_items(it, v):
        r = it.yield_(blk, x)
        if truthy(r):
            out.append(r)
    return out


@_e("select", "filter", "find_all")
def en_select(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "select", args, blk)
    if e is not None:
        return e
    return [x for x in each_items(it, v) if truthy(it.yield_(blk, x))]


@_e("reject")
def en_reject(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "reject", args, blk)
    if e is not None:
        return e
    return [x for x in each_items(it, v) if not truthy(it.yield_(blk, x))]


@_e("find", "detect")
def en_find(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "find", args, blk)
    if e is not None:
        return e
    for x in each_items(it, v):
        if truthy(it.yield_(blk, x)):
            return x
    return None


@_e("find_index")
def en_find_index(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    if not args and blk is None:
        return REnumerator(v, "find_index", [])
    i = 0
    for x in each_items(it, v):
        if req(x, args[0]) if args else truthy(it.yield_(blk, x)):
            return i
        i += 1
    return None


def _pred(name: str) -> Any:
    def h(it: Any, v: Value, args: list, blk: Any) -> Value:
        argc(args, 0, 1)
        count = 0
        for x in each_items(it, v):
            if args:
                ok = case_eq(it, args[0], x)
            elif blk is not None:
                ok = truthy(it.yield_(blk, x))
            else:
                ok = truthy(x)
            if name == "any?" and ok:
                return True
            if name == "all?" and not ok:
                return False
            if name == "none?" and ok:
                return False
            if name == "one?" and ok:
                count += 1
                if count > 1:
                    return False
        if name == "any?":
            return False
        if name == "one?":
            return count == 1
        return True

    return h


for _n in ("any?", "all?", "none?", "one?"):
    E[_n] = _pred(_n)


@_e("count")
def en_count(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    if not args and blk is None:
        if type(v) in (list, RHash):
            return len(v)
        return sum(1 for _ in each_items(it, v))
    n = 0
    for x in each_items(it, v):
        if req(x, args[0]) if args else truthy(it.yield_(blk, x)):
            n += 1
    return n


@_e("include?", "member?")
def en_include(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    return any(req(x, args[0]) for x in each_items(it, v))


def _minmax_impl(it: Any, v: Value, args: list, blk: Any, sign: int) -> Value:
    argc(args, 0, 1)
    if blk is not None:

        def c(a: Value, b: Value) -> int:
            r = it.yield_(blk, a, b)
            if type(r) is not int:
                if type(r) is float and r == r:
                    return -1 if r < 0 else (1 if r > 0 else 0)
                raise RubyError("ArgumentError", f"comparison of {class_name(a)} with {class_name(b)} failed")
            return r

    else:
        c = cmp_or_fail
    if args and args[0] is not None:
        n = to_int(args[0])
        if n < 0:
            raise RubyError("ArgumentError", f"negative size ({n})")
        items = to_list(it, v)
        srt = sorted(items, key=functools.cmp_to_key(c))
        if sign > 0:
            srt = sorted(items, key=functools.cmp_to_key(lambda a, b: -c(a, b)))
        return srt[:n]
    best = None
    first = True
    for x in each_items(it, v):
        if first:
            best = x
            first = False
        elif c(x, best) * sign > 0:
            best = x
    return best


@_e("min")
def en_min(it: Any, v: Value, args: list, blk: Any) -> Value:
    return _minmax_impl(it, v, args, blk, -1)


@_e("max")
def en_max(it: Any, v: Value, args: list, blk: Any) -> Value:
    return _minmax_impl(it, v, args, blk, 1)


@_e("minmax")
def en_minmax(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return [_minmax_impl(it, v, [], blk, -1), _minmax_impl(it, v, [], blk, 1)]


def _by_impl(it: Any, v: Value, args: list, blk: Any, sign: int, name: str) -> Value:
    argc(args, 0, 1)
    e = enum_or(it, v, name, args, blk)
    if e is not None:
        return e
    if args and args[0] is not None:
        n = to_int(args[0])
        items = sort_by_values(it, to_list(it, v), blk)
        if sign > 0:
            keyed = [(it.yield_(blk, x), x) for x in to_list(it, v)]
            keyed.sort(key=functools.cmp_to_key(lambda a, b: -cmp_or_fail(a[0], b[0])))
            items = [x for _, x in keyed]
        return items[:n]
    best = None
    best_key = None
    first = True
    for x in each_items(it, v):
        k = it.yield_(blk, x)
        if first:
            best, best_key, first = x, k, False
        elif cmp_or_fail(k, best_key) * sign > 0:
            best, best_key = x, k
    return best


@_e("min_by")
def en_min_by(it: Any, v: Value, args: list, blk: Any) -> Value:
    return _by_impl(it, v, args, blk, -1, "min_by")


@_e("max_by")
def en_max_by(it: Any, v: Value, args: list, blk: Any) -> Value:
    return _by_impl(it, v, args, blk, 1, "max_by")


@_e("sum")
def en_sum(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    init = args[0] if args else 0
    return ruby_sum(it, each_items(it, v), init, blk, True)


@_e("inject", "reduce")
def en_inject(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0, 2)
    op = None
    has_init = False
    init = None
    if len(args) == 2:
        has_init, init, op = True, args[0], args[1]
    elif len(args) == 1:
        if blk is None:
            op = args[0]
        else:
            has_init, init = True, args[0]
    elif blk is None:
        raise RubyError("ArgumentError", "no block given")
    if op is not None:
        if type(op) is Symbol:
            op = op.name
        elif isinstance(op, str):
            op = str(op)
        else:
            raise RubyError("TypeError", f"{inspect(op)} is not a symbol nor a string")
    acc = init
    first = not has_init
    for x in each_items(it, v):
        if first:
            acc = x
            first = False
            continue
        if op is not None:
            acc = it.call_method(acc, op, [x], None)
        else:
            acc = it.yield_(blk, acc, x)
    return acc


@_e("group_by")
def en_group_by(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "group_by", args, blk)
    if e is not None:
        return e
    h = RHash()
    for x in each_items(it, v):
        k = it.yield_(blk, x)
        cur = h.get(k, None)
        if cur is None and not h.has(k):
            cur = []
            h.set(k, cur)
        cur.append(x)
    return h


@_e("partition")
def en_partition(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "partition", args, blk)
    if e is not None:
        return e
    a: list = []
    b: list = []
    for x in each_items(it, v):
        (a if truthy(it.yield_(blk, x)) else b).append(x)
    return [a, b]


def _chunk(it: Any, v: Value, blk: Any, split_when_true: bool) -> list:
    if blk is None:
        raise RubyError("ArgumentError", "no block given")
    out: list = []
    cur: list = []
    prev = None
    for x in each_items(it, v):
        if cur:
            r = truthy(it.yield_(blk, prev, x))
            if r == split_when_true:
                out.append(cur)
                cur = []
        cur.append(x)
        prev = x
    if cur:
        out.append(cur)
    return out


@_e("chunk_while")
def en_chunk_while(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return _chunk(it, v, blk, False)


@_e("slice_when")
def en_slice_when(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return _chunk(it, v, blk, True)


@_e("tally")
def en_tally(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    h = RHash()
    for x in each_items(it, v):
        h.set(x, h.get(x, 0) + 1)
    return h


@_e("zip")
def en_zip(it: Any, v: Value, args: list, blk: Any) -> Value:
    others = [to_array_arg(it, a) for a in args]
    out = []
    for i, x in enumerate(each_items(it, v)):
        row = [x] + [o[i] if i < len(o) else None for o in others]
        if blk is not None:
            it.yield_(blk, row)
        else:
            out.append(row)
    return None if blk is not None else out


@_e("take")
def en_take(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    n = to_int(args[0])
    if n < 0:
        raise RubyError("ArgumentError", "attempt to take negative size")
    out: list = []
    if n == 0:
        return out
    for x in each_items(it, v):
        out.append(x)
        if len(out) >= n:
            break
    return out


@_e("drop")
def en_drop(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 1)
    n = to_int(args[0])
    if n < 0:
        raise RubyError("ArgumentError", "attempt to drop negative size")
    return to_list(it, v)[n:]


@_e("take_while")
def en_take_while(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "take_while", args, blk)
    if e is not None:
        return e
    out = []
    for x in each_items(it, v):
        if not truthy(it.yield_(blk, x)):
            break
        out.append(x)
    return out


@_e("drop_while")
def en_drop_while(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "drop_while", args, blk)
    if e is not None:
        return e
    out = []
    dropping = True
    for x in each_items(it, v):
        if dropping and truthy(it.yield_(blk, x)):
            continue
        dropping = False
        out.append(x)
    return out


@_e("first")
def en_first(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    if not args:
        for x in each_items(it, v):
            return x
        return None
    n = to_int(args[0])
    if n < 0:
        raise RubyError("ArgumentError", "attempt to take negative size")
    return en_take(it, v, [n], None)


@_e("sort")
def en_sort(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return sort_values(it, to_list(it, v), blk)


@_e("sort_by")
def en_sort_by(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "sort_by", args, blk)
    if e is not None:
        return e
    return sort_by_values(it, to_list(it, v), blk)


@_e("uniq")
def en_uniq(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    seen = set()
    out = []
    for x in each_items(it, v):
        k = hash_key(it.yield_(blk, x) if blk is not None else x)
        if k in seen:
            continue
        seen.add(k)
        out.append(x)
    return out


@_e("to_a", "entries")
def en_to_a(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return to_list(it, v)


def pairs_to_hash(it: Any, items: Iterator[Value], blk: Any) -> RHash:
    h = RHash()
    for i, x in enumerate(items):
        if blk is not None:
            x = it.yield_(blk, x)
        if type(x) is not list:
            raise RubyError("TypeError", f"wrong element type {class_name(x)} at {i} (expected array)")
        if len(x) != 2:
            raise RubyError("ArgumentError", f"wrong array length at {i} (expected 2, was {len(x)})")
        h.set(x[0], x[1])
    return h


@_e("to_h")
def en_to_h(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    return pairs_to_hash(it, each_items(it, v), blk)


@_e("reverse_each")
def en_reverse_each(it: Any, v: Value, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "reverse_each", args, blk)
    if e is not None:
        return e
    for x in reversed(to_list(it, v)):
        it.yield_void(blk, x)
    return v


# ---------------------------------------------------------------------------------------
# Enumerator


@method(ENUMERATOR, "each")
def enr_each(it: Any, e: REnumerator, args: list, blk: Any) -> Value:
    argc(args, 0)
    if blk is None:
        return e
    return it.call_method(e.recv, e.meth, e.args, blk)


def _with_index(it: Any, e: REnumerator, off: int, blk: Any) -> Value:
    counter = [off]

    def fn(vals: list) -> Value:
        x = vals[0] if len(vals) == 1 else list(vals)
        i = counter[0]
        counter[0] += 1
        return it.yield_(blk, x, i)

    return it.call_method(e.recv, e.meth, e.args, NativeBlock(fn))


@method(ENUMERATOR, "with_index")
def enr_with_index(it: Any, e: REnumerator, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    if blk is None:
        return REnumerator(e, "with_index", list(args))
    off = 0 if not args or args[0] is None else to_int(args[0])
    return _with_index(it, e, off, blk)


@method(ENUMERATOR, "each_with_index")
def enr_each_with_index(it: Any, e: REnumerator, args: list, blk: Any) -> Value:
    argc(args, 0)
    if blk is None:
        return REnumerator(e, "each_with_index", [])
    return _with_index(it, e, 0, blk)


@method(ENUMERATOR, "with_object", "each_with_object")
def enr_with_object(it: Any, e: REnumerator, args: list, blk: Any) -> Value:
    argc(args, 1)
    if blk is None:
        return REnumerator(e, "with_object", list(args))
    memo = args[0]

    def fn(vals: list) -> Value:
        x = vals[0] if len(vals) == 1 else list(vals)
        return it.yield_(blk, x, memo)

    it.call_method(e.recv, e.meth, e.args, NativeBlock(fn))
    return memo


@method(ENUMERATOR, "size")
def enr_size(it: Any, e: REnumerator, args: list, blk: Any) -> Value:
    argc(args, 0)
    return len(enum_to_list(it, e))


# ---------------------------------------------------------------------------------------
# Array

A = ARRAY


def _a(*names: str):  # noqa: ANN202
    return method(A, *names)


def _check_ary_size(n: int) -> None:
    if n > MAX_ARRAY:
        raise RubyError("LimitError", f"array larger than {MAX_ARRAY} elements")


@_a("[]", "slice")
def a_index(it: Any, v: list, args: list, blk: Any) -> Value:
    r = seq_index(v, args, [])
    return r


def _splice(v: list, start: int, length: int, val: Value) -> None:
    rep = list(val) if type(val) is list else [val]
    n = len(v)
    if start > n:
        _check_ary_size(start)
        v.extend([None] * (start - n))
    v[start : start + length] = rep


@_a("[]=")
def a_aset(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 2, 3)
    n = len(v)
    if len(args) == 3:
        start = to_int(args[0])
        length = to_int(args[1])
        if start < 0:
            if start + n < 0:
                raise RubyError("IndexError", f"index {start} too small for array; minimum: -{n}")
            start += n
        if length < 0:
            raise RubyError("IndexError", f"negative length ({length})")
        _splice(v, start, length, args[2])
        return args[2]
    idx, val = args
    if type(idx) is RRange:
        first = 0 if idx.first is None else to_int(idx.first)
        if first < 0:
            if first + n < 0:
                raise RubyError("RangeError", f"{inspect(idx)} out of range")
            first += n
        if idx.last is None:
            end = n
        else:
            end = to_int(idx.last)
            if end < 0:
                end += n
            if not idx.excl:
                end += 1
        length = max(0, end - first)
        _splice(v, first, length, val)
        return val
    i = to_int(idx)
    if i < 0:
        if i + n < 0:
            raise RubyError("IndexError", f"index {i} too small for array; minimum: -{n}")
        i += n
    if i >= n:
        _check_ary_size(i + 1)
        v.extend([None] * (i - n + 1))
    v[i] = val
    return val


def _ary_arg(v: Value) -> list:
    if type(v) is not list:
        raise no_implicit(v, "Array")
    return v


@_a("+")
def a_plus(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 1)
    o = _ary_arg(args[0])
    _check_ary_size(len(v) + len(o))
    return v + o


@_a("-")
def a_minus(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 1)
    keys = {hash_key(x) for x in _ary_arg(args[0])}
    return [x for x in v if hash_key(x) not in keys]


@_a("*")
def a_mul(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 1)
    a = args[0]
    if isinstance(a, str):
        return ary_join(v, str(a), set())
    n = to_int(a)
    if n < 0:
        raise RubyError("ArgumentError", "negative argument")
    _check_ary_size(len(v) * n)
    return v * n


@_a("&")
def a_and(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 1)
    keys = {hash_key(x) for x in _ary_arg(args[0])}
    seen = set()
    out = []
    for x in v:
        k = hash_key(x)
        if k in keys and k not in seen:
            seen.add(k)
            out.append(x)
    return out


@_a("|")
def a_or(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 1)
    seen = set()
    out = []
    for x in v + _ary_arg(args[0]):
        k = hash_key(x)
        if k not in seen:
            seen.add(k)
            out.append(x)
    return out


@_a("<<")
def a_push1(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 1)
    _check_ary_size(len(v) + 1)
    v.append(args[0])
    return v


@_a("push", "append")
def a_push(it: Any, v: list, args: list, blk: Any) -> Value:
    _check_ary_size(len(v) + len(args))
    v.extend(args)
    return v


@_a("unshift", "prepend")
def a_unshift(it: Any, v: list, args: list, blk: Any) -> Value:
    _check_ary_size(len(v) + len(args))
    v[0:0] = args
    return v


@_a("insert")
def a_insert(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 1, -1)
    i = to_int(args[0])
    if len(args) == 1:
        return v
    n = len(v)
    if i < 0:
        i += n + 1
        if i < 0:
            raise RubyError("IndexError", f"index {i - n - 1} too small for array; minimum: -{n + 1}")
    if i > n:
        _check_ary_size(i)
        v.extend([None] * (i - n))
    v[i:i] = args[1:]
    return v


@_a("pop")
def a_pop(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    if args:
        n = to_int(args[0])
        if n < 0:
            raise RubyError("ArgumentError", "negative array size")
        if n == 0:
            return []
        out = v[-n:]
        del v[-n:]
        return out
    return v.pop() if v else None


@_a("shift")
def a_shift(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    if args:
        n = to_int(args[0])
        if n < 0:
            raise RubyError("ArgumentError", "negative array size")
        out = v[:n]
        del v[:n]
        return out
    return v.pop(0) if v else None


@_a("concat")
def a_concat(it: Any, v: list, args: list, blk: Any) -> Value:
    parts = [list(_ary_arg(a)) for a in args]
    _check_ary_size(len(v) + sum(len(x) for x in parts))
    for p in parts:
        v.extend(p)
    return v


@_a("delete")
def a_delete(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 1)
    target = args[0]
    found = None
    hit = False
    keep = []
    for x in v:
        if req(x, target):
            found = x
            hit = True
        else:
            keep.append(x)
    if not hit:
        return it.yield_(blk, target) if blk is not None else None
    v[:] = keep
    return found


@_a("delete_at")
def a_delete_at(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 1)
    i = to_int(args[0])
    n = len(v)
    if i < 0:
        i += n
    if i < 0 or i >= n:
        return None
    return v.pop(i)


@_a("delete_if")
def a_delete_if(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "delete_if", args, blk)
    if e is not None:
        return e
    keep = [x for x in list(v) if not truthy(it.yield_(blk, x))]
    v[:] = keep
    return v


@_a("clear")
def a_clear(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0)
    v.clear()
    return v


@_a("fill")
def a_fill(it: Any, v: list, args: list, blk: Any) -> Value:
    if blk is not None:
        argc(args, 0)
        for i in range(len(v)):
            v[i] = it.yield_(blk, i)
        return v
    argc(args, 1)
    for i in range(len(v)):
        v[i] = args[0]
    return v


@_a("index", "find_index")
def a_index_of(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    if not args and blk is None:
        return REnumerator(v, "index", [])
    for i, x in enumerate(list(v)):
        if req(x, args[0]) if args else truthy(it.yield_(blk, x)):
            return i
    return None


@_a("rindex")
def a_rindex(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    if not args and blk is None:
        return REnumerator(v, "rindex", [])
    for i in range(len(v) - 1, -1, -1):
        x = v[i]
        if req(x, args[0]) if args else truthy(it.yield_(blk, x)):
            return i
    return None


def ary_join(v: list, sep: str, seen: set[int]) -> str:
    if id(v) in seen:
        raise RubyError("ArgumentError", "recursive array join")
    seen.add(id(v))
    parts = []
    for x in v:
        if type(x) is list:
            parts.append(ary_join(x, sep, seen))
        else:
            parts.append(to_s(x))
    seen.discard(id(v))
    return sep.join(parts)


@_a("join")
def a_join(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    sep = "" if not args or args[0] is None else str(to_str(args[0]))
    return ary_join(v, sep, set())


@_a("reverse")
def a_reverse(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v[::-1]


@_a("rotate")
def a_rotate(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    n = to_int(args[0]) if args else 1
    if not v:
        return []
    n %= len(v)
    return v[n:] + v[:n]


def _flatten(v: list, depth: int | None, path: set[int]) -> list:
    out: list = []
    if id(v) in path:
        raise RubyError("ArgumentError", "tried to flatten recursive array")
    path.add(id(v))
    for x in v:
        if type(x) is list and (depth is None or depth > 0):
            out.extend(_flatten(x, None if depth is None else depth - 1, path))
        else:
            out.append(x)
    path.discard(id(v))
    return out


@_a("flatten")
def a_flatten(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    depth = None if not args or args[0] is None else to_int(args[0])
    if depth is not None and depth < 0:
        depth = None
    return _flatten(v, depth, set())


@_a("compact")
def a_compact(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0)
    return [x for x in v if x is not None]


@_a("transpose")
def a_transpose(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0)
    if not v:
        return []
    rows = [_ary_arg(r) for r in v]
    w = len(rows[0])
    for r in rows:
        if len(r) != w:
            raise RubyError("IndexError", f"element size differs ({len(r)} should be {w})")
    return [[r[i] for r in rows] for i in range(w)]


@_a("values_at")
def a_values_at(it: Any, v: list, args: list, blk: Any) -> Value:
    out: list = []
    n = len(v)
    for a in args:
        if type(a) is RRange:
            span = range_span(a, n)
            if span is None:
                continue
            start, _ = span
            end = n if a.last is None else to_int(a.last)
            if a.last is not None:
                if end < 0:
                    end += n
                if not a.excl:
                    end += 1
            for i in range(start, end):
                out.append(v[i] if i < n else None)
            continue
        i = to_int(a)
        if i < 0:
            i += n
        out.append(v[i] if 0 <= i < n else None)
    return out


def dig_impl(it: Any, v: Value, args: list) -> Value:
    argc(args, 1, -1)
    cur = it.call_method(v, "[]", [args[0]], None)
    rest = args[1:]
    if not rest or cur is None:
        return cur
    if find_method(cur, "dig") is None or type(cur) not in (list, RHash):
        raise RubyError("TypeError", f"{class_name(cur)} does not have #dig method")
    return dig_impl(it, cur, rest)


@_a("dig")
def a_dig(it: Any, v: list, args: list, blk: Any) -> Value:
    return dig_impl(it, v, args)


@_a("assoc")
def a_assoc(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 1)
    for x in v:
        if type(x) is list and x and req(x[0], args[0]):
            return x
    return None


@_a("product")
def a_product(it: Any, v: list, args: list, blk: Any) -> Value:
    others = [_ary_arg(a) for a in args]
    total = len(v)
    for o in others:
        total *= len(o)
    _check_ary_size(total)
    combos = [list(c) for c in itertools.product(v, *others)]
    if blk is not None:
        for c in combos:
            it.yield_(blk, c)
        return v
    return combos


@_a("combination")
def a_combination(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 1)
    n = to_int(args[0])
    if n < 0 or n > len(v):
        combos: list = []
    else:
        _check_ary_size(math.comb(len(v), n))
        combos = [list(c) for c in itertools.combinations(v, n)]
    if blk is not None:
        for c in combos:
            it.yield_(blk, c)
        return v
    return combos


@_a("permutation")
def a_permutation(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    n = to_int(args[0]) if args else len(v)
    if n < 0 or n > len(v):
        perms: list = []
    else:
        _check_ary_size(math.perm(len(v), n))
        perms = [list(c) for c in itertools.permutations(v, n)]
    if blk is not None:
        for c in perms:
            it.yield_(blk, c)
        return v
    return perms


@_a("to_a", "entries")
def a_to_a(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v


@_a("each")
def a_each(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "each", args, blk)
    if e is not None:
        return e
    i = 0
    while i < len(v):
        it.yield_void(blk, v[i])
        i += 1
    return v


@_a("each_index")
def a_each_index(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "each_index", args, blk)
    if e is not None:
        return e
    i = 0
    while i < len(v):
        it.yield_void(blk, i)
        i += 1
    return v


@_a("length", "size")
def a_length(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0)
    return len(v)


@_a("empty?")
def a_empty(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0)
    return not v


@_a("first")
def a_first(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    if not args:
        return v[0] if v else None
    n = to_int(args[0])
    if n < 0:
        raise RubyError("ArgumentError", "negative array size")
    return v[:n]


@_a("last")
def a_last(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    if not args:
        return v[-1] if v else None
    n = to_int(args[0])
    if n < 0:
        raise RubyError("ArgumentError", "negative array size")
    return v[len(v) - n :] if n else []


@_a("include?")
def a_include(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 1)
    return any(req(x, args[0]) for x in v)


@_a("sum")
def a_sum(it: Any, v: list, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    init = args[0] if args else 0
    return ruby_sum(it, iter(list(v)), init, blk, False)


# ---------------------------------------------------------------------------------------
# Hash

H = HASH


def _h(*names: str):  # noqa: ANN202
    return method(H, *names)


@_h("[]")
def h_get(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 1)
    e = v.store.get(hash_key(args[0]))
    return v.default if e is None else e[1]


@_h("[]=", "store")
def h_set(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 2)
    v.set(args[0], args[1])
    return args[1]


def _key_error(k: Value) -> RubyError:
    return RubyError("KeyError", f"key not found: {inspect(k)}")


@_h("fetch")
def h_fetch(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 1, 2)
    e = v.store.get(hash_key(args[0]))
    if e is not None:
        return e[1]
    if blk is not None:
        return it.yield_(blk, args[0])
    if len(args) > 1:
        return args[1]
    raise _key_error(args[0])


@_h("dig")
def h_dig(it: Any, v: RHash, args: list, blk: Any) -> Value:
    return dig_impl(it, v, args)


@_h("key?", "has_key?", "include?", "member?")
def h_has_key(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 1)
    return v.has(args[0])


@_h("value?", "has_value?")
def h_has_value(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 1)
    return any(req(x, args[0]) for x in v.values())


@_h("key")
def h_key(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 1)
    for k, x in v.items():
        if req(x, args[0]):
            return k
    return None


@_h("keys")
def h_keys(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v.keys()


@_h("values")
def h_values(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v.values()


@_h("values_at")
def h_values_at(it: Any, v: RHash, args: list, blk: Any) -> Value:
    return [v.get(k, v.default) for k in args]


@_h("fetch_values")
def h_fetch_values(it: Any, v: RHash, args: list, blk: Any) -> Value:
    out = []
    for k in args:
        e = v.store.get(hash_key(k))
        if e is not None:
            out.append(e[1])
        elif blk is not None:
            out.append(it.yield_(blk, k))
        else:
            raise _key_error(k)
    return out


@_h("length", "size")
def h_length(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    return len(v)


@_h("empty?")
def h_empty(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    return len(v) == 0


@_h("delete")
def h_delete(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 1)
    found, val = v.delete(args[0])
    if not found and blk is not None:
        return it.yield_(blk, args[0])
    return val


@_h("delete_if")
def h_delete_if(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "delete_if", args, blk)
    if e is not None:
        return e
    for k, x in v.items():
        if truthy(it.yield_(blk, k, x)):
            v.delete(k)
    return v


@_h("clear")
def h_clear(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    v.store.clear()
    return v


def _hash_arg(v: Value) -> RHash:
    if type(v) is not RHash:
        raise no_implicit(v, "Hash")
    return v


def _merge_into(it: Any, target: RHash, others: list, blk: Any) -> RHash:
    for o in others:
        for k, x in _hash_arg(o).items():
            if blk is not None and target.has(k):
                x = it.yield_(blk, k, target.get(k), x)
            target.set(k, x)
    return target


@_h("merge")
def h_merge(it: Any, v: RHash, args: list, blk: Any) -> Value:
    return _merge_into(it, v.copy(), args, blk)


@_h("merge!", "update")
def h_merge_bang(it: Any, v: RHash, args: list, blk: Any) -> Value:
    return _merge_into(it, v, args, blk)


@_h("to_a", "entries")
def h_to_a(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    return [[k, x] for k, x in v.items()]


@_h("to_h")
def h_to_h(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    if blk is None:
        return v
    return pairs_to_hash(it, ([k, x] for k, x in v.items()), blk)


@_h("invert")
def h_invert(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    h = RHash()
    for k, x in v.items():
        h.set(x, k)
    return h


@_h("transform_values")
def h_transform_values(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "transform_values", args, blk)
    if e is not None:
        return e
    h = RHash()
    for k, x in v.items():
        h.set(k, it.yield_(blk, x))
    return h


@_h("transform_keys")
def h_transform_keys(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    mapping = _hash_arg(args[0]) if args else None
    if mapping is None and blk is None:
        return REnumerator(v, "transform_keys", [])
    h = RHash()
    for k, x in v.items():
        if mapping is not None and mapping.has(k):
            nk = mapping.get(k)
        elif blk is not None:
            nk = it.yield_(blk, k)
        else:
            nk = k
        h.set(nk, x)
    return h


@_h("select", "filter")
def h_select(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "select", args, blk)
    if e is not None:
        return e
    h = RHash()
    for k, x in v.items():
        if truthy(it.yield_(blk, k, x)):
            h.set(k, x)
    return h


@_h("reject")
def h_reject(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "reject", args, blk)
    if e is not None:
        return e
    h = RHash()
    for k, x in v.items():
        if not truthy(it.yield_(blk, k, x)):
            h.set(k, x)
    return h


@_h("each", "each_pair")
def h_each(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "each", args, blk)
    if e is not None:
        return e
    for k, x in v.items():
        it.yield_void(blk, [k, x])
    return v


@_h("each_key")
def h_each_key(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "each_key", args, blk)
    if e is not None:
        return e
    for k in v.keys():
        it.yield_void(blk, k)
    return v


@_h("each_value")
def h_each_value(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, v, "each_value", args, blk)
    if e is not None:
        return e
    for x in v.values():
        it.yield_void(blk, x)
    return v


@_h("slice")
def h_slice(it: Any, v: RHash, args: list, blk: Any) -> Value:
    h = RHash()
    for k in args:
        e = v.store.get(hash_key(k))
        if e is not None:
            h.set(e[0], e[1])
    return h


@_h("except")
def h_except(it: Any, v: RHash, args: list, blk: Any) -> Value:
    h = v.copy()
    h.default = None
    for k in args:
        h.delete(k)
    return h


@_h("compact")
def h_compact(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0)
    h = RHash()
    for k, x in v.items():
        if x is not None:
            h.set(k, x)
    return h


@_h("default")
def h_default(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    return v.default


@_h("default=")
def h_set_default(it: Any, v: RHash, args: list, blk: Any) -> Value:
    argc(args, 1)
    v.default = args[0]
    return args[0]


# ---------------------------------------------------------------------------------------
# Range

R = RANGE


def _r(*names: str):  # noqa: ANN202
    return method(R, *names)


def _int_range(r: RRange) -> bool:
    return type(r.first) is int and (type(r.last) is int or r.last is None)


@_r("first")
def r_first(it: Any, r: RRange, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    if not args:
        if r.first is None:
            raise RubyError("RangeError", "cannot get the first element of beginless range")
        return r.first
    return en_first(it, r, args, blk)


@_r("begin")
def r_begin(it: Any, r: RRange, args: list, blk: Any) -> Value:
    argc(args, 0)
    return r.first


@_r("end")
def r_end(it: Any, r: RRange, args: list, blk: Any) -> Value:
    argc(args, 0)
    return r.last


@_r("last")
def r_last(it: Any, r: RRange, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    if not args:
        if r.last is None:
            raise RubyError("RangeError", "cannot get the last element of endless range")
        return r.last
    n = to_int(args[0])
    if n < 0:
        raise RubyError("ArgumentError", "negative array size")
    items = to_list(it, r)
    return items[len(items) - n :] if n else []


@_r("min")
def r_min(it: Any, r: RRange, args: list, blk: Any) -> Value:
    if blk is not None or args:
        return en_min(it, r, args, blk)
    if r.last is None:
        return r.first
    c = cmp_or_fail(r.first, r.last)
    if c > 0 or (c == 0 and r.excl):
        return None
    return r.first


@_r("max")
def r_max(it: Any, r: RRange, args: list, blk: Any) -> Value:
    if blk is not None or args:
        return en_max(it, r, args, blk)
    if r.last is None:
        raise RubyError("RangeError", "cannot get the maximum of endless range")
    if r.excl:
        if type(r.last) is not int:
            raise RubyError("TypeError", "cannot exclude non Integer end value")
        if type(r.first) is not int and r.first is not None:
            raise RubyError("TypeError", "cannot exclude end value with non Integer begin value")
        c = cmp_or_fail(r.first, r.last)
        if c >= 0:
            return None
        return r.last - 1
    c = cmp_or_fail(r.first, r.last)
    if c > 0:
        return None
    return r.last


@_r("minmax")
def r_minmax(it: Any, r: RRange, args: list, blk: Any) -> Value:
    if blk is not None:
        return en_minmax(it, r, args, blk)
    return [r_min(it, r, [], None), r_max(it, r, [], None)]


def _range_size(r: RRange) -> Value:
    if type(r.first) is not int:
        if type(r.first) is float:
            raise RubyError("TypeError", f"can't iterate from {class_name(r.first)}")
        return None
    if r.last is None:
        return math.inf
    if type(r.last) not in (int, float):
        return None
    start, end = range_int_bounds(r)
    if end is None:
        return math.inf
    return max(0, end - start + 1)


@_r("size")
def r_size(it: Any, r: RRange, args: list, blk: Any) -> Value:
    argc(args, 0)
    return _range_size(r)


@_r("count")
def r_count(it: Any, r: RRange, args: list, blk: Any) -> Value:
    if args or blk is not None:
        return en_count(it, r, args, blk)
    if _int_range(r):
        return _range_size(r)
    return en_count(it, r, args, blk)


@_r("sum")
def r_sum(it: Any, r: RRange, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    init = args[0] if args else 0
    if blk is None and type(r.first) is int and type(r.last) is int and type(init) is int:
        a = r.first
        b = r.last - 1 if r.excl else r.last
        if b < a:
            return init
        return check_int((b - a + 1) * (a + b) // 2 + init)
    return ruby_sum(it, each_items(it, r), init, blk, True)


@_r("each")
def r_each(it: Any, r: RRange, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, r, "each", args, blk)
    if e is not None:
        return e
    for x in range_iter(r):
        it.step()
        it.yield_void(blk, x)
    return r


@_r("reverse_each")
def r_reverse_each(it: Any, r: RRange, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = enum_or(it, r, "reverse_each", args, blk)
    if e is not None:
        return e
    if type(r.last) is int and (type(r.first) is int or r.first is None):
        end = r.last - 1 if r.excl else r.last
        i = end
        while r.first is None or i >= r.first:
            it.step()
            it.yield_void(blk, i)
            i -= 1
        return r
    if r.last is None:
        raise RubyError("TypeError", f"can't iterate from {class_name(r.last)}")
    for x in reversed(to_list(it, r)):
        it.yield_void(blk, x)
    return r


@_r("to_a", "entries")
def r_to_a(it: Any, r: RRange, args: list, blk: Any) -> Value:
    argc(args, 0)
    if r.last is None:
        raise RubyError("RangeError", "cannot convert endless range to an array")
    return to_list(it, r)


@_r("step")
def r_step(it: Any, r: RRange, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    stp = args[0] if args else 1
    if type(stp) not in (int, float):
        raise RubyError("TypeError", f"no implicit conversion of {class_name(stp)} into Integer")
    if stp == 0:
        raise RubyError("ArgumentError", "step can't be 0")
    if stp < 0:
        raise RubyError("ArgumentError", "step can't be negative")
    if blk is None:
        return REnumerator(r, "step", list(args))
    if type(r.first) is int and type(stp) is int and (type(r.last) is int or r.last is None):
        i = r.first
        while r.last is None or (i < r.last if r.excl else i <= r.last):
            it.step()
            it.yield_void(blk, i)
            i += stp
        return r
    if type(r.first) in (int, float) and type(r.last) in (int, float):
        float_step(it, float(r.first), float(r.last), float(stp), r.excl, blk)
        return r
    raise RubyError("TypeError", f"can't iterate from {class_name(r.first)}")


@_r("include?", "member?", "cover?", "===")
def r_include(it: Any, r: RRange, args: list, blk: Any) -> Value:
    argc(args, 1)
    x = args[0]
    if type(x) is RRange:
        if x.first is None or x.last is None or not range_cover(r, x.first):
            return False
        if range_cover(r, x.last):
            return True
        if x.excl and r.last is not None:
            c = cmp(x.last, r.last)
            return c is not None and c <= 0
        return False
    return range_cover(r, x)


@_r("exclude_end?")
def r_exclude_end(it: Any, r: RRange, args: list, blk: Any) -> Value:
    argc(args, 0)
    return r.excl
