"""Template scanner and code tokenizer.

The template is first split into text, code (``<% %>``), output (``<%= %>``) and comment
segments using the same rules as Ruby's ERB ``ExplicitScanner`` (trim mode ``-``).  The
code inside the tags is then tokenized.  All segments become one flat token stream that
the parser reads as a single program.
"""

from __future__ import annotations

import re

from .errors import RubyError, syntax_error

INT_MIN = -(2**63)
INT_MAX = 2**63 - 1

KEYWORDS = frozenset(
    "if elsif else unless then end while until for in do case when break next nil true false and or not "
    "begin self".split()
)
# Ruby keywords and constructs that ERBLite does not support. Using one is a SyntaxError.
RESERVED = frozenset(
    "def class module return yield rescue ensure super alias undef redo retry defined? __FILE__ __LINE__ "
    "__method__ __dir__ __ENCODING__ BEGIN END begin self".split()
)

# Tokens after which a newline does not end the statement.
OPERATORS = [
    "**=",
    "<=>",
    "===",
    "...",
    "<<=",
    ">>=",
    "&&=",
    "||=",
    "**",
    "==",
    "!=",
    ">=",
    "<=",
    "&&",
    "||",
    "<<",
    ">>",
    "+=",
    "-=",
    "*=",
    "/=",
    "%=",
    "|=",
    "&=",
    "^=",
    "..",
    "::",
    "&.",
    "=>",
    "+",
    "-",
    "*",
    "/",
    "%",
    "=",
    "<",
    ">",
    "!",
    "&",
    "|",
    "^",
    "~",
    "?",
    ":",
    ",",
    ".",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
]

SYMBOL_OPERATORS = [
    "[]=",
    "<=>",
    "===",
    "==",
    "!=",
    "<=",
    ">=",
    "<<",
    ">>",
    "**",
    "[]",
    "+@",
    "-@",
    "+",
    "-",
    "*",
    "/",
    "%",
    "<",
    ">",
    "!",
    "&",
    "|",
    "^",
    "~",
]

VALUE_END_KINDS = frozenset(["int", "float", "str", "sym", "dsym", "ident", "const", "words", "symwords"])
VALUE_END_OPS = frozenset([")", "]", "}"])
VALUE_END_KWS = frozenset(["nil", "true", "false", "end"])

PAIRS = {"[": "]", "(": ")", "{": "}", "<": ">"}

SIMPLE_ESCAPES = {
    "n": "\n",
    "t": "\t",
    "r": "\r",
    "s": " ",
    "e": "\x1b",
    "a": "\x07",
    "b": "\x08",
    "f": "\x0c",
    "v": "\x0b",
}


class Token:
    __slots__ = ("kind", "value", "line", "spc", "neg")

    def __init__(self, kind: str, value: object, line: int, spc: bool = False) -> None:
        self.kind = kind
        self.value = value
        self.line = line
        self.spc = spc
        self.neg = False  # numeric literal that carried a leading "-"

    def is_op(self, v: str) -> bool:
        return self.kind == "op" and self.value == v

    def is_kw(self, v: str) -> bool:
        return self.kind == "kw" and self.value == v

    def __repr__(self) -> str:
        return f"Token({self.kind}, {self.value!r}, line={self.line})"


def _ends_value(tok: Token | None) -> bool:
    if tok is None:
        return False
    if tok.kind in VALUE_END_KINDS:
        return True
    if tok.kind == "op":
        return tok.value in VALUE_END_OPS
    if tok.kind == "kw":
        return tok.value in VALUE_END_KWS
    return False


def _is_ident_start(c: str) -> bool:
    return c == "_" or ("a" <= c <= "z") or ("A" <= c <= "Z") or ord(c) > 127


def _is_ident_char(c: str) -> bool:
    return c == "_" or c.isascii() and c.isalnum() or ord(c) > 127


