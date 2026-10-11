"""The file sandbox (spec §9.1): ``File``, ``Dir`` and the reading part of ``render``.

Every path is resolved against ``base_dir`` with :func:`os.path.realpath` (which resolves
``..`` and symlinks) and must stay inside the real path of ``base_dir``.
"""

from __future__ import annotations

import os
from typing import Any

from .errors import RubyError
from .values import Value

MAX_FILE_SIZE = 64 * 1024 * 1024
MAX_GLOB_RESULTS = 100_000


def _security(msg: str) -> RubyError:
    return RubyError("SecurityError", msg)


class Sandbox:
    def __init__(self, base_dir: str | os.PathLike[str] | None) -> None:
        self.base = os.path.realpath(os.fspath(base_dir)) if base_dir is not None else None

    def _require_base(self) -> str:
        if self.base is None:
            raise _security("file access is disabled (no base_dir given)")
        return self.base

    def inside(self, real: str) -> bool:
        base = self._require_base()
        return real == base or real.startswith(base.rstrip(os.sep) + os.sep)

    def resolve(self, path: str) -> str:
        base = self._require_base()
        if "\0" in path:
            raise RubyError("ArgumentError", "string contains null byte")
        real = os.path.realpath(os.path.join(base, path))
        if not self.inside(real):
            raise _security(f"path is outside the sandbox: {path}")
        return real

    def read(self, path: str) -> str:
        real = self.resolve(path)
        try:
            if os.path.isdir(real):
                raise RubyError("IOError", f"Is a directory @ io_fread - {path}")
            with open(real, "rb") as f:
                data = f.read(MAX_FILE_SIZE + 1)
        except FileNotFoundError:
            raise RubyError("IOError", f"No such file or directory @ rb_sysopen - {path}") from None
        except OSError as e:
            raise RubyError("IOError", f"{e.strerror or 'cannot read'} - {path}") from None
        if len(data) > MAX_FILE_SIZE:
            raise RubyError("IOError", f"file too large - {path}")
        return data.decode("utf-8", errors="replace")

    def check(self, path: str, kind: str) -> bool:
        base = self._require_base()
        if "\0" in path:
            raise RubyError("ArgumentError", "string contains null byte")
        real = os.path.realpath(os.path.join(base, path))
        if not self.inside(real):
            return False
        if kind == "exist":
            return os.path.exists(real)
        if kind == "file":
            return os.path.isfile(real)
        return os.path.isdir(real)

    # -- glob ------------------------------------------------------------------------------

    def glob(self, pattern: str) -> list[str]:
        base = self._require_base()
        if "\0" in pattern:
            raise RubyError("ArgumentError", "string contains null byte")
        out: list[str] = []
        for p in brace_expand(pattern):
            if p.startswith("/") or os.path.isabs(p) or ".." in p.split("/"):
                raise _security(f"glob pattern is outside the sandbox: {pattern}")
            self._glob(base, "", p.split("/"), out)
        seen = set()
        result = []
        for r in out:
            if r not in seen:
                seen.add(r)
                result.append(r)
        result.sort(key=lambda s: s.split("/"))
        return result

    def _add(self, out: list[str], full: str, rel: str) -> None:
        if self.inside(os.path.realpath(full)):
            out.append(rel)
            if len(out) > MAX_GLOB_RESULTS:
                raise RubyError("LimitError", "too many glob results")

    def _glob(self, dirpath: str, prefix: str, segs: list[str], out: list[str]) -> None:
        if not segs or not self.inside(os.path.realpath(dirpath)):
            return
        seg, rest = segs[0], segs[1:]
        if seg == "**" and rest:
            self._glob(dirpath, prefix, rest, out)
            for name in _listdir(dirpath):
                full = os.path.join(dirpath, name)
                if name.startswith(".") or os.path.islink(full) or not os.path.isdir(full):
                    continue
                self._glob(full, prefix + name + "/", segs, out)
            return
        if seg == "**":
            seg = "*"
        if seg == "" and not rest:
            if prefix:
                self._add(out, dirpath, prefix)
            return
        if not _has_magic(seg):
            full = os.path.join(dirpath, seg) if seg else dirpath
            if not rest:
                if os.path.lexists(full):
                    self._add(out, full, prefix + seg)
            elif os.path.isdir(full):
                self._glob(full, prefix + seg + "/", rest, out)
            return
        toks = _compile_glob(seg)
        names = _listdir(dirpath)
        if seg.startswith("."):
            names = sorted(names + ["."])  # Ruby's glob yields "." (but not "..") for dot patterns
        for name in names:
            if name.startswith(".") and not seg.startswith("."):
                continue
            if not _glob_match(toks, name):
                continue
            full = os.path.join(dirpath, name)
            if not rest:
                self._add(out, full, prefix + name)
            elif rest == [""]:
                if os.path.isdir(full):
                    self._add(out, full, prefix + name + "/")
            elif os.path.isdir(full):
                self._glob(full, prefix + name + "/", rest, out)


