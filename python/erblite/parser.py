"""Recursive-descent parser producing the AST in :mod:`erblite.nodes`.

Local variable scoping is resolved here, the same way Ruby does it: a name is a local
variable once the parser has seen an assignment to it (or it is a block parameter, a
``for`` variable or a host variable).
"""

from __future__ import annotations

from collections.abc import Iterable

from . import nodes as N
from .errors import RubyError, syntax_error
from .lexer import RESERVED, Token
from .values import Symbol

MAX_DEPTH = 256

BINOP_PREC = {
    "||": 1,
    "&&": 2,
    "==": 3,
    "!=": 3,
    "<=>": 3,
    "===": 3,
    "<": 4,
    "<=": 4,
    ">": 4,
    ">=": 4,
    "|": 5,
    "^": 5,
    "&": 6,
    "<<": 7,
    ">>": 7,
    "+": 8,
    "-": 8,
    "*": 9,
    "/": 9,
    "%": 9,
}
OP_ASSIGN = {
    "+=": "+",
    "-=": "-",
    "*=": "*",
    "/=": "/",
    "%=": "%",
    "**=": "**",
    "||=": "||",
    "&&=": "&&",
    "<<=": "<<",
    ">>=": ">>",
    "|=": "|",
    "&=": "&",
    "^=": "^",
}
OPERATOR_METHODS = frozenset("+ - * / % ** == != < > <= >= <=> << >> & | ^ ! ===".split())
CLOSER_KWS = frozenset(["end", "else", "elsif", "when", "in", "then", "do"])
MODIFIER_KWS = frozenset(["if", "unless", "while", "until"])
VALUE_START_KINDS = frozenset(["int", "float", "str", "sym", "dsym", "ident", "const", "words", "symwords", "label"])
VALUE_START_KWS = frozenset(["nil", "true", "false", "not", "if", "unless", "while", "until", "case", "for"])
VALUE_START_OPS = frozenset(["(", "[", "-", "!", "~", "+", "::", "{"])
COMMAND_ARG_KINDS = frozenset(["int", "float", "str", "sym", "dsym", "label", "ident", "const"])


def _tok_desc(t: Token) -> str:
    if t.kind == "eof":
        return "end-of-input"
    if t.kind == "out_end":
        return "'%>'"
    if t.kind in ("nl",):
        return "newline"
    if t.kind == "semi":
        return "';'"
    if t.kind == "text":
        return "template text"
    if t.kind in ("op", "kw", "ident", "const"):
        return f"'{t.value}'"
    return t.kind


