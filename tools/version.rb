# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

# Keeps the package version in sync across all three implementations.
#
#   ruby tools/version.rb               # print versions; exit 1 if they differ
#   ruby tools/version.rb 0.2.0         # set every package to 0.2.0
#   ruby tools/version.rb --check v0.2.0  # exit 1 unless all equal the tag

ROOT = File.expand_path("..", __dir__)

# [file, regexp whose first capture group is the version]
LOCATIONS = [
  ["python/pyproject.toml", /^version = "([^"]+)"/],
  ["python/erblite/__init__.py", /^__version__ = "([^"]+)"/],
  ["ruby/lib/erblite/version.rb", /VERSION = "([^"]+)"/],
  ["ruby/Gemfile.lock", /^    erblite \(([^)]+)\)/],
  ["rust/Cargo.toml", /^\[package\]\nname = "erblite"\nversion = "([^"]+)"/],
  ["rust/Cargo.lock", /^name = "erblite"\nversion = "([^"]+)"/],
].freeze

def read_versions
  LOCATIONS.to_h do |file, re|
    m = File.read(File.join(ROOT, file)).match(re) or abort("#{file}: version not found")
    [file, m[1]]
  end
end

def set_version(version)
  abort("invalid version #{version.inspect}") unless version.match?(/\A\d+\.\d+\.\d+(?:[.-]?[A-Za-z0-9.]+)?\z/)
  LOCATIONS.each do |file, re|
    path = File.join(ROOT, file)
    text = File.read(path)
    m = text.match(re) or abort("#{file}: version not found")
    text[m.begin(1)...m.end(1)] = version
    File.write(path, text)
  end
end

case ARGV
in []
  versions = read_versions
  versions.each { |f, v| puts "#{v.ljust(12)} #{f}" }
  exit(versions.values.uniq.size == 1 ? 0 : 1)
in ["--check", tag]
  want = tag.delete_prefix("refs/tags/").delete_prefix("v")
  bad = read_versions.reject { |_, v| v == want }
  bad.each { |f, v| warn "#{f}: #{v} != #{want}" }
  exit(bad.empty? ? 0 : 1)
in [version]
  set_version(version)
  read_versions.each { |f, v| puts "#{v.ljust(12)} #{f}" }
else
  abort("usage: ruby tools/version.rb [VERSION | --check TAG]")
end
