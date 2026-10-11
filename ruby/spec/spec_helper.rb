# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

# frozen_string_literal: true

$LOAD_PATH.unshift File.expand_path("../lib", __dir__)
require "erblite"

FIXTURES = File.expand_path("../../conformance/fixtures", __dir__)

module RenderHelper
  def render(src, vars = {}, **opts)
    ERBLite.render(src, vars, filename: "test.erb", **opts)
  end

  def render_error(src, vars = {}, **opts)
    render(src, vars, **opts)
    raise "expected an ERBLite::Error"
  rescue ERBLite::Error => e
    e
  end
end

RSpec.configure do |c|
  c.include RenderHelper
end
