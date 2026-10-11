# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

# frozen_string_literal: true

require_relative "erblite/version"
require_relative "erblite/error"
require_relative "erblite/nodes"
require_relative "erblite/lexer"
require_relative "erblite/parser"
require_relative "erblite/values"
require_relative "erblite/builtins"
require_relative "erblite/format"
require_relative "erblite/json"
require_relative "erblite/sandbox"
require_relative "erblite/interpreter"

# ERBLite: a sandboxed template engine with ERB syntax and a small subset of
# Ruby. Templates are lexed, parsed into an AST and run by a tree-walking
# interpreter; no template text is ever evaluated as Ruby code.
module ERBLite
  NAME_RE = /\A[a-z_][A-Za-z0-9_]*\z/

  # A compiled template. The source is tokenized once; the token stream is
  # parsed per set of host variable names (host variables are locals of the
  # template, which affects parsing as in Ruby).
  class Template
    attr_reader :filename

    def initialize(source, filename: "(erblite)")
      @filename = filename
      @source = ERBLite.normalize_source(source, filename)
      @tokens = Lexer.tokenize(@source, filename)
      @asts = {}
    rescue SystemStackError
      raise Error.new("LimitError", "nesting is too deep (stack exhausted)", filename, nil)
    end

    # Variables may be passed as a Hash or as extra keyword arguments
    # (render(name: "x")); base_dir and max_steps are always options.
    def render(vars = {}, base_dir: nil, max_steps: nil, **more)
      vars = vars.merge(more) unless more.empty?
      locals = ERBLite.convert_vars(vars)
      names = locals.keys
      ast = @asts[names.sort] ||= Parser.parse(@tokens, @filename, names)
      state = State.new(base_dir, max_steps)
      Interpreter.new(state, @filename).run(ast, locals)
    rescue SystemStackError
      raise Error.new("LimitError", "nesting is too deep (stack exhausted)", @filename, nil)
    rescue NoMemoryError
      raise Error.new("LimitError", "out of memory", @filename, nil)
    end
  end

  module_function

  def render(source, vars = {}, base_dir: nil, filename: "(erblite)", max_steps: nil, **more)
    Template.new(source, filename: filename).render(vars, base_dir: base_dir, max_steps: max_steps, **more)
  end

  def render_file(path, vars = {}, base_dir: nil, filename: nil, max_steps: nil, **more)
    vars = vars.merge(more) unless more.empty?
    source = begin
      File.binread(path)
    rescue SystemCallError => e
      raise Error.new("IOError", e.message, path.to_s, nil)
    end
    base_dir ||= File.dirname(File.expand_path(path))
    Template.new(source, filename: filename || path.to_s).render(vars, base_dir: base_dir, max_steps: max_steps)
  end

  def normalize_source(source, filename)
    raise Error.new("TypeError", "template source must be a String", filename, nil) unless source.is_a?(String)

    s = source.dup.force_encoding(Encoding::UTF_8)
    raise Error.new("SyntaxError", "template is not valid UTF-8", filename, 1) unless s.valid_encoding?

    s
  end

  # Host variables: Symbol or String keys naming valid identifiers. Values
  # are copied into fresh template values (Arrays/Hashes are copied, sharing
  # is preserved), so templates never see host objects.
  def convert_vars(vars)
    raise Error.new("TypeError", "variables must be a Hash") unless vars.is_a?(Hash)

    memo = {}.compare_by_identity
    out = {}
    vars.each do |k, v|
      unless k.is_a?(Symbol) || k.is_a?(String)
        raise Error.new("TypeError", "variable names must be Symbols or Strings, got #{k.inspect}")
      end

      name = k.to_s
      next unless name.match?(NAME_RE) && !CodeLexer::KEYWORD_SET[name]

      out[name] = convert_value(v, memo)
    end
    out
  end

  def convert_value(v, memo)
    case v
    when nil, true, false, Float, Symbol then v
    when Integer then Builtins.ck(v)
    when String
      s = v.dup
      s.force_encoding(Encoding::UTF_8) unless s.encoding == Encoding::UTF_8
      s
    when Array
      return memo[v] if memo.key?(v)

      arr = memo[v] = []
      v.each { |x| arr << convert_value(x, memo) }
      arr
    when Hash
      return memo[v] if memo.key?(v)

      h = memo[v] = {}
      h.default = convert_value(v.default, memo) unless v.default_proc
      v.each { |k, x| h[convert_value(k, memo)] = convert_value(x, memo) }
      h
    when Range
      Range.new(convert_value(v.begin, memo), convert_value(v.end, memo), v.exclude_end?)
    else
      raise Error.new("TypeError", "unsupported variable value of class #{v.class}")
    end
  end
end
