# frozen_string_literal: true

module ERBLite
  # Render-wide state shared by a template and all of its partials.
  class State
    attr_accessor :steps
    attr_reader :max_steps, :base_dir, :real_base, :cache

    def initialize(base_dir, max_steps)
      @steps = 0
      @max_steps = max_steps
      @base_dir = base_dir
      @real_base = nil
      if base_dir
        begin
          @real_base = File.realpath(base_dir.to_s)
        rescue SystemCallError => e
          raise Error.new("IOError", "base_dir is not accessible: #{e.message}")
        end
      end
      @cache = {}
    end
  end

  # Tree-walking interpreter. Every method call on a template value goes
  # through call_method, which only dispatches through Builtins' allowlist
  # tables.
  class Interpreter
    include Nodes

    NO_RECV = Object.new.freeze
    EMPTY = [].freeze
    MAX_RENDER_DEPTH = 32
    # Methods that ignore what their block returns.
    BLOCK_RESULT_UNUSED = %w[each each_with_index each_with_object each_slice each_cons each_char each_line
                             each_pair each_key each_value each_index reverse_each times upto downto step
                             loop tap].to_h { |m| [m, true] }.freeze

    attr_reader :state, :filename, :depth

    def initialize(state, filename, depth = 0)
      @state = state
      @filename = filename
      @depth = depth
      @loops = []
      @line = 1
      @out = +""
    end

    def run(ast, vars)
      @want = false
      env = Env.new(nil)
      vars.each { |k, v| env.vars[k] = v }
      ev(ast, env)
      @out
    end

    def step!
      st = @state
      st.steps += 1
      return unless st.max_steps && st.steps > st.max_steps

      raise Error.new("LimitError", "step limit of #{st.max_steps} exceeded", @filename, @line)
    end

    def truthy(v)
      !(v.nil? || false.equal?(v))
    end

    # A Ruby block that forwards to a template block, keeping the exact
    # number of yielded values.
    def bp(blk)
      return nil unless blk

      proc { |*a| blk.call(*a) }
    end

    # -- evaluation ------------------------------------------------------------

    attr_writer :want

    # @want is true when the value of the node being evaluated can be used.
    # Text and <%= %> statements only build their value (a snapshot of the
    # output buffer [ERBLite]) when it is wanted, so loops that end in text
    # do not copy the whole buffer on every iteration.
    def ev(n, env)
      step!
      w = @want
      @want = true
      case n
      when Call then ev_call(n, env)
      when LVar then env_at(env, n.depth).vars[n.name]
      when Lit then n.value
      when Str then ev_str(n.parts, env)
      when Seq
        r = nil
        last = n.stmts.length - 1
        n.stmts.each_with_index do |s, i|
          @want = i == last ? w : false
          r = ev(s, env)
        end
        r
      when Text
        @out << n.text
        w ? @out.dup : nil
      when Output
        v = ev(n.expr, env)
        @line = n.line
        @out << Builtins.to_s(v)
        w ? @out.dup : nil
      when LAsgn then env_at(env, n.depth).vars[n.name] = ev(n.value, env)
      when If
        c = truthy(ev(n.cond, env))
        @want = w
        if c
          n.thn ? ev(n.thn, env) : nil
        else
          n.els ? ev(n.els, env) : nil
        end
      when And
        l = ev(n.left, env)
        truthy(l) ? ev(n.right, env) : l
      when Or
        l = ev(n.left, env)
        truthy(l) ? l : ev(n.right, env)
      when Not then !truthy(ev(n.expr, env))
      when FCall then ev_fcall(n, env)
      when VCall then ev_vcall(n, env)
      when Arr then n.elems.map { |e| ev(e, env) }
      when HashLit
        h = {}
        n.pairs.each { |k, v| h[ev(k, env)] = ev(v, env) }
        h
      when RangeLit
        lo = ev(n.lo, env)
        hi = n.hi ? ev(n.hi, env) : nil
        @line = n.line
        Builtins.make_range(lo, hi, n.excl)
      when OpAsgn then ev_opasgn(n, env)
      when IndexAsgn
        r = ev(n.recv, env)
        args = n.args.map { |a| ev(a, env) }
        v = ev(n.value, env)
        @line = n.line
        invoke(r, "[]=", args + [v], nil, env, n.line)
        v
      when AttrAsgn
        r = ev(n.recv, env)
        v = ev(n.value, env)
        @line = n.line
        invoke(r, "#{n.name}=", [v], nil, env, n.line)
        v
      when MAsgn then ev_masgn(n, env)
      when While then ev_while(n, env)
      when For then ev_for(n, env)
      when Case then ev_case(n, env, w)
      when Break
        v = n.value ? ev(n.value, env) : nil
        throw(n.kind == :loop ? @loops.last[0] : env.blk, v)
      when Next
        v = n.value ? ev(n.value, env) : nil
        throw(n.kind == :loop ? @loops.last[1] : env.blk.next_tag, v)
      when Const
        @line = n.line
        Builtins.lookup_constant(n.name) ||
          raise(Error.new("NameError", "uninitialized constant #{n.name}", @filename, n.line))
      when Colon2
        scope = ev(n.scope, env)
        @line = n.line
        Builtins.scoped_constant(scope, n.name, @filename, n.line)
      when DSym then ev_str(n.parts, env).to_sym
      else
        raise Error.new("SyntaxError", "unsupported construct", @filename, @line)
      end
    rescue Error => e
      e.fill(@filename, n.line)
      raise
    end

    def env_at(env, depth)
      depth.times { env = env.parent }
      env
    end

    def ev_str(parts, env)
      return parts[0].dup if parts.length == 1 && parts[0].is_a?(String)

      s = +""
      parts.each { |p| s << (p.is_a?(String) ? p : Builtins.to_s(ev(p, env))) }
      s
    end

    def ev_call(n, env)
      return ev_shovel(n, env)[0] if n.name == "<<" && n.args.length == 1 && n.block.nil?

      recv = ev(n.recv, env)
      return nil if n.safe && recv.nil?

      args = n.args.empty? ? EMPTY : n.args.map { |a| ev(a, env) }
      @line = n.line
      invoke(recv, n.name, args, n.block, env, n.line)
    end

    def ev_fcall(n, env)
      args = n.args.empty? ? EMPTY : n.args.map { |a| ev(a, env) }
      @line = n.line
      invoke(NO_RECV, n.name, args, n.block, env, n.line)
    end

    def ev_vcall(n, env)
      @line = n.line
      return invoke(NO_RECV, n.name, EMPTY, nil, env, n.line) if Builtins::FUNCS[n.name]

      raise Error.new("NameError", "undefined local variable or method '#{n.name}' for main", @filename, n.line)
    end

    # Calls +name+ on +recv+ (or the function +name+ when recv is NO_RECV),
    # setting up the template block if there is one. `break` inside the
    # block is a throw to the UserBlock, caught here.
    def invoke(recv, name, args, bnode, env, line)
      if bnode.is_a?(BlockNode)
        blk = UserBlock.new(self, bnode, env, !(BLOCK_RESULT_UNUSED[name] && !recv.is_a?(Enum)))
        catch(blk) { dispatch(recv, name, args, blk) }
      elsif bnode
        dispatch(recv, name, args, SymBlock.new(self, bnode.name))
      else
        dispatch(recv, name, args, nil)
      end
    rescue Error => e
      e.fill(@filename, line)
      raise
    end

    def dispatch(recv, name, args, blk)
      recv.equal?(NO_RECV) ? call_function(name, args, blk) : call_method(recv, name, args, blk)
    end

    def call_function(name, args, blk)
      f = Builtins::FUNCS[name]
      raise Error.new("NoMethodError", "undefined method '#{name}' for main:Object") unless f

      Builtins.check_arity(f, args.length)
      f.fn.call(self, nil, args, blk)
    rescue Error
      raise
    rescue ::StandardError => e
      raise Builtins.map_error(e)
    end

    def call_method(recv, name, args, blk)
      m = Builtins.lookup(recv, name)
      unless m
        raise Error.new("NoMethodError", "undefined method '#{name}' for #{Builtins.describe(recv)}")
      end

      Builtins.check_arity(m, args.length)
      return Enum.new(recv, name, args.dup) if blk.nil? && Builtins.enumerator_call?(recv, name, args)

      m.fn.call(self, recv, args, blk)
    rescue Error
      raise
    rescue ::StandardError => e
      raise Builtins.map_error(e)
    end

    def responds?(recv, name)
      !Builtins.lookup(recv, name).nil?
    end

    # -- assignment helpers ------------------------------------------------------

    # Evaluates an expression that may be written back to (for `<<` on
    # strings and op-assignment). Returns [value, setter or nil].
    def lvalue(node, env)
      case node
      when LVar
        target = env_at(env, node.depth)
        [target.vars[node.name], ->(x) { target.vars[node.name] = x }]
      when Call
        if node.name == "[]" && node.recv && !node.safe && node.block.nil?
          r = ev(node.recv, env)
          a = node.args.map { |x| ev(x, env) }
          @line = node.line
          v = invoke(r, "[]", a, nil, env, node.line)
          [v, lambda { |x|
            invoke(r, "[]=", a + [x], nil, env, node.line)
            x
          }]
        elsif node.name == "<<" && node.args.length == 1 && node.block.nil?
          ev_shovel(node, env)
        else
          [ev(node, env), nil]
        end
      else
        [ev(node, env), nil]
      end
    end

    # `a << b`. Strings are values [ERBLite]: when +a+ is a String, the new
    # string is assigned back to whatever +a+ was read from.
    def ev_shovel(n, env)
      step!
      v, setter = lvalue(n.recv, env)
      arg = ev(n.args[0], env)
      @line = n.line
      res = invoke(v, "<<", [arg], nil, env, n.line)
      if v.is_a?(String)
        setter&.call(res)
        [res, setter]
      else
        [res, nil]
      end
    end

    def ev_opasgn(n, env)
      t = n.target
      if t.is_a?(Call) && t.name != "[]"
        r = ev(t.recv, env)
        @line = n.line
        v = invoke(r, t.name, EMPTY, nil, env, n.line)
        setter = lambda { |x|
          invoke(r, "#{t.name}=", [x], nil, env, n.line)
          x
        }
      else
        v, setter = lvalue(t, env)
      end
      case n.op
      when "||" then truthy(v) ? v : setter.call(ev(n.value, env))
      when "&&" then truthy(v) ? setter.call(ev(n.value, env)) : v
      else
        rhs = ev(n.value, env)
        @line = n.line
        setter.call(invoke(v, n.op, [rhs], nil, env, n.line))
      end
    end

    def ev_masgn(n, env)
      if n.values.length == 1
        result = ev(n.values[0], env)
        arr = result.is_a?(Array) ? result : [result]
      else
        result = arr = n.values.map { |v| ev(v, env) }
      end
      n.targets.each_with_index { |(name, depth), i| env_at(env, depth).vars[name] = arr[i] }
      result
    end

    # -- control flow ------------------------------------------------------------

    def ev_while(n, env)
      brk = Object.new
      nxt = Object.new
      @loops.push([brk, nxt])
      begin
        catch(brk) do
          while true # rubocop:disable Style/InfiniteLoop
            c = truthy(ev(n.cond, env))
            break if n.neg ? c : !c

            step!
            @want = false
            catch(nxt) { ev(n.body, env) }
          end
          nil
        end
      ensure
        @loops.pop
      end
    end

    def ev_for(n, env)
      iter = ev(n.iter, env)
      brk = Object.new
      nxt = Object.new
      @loops.push([brk, nxt])
      begin
        catch(brk) do
          blk = NativeBlock.new do |*vals|
            step!
            v = vals.length == 1 ? vals[0] : vals
            if n.vars.length == 1
              name, depth = n.vars[0]
              env_at(env, depth).vars[name] = v
            else
              arr = v.is_a?(Array) ? v : [v]
              n.vars.each_with_index { |(name, depth), i| env_at(env, depth).vars[name] = arr[i] }
            end
            @want = false
            catch(nxt) { ev(n.body, env) }
            nil
          end
          @line = n.line
          begin
            call_method(iter, "each", EMPTY, blk)
          rescue Error => e
            e.fill(@filename, n.line)
            raise
          end
          iter
        end
      ensure
        @loops.pop
      end
    end

    def ev_case(n, env, want)
      v = n.subject ? ev(n.subject, env) : nil
      n.whens.each do |conds, body|
        conds.each do |c|
          hit = n.subject ? case_eq(ev(c, env), v) : truthy(ev(c, env))
          next unless hit

          @want = want
          return ev(body, env)
        end
      end
      @want = want
      n.els ? ev(n.els, env) : nil
    end

    def case_eq(pat, v)
      Builtins.case_eq(pat, v)
    end

    # -- enumerators -------------------------------------------------------------

    def enum_each(e, blk)
      call_method(e.recv, e.meth, e.args, blk)
    end

    def enum_to_a(e)
      res = []
      enum_each(e, NativeBlock.new { |*v|
        res << (v.length == 1 ? v[0] : v)
        nil
      })
      res
    end

    # The collection used by the shared Enumerable methods.
    def coll(r)
      case r
      when Enum then enum_to_a(r)
      when Range
        # Native Enumerable methods walk the whole range without counting
        # steps, so bound the work like any other generated collection.
        Builtins.cap(r.size) if r.begin.is_a?(Integer) && r.end.is_a?(Integer)
        r
      else r
      end
    end

    # -- partials ------------------------------------------------------------------

    def render_partial(path, locals)
      if @depth + 1 > MAX_RENDER_DEPTH
        raise Error.new("LimitError", "render nesting is deeper than #{MAX_RENDER_DEPTH}")
      end
      raise Error.new("TypeError", "render locals must be a Hash") unless locals.is_a?(Hash)

      vars = {}
      locals.each do |k, v|
        unless k.is_a?(Symbol) || k.is_a?(String)
          raise Error.new("TypeError", "render local names must be Symbols or Strings")
        end
        vars[k.to_s] = v
      end
      source = Sandbox.read(@state, path)
      key = [Sandbox.resolve(@state, path), vars.keys.sort]
      ast = @state.cache[key] ||= begin
        src = ERBLite.normalize_source(source, path)
        Parser.parse(Lexer.tokenize(src, path), path, vars.keys)
      end
      Interpreter.new(@state, path, @depth + 1).run(ast, vars)
    end
  end
end
