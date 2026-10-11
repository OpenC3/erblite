# frozen_string_literal: true

require "strscan"

module ERBLite
  # A lexical token. +type+ is one of:
  #   :text :ostart :oend :tagend :nl :semi :eof
  #   :int :float :str :sym :dsym :words :symwords :ident :const :label :kw :op
  # +spc+ is true when whitespace came directly before the token.
  # +neg+ marks a negative numeric literal ("-2").
  class Token
    attr_reader :type, :value, :line, :spc
    attr_accessor :neg

    def initialize(type, value, line, spc = false)
      @type = type
      @value = value
      @line = line
      @spc = spc
      @neg = false
    end

    def inspect
      "#<Token #{@type} #{@value.inspect} L#{@line}>"
    end
  end

  # Splits a template into text and tags exactly like Ruby's ERB with
  # trim_mode "-" (ERB::Compiler::ExplicitScanner), then tokenizes the code
  # inside each tag. The result is one flat token stream for the whole
  # template, so a statement can span several tags.
  class Lexer
    STAG_RE = /(.*?)(^[ \t]*<%-|<%-|<%%|<%=|<%#|<%|\z)/m
    ETAG_RE = /(.*?)(-%>|%%>|%>|\z)/m

    def self.tokenize(source, filename)
      new(source, filename).run
    end

    def initialize(source, filename)
      @src = source
      @filename = filename
    end

    def run
      toks = []
      ss = StringScanner.new(@src)
      line = 1
      text = +""
      text_line = 1
      until ss.eos?
        ss.scan(STAG_RE)
        part = ss[1]
        elem = ss[2]
        text << part
        line += part.count("\n")
        break if elem.empty?

        if elem == "<%%"
          text << "<%"
          next
        end
        elem = "<%" if elem.end_with?("<%-")
        unless text.empty?
          toks << Token.new(:text, text, text_line)
          text = +""
        end
        tag_line = line
        code = +""
        loop do
          ss.scan(ETAG_RE)
          code << ss[1]
          etag = ss[2]
          case etag
          when "%%>"
            code << "%>"
          when "-%>"
            line += code.count("\n")
            emit_tag(toks, elem, code, tag_line, line)
            line += 1 if ss.scan(/\r?\n/)
            break
          when "%>"
            line += code.count("\n")
            emit_tag(toks, elem, code, tag_line, line)
            break
          else
            raise Error.new("SyntaxError", "unterminated tag (missing %>)", @filename, tag_line)
          end
        end
        text_line = line
      end
      toks << Token.new(:text, text, text_line) unless text.empty?
      toks << Token.new(:eof, nil, line)
      toks
    end

    private

    def emit_tag(toks, stag, code, tag_line, end_line)
      case stag
      when "<%#"
        nil
      when "<%="
        toks << Token.new(:ostart, nil, tag_line)
        CodeLexer.new(code, tag_line, toks, @filename).run
        toks << Token.new(:oend, nil, end_line)
      else
        CodeLexer.new(code, tag_line, toks, @filename).run
        toks << Token.new(:tagend, nil, end_line)
      end
    end
  end

  # Tokenizes the Ruby-subset code inside one tag.
  class CodeLexer
    KEYWORDS = %w[if elsif else unless then end while until for in do case when
                  break next nil true false and or not begin self].freeze
    # Ruby keywords that ERBLite rejects with a SyntaxError.
    RESERVED = %w[def class module return yield super rescue ensure alias undef
                  redo retry defined? __FILE__ __LINE__ __ENCODING__].freeze
    KEYWORD_SET = (KEYWORDS + RESERVED).to_h { |k| [k, true] }.freeze

    OPS = %w[**= <=> === ... <<= >>= &&= ||= ** == != >= <= && || << >> += -= *= /=
             %= |= &= ^= => .. :: &. -> =~ !~ + - * / % = < > ! & | ^ ~ ? : , . ( ) [ ] { }].freeze
    OPS_BY_LEN = OPS.group_by(&:length).sort_by { |len, _| -len }.map { |len, ops| [len, ops.to_h { |o| [o, true] }] }.freeze

    # A newline right after one of these does not end the statement.
    NL_CONT_OPS = "+ - * / % ** == != < > <= >= <=> === && || & | ^ << >> = += -= *= /= %= **= ||= &&= " \
                  "<<= >>= |= &= ^= , ( [ { . &. ? : => :: ! ~".split.to_h { |o| [o, true] }.freeze

    SYMBOL_OPS = %w[[]= [] <=> === == != ** +@ -@ << >> <= >= + - * / % < > ! & | ^ ~].freeze

    ESCAPES = { "n" => "\n", "t" => "\t", "r" => "\r", "s" => " ", "e" => "\e", "a" => "\a",
                "b" => "\b", "f" => "\f", "v" => "\v" }.freeze

    WORD_DELIMS = { "[" => "]", "(" => ")", "{" => "}", "<" => ">" }.freeze

    def initialize(src, line, out, filename)
      @s = src
      @i = 0
      @line = line
      @out = out
      @filename = filename
      @interp_depth = 0
    end

    def run
      lex_until(false)
    end

    private

    def error(msg, line = @line)
      raise Error.new("SyntaxError", msg, @filename, line)
    end

    # Lexes tokens into @out. With +stop_at_brace+ it stops after the "}"
    # that closes a string interpolation.
    def lex_until(stop_at_brace)
      depth = 0
      spc = false
      while @i < @s.length
        c = @s[@i]
        case c
        when " ", "\t", "\r", "\f", "\v"
          @i += 1
          spc = true
          next
        when "\\"
          nl = @s[@i + 1] == "\n" ? 2 : (@s[@i + 1] == "\r" && @s[@i + 2] == "\n" ? 3 : 0)
          if nl.positive?
            @i += nl
            @line += 1
            spc = true
            next
          end
          error("unexpected backslash")
        when "\n"
          emit_nl
          @i += 1
          @line += 1
          spc = true
          next
        when "#"
          @i += 1 while @i < @s.length && @s[@i] != "\n"
          next
        when ";"
          @out << Token.new(:semi, ";", @line, spc)
          @i += 1
          spc = false
          next
        end
        if stop_at_brace
          if c == "{"
            depth += 1
          elsif c == "}"
            if depth.zero?
              @i += 1
              return
            end
            depth -= 1
          end
        end
        lex_token(spc)
        spc = false
      end
      error("unterminated string interpolation") if stop_at_brace
    end

    def emit_nl
      last = @out.last
      return if last.nil?
      return if %i[nl semi tagend ostart text].include?(last.type)
      return if last.type == :op && NL_CONT_OPS[last.value]
      return if last.type == :kw && %w[and or not].include?(last.value)

      # A line starting with ".foo" or "&.foo" continues the previous line.
      j = @i + 1
      j += 1 while j < @s.length && " \t\r\n".include?(@s[j])
      return if @s[j] == "." && @s[j + 1] != "."
      return if @s[j] == "&" && @s[j + 1] == "."

      @out << Token.new(:nl, nil, @line)
    end

    def ends_value?(tok)
      return false unless tok

      case tok.type
      when :int, :float, :str, :sym, :dsym, :words, :symwords, :ident, :const then true
      when :op then tok.value == ")" || tok.value == "]" || tok.value == "}"
      when :kw then %w[nil true false end].include?(tok.value)
      else false
      end
    end

    def push(type, value, spc, line = @line)
      t = Token.new(type, value, line, spc)
      @out << t
      t
    end

    def lex_token(spc)
      c = @s[@i]
      if c.match?(/[0-9]/)
        lex_number(false, spc)
      elsif c == "-" && @s[@i + 1]&.match?(/[0-9]/) && !ends_value?(@out.last)
        lex_number(true, spc)
      elsif c.match?(/[a-z_]/)
        lex_ident(spc)
      elsif c.match?(/[A-Z]/)
        lex_const(spc)
      elsif c == '"'
        @i += 1
        line = @line
        parts = lex_dstring_body('"')
        string_token(parts, spc, line)
      elsif c == "'"
        lex_sstring(spc)
      elsif c == ":"
        lex_colon(spc)
      elsif c == "%" && !ends_value?(@out.last) && (@s[@i + 1] == "w" || @s[@i + 1] == "i") && WORD_DELIMS[@s[@i + 2]]
        lex_words(spc)
      elsif c == "@"
        error("instance variables are not supported")
      elsif c == "$"
        error("global variables are not supported")
      elsif c == "`"
        error("backticks (shell commands) are not supported")
      else
        lex_op(spc)
      end
    end

    def lex_op(spc)
      OPS_BY_LEN.each do |len, ops|
        cand = @s[@i, len]
        next unless cand && cand.length == len && ops[cand]

        case cand
        when "->" then error("lambdas are not supported")
        when "=~", "!~" then error("regular expressions are not supported")
        end
        @i += len
        push(:op, cand, spc)
        return
      end
      error("unexpected character #{@s[@i].inspect}")
    end

    # -- numbers -------------------------------------------------------------

    def lex_number(neg, spc)
      line = @line
      start = @i
      @i += 1 if neg
      s = @s
      if s[@i] == "0" && s[@i + 1]&.match?(/[xXbBoO]/)
        kind = s[@i + 1].downcase
        @i += 2
        digits_start = @i
        @i += 1 while @i < s.length && s[@i].match?(/[0-9a-zA-Z_]/)
        digits = s[digits_start...@i]
        base, re = { "x" => [16, /\A[0-9a-f]+(_[0-9a-f]+)*\z/i], "b" => [2, /\A[01]+(_[01]+)*\z/],
                     "o" => [8, /\A[0-7]+(_[0-7]+)*\z/] }[kind]
        error("invalid numeric literal") unless digits.match?(re)
        value = digits.delete("_").to_i(base)
        int_token(neg ? -value : value, neg, spc, line)
        return
      end
      @i += 1 while @i < s.length && s[@i].match?(/[0-9_]/)
      int_part = s[(neg ? start + 1 : start)...@i]
      is_float = false
      if s[@i] == "." && s[@i + 1]&.match?(/[0-9]/)
        is_float = true
        @i += 1
        @i += 1 while @i < s.length && s[@i].match?(/[0-9_]/)
      end
      if (s[@i] == "e" || s[@i] == "E") &&
         (s[@i + 1]&.match?(/[0-9]/) || (s[@i + 1]&.match?(/[+-]/) && s[@i + 2]&.match?(/[0-9]/)))
        is_float = true
        @i += 2
        @i += 1 while @i < s.length && s[@i].match?(/[0-9_]/)
      end
      error("invalid numeric literal") if @i < s.length && s[@i].match?(/[A-Za-z_]/)
      text = s[start...@i]
      if is_float
        body = text.sub(/\A-/, "")
        mant = body.split(/[eE]/).first
        error("invalid numeric literal") unless mant.match?(/\A\d+(_\d+)*(\.\d+(_\d+)*)?\z/)
        value = Float(text.delete("_"))
        t = push(:float, value, spc, line)
        t.neg = neg
      else
        error("invalid numeric literal") unless int_part.match?(/\A\d+(_\d+)*\z/)
        digits = int_part.delete("_")
        if digits.length > 1 && digits.start_with?("0")
          error("Invalid octal digit") unless digits.match?(/\A[0-7]+\z/)
          value = digits.to_i(8)
        else
          value = digits.to_i
        end
        int_token(neg ? -value : value, neg, spc, line)
      end
    end

    def int_token(value, neg, spc, line)
      unless value.between?(Builtins::INT_MIN, Builtins::INT_MAX)
        raise Error.new("RangeError", "integer literal out of 64-bit range", @filename, line)
      end
      t = push(:int, value, spc, line)
      t.neg = neg
    end

    # -- identifiers ---------------------------------------------------------

    def after_dot?
      last = @out.last
      last && last.type == :op && (last.value == "." || last.value == "&.")
    end

    def read_name
      start = @i
      @i += 1 while @i < @s.length && @s[@i].match?(/[A-Za-z0-9_]/)
      if (@s[@i] == "?" || @s[@i] == "!") && !(@s[@i + 1] == "=" && !["=", "~", ">"].include?(@s[@i + 2]))
        @i += 1
      end
      @s[start...@i]
    end

    def label_follows?
      @s[@i] == ":" && @s[@i + 1] != ":"
    end

    def lex_ident(spc)
      line = @line
      name = read_name
      dot = after_dot?
      if !dot && label_follows? && !ternary_context?
        @i += 1
        push(:label, name, spc, line)
      elsif !dot && KEYWORD_SET[name]
        push(:kw, name, spc, line)
      else
        push(:ident, name, spc, line)
      end
    end

    def lex_const(spc)
      line = @line
      start = @i
      @i += 1 while @i < @s.length && @s[@i].match?(/[A-Za-z0-9_]/)
      name = @s[start...@i]
      if after_dot?
        if (@s[@i] == "?" || @s[@i] == "!") && @s[@i + 1] != "="
          name += @s[@i]
          @i += 1
        end
        push(:ident, name, spc, line)
      elsif label_follows? && !ternary_context?
        @i += 1
        push(:label, name, spc, line)
      else
        push(:const, name, spc, line)
      end
    end

    def ternary_context?
      last = @out.last
      last && last.type == :op && last.value == "?"
    end

    # -- strings -------------------------------------------------------------

    def lex_sstring(spc)
      line = @line
      @i += 1
      buf = +""
      loop do
        c = @s[@i]
        error("unterminated string", line) if c.nil?
        if c == "\\" && (@s[@i + 1] == "\\" || @s[@i + 1] == "'")
          buf << @s[@i + 1]
          @i += 2
        elsif c == "'"
          @i += 1
          break
        elsif c == "\r" && @s[@i + 1] == "\n"
          @i += 1 # Ruby reads CRLF in source code as LF
        else
          @line += 1 if c == "\n"
          buf << c
          @i += 1
        end
      end
      string_token([buf], spc, line)
    end

    def string_token(parts, spc, line)
      last = @out.last
      if label_follows? && parts.all?(String) && last && last.type == :op && ["{", ",", "("].include?(last.value)
        @i += 1
        push(:label, parts.join, spc, line)
      else
        push(:str, parts, spc, line)
      end
    end

    # Reads a double-quoted string body (after the opening quote) up to
    # +term+. Returns an Array of String parts and [:code, tokens, line]
    # interpolation parts.
    def lex_dstring_body(term)
      start_line = @line
      parts = []
      buf = (+"").force_encoding(Encoding::BINARY)
      flush = lambda do
        unless buf.empty?
          str = buf.dup.force_encoding(Encoding::UTF_8)
          error("invalid UTF-8 in string literal", start_line) unless str.valid_encoding?
          parts << str
          buf.clear
        end
      end
      loop do
        c = @s[@i]
        error("unterminated string", start_line) if c.nil?
        if c == term
          @i += 1
          break
        elsif c == "\\"
          read_escape(buf)
        elsif c == "#" && @s[@i + 1] == "{"
          flush.call
          @i += 2
          line = @line
          saved = @out
          @out = []
          @interp_depth += 1
          if @interp_depth > Parser::MAX_DEPTH
            raise Error.new("LimitError", "string interpolation is nested too deeply", @filename, line)
          end

          lex_until(true)
          @interp_depth -= 1
          parts << [:code, @out, line]
          @out = saved
        elsif c == "#" && ((@s[@i + 1] == "@" && @s[@i + 2]&.match?(/[A-Za-z_@]/)) ||
                           (@s[@i + 1] == "$" && @s[@i + 2]&.match?(%r{[A-Za-z0-9_!@&`'+~=/\\,;.<>*$?:-]})))
          error("instance/global variable interpolation is not supported")
        elsif c == "\r" && @s[@i + 1] == "\n"
          @i += 1 # Ruby reads CRLF in source code as LF
        else
          @line += 1 if c == "\n"
          buf << c.b
          @i += 1
        end
      end
      flush.call
      parts
    end

    def read_escape(buf)
      d = @s[@i + 1]
      error("unterminated string") if d.nil?
      @i += 2
      if ESCAPES[d]
        buf << ESCAPES[d]
      elsif d.match?(/[0-7]/)
        digits = d
        while digits.length < 3 && @s[@i]&.match?(/[0-7]/)
          digits << @s[@i]
          @i += 1
        end
        buf << (digits.to_i(8) & 0xFF).chr
      elsif d == "x"
        digits = +""
        while digits.length < 2 && @s[@i]&.match?(/[0-9a-fA-F]/)
          digits << @s[@i]
          @i += 1
        end
        error("invalid hex escape") if digits.empty?
        buf << digits.to_i(16).chr
      elsif d == "u"
        if @s[@i] == "{"
          close = @s.index("}", @i)
          error("unterminated Unicode escape") unless close
          codes = @s[(@i + 1)...close].strip.split(/[ \t]+/)
          error("invalid Unicode escape") if codes.empty?
          codes.each { |h| buf << codepoint(h) }
          @i = close + 1
        else
          h = @s[@i, 4]
          error("invalid Unicode escape") unless h&.match?(/\A[0-9a-fA-F]{4}\z/)
          buf << codepoint(h)
          @i += 4
        end
      elsif d == "\n"
        @line += 1
      elsif d == "\r" && @s[@i] == "\n"
        @i += 1
        @line += 1
      else
        buf << d.b
      end
    end

    def codepoint(hex)
      error("invalid Unicode escape") unless hex.match?(/\A[0-9a-fA-F]{1,6}\z/)
      cp = hex.to_i(16)
      error("invalid Unicode codepoint") if cp > 0x10FFFF || (0xD800..0xDFFF).cover?(cp)
      [cp].pack("U").b
    end

    # -- symbols -------------------------------------------------------------

    def lex_colon(spc)
      line = @line
      nxt = @s[@i + 1]
      if nxt == ":"
        @i += 2
        push(:op, "::", spc, line)
      elsif nxt == '"'
        @i += 2
        parts = lex_dstring_body('"')
        push(:dsym, parts, spc, line)
      elsif nxt == "'"
        @i += 1
        start_count = @out.size
        lex_sstring(false)
        t = @out.pop
        error("invalid symbol") unless @out.size == start_count && t.type == :str
        push(:dsym, t.value, spc, line)
      elsif nxt&.match?(/[A-Za-z_]/) || (nxt == "@" || nxt == "$") && @s[@i + 2]&.match?(/[A-Za-z_@]/)
        @i += 1
        start = @i
        @i += 1 while @s[@i] == "@" || @s[@i] == "$"
        @i += 1 while @i < @s.length && @s[@i].match?(/[A-Za-z0-9_]/)
        if @s[@i] == "?" || @s[@i] == "!"
          @i += 1 unless @s[@i + 1] == "="
        elsif @s[@i] == "=" && !["=", "~", ">"].include?(@s[@i + 1])
          @i += 1
        end
        push(:sym, @s[start...@i], spc, line)
      else
        op = SYMBOL_OPS.find { |o| @s[@i + 1, o.length] == o }
        if op
          @i += 1 + op.length
          push(:sym, op, spc, line)
        else
          @i += 1
          push(:op, ":", spc, line)
        end
      end
    end

    # -- %w[] / %i[] -----------------------------------------------------------

    def lex_words(spc)
      line = @line
      kind = @s[@i + 1]
      open = @s[@i + 2]
      close = WORD_DELIMS[open]
      @i += 3
      words = []
      cur = nil
      depth = 0
      loop do
        c = @s[@i]
        error("unterminated %#{kind} list", line) if c.nil?
        if c == "\\" && @s[@i + 1]
          d = @s[@i + 1]
          cur ||= +""
          if d.match?(/\s/) || d == open || d == close || d == "\\"
            cur << d
          else
            cur << c << d
          end
          @line += 1 if d == "\n"
          @i += 2
          next
        end
        if c == close && depth.zero?
          @i += 1
          break
        end
        if c.match?(/\s/)
          @line += 1 if c == "\n"
          words << cur if cur
          cur = nil
        else
          depth += 1 if c == open
          depth -= 1 if c == close
          (cur ||= +"") << c
        end
        @i += 1
      end
      words << cur if cur
      push(kind == "w" ? :words : :symwords, words, spc, line)
    end
  end
end
