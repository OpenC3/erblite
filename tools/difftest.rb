# Differential fuzzer: generates random ERBLite expressions, uses real Ruby as the
# oracle, and checks that every implementation's CLI renders identical output.
#
# Usage: ruby tools/difftest.rb [--count N] [--seed S] [--impl python,ruby,rust]
#
# This tool evals generated (trusted) expressions in real Ruby to obtain expected
# output. It is test tooling only; ERBLite implementations never eval.

require "erb"
require "json"
require "open3"
require "optparse"
require "tmpdir"

ROOT = File.expand_path("..", __dir__)

IMPLS = {
  "python" => ->(tpl) { [File.join(ROOT, "python/.venv/bin/python"), "-m", "erblite", tpl] },
  "ruby" => ->(tpl) { ["ruby", "-I", File.join(ROOT, "ruby/lib"), File.join(ROOT, "ruby/bin/erblite"), tpl] },
  "rust" => ->(tpl) { [File.join(ROOT, "rust/target/release/erblite"), tpl] },
}

opts = { count: 2000, seed: Random.new_seed % 1_000_000, impls: IMPLS.keys }
OptionParser.new do |o|
  o.on("--count N", Integer) { |v| opts[:count] = v }
  o.on("--seed S", Integer) { |v| opts[:seed] = v }
  o.on("--impl LIST") { |v| opts[:impls] = v.split(",") }
end.parse!

class Gen
  def initialize(rng) = @r = rng
  def pick(*a) = a[@r.rand(a.size)]
  def chance(p) = @r.rand < p

  def int = pick(0, 1, 2, 3, 7, 10, -1, -7, 255, 1000, 65_535, 123_456_789, -2_147_483_648, @r.rand(-1000..1000))

  def float
    pick(0.0, -0.0, 0.1, 0.5, 1.5, 2.5, -2.5, 1.005, 2.675, 1e15, 1e16, 1e-5, 1e-4, 123.456, 1.0 / 3,
         1e300, 5e-324, 9.999999, 0.125, @r.rand * 10**@r.rand(-8..20), -@r.rand * 1000,
         (@r.rand(-1_000_000..1_000_000) / 100.0))
  end

  def str = pick("abc", "Hello World", "  pad  ", "a,b,,c,,", "", "héllo", "x.y.z", "aaa", "Tab\tNew\nline", "#{@r.rand(1000)}", "12abc", "0x1F", "3.75e2")

  def num_expr(depth = 0)
    return pick(int.to_s, flit(float)) if depth > 2 || chance(0.3)
    a = num_expr(depth + 1)
    b = num_expr(depth + 1)
    case @r.rand(12)
    when 0 then "(#{a} + #{b})"
    when 1 then "(#{a} - #{b})"
    when 2 then "(#{a} * #{b})"
    when 3 then "(#{a}).abs"
    when 4 then "(#{a}).round(#{@r.rand(0..6)})"
    when 5 then "(#{a}).round"
    when 6 then "(#{a}).floor"
    when 7 then "(#{a}).ceil"
    when 8 then "(#{a}).to_f / #{pick(3, 7, 9, 11, 0.3)}"
    when 9 then "(#{a}).divmod(#{pick(3, -3, 2.5)})"
    when 10 then "(#{a}) % #{pick(3, -3, 2.5, -0.75)}"
    else "[#{a}, #{b}].sum"
    end
  end

  def flit(f)
    s = f.to_s
    s = "(#{s})" if s.start_with?("-")
    s.sub("Infinity", "Float::INFINITY").sub("NaN", "Float::NAN")
  end

  def fmt_expr
    flags = Array.new(@r.rand(0..2)) { pick("-", "+", " ", "0", "#") }.uniq.join
    width = chance(0.5) ? @r.rand(1..12).to_s : ""
    prec = chance(0.5) ? ".#{@r.rand(0..8)}" : ""
    type = pick("d", "f", "e", "E", "g", "G", "x", "o", "b", "s", "p")
    arg = case type
          when "d", "x", "o", "b" then pick(int.abs.to_s, flit(float.abs))
          when "s", "p" then pick(str.inspect, int.to_s, flit(float), ":sym", "nil", "[1, 'a']")
          else flit(float)
          end
    "format(#{"%#{flags}#{width}#{prec}#{type}".inspect}, #{arg})"
  end

  def str_expr
    s = str.inspect
    case @r.rand(16)
    when 0 then "#{s}.split(#{pick('","', '" "', 'nil', '""', '","')}#{chance(0.3) ? ", #{pick(-1, 0, 1, 2)}" : ''}).inspect"
    when 1 then "#{s}[#{@r.rand(-4..6)}#{chance(0.5) ? ", #{@r.rand(0..4)}" : ''}].inspect"
    when 2 then "#{s}.#{pick('upcase', 'downcase', 'capitalize', 'swapcase', 'strip', 'reverse', 'chars', 'lines', 'chomp', 'chop', 'lstrip', 'rstrip')}.inspect"
    when 3 then "#{s}.#{pick('to_i', 'to_f', 'hex', 'oct', 'length', 'bytesize')}"
    when 4 then "#{s}.#{pick('center', 'ljust', 'rjust')}(#{@r.rand(0..12)}, #{pick('" "', '"*"', '"ab"')})"
    when 5 then "#{s}.gsub(#{pick('"a"', '","', '"l"', '"."')}, #{pick('"-"', '"[\\\\0]"', '""')})"
    when 6 then "#{s}.tr(#{pick('"a-y"', '"lo"', '"^a"')}, #{pick('"b-z"', '"*"', '"01"')})"
    when 7 then "#{s}.inspect"
    when 8 then "#{s}.to_sym.inspect"
    when 9 then "#{s}.index(#{pick('"a"', '"l"', '","')}).inspect"
    when 10 then "#{s}.partition(#{pick('","', '"l"', '"."')}).inspect"
    when 11 then "#{s}.start_with?(#{pick('"a"', '"H"', '""')})"
    when 12 then "#{s}.each_char.to_a.length"
    when 13 then "(#{s} * #{@r.rand(0..3)}).size"
    when 14 then "#{s}.sub(#{pick('"a"', '","')}) { |m| m.upcase }"
    else "#{s}.squeeze"
    end
  end

  def coll_expr
    arr = "[#{Array.new(@r.rand(0..6)) { pick(int.to_s, flit(float)) }.join(', ')}]"
    sarr = "[#{Array.new(@r.rand(0..5)) { str.inspect }.join(', ')}]"
    case @r.rand(14)
    when 0 then "#{arr}.sort.inspect"
    when 1 then "#{arr}.sum"
    when 2 then "#{arr}.minmax.inspect"
    when 3 then "#{arr}.each_slice(#{@r.rand(1..3)}).to_a.inspect"
    when 4 then "#{arr}.map { |x| x * 2 }.inspect"
    when 5 then "#{arr}.group_by { |x| x.to_i % 3 }.inspect"
    when 6 then "#{sarr}.tally.inspect"
    when 7 then "#{sarr}.sort_by { |s| [s.length, s] }.inspect"
    when 8 then "#{arr}.each_with_index.map { |x, i| x * i }.inspect"
    when 9 then "#{arr}.partition { |x| x > 0 }.inspect"
    when 10 then "#{arr}.inject(0) { |s, x| s + x }"
    when 11 then "#{sarr}.join(#{pick('","', '" | "', '""')}).inspect"
    when 12 then "#{arr}.to_h { |x| [x, x.to_s] }.inspect"
    else "#{sarr}.each_with_object({}) { |s, h| h[s] = s.length }.inspect"
    end
  end

  def expr = [method(:num_expr), method(:fmt_expr), method(:str_expr), method(:coll_expr)].sample(random: @r).call