def _listdir(path: str) -> list[str]:
    try:
        return sorted(os.listdir(path))
    except OSError:
        return []


def _has_magic(seg: str) -> bool:
    return any(c in seg for c in "*?[")


def _compile_glob(seg: str) -> list[tuple]:
    """Parses one path segment of a glob into tokens: ('*',), ('?',), ('c', char) or
    ('set', negate, [(lo, hi), ...])."""
    toks: list[tuple] = []
    i = 0
    n = len(seg)
    while i < n:
        c = seg[i]
        if c == "*":
            if not toks or toks[-1] != ("*",):
                toks.append(("*",))
        elif c == "?":
            toks.append(("?",))
        elif c == "[":
            j = i + 1
            neg = j < n and seg[j] in "!^"
            if neg:
                j += 1
            ranges: list[tuple[str, str]] = []
            first = True
            while j < n and (seg[j] != "]" or first):
                first = False
                lo = seg[j]
                if lo == "\\" and j + 1 < n:
                    j += 1
                    lo = seg[j]
                if j + 2 < n and seg[j + 1] == "-" and seg[j + 2] != "]":
                    ranges.append((lo, seg[j + 2]))
                    j += 3
                else:
                    ranges.append((lo, lo))
                    j += 1
            if j >= n:
                toks.append(("c", "["))
            else:
                toks.append(("set", neg, ranges))
                i = j
        elif c == "\\" and i + 1 < n:
            i += 1
            toks.append(("c", seg[i]))
        else:
            toks.append(("c", c))
        i += 1
    return toks


def _tok_match(tok: tuple, ch: str) -> bool:
    kind = tok[0]
    if kind == "?":
        return True
    if kind == "c":
        return tok[1] == ch
    hit = any(lo <= ch <= hi for lo, hi in tok[2])
    return hit != tok[1]


def _glob_match(toks: list[tuple], name: str) -> bool:
    """Wildcard match in O(len(toks) * len(name)) (no regular expressions, no backtracking blowup)."""
    pi = si = 0
    star = -1
    mark = 0
    nt = len(toks)
    while si < len(name):
        if pi < nt and toks[pi][0] == "*":
            star = pi
            mark = si
            pi += 1
        elif pi < nt and _tok_match(toks[pi], name[si]):
            pi += 1
            si += 1
        elif star >= 0:
            pi = star + 1
            mark += 1
            si = mark
        else:
            return False
    while pi < nt and toks[pi][0] == "*":
        pi += 1
    return pi == nt


