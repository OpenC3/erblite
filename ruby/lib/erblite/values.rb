# frozen_string_literal: true

module ERBLite
  # A built-in class or module constant (Integer, Comparable, Math, File, ...).
  # Templates can only reach the instances created in Builtins::CONSTANTS.
  class ClassValue
    attr_reader :name

    def initialize(name, is_module)
      @name = name.freeze
      @module = is_module
    end

    def module?
      @module
    end

    def to_s
      @name.dup
    end

    def inspect
      @name.dup
    end
  end

  # What an iterator method returns when it is called without a block.
  # Iterating it calls method +meth+ on +recv+ with +args+ and a block.
  class Enum
    attr_reader :recv, :meth, :args

    def initialize(recv, meth, args)
      @recv = recv
      @meth = meth
      @args = args
    end

    def inspect
      a = @args.empty? ? "" : "(#{@args.map(&:inspect).join(", ")})"
      if @meth == "step" && arithmetic_sequence?
        # Ruby returns an ArithmeticSequence here, which prints differently.
        return @recv.is_a?(Range) ? "((#{@recv.inspect}).step#{a})" : "(#{@recv.inspect}.step#{a})"
      end
      "#<Enumerator: #{@recv.inspect}:#{@meth}#{a}>"
    end

    def arithmetic_sequence?
      num = ->(x) { x.is_a?(Integer) || x.is_a?(Float) }
      return @args.all?(&num) if @recv.is_a?(Range)

      (@recv.is_a?(Integer) || @recv.is_a?(Float)) && @args.all? { |x| num.call(x) || x.nil? }
    end

    def to_s
      inspect
    end
  end

  # Local variable scope. Each block call gets its own Env whose +blk+ is the
  # UserBlock being run, so `break`/`next` know where to go.
  class Env
    attr_reader :vars, :parent, :blk

    def initialize(parent, blk = nil)
      @vars = {}
      @parent = parent
      @blk = blk
    end
  end

  # A block written in the template ({ |x| ... } or do |x| ... end).
  class UserBlock
    attr_reader :next_tag

    def initialize(interp, node, env, value_used = true)
      @it = interp
      @node = node
      @env = env
      @value_used = value_used
      @next_tag = Object.new
    end

    def call(*vals)
      @it.step!
      e = Env.new(@env, self)
      params = @node.params
      vals = vals[0] if params.length > 1 && vals.length == 1 && vals[0].is_a?(Array)
      params.each_with_index { |p, i| bind(p, vals[i], e) }
      @it.want = @value_used
      catch(@next_tag) { @it.ev(@node.body, e) }
    end

    private

    def bind(param, value, env)
      if param.is_a?(Array)
        arr = value.is_a?(Array) ? value : [value]
        param.each_with_index { |q, j| bind(q, arr[j], env) }
      else
        env.vars[param] = value
      end
    end
  end

  # &:name - calls method +name+ (through the allowlist) on the first value.
  class SymBlock
    def initialize(interp, name)
      @it = interp
      @name = name
    end

    def call(*vals)
      @it.step!
      raise Error.new("ArgumentError", "no receiver given") if vals.empty?

      @it.call_method(vals[0], @name, vals[1..], nil)
    end
  end

  # A block implemented by the interpreter itself (for loops, enumerators).
  class NativeBlock
    def initialize(&body)
      @body = body
    end

    def call(*vals)
      @body.call(*vals)
    end
  end
end
