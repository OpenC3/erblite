"""``format`` / ``sprintf`` / ``String#%`` (spec §10), implemented explicitly."""

from __future__ import annotations

import math
from decimal import Decimal

from .errors import RubyError
from .values import RHash, Symbol, Value, class_name, inspect, to_s

MAX_WIDTH = 10_000_000
MAXFRACT = 1026

_UNSET = object()


def _arg_error(msg: str) -> RubyError:
    return RubyError("ArgumentError", msg)


def _to_integer(v: Value) -> int:
    from .builtins import float_to_int
    from .builtins_str import parse_integer_strict

    t = type(v)
    if t is int:
        return v
    if t is float:
        return float_to_int(v)
    if isinstance(v, str):
        return parse_integer_strict(v, None)
    if v is None:
        raise RubyError("TypeError", "can't convert nil into Integer")
    raise RubyError("TypeError", f"can't convert {class_name(v)} into Integer")


def _to_float(v: Value) -> float:
    from .builtins_str import parse_float_strict

    t = type(v)
    if t is float:
        return v
    if t is int:
        return float(v)
    if isinstance(v, str):
        return parse_float_strict(v)
    if v is None:
        raise RubyError("TypeError", "can't convert nil into Float")
    raise RubyError("TypeError", f"can't convert {class_name(v)} into Float")


def _pad(body: str, width: int | None, left: bool) -> str:
    if width is None or len(body) >= width:
        return body
    return body.ljust(width) if left else body.rjust(width)


def _fmt_int(n: int, t: str, flags: str, width: int | None, prec: int | None) -> str:
    neg = n < 0
    if t in "xXobB" and neg and "+" not in flags and " " not in flags:
        raise _arg_error(f"negative value {n} with %{t} is not supported (Ruby prints two's complement)")
    a = abs(n)
    if t in "diu":
        digits = str(a)
    elif t == "x":
        digits = format(a, "x")
    elif t == "X":
        digits = format(a, "X")
    elif t == "o":
        digits = format(a, "o")
    else:
        digits = format(a, "b")
    if prec is not None:
        digits = "" if (prec == 0 and a == 0) else digits.rjust(prec, "0")
    sign = "-" if neg else ("+" if "+" in flags else (" " if " " in flags else ""))
    prefix = ""
    if "#" in flags and a != 0:
        if t == "x":
            prefix = "0x"
        elif t == "X":
            prefix = "0X"
        elif t == "b":
            prefix = "0b"
        elif t == "B":
            prefix = "0B"
        elif t == "o" and not digits.startswith("0"):
            prefix = "0"
    head = sign + prefix
    if width is not None and len(head) + len(digits) < width:
        if "-" in flags:
            return (head + digits).ljust(width)
        if "0" in flags and prec is None:
            return head + digits.rjust(width - len(head), "0")
        return (head + digits).rjust(width)
    return head + digits


TENS = [float(10**i) for i in range(23)]
BIGTENS = [1e16, 1e32, 1e64, 1e128, 1e256]
_TWO_M52 = 2.0**-52


class _Fail(Exception):
    def __init__(self, half: bool) -> None:
        super().__init__()
        self.half = half


def _bump_up(digits: list[int], k: int) -> tuple[list[int], int]:
    """dtoa's ``bump_up``: drop trailing 9s and increment the last remaining digit."""
    while digits and digits[-1] == 9:
        digits.pop()
    if not digits:
        return [1], k + 1
    digits[-1] += 1
    return digits, k


def _estimate_k(d: float) -> tuple[int, bool]:
    m, e = math.frexp(d)
    d2 = m * 2.0
    i = e - 1
    ds = (d2 - 1.5) * 0.289529654602168 + 0.1760912590558 + i * 0.301029995663981
    k = int(ds)
    if ds < 0.0 and ds != k:
        k -= 1
    k_check = True
    if 0 <= k <= 22:
        if d < TENS[k]:
            k -= 1
        k_check = False
    return k, k_check