def brace_expand(p: str, limit: int = 1024) -> list[str]:
    depth = 0
    start = -1
    for i, c in enumerate(p):
        if c == "{":
            if depth == 0:
                start = i
            depth += 1
        elif c == "}" and depth > 0:
            depth -= 1
            if depth == 0:
                inner = p[start + 1 : i]
                alts = []
                d = 0
                cur = ""
                for ch in inner:
                    if ch == "," and d == 0:
                        alts.append(cur)
                        cur = ""
                        continue
                    if ch == "{":
                        d += 1
                    elif ch == "}":
                        d -= 1
                    cur += ch
                alts.append(cur)
                out: list[str] = []
                for a in alts:
                    out.extend(brace_expand(p[:start] + a + p[i + 1 :], limit))
                    if len(out) > limit:
                        raise RubyError("LimitError", "glob pattern expands to too many alternatives")
                return out
    return [p]


# ---------------------------------------------------------------------------------------
# Pure path functions


def basename(p: str, suffix: str | None = None) -> str:
    if p == "":
        return ""
    s = p.rstrip("/")
    if s == "":
        return "/"
    name = s.rsplit("/", 1)[-1]
    if suffix:
        if suffix == ".*":
            idx = name.rfind(".")
            if idx > 0:
                name = name[:idx]
        elif name.endswith(suffix) and name != suffix:
            name = name[: -len(suffix)]
    return name


def dirname(p: str) -> str:
    if p == "":
        return "."
    s = p.rstrip("/")
    if s == "":
        return "/"
    idx = s.rfind("/")
    if idx < 0:
        return "."
    d = s[:idx].rstrip("/")
    return d if d else "/"


def extname(p: str) -> str:
    name = basename(p)
    if name == "/":
        return ""
    n = name.lstrip(".")
    idx = n.rfind(".")
    if idx < 0:
        return ""
    return n[idx:]


def join(parts: list[str]) -> str:
    result = ""
    for i, part in enumerate(parts):
        if i == 0:
            result = part
            continue
        if part.startswith("/"):
            result = result.rstrip("/")
        elif not result.endswith("/"):
            result += "/"
        result += part
    return result


def register() -> None:
    from .builtins import argc, singleton, to_str

    @singleton("File", "read")
    def f_read(it: Any, v: Value, args: list, blk: Any) -> Value:
        argc(args, 1)
        return it.ctx.sandbox.read(str(to_str(args[0])))

    def checker(kind: str) -> Any:
        def h(it: Any, v: Value, args: list, blk: Any) -> Value:
            argc(args, 1)
            return it.ctx.sandbox.check(str(to_str(args[0])), kind)

        return h

    singleton("File", "exist?")(checker("exist"))
    singleton("File", "file?")(checker("file"))
    singleton("File", "directory?")(checker("dir"))

    @singleton("File", "basename")
    def f_basename(it: Any, v: Value, args: list, blk: Any) -> Value:
        argc(args, 1, 2)
        suffix = str(to_str(args[1])) if len(args) > 1 else None
        return basename(str(to_str(args[0])), suffix)

    @singleton("File", "dirname")
    def f_dirname(it: Any, v: Value, args: list, blk: Any) -> Value:
        argc(args, 1)
        return dirname(str(to_str(args[0])))

    @singleton("File", "extname")
    def f_extname(it: Any, v: Value, args: list, blk: Any) -> Value:
        argc(args, 1)
        return extname(str(to_str(args[0])))

    @singleton("File", "join")
    def f_join(it: Any, v: Value, args: list, blk: Any) -> Value:
        flat: list[str] = []

        def add(x: Value, depth: int) -> None:
            if type(x) is list:
                if depth > 100:
                    raise RubyError("ArgumentError", "recursive array")
                for y in x:
                    add(y, depth + 1)
            else:
                flat.append(str(to_str(x)))

        for a in args:
            add(a, 0)
        return join(flat)

    @singleton("Dir", "glob")
    def d_glob(it: Any, v: Value, args: list, blk: Any) -> Value:
        argc(args, 1)
        pats = args[0] if type(args[0]) is list else [args[0]]
        out: list[str] = []
        for p in pats:
            out.extend(it.ctx.sandbox.glob(str(to_str(p))))
        if blk is not None:
            for r in out:
                it.yield_void(blk, r)
            return None
        return out


register()
