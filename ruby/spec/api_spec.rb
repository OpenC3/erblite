# frozen_string_literal: true

require "tmpdir"

RSpec.describe "ERBLite host API" do
  it "renders with symbol or string variable keys" do
    expect(ERBLite.render("<%= a %>-<%= b %>", { a: 1, "b" => "x" })).to eq("1-x")
  end

  it "defaults vars to empty" do
    expect(ERBLite.render("hi")).to eq("hi")
  end

  it "compiles once and renders many times" do
    t = ERBLite::Template.new("<%= n * 2 %>", filename: "double.erb")
    expect(t.render(n: 2)).to eq("4")
    expect(t.render(n: 5)).to eq("10")
    expect(t.filename).to eq("double.erb")
  end

  it "parses with host variable names as locals (x -1 is subtraction)" do
    expect(ERBLite.render("<%= x -1 %>", x: 5)).to eq("4")
  end

  it "reports errors with type, message, filename and line" do
    err = render_error("a\nb\n<%= nil.upcase %>")
    expect(err).to be_a(ERBLite::Error)
    expect(err).to be_a(StandardError)
    expect(err.type).to eq("NoMethodError")
    expect(err.filename).to eq("test.erb")
    expect(err.line).to eq(3)
    expect(err.message).to include("upcase")
    expect(err.to_s).to eq("test.erb:3: NoMethodError: #{err.message}")
  end

  it "uses (erblite) as the default filename" do
    expect { ERBLite.render("<%= 1 / 0 %>") }.to raise_error(ERBLite::Error) { |e|
      expect(e.to_s).to eq("(erblite):1: ZeroDivisionError: divided by 0")
    }
  end

  it "reports syntax errors at the offending line" do
    err = render_error("ok\n<% x = 1 +\n%>\n")
    expect(err.type).to eq("SyntaxError")
    expect(err.line).to eq(3)
  end

  it "reports the line of a failing call inside a multi-line block" do
    src = "<% [1, 2].each do |x| %>\nfine\n<%= x.bogus %>\n<% end %>"
    err = render_error(src)
    expect([err.type, err.line]).to eq(["NoMethodError", 3])
  end

  it "reports errors inside partials with the partial's file and line" do
    Dir.mktmpdir do |dir|
      File.write(File.join(dir, "_p.erb"), "line1\n<%= missing_thing %>")
      err = render_error("x\n<%= render('_p.erb') %>", {}, base_dir: dir)
      expect([err.type, err.filename, err.line]).to eq(["NameError", "_p.erb", 2])
    end
  end

  it "render_file defaults base_dir to the file's directory" do
    Dir.mktmpdir do |dir|
      File.write(File.join(dir, "main.erb"), "<%= render('_part.erb', v: who) %>!")
      File.write(File.join(dir, "_part.erb"), "Hi <%= v %>")
      expect(ERBLite.render_file(File.join(dir, "main.erb"), who: "Ann")).to eq("Hi Ann!")
    end
  end

  it "converts host values into template values without sharing host objects" do
    list = [1, 2]
    out = ERBLite.render("<% l << 3 %><%= l.inspect %>", l: list)
    expect(out).to eq("[1, 2, 3]")
    expect(list).to eq([1, 2])
  end

  it "preserves aliasing inside host data" do
    shared = [1]
    expect(ERBLite.render("<% a << 2 %><%= b.inspect %>", a: shared, b: shared)).to eq("[1, 2]")
  end

  it "keeps a Hash default value from the host" do
    expect(ERBLite.render("<%= h[:x] %>", h: Hash.new(7))).to eq("7")
  end

  it "rejects host objects of unsupported types" do
    expect { ERBLite.render("<%= t %>", t: Object.new) }.to raise_error(ERBLite::Error) { |e|
      expect(e.type).to eq("TypeError")
    }
  end

  it "rejects integers outside 64 bits" do
    expect { ERBLite.render("<%= n %>", n: 2**70) }.to raise_error(ERBLite::Error) { |e|
      expect(e.type).to eq("RangeError")
    }
  end

  it "enforces max_steps across partials" do
    Dir.mktmpdir do |dir|
      File.write(File.join(dir, "_loop.erb"), "<% 100.times { } %>")
      expect(ERBLite.render("<%= render('_loop.erb') %>ok", {}, base_dir: dir, max_steps: 10_000)).to eq("ok")
      err = render_error("<% 3.times { %><%= render('_loop.erb') %><% } %>", {}, base_dir: dir, max_steps: 150)
      expect(err.type).to eq("LimitError")
    end
  end

  it "has a command line tool" do
    Dir.mktmpdir do |dir|
      File.write(File.join(dir, "t.erb"), "<%= name %>: <%= File.read('x.txt') %>")
      File.write(File.join(dir, "x.txt"), "data")
      File.write(File.join(dir, "v.json"), '{"name": "N"}')
      bin = File.expand_path("../bin/erblite", __dir__)
      out = IO.popen([RbConfig.ruby, bin, File.join(dir, "t.erb"), "--vars", File.join(dir, "v.json")], &:read)
      expect(out).to eq("N: data")
      File.write(File.join(dir, "bad.erb"), "\n<%= 1 / 0 %>")
      err = IO.popen([RbConfig.ruby, bin, File.join(dir, "bad.erb")], err: [:child, :out], &:read)
      expect(err).to include("bad.erb:2: ZeroDivisionError")
      expect($?.exitstatus).to eq(1)
    end
  end
end
