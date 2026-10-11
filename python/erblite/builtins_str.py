"""String methods (spec §8.5)."""

from __future__ import annotations

from typing import Any

from .builtins import (
    STRING,
    argc,
    method,
    no_implicit,
    opt_block_enum,
    to_int,
    to_str,
)
from .errors import RubyError
from .values import INT_MAX, INT_MIN, RHash, RRange, Symbol, Value, class_name, to_s

WS_STRIP = " \t\n\v\f\r\0"
WS_SPLIT = " \t\n\v\f\r"
MAX_STR = 10_000_000  # [ERBLite] size limit (spec §14)


def _check_size(n: int) -> None:
    if n > MAX_STR:
        raise RubyError("LimitError", f"string larger than {MAX_STR} characters")


def str_shovel(base: str, val: Value) -> str:
    if isinstance(val, str):
        _check_size(len(base) + len(val))
        return str(base) + str(val)
    if type(val) is int:
        if val < 0 or val > 0x10FFFF:
            raise RubyError("RangeError", f"{val} out of char range")
        return str(base) + chr(val)
    raise no_implicit(val, "String")


# ---------------------------------------------------------------------------------------
# Number parsing


def _check_range(v: int, text: str) -> int:
    if v < INT_MIN or v > INT_MAX:
        raise RubyError("RangeError", f"integer {text} out of 64-bit range")
    return v


_DIGITS = "0123456789abcdefghijklmnopqrstuvwxyz"


def _digit_val(c: str) -> int:
    o = c.lower()
    if len(o) == 1 and o in _DIGITS:
        return _DIGITS.index(o)
    return 99


def _scan_digits(s: str, i: int, base: int, strict: bool) -> tuple[str, int, bool]:
    """Reads digits with single '_' separators. Returns (digits, new index, ok)."""
    out = []
    n = len(s)
    ok = True
    while i < n:
        c = s[i]
        if _digit_val(c) < base:
            out.append(c)
            i += 1
        elif c == "_" and out and i + 1 < n and _digit_val(s[i + 1]) < base:
            i += 1
        else:
            if c == "_" and strict:
                ok = False
            break
    return "".join(out), i, ok


_PREFIX_BASE = {"x": 16, "b": 2, "o": 8, "d": 10}


def str_to_i(s: str, base: int) -> int:
    """Lenient String#to_i. ``base`` < 0 means "default -base, but honor a prefix" (oct)."""
    if base == 1 or base > 36 or base < -36:
        raise RubyError("ArgumentError", f"invalid radix {base}")
    i = 0
    n = len(s)
    while i < n and s[i] in WS_SPLIT:
        i += 1
    neg = False
    if i < n and s[i] in "+-":
        neg = s[i] == "-"
        i += 1
    auto = base < 0
    b = -base if auto else base
    if base == 0:
        auto, b = True, 10
    if i + 1 < n and s[i] == "0" and s[i + 1].lower() in _PREFIX_BASE:
        pb = _PREFIX_BASE[s[i + 1].lower()]
        if auto or pb == b:
            if i + 2 < n and _digit_val(s[i + 2]) < pb:
                b = pb
                i += 2
            elif not auto:
                return 0
    elif auto and i + 1 < n and s[i] == "0" and base == 0:
        b = 8
    digits, _, _ = _scan_digits(s, i, b, False)
    if not digits:
        return 0
    v = int(digits, b)
    return _check_range(-v if neg else v, s)


def str_to_f(s: str) -> float:
    i = 0
    n = len(s)
    while i < n and s[i] in WS_SPLIT:
        i += 1
    text = ""
    if i < n and s[i] in "+-":
        text += s[i]
        i += 1
    ip, i, _ = _scan_digits(s, i, 10, False)
    text += ip
    fp = ""
    if i + 1 < n and s[i] == "." and s[i + 1].isdigit() and s[i + 1].isascii():
        fp, i, _ = _scan_digits(s, i + 1, 10, False)
        text += "." + fp
    if not ip and not fp:
        return 0.0
    if i < n and s[i] in "eE":
        j = i + 1
        sign = ""
        if j < n and s[j] in "+-":
            sign = s[j]
            j += 1
        ep, j, _ = _scan_digits(s, j, 10, False)
        if ep:
            text += "e" + sign + ep
    try:
        return float(text)
    except ValueError:
        return 0.0