end

rng = Random.new(opts[:seed])
gen = Gen.new(rng)
cases = []
attempts = 0
while cases.size < opts[:count] && attempts < opts[:count] * 20
  attempts += 1
  expr = gen.expr
  tpl = "<%= #{expr} %>"
  begin
    out = ERB.new(tpl, trim_mode: "-").result_with_hash({})
  rescue Exception # rubocop:disable Lint/RescueException
    next # only compare expressions that succeed in Ruby
  end
  next if out.include?("\n") # keep one line per case
  next if out.match?(/\d{20,}/) && !expr.include?("format") # bignums are an ERBLite RangeError
  cases << [expr, out]
end

puts "seed=#{opts[:seed]} cases=#{cases.size}"
template = cases.each_with_index.map { |(e, _), i| "#{i}: <%= #{e} %>\n" }.join
expected = cases.each_with_index.map { |(_, o), i| "#{i}: #{o}\n" }.join

failed = false
Dir.mktmpdir do |dir|
  path = File.join(dir, "fuzz.erb")
  opts[:impls].each do |name|
    File.write(path, template)
    out, err, status = Open3.capture3(*IMPLS.fetch(name).call(path))
    unless status.success?
      puts "#{name}: batch failed (#{err.lines.first&.strip}); checking cases one by one"
      bad = 0
      cases.each do |(e, o)|
        File.write(path, "<%= #{e} %>")
        o2, e2, s2 = Open3.capture3(*IMPLS.fetch(name).call(path))
        next if s2.success? && o2 == o
        next if !s2.success? && e2.include?("RangeError") # Ruby used a bignum intermediate
        bad += 1
        puts "  #{e}\n    ruby: #{o.inspect}\n    #{name}: #{s2.success? ? o2.inspect : e2.strip}"
      end
      puts "#{name}: #{bad.zero? ? 'OK (only expected RangeErrors)' : "#{bad} mismatches"}"
      failed ||= bad.positive?
      next
    end
    diffs = expected.lines.zip(out.lines).reject { |a, b| a == b }
    if diffs.empty?
      puts "#{name}: OK"
    else
      failed = true
      puts "#{name}: #{diffs.size} mismatches"
      diffs.first(25).each do |a, b|
        idx = a.to_i
        puts "  #{cases[idx][0]}\n    ruby: #{a.chomp}\n    #{name}: #{b&.chomp}"
      end
    end
  end
end
exit(failed ? 1 : 0)