def _quick(d: float, k: int, k_check: bool, ilim: int, ilim1: int) -> tuple[list[int], int]:
    """dtoa's floating-point fast path (modes 2 and 3). Raises _Fail when undecided."""
    ieps = 2
    i = 0
    if k > 0:
        ds = TENS[k & 0xF]
        j = k >> 4
        if j & 0x10:
            j &= 0x10 - 1
            d /= BIGTENS[4]
            ieps += 1
        while j:
            if j & 1:
                ieps += 1
                ds *= BIGTENS[i]
            j >>= 1
            i += 1
        d /= ds
    elif k < 0:
        j1 = -k
        d *= TENS[j1 & 0xF]
        j = j1 >> 4
        while j:
            if j & 1:
                ieps += 1
                d *= BIGTENS[i]
            j >>= 1
            i += 1
    if k_check and d < 1.0 and ilim > 0:
        if ilim1 <= 0:
            raise _Fail(False)
        ilim = ilim1
        k -= 1
        d *= 10.0
        ieps += 1
    eps = (ieps * d + 7.0) * _TWO_M52
    if ilim == 0:
        d -= 5.0
        if d > eps:
            return [1], k + 1
        if d < -eps:
            return [], k  # no digits (caller sets decpt)
        raise _Fail(False)
    eps *= TENS[ilim - 1]
    digits: list[int] = []
    i = 1
    while True:
        L = int(d)
        d -= L
        if d == 0.0:
            ilim = i
        digits.append(L)
        if i == ilim:
            if d > 0.5 + eps:
                return _bump_up(digits, k)
            if d < 0.5 - eps:
                while digits and digits[-1] == 0:
                    digits.pop()
                return digits, k
            if L & 1:
                return _bump_up(digits, k)
            raise _Fail(True)
        i += 1
        d *= 10.0


def ruby_dtoa(value: float, mode: int, ndigits: int) -> tuple[str, int]:
    """Port of Ruby's BSD__dtoa for modes 2 (significant digits) and 3 (fraction digits).

    Returns (digits, decpt) with value ~= 0.digits * 10**decpt. Ruby's version rounds
    half-even on the floating-point estimate when it is within rounding error of a tie,
    which is why e.g. ``format("%.1f", 0.35)`` gives ``"0.4"``.
    """
    d = abs(value)
    if d == 0.0:
        return "0", 1
    k, k_check = _estimate_k(d)
    if mode == 2:
        if ndigits <= 0:
            ndigits = 1
        ilim = ilim1 = ndigits
    else:
        ilim = ndigits + k + 1
        ilim1 = ilim - 1
    half = False
    if 0 <= ilim <= 14:
        try:
            digits, kk = _quick(d, k, k_check, ilim, ilim1)
            if not digits:
                return "", -ndigits
            return "".join(map(str, digits)), kk + 1
        except _Fail as f:
            half = f.half
    # Small integer path
    if d == math.floor(d) and d < 2.0**53 and k <= 14:
        ds = TENS[k]
        digits = []
        i = 1
        while True:
            L = int(d / ds)
            d -= L * ds
            digits.append(L)
            if d == 0.0:
                break
            if i == ilim:
                d += d
                if d > ds or (d == ds and L & 1):
                    digits, k = _bump_up(digits, k)
                break
            i += 1
            d *= 10.0
        return "".join(map(str, digits)), k + 1
    # Exact (bignum) path, using the finite exact decimal expansion of the double.
    t = Decimal(d).as_tuple()
    digs = list(t.digits)
    exp = int(t.exponent)
    while len(digs) > 1 and digs[-1] == 0:
        digs.pop()
        exp += 1
    k = len(digs) + exp - 1
    if mode == 3:
        ilim = ndigits + k + 1
        if ilim <= 0:
            if ilim < 0 or digs[0] < 5 or (digs[0] == 5 and len(digs) == 1):
                return "", -ndigits
            return "1", k + 2
    if ilim >= len(digs):
        return "".join(map(str, digs)), k + 1
    kept = digs[:ilim]
    rest = digs[ilim:]
    above = rest[0] > 5 or (rest[0] == 5 and len(rest) > 1)
    tie = rest[0] == 5 and len(rest) == 1
    if above or (tie and kept[-1] & 1):
        while kept and kept[-1] == 9:
            kept.pop()
        if not kept:
            return "1", k + 2
        if not half or kept[-1] & 1:
            kept[-1] += 1
        return "".join(map(str, kept)), k + 1
    while kept and kept[-1] == 0:
        kept.pop()
    return "".join(map(str, kept)), k + 1