def parse_integer_strict(s: str, base: int | None) -> int:
    def bad() -> RubyError:
        from .values import inspect_str

        return RubyError("ArgumentError", f"invalid value for Integer(): {inspect_str(s)}")

    if base is not None and (base == 1 or base > 36 or base < 0):
        raise RubyError("ArgumentError", f"invalid radix {base}")
    if "\0" in s:
        raise RubyError("ArgumentError", "string contains null byte")
    t = s.strip(WS_SPLIT)
    i = 0
    n = len(t)
    neg = False
    if i < n and t[i] in "+-":
        neg = t[i] == "-"
        i += 1
    b = 10 if not base else base
    if i + 1 < n and t[i] == "0" and t[i + 1].lower() in _PREFIX_BASE:
        pb = _PREFIX_BASE[t[i + 1].lower()]
        if not base or pb == base:
            b = pb
            i += 2
    elif not base and i + 1 < n and t[i] == "0":
        b = 8
        i += 1
        if t[i] == "_":
            i += 1
    digits, j, ok = _scan_digits(t, i, b, True)
    if not ok or j != n or not digits:
        raise bad()
    v = int(digits, b)
    return _check_range(-v if neg else v, s)


def parse_float_strict(s: str) -> float:
    from .values import inspect_str

    def bad() -> RubyError:
        return RubyError("ArgumentError", f"invalid value for Float(): {inspect_str(s)}")

    if "\0" in s:
        raise RubyError("ArgumentError", "string contains null byte")
    t = s.strip(WS_SPLIT)
    n = len(t)
    i = 0
    sign = ""
    if i < n and t[i] in "+-":
        sign = t[i]
        i += 1
    if i + 1 < n and t[i] == "0" and t[i + 1] in "xX":
        digits, j, ok = _scan_digits(t, i + 2, 16, True)
        if not digits or not ok or j != n:
            raise bad()
        return float(int(sign + digits, 16))
    ip, i, ok = _scan_digits(t, i, 10, True)
    if not ok:
        raise bad()
    fp = ""
    if i < n and t[i] == ".":
        fp, i, ok = _scan_digits(t, i + 1, 10, True)
        if not ok or (not fp and not ip):
            raise bad()
    if not ip and not fp:
        raise bad()
    text = sign + (ip or "0") + "." + (fp or "0")
    if i < n and t[i] in "eE":
        i += 1
        es = ""
        if i < n and t[i] in "+-":
            es = t[i]
            i += 1
        ep, i, ok = _scan_digits(t, i, 10, True)
        if not ep or not ok:
            raise bad()
        text += "e" + es + ep
    if i != n:
        raise bad()
    return float(text)


# ---------------------------------------------------------------------------------------
# Character sets (tr / delete / squeeze)


class CharSet:
    def __init__(self, spec: str) -> None:
        self.negate = len(spec) > 1 and spec[0] == "^"
        if self.negate:
            spec = spec[1:]
        chars: list[str] = []
        i = 0
        n = len(spec)
        while i < n:
            c = spec[i]
            if c == "\\" and i + 1 < n:
                chars.append(spec[i + 1])
                i += 2
                continue
            if i + 2 < n and spec[i + 1] == "-":
                hi = spec[i + 2]
                if ord(c) > ord(hi):
                    raise RubyError("ArgumentError", f'invalid range "{c}-{hi}" in string transliteration')
                chars.extend(chr(x) for x in range(ord(c), ord(hi) + 1))
                i += 3
                continue
            chars.append(c)
            i += 1
        self.chars = chars
        self.set = frozenset(chars)

    def __contains__(self, c: str) -> bool:
        return (c in self.set) != self.negate


def str_tr(s: str, frm: str, to: str) -> str:
    fs = CharSet(frm)
    if to == "":
        return "".join(c for c in s if c not in fs)
    ts = CharSet(to) if not (len(to) > 1 and to[0] == "^") else CharSet("\\" + to)
    tchars = ts.chars
    if not tchars:
        return "".join(c for c in s if c not in fs)
    if fs.negate:
        last = tchars[-1]
        return "".join(last if c in fs else c for c in s)
    mapping: dict[str, str] = {}
    for idx, c in enumerate(fs.chars):
        if c not in mapping:
            mapping[c] = tchars[idx] if idx < len(tchars) else tchars[-1]
    return "".join(mapping.get(c, c) for c in s)


# ---------------------------------------------------------------------------------------
# Indexing helpers (shared with Array)


def range_span(r: RRange, n: int) -> tuple[int, int] | None:
    """Converts a Range into (start, length) for a sequence of length n, or None."""
    first = 0 if r.first is None else to_int(r.first)
    if first < 0:
        first += n
        if first < 0:
            return None
    if first > n:
        return None
    if r.last is None:
        end = n
    else:
        end = to_int(r.last)
        if end < 0:
            end += n
        if not r.excl:
            end += 1
    length = end - first
    if length < 0:
        length = 0
    if first + length > n:
        length = n - first
    return first, length


