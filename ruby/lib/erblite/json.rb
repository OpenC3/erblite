# frozen_string_literal: true

module ERBLite
  # JSON.parse / JSON.generate / JSON.pretty_generate / #to_json.
  #
  # Self-contained so output does not depend on the installed json gem. The
  # parser follows Ruby's json gem: comments are allowed, trailing commas,
  # leading zeros, raw control characters and lone surrogates are errors.
  module JSONSupport
    MAX_NESTING = 100

    module_function

    # -- parsing ---------------------------------------------------------------

    def parse(str, opts)
      Builtins.need_str(str)
      sym = false
      unless opts.nil?
        Builtins.need_hash(opts)
        opts.each do |k, v|
          sym = !(v.nil? || v == false) if k == :symbolize_names || k == "symbolize_names"
        end
      end
      s = str.encoding == Encoding::UTF_8 ? str : str.dup.force_encoding(Encoding::UTF_8)
      parse_error("source is not valid UTF-8") unless s.valid_encoding?
      Parser.new(s, sym).parse
    end

    def parse_error(msg)
      raise Error.new("JSON::ParserError", msg)
    end

    class Parser
      NUMBER_RE = /-?(?:0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?/

      def initialize(src, symbolize)
        @s = src
        @i = 0
        @sym = symbolize
        @depth = 0
      end

      def parse
        skip_ws
        JSONSupport.parse_error("unexpected end of input") if @i >= @s.length
        v = value
        skip_ws
        error("unexpected character") if @i < @s.length
        v
      end

      private

      def error(msg)
        JSONSupport.parse_error("#{msg} at offset #{@i}")
      end

      def skip_ws
        loop do
          c = @s[@i]
          if c == " " || c == "\t" || c == "\n" || c == "\r"
            @i += 1
          elsif c == "/" && @s[@i + 1] == "*"
            close = @s.index("*/", @i + 2)
            error("unterminated comment") unless close
            @i = close + 2
          elsif c == "/" && @s[@i + 1] == "/"
            nl = @s.index("\n", @i + 2)
            @i = nl ? nl + 1 : @s.length
          else
            return
          end
        end
      end

      def value
        c = @s[@i]
        case c
        when "{" then object
        when "[" then array
        when '"' then string
        when "t" then literal("true", true)
        when "f" then literal("false", false)
        when "n" then literal("null", nil)
        when "-", "0".."9" then number
        when nil then error("unexpected end of input")
        else error("unexpected character #{c.inspect}")
        end
      end

      def literal(word, v)
        error("unexpected token") unless @s[@i, word.length] == word
        @i += word.length
        v
      end

      def number
        m = NUMBER_RE.match(@s, @i)
        error("invalid number") unless m && m.begin(0) == @i && m[0] != "-"
        text = m[0]
        @i += text.length
        error("invalid number") if @s[@i]&.match?(/[0-9A-Za-z_.+-]/)
        if m[1] || m[2]
          Float(text)
        else
          Builtins.ck(text.to_i)
        end
      end

      def nest
        @depth += 1
        # json 2.21 accepts 101 levels when parsing (it rejects 102).
        error("nesting of #{@depth} is too deep") if @depth > MAX_NESTING + 1
        yield
      ensure
        @depth -= 1
      end

      def array
        nest do
          @i += 1
          out = []
          skip_ws
          if @s[@i] == "]"
            @i += 1
            return out
          end
          loop do
            skip_ws
            out << value
            skip_ws
            c = @s[@i]
            @i += 1
            break if c == "]"

            error("expected ',' or ']'") unless c == ","
          end
          out
        end
      end

      def object
        nest do
          @i += 1
          out = {}
          skip_ws
          if @s[@i] == "}"
            @i += 1
            return out
          end
          loop do
            skip_ws
            error("expected a string key") unless @s[@i] == '"'
            k = string
            k = k.to_sym if @sym
            skip_ws
            error("expected ':'") unless @s[@i] == ":"
            @i += 1
            skip_ws
            out[k] = value
            skip_ws
            c = @s[@i]
            @i += 1
            break if c == "}"

            error("expected ',' or '}'") unless c == ","
          end
          out
        end
      end

      ESC = { '"' => '"', "\\" => "\\", "/" => "/", "b" => "\b", "f" => "\f", "n" => "\n",
              "r" => "\r", "t" => "\t" }.freeze

      def string
        @i += 1
        out = +""
        loop do
          c = @s[@i]
          error("unterminated string") if c.nil?
          @i += 1
          if c == '"'
            return out
          elsif c == "\\"
            d = @s[@i]
            @i += 1
            if ESC[d]
              out << ESC[d]
            elsif d == "u"
              cp = hex4
              if cp.between?(0xD800, 0xDBFF)
                error("invalid surrogate pair") unless @s[@i, 2] == "\\u"
                @i += 2
                lo = hex4
                error("invalid surrogate pair") unless lo.between?(0xDC00, 0xDFFF)
                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00)
              elsif cp.between?(0xDC00, 0xDFFF)
                error("invalid surrogate")
              end
              out << cp.chr(Encoding::UTF_8)
            else
              error("invalid escape character in string")
            end
          elsif c.ord < 0x20
            error("invalid control character in string")
          else
            out << c
          end
        end
      end

      def hex4
        h = @s[@i, 4]
        error("incomplete unicode escape") unless h&.match?(/\A[0-9a-fA-F]{4}\z/)
        @i += 4
        h.to_i(16)
      end
    end

    # -- generation ------------------------------------------------------------

    def generate(obj)
      out = +""
      gen(obj, out, nil, 0)
      out
    end

    def pretty_generate(obj)
      out = +""
      gen(obj, out, "", 0)
      out
    end

    def gen_error(msg)
      raise Error.new("JSON::GeneratorError", msg)
    end

    # +indent+ is nil for compact output, else the current indentation.
    def gen(v, out, indent, depth)
      case v
      when nil then out << "null"
      when true then out << "true"
      when false then out << "false"
      when Integer then out << v.to_s
      when Float then out << float(v)
      when String then string(v, out)
      when Symbol then string(v.name, out)
      when Array
        gen_error("nesting of #{depth + 1} is too deep") if depth + 1 > MAX_NESTING
        if v.empty?
          out << "[]"
        elsif indent
          inner = "#{indent}  "
          out << "[\n"
          v.each_with_index do |x, i|
            out << ",\n" if i.positive?
            out << inner
            gen(x, out, inner, depth + 1)
          end
          out << "\n" << indent << "]"
        else
          out << "["
          v.each_with_index do |x, i|
            out << "," if i.positive?
            gen(x, out, nil, depth + 1)
          end
          out << "]"
        end
      when Hash
        gen_error("nesting of #{depth + 1} is too deep") if depth + 1 > MAX_NESTING
        if v.empty?
          out << "{}"
        else
          inner = indent ? "#{indent}  " : nil
          out << (indent ? "{\n" : "{")
          first = true
          v.each do |k, x|
            out << (indent ? ",\n" : ",") unless first
            first = false
            out << inner if indent
            string(k.is_a?(String) ? k : Builtins.to_s(k), out)
            out << (indent ? ": " : ":")
            gen(x, out, inner, depth + 1)
          end
          out << "\n" << indent if indent
          out << "}"
        end
      else
        string(Builtins.to_s(v), out)
      end
    end

    def string(s, out)
      gen_error("source sequence is illegal/malformed utf-8") unless s.valid_encoding?

      out << '"'
      s.each_char do |c|
        case c
        when '"' then out << '\\"'
        when "\\" then out << "\\\\"
        when "\n" then out << "\\n"
        when "\r" then out << "\\r"
        when "\t" then out << "\\t"
        when "\b" then out << "\\b"
        when "\f" then out << "\\f"
        else
          o = c.ord
          o < 0x20 ? out << format("\\u%04x", o) : out << c
        end
      end
      out << '"'
    end

    # [ERBLite] Floats are written with Float#to_s (1.0e+20), as Ruby 3.4's
    # bundled json 2.9.1 does.
    def float(f)
      gen_error("#{f} not allowed in JSON") if f.nan? || f.infinite?

      f.to_s
    end
  end
end