class Parser:
    def __init__(self, toks: list[Token], host_vars: Iterable[str] = ()) -> None:
        self.toks = toks
        self.pos = 0
        self.scopes: list[set[str]] = [set(host_vars)]
        self.loop_depth = 0
        self.no_cmd = False
        self.no_do = False
        self.depth = 0

    # -- token helpers ---------------------------------------------------------------------

    @property
    def tok(self) -> Token:
        return self.toks[self.pos]

    def peek(self, k: int = 1) -> Token:
        p = self.pos + k
        if p < len(self.toks):
            return self.toks[p]
        return self.toks[-1]

    def advance(self) -> Token:
        t = self.toks[self.pos]
        if t.kind != "eof":
            self.pos += 1
        return t

    def is_op(self, v: str) -> bool:
        t = self.toks[self.pos]
        return t.kind == "op" and t.value == v

    def is_kw(self, v: str) -> bool:
        t = self.toks[self.pos]
        return t.kind == "kw" and t.value == v

    def accept_op(self, v: str) -> bool:
        if self.is_op(v):
            self.pos += 1
            return True
        return False

    def accept_kw(self, v: str) -> bool:
        if self.is_kw(v):
            self.pos += 1
            return True
        return False

    def unexpected(self, t: Token | None = None) -> RubyError:
        t = t or self.tok
        return syntax_error(f"syntax error, unexpected {_tok_desc(t)}", t.line)

    def expect_op(self, v: str) -> Token:
        if not self.is_op(v):
            t = self.tok
            raise syntax_error(f"syntax error, unexpected {_tok_desc(t)}, expecting '{v}'", t.line)
        return self.advance()

    def expect_kw(self, v: str) -> Token:
        if not self.is_kw(v):
            t = self.tok
            raise syntax_error(f"syntax error, unexpected {_tok_desc(t)}, expecting '{v}'", t.line)
        return self.advance()

    def skip_nl(self) -> None:
        while self.toks[self.pos].kind == "nl":
            self.pos += 1

    def skip_terms(self) -> bool:
        found = False
        while self.toks[self.pos].kind in ("nl", "semi"):
            self.pos += 1
            found = True
        return found

    def enter(self) -> None:
        self.depth += 1
        if self.depth > MAX_DEPTH:
            raise RubyError("LimitError", "template nesting too deep", self.tok.line)

    def leave(self) -> None:
        self.depth -= 1

    # -- scopes ----------------------------------------------------------------------------

    def lookup(self, name: str) -> int | None:
        for i in range(len(self.scopes) - 1, -1, -1):
            if name in self.scopes[i]:
                return len(self.scopes) - 1 - i
        return None

    def declare(self, name: str) -> int:
        hops = self.lookup(name)
        if hops is not None:
            return hops
        self.scopes[-1].add(name)
        return 0

    # -- program / statements --------------------------------------------------------------

    def parse_program(self) -> N.Seq:
        body = self.parse_statements()
        if self.tok.kind != "eof":
            raise self.unexpected()
        return body

    def at_closer(self) -> bool:
        t = self.tok
        if t.kind in ("eof", "out_end"):
            return True
        if t.kind == "kw" and t.value in CLOSER_KWS:
            return True
        return t.kind == "op" and t.value in ("}", ")")

    def parse_statements(self) -> N.Seq:
        self.enter()
        saved = (self.no_cmd, self.no_do)
        self.no_cmd = self.no_do = False
        line = self.tok.line
        stmts: list[N.Node] = []
        while True:
            self.skip_terms()
            if self.at_closer():
                break
            stmts.append(self.parse_statement())
            if self.tok.kind in ("nl", "semi"):
                continue
            if self.at_closer():
                break
            raise self.unexpected()
        self.no_cmd, self.no_do = saved
        self.leave()
        return N.Seq(line, stmts)

    def parse_statement(self) -> N.Node:
        t = self.tok
        if t.kind == "text":
            self.advance()
            return N.Text(t.line, t.value)
        if t.kind == "out_begin":
            self.advance()
            body = self.parse_statements()
            if self.tok.kind != "out_end":
                raise self.unexpected()
            self.advance()
            return N.Output(t.line, body)
        node = self.parse_simple()
        while self.tok.kind == "kw" and self.tok.value in MODIFIER_KWS:
            kw = self.advance()
            self.skip_nl()
            if kw.value in ("while", "until"):
                saved = self.no_do
                self.no_do = False
                cond = self.parse_expr()
                self.no_do = saved
                node = N.While(kw.line, cond, node, kw.value == "until")
            else:
                cond = self.parse_expr()
                if kw.value == "if":
                    node = N.If(kw.line, cond, node, None)
                else:
                    node = N.If(kw.line, cond, None, node)
        return node

    def parse_simple(self) -> N.Node:
        node = self.parse_expr()
        if self.is_op(","):
            if isinstance(node, (N.LAsgn, N.IndexAsgn, N.AttrAsgn)):
                # a = 1, 2  =>  a = [1, 2]
                vals = [node.value]
                while self.accept_op(","):
                    self.skip_nl()
                    vals.append(self.parse_arg_expr())
                node.value = N.ArrayLit(node.value.line, vals)
                return node
            if self.assignable(node):
                return self.parse_mlhs(node)
        return node

    def parse_mlhs(self, first: N.Node) -> N.Node:
        line = first.line
        items = [first]
        while self.accept_op(","):
            self.skip_nl()
            if self.is_op("="):
                break
            items.append(self.parse_primary_postfix(allow_assign=False))
        if not self.is_op("="):
            raise self.unexpected()
        self.advance()
        self.skip_nl()
        targets = [self.to_target(n) for n in items]
        values = [self.parse_arg_expr()]
        while self.accept_op(","):
            self.skip_nl()
            values.append(self.parse_arg_expr())
        return N.MultiAsgn(line, targets, values)

    # -- expressions -----------------------------------------------------------------------

    def parse_expr(self) -> N.Node:
        left = self.parse_not_expr()
        while self.tok.kind == "kw" and self.tok.value in ("and", "or"):
            kw = self.advance()
            self.skip_nl()
            right = self.parse_not_expr()
            left = N.And(kw.line, left, right) if kw.value == "and" else N.Or(kw.line, left, right)
        return left

    def parse_not_expr(self) -> N.Node:
        if self.is_kw("not"):
            t = self.advance()
            return N.Not(t.line, self.parse_not_expr())
        return self.parse_ternary()

    def parse_arg_expr(self) -> N.Node:
        if self.is_kw("not"):
            t = self.advance()
            return N.Not(t.line, self.parse_arg_expr())
        return self.parse_ternary()

    def parse_ternary(self) -> N.Node:
        cond = self.parse_range()
        if self.is_op("?"):
            q = self.advance()
            self.skip_nl()
            a = self.parse_ternary()
            self.skip_nl()
            self.expect_op(":")
            self.skip_nl()
            b = self.parse_ternary()
            return N.If(q.line, cond, a, b)
        return cond

    def can_start_value(self) -> bool:
        t = self.tok
        if t.kind in VALUE_START_KINDS:
            return True
        if t.kind == "kw":
            return t.value in VALUE_START_KWS
        if t.kind == "op":
            return t.value in VALUE_START_OPS
        return False

    def parse_range(self) -> N.Node:
        left = self.parse_binary(1)
        if self.is_op("..") or self.is_op("..."):
            t = self.advance()
            right = None
            if self.can_start_value() and not self.is_op("{"):
                right = self.parse_binary(1)
            return N.RangeLit(t.line, left, right, t.value == "...")
        return left

    def parse_binary(self, min_prec: int) -> N.Node:
        left = self.parse_uminus()
        while True:
            t = self.tok
            if t.kind != "op":
                break
            prec = BINOP_PREC.get(t.value)
            if prec is None or prec < min_prec:
                break
            self.advance()
            self.skip_nl()
            right = self.parse_binary(prec + 1)
            op = t.value
            if op == "||":
                left = N.Or(t.line, left, right)
            elif op == "&&":
                left = N.And(t.line, left, right)
            elif op == "!=":
                left = N.Not(t.line, N.Call(t.line, left, "==", [right]))
            elif op == "<<" and self.shovel_target(left):
                left = N.Shovel(t.line, left, right)
            else:
                left = N.Call(t.line, left, op, [right])
            if prec == 3 and self.tok.kind == "op" and BINOP_PREC.get(self.tok.value) == 3:
                raise self.unexpected()
        return left

    def parse_uminus(self) -> N.Node:
        if self.is_op("-"):
            t = self.advance()
            operand = self.parse_uminus()
            return N.Call(t.line, operand, "-@")
        return self.parse_pow()

    def parse_pow(self) -> N.Node:
        t = self.tok
        if t.kind in ("int", "float") and t.neg and self.peek().kind == "op" and self.peek().value == "**":
            # -2 ** 2 is -(2 ** 2)
            self.advance()
            self.advance()
            self.skip_nl()
            exp = self.parse_uminus()
            return N.Call(t.line, N.Call(t.line, N.Literal(t.line, -t.value), "**", [exp]), "-@")
        base = self.parse_unary()
        if self.is_op("**"):
            op = self.advance()
            self.skip_nl()
            exp = self.parse_uminus()
            return N.Call(op.line, base, "**", [exp])
        return base

    def parse_unary(self) -> N.Node:
        t = self.tok
        if t.kind == "op" and t.value in ("!", "~", "+", "-", "&"):
            if t.value == "&":
                raise self.unexpected()
            self.enter()
            self.advance()
            operand = self.parse_unary()
            self.leave()
            if t.value == "!":
                return N.Not(t.line, operand)
            return N.Call(t.line, operand, {"~": "~", "+": "+@", "-": "-@"}[t.value])
        return self.parse_primary_postfix(allow_assign=True)

    def parse_primary_postfix(self, allow_assign: bool) -> N.Node:
        node = self.parse_primary()
        node = self.parse_postfix(node)
        if allow_assign:
            node = self.parse_assignment(node)
        return node

    # -- assignment ------------------------------------------------------------------------

    def assignable(self, node: N.Node) -> bool:
        if isinstance(node, N.LVar):
            return True
        if isinstance(node, N.Call) and node.block is None and node.block_sym is None:
            if node.name == "[]":
                return node.recv is not None
            if node.args or node.has_parens:
                return False
            if node.recv is None:
                return node.name[-1] not in "?!"
            name = node.name
            return (name[0] == "_" or name[0].islower()) and name[-1] not in "?!=" and name.isidentifier()
        return False

    @staticmethod
    def shovel_target(node: N.Node) -> bool:
        if isinstance(node, (N.LVar, N.Shovel)):
            return True
        return isinstance(node, N.Call) and node.name == "[]" and node.recv is not None and node.block is None

    def to_target(self, node: N.Node) -> N.Node:
        if isinstance(node, N.LVar):
            return node
        if isinstance(node, N.Call) and node.recv is None:
            if node.name in RESERVED:
                raise syntax_error(f"syntax error, unexpected '{node.name}'", node.line)
            hops = self.declare(node.name)
            return N.LVar(node.line, node.name, hops)
        return node

    def parse_assignment(self, node: N.Node) -> N.Node:
        t = self.tok
        if t.kind != "op":
            return node
        if t.value == "=":
            if isinstance(node, N.Const):
                raise syntax_error("dynamic constant assignment", t.line)
            if not self.assignable(node):
                return node
            self.advance()
            self.skip_nl()
            target = self.to_target(node)
            value = self.parse_assign_rhs()
            if isinstance(target, N.LVar):
                return N.LAsgn(t.line, target.name, target.hops, value)
            assert isinstance(target, N.Call)
            if target.name == "[]":
                return N.IndexAsgn(t.line, target.recv, target.args, value)
            return N.AttrAsgn(t.line, target.recv, target.name, value, target.safe)
        op = OP_ASSIGN.get(t.value)
        if op is not None:
            if isinstance(node, N.Const):
                raise syntax_error("dynamic constant assignment", t.line)
            if not self.assignable(node):
                return node
            self.advance()
            self.skip_nl()
            target = self.to_target(node)
            value = self.parse_assign_rhs()
            return N.OpAsgn(t.line, target, op, value)
        return node

    def parse_assign_rhs(self) -> N.Node:
        if self.is_kw("not"):
            t = self.advance()
            return N.Not(t.line, self.parse_assign_rhs())
        return self.parse_ternary()

    # -- postfix ---------------------------------------------------------------------------

    def parse_postfix(self, node: N.Node) -> N.Node:
        while True:
            t = self.tok
            if t.kind == "nl":
                nxt = self.peek()
                if nxt.kind == "op" and nxt.value in (".", "&."):
                    self.advance()
                    continue
                break
            if t.kind != "op":
                break
            if t.value in (".", "&."):
                self.advance()
                self.skip_nl()
                nt = self.tok
                if nt.kind in ("ident", "const", "kw"):
                    name = nt.value
                elif nt.kind == "op" and nt.value in OPERATOR_METHODS:
                    name = nt.value
                elif nt.kind == "op" and nt.value == "[" and self.peek().is_op("]"):
                    self.advance()
                    name = "[]"
                else:
                    raise self.unexpected(nt)
                self.advance()
                node = self.parse_call_rest(node, name, nt.line, t.value == "&.")
                continue
            if t.value == "::":
                self.advance()
                nt = self.tok
                if nt.kind == "const" and not (self.peek().is_op("(") and not self.peek().spc):
                    self.advance()
                    node = N.Const(nt.line, node, nt.value)
                    continue
                if nt.kind in ("ident", "const"):
                    self.advance()
                    node = self.parse_call_rest(node, nt.value, nt.line, False)
                    continue
                raise self.unexpected(nt)
            if t.value == "[":
                self.advance()
                args, bsym = self.parse_paren_args("]")
                if bsym is not None:
                    raise self.unexpected()
                node = N.Call(t.line, node, "[]", args)
                continue
            break
        return node

    def parse_call_rest(self, recv: N.Node | None, name: str, line: int, safe: bool) -> N.Call:
        call = N.Call(line, recv, name, safe=safe)
        t = self.tok
        if t.kind == "op" and t.value == "(" and not t.spc:
            self.advance()
            call.args, call.block_sym = self.parse_paren_args(")")
            call.has_parens = True
        elif self.can_command_args():
            call.args, call.block_sym = self.parse_command_args()
            if self.is_kw("do") and not self.no_do and not self.no_cmd and call.block_sym is None:
                call.block = self.parse_block()
            return call
        if call.block_sym is None:
            if self.is_op("{") or (self.is_kw("do") and not self.no_do and not self.no_cmd):
                call.block = self.parse_block()
        return call

    def can_command_args(self) -> bool:
        if self.no_cmd:
            return False
        t = self.tok
        if not t.spc:
            return False
        if t.kind in COMMAND_ARG_KINDS:
            return True
        if t.kind == "kw":
            return t.value in ("nil", "true", "false")
        if t.kind == "op":
            nxt = self.peek()
            if t.value in ("(", "["):
                return True
            if t.value == "-" and nxt.kind in ("int", "float") and not nxt.spc:
                return True
            if t.value == "&" and nxt.kind == "sym" and not nxt.spc:
                return True
            if t.value == "!" and not nxt.spc:
                return True
        return False

    def parse_command_args(self) -> tuple[list[N.Node], str | None]:
        saved = self.no_cmd
        self.no_cmd = True
        t = self.tok
        if t.kind == "op" and t.value == "-":
            # "foo -1": the sign belongs to the literal
            nxt = self.peek()
            nxt.value = -nxt.value
            nxt.neg = True
            nxt.spc = True
            self.advance()
        args, bsym = self.parse_arg_list(None)
        self.no_cmd = saved
        return args, bsym

    def parse_paren_args(self, close: str) -> tuple[list[N.Node], str | None]:
        self.enter()
        saved = (self.no_cmd, self.no_do)
        self.no_cmd = self.no_do = False
        self.skip_nl()
        args, bsym = self.parse_arg_list(close)
        self.no_cmd, self.no_do = saved
        self.leave()
        return args, bsym

    def parse_arg_list(self, close: str | None) -> tuple[list[N.Node], str | None]:
        args: list[N.Node] = []
        pairs: list[tuple[N.Node, N.Node]] = []
        pairs_line = self.tok.line
        bsym: str | None = None
        while True:
            if close is not None:
                self.skip_nl()
                if self.is_op(close):
                    self.advance()
                    break
            if bsym is not None:
                raise self.unexpected()
            t = self.tok
            if t.kind == "label":
                self.advance()
                self.skip_nl()
                if not pairs:
                    pairs_line = t.line
                pairs.append((N.Literal(t.line, Symbol(t.value)), self.parse_arg_expr()))
            elif t.kind == "str" and self.peek().is_op(":") and not self.peek().spc:
                key = self.parse_string_token(self.advance())
                self.advance()
                self.skip_nl()
                if not pairs:
                    pairs_line = t.line
                pairs.append((N.DSym(t.line, self._as_str(key)), self.parse_arg_expr()))
            elif t.kind == "op" and t.value == "&":
                self.advance()
                st = self.tok
                if st.kind != "sym":
                    raise self.unexpected(st)
                self.advance()
                bsym = st.value
            elif t.kind == "op" and t.value in ("*", "**"):
                raise syntax_error("splat arguments are not supported", t.line)
            else:
                e = self.parse_arg_expr()
                if self.is_op("=>"):
                    self.advance()
                    self.skip_nl()
                    if not pairs:
                        pairs_line = t.line
                    pairs.append((e, self.parse_arg_expr()))
                else:
                    if pairs:
                        raise self.unexpected()
                    args.append(e)
            if close is not None:
                self.skip_nl()
            if self.accept_op(","):
                self.skip_nl()
                if close is None and self.tok.kind in ("eof", "out_end"):
                    raise self.unexpected()
                continue
            if close is not None:
                self.expect_op(close)
            break
        if pairs:
            args.append(N.HashLit(pairs_line, pairs))
        return args, bsym

    @staticmethod
    def _as_str(node: N.Node) -> N.Str:
        if isinstance(node, N.Str):
            return node
        assert isinstance(node, N.Literal)
        return N.Str(node.line, [node.value])

    # -- blocks ----------------------------------------------------------------------------

    def parse_block(self) -> N.BlockNode:
        t = self.advance()
        brace = t.value == "{"
        self.enter()
        self.scopes.append(set())
        params: list = []
        if self.is_op("||"):
            self.advance()
        elif self.is_op("|"):
            self.advance()
            params = self.parse_block_params()
            self.expect_op("|")
        for name in _param_names(params):
            self.scopes[-1].add(name)
        self.loop_depth += 1
        body = self.parse_statements()
        self.loop_depth -= 1
        if brace:
            self.expect_op("}")
        else:
            self.expect_kw("end")
        self.scopes.pop()
        self.leave()
        return N.BlockNode(t.line, params, body)

    def parse_block_params(self) -> list:
        params: list = []
        while True:
            self.skip_nl()
            t = self.tok
            if t.kind == "ident":
                if t.value in RESERVED or t.value[-1] in "?!":
                    raise self.unexpected(t)
                self.advance()
                params.append(t.value)
            elif t.kind == "op" and t.value == "(":
                self.advance()
                params.append(self.parse_block_params())
                self.expect_op(")")
            else:
                raise self.unexpected(t)
            self.skip_nl()
            if not self.accept_op(","):
                break
        return params

    # -- primary ---------------------------------------------------------------------------

    def parse_string_token(self, t: Token) -> N.Node:
        parts: list = []
        for p in t.value:
            if isinstance(p, str):
                if parts and isinstance(parts[-1], str):
                    parts[-1] += p
                else:
                    parts.append(p)
            else:
                parts.append(self.parse_interpolation(p))
        if all(isinstance(p, str) for p in parts):
            return N.Literal(t.line, "".join(parts))
        return N.Str(t.line, parts)

    def parse_interpolation(self, toks: list[Token]) -> N.Node:
        saved = (self.toks, self.pos)
        self.toks, self.pos = toks, 0
        body = self.parse_statements()
        if self.tok.kind != "eof":
            raise self.unexpected()
        self.toks, self.pos = saved
        return body

    def parse_primary(self) -> N.Node:
        t = self.tok
        k = t.kind
        if k == "int" or k == "float":
            self.advance()
            return N.Literal(t.line, t.value)
        if k == "str":
            self.advance()
            node = self.parse_string_token(t)
            # adjacent string literals concatenate
            while self.tok.kind == "str" and not (self.peek().is_op(":") and not self.peek().spc):
                nxt = self.parse_string_token(self.advance())
                a, b = self._as_str(node), self._as_str(nxt)
                node = N.Str(t.line, a.parts + b.parts)
            if isinstance(node, N.Str) and all(isinstance(p, str) for p in node.parts):
                node = N.Literal(t.line, "".join(node.parts))
            return node
        if k == "sym":
            self.advance()
            return N.Literal(t.line, Symbol(t.value))
        if k == "dsym":
            self.advance()
            s = self.parse_string_token(t)
            if isinstance(s, N.Literal):
                return N.Literal(t.line, Symbol(s.value))
            return N.DSym(t.line, self._as_str(s))
        if k == "words":
            self.advance()
            return N.ArrayLit(t.line, [N.Literal(t.line, w) for w in t.value])
        if k == "symwords":
            self.advance()
            return N.ArrayLit(t.line, [N.Literal(t.line, Symbol(w)) for w in t.value])
        if k == "ident":
            return self.parse_identifier()
        if k == "const":
            self.advance()
            nxt = self.tok
            if nxt.kind == "op" and nxt.value == "(" and not nxt.spc:
                return self.parse_call_rest(None, t.value, t.line, False)
            return N.Const(t.line, None, t.value)
        if k == "kw":
            return self.parse_keyword()
        if k == "op":
            v = t.value
            if v == "(":
                self.advance()
                body = self.parse_statements()
                self.expect_op(")")
                if len(body.stmts) == 1:
                    return body.stmts[0]
                return body
            if v == "[":
                self.advance()
                elems, bsym = self.parse_paren_args("]")
                if bsym is not None:
                    raise self.unexpected()
                return N.ArrayLit(t.line, elems)
            if v == "{":
                return self.parse_hash()
            if v in ("-", "!", "~", "+"):
                return self.parse_unary()
            if v in ("..", "..."):
                self.advance()
                hi = self.parse_binary(1)
                return N.RangeLit(t.line, None, hi, v == "...")
        if k == "label":
            raise syntax_error(f"syntax error, unexpected label '{t.value}:'", t.line)
        raise self.unexpected()

    def parse_identifier(self) -> N.Node:
        t = self.advance()
        name = t.value
        if name in RESERVED:
            raise syntax_error(f"'{name}' is not supported", t.line)
        nxt = self.tok
        if nxt.kind == "op" and nxt.value == "(" and not nxt.spc:
            return self.parse_call_rest(None, name, t.line, False)
        hops = self.lookup(name) if name[-1] not in "?!" else None
        if hops is not None:
            return N.LVar(t.line, name, hops)
        return self.parse_call_rest(None, name, t.line, False)

    def parse_hash(self) -> N.Node:
        t = self.advance()
        self.enter()
        saved = (self.no_cmd, self.no_do)
        self.no_cmd = self.no_do = False
        pairs: list[tuple[N.Node, N.Node]] = []
        while True:
            self.skip_nl()
            if self.accept_op("}"):
                break
            kt = self.tok
            if kt.kind == "label":
                self.advance()
                self.skip_nl()
                key: N.Node = N.Literal(kt.line, Symbol(kt.value))
            elif kt.kind == "str" and self.peek().is_op(":") and not self.peek().spc:
                s = self.parse_string_token(self.advance())
                self.advance()
                self.skip_nl()
                key = N.DSym(kt.line, self._as_str(s))
            else:
                key = self.parse_arg_expr()
                self.skip_nl()
                self.expect_op("=>")
                self.skip_nl()
            value = self.parse_arg_expr()
            pairs.append((key, value))
            self.skip_nl()
            if self.accept_op(","):
                continue
            self.skip_nl()
            self.expect_op("}")
            break
        self.no_cmd, self.no_do = saved
        self.leave()
        return N.HashLit(t.line, pairs)

    def parse_keyword(self) -> N.Node:
        t = self.tok
        v = t.value
        if v == "nil":
            self.advance()
            return N.Literal(t.line, None)
        if v == "true":
            self.advance()
            return N.Literal(t.line, True)
        if v == "false":
            self.advance()
            return N.Literal(t.line, False)
        if v == "if" or v == "unless":
            return self.parse_if()
        if v == "while" or v == "until":
            return self.parse_while()
        if v == "for":
            return self.parse_for()
        if v == "case":
            return self.parse_case()
        if v == "break" or v == "next":
            return self.parse_break_next()
        if v == "not":
            self.advance()
            return N.Not(t.line, self.parse_expr())
        if v in ("begin", "self"):
            raise syntax_error(f"'{v}' is not supported", t.line)
        raise self.unexpected()

    def parse_then(self) -> None:
        found = self.skip_terms()
        if self.accept_kw("then"):
            return
        if not found:
            raise self.unexpected()

    def parse_if(self) -> N.Node:
        t = self.advance()
        self.enter()
        saved = self.no_do
        self.no_do = False
        cond = self.parse_expr()
        self.no_do = saved
        self.parse_then()
        body = self.parse_statements()
        if t.value == "unless":
            else_ = None
            if self.accept_kw("else"):
                else_ = self.parse_statements()
            self.expect_kw("end")
            self.leave()
            return N.If(t.line, cond, else_, body)
        branches = [(t.line, cond, body)]
        else_ = None
        while self.is_kw("elsif"):
            et = self.advance()
            c = self.parse_expr()
            self.parse_then()
            b = self.parse_statements()
            branches.append((et.line, c, b))
        if self.accept_kw("else"):
            else_ = self.parse_statements()
        self.expect_kw("end")
        node: N.Node | None = else_
        for line, c, b in reversed(branches):
            node = N.If(line, c, b, node)
        self.leave()
        assert node is not None
        return node

    def parse_while(self) -> N.Node:
        t = self.advance()
        self.enter()
        saved = self.no_do
        self.no_do = True
        cond = self.parse_expr()
        self.no_do = saved
        if not self.accept_kw("do"):
            if not self.skip_terms():
                raise self.unexpected()
        self.loop_depth += 1
        body = self.parse_statements()
        self.loop_depth -= 1
        self.expect_kw("end")
        self.leave()
        return N.While(t.line, cond, body, t.value == "until")

    def parse_for(self) -> N.Node:
        t = self.advance()
        self.enter()
        names = []
        while True:
            nt = self.tok
            if nt.kind != "ident" or nt.value in RESERVED or nt.value[-1] in "?!":
                raise self.unexpected(nt)
            self.advance()
            names.append(nt)
            if not self.accept_op(","):
                break
        targets = [N.LVar(nt.line, nt.value, self.declare(nt.value)) for nt in names]
        self.expect_kw("in")
        saved = self.no_do
        self.no_do = True
        it = self.parse_expr()
        self.no_do = saved
        if not self.accept_kw("do"):
            if not self.skip_terms():
                raise self.unexpected()
        self.loop_depth += 1
        body = self.parse_statements()
        self.loop_depth -= 1
        self.expect_kw("end")
        self.leave()
        return N.For(t.line, targets, it, body)

    def parse_case(self) -> N.Node:
        t = self.advance()
        self.enter()
        subject = None
        if self.tok.kind not in ("nl", "semi") and not self.is_kw("when"):
            subject = self.parse_expr()
        self.skip_terms()
        whens: list[tuple[list[N.Node], N.Node]] = []
        while self.is_kw("when"):
            self.advance()
            conds = [self.parse_ternary()]
            while self.accept_op(","):
                self.skip_nl()
                conds.append(self.parse_ternary())
            self.parse_then()
            body = self.parse_statements()
            whens.append((conds, body))
        if not whens:
            raise self.unexpected()
        else_ = None
        if self.accept_kw("else"):
            else_ = self.parse_statements()
        self.expect_kw("end")
        self.leave()
        return N.Case(t.line, subject, whens, else_)

    def parse_break_next(self) -> N.Node:
        t = self.advance()
        if self.loop_depth == 0:
            raise syntax_error(f"Invalid {t.value}", t.line)
        value = None
        nt = self.tok
        if self.can_start_value() and not (nt.kind == "kw" and nt.value in MODIFIER_KWS) and not nt.is_op("{"):
            value = self.parse_ternary()
            if self.is_op(","):
                vals = [value]
                while self.accept_op(","):
                    self.skip_nl()
                    vals.append(self.parse_ternary())
                value = N.ArrayLit(value.line, vals)
        if t.value == "break":
            return N.Break(t.line, value)
        return N.Next(t.line, value)


def _param_names(params: list) -> list[str]:
    out: list[str] = []
    for p in params:
        if isinstance(p, list):
            out.extend(_param_names(p))
        else:
            out.append(p)
    return out


def parse(toks: list[Token], host_vars: Iterable[str] = ()) -> N.Seq:
    return Parser(toks, host_vars).parse_program()
