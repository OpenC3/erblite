# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

# frozen_string_literal: true

module ERBLite
  # The allowlist of methods, functions and constants templates can use.
  #
  # Every table maps a method name (a String) to an Fn holding a lambda and
  # the allowed argument count. A name that is not in a table does not exist
  # for templates (NoMethodError). Handlers receive (interp, receiver, args,
  # block) and call fixed, native Ruby methods on the receiver; no handler
  # ever calls a method whose name comes from the template.
  module Builtins
    INT_MIN = -(2**63)
    INT_MAX = (2**63) - 1

    Fn = Struct.new(:fn, :min, :max)

    def self.defm(table, names, min, max = min, &fn)
      Array(names).each { |n| table[n] = Fn.new(fn, min, max) }
    end

    def self.check_arity(m, n)
      return if n >= m.min && (m.max.negative? || n <= m.max)

      exp = if m.max == m.min then m.min.to_s
            elsif m.max.negative? then "#{m.min}+"
            else "#{m.min}..#{m.max}"
            end
      raise Error.new("ArgumentError", "wrong number of arguments (given #{n}, expected #{exp})")
    end

    # -- errors and type helpers -------------------------------------------------

    def self.err(type, msg)
      Error.new(type, msg)
    end

    # Converts a native Ruby exception raised inside a handler into a template
    # error with the matching type name.
    def self.map_error(e)
      type = case e
             when ::FloatDomainError then "FloatDomainError"
             when ::ZeroDivisionError then "ZeroDivisionError"
             when ::Math::DomainError then "Math::DomainError"
             when ::KeyError then "KeyError"
             when ::StopIteration then "IndexError"
             when ::IndexError then "IndexError"
             when ::RangeError then "RangeError"
             when ::FrozenError then "FrozenError"
             when ::NoMethodError then "NoMethodError"
             when ::NameError then "NameError"
             when ::TypeError then "TypeError"
             when ::EncodingError then "ArgumentError"
             when ::ArgumentError then "ArgumentError"
             when ::IOError, ::SystemCallError then "IOError"
             else "RuntimeError"
             end
      msg = e.message.to_s.gsub("ERBLite::ClassValue", "Class").gsub("ERBLite::Enum", "Enumerator")
      Error.new(type, msg)
    end

    # [ERBLite] Largest String/Array an operation may build from a numeric
    # size argument (String#*, Array#*, Array.new, ljust, a[i] = v, ...).
    MAX_SIZE = 10_000_000

    def self.cap(n)
      raise err("LimitError", "result would be too large (more than #{MAX_SIZE} elements)") if n > MAX_SIZE

      n
    end

    def self.ck(v)
      if v.is_a?(Integer) && (v < INT_MIN || v > INT_MAX)
        raise err("RangeError", "integer overflow: result does not fit in 64 bits")
      end
      v
    end

    def self.class_name(v)
      case v
      when nil then "NilClass"
      when true then "TrueClass"
      when false then "FalseClass"
      when Integer then "Integer"
      when Float then "Float"
      when String then "String"
      when Symbol then "Symbol"
      when Array then "Array"
      when Hash then "Hash"
      when Range then "Range"
      when Enum then "Enumerator"
      when ClassValue then v.module? ? "Module" : "Class"
      else "Object"
      end
    end

    # Name used in "no implicit conversion of X into Y" messages.
    def self.tname(v)
      case v
      when nil then "nil"
      when true then "true"
      when false then "false"
      else class_name(v)
      end
    end

    def self.describe(v)
      case v
      when nil then "nil"
      when true then "true"
      when false then "false"
      when ClassValue then "#{v.module? ? "module" : "class"} #{v.name}"
      else "an instance of #{class_name(v)}"
      end
    end

    def self.need_int(v)
      raise err("TypeError", "no implicit conversion of #{tname(v)} into Integer") unless v.is_a?(Integer)

      v
    end

    # Integer or Float (truncated), as Ruby accepts for indexes and sizes.
    def self.need_intish(v)
      return v if v.is_a?(Integer)
      return ck(v.to_i) if v.is_a?(Float)

      raise err("TypeError", "no implicit conversion of #{tname(v)} into Integer")
    end

    def self.need_str(v)
      raise err("TypeError", "no implicit conversion of #{tname(v)} into String") unless v.is_a?(String)

      v
    end

    def self.need_ary(v)
      raise err("TypeError", "no implicit conversion of #{tname(v)} into Array") unless v.is_a?(Array)

      v
    end

    def self.need_hash(v)
      raise err("TypeError", "no implicit conversion of #{tname(v)} into Hash") unless v.is_a?(Hash)

      v
    end

    def self.need_num(v, into)
      return v if v.is_a?(Integer) || v.is_a?(Float)

      raise err("TypeError", "#{tname(v)} can't be coerced into #{into}")
    end

    def self.need_blk(b, name)
      raise err("ArgumentError", "no block given (#{name})") unless b

      b
    end

    def self.cmp_desc(v)
      case v
      when nil, true, false, Integer, Float, Symbol then v.inspect
      else class_name(v)
      end
    end

    def self.cmp_fail(a, b)
      raise err("ArgumentError", "comparison of #{class_name(a)} with #{cmp_desc(b)} failed")
    end

    def self.to_s(v)
      case v
      when String then v
      when nil then +""
      when Symbol then v.to_s
      when Array, Hash then inspect(v)
      else v.to_s
      end
    end

    # Symbol hash keys that print bare (`name: v`) in Ruby 3.4's Hash#inspect.
    BARE_KEY = /\A(?:[A-Za-z_]|[^\x00-\x7f])(?:[A-Za-z0-9_]|[^\x00-\x7f])*[?!]?\z/

    # Ruby 3.4 inspect for every template value, independent of the host
    # Ruby version (Hash#inspect changed format in 3.4).
    def self.inspect(v, seen = {}.compare_by_identity)
      case v
      when Array
        return "[...]" if seen[v]

        seen[v] = true
        out = "[#{v.map { |e| inspect(e, seen) }.join(", ")}]"
        seen.delete(v)
        out
      when Hash
        return "{...}" if seen[v]
        return "{}" if v.empty?

        seen[v] = true
        out = "{#{v.map { |k, val| "#{inspect_key(k, seen)} #{inspect(val, seen)}" }.join(", ")}}"
        seen.delete(v)
        out
      when Range
        "#{inspect(v.begin, seen) unless v.begin.nil?}#{v.exclude_end? ? "..." : ".."}#{inspect(v.end, seen) unless v.end.nil?}"
      when Enum
        v.inspect
      else
        v.inspect
      end
    end

    def self.inspect_key(k, seen)
      return "#{inspect(k, seen)} =>" unless k.is_a?(Symbol)

      name = k.to_s
      name.match?(BARE_KEY) ? "#{name}:" : "#{name.inspect}:"
    end

    def self.is_a(v, cls)
      raise err("TypeError", "class or module required") unless cls.is_a?(ClassValue)

      ANCESTORS.fetch(class_name(v), ["Object"]).include?(cls.name)
    end

    def self.case_eq(pat, v)
      case pat
      when ClassValue then is_a(v, pat)
      when Range
        begin
          pat === v # rubocop:disable Style/CaseEquality
        rescue ::StandardError
          false
        end
      else pat == v
      end
    end

    def self.make_range(lo, hi, excl)
      Range.new(lo, hi, excl)
    rescue ::ArgumentError
      raise err("ArgumentError", "bad value for range")
    end

    # -- integer arithmetic with 64-bit overflow checks ----------------------------

    def self.ipow(b, e)
      raise err("RangeError", "negative exponent: Integer ** negative Integer is not supported") if e.negative?
      return 1 if e.zero?
      return b if b.zero? || b == 1
      return e.even? ? 1 : -1 if b == -1
      raise err("RangeError", "integer overflow: result does not fit in 64 bits") if e > 64

      ck(b**e)
    end

    def self.shl(r, n)
      return shr(r, -n) if n.negative?
      return 0 if r.zero?
      raise err("RangeError", "integer overflow: result does not fit in 64 bits") if n >= 64

      ck(r << n)
    end

    def self.shr(r, n)
      return shl(r, -n) if n.negative?
      return r.negative? ? -1 : 0 if n >= 64

      r >> n
    end

    def self.fpow(r, o)
      res = r**o
      res.is_a?(Float) || res.is_a?(Integer) ? res : Float::NAN
    end

    # -- classes and constants -------------------------------------------------------

    ANCESTORS = {
      "NilClass" => %w[NilClass Object], "TrueClass" => %w[TrueClass Object],
      "FalseClass" => %w[FalseClass Object],
      "Integer" => %w[Integer Numeric Comparable Object], "Float" => %w[Float Numeric Comparable Object],
      "String" => %w[String Comparable Object], "Symbol" => %w[Symbol Comparable Object],
      "Array" => %w[Array Enumerable Object], "Hash" => %w[Hash Enumerable Object],
      "Range" => %w[Range Enumerable Object], "Enumerator" => %w[Enumerator Enumerable Object],
      "Class" => %w[Class Module Object], "Module" => %w[Module Object]
    }.freeze

    CLASS_NAMES = %w[Integer Float String Symbol Array Hash Range NilClass TrueClass FalseClass
                     Numeric Object Enumerator File Dir].freeze
    MODULE_NAMES = %w[Comparable Enumerable Math JSON].freeze
    CONSTANTS = (CLASS_NAMES.to_h { |n| [n, ClassValue.new(n, false)] }
                 .merge(MODULE_NAMES.to_h { |n| [n, ClassValue.new(n, true)] })
                 .merge("Class" => ClassValue.new("Class", false), "Module" => ClassValue.new("Module", false)))
                .freeze
    PUBLIC_CONSTANTS = (CLASS_NAMES + MODULE_NAMES).to_h { |n| [n, true] }.freeze

    SCOPED_CONSTANTS = {
      "Math" => { "PI" => Math::PI, "E" => Math::E },
      "Float" => { "INFINITY" => Float::INFINITY, "NAN" => Float::NAN, "EPSILON" => Float::EPSILON,
                   "MAX" => Float::MAX, "MIN" => Float::MIN }
    }.freeze

    def self.lookup_constant(name)
      PUBLIC_CONSTANTS[name] ? CONSTANTS[name] : nil
    end

    def self.scoped_constant(scope, name, filename, line)
      unless scope.is_a?(ClassValue)
        raise Error.new("TypeError", "#{scope.inspect} is not a class/module", filename, line)
      end

      table = SCOPED_CONSTANTS[scope.name]
      return table[name] if table&.key?(name)

      raise Error.new("NameError", "uninitialized constant #{scope.name}::#{name}", filename, line)
    end

    def self.class_of(v)
      CONSTANTS[class_name(v)]
    end

    # -- method lookup -----------------------------------------------------------------

    COMMON_T = {}
    NIL_T = {}
    BOOL_T = {}
    INT_T = {}
    FLOAT_T = {}
    STR_T = {}
    SYM_T = {}
    ARR_T = {}
    HASH_T = {}
    RANGE_T = {}
    ENUMR_T = {}
    ENUM_T = {}
    CLASS_T = {}
    CLASS_METHODS = Hash.new({}.freeze)
    FUNCS = {}

    def self.lookup(r, name)
      case r
      when String then STR_T[name] || COMMON_T[name]
      when Integer then INT_T[name] || COMMON_T[name]
      when Array then ARR_T[name] || ENUM_T[name] || COMMON_T[name]
      when Hash then HASH_T[name] || ENUM_T[name] || COMMON_T[name]
      when nil then NIL_T[name] || COMMON_T[name]
      when Float then FLOAT_T[name] || COMMON_T[name]
      when Symbol then SYM_T[name] || COMMON_T[name]
      when true, false then BOOL_T[name] || COMMON_T[name]
      when Range then RANGE_T[name] || ENUM_T[name] || COMMON_T[name]
      when Enum then ENUMR_T[name] || ENUM_T[name] || COMMON_T[name]
      when ClassValue then CLASS_METHODS[r.name][name] || CLASS_T[name] || COMMON_T[name]
      end
    end

    # Iterator methods that return an Enumerator when called without a block.
    ENUM_NO_BLOCK = %w[each each_pair each_with_index each_with_object each_slice each_cons map collect
                       flat_map collect_concat select filter reject filter_map find_all
                       each_key each_value each_index reverse_each sort_by
                       min_by max_by group_by partition delete_if find detect find_index index rindex
                       transform_values transform_keys take_while drop_while with_index with_object
                       step].to_h { |n| [n, true] }.freeze
    INT_ENUM_NO_BLOCK = %w[times upto downto step].to_h { |n| [n, true] }.freeze
    STR_ENUM_NO_BLOCK = %w[each_char each_line].to_h { |n| [n, true] }.freeze
    ARG_FORMS = %w[find_index index rindex transform_keys].to_h { |n| [n, true] }.freeze

    def self.enumerator_call?(recv, name, args)
      case recv
      when Array, Hash, Range, Enum
        return false unless ENUM_NO_BLOCK[name]
        return false if name == "each" && recv.is_a?(Enum)
        return false if ARG_FORMS[name] && !args.empty?

        if name == "each_slice" || name == "each_cons"
          n = need_intish(args[0])
          raise err("ArgumentError", "invalid #{name == "each_slice" ? "slice " : ""}size") unless n.positive?
        end
        true
      when Integer then INT_ENUM_NO_BLOCK[name] || name == "then"
      when String then STR_ENUM_NO_BLOCK[name] || name == "then"
      else name == "then"
      end
    end

    # ===================================================================================
    # All values
    # ===================================================================================

    defm(COMMON_T, "==", 1) { |_, r, a| r == a[0] }
    defm(COMMON_T, "!=", 1) { |_, r, a| r != a[0] }
    defm(COMMON_T, "!", 0) { |it, r| !it.truthy(r) }
    defm(COMMON_T, "nil?", 0) { |_, r| r.nil? }
    defm(COMMON_T, "to_s", 0) { |_, r| Builtins.to_s(r) }
    defm(COMMON_T, "inspect", 0) { |_, r| Builtins.inspect(r) }
    defm(COMMON_T, "class", 0) { |_, r| Builtins.class_of(r) }
    defm(COMMON_T, %w[is_a? kind_of?], 1) { |_, r, a| Builtins.is_a(r, a[0]) }
    defm(COMMON_T, "instance_of?", 1) do |_, r, a|
      raise Builtins.err("TypeError", "class or module required") unless a[0].is_a?(ClassValue)

      Builtins.class_name(r) == a[0].name
    end
    defm(COMMON_T, "respond_to?", 1, 2) do |it, r, a|
      n = a[0]
      unless n.is_a?(Symbol) || n.is_a?(String)
        raise Builtins.err("TypeError", "#{n.inspect} is not a symbol nor a string")
      end

      it.responds?(r, n.to_s)
    end
    defm(COMMON_T, "dup", 0) { |_, r| r.is_a?(String) || r.is_a?(Array) || r.is_a?(Hash) ? r.dup : r }
    defm(COMMON_T, "freeze", 0) do |_, r|
      r.freeze if r.is_a?(String) || r.is_a?(Array) || r.is_a?(Hash)
      r
    end
    defm(COMMON_T, "frozen?", 0) { |_, r| r.is_a?(ClassValue) || r.is_a?(Enum) ? false : r.frozen? }
    defm(COMMON_T, "to_json", 0, -1) { |_, r| JSONSupport.generate(r) }
    defm(COMMON_T, "then", 0) { |_, r, _, b| b.call(r) }
    defm(COMMON_T, "tap", 0) do |_, r, _, b|
      Builtins.need_blk(b, "tap").call(r)
      r
    end
    defm(COMMON_T, "eql?", 1) { |_, r, a| r.eql?(a[0]) }
    defm(COMMON_T, "equal?", 1) { |_, r, a| r.equal?(a[0]) }
    defm(COMMON_T, "===", 1) { |_, r, a| Builtins.case_eq(r, a[0]) }
    defm(COMMON_T, "<=>", 1) { |_, r, a| r == a[0] ? 0 : nil }

    # ===================================================================================
    # nil, true, false
    # ===================================================================================

    defm(NIL_T, "to_a", 0) { [] }
    defm(NIL_T, "to_i", 0) { 0 }
    defm(NIL_T, "to_f", 0) { 0.0 }
    defm(NIL_T, "to_h", 0) { {} }
    defm(NIL_T, "&", 1) { false }
    defm(NIL_T, %w[| ^], 1) { |it, _, a| it.truthy(a[0]) }

    defm(BOOL_T, "&", 1) { |it, r, a| r ? it.truthy(a[0]) : false }
    defm(BOOL_T, "|", 1) { |it, r, a| r ? true : it.truthy(a[0]) }
    defm(BOOL_T, "^", 1) { |it, r, a| r ? !it.truthy(a[0]) : it.truthy(a[0]) }

    # ===================================================================================
    # Integer
    # ===================================================================================

    defm(INT_T, "+", 1) do |_, r, a|
      o = a[0]
      o.is_a?(Integer) ? Builtins.ck(r + o) : r + Builtins.need_num(o, "Integer")
    end
    defm(INT_T, "-", 1) do |_, r, a|
      o = a[0]
      o.is_a?(Integer) ? Builtins.ck(r - o) : r - Builtins.need_num(o, "Integer")
    end
    defm(INT_T, "*", 1) do |_, r, a|
      o = a[0]
      o.is_a?(Integer) ? Builtins.ck(r * o) : r * Builtins.need_num(o, "Integer")
    end
    defm(INT_T, "/", 1) do |_, r, a|
      o = a[0]
      if o.is_a?(Integer)
        raise Builtins.err("ZeroDivisionError", "divided by 0") if o.zero?

        Builtins.ck(r / o)
      else
        r / Builtins.need_num(o, "Integer")
      end
    end
    defm(INT_T, %w[% modulo], 1) do |_, r, a|
      o = a[0]
      if o.is_a?(Integer)
        raise Builtins.err("ZeroDivisionError", "divided by 0") if o.zero?

        r % o
      else
        r % Builtins.need_num(o, "Integer")
      end
    end
    defm(INT_T, "**", 1) do |_, r, a|
      o = a[0]
      o.is_a?(Integer) ? Builtins.ipow(r, o) : Builtins.fpow(r, Builtins.need_num(o, "Integer"))
    end
    defm(INT_T, "-@", 0) { |_, r| Builtins.ck(-r) }
    defm(INT_T, "+@", 0) { |_, r| r }
    defm(INT_T, "~", 0) { |_, r| ~r }
    defm(INT_T, "&", 1) { |_, r, a| r & Builtins.need_int(a[0]) }
    defm(INT_T, "|", 1) { |_, r, a| r | Builtins.need_int(a[0]) }
    defm(INT_T, "^", 1) { |_, r, a| r ^ Builtins.need_int(a[0]) }
    defm(INT_T, "<<", 1) { |_, r, a| Builtins.shl(r, Builtins.need_intish(a[0])) }
    defm(INT_T, ">>", 1) { |_, r, a| Builtins.shr(r, Builtins.need_intish(a[0])) }
    defm(INT_T, "<=>", 1) { |_, r, a| a[0].is_a?(Integer) || a[0].is_a?(Float) ? r <=> a[0] : nil }
    defm(INT_T, "<", 1) { |_, r, a| a[0].is_a?(Integer) || a[0].is_a?(Float) ? r < a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(INT_T, "<=", 1) { |_, r, a| a[0].is_a?(Integer) || a[0].is_a?(Float) ? r <= a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(INT_T, ">", 1) { |_, r, a| a[0].is_a?(Integer) || a[0].is_a?(Float) ? r > a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(INT_T, ">=", 1) { |_, r, a| a[0].is_a?(Integer) || a[0].is_a?(Float) ? r >= a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(INT_T, "abs", 0) { |_, r| Builtins.ck(r.abs) }
    defm(INT_T, %w[succ next], 0) { |_, r| Builtins.ck(r + 1) }
    defm(INT_T, "pred", 0) { |_, r| Builtins.ck(r - 1) }
    defm(INT_T, %w[to_i to_int ord], 0) { |_, r| r }
    defm(INT_T, "to_f", 0) { |_, r| r.to_f }
    defm(INT_T, %w[to_s inspect], 0, 1) do |_, r, a|
      base = a.empty? ? 10 : Builtins.need_intish(a[0])
      raise Builtins.err("ArgumentError", "invalid radix #{base}") unless base.between?(2, 36)

      r.to_s(base)
    end
    defm(INT_T, "chr", 0) do |_, r|
      raise Builtins.err("RangeError", "#{r} out of char range") unless r.between?(0, 255)

      r.chr(Encoding::UTF_8)
    end
    defm(INT_T, "times", 0) { |it, r, _, b| r.times(&it.bp(b)) }
    defm(INT_T, "upto", 1) { |it, r, a, b| r.upto(a[0], &it.bp(b)) }
    defm(INT_T, "downto", 1) { |it, r, a, b| r.downto(a[0], &it.bp(b)) }
    defm(INT_T, "step", 1, 2) { |it, r, a, b| r.step(a[0], a.length > 1 ? a[1] : 1, &it.bp(b)) }
    defm(INT_T, "even?", 0) { |_, r| r.even? }
    defm(INT_T, "odd?", 0) { |_, r| r.odd? }
    defm(INT_T, "zero?", 0) { |_, r| r.zero? }
    defm(INT_T, "positive?", 0) { |_, r| r.positive? }
    defm(INT_T, "negative?", 0) { |_, r| r.negative? }
    defm(INT_T, "between?", 2) { |_, r, a| r.between?(a[0], a[1]) }
    defm(INT_T, "clamp", 1, 2) { |_, r, a| a.length == 1 ? r.clamp(a[0]) : r.clamp(a[0], a[1]) }
    defm(INT_T, "divmod", 1) do |_, r, a|
      o = a[0]
      raise Builtins.err("ZeroDivisionError", "divided by 0") if o.is_a?(Integer) && o.zero?

      r.divmod(Builtins.need_num(o, "Integer")).map { |x| Builtins.ck(x) }
    end
    defm(INT_T, "fdiv", 1) { |_, r, a| r.fdiv(Builtins.need_num(a[0], "Integer")) }
    defm(INT_T, "gcd", 1) { |_, r, a| Builtins.ck(r.gcd(Builtins.need_int(a[0]))) }
    defm(INT_T, "lcm", 1) { |_, r, a| Builtins.ck(r.lcm(Builtins.need_int(a[0]))) }
    defm(INT_T, "digits", 0, 1) { |_, r, a| a.empty? ? r.digits : r.digits(Builtins.need_intish(a[0])) }
    defm(INT_T, "bit_length", 0) { |_, r| r.bit_length }
    defm(INT_T, "floor", 0, 1) { |_, r, a| Builtins.ck(a.empty? ? r : r.floor(Builtins.need_intish(a[0]))) }
    defm(INT_T, "ceil", 0, 1) { |_, r, a| Builtins.ck(a.empty? ? r : r.ceil(Builtins.need_intish(a[0]))) }
    defm(INT_T, "truncate", 0, 1) { |_, r, a| Builtins.ck(a.empty? ? r : r.truncate(Builtins.need_intish(a[0]))) }
    defm(INT_T, "round", 0, 1) { |_, r, a| Builtins.ck(a.empty? ? r : r.round(Builtins.need_intish(a[0]))) }

    # ===================================================================================
    # Float
    # ===================================================================================

    defm(FLOAT_T, "+", 1) { |_, r, a| r + Builtins.need_num(a[0], "Float") }
    defm(FLOAT_T, "-", 1) { |_, r, a| r - Builtins.need_num(a[0], "Float") }
    defm(FLOAT_T, "*", 1) { |_, r, a| r * Builtins.need_num(a[0], "Float") }
    defm(FLOAT_T, "/", 1) { |_, r, a| r / Builtins.need_num(a[0], "Float") }
    defm(FLOAT_T, %w[% modulo], 1) { |_, r, a| r % Builtins.need_num(a[0], "Float") }
    defm(FLOAT_T, "**", 1) { |_, r, a| Builtins.fpow(r, Builtins.need_num(a[0], "Float")) }
    defm(FLOAT_T, "-@", 0) { |_, r| -r }
    defm(FLOAT_T, "+@", 0) { |_, r| r }
    defm(FLOAT_T, "<=>", 1) { |_, r, a| a[0].is_a?(Integer) || a[0].is_a?(Float) ? r <=> a[0] : nil }
    defm(FLOAT_T, "<", 1) { |_, r, a| a[0].is_a?(Integer) || a[0].is_a?(Float) ? r < a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(FLOAT_T, "<=", 1) { |_, r, a| a[0].is_a?(Integer) || a[0].is_a?(Float) ? r <= a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(FLOAT_T, ">", 1) { |_, r, a| a[0].is_a?(Integer) || a[0].is_a?(Float) ? r > a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(FLOAT_T, ">=", 1) { |_, r, a| a[0].is_a?(Integer) || a[0].is_a?(Float) ? r >= a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(FLOAT_T, "abs", 0) { |_, r| r.abs }
    defm(FLOAT_T, "to_f", 0) { |_, r| r }
    defm(FLOAT_T, %w[to_s inspect], 0) { |_, r| r.to_s }
    defm(FLOAT_T, "nan?", 0) { |_, r| r.nan? }
    defm(FLOAT_T, "finite?", 0) { |_, r| r.finite? }
    defm(FLOAT_T, "infinite?", 0) { |_, r| r.infinite? }
    defm(FLOAT_T, "zero?", 0) { |_, r| r.zero? }
    defm(FLOAT_T, "positive?", 0) { |_, r| r.positive? }
    defm(FLOAT_T, "negative?", 0) { |_, r| r.negative? }
    defm(FLOAT_T, "between?", 2) { |_, r, a| r.between?(a[0], a[1]) }
    defm(FLOAT_T, "clamp", 1, 2) { |_, r, a| a.length == 1 ? r.clamp(a[0]) : r.clamp(a[0], a[1]) }
    defm(FLOAT_T, "divmod", 1) { |_, r, a| r.divmod(Builtins.need_num(a[0], "Float")).map { |x| Builtins.ck(x) } }
    defm(FLOAT_T, "fdiv", 1) { |_, r, a| r.fdiv(Builtins.need_num(a[0], "Float")) }
    defm(FLOAT_T, %w[to_i to_int], 0) { |_, r| Builtins.ck(r.to_i) }
    defm(FLOAT_T, "truncate", 0, 1) { |_, r, a| Builtins.ck(a.empty? ? r.truncate : r.truncate(Builtins.need_intish(a[0]))) }
    defm(FLOAT_T, "floor", 0, 1) { |_, r, a| Builtins.ck(a.empty? ? r.floor : r.floor(Builtins.need_intish(a[0]))) }
    defm(FLOAT_T, "ceil", 0, 1) { |_, r, a| Builtins.ck(a.empty? ? r.ceil : r.ceil(Builtins.need_intish(a[0]))) }
    defm(FLOAT_T, "round", 0, 1) { |_, r, a| Builtins.ck(a.empty? ? r.round : r.round(Builtins.need_intish(a[0]))) }

    # ===================================================================================
    # String (no method changes a string in place)
    # ===================================================================================

    def self.str_index_args(a)
      if a.length == 2
        [need_intish(a[0]), need_intish(a[1])]
      else
        x = a[0]
        return [x] if x.is_a?(Range)

        [need_intish(x)]
      end
    end

    defm(STR_T, %w[[] slice], 1, 2) do |_, r, a|
      idx = Builtins.str_index_args(a)
      idx.length == 2 ? r[idx[0], idx[1]] : r[idx[0]]
    end
    defm(STR_T, "+", 1) { |_, r, a| r + Builtins.need_str(a[0]) }
    defm(STR_T, "+@", 0) { |_, r| r.frozen? ? r.dup : r }
    defm(STR_T, "*", 1) do |_, r, a|
      n = Builtins.need_intish(a[0])
      Builtins.cap(r.length * n)
      r * n
    end
    defm(STR_T, "%", 1) { |it, r, a| Format.format(it, r, a[0].is_a?(Array) ? a[0] : [a[0]]) }
    # Strings are values [ERBLite]: returns a new string (the interpreter
    # assigns it back). An Integer appends that code point.
    defm(STR_T, "<<", 1) do |_, r, a|
      o = a[0]
      case o
      when String then r + o
      when Integer
        raise Builtins.err("RangeError", "#{o} out of char range") unless o.between?(0, 0x10FFFF)

        r + o.chr(Encoding::UTF_8)
      else raise Builtins.err("TypeError", "no implicit conversion of #{Builtins.tname(o)} into String")
      end
    end
    defm(STR_T, "<=>", 1) { |_, r, a| a[0].is_a?(String) ? r <=> a[0] : nil }
    defm(STR_T, "<", 1) { |_, r, a| a[0].is_a?(String) ? r < a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(STR_T, "<=", 1) { |_, r, a| a[0].is_a?(String) ? r <= a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(STR_T, ">", 1) { |_, r, a| a[0].is_a?(String) ? r > a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(STR_T, ">=", 1) { |_, r, a| a[0].is_a?(String) ? r >= a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(STR_T, %w[length size], 0) { |_, r| r.length }
    defm(STR_T, "bytesize", 0) { |_, r| r.bytesize }
    defm(STR_T, "empty?", 0) { |_, r| r.empty? }
    defm(STR_T, "upcase", 0) { |_, r| r.upcase }
    defm(STR_T, "downcase", 0) { |_, r| r.downcase }
    defm(STR_T, "capitalize", 0) { |_, r| r.capitalize }
    defm(STR_T, "swapcase", 0) { |_, r| r.swapcase }
    defm(STR_T, "strip", 0) { |_, r| r.strip }
    defm(STR_T, "lstrip", 0) { |_, r| r.lstrip }
    defm(STR_T, "rstrip", 0) { |_, r| r.rstrip }
    defm(STR_T, "chomp", 0, 1) { |_, r, a| a.empty? || a[0].nil? ? r.chomp : r.chomp(Builtins.need_str(a[0])) }
    defm(STR_T, "chop", 0) { |_, r| r.chop }
    defm(STR_T, "chars", 0) { |it, r, _, b| b ? Builtins.yield_each(it, r.chars, b, r) : r.chars }
    defm(STR_T, "lines", 0, 1) do |it, r, a, b|
      res = a.empty? ? r.lines : r.lines(Builtins.need_str(a[0]))
      b ? Builtins.yield_each(it, res, b, r) : res
    end
    defm(STR_T, "bytes", 0) { |it, r, _, b| b ? Builtins.yield_each(it, r.bytes, b, r) : r.bytes }
    defm(STR_T, "reverse", 0) { |_, r| r.reverse }
    defm(STR_T, "include?", 1) { |_, r, a| r.include?(Builtins.need_str(a[0])) }
    defm(STR_T, "start_with?", 0, -1) { |_, r, a| a.any? { |s| r.start_with?(Builtins.need_str(s)) } }
    defm(STR_T, "end_with?", 0, -1) { |_, r, a| a.any? { |s| r.end_with?(Builtins.need_str(s)) } }
    defm(STR_T, "index", 1, 2) do |_, r, a|
      a.length == 1 ? r.index(Builtins.need_str(a[0])) : r.index(Builtins.need_str(a[0]), Builtins.need_intish(a[1]))
    end
    defm(STR_T, "rindex", 1, 2) do |_, r, a|
      a.length == 1 ? r.rindex(Builtins.need_str(a[0])) : r.rindex(Builtins.need_str(a[0]), Builtins.need_intish(a[1]))
    end
    # [ERBLite] counts non-overlapping occurrences of the substring.
    defm(STR_T, "count", 1) do |_, r, a|
      s = Builtins.need_str(a[0])
      if s.empty?
        0
      else
        n = 0
        i = 0
        while (j = r.index(s, i))
          n += 1
          i = j + s.length
        end
        n
      end
    end
    defm(STR_T, "sub", 1, 2) { |it, r, a, b| Builtins.str_sub(it, r, a, b, false) }
    defm(STR_T, "gsub", 1, 2) { |it, r, a, b| Builtins.str_sub(it, r, a, b, true) }
    defm(STR_T, "tr", 2) { |_, r, a| r.tr(Builtins.need_str(a[0]), Builtins.need_str(a[1])) }
    defm(STR_T, "delete", 1, -1) { |_, r, a| r.delete(*a.map { |s| Builtins.need_str(s) }) }
    defm(STR_T, "squeeze", 0, -1) { |_, r, a| r.squeeze(*a.map { |s| Builtins.need_str(s) }) }
    %w[center ljust rjust].each do |nm|
      defm(STR_T, nm, 1, 2) do |_, r, a|
        w = Builtins.cap(Builtins.need_intish(a[0]))
        pad = a.length > 1 ? Builtins.need_str(a[1]) : " "
        case nm
        when "center" then r.center(w, pad)
        when "ljust" then r.ljust(w, pad)
        else r.rjust(w, pad)
        end
      end
    end
    defm(STR_T, "to_i", 0, 1) { |_, r, a| Builtins.ck(r.to_i(a.empty? ? 10 : Builtins.need_int(a[0]))) }
    defm(STR_T, "to_f", 0) { |_, r| r.to_f }
    defm(STR_T, %w[to_s to_str], 0) { |_, r| r }
    defm(STR_T, %w[to_sym intern], 0) { |_, r| r.to_sym }
    defm(STR_T, "ord", 0) { |_, r| r.ord }
    defm(STR_T, "hex", 0) { |_, r| Builtins.ck(r.hex) }
    defm(STR_T, "oct", 0) { |_, r| Builtins.ck(r.oct) }
    defm(STR_T, "each_char", 0) { |it, r, _, b| r.each_char(&it.bp(b)) }
    defm(STR_T, "each_line", 0, 1) do |it, r, a, b|
      a.empty? ? r.each_line(&it.bp(b)) : r.each_line(Builtins.need_str(a[0]), &it.bp(b))
    end
    defm(STR_T, "split", 0, 2) do |it, r, a, b|
      sep = a.empty? || a[0].nil? ? nil : Builtins.need_str(a[0])
      limit = a.length > 1 ? Builtins.need_intish(a[1]) : 0
      res = r.split(sep, limit)
      b ? Builtins.yield_each(it, res, b, r) : res
    end

    def self.yield_each(_it, list, blk, result)
      list.each { |x| blk.call(x) }
      result
    end
    defm(STR_T, "partition", 1) { |_, r, a| r.partition(Builtins.need_str(a[0])) }
    defm(STR_T, "rpartition", 1) { |_, r, a| r.rpartition(Builtins.need_str(a[0])) }
    defm(STR_T, "casecmp", 1) { |_, r, a| a[0].is_a?(String) ? r.casecmp(a[0]) : nil }
    defm(STR_T, "casecmp?", 1) { |_, r, a| a[0].is_a?(String) ? r.casecmp?(a[0]) : nil }
    defm(STR_T, "between?", 2) { |_, r, a| r.between?(a[0], a[1]) }
    defm(STR_T, "ascii_only?", 0) { |_, r| r.ascii_only? }

    # sub/gsub with a literal String pattern [ERBLite: no Regexp].
    def self.str_sub(it, r, a, b, global)
      pat = a[0]
      unless pat.is_a?(String)
        raise err("TypeError", "wrong argument type #{class_name(pat)} (expected String; regular expressions are not supported)")
      end

      rep = a[1]
      if a.length == 1 && b.nil?
        raise err("ArgumentError", "wrong number of arguments (given 1, expected 2)")
      end
      if a.length == 2 && !rep.is_a?(String) && !rep.is_a?(Hash)
        raise err("TypeError", "no implicit conversion of #{tname(rep)} into String")
      end

      out = +""
      pos = 0
      len = r.length
      while (idx = r.index(pat, pos))
        out << r[pos...idx]
        repl = if a.length == 2
                 rep.is_a?(Hash) ? to_s(rep[pat]) : expand_replacement(rep, pat, r[0...idx], r[(idx + pat.length)..])
               else
                 to_s(b.call(pat.dup))
               end
        out << repl
        if pat.empty?
          out << r[idx] if idx < len
          pos = idx + 1
        else
          pos = idx + pat.length
        end
        break if !global || pos > len
      end
      out << r[pos..] if pos <= len
      out
    end

    def self.expand_replacement(rep, m, pre, post)
      return rep unless rep.include?("\\")

      out = +""
      i = 0
      n = rep.length
      while i < n
        c = rep[i]
        if c == "\\" && i + 1 < n
          d = rep[i + 1]
          case d
          when "0", "&" then out << m
          when "`" then out << pre
          when "'" then out << post
          when "\\" then out << "\\"
          when "1", "2", "3", "4", "5", "6", "7", "8", "9" then nil
          else out << c << d
          end
          i += 2
        else
          out << c
          i += 1
        end
      end
      out
    end

    # ===================================================================================
    # Symbol
    # ===================================================================================

    defm(SYM_T, %w[to_s id2name], 0) { |_, r| r.to_s }
    defm(SYM_T, "name", 0) { |_, r| r.name }
    defm(SYM_T, "to_sym", 0) { |_, r| r }
    defm(SYM_T, %w[length size], 0) { |_, r| r.length }
    defm(SYM_T, "upcase", 0) { |_, r| r.upcase }
    defm(SYM_T, "downcase", 0) { |_, r| r.downcase }
    defm(SYM_T, "capitalize", 0) { |_, r| r.capitalize }
    defm(SYM_T, "<=>", 1) { |_, r, a| a[0].is_a?(Symbol) ? r <=> a[0] : nil }
    defm(SYM_T, "<", 1) { |_, r, a| a[0].is_a?(Symbol) ? r < a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(SYM_T, "<=", 1) { |_, r, a| a[0].is_a?(Symbol) ? r <= a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(SYM_T, ">", 1) { |_, r, a| a[0].is_a?(Symbol) ? r > a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(SYM_T, ">=", 1) { |_, r, a| a[0].is_a?(Symbol) ? r >= a[0] : Builtins.cmp_fail(r, a[0]) }
    defm(SYM_T, "between?", 2) { |_, r, a| r.between?(a[0], a[1]) }
    defm(SYM_T, "start_with?", 0, -1) { |_, r, a| a.any? { |s| r.start_with?(Builtins.need_str(s)) } }
    defm(SYM_T, "end_with?", 0, -1) { |_, r, a| a.any? { |s| r.end_with?(Builtins.need_str(s)) } }
    defm(SYM_T, "empty?", 0) { |_, r| r.empty? }

    # ===================================================================================
    # Array
    # ===================================================================================

    def self.ary_index_args(a)
      if a.length == 2
        [need_intish(a[0]), need_intish(a[1])]
      else
        x = a[0]
        return [x] if x.is_a?(Range)

        [need_intish(x)]
      end
    end

    defm(ARR_T, %w[[] slice], 1, 2) do |_, r, a|
      idx = Builtins.ary_index_args(a)
      idx.length == 2 ? r[idx[0], idx[1]] : r[idx[0]]
    end
    defm(ARR_T, "[]=", 2, 3) do |_, r, a|
      if a.length == 3
        r[Builtins.cap(Builtins.need_intish(a[0])), Builtins.need_intish(a[1])] = a[2]
      elsif a[0].is_a?(Range)
        Builtins.cap(a[0].begin) if a[0].begin.is_a?(Integer)
        r[a[0]] = a[1]
      else
        r[Builtins.cap(Builtins.need_intish(a[0]))] = a[1]
      end
      a.last
    end
    defm(ARR_T, "+", 1) { |_, r, a| r + Builtins.need_ary(a[0]) }
    defm(ARR_T, "-", 1) { |_, r, a| r - Builtins.need_ary(a[0]) }
    defm(ARR_T, "&", 1) { |_, r, a| r & Builtins.need_ary(a[0]) }
    defm(ARR_T, "|", 1) { |_, r, a| r | Builtins.need_ary(a[0]) }
    defm(ARR_T, "*", 1) do |_, r, a|
      o = a[0]
      if o.is_a?(String)
        r.join(o)
      else
        n = Builtins.need_intish(o)
        Builtins.cap(r.length * n)
        r * n
      end
    end
    defm(ARR_T, "<<", 1) { |_, r, a| r << a[0] }
    defm(ARR_T, "<=>", 1) { |_, r, a| a[0].is_a?(Array) ? r <=> a[0] : nil }
    defm(ARR_T, %w[length size], 0) { |_, r| r.length }
    defm(ARR_T, "empty?", 0) { |_, r| r.empty? }
    defm(ARR_T, "first", 0, 1) { |_, r, a| a.empty? ? r.first : r.first(Builtins.need_intish(a[0])) }
    defm(ARR_T, "last", 0, 1) { |_, r, a| a.empty? ? r.last : r.last(Builtins.need_intish(a[0])) }
    defm(ARR_T, %w[push append], 0, -1) { |_, r, a| r.push(*a) }
    defm(ARR_T, %w[unshift prepend], 0, -1) { |_, r, a| r.unshift(*a) }
    defm(ARR_T, "insert", 1, -1) { |_, r, a| r.insert(Builtins.cap(Builtins.need_intish(a[0])), *a[1..]) }
    defm(ARR_T, "pop", 0, 1) { |_, r, a| a.empty? ? r.pop : r.pop(Builtins.need_intish(a[0])) }
    defm(ARR_T, "shift", 0, 1) { |_, r, a| a.empty? ? r.shift : r.shift(Builtins.need_intish(a[0])) }
    defm(ARR_T, "concat", 0, -1) { |_, r, a| r.concat(*a.map { |x| Builtins.need_ary(x) }) }
    defm(ARR_T, "delete", 1) { |it, r, a, b| r.delete(a[0], &it.bp(b)) }
    defm(ARR_T, "delete_at", 1) { |_, r, a| r.delete_at(Builtins.need_intish(a[0])) }
    defm(ARR_T, "delete_if", 0) { |it, r, _, b| r.delete_if(&it.bp(b)) }
    defm(ARR_T, "clear", 0) { |_, r| r.clear }
    defm(ARR_T, "fill", 0, 3) do |it, r, a, b|
      if b
        r.fill(*a.map { |x| x.is_a?(Range) || x.nil? ? x : Builtins.need_intish(x) }, &it.bp(b))
      else
        raise Builtins.err("ArgumentError", "wrong number of arguments (given 0, expected 1..3)") if a.empty?

        rest = a[1..].map { |x| x.is_a?(Range) || x.nil? ? x : Builtins.need_intish(x) }
        r.fill(a[0], *rest)
      end
    end
    defm(ARR_T, %w[index find_index], 0, 1) { |it, r, a, b| a.empty? ? r.index(&it.bp(b)) : r.index(a[0]) }
    defm(ARR_T, "rindex", 0, 1) { |it, r, a, b| a.empty? ? r.rindex(&it.bp(b)) : r.rindex(a[0]) }
    defm(ARR_T, "join", 0, 1) do |_, r, a|
      sep = a.empty? || a[0].nil? ? "" : Builtins.need_str(a[0])
      r.join(sep)
    end
    defm(ARR_T, "reverse", 0) { |_, r| r.reverse }
    defm(ARR_T, "rotate", 0, 1) { |_, r, a| r.rotate(a.empty? ? 1 : Builtins.need_intish(a[0])) }
    defm(ARR_T, "flatten", 0, 1) { |_, r, a| a.empty? || a[0].nil? ? r.flatten : r.flatten(Builtins.need_intish(a[0])) }
    defm(ARR_T, "compact", 0) { |_, r| r.compact }
    defm(ARR_T, "transpose", 0) { |_, r| r.transpose }
    defm(ARR_T, "values_at", 0, -1) do |_, r, a|
      r.values_at(*a.map { |x| x.is_a?(Range) ? x : Builtins.need_intish(x) })
    end
    defm(ARR_T, "dig", 1, -1) { |_, r, a| r.dig(*a) }
    defm(ARR_T, "assoc", 1) { |_, r, a| r.assoc(a[0]) }
    # [ERBLite] product/combination/permutation return an Array without a block.
    defm(ARR_T, "product", 0, -1) do |it, r, a, b|
      others = a.map { |x| Builtins.need_ary(x) }
      if b
        r.product(*others) { |c| b.call(c) }
        r
      else
        res = []
        r.product(*others) do |c|
          it.step!
          res << c
        end
        res
      end
    end
    defm(ARR_T, "combination", 1) do |it, r, a, b|
      n = Builtins.need_intish(a[0])
      if b
        r.combination(n) { |c| b.call(c) }
        r
      else
        res = []
        r.combination(n) do |c|
          it.step!
          res << c
        end
        res
      end
    end
    defm(ARR_T, "permutation", 0, 1) do |it, r, a, b|
      n = a.empty? || a[0].nil? ? r.length : Builtins.need_intish(a[0])
      if b
        r.permutation(n) { |c| b.call(c) }
        r
      else
        res = []
        r.permutation(n) do |c|
          it.step!
          res << c
        end
        res
      end
    end
    defm(ARR_T, "to_a", 0) { |_, r| r }
    defm(ARR_T, "each", 0) { |it, r, _, b| r.each(&it.bp(b)) }
    defm(ARR_T, "each_index", 0) { |it, r, _, b| r.each_index(&it.bp(b)) }
    defm(ARR_T, "reverse_each", 0) { |it, r, _, b| r.reverse_each(&it.bp(b)) }

    # ===================================================================================
    # Hash
    # ===================================================================================

    defm(HASH_T, "[]", 1) { |_, r, a| r[a[0]] }
    defm(HASH_T, %w[[]= store], 2) { |_, r, a| r[a[0]] = a[1] }
    defm(HASH_T, "fetch", 1, 2) do |it, r, a, b|
      if a.length == 2 then r.fetch(a[0], a[1])
      elsif b then r.fetch(a[0], &it.bp(b))
      else r.fetch(a[0])
      end
    end
    defm(HASH_T, "dig", 1, -1) { |_, r, a| r.dig(*a) }
    defm(HASH_T, %w[key? has_key? include? member?], 1) { |_, r, a| r.key?(a[0]) }
    defm(HASH_T, %w[value? has_value?], 1) { |_, r, a| r.value?(a[0]) }
    defm(HASH_T, "key", 1) { |_, r, a| r.key(a[0]) }
    defm(HASH_T, "keys", 0) { |_, r| r.keys }
    defm(HASH_T, "values", 0) { |_, r| r.values }
    defm(HASH_T, "values_at", 0, -1) { |_, r, a| r.values_at(*a) }
    defm(HASH_T, "fetch_values", 0, -1) { |it, r, a, b| r.fetch_values(*a, &it.bp(b)) }
    defm(HASH_T, %w[length size], 0) { |_, r| r.length }
    defm(HASH_T, "empty?", 0) { |_, r| r.empty? }
    defm(HASH_T, "delete", 1) { |it, r, a, b| r.delete(a[0], &it.bp(b)) }
    defm(HASH_T, "delete_if", 0) { |it, r, _, b| r.delete_if(&it.bp(b)) }
    defm(HASH_T, "clear", 0) { |_, r| r.clear }
    defm(HASH_T, "merge", 0, -1) { |it, r, a, b| r.merge(*a.map { |h| Builtins.need_hash(h) }, &it.bp(b)) }
    defm(HASH_T, %w[merge! update], 0, -1) { |it, r, a, b| r.merge!(*a.map { |h| Builtins.need_hash(h) }, &it.bp(b)) }
    defm(HASH_T, "to_a", 0) { |_, r| r.to_a }
    defm(HASH_T, "to_h", 0) { |it, r, _, b| b ? r.to_h(&it.bp(b)) : r }
    defm(HASH_T, "invert", 0) { |_, r| r.invert }
    defm(HASH_T, "transform_values", 0) { |it, r, _, b| r.transform_values(&it.bp(b)) }
    defm(HASH_T, "transform_keys", 0, 1) do |it, r, a, b|
      a.empty? ? r.transform_keys(&it.bp(b)) : r.transform_keys(Builtins.need_hash(a[0]), &it.bp(b))
    end
    defm(HASH_T, %w[select filter], 0) { |it, r, _, b| r.select(&it.bp(b)) }
    defm(HASH_T, "reject", 0) { |it, r, _, b| r.reject(&it.bp(b)) }
    defm(HASH_T, %w[each each_pair], 0) { |it, r, _, b| r.each(&it.bp(b)) }
    defm(HASH_T, "each_key", 0) { |it, r, _, b| r.each_key(&it.bp(b)) }
    defm(HASH_T, "each_value", 0) { |it, r, _, b| r.each_value(&it.bp(b)) }
    defm(HASH_T, "slice", 0, -1) { |_, r, a| r.slice(*a) }
    defm(HASH_T, "except", 0, -1) { |_, r, a| r.except(*a) }
    defm(HASH_T, "compact", 0) { |_, r| r.compact }
    defm(HASH_T, "default", 0) { |_, r| r.default }
    defm(HASH_T, "default=", 1) { |_, r, a| r.default = a[0] }

    # ===================================================================================
    # Range
    # ===================================================================================

    defm(RANGE_T, "first", 0, 1) { |_, r, a| a.empty? ? r.first : r.first(Builtins.need_intish(a[0])) }
    defm(RANGE_T, "begin", 0) { |_, r| r.begin }
    defm(RANGE_T, "last", 0, 1) { |_, r, a| a.empty? ? r.last : r.last(Builtins.need_intish(a[0])) }
    defm(RANGE_T, "end", 0) { |_, r| r.end }
    defm(RANGE_T, "min", 0, 1) { |it, r, a, b| a.empty? ? r.min(&it.bp(b)) : r.min(Builtins.need_intish(a[0]), &it.bp(b)) }
    defm(RANGE_T, "max", 0, 1) { |it, r, a, b| a.empty? ? r.max(&it.bp(b)) : r.max(Builtins.need_intish(a[0]), &it.bp(b)) }
    defm(RANGE_T, "size", 0) { |_, r| r.size }
    defm(RANGE_T, "sum", 0, 1) { |it, r, a, b| Builtins.ck(r.sum(*a, &it.bp(b))) }
    defm(RANGE_T, %w[to_a entries], 0) do |_, r|
      Builtins.cap(r.size) if r.begin.is_a?(Integer) && r.end.is_a?(Integer)
      r.to_a
    end
    defm(RANGE_T, "each", 0) { |it, r, _, b| r.each(&it.bp(b)) }
    defm(RANGE_T, "reverse_each", 0) { |it, r, _, b| r.reverse_each(&it.bp(b)) }
    defm(RANGE_T, "step", 1) do |it, r, a, b|
      r.step(Builtins.need_num(a[0], "Integer"), &it.bp(b))
      r
    end
    defm(RANGE_T, %w[include? member?], 1) { |_, r, a| r.include?(a[0]) }
    defm(RANGE_T, %w[cover? ===], 1) { |_, r, a| r.cover?(a[0]) }
    defm(RANGE_T, "exclude_end?", 0) { |_, r| r.exclude_end? }

    # ===================================================================================
    # Enumerator
    # ===================================================================================

    defm(ENUMR_T, "each", 0) { |it, r, _, b| b ? it.enum_each(r, b) : r }
    defm(ENUMR_T, "each_with_index", 0) { |it, r, _, b| Builtins.with_index(it, r, 0, b) }
    defm(ENUMR_T, "with_index", 0, 1) do |it, r, a, b|
      Builtins.with_index(it, r, a.empty? || a[0].nil? ? 0 : Builtins.need_intish(a[0]), b)
    end

    def self.with_index(it, r, start, b)
      i = start
      it.enum_each(r, NativeBlock.new { |*v|
        x = v.length == 1 ? v[0] : v
        idx = i
        i += 1
        b.call(x, idx)
      })
    end

    defm(ENUMR_T, %w[with_object each_with_object], 1) do |it, r, a, b|
      memo = a[0]
      it.enum_each(r, NativeBlock.new { |*v| b.call(v.length == 1 ? v[0] : v, memo) })
      memo
    end
    defm(ENUMR_T, "size", 0) { |it, r| it.enum_to_a(r).length }
    defm(ENUMR_T, %w[to_a entries], 0) { |it, r| it.enum_to_a(r) }

    # ===================================================================================
    # Enumerable (Array, Hash, Range, Enumerator)
    # ===================================================================================

    def self.pattern_or_block(it, a, b)
      return it.bp(b) if a.empty?

      pat = a[0]
      proc { |*v| case_eq(pat, v.length == 1 ? v[0] : v) }
    end

    defm(ENUM_T, "each_with_index", 0) { |it, r, _, b| it.coll(r).each_with_index(&it.bp(b)) }
    defm(ENUM_T, "each_with_object", 1) { |it, r, a, b| it.coll(r).each_with_object(a[0], &it.bp(b)) }
    defm(ENUM_T, "each_slice", 1) { |it, r, a, b| it.coll(r).each_slice(Builtins.need_intish(a[0]), &it.bp(b)) }
    defm(ENUM_T, "each_cons", 1) { |it, r, a, b| it.coll(r).each_cons(Builtins.need_intish(a[0]), &it.bp(b)) }
    defm(ENUM_T, %w[map collect], 0) { |it, r, _, b| it.coll(r).map(&it.bp(b)) }
    defm(ENUM_T, %w[flat_map collect_concat], 0) { |it, r, _, b| it.coll(r).flat_map(&it.bp(b)) }
    defm(ENUM_T, "filter_map", 0) { |it, r, _, b| it.coll(r).filter_map(&it.bp(b)) }
    defm(ENUM_T, %w[select filter], 0) { |it, r, _, b| it.coll(r).select(&it.bp(b)) }
    defm(ENUM_T, "find_all", 0) { |it, r, _, b| it.coll(r).find_all(&it.bp(b)) }
    defm(ENUM_T, "reject", 0) { |it, r, _, b| it.coll(r).reject(&it.bp(b)) }
    defm(ENUM_T, %w[find detect], 0) { |it, r, _, b| it.coll(r).find(&it.bp(b)) }
    defm(ENUM_T, "find_index", 0, 1) do |it, r, a, b|
      a.empty? ? it.coll(r).find_index(&it.bp(b)) : it.coll(r).find_index(a[0])
    end
    defm(ENUM_T, "take_while", 0) { |it, r, _, b| it.coll(r).take_while(&it.bp(b)) }
    defm(ENUM_T, "drop_while", 0) { |it, r, _, b| it.coll(r).drop_while(&it.bp(b)) }
    defm(ENUM_T, "any?", 0, 1) { |it, r, a, b| it.coll(r).any?(&Builtins.pattern_or_block(it, a, b)) }
    defm(ENUM_T, "all?", 0, 1) { |it, r, a, b| it.coll(r).all?(&Builtins.pattern_or_block(it, a, b)) }
    defm(ENUM_T, "none?", 0, 1) { |it, r, a, b| it.coll(r).none?(&Builtins.pattern_or_block(it, a, b)) }
    defm(ENUM_T, "one?", 0, 1) { |it, r, a, b| it.coll(r).one?(&Builtins.pattern_or_block(it, a, b)) }
    defm(ENUM_T, "count", 0, 1) { |it, r, a, b| a.empty? ? it.coll(r).count(&it.bp(b)) : it.coll(r).count(a[0]) }
    defm(ENUM_T, %w[include? member?], 1) { |it, r, a| it.coll(r).include?(a[0]) }
    defm(ENUM_T, "min", 0, 1) do |it, r, a, b|
      c = it.coll(r)
      a.empty? ? c.min(&it.bp(b)) : c.min(Builtins.need_intish(a[0]), &it.bp(b))
    end
    defm(ENUM_T, "max", 0, 1) do |it, r, a, b|
      c = it.coll(r)
      a.empty? ? c.max(&it.bp(b)) : c.max(Builtins.need_intish(a[0]), &it.bp(b))
    end
    defm(ENUM_T, "min_by", 0, 1) do |it, r, a, b|
      a.empty? ? it.coll(r).min_by(&it.bp(b)) : it.coll(r).min_by(Builtins.need_intish(a[0]), &it.bp(b))
    end
    defm(ENUM_T, "max_by", 0, 1) do |it, r, a, b|
      a.empty? ? it.coll(r).max_by(&it.bp(b)) : it.coll(r).max_by(Builtins.need_intish(a[0]), &it.bp(b))
    end
    defm(ENUM_T, "minmax", 0) { |it, r, _, b| it.coll(r).minmax(&it.bp(b)) }
    defm(ENUM_T, "sum", 0, 1) { |it, r, a, b| Builtins.ck(it.coll(r).sum(*a, &it.bp(b))) }
    defm(ENUM_T, %w[inject reduce], 0, 2) { |it, r, a, b| Builtins.inject(it, r, a, b) }
    defm(ENUM_T, "group_by", 0) { |it, r, _, b| it.coll(r).group_by(&it.bp(b)) }
    defm(ENUM_T, "partition", 0) { |it, r, _, b| it.coll(r).partition(&it.bp(b)) }
    defm(ENUM_T, "chunk_while", 0) do |it, r, _, b|
      it.coll(r).chunk_while(&it.bp(Builtins.need_blk(b, "chunk_while"))).to_a
    end
    defm(ENUM_T, "slice_when", 0) do |it, r, _, b|
      it.coll(r).slice_when(&it.bp(Builtins.need_blk(b, "slice_when"))).to_a
    end
    defm(ENUM_T, "tally", 0) { |it, r| it.coll(r).tally }
    defm(ENUM_T, "zip", 0, -1) do |it, r, a, b|
      others = a.map do |x|
        case x
        when Array then x
        when Hash, Range then x.to_a
        when Enum then it.enum_to_a(x)
        else raise Builtins.err("TypeError", "wrong argument type #{Builtins.class_name(x)} (must respond to :each)")
        end
      end
      it.coll(r).zip(*others, &it.bp(b))
    end
    defm(ENUM_T, "first", 0, 1) { |it, r, a| a.empty? ? it.coll(r).first : it.coll(r).first(Builtins.need_intish(a[0])) }
    defm(ENUM_T, "take", 1) { |it, r, a| it.coll(r).take(Builtins.need_intish(a[0])) }
    defm(ENUM_T, "drop", 1) { |it, r, a| it.coll(r).drop(Builtins.need_intish(a[0])) }
    defm(ENUM_T, "sort", 0) { |it, r, _, b| it.coll(r).sort(&it.bp(b)) }
    defm(ENUM_T, "sort_by", 0) { |it, r, _, b| it.coll(r).sort_by(&it.bp(b)) }
    defm(ENUM_T, "uniq", 0) { |it, r, _, b| it.coll(r).uniq(&it.bp(b)) }
    defm(ENUM_T, %w[to_a entries], 0) { |it, r| it.coll(r).to_a }
    defm(ENUM_T, "to_h", 0) { |it, r, _, b| it.coll(r).to_h(&it.bp(b)) }
    defm(ENUM_T, "reverse_each", 0) do |it, r, _, b|
      it.coll(r).reverse_each(&it.bp(b))
      r
    end

    UNSET = Object.new.freeze

    # inject/reduce. A Symbol operator is dispatched through the allowlist.
    def self.inject(it, r, a, b)
      acc = UNSET
      op = nil
      op_given = false
      case a.length
      when 0
        need_blk(b, "inject")
      when 1
        if b
          acc = a[0]
        else
          op = a[0]
          op_given = true
        end
      else
        acc = a[0]
        op = a[1]
        op_given = true
      end
      if op_given
        unless op.is_a?(Symbol) || op.is_a?(String)
          raise err("TypeError", "#{op.inspect} is not a symbol nor a string")
        end

        op = op.to_s
      end
      it.coll(r).each do |*v|
        x = v.length == 1 ? v[0] : v
        acc = if UNSET.equal?(acc) then x
              elsif op then it.call_method(acc, op, [x], nil)
              else b.call(acc, x)
              end
      end
      UNSET.equal?(acc) ? nil : acc
    end

    # ===================================================================================
    # Class values (Integer, Array, Math, File, ...)
    # ===================================================================================

    defm(CLASS_T, %w[name to_s inspect], 0) { |_, r| r.name.dup }
    defm(CLASS_T, "==", 1) { |_, r, a| r.equal?(a[0]) }
    defm(CLASS_T, "===", 1) { |_, r, a| Builtins.is_a(a[0], r) }

    array_class = {}
    defm(array_class, "new", 0, 2) do |_, _, a, b|
      if a.length == 1 && a[0].is_a?(Array)
        a[0].dup
      else
        n = Builtins.cap(a.empty? ? 0 : Builtins.need_intish(a[0]))
        if b
          raise Builtins.err("ArgumentError", "negative array size") if n.negative?

          Array.new(n) { |i| b.call(i) }
        else
          Array.new(n, a[1])
        end
      end
    end
    CLASS_METHODS["Array"] = array_class

    hash_class = {}
    defm(hash_class, "new", 0, 1) do |_, _, a, b|
      raise Builtins.err("ArgumentError", "Hash.new with a block is not supported") if b

      Hash.new(a[0])
    end
    CLASS_METHODS["Hash"] = hash_class

    math = {}
    %w[sqrt cbrt sin cos tan asin acos atan exp log2 log10].each do |nm|
      defm(math, nm, 1) do |_, _, a|
        x = Builtins.math_arg(a[0])
        case nm
        when "sqrt" then Math.sqrt(x)
        when "cbrt" then Math.cbrt(x)
        when "sin" then Math.sin(x)
        when "cos" then Math.cos(x)
        when "tan" then Math.tan(x)
        when "asin" then Math.asin(x)
        when "acos" then Math.acos(x)
        when "atan" then Math.atan(x)
        when "exp" then Math.exp(x)
        when "log2" then Math.log2(x)
        else Math.log10(x)
        end
      end
    end
    defm(math, "log", 1, 2) do |_, _, a|
      a.length == 1 ? Math.log(Builtins.math_arg(a[0])) : Math.log(Builtins.math_arg(a[0]), Builtins.math_arg(a[1]))
    end
    defm(math, "atan2", 2) { |_, _, a| Math.atan2(Builtins.math_arg(a[0]), Builtins.math_arg(a[1])) }
    defm(math, "hypot", 2) { |_, _, a| Math.hypot(Builtins.math_arg(a[0]), Builtins.math_arg(a[1])) }
    CLASS_METHODS["Math"] = math

    def self.math_arg(x)
      return x if x.is_a?(Integer) || x.is_a?(Float)

      raise err("TypeError", "can't convert #{tname(x)} into Float")
    end

    json = {}
    defm(json, "parse", 1, 2) { |_, _, a| JSONSupport.parse(a[0], a[1]) }
    defm(json, "generate", 1) { |_, _, a| JSONSupport.generate(a[0]) }
    defm(json, "pretty_generate", 1) { |_, _, a| JSONSupport.pretty_generate(a[0]) }
    CLASS_METHODS["JSON"] = json

    file = {}
    defm(file, "read", 1) { |it, _, a| Sandbox.read(it.state, a[0]) }
    defm(file, "exist?", 1) { |it, _, a| Sandbox.check(it.state, a[0], :exist) }
    defm(file, "file?", 1) { |it, _, a| Sandbox.check(it.state, a[0], :file) }
    defm(file, "directory?", 1) { |it, _, a| Sandbox.check(it.state, a[0], :directory) }
    defm(file, "basename", 1, 2) do |_, _, a|
      a.length == 1 ? File.basename(Builtins.need_str(a[0])) : File.basename(Builtins.need_str(a[0]), Builtins.need_str(a[1]))
    end
    defm(file, "dirname", 1) { |_, _, a| File.dirname(Builtins.need_str(a[0])) }
    defm(file, "extname", 1) { |_, _, a| File.extname(Builtins.need_str(a[0])) }
    defm(file, "join", 0, -1) { |_, _, a| File.join(*Builtins.join_parts(a)) }
    CLASS_METHODS["File"] = file

    def self.join_parts(a)
      a.map do |x|
        x.is_a?(Array) ? join_parts(x) : need_str(x)
      end
    end

    dir = {}
    defm(dir, "glob", 1) { |it, _, a| Sandbox.glob(it.state, a[0]) }
    CLASS_METHODS["Dir"] = dir

    # ===================================================================================
    # Functions
    # ===================================================================================

    defm(FUNCS, %w[format sprintf], 1, -1) { |it, _, a| Format.format(it, a[0], a[1..]) }
    defm(FUNCS, "Integer", 1, 2) do |_, _, a|
      x = a[0]
      if a.length == 2 && !a[1].nil?
        base = Builtins.need_intish(a[1])
        raise Builtins.err("ArgumentError", "base specified for non string value") unless x.is_a?(String)

        Builtins.ck(Kernel.Integer(x, base))
      else
        case x
        when Integer then x
        when Float, String then Builtins.ck(Kernel.Integer(x))
        else raise Builtins.err("TypeError", "can't convert #{Builtins.tname(x)} into Integer")
        end
      end
    end
    defm(FUNCS, "Float", 1) do |_, _, a|
      x = a[0]
      case x
      when Float then x
      when Integer then x.to_f
      when String then Kernel.Float(x)
      else raise Builtins.err("TypeError", "can't convert #{Builtins.tname(x)} into Float")
      end
    end
    defm(FUNCS, "String", 1) { |_, _, a| Builtins.to_s(a[0]) }
    defm(FUNCS, "Array", 1) do |it, _, a|
      x = a[0]
      case x
      when nil then []
      when Array then x
      when Hash, Range then x.to_a
      when Enum then it.enum_to_a(x)
      else [x]
      end
    end
    defm(FUNCS, "raise", 0, 1) do |_, _, a|
      raise Builtins.err("RuntimeError", "unhandled exception") if a.empty?
      raise Builtins.err("TypeError", "exception class/object expected") unless a[0].is_a?(String)

      raise Builtins.err("RuntimeError", a[0])
    end
    defm(FUNCS, "loop", 0) do |_, _, _, b|
      Builtins.need_blk(b, "loop")
      b.call while true # rubocop:disable Style/InfiniteLoop
    end
    defm(FUNCS, "render", 1, 2) { |it, _, a| it.render_partial(a[0], a.length > 1 ? a[1] : {}) }

    [COMMON_T, NIL_T, BOOL_T, INT_T, FLOAT_T, STR_T, SYM_T, ARR_T, HASH_T, RANGE_T, ENUMR_T, ENUM_T,
     CLASS_T, FUNCS, array_class, hash_class, math, json, file, dir].each(&:freeze)
    CLASS_METHODS.freeze
  end
end
