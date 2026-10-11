# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

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
  spec.homepage = "https://github.com/OpenC3/erblite"
  spec.metadata = {
    "source_code_uri" => "https://github.com/OpenC3/erblite",
    "bug_tracker_uri" => "https://github.com/OpenC3/erblite/issues",
  }
  spec.required_ruby_version = ">= 3.2"
  spec.files = Dir["lib/**/*.rb", "bin/*", "README.md", "LICENSE.md"]
  spec.bindir = "bin"
  spec.executables = ["erblite"]
  spec.require_paths = ["lib"]
end
