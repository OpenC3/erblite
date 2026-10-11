# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

# frozen_string_literal: true

module ERBLite
  # The single exception class raised for every template problem (syntax,
  # runtime, sandbox and limit errors). +type+ is the Ruby-style error name
  # from SPEC section 11 ("NoMethodError", "Math::DomainError", ...).
  class Error < StandardError
    attr_reader :type, :filename, :line

    def initialize(type, message, filename = nil, line = nil)
      @type = type
      @raw_message = message.to_s
      @filename = filename
      @line = line
      super(@raw_message)
    end

    # The human readable message without location information.
    def message
      @raw_message
    end

    # Fills in the location if it is not known yet. Errors raised by built-in
    # methods get the line of the call that raised them.
    def fill(filename, line)
      @filename ||= filename
      @line ||= line
      self
    end

    # Display form from SPEC section 11.
    def to_s
      "#{@filename}:#{@line}: #{@type}: #{@raw_message}"
    end

    def full_message(**)
      to_s
    end

    def detailed_message(**)
      to_s
    end
  end
end