def _sign_pad(neg: bool, body: str, flags: str, width: int | None) -> str:
    sign = "-" if neg else ("+" if "+" in flags else (" " if " " in flags else ""))
    if width is not None and len(sign) + len(body) < width:
        if "-" in flags:
            return (sign + body).ljust(width)
        if "0" in flags:
            return sign + body.rjust(width - len(sign), "0")
        return (sign + body).rjust(width)
    return sign + body


def _fixed(digits: str, decpt: int, prec: int, alt: bool) -> str:
    if decpt > 0:
        ip = (digits[:decpt]).ljust(decpt, "0")
        frac = digits[decpt:]
    else:
        ip = "0"
        frac = "0" * (-decpt) + digits
    frac = frac[:prec].ljust(prec, "0")
    if prec > 0 or alt:
        return ip + "." + frac
    return ip


def _exp_str(e: int) -> str:
    return ("-" if e < 0 else "+") + f"{abs(e):02d}"


def _fmt_float(x: float, t: str, flags: str, width: int | None, prec: int | None) -> str:
    alt = "#" in flags
    if not math.isfinite(x):
        body = "NaN" if x != x else "Inf"
        if x != x or x > 0:
            body = ("+" if "+" in flags else (" " if " " in flags else "")) + body
        else:
            body = "-" + body
        return _pad(body, width, "-" in flags)
    neg = math.copysign(1.0, x) < 0
    if prec is None:
        prec = 6
    if t == "f":
        # Ruby's BSD vfprintf asks dtoa for at most MAXFRACT (1026) fraction digits, then pads zeros.
        digits, decpt = ruby_dtoa(x, 3, min(prec, MAXFRACT))
        if not digits:
            digits, decpt = "0", 1
        body = _fixed(digits, decpt, prec, alt)
    elif t in "eE":
        digits, decpt = ruby_dtoa(x, 2, prec + 1)
        if x == 0.0:
            decpt = 1
        digits = digits.ljust(prec + 1, "0")
        body = digits[0] + ("." + digits[1:] if prec > 0 or alt else "") + t + _exp_str(decpt - 1)
    else:
        p = prec if prec > 0 else 1
        digits, decpt = ruby_dtoa(x, 2, p)
        if alt:
            digits = digits.ljust(p, "0")
        if decpt <= -4 or (decpt > p and decpt > 1):
            e = "e" if t == "g" else "E"
            rest = digits[1:]
            body = digits[0] + ("." + rest if rest or alt else "") + e + _exp_str(decpt - 1)
        elif decpt >= len(digits):
            body = digits + "0" * (decpt - len(digits)) + ("." if alt else "")
        elif decpt > 0:
            body = digits[:decpt] + "." + digits[decpt:]
        else:
            body = "0." + "0" * (-decpt) + digits
    return _sign_pad(neg, body, flags, width)


def _fmt_int_as_f(n: int, flags: str, width: int | None, prec: int | None) -> str:
    if prec is None:
        prec = 6
    body = str(abs(n))
    if prec > 0 or "#" in flags:
        body += "." + "0" * prec
    return _sign_pad(n < 0, body, flags, width)


