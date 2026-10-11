# Generates conformance/cases/*.json from conformance/src/*.yaml.
#
# By default each case's expected output (or error class) is produced by running
# the template through real Ruby ERB (trim_mode "-"), which is the oracle that
# defines ERBLite semantics.  Cases marked `oracle: false` cover ERBLite-specific
# behavior (sandbox, render, overflow, limits) and carry hand-written expectations.
#
# This script evals trusted test templates with real Ruby. It is a test-generation
# tool only; no ERBLite implementation may ever use eval.
#
# Usage: ruby conformance/generate.rb [--check]
#   --check  fail if regenerated output differs from the committed JSON

require "erb"
require "json"
require "yaml"

ROOT = File.expand_path(__dir__)
FIXTURES = File.join(ROOT, "fixtures")

def ruby_vars(value)
  case value
  when Hash then value.to_h { |k, v| [k.to_s, ruby_vars(v)] }
  when Array then value.map { |v| ruby_vars(v) }
  else value
  end
end

def run_oracle(template, vars)
  locals = (vars || {}).to_h { |k, v| [k.to_sym, ruby_vars(v)] }
  Dir.chdir(FIXTURES) do
    out = ERB.new(template, trim_mode: "-").result_with_hash(locals)
    { "expected" => out }
  end
rescue Exception => e # rubocop:disable Lint/RescueException
  { "error" => e.class.name }
end

# Wrapped in a method: ERB's result_with_hash evaluates templates in a copy of
# TOPLEVEL_BINDING, so top-level locals here would leak into test templates.
def main
  check = ARGV.include?("--check")
  failures = []
  total = 0

  Dir[File.join(ROOT, "src", "*.yaml")].sort.each do |src|
    base = File.basename(src, ".yaml")
    cases = YAML.safe_load(File.read(src), permitted_classes: [], aliases: false) || []
    out = cases.each_with_index.map do |c, i|
      raise "#{src}[#{i}]: missing t" unless c.key?("t")
      name = c["n"] || "#{base}_#{i + 1}"
      vars = c["v"] || {}
      result = {}
      if c.fetch("oracle", true)
        result = run_oracle(c["t"], vars)
        # Hand-written expectations on oracle cases must agree with Ruby.
        if c.key?("x") && result["expected"] != c["x"]
          failures << "#{base}/#{name}: yaml expects #{c['x'].inspect}, ruby gives #{result.inspect}"
        end
        if c.key?("e") && result["error"] != c["e"]
          failures << "#{base}/#{name}: yaml expects error #{c['e']}, ruby gives #{result.inspect}"
        end
      else
        if c.key?("e") then result["error"] = c["e"]
        elsif c.key?("x") then result["expected"] = c["x"]
        else raise "#{base}/#{name}: oracle:false case needs x or e"
        end
      end
      entry = { "name" => name, "template" => c["t"], "vars" => vars }
      entry["max_steps"] = c["max_steps"] if c["max_steps"]
      entry["line"] = c["line"] if c["line"]
      entry["ruby_compatible"] = c.fetch("oracle", true)
      entry.merge(result)
    end
    total += out.size
    json = JSON.pretty_generate(out) + "\n"
    dest = File.join(ROOT, "cases", "#{base}.json")
    if check
      failures << "#{dest} is stale" unless File.exist?(dest) && File.read(dest) == json
    else
      File.write(dest, json)
    end
  end

  if failures.any?
    warn failures.join("\n")
    exit 1
  end
  puts "#{total} cases #{check ? 'verified' : 'generated'}"
end

main