class CodeLexer:
    """Tokenizes the Ruby subset inside template tags."""

    def __init__(self, src: str, line: int) -> None:
        self.src = src
        self.pos = 0
        self.line = line

    def error(self, msg: str) -> RubyError:
        return syntax_error(msg, self.line)

    def peek(self, off: int = 0) -> str:
        p = self.pos + off
        return self.src[p] if p < len(self.src) else ""

    def tokenize(self, until_brace: bool = False) -> list[Token]:
        """Tokenize until end of input (or the ``}`` closing an interpolation)."""
        toks: list[Token] = []
        depth = 0
        src = self.src
        n = len(src)
        spc = False
        while True:
            if self.pos >= n:
                if until_brace:
                    raise self.error("unterminated string interpolation")
                break
            c = src[self.pos]
            if c in " \t\r\f\v":
                self.pos += 1
                spc = True
                continue
            if c == "\\" and self.peek(1) == "\n":
                self.pos += 2
                self.line += 1
                spc = True
                continue
            if c == "\\" and self.peek(1) == "\r" and self.peek(2) == "\n":
                self.pos += 3
                self.line += 1
                spc = True
                continue
            if c == "#":
                while self.pos < n and src[self.pos] != "\n":
                    self.pos += 1
                continue
            if c == "\n":
                if not toks or toks[-1].kind != "nl":
                    toks.append(Token("nl", None, self.line))
                self.pos += 1
                self.line += 1
                spc = False
                continue
            prev = toks[-1] if toks else None
            line = self.line
            if c == ";":
                toks.append(Token("semi", None, line))
                self.pos += 1
                spc = False
                continue
            if c.isdigit():
                toks.append(self.lex_number(spc, False))
                spc = False
                continue
            if c == "-" and self.peek(1).isdigit() and not _ends_value(prev):
                self.pos += 1
                toks.append(self.lex_number(spc, True))
                spc = False
                continue
            if _is_ident_start(c):
                toks.append(self.lex_ident(spc))
                spc = False
                continue
            if c == "'":
                self.pos += 1
                toks.append(Token("str", [self.lex_single_quoted()], line, spc))
                spc = False
                continue
            if c == '"':
                self.pos += 1
                toks.append(Token("str", self.lex_double_quoted('"'), line, spc))
                spc = False
                continue
            if c == ":" and self.peek(1) != ":":
                tok = self.lex_symbol(spc, prev)
                if tok is not None:
                    toks.append(tok)
                    spc = False
                    continue
            if c == "%" and self.peek(1) in ("w", "i") and self.peek(2) in PAIRS and not _ends_value(prev):
                toks.append(self.lex_words(spc))
                spc = False
                continue
            if c in "$@`":
                raise self.error(f"unsupported syntax '{c}'")
            if c == "=" and self.src.startswith("=begin", self.pos) and (self.pos == 0 or src[self.pos - 1] == "\n"):
                raise self.error("=begin comments are not supported")
            if src.startswith("=~", self.pos) or src.startswith("!~", self.pos):
                raise self.error("regular expressions are not supported")
            if src.startswith("->", self.pos):
                raise self.error("lambdas are not supported")
            op = None
            for cand in OPERATORS:
                if src.startswith(cand, self.pos):
                    op = cand
                    break
            if op is None:
                raise self.error(f"unexpected character '{c}'")
            self.pos += len(op)
            if until_brace:
                if op == "{":
                    depth += 1
                elif op == "}":
                    if depth == 0:
                        toks.append(Token("eof", None, line))
                        return toks
                    depth -= 1
            toks.append(Token("op", op, line, spc))
            spc = False
        toks.append(Token("eof", None, self.line))
        return toks

    # -- numbers ---------------------------------------------------------------------------

    def _digits(self, valid: str) -> str:
        """Reads digits (with single underscores between them)."""
        start = self.pos
        src = self.src
        out = []
        while self.pos < len(src):
            ch = src[self.pos]
            if ch in valid:
                out.append(ch)
                self.pos += 1
            elif ch == "_" and out and self.pos + 1 < len(src) and src[self.pos + 1] in valid:
                self.pos += 1
            elif ch == "_":
                raise self.error("trailing '_' in number")
            else:
                break
        if self.pos == start:
            return ""
        return "".join(out)

    def lex_number(self, spc: bool, neg: bool) -> Token:
        line = self.line
        src = self.src
        dec = "0123456789"
        if src[self.pos] == "0" and self.peek(1) in ("x", "X", "b", "B", "o", "O", "d", "D"):
            kind = self.peek(1).lower()
            self.pos += 2
            valid = {"x": "0123456789abcdefABCDEF", "b": "01", "o": "01234567", "d": dec}[kind]
            digits = self._digits(valid)
            if not digits:
                raise self.error("numeric literal without digits")
            base = {"x": 16, "b": 2, "o": 8, "d": 10}[kind]
            value = int(digits, base)
            if self.pos < len(src) and _is_ident_char(src[self.pos]):
                raise self.error("invalid numeric literal")
        else:
            digits = self._digits(dec)
            is_float = False
            text = digits
            if self.peek() == "." and self.peek(1).isdigit():
                self.pos += 1
                frac = self._digits(dec)
                text += "." + frac
                is_float = True
            if self.peek() in ("e", "E") and (
                self.peek(1).isdigit() or (self.peek(1) in "+-" and self.peek(2).isdigit())
            ):
                self.pos += 1
                sign = ""
                if self.peek() in "+-":
                    sign = self.peek()
                    self.pos += 1
                exp = self._digits(dec)
                text += "e" + sign + exp
                is_float = True
            if self.pos < len(src) and _is_ident_char(src[self.pos]):
                raise self.error("invalid numeric literal")
            if is_float:
                fv = float(text)
                tok = Token("float", -fv if neg else fv, line, spc)
                tok.neg = neg
                return tok
            if len(digits) > 1 and digits[0] == "0":
                if any(ch not in "01234567" for ch in digits):
                    raise self.error("Invalid octal digit")
                value = int(digits, 8)
            else:
                value = int(digits)
        if neg:
            value = -value
        if value < INT_MIN or value > INT_MAX:
            raise RubyError("RangeError", "integer literal out of 64-bit range", line)
        tok = Token("int", value, line, spc)
        tok.neg = neg
        return tok

    # -- identifiers -----------------------------------------------------------------------

    def lex_ident(self, spc: bool) -> Token:
        line = self.line
        src = self.src
        start = self.pos
        while self.pos < len(src) and _is_ident_char(src[self.pos]):
            self.pos += 1
        name = src[start : self.pos]
        if self.peek() in ("?", "!") and (self.peek(1) != "=" or self.peek(2) == "=") and not name[0].isupper():
            name += self.peek()
            self.pos += 1
        # label "name:" (but not "name::")
        if self.peek() == ":" and self.peek(1) != ":":
            self.pos += 1
            return Token("label", name, line, spc)
        if name in KEYWORDS:
            return Token("kw", name, line, spc)
        if name[0].isupper():
            return Token("const", name, line, spc)
        return Token("ident", name, line, spc)

    def lex_symbol(self, spc: bool, prev: Token | None) -> Token | None:
        line = self.line
        src = self.src
        nxt = self.peek(1)
        if _ends_value(prev) and not spc:
            return None  # "cond ? 'a':'b'" -- a ternary colon, not a symbol
        if nxt == '"':
            self.pos += 2
            parts = self.lex_double_quoted('"')
            return Token("dsym", parts, line, spc)
        if nxt == "'":
            self.pos += 2
            return Token("dsym", [self.lex_single_quoted()], line, spc)
        if nxt in ("@", "$") and nxt:
            j = self.pos + 2
            if nxt == "@" and self.peek(2) == "@":
                j += 1
            if j < len(src) and _is_ident_start(src[j]):
                start = self.pos + 1
                self.pos = j
                while self.pos < len(src) and _is_ident_char(src[self.pos]):
                    self.pos += 1
                return Token("sym", src[start : self.pos], line, spc)
            return None
        if nxt and _is_ident_start(nxt):
            self.pos += 1
            start = self.pos
            while self.pos < len(src) and _is_ident_char(src[self.pos]):
                self.pos += 1
            name = src[start : self.pos]
            c = self.peek()
            if c in ("?", "!") and self.peek(1) != "=":
                name += c
                self.pos += 1
            elif c == "=" and self.peek(1) not in ("=", "~", ">"):
                name += c
                self.pos += 1
            return Token("sym", name, line, spc)
        if nxt and nxt not in " \t\n":
            for cand in SYMBOL_OPERATORS:
                if src.startswith(cand, self.pos + 1):
                    self.pos += 1 + len(cand)
                    return Token("sym", cand, line, spc)
        return None

    # -- strings ---------------------------------------------------------------------------

    def lex_single_quoted(self) -> str:
        src = self.src
        out = []
        while True:
            if self.pos >= len(src):
                raise self.error("unterminated string meets end of file")
            c = src[self.pos]
            if c == "'":
                self.pos += 1
                return "".join(out)
            if c == "\\" and self.peek(1) in ("\\", "'"):
                out.append(self.peek(1))
                self.pos += 2
                continue
            if c == "\n":
                self.line += 1
            out.append(c)
            self.pos += 1

    def lex_double_quoted(self, term: str) -> list:
        """Returns parts: str pieces and token lists (interpolations)."""
        src = self.src
        parts: list = []
        buf: list[str] = []
        while True:
            if self.pos >= len(src):
                raise self.error("unterminated string meets end of file")
            c = src[self.pos]
            if c == term:
                self.pos += 1
                break
            if c == "\\":
                self.pos += 1
                buf.append(self.lex_escape())
                continue
            if c == "#" and self.peek(1) == "{":
                self.pos += 2
                if buf:
                    parts.append("".join(buf))
                    buf = []
                sub = CodeLexer(src, self.line)
                sub.pos = self.pos
                toks = sub.tokenize(until_brace=True)
                self.pos = sub.pos
                self.line = sub.line
                parts.append(toks)
                continue
            if c == "#" and self.peek(1) in ("@", "$") and self.peek(2):
                c2 = self.peek(2)
                if (
                    _is_ident_start(c2)
                    or (self.peek(1) == "@" and c2 == "@")
                    or (self.peek(1) == "$" and (c2.isdigit() or c2 in "~*$?!@/\\;,.=:<>&`'+"))
                ):
                    raise self.error("instance/global variable interpolation is not supported")
            if c == "\n":
                self.line += 1
            buf.append(c)
            self.pos += 1
        if buf or not parts:
            parts.append("".join(buf))
        return parts

    def lex_escape(self) -> str:
        src = self.src
        if self.pos >= len(src):
            raise self.error("unterminated string meets end of file")
        c = src[self.pos]
        self.pos += 1
        if c in SIMPLE_ESCAPES:
            return SIMPLE_ESCAPES[c]
        if c in "01234567":
            digits = c
            while len(digits) < 3 and self.peek() in "01234567" and self.peek():
                digits += self.peek()
                self.pos += 1
            return chr(int(digits, 8) & 0xFF)
        if c == "x":
            digits = ""
            while len(digits) < 2 and self.peek() and self.peek() in "0123456789abcdefABCDEF":
                digits += self.peek()
                self.pos += 1
            if not digits:
                raise self.error("invalid hex escape")
            return chr(int(digits, 16))
        if c == "u":
            if self.peek() == "{":
                self.pos += 1
                out = []
                while True:
                    while self.peek() in (" ", "\t"):
                        self.pos += 1
                    if self.peek() == "}":
                        self.pos += 1
                        break
                    digits = ""
                    while self.peek() and self.peek() in "0123456789abcdefABCDEF":
                        digits += self.peek()
                        self.pos += 1
                    if not digits or len(digits) > 6:
                        raise self.error("invalid Unicode escape")
                    cp = int(digits, 16)
                    if cp > 0x10FFFF or 0xD800 <= cp <= 0xDFFF:
                        raise self.error("invalid Unicode codepoint")
                    out.append(chr(cp))
                if not out:
                    raise self.error("invalid Unicode escape")
                return "".join(out)
            digits = src[self.pos : self.pos + 4]
            if len(digits) != 4 or any(ch not in "0123456789abcdefABCDEF" for ch in digits):
                raise self.error("invalid Unicode escape")
            self.pos += 4
            cp = int(digits, 16)
            if 0xD800 <= cp <= 0xDFFF:
                raise self.error("invalid Unicode codepoint")
            return chr(cp)
        if c == "\n":
            self.line += 1
            return ""
        if c in ("c", "C", "M"):
            raise self.error("control/meta escapes are not supported")
        return c

    def lex_words(self, spc: bool) -> Token:
        line = self.line
        kind = "words" if self.peek(1) == "w" else "symwords"
        open_ = self.peek(2)
        close = PAIRS[open_]
        self.pos += 3
        words: list[str] = []
        buf: list[str] = []
        depth = 0
        src = self.src
        while True:
            if self.pos >= len(src):
                raise self.error("unterminated list meets end of file")
            c = src[self.pos]
            if c == "\\" and self.pos + 1 < len(src):
                buf.append(src[self.pos + 1])
                if src[self.pos + 1] == "\n":
                    self.line += 1
                self.pos += 2
                continue
            if c == open_:
                depth += 1
            elif c == close:
                if depth == 0:
                    self.pos += 1
                    break
                depth -= 1
            if c in " \t\r\n\f\v":
                if c == "\n":
                    self.line += 1
                if buf:
                    words.append("".join(buf))
                    buf = []
                self.pos += 1
                continue
            buf.append(c)
            self.pos += 1
        if buf:
            words.append("".join(buf))
        return Token(kind, words, line, spc)