def seq_index(seq: Any, args: list, empty: Any) -> Value:
    """Ruby ``[]`` for String and Array. ``empty`` is the empty value of the type."""
    argc(args, 1, 2)
    n = len(seq)
    if len(args) == 2:
        start = to_int(args[0])
        length = to_int(args[1])
        if start < 0:
            start += n
            if start < 0:
                return None
        if start > n or length < 0:
            return None
        return seq[start : start + length]
    a = args[0]
    if type(a) is RRange:
        span = range_span(a, n)
        if span is None:
            return None
        return seq[span[0] : span[0] + span[1]]
    i = to_int(a)
    if i < 0:
        i += n
    if i < 0 or i >= n:
        return None
    return seq[i]


# ---------------------------------------------------------------------------------------
# Methods

S = STRING


def _m(*names: str):  # noqa: ANN202
    return method(S, *names)


@_m("+")
def s_plus(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 1)
    o = to_str(args[0])
    _check_size(len(v) + len(o))
    return str(v) + str(o)


@_m("*")
def s_mul(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 1)
    n = to_int(args[0])
    if n < 0:
        raise RubyError("ArgumentError", "negative argument")
    _check_size(len(v) * n)
    return str(v) * n


@_m("%")
def s_format(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 1)
    from .formatting import format_str

    a = args[0]
    return format_str(v, list(a) if type(a) is list else [a])


@_m("<<")
def s_shovel(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 1)
    return str_shovel(v, args[0])


@_m("[]", "slice")
def s_index(it: Any, v: str, args: list, blk: Any) -> Value:
    if len(args) == 1 and isinstance(args[0], str):
        return str(args[0]) if args[0] in v else None
    r = seq_index(v, args, "")
    return None if r is None else str(r)


