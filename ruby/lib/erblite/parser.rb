# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

# frozen_string_literal: true

module ERBLite
  # Recursive-descent parser producing the AST in Nodes. It resolves local
  # variable scopes while parsing, the same way Ruby does: a name is a local
  # variable once an assignment to it has been seen.
  class Parser
    include Nodes

    ASSIGN_OPS = %w[= += -= *= /= %= **= ||= &&= <<= >>= |= &= ^=].to_h { |o| [o, true] }.freeze
    MODIFIERS = %w[if unless while until].freeze
    EQUALITY_OPS = %w[== != <=> ===].freeze
    OPERATOR_METHODS = %w[+ - * / % ** == != < > <= >= <=> === << >> & | ^ ! ~ []].to_h { |o| [o, true] }.freeze
    MAX_DEPTH = 256

    def self.parse(tokens, filename, host_names)
      new(tokens, filename, host_names).parse_program
    end

    def initialize(tokens, filename, host_names)
      @toks = tokens
      @pos = 0
      @filename = filename
      @scopes = [host_names.to_h { |n| [n, true] }]
      @ctx = []
      @no_do = false
      @no_command = false
      @depth = 0
    end

    def parse_program
      body = parse_statements
      t = peek
      unexpected(t) unless t.type == :eof
      body
    end

    private

    # -- token helpers ---------------------------------------------------------

    def peek(n = 0)
      @toks[@pos + n] || @toks.last
    end

    def advance
      t = @toks[@pos]
      @pos += 1 if @pos < @toks.length - 1
      t
    end

    def op?(value, t = peek)
      t.type == :op && t.value == value
    end

    def kw?(value, t = peek)
      t.type == :kw && t.value == value
    end

    def accept_op(value)
      return nil unless op?(value)

      advance
    end

    def expect_op(value)
      t = peek
      unexpected(t, "expected '#{value}'") unless op?(value, t)
      advance
    end

    def expect_kw(value)
      t = peek
      unexpected(t, "expected '#{value}'") unless kw?(value, t)
      advance
    end

    def term?(t = peek)
      t.type == :nl || t.type == :semi || t.type == :tagend
    end

    def skip_terms
      advance while term?
    end

    def skip_nl
      advance while peek.type == :nl
    end

    def error(msg, line)
      raise Error.new("SyntaxError", msg, @filename, line)
    end

    def describe(t)
      case t.type
      when :eof then "end of template"
      when :tagend then "end of tag (%>)"
      when :oend then "end of output tag (%>)"
      when :nl then "newline"
      when :text then "template text"
      when :ostart then "output tag (<%=)"
      when :str then "string literal"
      when :label then "label '#{t.value}:'"
      else "'#{t.value}'"
      end
    end

    def unexpected(t, extra = nil)
      if t.type == :kw && CodeLexer::RESERVED.include?(t.value)
        error("'#{t.value}' is not supported in ERBLite", t.line)
      end
      error("syntax error, unexpected #{describe(t)}#{extra ? " (#{extra})" : ""}", t.line)
    end

    def nest
      @depth += 1
      if @depth > MAX_DEPTH
        raise Error.new("LimitError", "template nesting is too deep", @filename, peek.line)
      end
      yield
    ensure
      @depth -= 1
    end

    def with_flags(no_do: false, no_command: false)
      saved = [@no_do, @no_command]
      @no_do = no_do
      @no_command = no_command
      yield
    ensure
      @no_do, @no_command = saved
    end

    # -- scopes ----------------------------------------------------------------

    def lookup(name)
      i = @scopes.length - 1
      while i >= 0
        return @scopes.length - 1 - i if @scopes[i][name]

        i -= 1
      end
      nil
    end

    def declare(name)
      d = lookup(name)
      return d if d

      @scopes.last[name] = true
      0
    end

    # -- statements ------------------------------------------------------------

    def stmts_end?(t)
      case t.type
      when :eof, :oend then true
      when :kw then %w[end else elsif when in then do].include?(t.value)
      when :op then t.value == "}" || t.value == ")" || t.value == "]"
      else false
      end
    end

    def parse_statements
      line = peek.line
      stmts = []
      loop do
        skip_terms
        t = peek
        break if stmts_end?(t)

        s = parse_statement
        stmts << s
        next if s.is_a?(Text) || s.is_a?(Output)

        t = peek
        next if term?(t) || stmts_end?(t)

        unexpected(t)
      end
      Seq.new(stmts, line)
    end

    def parse_statement
      t = peek
      case t.type
      when :text
        advance
        Text.new(t.value, t.line)
      when :ostart
        advance
        # Like ERB's `_erbout << ((code).to_s)`, the tag may hold several
        # statements; the value of the last one is output.
        e = with_flags { parse_statements }
        unexpected(peek) unless peek.type == :oend
        advance
        Output.new(e, t.line)
      else
        parse_stmt_mod
      end
    end

    def parse_stmt_mod
      s = parse_simple
      while (t = peek).type == :kw && MODIFIERS.include?(t.value)
        advance
        c = parse_expr
        s = case t.value
            when "if" then If.new(c, s, nil, t.line)
            when "unless" then If.new(c, nil, s, t.line)
            when "while" then While.new(c, s, false, t.line)
            else While.new(c, s, true, t.line)
            end
      end
      s
    end

    def expr_start?(t)
      case t.type
      when :int, :float, :str, :sym, :dsym, :words, :symwords, :ident, :const, :label then true
      when :kw then %w[nil true false not if unless while until case for begin self].include?(t.value)
      when :op then ["(", "[", "{", "-", "!", "~", "+", "::", "..", "..."].include?(t.value)
      else false
      end
    end

    def parse_simple
      t = peek
      if kw?("break") || kw?("next")
        advance
        kind = @ctx.last
        error("Invalid #{t.value}", t.line) unless kind
        nt = peek
        value = expr_start?(nt) && !(nt.type == :kw && MODIFIERS.include?(nt.value)) ? parse_expr : nil
        return t.value == "break" ? Break.new(value, kind, t.line) : Next.new(value, kind, t.line)
      end
      return parse_masgn if t.type == :ident && op?(",", peek(1)) && masgn_ahead?

      e = parse_expr
      if e.is_a?(LAsgn) && op?(",")
        values = [e.value]
        while accept_op(",")
          skip_nl
          values << parse_ternary
        end
        e = LAsgn.new(e.name, e.depth, Arr.new(values, e.line), e.line)
      end
      e
    end

    def masgn_ahead?
      i = @pos
      loop do
        return false unless @toks[i]&.type == :ident

        i += 1
        t = @toks[i]
        return true if t&.type == :op && t.value == "="
        return false unless t&.type == :op && t.value == ","

        i += 1
      end
    end

    def parse_masgn
      line = peek.line
      names = [advance.value]
      while accept_op(",")
        names << advance.value
      end
      expect_op("=")
      skip_nl
      targets = names.map { |n| [n, declare(n)] }
      values = [parse_ternary]
      while accept_op(",")
        skip_nl
        values << parse_ternary
      end
      MAsgn.new(targets, values, line)
    end

    # -- expressions -----------------------------------------------------------

    def parse_expr
      left = parse_not_expr
      while (t = peek).type == :kw && (t.value == "and" || t.value == "or")
        advance
        skip_nl
        right = parse_not_expr
        left = t.value == "and" ? And.new(left, right, t.line) : Or.new(left, right, t.line)
      end
      left
    end

    def parse_not_expr
      if kw?("not")
        t = advance
        return Not.new(nest { parse_not_expr }, t.line)
      end
      parse_ternary
    end

    def parse_ternary
      c = parse_range
      return c unless op?("?")

      t = advance
      skip_nl
      a = nest { parse_ternary }
      skip_nl
      expect_op(":")
      skip_nl
      b = nest { parse_ternary }
      If.new(c, a, b, t.line)
    end

    def parse_range
      if op?("..") || op?("...")
        t = advance
        return RangeLit.new(Lit.new(nil, t.line), parse_oror, t.value == "...", t.line)
      end
      l = parse_oror
      if op?("..") || op?("...")
        t = advance
        r = expr_start?(peek) && peek.type != :label ? parse_oror : nil
        return RangeLit.new(l, r, t.value == "...", t.line)
      end
      l
    end

    def parse_oror
      l = parse_andand
      while op?("||")
        t = advance
        l = Or.new(l, parse_andand, t.line)
      end
      l
    end

    def parse_andand
      l = parse_equality
      while op?("&&")
        t = advance
        l = And.new(l, parse_equality, t.line)
      end
      l
    end

    def parse_equality
      l = parse_compare
      t = peek
      if t.type == :op && EQUALITY_OPS.include?(t.value)
        advance
        r = parse_compare
        l = if t.value == "!="
              Not.new(Call.new(l, "==", [r], nil, false, t.line), t.line)
            else
              Call.new(l, t.value, [r], nil, false, t.line)
            end
        t2 = peek
        unexpected(t2) if t2.type == :op && EQUALITY_OPS.include?(t2.value)
      end
      l
    end

    def binary_loop(ops)
      l = yield
      while (t = peek).type == :op && ops.include?(t.value)
        advance
        l = Call.new(l, t.value, [yield], nil, false, t.line)
      end
      l
    end

    def parse_compare = binary_loop(%w[< <= > >=]) { parse_bor }
    def parse_bor = binary_loop(%w[| ^]) { parse_band }
    def parse_band = binary_loop(%w[&]) { parse_shift }
    def parse_shift = binary_loop(%w[<< >>]) { parse_add }
    def parse_add = binary_loop(%w[+ -]) { parse_mul }
    def parse_mul = binary_loop(%w[* / %]) { parse_uminus }

    def parse_uminus
      if op?("-")
        t = advance
        return Call.new(nest { parse_uminus }, "-@", [], nil, false, t.line)
      end
      parse_pow
    end

    def parse_pow
      t = peek
      if (t.type == :int || t.type == :float) && t.neg && op?("**", peek(1))
        advance
        advance
        exp = nest { parse_uminus }
        base = Lit.new(-t.value, t.line)
        return Call.new(Call.new(base, "**", [exp], nil, false, t.line), "-@", [], nil, false, t.line)
      end
      base = parse_unary
      if op?("**")
        op = advance
        exp = nest { parse_uminus }
        return Call.new(base, "**", [exp], nil, false, op.line)
      end
      base
    end

    def parse_unary
      t = peek
      if t.type == :op
        case t.value
        when "!"
          advance
          return Not.new(nest { parse_unary }, t.line)
        when "~"
          advance
          return Call.new(nest { parse_unary }, "~", [], nil, false, t.line)
        when "+"
          advance
          return Call.new(nest { parse_unary }, "+@", [], nil, false, t.line)
        when "-"
          return nest { parse_uminus }
        when "*", "**", "&"
          error("splat and block-pass operators are not supported here", t.line)
        end
      end
      parse_postfix
    end

    def command_start?(t)
      return false unless t.spc

      case t.type
      when :int, :float, :str, :sym, :dsym, :label, :const, :ident then true
      when :kw then t.value == "nil" || t.value == "true" || t.value == "false"
      when :op
        case t.value
        when "(", "[", "!" then true
        when "&" then peek(1).type == :sym && !peek(1).spc
        when "-"
          nt = peek(1)
          (nt.type == :int || nt.type == :float) && !nt.spc
        else false
        end
      else false
      end
    end

    def parse_postfix
      e = parse_primary
      loop do
        t = peek
        break unless t.type == :op

        case t.value
        when ".", "&."
          advance
          nt = advance
          name = nt.value
          if nt.type == :op && (OPERATOR_METHODS[nt.value] || nt.value == "[")
            if nt.value == "["
              unexpected(peek, "expected ']'") unless op?("]")
              advance
              name = "[]"
            end
          elsif nt.type != :ident && nt.type != :const
            unexpected(nt, "expected a method name")
          end
          args = []
          bp = nil
          if op?("(") && !peek.spc
            advance
            args, bp = parse_arglist(")")
          elsif !@no_command && command_start?(peek)
            args, bp = parse_command_args
          end
          blk = parse_block_opt
          error("both block arg and actual block given", nt.line) if blk && bp
          e = Call.new(e, name, args, blk || bp, t.value == "&.", nt.line)
        when "::"
          advance
          nt = advance
          unexpected(nt, "expected a constant name") unless nt.type == :const
          e = Colon2.new(e, nt.value, nt.line)
        when "["
          advance
          args, bp = parse_arglist("]")
          error("block argument is not allowed in []", t.line) if bp
          e = Call.new(e, "[]", args, nil, false, t.line)
        else
          return parse_assignment(e, t) if ASSIGN_OPS[t.value]

          break
        end
      end
      e
    end

    def parse_assignment(target, t)
      op = t.value
      case target
      when VCall, LVar
        advance
        skip_nl
        depth = declare(target.name)
        rhs = nest { parse_ternary }
        return LAsgn.new(target.name, depth, rhs, t.line) if op == "="

        OpAsgn.new(LVar.new(target.name, depth, target.line), op[0..-2], rhs, t.line)
      when Call
        if target.name == "[]" && !target.block && !target.safe
          advance
          skip_nl
          rhs = nest { parse_ternary }
          return IndexAsgn.new(target.recv, target.args, rhs, t.line) if op == "="

          OpAsgn.new(target, op[0..-2], rhs, t.line)
        elsif target.recv && target.args.empty? && !target.block && !target.safe && target.name.match?(/\A[a-z_][A-Za-z0-9_]*\z/)
          advance
          skip_nl
          rhs = nest { parse_ternary }
          return AttrAsgn.new(target.recv, target.name, rhs, t.line) if op == "="

          OpAsgn.new(target, op[0..-2], rhs, t.line)
        else
          error("cannot assign to this expression", t.line)
        end
      when Const, Colon2
        error("dynamic constant assignment is not supported", t.line)
      else
        error("cannot assign to this expression", t.line)
      end
    end

    # -- primaries -------------------------------------------------------------

    def parse_primary
      t = peek
      nest do
        case t.type
        when :int, :float
          advance
          Lit.new(t.value, t.line)
        when :str
          advance
          Str.new(parse_parts(t.value), t.line)
        when :sym
          advance
          Lit.new(t.value.to_sym, t.line)
        when :dsym
          advance
          DSym.new(parse_parts(t.value), t.line)
        when :words
          advance
          Arr.new(t.value.map { |w| Str.new([w], t.line) }, t.line)
        when :symwords
          advance
          Arr.new(t.value.map { |w| Lit.new(w.to_sym, t.line) }, t.line)
        when :ident
          parse_identifier
        when :const
          parse_constant
        when :kw
          parse_keyword(t)
        when :op
          case t.value
          when "(" then parse_paren
          when "[" then parse_array
          when "{" then parse_hash
          else unexpected(t)
          end
        else
          unexpected(t)
        end
      end
    end

    def parse_keyword(t)
      case t.value
      when "nil"
        advance
        Lit.new(nil, t.line)
      when "true"
        advance
        Lit.new(true, t.line)
      when "false"
        advance
        Lit.new(false, t.line)
      when "if", "unless" then parse_if
      when "while", "until" then parse_while
      when "for" then parse_for
      when "case" then parse_case
      when "begin" then error("begin/rescue is not supported in ERBLite", t.line)
      when "self" then error("self is not supported in ERBLite", t.line)
      else unexpected(t)
      end
    end

    def parse_parts(raw)
      raw.map do |p|
        next p if p.is_a?(String)

        sub_parse(p[1], p[2])
      end
    end

    def sub_parse(toks, line)
      saved = [@toks, @pos]
      @toks = toks + [Token.new(:eof, nil, toks.last ? toks.last.line : line)]
      @pos = 0
      body = with_flags { parse_statements }
      unexpected(peek) unless peek.type == :eof
      body
    ensure
      @toks, @pos = saved
    end

    def parse_identifier
      t = advance
      name = t.value
      depth = lookup(name)
      nt = peek
      if op?("(", nt) && !nt.spc
        advance
        args, bp = parse_arglist(")")
        blk = parse_block_opt
        error("both block arg and actual block given", t.line) if blk && bp
        FCall.new(name, args, blk || bp, t.line)
      elsif depth
        LVar.new(name, depth, t.line)
      elsif !@no_command && command_start?(nt)
        args, bp = parse_command_args
        blk = parse_block_opt
        error("both block arg and actual block given", t.line) if blk && bp
        FCall.new(name, args, blk || bp, t.line)
      else
        blk = parse_block_opt
        blk ? FCall.new(name, [], blk, t.line) : VCall.new(name, t.line)
      end
    end

    def parse_constant
      t = advance
      nt = peek
      if op?("(", nt) && !nt.spc
        advance
        args, bp = parse_arglist(")")
        blk = parse_block_opt
        error("both block arg and actual block given", t.line) if blk && bp
        FCall.new(t.value, args, blk || bp, t.line)
      elsif !@no_command && command_start?(nt)
        args, bp = parse_command_args
        FCall.new(t.value, args, bp, t.line)
      else
        Const.new(t.value, t.line)
      end
    end

    def parse_paren
      t = advance
      body = with_flags { parse_statements }
      expect_op(")")
      return Lit.new(nil, t.line) if body.stmts.empty?
      return body.stmts[0] if body.stmts.length == 1

      body
    end

    def parse_array
      t = advance
      args, bp = parse_arglist("]")
      error("block argument is not allowed in an array literal", t.line) if bp
      Arr.new(args, t.line)
    end

    def parse_hash
      t = advance
      pairs = []
      with_flags do
        loop do
          skip_nl
          break if op?("}")

          kt = peek
          if kt.type == :label
            advance
            skip_nl
            pairs << [Lit.new(kt.value.to_sym, kt.line), parse_ternary]
          else
            k = parse_ternary
            skip_nl
            expect_op("=>")
            skip_nl
            pairs << [k, parse_ternary]
          end
          skip_nl
          break unless accept_op(",")
        end
        skip_nl
        expect_op("}")
      end
      HashLit.new(pairs, t.line)
    end

    # Parses arguments up to +closer+. Returns [args, block_pass].
    def parse_arglist(closer)
      args = []
      pairs = []
      bp_ref = [nil]
      with_flags do
        loop do
          skip_nl
          break if op?(closer)

          parse_one_arg(args, pairs, bp_ref)
          skip_nl
          break unless accept_op(",")
        end
        skip_nl
        expect_op(closer)
      end
      args << HashLit.new(pairs, pairs.first[0].line) unless pairs.empty?
      [args, bp_ref[0]]
    end

    def parse_command_args
      args = []
      pairs = []
      bp_ref = [nil]
      with_flags(no_do: true, no_command: true) do
        loop do
          parse_one_arg(args, pairs, bp_ref)
          break unless accept_op(",")

          skip_nl
        end
      end
      args << HashLit.new(pairs, pairs.first[0].line) unless pairs.empty?
      [args, bp_ref[0]]
    end

    def parse_one_arg(args, pairs, bp_ref)
      t = peek
      error("block argument must be the last argument", t.line) if bp_ref[0]
      if op?("&")
        advance
        st = advance
        error("only &:symbol block arguments are supported", st.line) unless st.type == :sym
        bp_ref[0] = BlockPass.new(st.value, st.line)
      elsif t.type == :label
        advance
        skip_nl
        pairs << [Lit.new(t.value.to_sym, t.line), parse_ternary]
      else
        e = parse_ternary
        if op?("=>")
          advance
          skip_nl
          pairs << [e, parse_ternary]
        else
          error("positional argument after keyword arguments", t.line) unless pairs.empty?
          args << e
        end
      end
    end

    def parse_block_opt
      if op?("{")
        parse_block("}")
      elsif kw?("do") && !@no_do
        parse_block("end")
      end
    end

    def parse_block(closer)
      t = advance
      @scopes.push({})
      @ctx.push(:block)
      begin
        with_flags do
          params = []
          if op?("||")
            advance
          elsif op?("|")
            advance
            params = parse_params({})
            expect_op("|")
          end
          body = parse_statements
          closer == "}" ? expect_op("}") : expect_kw("end")
          BlockNode.new(params, body, t.line)
        end
      ensure
        @ctx.pop
        @scopes.pop
      end
    end

    def parse_params(seen)
      list = []
      loop do
        t = peek
        if op?("(")
          advance
          list << parse_params(seen)
          expect_op(")")
        elsif t.type == :ident && t.value.match?(/\A[a-z_][A-Za-z0-9_]*\z/)
          advance
          if seen[t.value] && !t.value.start_with?("_")
            error("duplicated argument name", t.line)
          end
          seen[t.value] = true
          @scopes.last[t.value] = true
          list << t.value
        else
          error("unsupported block parameter (only names and (a, b) are allowed)", t.line)
        end
        break unless accept_op(",")
      end
      list
    end

    def parse_then
      t = peek
      if term?(t)
        skip_terms
        advance if kw?("then")
      elsif kw?("then", t)
        advance
      else
        unexpected(t, "expected 'then' or a newline")
      end
    end

    def parse_if
      t = advance
      cond = with_flags { parse_expr }
      parse_then
      body = with_flags { parse_statements }
      if t.value == "unless"
        els = nil
        if kw?("else")
          advance
          els = with_flags { parse_statements }
        end
        expect_kw("end")
        return If.new(cond, els, body, t.line)
      end
      node = If.new(cond, body, nil, t.line)
      tail = node
      loop do
        if kw?("elsif")
          et = advance
          c = with_flags { parse_expr }
          parse_then
          b = with_flags { parse_statements }
          n = If.new(c, b, nil, et.line)
          tail.els = n
          tail = n
        elsif kw?("else")
          advance
          tail.els = with_flags { parse_statements }
          break
        else
          break
        end
      end
      expect_kw("end")
      node
    end

    def loop_header_end
      if kw?("do")
        advance
      elsif term?
        skip_terms
      else
        unexpected(peek, "expected 'do' or a newline")
      end
    end

    def parse_while
      t = advance
      cond = with_flags(no_do: true) { parse_expr }
      loop_header_end
      @ctx.push(:loop)
      body = begin
        with_flags { parse_statements }
      ensure
        @ctx.pop
      end
      expect_kw("end")
      While.new(cond, body, t.value == "until", t.line)
    end

    def parse_for
      t = advance
      names = []
      loop do
        nt = advance
        unexpected(nt, "expected a variable name") unless nt.type == :ident
        names << nt.value
        break unless accept_op(",")
      end
      expect_kw("in")
      iter = with_flags(no_do: true) { parse_expr }
      loop_header_end
      vars = names.map { |n| [n, declare(n)] }
      @ctx.push(:loop)
      body = begin
        with_flags { parse_statements }
      ensure
        @ctx.pop
      end
      expect_kw("end")
      For.new(vars, iter, body, t.line)
    end

    def parse_case
      t = advance
      subject = term?(peek) || kw?("when") ? nil : with_flags { parse_expr }
      skip_terms
      whens = []
      while kw?("when")
        advance
        conds = [with_flags { parse_ternary }]
        while accept_op(",")
          skip_nl
          conds << with_flags { parse_ternary }
        end
        parse_then
        whens << [conds, with_flags { parse_statements }]
      end
      unexpected(peek, "expected 'when'") if whens.empty?
      els = nil
      if kw?("else")
        advance
        els = with_flags { parse_statements }
      end
      expect_kw("end")
      Case.new(subject, whens, els, t.line)
    end
  end
end