# ---------------------------------------------------------------------------------------
# Template scanning (ERB ExplicitScanner, trim_mode "-")

_STAG_RE = re.compile(r"^[ \t]*<%-|<%-|<%%|<%=|<%#|<%", re.M)
# Ruby's StringScanner also matches "^" at the scan position, i.e. right after "%>" or "<%%".
_TRIM_AT_POS_RE = re.compile(r"[ \t]*<%-")
_ETAG_RE = re.compile(r"-%>|%%>|%>")


def tokenize_template(source: str) -> list[Token]:
    """Splits a template into one token stream."""
    toks: list[Token] = []
    pos = 0
    n = len(source)
    line = 1
    text: list[str] = []
    text_line = 1

    def flush_text() -> None:
        nonlocal text
        if text:
            s = "".join(text)
            if s:
                toks.append(Token("text", s, text_line))
                toks.append(Token("semi", None, text_line))
            text = []

    while pos < n:
        m = _TRIM_AT_POS_RE.match(source, pos) or _STAG_RE.search(source, pos)
        if m is None:
            if not text:
                text_line = line
            text.append(source[pos:])
            break
        before = source[pos : m.start()]
        if before:
            if not text:
                text_line = line
            text.append(before)
            line += before.count("\n")
        tag = m.group(0)
        pos = m.end()
        if tag == "<%%":
            if not text:
                text_line = line
            text.append("<%")
            continue
        flush_text()
        if tag.endswith("<%-"):
            kind = "<%"
        else:
            kind = tag
        tag_line = line
        # scan tag body
        content: list[str] = []
        closed = False
        while True:
            em = _ETAG_RE.search(source, pos)
            if em is None:
                break
            content.append(source[pos : em.start()])
            pos = em.end()
            if em.group(0) == "%%>":
                content.append("%>")
                continue
            closed = True
            if em.group(0) == "-%>":
                if source.startswith("\r\n", pos):
                    pos += 2
                    trailing_nl = 1
                elif source.startswith("\n", pos):
                    pos += 1
                    trailing_nl = 1
                else:
                    trailing_nl = 0
            else:
                trailing_nl = 0
            break
        if not closed:
            raise syntax_error("unterminated ERB tag", tag_line)
        body = "".join(content)
        if kind == "<%#":
            pass
        elif kind == "<%=":
            toks.append(Token("out_begin", None, tag_line))
            sub = CodeLexer(body, tag_line).tokenize()
            sub.pop()  # eof
            toks.extend(sub)
            toks.append(Token("out_end", None, tag_line + body.count("\n")))
            toks.append(Token("semi", None, tag_line))
        else:
            toks.append(Token("semi", None, tag_line))
            sub = CodeLexer(body, tag_line).tokenize()
            sub.pop()
            toks.extend(sub)
            toks.append(Token("semi", None, tag_line + body.count("\n")))
        line = tag_line + body.count("\n") + trailing_nl
        text_line = line
    flush_text()
    toks.append(Token("eof", None, line))
    return toks
