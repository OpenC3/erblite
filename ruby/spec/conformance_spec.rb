# frozen_string_literal: true

require "json"

CASES_DIR = File.expand_path("../../conformance/cases", __dir__)

RSpec.describe "ERBLite conformance" do
  Dir[File.join(CASES_DIR, "*.json")].sort.each do |file|
    group = File.basename(file, ".json")
    describe group do
      JSON.parse(File.read(file)).each do |c|
        it c["name"] do
          vars = c["vars"] || {}
          opts = { base_dir: FIXTURES, filename: "test.erb" }
          opts[:max_steps] = c["max_steps"] if c["max_steps"]
          if c.key?("expected")
            expect(ERBLite.render(c["template"], vars, **opts)).to eq(c["expected"])
          else
            expect { ERBLite.render(c["template"], vars, **opts) }.to raise_error(ERBLite::Error) { |e|
              expect(e.type).to eq(c["error"])
              expect(e.line).to eq(c["line"]) if c["line"]
            }
          end
        end
      end
    end
  end
end