def format_str(fmt: str, args: list) -> str:
    out: list[str] = []
    i = 0
    n = len(fmt)
    pos = 0
    mode = None  # "unnumbered" | "named" | "numbered"
    hash_arg: RHash | None = None

    def set_mode(m: str) -> None:
        nonlocal mode
        if mode is not None and mode != m:
            if m == "named" or mode == "named":
                raise _arg_error(
                    f"{'named' if m == 'named' else 'unnumbered'} mixed with "
                    f"{'named' if mode == 'named' else 'unnumbered'}"
                )
            raise _arg_error(f"{m} mixed with {mode}")
        mode = m

    def next_arg() -> Value:
        nonlocal pos
        set_mode("unnumbered")
        if pos >= len(args):
            raise _arg_error("too few arguments")
        v = args[pos]
        pos += 1
        return v

    def get_hash() -> RHash:
        nonlocal hash_arg
        if hash_arg is None:
            if len(args) != 1 or type(args[0]) is not RHash:
                raise _arg_error("one hash required")
            hash_arg = args[0]
        return hash_arg

    def named(name: str, open_: str, close: str) -> Value:
        set_mode("named")
        h = get_hash()
        key = Symbol(name)
        if h.has(key):
            return h.get(key)
        if h.default is not None:
            return h.default
        raise RubyError("KeyError", f"key{open_}{name}{close} not found")

    while i < n:
        c = fmt[i]
        if c != "%":
            j = fmt.find("%", i)
            if j < 0:
                j = n
            out.append(fmt[i:j])
            i = j
            continue
        i += 1
        if i >= n:
            raise _arg_error("incomplete format specifier; use %% (double %) instead")
        flags = ""
        width: int | None = None
        prec: int | None = None
        value: Value = _UNSET
        brace_value: Value = _UNSET
        while True:
            if i >= n:
                raise _arg_error("malformed format string - %*[0-9]")
            c = fmt[i]
            if c in "-+ 0#":
                flags += c
                i += 1
                continue
            if c == "<" or c == "{":
                close = ">" if c == "<" else "}"
                j = fmt.find(close, i + 1)
                if j < 0:
                    raise _arg_error("malformed name - unmatched parenthesis")
                name = fmt[i + 1 : j]
                v = named(name, c, close)
                i = j + 1
                if c == "{":
                    brace_value = v
                    break
                value = v
                continue
            if c == "*":
                i += 1
                wv = _to_integer(next_arg())
                if wv < 0:
                    flags += "-"
                    wv = -wv
                width = wv
                continue
            if c.isdigit():
                j = i
                while j < n and fmt[j].isdigit():
                    j += 1
                num = int(fmt[i:j])
                if j < n and fmt[j] == "$":
                    set_mode("numbered")
                    if num < 1 or num > len(args):
                        raise _arg_error(f"invalid index - {num}$")
                    value = args[num - 1]
                    i = j + 1
                    continue
                width = num
                i = j
                continue
            if c == ".":
                i += 1
                if i < n and fmt[i] == "*":
                    i += 1
                    pv = _to_integer(next_arg())
                    prec = pv if pv >= 0 else None
                else:
                    j = i
                    while j < n and fmt[j].isdigit():
                        j += 1
                    prec = int(fmt[i:j]) if j > i else 0
                    i = j
                continue
            break
        if (width is not None and width > MAX_WIDTH) or (prec is not None and prec > MAX_WIDTH):
            raise RubyError("LimitError", "format width or precision too big")
        if brace_value is not _UNSET:
            out.append(_pad(to_s(brace_value), width, "-" in flags))
            continue
        t = fmt[i]
        i += 1
        if t == "%":
            if flags or width is not None or prec is not None or value is not _UNSET:
                raise _arg_error("invalid format character - %")
            out.append("%")
            continue
        if t == "\n" or t == "\0":
            out.append("%" + t)
            continue
        if t not in "diuxXobBfeEgGaAscp":
            raise _arg_error(f"malformed format string - %{t}")
        if value is _UNSET:
            value = next_arg()
        if t in "diuxXobB":
            out.append(_fmt_int(_to_integer(value), t, flags, width, prec))
        elif t == "f" and type(value) is int:
            out.append(_fmt_int_as_f(value, flags, width, prec))
        elif t in "feEgG":
            out.append(_fmt_float(_to_float(value), t, flags, width, prec))
        elif t in "aA":
            raise _arg_error(f"%{t} is not supported")
        elif t == "s" or t == "p":
            s = to_s(value) if t == "s" else inspect(value)
            if prec is not None:
                s = s[:prec]
            out.append(_pad(s, width, "-" in flags))
        else:  # c
            if type(value) is int:
                if value < 0 or value > 0x10FFFF or 0xD800 <= value <= 0xDFFF:
                    raise _arg_error("invalid character")
                s = chr(value)
            elif isinstance(value, str):
                s = value[:1]
            else:
                s = chr(_to_integer(value))
            out.append(_pad(s, width, "-" in flags))
    return "".join(out)
