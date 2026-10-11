# frozen_string_literal: true

require_relative "lib/erblite/version"

Gem::Specification.new do |spec|
  spec.name = "erblite"
  spec.version = ERBLite::VERSION
  spec.authors = ["OpenC3"]
  spec.summary = "Sandboxed ERB-syntax template engine running a safe subset of Ruby"
  spec.description = "ERBLite renders ERB templates with a tree-walking interpreter for a small, " \
                     "sandboxed subset of Ruby. It never evals template code."
  spec.license = "MIT"
  spec.required_ruby_version = ">= 3.1"
  spec.files = Dir["lib/**/*.rb", "bin/*", "README.md"]
  spec.bindir = "bin"
  spec.executables = ["erblite"]
  spec.require_paths = ["lib"]
end