@_m("length", "size")
def s_length(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return len(v)


@_m("bytesize")
def s_bytesize(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return len(v.encode("utf-8", "surrogatepass"))


@_m("empty?")
def s_empty(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return len(v) == 0


@_m("upcase")
def s_upcase(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v.upper()


def _lower(s: str) -> str:
    # Ruby has no context-sensitive final sigma: "ΣΑΣ".downcase is "σασ" (Python gives "σας").
    return s.replace("\u03a3", "\u03c3").lower()


@_m("downcase")
def s_downcase(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return _lower(v)


@_m("capitalize")
def s_capitalize(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v[:1].capitalize() + _lower(v[1:])


@_m("swapcase")
def s_swapcase(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return "".join("\u03c3" if c == "\u03a3" else c.swapcase() for c in v)


@_m("strip")
def s_strip(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v.strip(WS_STRIP)


@_m("lstrip")
def s_lstrip(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v.lstrip(WS_STRIP)


@_m("rstrip")
def s_rstrip(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v.rstrip(WS_STRIP)


@_m("chomp")
def s_chomp(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    v = str(v)
    if args:
        suf = args[0]
        if suf is None:
            return v
        suf = to_str(suf)
        if suf == "":
            while v.endswith("\n"):
                v = v[:-2] if v.endswith("\r\n") else v[:-1]
            return v
        if suf != "\n":
            return v[: -len(suf)] if v.endswith(suf) else v
    if v.endswith("\r\n"):
        return v[:-2]
    if v.endswith("\n") or v.endswith("\r"):
        return v[:-1]
    return v


@_m("chop")
def s_chop(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    if v.endswith("\r\n"):
        return str(v[:-2])
    return str(v[:-1])


@_m("chars")
def s_chars(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return list(str(v))


def _lines(v: str, sep: str = "\n") -> list[str]:
    out = []
    pos = 0
    n = len(v)
    while pos < n:
        j = v.find(sep, pos)
        if j < 0:
            out.append(v[pos:])
            break
        out.append(v[pos : j + len(sep)])
        pos = j + len(sep)
    return out


@_m("lines")
def s_lines(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    sep = to_str(args[0]) if args else "\n"
    if sep == "":
        sep = "\n"
    return _lines(str(v), sep)


@_m("each_line")
def s_each_line(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    e = opt_block_enum(it, v, "each_line", args, blk)
    if e is not None:
        return e
    sep = to_str(args[0]) if args else "\n"
    for line in _lines(str(v), sep or "\n"):
        it.yield_void(blk, line)
    return v


@_m("each_char")
def s_each_char(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    e = opt_block_enum(it, v, "each_char", args, blk)
    if e is not None:
        return e
    for c in str(v):
        it.yield_void(blk, c)
    return v


@_m("bytes")
def s_bytes(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return list(v.encode("utf-8", "surrogatepass"))


@_m("reverse")
def s_reverse(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return str(v[::-1])


@_m("include?")
def s_include(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 1)
    return to_str(args[0]) in v


@_m("start_with?")
def s_start_with(it: Any, v: str, args: list, blk: Any) -> Value:
    return any(v.startswith(to_str(a)) for a in args)


@_m("end_with?")
def s_end_with(it: Any, v: str, args: list, blk: Any) -> Value:
    return any(v.endswith(to_str(a)) for a in args)


@_m("index")
def s_find(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 1, 2)
    sub = to_str(args[0])
    n = len(v)
    start = to_int(args[1]) if len(args) > 1 else 0
    if start < 0:
        start += n
        if start < 0:
            return None
    if start > n:
        return None
    r = v.find(sub, start)
    return None if r < 0 else r


@_m("rindex")
def s_rindex(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 1, 2)
    sub = to_str(args[0])
    n = len(v)
    pos = to_int(args[1]) if len(args) > 1 else n
    if pos < 0:
        pos += n
        if pos < 0:
            return None
    if pos > n:
        pos = n
    r = v.rfind(sub, 0, pos + len(sub))
    return None if r < 0 else r


@_m("count")
def s_count(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 1)
    sub = to_str(args[0])
    if sub == "":
        return 0
    return v.count(sub)


def _expand_replacement(rep: str, match: str, pre: str, post: str) -> str:
    out = []
    i = 0
    n = len(rep)
    while i < n:
        c = rep[i]
        if c == "\\" and i + 1 < n:
            d = rep[i + 1]
            if d == "0" or d == "&":
                out.append(match)
                i += 2
                continue
            if d == "`":
                out.append(pre)
                i += 2
                continue
            if d == "'":
                out.append(post)
                i += 2
                continue
            if d == "\\":
                out.append("\\")
                i += 2
                continue
            if d in "123456789":
                i += 2
                continue
        out.append(c)
        i += 1
    return "".join(out)


def _sub_impl(it: Any, v: str, args: list, blk: Any, global_: bool) -> str:
    argc(args, 1 if blk is not None else 2, 2)
    pat = args[0]
    if not isinstance(pat, str):
        raise RubyError("TypeError", f"wrong argument type {class_name(pat)} (expected Regexp)")
    v = str(v)
    rep = args[1] if len(args) > 1 else None
    if rep is not None and type(rep) is not RHash:
        rep = to_str(rep)
    out = []
    pos = 0
    n = len(v)
    plen = len(pat)
    while pos <= n:
        idx = v.find(pat, pos)
        if idx < 0:
            break
        out.append(v[pos:idx])
        if type(rep) is RHash:
            out.append(to_s(rep.get(pat, rep.default)))
        elif rep is not None:
            out.append(_expand_replacement(rep, pat, v[:idx], v[idx + plen :]))
        else:
            out.append(to_s(it.yield_(blk, str(pat))))
        if plen == 0:
            if idx < n:
                out.append(v[idx])
            pos = idx + 1
        else:
            pos = idx + plen
        if not global_:
            break
    if pos <= n:
        out.append(v[pos:])
    res = "".join(out)
    _check_size(len(res))
    return res


@_m("sub")
def s_sub(it: Any, v: str, args: list, blk: Any) -> Value:
    return _sub_impl(it, v, args, blk, False)


@_m("gsub")
def s_gsub(it: Any, v: str, args: list, blk: Any) -> Value:
    return _sub_impl(it, v, args, blk, True)


@_m("tr")
def s_tr(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 2)
    return str_tr(str(v), to_str(args[0]), to_str(args[1]))


@_m("delete")
def s_delete(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 1, -1)
    sets = [CharSet(to_str(a)) for a in args]
    return "".join(c for c in v if not all(c in s for s in sets))


@_m("squeeze")
def s_squeeze(it: Any, v: str, args: list, blk: Any) -> Value:
    sets = [CharSet(to_str(a)) for a in args]
    out: list[str] = []
    for c in v:
        if out and out[-1] == c and all(c in s for s in sets):
            continue
        out.append(c)
    return "".join(out)


def _justify(v: str, args: list, mode: str) -> str:
    argc(args, 1, 2)
    w = to_int(args[0])
    pad = to_str(args[1]) if len(args) > 1 else " "
    if pad == "":
        raise RubyError("ArgumentError", "zero width padding")
    v = str(v)
    n = len(v)
    if w <= n:
        return v
    _check_size(w)
    total = w - n
    if mode == "l":
        left, right = 0, total
    elif mode == "r":
        left, right = total, 0
    else:
        left = total // 2
        right = total - left

    def fill(k: int) -> str:
        return (pad * (k // len(pad) + 1))[:k]

    return fill(left) + v + fill(right)


@_m("center")
def s_center(it: Any, v: str, args: list, blk: Any) -> Value:
    return _justify(v, args, "c")


@_m("ljust")
def s_ljust(it: Any, v: str, args: list, blk: Any) -> Value:
    return _justify(v, args, "l")


@_m("rjust")
def s_rjust(it: Any, v: str, args: list, blk: Any) -> Value:
    return _justify(v, args, "r")


@_m("to_i")
def s_to_i(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0, 1)
    base = to_int(args[0]) if args else 10
    if base < 0 or base == 1 or base > 36:
        raise RubyError("ArgumentError", f"invalid radix {base}")
    return str_to_i(v, base)


@_m("hex")
def s_hex(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return str_to_i(v, 16)


@_m("oct")
def s_oct(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return str_to_i(v, -8)


@_m("to_f")
def s_to_f(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return str_to_f(v)


@_m("to_s", "to_str")
def s_to_s(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v


@_m("to_sym")
def s_to_sym(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return Symbol(str(v))


@_m("ord")
def s_ord(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    if not v:
        raise RubyError("ArgumentError", "empty string")
    return ord(v[0])


def _split_awk(v: str, limit: int) -> list[str]:
    parts: list[str] = []
    i = 0
    n = len(v)
    while i < n and v[i] in WS_SPLIT:
        i += 1
    while i < n:
        if limit > 0 and len(parts) == limit - 1:
            parts.append(v[i:])
            return parts
        j = i
        while j < n and v[j] not in WS_SPLIT:
            j += 1
        parts.append(v[i:j])
        i = j
        skipped = False
        while i < n and v[i] in WS_SPLIT:
            i += 1
            skipped = True
        if i == n and skipped:
            parts.append("")
    return parts


@_m("split")
def s_split(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0, 2)
    sep = args[0] if args else None
    limit = to_int(args[1]) if len(args) > 1 else 0
    v = str(v)
    if sep is not None and not isinstance(sep, str):
        raise RubyError("TypeError", f"wrong argument type {class_name(sep)} (expected Regexp)")
    if v == "":
        return []
    if limit == 1:
        return [v]
    if sep is None or sep == " ":
        parts = _split_awk(v, limit)
    elif sep == "":
        chars = list(v)
        if limit > 0 and len(chars) > limit:
            parts = chars[: limit - 1] + ["".join(chars[limit - 1 :])]
        else:
            parts = chars
            if limit < 0:
                parts = parts + [""]
    else:
        parts = []
        pos = 0
        while True:
            if limit > 0 and len(parts) == limit - 1:
                break
            idx = v.find(sep, pos)
            if idx < 0:
                break
            parts.append(v[pos:idx])
            pos = idx + len(sep)
        parts.append(v[pos:])
    if limit == 0:
        while parts and parts[-1] == "":
            parts.pop()
    if blk is not None:
        for p in parts:
            it.yield_(blk, p)
        return v
    return parts


@_m("partition")
def s_partition(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 1)
    sep = to_str(args[0])
    v = str(v)
    idx = v.find(sep)
    if idx < 0:
        return [v, "", ""]
    return [v[:idx], str(sep), v[idx + len(sep) :]]


@_m("rpartition")
def s_rpartition(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 1)
    sep = to_str(args[0])
    v = str(v)
    idx = v.rfind(sep)
    if idx < 0:
        return ["", "", v]
    return [v[:idx], str(sep), v[idx + len(sep) :]]


def _ascii_lower(s: str) -> str:
    return "".join(chr(ord(c) + 32) if "A" <= c <= "Z" else c for c in s)


@_m("casecmp")
def s_casecmp(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 1)
    o = args[0]
    if not isinstance(o, str):
        if type(o) is Symbol:
            raise no_implicit(o, "String")
        return None
    a, b = _ascii_lower(v), _ascii_lower(o)
    return -1 if a < b else (1 if a > b else 0)


@_m("casecmp?")
def s_casecmp_p(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 1)
    o = args[0]
    if not isinstance(o, str):
        if type(o) is Symbol:
            raise no_implicit(o, "String")
        return None
    return v.casefold() == o.casefold()


@_m("ascii_only?")
def s_ascii_only(it: Any, v: str, args: list, blk: Any) -> Value:
    argc(args, 0)
    return v.isascii()
