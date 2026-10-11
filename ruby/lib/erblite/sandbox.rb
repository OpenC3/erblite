# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

# frozen_string_literal: true

module ERBLite
  # The only file system access templates have (SPEC section 9.1). Every path
  # is resolved against the real path of base_dir, symlinks included, and
  # must stay inside it.
  module Sandbox
    module_function

    def require_base(state)
      return state.real_base if state.real_base

      raise Error.new("SecurityError", "file access is disabled (no base_dir was given)")
    end

    def check_path_string(path)
      unless path.is_a?(String)
        raise Error.new("TypeError", "no implicit conversion of #{Builtins.tname(path)} into String")
      end
      raise Error.new("ArgumentError", "string contains null byte") if path.include?("\0")
    end

    def inside?(real, base)
      return true if real == base

      prefix = base.end_with?("/") ? base : "#{base}/"
      real.start_with?(prefix)
    end

    # Lexically normalized absolute path for +path+ under +base+. A leading
    # "~" is never expanded to a home directory.
    def expand(path, base)
      p = path.start_with?("~") ? "./#{path}" : path
      File.expand_path(p, base)
    end

    # Resolves +path+ to a real path inside the sandbox or raises
    # SecurityError. Missing trailing components are allowed (the caller
    # reports IOError for them); the deepest existing ancestor is resolved
    # with File.realpath so symlinks cannot escape.
    def resolve(state, path)
      base = require_base(state)
      check_path_string(path)
      full = expand(path, base)
      rest = []
      cur = full
      until File.exist?(cur)
        parent = File.dirname(cur)
        break if parent == cur

        rest.unshift(File.basename(cur))
        cur = parent
      end
      real = begin
        File.realpath(cur)
      rescue SystemCallError
        cur
      end
      real = File.join(real, *rest) unless rest.empty?
      raise Error.new("SecurityError", "path is outside the sandbox: #{path}") unless inside?(real, base)

      real
    end

    def read(state, path)
      real = resolve(state, path)
      raise Error.new("IOError", "No such file or directory - #{path}") unless File.exist?(real)
      raise Error.new("IOError", "Is a directory - #{path}") if File.directory?(real)

      File.binread(real).force_encoding(Encoding::UTF_8)
    rescue SystemCallError => e
      raise Error.new("IOError", "#{e.message} - #{path}")
    end

    def check(state, path, kind)
      require_base(state)
      check_path_string(path)
      real = begin
        resolve(state, path)
      rescue Error => e
        return false if e.type == "SecurityError"

        raise
      end
      case kind
      when :exist then File.exist?(real)
      when :file then File.file?(real)
      else File.directory?(real)
      end
    end

    # Dir.glob with *, ** and ? only. Results are relative to base_dir and
    # sorted.
    def glob(state, pattern)
      base = require_base(state)
      check_path_string(pattern)
      full = expand(pattern, base)
      raise Error.new("SecurityError", "glob pattern is outside the sandbox: #{pattern}") unless inside?(full, base)

      rel = full == base ? "" : full[(base.end_with?("/") ? base.length : base.length + 1)..]
      segs = rel.split("/").reject(&:empty?)
      out = []
      walk(base, base, "", segs, out) unless segs.empty?
      out.uniq.sort
    end

    def children(dir)
      Dir.children(dir).sort
    rescue SystemCallError
      []
    end

    def safe_dir?(path, base)
      File.directory?(path) && inside?(File.realpath(path), base)
    rescue SystemCallError
      false
    end

    def walk(base, dir, prefix, segs, out)
      seg = segs[0]
      last = segs.length == 1
      join = ->(name) { prefix.empty? ? name : "#{prefix}/#{name}" }
      if seg == "**" && !last
        walk(base, dir, prefix, segs[1..], out)
        children(dir).each do |name|
          next if name.start_with?(".")

          path = File.join(dir, name)
          next if File.symlink?(path) || !safe_dir?(path, base)

          walk(base, path, join.call(name), segs, out)
        end
      elsif seg.include?("*") || seg.include?("?")
        children(dir).each do |name|
          next if name.start_with?(".") && !seg.start_with?(".")
          next unless wildcard_match?(seg, name)

          path = File.join(dir, name)
          if last
            out << join.call(name)
          elsif safe_dir?(path, base)
            walk(base, path, join.call(name), segs[1..], out)
          end
        end
      else
        path = File.join(dir, seg)
        return unless File.exist?(path) || File.symlink?(path)

        if last
          out << join.call(seg)
        elsif safe_dir?(path, base)
          walk(base, path, join.call(seg), segs[1..], out)
        end
      end
    end

    # Matches +name+ against a glob segment where * is any run of characters
    # and ? is one character.
    def wildcard_match?(pat, name)
      p = 0
      n = 0
      star_p = nil
      star_n = 0
      while n < name.length
        if p < pat.length && (pat[p] == "?" || pat[p] == name[n])
          p += 1
          n += 1
        elsif p < pat.length && pat[p] == "*"
          star_p = p
          star_n = n
          p += 1
        elsif star_p
          p = star_p + 1
          star_n += 1
          n = star_n
        else
          return false
        end
      end
      p += 1 while p < pat.length && pat[p] == "*"
      p == pat.length
    end
  end
end
