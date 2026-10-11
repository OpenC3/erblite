# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

# frozen_string_literal: true

require "tmpdir"
require "fileutils"

RSpec.describe "ERBLite sandbox" do
  def expect_error(src, type, vars = {}, **opts)
    err = render_error(src, vars, **opts)
    expect(err.type).to eq(type), "#{src}: expected #{type}, got #{err.type}: #{err.message}"
  end

  describe "no dynamic dispatch or code execution" do
    {
      "<%= 1.send(:+, 2) %>" => "NoMethodError",
      "<%= 1.public_send(:+, 2) %>" => "NoMethodError",
      "<%= 1.__send__(:+, 2) %>" => "NoMethodError",
      "<%= eval('1') %>" => "NoMethodError",
      "<%= eval '1' %>" => "NoMethodError",
      "<%= 'x'.instance_eval('upcase') %>" => "NoMethodError",
      "<%= 'x'.instance_eval { upcase } %>" => "NoMethodError",
      "<%= 'x'.instance_exec { 1 } %>" => "NoMethodError",
      "<%= Integer.class_eval('1') %>" => "NoMethodError",
      "<%= Integer.module_eval('1') %>" => "NoMethodError",
      "<%= binding %>" => "NameError",
      "<%= binding.local_variables %>" => "NameError",
      "<%= 1.method(:+).call(2) %>" => "NoMethodError",
      "<%= method(:puts) %>" => "NoMethodError",
      "<%= 1.methods %>" => "NoMethodError",
      "<%= Object.const_get(:File) %>" => "NoMethodError",
      "<%= Integer.const_get(:MAX) %>" => "NoMethodError",
      "<%= Integer.define_method(:x) { 1 } %>" => "NoMethodError",
      "<%= 'x'.define_singleton_method(:y) { 1 } %>" => "NoMethodError",
      "<%= 'x'.instance_variable_get(:@a) %>" => "NoMethodError",
      "<%= 'x'.instance_variable_set(:@a, 1) %>" => "NoMethodError",
      "<%= 'x'.instance_variables %>" => "NoMethodError",
      "<%= 'x'.singleton_class %>" => "NoMethodError",
      "<%= 'x'.extend(Comparable) %>" => "NoMethodError",
      "<%= 1.class.ancestors %>" => "NoMethodError",
      "<%= 1.class.class.new %>" => "NoMethodError",
      "<%= Object.new %>" => "NoMethodError",
      "<%= Integer.new %>" => "NoMethodError",
      "<%= [1].map(&:send) %>" => "NoMethodError",
      "<%= [[1, :upcase]].map { |a| a.first.send(a.last) } %>" => "NoMethodError",
      "<%= [1, 2].inject(:send) %>" => "NoMethodError",
      "<%= ['instance_eval'].map(&:to_sym).map { |m| 1.respond_to?(m) } %>" => nil,
      "<%= 'x'.to_sym.to_proc %>" => "NoMethodError",
      "<%= proc { 1 } %>" => "NoMethodError",
      "<%= lambda { 1 } %>" => "NoMethodError",
      "<% f = ->(x) { x } %>" => "SyntaxError",
      "<%= system('echo hi') %>" => "NoMethodError",
      "<%= exec('ls') %>" => "NoMethodError",
      "<%= spawn('ls') %>" => "NoMethodError",
      "<%= `ls` %>" => "SyntaxError",
      "<%= %x(ls) %>" => "SyntaxError",
      "<%= fork %>" => "NameError",
      "<%= exit %>" => "NameError",
      "<%= exit! %>" => "NameError",
      "<%= abort %>" => "NameError",
      "<%= sleep 1 %>" => "NoMethodError",
      "<%= puts 1 %>" => "NoMethodError",
      "<%= require 'socket' %>" => "NoMethodError",
      "<%= require_relative 'x' %>" => "NoMethodError",
      "<%= load 'x.rb' %>" => "NoMethodError",
      "<%= open('|ls') %>" => "NoMethodError",
      "<%= Kernel.system('ls') %>" => "NameError",
      "<%= Kernel %>" => "NameError",
      "<%= ObjectSpace.each_object %>" => "NameError",
      "<%= Process.pid %>" => "NameError",
      "<%= IO.read('x') %>" => "NameError",
      "<%= ENV['HOME'] %>" => "NameError",
      "<%= ARGV %>" => "NameError",
      "<%= STDOUT %>" => "NameError",
      "<%= RUBY_VERSION %>" => "NameError",
      "<%= Marshal.load('') %>" => "NameError",
      "<%= Class.new %>" => "NameError",
      "<%= Module %>" => "NameError",
      "<%= Struct.new(:a) %>" => "NameError",
      "<%= BasicObject %>" => "NameError",
      "<%= Math::DomainError %>" => "NameError",
      "<%= ERBLite %>" => "NameError",
      "<%= ERBLite::Builtins %>" => "NameError",
      "<%= Integer::Builtins %>" => "NameError",
      "<%= $stdout %>" => "SyntaxError",
      "<%= $0 %>" => "SyntaxError",
      "<%= @x %>" => "SyntaxError",
      "<%= @@x %>" => "SyntaxError",
      '<%= "#@x" %>' => "SyntaxError",
      '<%= "#$0" %>' => "SyntaxError",
      "<%= __FILE__ %>" => "SyntaxError",
      "<%= __method__ %>" => "NameError",
      "<%= self %>" => "SyntaxError",
      "<%= self.class %>" => "SyntaxError",
      "<% def foo; end %>" => "SyntaxError",
      "<% class Foo; end %>" => "SyntaxError",
      "<% module Foo; end %>" => "SyntaxError",
      "<% alias foo bar %>" => "SyntaxError",
      "<% undef foo %>" => "SyntaxError",
      "<% begin; rescue; end %>" => "SyntaxError",
      "<% yield %>" => "SyntaxError",
      "<% return %>" => "SyntaxError",
      "<% super %>" => "SyntaxError",
      "<%= defined?(x) %>" => "SyntaxError",
      "<% BEGIN { } %>" => "SyntaxError",
      "<%= 'a' =~ /a/ %>" => "SyntaxError",
      "<%= [*[1]] %>" => "SyntaxError",
      "<%= x = <<~EOS\nhi\nEOS\n %>" => "SyntaxError"
    }.each do |src, type|
      it "blocks #{src.inspect}" do
        if type
          expect_error(src, type)
        else
          expect(render(src)).to eq("[false]")
        end
      end
    end

    it "respond_to? only reports allowlisted methods" do
      %w[send public_send __send__ instance_eval eval system method define_method instance_variable_get
         const_get binding class_eval object_id singleton_class extend].each do |m|
        expect(render("<%= [1.respond_to?(:#{m}), 'x'.respond_to?('#{m}'), nil.respond_to?(:#{m}), Integer.respond_to?(:#{m})] %>"))
          .to eq("[false, false, false, false]")
      end
      expect(render("<%= 'x'.respond_to?(:upcase) %>,<%= [].respond_to?(:map) %>")).to eq("true,true")
    end

    it "dispatches symbol blocks and inject operators through the allowlist" do
      expect(render("<%= %w[a b].map(&:upcase).inspect %>,<%= [1, 2].inject(:+) %>")).to eq('["A", "B"],3')
      expect_error("<%= ['x'].map(&:instance_variables) %>", "NoMethodError")
      expect_error("<%= [1, 2].reduce(:instance_eval) %>", "NoMethodError")
    end

    it "builds method names only from the allowlist when assigning attributes" do
      expect_error("<% x = 'a' %><% x.instance_eval = 1 %>", "NoMethodError")
      expect_error("<% h = {} %><% h.send = 1 %>", "NoMethodError")
    end

    it "treats format strings as data" do
      expect(render("<%= format('%s', '%x') %>")).to eq("%x")
      expect_error("<%= format('%<a>s', a: 1) + `id` %>", "SyntaxError")
      expect(render("<%= format('%1$s', '%2$s') %>")).to eq("%2$s")
    end

    it "never touches the host program's globals" do
      expect_error("<%= $PROGRAM_NAME %>", "SyntaxError")
      before = $stdout
      render("<% x = 1 %>")
      expect($stdout).to equal(before)
    end

    it "does not use eval-like Ruby APIs in its own source" do
      forbidden = [
        /(?<![\w.])eval\s*\(/, /(?<![\w.])eval\s+['"]/, /\.instance_eval\b/, /\.instance_exec\b/, /\.class_eval\b/, /\.module_eval\b/,
        /\.send\s*[(\s]/, /\.public_send\b/, /\.__send__\b/, /\bconst_get\b/, /(?<![\w.])method\s*\(/,
        /\.method\s*\(/, /\bbinding\s*[.)]/, /\bRipper\b/, /\bRubyVM\b/, /\.define_method\b/, /define_method\s*\(/, /\bKernel\.(?!format|Integer|Float)\w/
      ]
      Dir[File.expand_path("../lib/**/*.rb", __dir__)].each do |f|
        File.readlines(f).each_with_index do |line, i|
          next if line.strip.start_with?("#")

          forbidden.each do |re|
            expect(line).not_to match(re), "#{File.basename(f)}:#{i + 1} uses #{re.source}: #{line.strip}"
          end
        end
      end
    end
  end

  describe "file sandbox" do
    it "refuses all file access without base_dir" do
      expect_error("<%= File.read('x') %>", "SecurityError")
      expect_error("<%= File.exist?('x') %>", "SecurityError")
      expect_error("<%= Dir.glob('*') %>", "SecurityError")
      expect_error("<%= render('x') %>", "SecurityError")
    end

    it "keeps pure path helpers available without base_dir" do
      expect(render("<%= File.join('a', 'b') %>,<%= File.basename('/x/y.rb', '.rb') %>")).to eq("a/b,y")
    end

    it "blocks traversal, absolute paths and home expansion" do
      %w[../outside.txt data/../../outside.txt /etc/passwd ../../../../etc/passwd].each do |p|
        expect_error("<%= File.read('#{p}') %>", "SecurityError", {}, base_dir: FIXTURES)
        expect_error("<%= render('#{p}') %>", "SecurityError", {}, base_dir: FIXTURES)
      end
      expect_error("<%= File.read('~/.bashrc') %>", "IOError", {}, base_dir: FIXTURES)
      expect_error("<%= Dir.glob('/etc/*') %>", "SecurityError", {}, base_dir: FIXTURES)
      expect_error("<%= Dir.glob('../**/*') %>", "SecurityError", {}, base_dir: FIXTURES)
      expect(render("<%= File.exist?('/etc/passwd') %>", {}, base_dir: FIXTURES)).to eq("false")
    end

    it "allows absolute paths that stay inside base_dir" do
      path = File.join(FIXTURES, "data", "lines.txt")
      expect(render("<%= File.read('#{path}').lines.size %>", {}, base_dir: FIXTURES)).to eq("2")
    end

    it "rejects null bytes and non-string paths" do
      expect_error("<%= File.read(\"data/lines.txt\\0x\") %>", "ArgumentError", {}, base_dir: FIXTURES)
      expect_error("<%= File.read(1) %>", "TypeError", {}, base_dir: FIXTURES)
    end

    it "does not follow symlinks out of the sandbox" do
      Dir.mktmpdir do |tmp|
        base = File.join(tmp, "base")
        outside = File.join(tmp, "outside")
        FileUtils.mkdir_p([base, outside])
        File.write(File.join(outside, "secret.txt"), "secret")
        File.write(File.join(base, "ok.txt"), "ok")
        File.symlink(File.join(outside, "secret.txt"), File.join(base, "link.txt"))
        File.symlink(outside, File.join(base, "dirlink"))
        File.symlink(File.join(base, "ok.txt"), File.join(base, "inner_link.txt"))
        File.symlink(File.join(outside, "missing.txt"), File.join(base, "dangling.txt"))

        expect(render("<%= File.read('ok.txt') %>", {}, base_dir: base)).to eq("ok")
        expect(render("<%= File.read('inner_link.txt') %>", {}, base_dir: base)).to eq("ok")
        expect_error("<%= File.read('link.txt') %>", "SecurityError", {}, base_dir: base)
        expect_error("<%= File.read('dirlink/secret.txt') %>", "SecurityError", {}, base_dir: base)
        expect_error("<%= File.read('dirlink/nope.txt') %>", "SecurityError", {}, base_dir: base)
        expect_error("<%= render('link.txt') %>", "SecurityError", {}, base_dir: base)
        expect_error("<%= File.read('dangling.txt') %>", "IOError", {}, base_dir: base)
        expect(render("<%= File.exist?('link.txt') %>,<%= File.file?('dirlink/secret.txt') %>", {}, base_dir: base))
          .to eq("false,false")
        expect(render("<%= Dir.glob('**/*.txt').inspect %>", {}, base_dir: base))
          .to eq('["dangling.txt", "inner_link.txt", "link.txt", "ok.txt"]')
        expect(render("<%= Dir.glob('dirlink/*').inspect %>", {}, base_dir: base)).to eq("[]")
      end
    end

    it "treats a symlinked base_dir by its real path" do
      Dir.mktmpdir do |tmp|
        real = File.join(tmp, "real")
        FileUtils.mkdir_p(real)
        File.write(File.join(real, "a.txt"), "A")
        File.symlink(real, File.join(tmp, "alias"))
        expect(render("<%= File.read('a.txt') %>", {}, base_dir: File.join(tmp, "alias"))).to eq("A")
      end
    end

    it "limits render recursion" do
      expect_error("<%= render('partials/_recurse.erb', n: 1) %>", "LimitError", {}, base_dir: FIXTURES)
    end
  end

  describe "resource limits" do
    it "stops runaway loops with max_steps" do
      expect_error("<% while true; end %>", "LimitError", {}, max_steps: 1000)
      expect_error("<% loop { } %>", "LimitError", {}, max_steps: 1000)
      expect_error("<% (1..Float::INFINITY.to_i rescue 1) %>", "SyntaxError")
      expect_error("<% (1..).each { } %>", "LimitError", {}, max_steps: 1000)
      expect_error("<% [1].cycle %>", "NoMethodError", {}, max_steps: 1000)
      expect_error("<% (1..30).to_a.permutation(30).first %>", "LimitError", {}, max_steps: 1000)
      expect_error("<% a = [1]; a.each { |x| a << x } %>", "LimitError", {}, max_steps: 1000)
    end

    it "caps the size of collections built from a numeric argument" do
      %w[[0]*(2**40) 'a'*(10**9) Array.new(10**9) (1..10**9).to_a 'a'.ljust(10**9)
         (1..10**12).sort.first format('%999999999d',1)].each do |expr|
        expect_error("<%= #{expr} %>", "LimitError")
      end
      expect_error("<% a = [] %><% a[10**10] = 1 %>", "LimitError")
      expect(render("<%= ('ab' * 1000).size %>,<%= (1..10**9).sum %>,<%= (1..10**12).include?(5) %>"))
        .to eq("2000,500000000500000000,true")
    end

    it "raises LimitError instead of crashing on deep nesting" do
      expect_error("<%= #{"(" * 5000}1#{")" * 5000} %>", "LimitError")
      expect_error("<%= #{'"#{' * 5000}1#{'}"' * 5000} %>", "LimitError")
      expect_error("<%= 1#{" + 1" * 200_000} %>", "LimitError")
    end
  end
end
