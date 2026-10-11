# frozen_string_literal: true

RSpec.describe "ERBLite language" do
  def expect_type(src, type, vars = {}, **opts)
    expect(render_error(src, vars, **opts).type).to eq(type)
  end

  describe "template syntax" do
    it "ends # comments at the end of the tag" do
      expect(render("<% a = 1 # note %>x<%= a %>")).to eq("x1")
      expect(render("<%= 2 # note %>")).to eq("2")
    end

    it "trims like ERB trim_mode '-' (including after a previous tag)" do
      expect(render("<%= 1 %>  <%- x = 1 %>y")).to eq("1y")
      expect(render("a\n  <%- x = 1 -%>\nb")).to eq("a\nb")
      expect(render("x  <%- y = 1 %>z")).to eq("x  z")
    end

    it "keeps %%> in text and turns it into %> inside tags" do
      expect(render("<%% a %%> <%= '%%>' %>")).to eq("<% a %%> %>")
    end

    it "treats the end of a code tag as a statement terminator" do
      expect_type("<% x = 1 + %><% 2 %>", "SyntaxError")
    end

    it "outputs the last statement of an output tag but cannot continue a block" do
      expect(render("<%= 1; 2 %>")).to eq("2")
      expect_type("<%= [1].each do |x| %>a<% end %>", "SyntaxError")
    end

    it "supports statements spanning several tags" do
      expect(render("<% [1, 2].each do |i| -%>\n<%= i %>,\n<% end -%>")).to eq("1,\n2,\n")
    end

    it "gives a snapshot of the output buffer as the value of text statements" do
      expect(render("<% x = if true %>hi<% end %>[<%= x %>]")).to eq("hi[hi]")
      expect(render("<% r = [1, 2].map { %>a<% } %>|<%= r.inspect %>")).to eq('aa|["a", "aa"]')
    end

    it "starts paren-less arguments with (, [, !, &:sym or -digit after a method name" do
      expect(render("<% a = [3] %><% a.push -1 %><%= a.inspect %>,<%= 3.7.round (0) %>,<%= [true].include? !false %>"))
        .to eq("[3, -1],4,true")
      expect(render("<% x = 5 %><%= x -1 %>")).to eq("4")
    end
  end

  describe "strings are values" do
    it "makes << reassign the variable instead of mutating a shared string" do
      expect(render("<% s = 'a'; t = s; t << 'b' %><%= s %>|<%= t %>")).to eq("a|ab")
    end

    it "reassigns through index targets and chains" do
      expect(render("<% h = {k: 'x'}; h[:k] << 'y' << 'z' %><%= h[:k] %>")).to eq("xyz")
    end

    it "appends an Integer code point with << and rejects other non-strings" do
      expect(render("<% s = 'n'; s << 66 %><%= s %>")).to eq("nB")
      expect_type("<% s = 'n'; s << nil %>", "TypeError")
      expect(render("<% s = 'a'.freeze; s << 'b' %><%= s %>")).to eq("ab")
    end

    it "still pushes onto arrays in place" do
      expect(render("<% a = []; b = a; b << 1 %><%= a.inspect %>")).to eq("[1]")
    end

    it "copies string literals on every evaluation" do
      expect(render("<% 2.times do %><%= 'x'.frozen? %><% end %>")).to eq("falsefalse")
      expect(render("<% s = 'abc'.freeze %><%= s.frozen? %>,<%= s.dup.frozen? %>")).to eq("true,false")
    end
  end

  describe "64-bit integers" do
    it "raises RangeError on overflow" do
      expect_type("<%= 9223372036854775807 + 1 %>", "RangeError")
      expect_type("<%= -9223372036854775807 - 2 %>", "RangeError")
      expect_type("<%= 3037000500 * 3037000500 %>", "RangeError")
      expect_type("<%= 2 ** 63 %>", "RangeError")
      expect_type("<%= 1 << 63 %>", "RangeError")
      expect_type("<%= (-9223372036854775807 - 1).abs %>", "RangeError")
      expect_type("<%= (-9223372036854775807 - 1) / -1 %>", "RangeError")
      expect_type("<%= 9223372036854775807.succ %>", "RangeError")
      expect_type("<%= '99999999999999999999'.to_i %>", "RangeError")
      expect_type("<%= Integer('9223372036854775808') %>", "RangeError")
      expect_type("<%= 1e19.to_i %>", "RangeError")
      expect_type("<%= 1e19.round %>", "RangeError")
      expect_type("<%= [9223372036854775807, 1].sum %>", "RangeError")
      expect_type("<%= 18446744073709551616 %>", "RangeError")
    end

    it "accepts the full range" do
      expect(render("<%= 9223372036854775807 %>,<%= -9223372036854775808 %>,<%= 2 ** 62 * -2 %>"))
        .to eq("9223372036854775807,-9223372036854775808,-9223372036854775808")
      expect(render("<%= [9223372036854775807, 1, -1].sum %>")).to eq("9223372036854775807")
    end

    it "raises RangeError for a negative integer exponent" do
      expect_type("<%= 2 ** -1 %>", "RangeError")
      expect(render("<%= 2.0 ** -1 %>,<%= 2 ** -1.0 %>")).to eq("0.5,0.5")
    end

    it "returns NaN where Ruby would return a Complex" do
      expect(render("<%= ((-8) ** 0.5).nan? %>,<%= ((-8.0) ** 0.5).nan? %>")).to eq("true,true")
    end

    it "shifts in the other direction for negative counts" do
      expect(render("<%= 1 << -1 %>,<%= 8 >> -1 %>,<%= -1 >> 100 %>,<%= 5 << 0 %>")).to eq("0,16,-1,5")
    end
  end

  describe "ERBLite-specific built-ins" do
    it "counts substrings with String#count" do
      expect(render("<%= 'abcabc'.count('bc') %>,<%= 'aaaa'.count('aa') %>,<%= 'abc'.count('') %>")).to eq("2,2,0")
    end

    it "uses literal patterns in sub/gsub" do
      expect(render("<%= 'a.b'.gsub('.', '-') %>,<%= 'a+b'.sub('+', '\\\\0\\\\0') %>,<%= 'abc'.gsub('', '-') %>"))
        .to eq("a-b,a++b,-a-b-c-")
      expect_type("<%= 'abc'.gsub(1, 'x') %>", "TypeError")
    end

    it "rejects negative numbers for %x/%o/%b without a sign flag" do
      expect_type("<%= format('%x', -1) %>", "ArgumentError")
      expect_type("<%= format('%b', -1) %>", "ArgumentError")
      expect(render("<%= format('%+x|% o', -255, -8) %>")).to eq("-ff|-10")
    end

    it "returns arrays from product/combination/permutation and chunk_while/slice_when" do
      expect(render("<%= [1, 2].combination(1).inspect %>,<%= [1, 2].permutation.inspect %>"))
        .to eq("[[1], [2]],[[1, 2], [2, 1]]")
      expect(render("<%= [1, 2, 4].slice_when { |a, b| b != a + 1 }.inspect %>")).to eq("[[1, 2], [4]]")
    end

    it "prints enumerators" do
      expect(render("<%= [1].each.inspect %>,<%= 3.times.inspect %>,<%= [1].each_slice(2).inspect %>"))
        .to eq("#<Enumerator: [1]:each>,#<Enumerator: 3:times>,#<Enumerator: [1]:each_slice(2)>")
      expect(render("<%= [1, 2].each.with_index(1).inspect %>"))
        .to eq("#<Enumerator: #<Enumerator: [1, 2]:each>:with_index(1)>")
    end

    it "maps Integer#chr 128..255 to the Unicode code point" do
      expect(render("<%= 233.chr %>")).to eq("é")
      expect_type("<%= 256.chr %>", "RangeError")
    end
  end

  describe "scoping" do
    it "lets blocks assign host variables" do
      expect(render("<% [1, 2].each { |i| t += i } %><%= t %>", t: 10)).to eq("13")
    end

    it "keeps for-loop variables after the loop and blocks locals inside" do
      expect(render("<% for i in 1..2; end %><%= i %>")).to eq("2")
      expect_type("<% [1].each { |i| inner = i } %><%= inner %>", "NameError")
    end

    it "gives partials only the locals passed to render" do
      expect_type("<% outer = 1 %><%= render('partials/_noscope.erb') %>", "NameError", {}, base_dir: FIXTURES)
      expect(render("<%= render('partials/_noscope.erb', outer: 5) %>", {}, base_dir: FIXTURES)).to eq("5")
    end
  end

  describe "break and next" do
    it "returns break values from the method that got the block" do
      expect(render("<%= [1, 2, 3].each { |x| break x * 10 if x == 2 } %>")).to eq("20")
      expect(render("<%= [1, 2, 3].map { |x| next 0 if x == 2; x }.inspect %>")).to eq("[1, 0, 3]")
      expect(render("<% i = 0 %><%= loop { i += 1; break i if i > 2 } %>")).to eq("3")
    end

    it "breaks the innermost loop or block" do
      src = "<% r = [] %><% [1, 2].each do |a| %><% i = 0 %><% while true %><% i += 1 %>" \
            "<% break if i > 1 %><% r << [a, i] %><% end %><% end %><%= r.inspect %>"
      expect(render(src)).to eq("[[1, 1], [2, 1]]")
    end

    it "rejects break/next outside loops" do
      expect_type("<% break %>", "SyntaxError")
      expect_type("<% if true %><% next %><% end %>", "SyntaxError")
    end
  end

  describe "Dir.glob" do
    it "supports *, ** and ?" do
      expect(render("<%= Dir.glob('partials/_???????.erb').inspect %>", {}, base_dir: FIXTURES))
        .to eq('["partials/_noscope.erb", "partials/_recurse.erb"]')
      expect(render("<%= Dir.glob('**/*').inspect %>", {}, base_dir: FIXTURES)).to include('"data/config.json"')
    end
  end
end
