# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

# frozen_string_literal: true

module ERBLite
  # format / sprintf / String#% (SPEC section 10).
  #
  # The format string is parsed here. Each directive is then rendered by
  # calling Kernel#format with a spec rebuilt from validated pieces only
  # (flags from "-+ 0#", digits, '.', and a type letter from a fixed set), so
  # template text never reaches Kernel#format directly.
  module Format
    FLAGS = "-+ 0#"
    INT_TYPES = %w[d i u].freeze
    BASE_TYPES = %w[x X o b B].freeze
    FLOAT_TYPES = %w[f e E g G].freeze

    module_function

    def arg_error(msg)
      Error.new("ArgumentError", msg)
    end

    def format(it, fmt, args)
      Builtins.need_str(fmt)
      st = { args: args, argi: 0, mode: nil }
      out = +""
      i = 0
      n = fmt.length
      while i < n
        c = fmt[i]
        if c != "%"
          j = fmt.index("%", i) || n
          out << fmt[i...j]
          i = j
          next
        end
        i += 1
        raise arg_error("incomplete format specifier; use %% (double %) instead") if i >= n

        if fmt[i] == "%"
          out << "%"
          i += 1
          next
        end
        flags = +""
        width = nil
        prec = nil
        value = nil
        has_value = false
        brace = false
        loop do
          ch = fmt[i]
          if ch && FLAGS.include?(ch)
            flags << ch unless flags.include?(ch)
            i += 1
          elsif ch == "<" || ch == "{"
            raise arg_error("named after a value was already given") if has_value

            close = fmt.index(ch == "<" ? ">" : "}", i)
            raise arg_error("malformed name - unmatched parenthesis") unless close

            value = named(st, fmt[(i + 1)...close])
            has_value = true
            brace = ch == "{"
            i = close + 1
            break if brace
          elsif ch&.match?(/[0-9]/) && width.nil?
            j = i
            j += 1 while fmt[j]&.match?(/[0-9]/)
            if fmt[j] == "$" && !has_value
              value = positional(st, fmt[i...j].to_i)
              has_value = true
              i = j + 1
            else
              width = fmt[i...j].to_i
              i = j
            end
          elsif ch == "*" && width.nil?
            i += 1
            w = int_arg(next_arg(st))
            if w.negative?
              flags << "-" unless flags.include?("-")
              w = -w
            end
            width = w
          elsif ch == "." && prec.nil?
            i += 1
            if fmt[i] == "*"
              i += 1
              pv = int_arg(next_arg(st))
              prec = pv.negative? ? nil : pv
            else
              j = i
              j += 1 while fmt[j]&.match?(/[0-9]/)
              prec = fmt[i...j].to_i
              i = j
            end
          else
            break
          end
        end
        Builtins.cap(width) if width
        Builtins.cap(prec) if prec
        spec = "%#{flags}#{width}#{prec ? ".#{prec}" : ""}"
        if brace
          out << Kernel.format("#{spec}s", Builtins.to_s(value))
          next
        end
        type = fmt[i]
        raise arg_error("malformed format string - %#{type}") if type.nil? || !type.match?(/[a-zA-Z]/)

        i += 1
        unless INT_TYPES.include?(type) || BASE_TYPES.include?(type) || FLOAT_TYPES.include?(type) ||
               %w[s p c].include?(type)
          raise arg_error("malformed format string - %#{type}")
        end

        value = next_arg(st) unless has_value
        out << directive(it, type, flags, spec, value)
      end
      out
    end

    def next_arg(st)
      raise arg_error("unnumbered mixed with named") if st[:mode] == :named
      raise arg_error("unnumbered mixed with numbered") if st[:mode] == :abs

      st[:mode] = :pos
      raise arg_error("too few arguments") if st[:argi] >= st[:args].length

      v = st[:args][st[:argi]]
      st[:argi] += 1
      v
    end

    def positional(st, n)
      raise arg_error("numbered(#{n}) after unnumbered") if st[:mode] == :pos
      raise arg_error("numbered(#{n}) after named") if st[:mode] == :named
      raise arg_error("invalid index - #{n}$") if n < 1 || n > st[:args].length

      st[:mode] = :abs
      st[:args][n - 1]
    end

    def named(st, name)
      raise arg_error("named<#{name}> after unnumbered") if st[:mode] == :pos || st[:mode] == :abs

      st[:mode] = :named
      h = st[:args][0]
      raise arg_error("one hash required") unless st[:args].length == 1 && h.is_a?(Hash)

      key = name.to_sym
      return h[key] if h.key?(key)
      return h.default unless h.default.nil?

      raise Error.new("KeyError", "key<#{name}> not found")
    end

    def int_arg(v)
      case v
      when Integer then v
      when Float
        raise Error.new("FloatDomainError", v.to_s) if v.nan? || v.infinite?

        Builtins.ck(v.to_i)
      when String then Builtins.ck(Kernel.Integer(v))
      else raise Error.new("TypeError", "can't convert #{Builtins.tname(v)} into Integer")
      end
    end

    def float_arg(v)
      case v
      when Integer, Float then v
      when String then Kernel.Float(v)
      else raise Error.new("TypeError", "can't convert #{Builtins.tname(v)} into Float")
      end
    end

    def directive(it, type, flags, spec, v)
      if INT_TYPES.include?(type)
        Kernel.format("#{spec}d", int_arg(v))
      elsif BASE_TYPES.include?(type)
        iv = int_arg(v)
        if iv.negative? && !flags.include?("+") && !flags.include?(" ")
          raise arg_error("negative argument for %#{type} (two's complement output is not supported)")
        end

        Kernel.format("#{spec}#{type}", iv)
      elsif FLOAT_TYPES.include?(type)
        Kernel.format("#{spec}#{type}", float_arg(v))
      else
        case type
        when "s" then Kernel.format("#{spec}s", Builtins.to_s(v))
        when "p" then Kernel.format("#{spec}s", Builtins.inspect(v))
        else
          case v
          when Integer, Float then Kernel.format("#{spec}c", int_arg(v))
          when String then Kernel.format("#{spec}c", v)
          else raise Error.new("TypeError", "no implicit conversion of #{Builtins.tname(v)} into Integer")
          end
        end
      end
    end
  end
end
