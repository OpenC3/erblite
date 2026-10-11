"""Error types for ERBLite."""

from __future__ import annotations


class Error(Exception):
    """A template error, visible to the host.

    ``type`` is the Ruby-style error class name (``NoMethodError``, ``SyntaxError``, ...),
    ``message`` a human readable message, ``filename`` the template name and ``line`` the
    1-based line number where the error happened.
    """

    def __init__(self, type: str, message: str, filename: str = "(erblite)", line: int = 0) -> None:
        self.type = type
        self.message = message
        self.filename = filename
        self.line = line
        super().__init__(str(self))

    def __str__(self) -> str:
        return f"{self.filename}:{self.line}: {self.type}: {self.message}"

    def __repr__(self) -> str:
        return f"erblite.Error({self.type!r}, {self.message!r}, {self.filename!r}, {self.line!r})"


class RubyError(Exception):
    """Internal: an error raised while running a template, before location is attached."""

    def __init__(self, type: str, message: str, line: int | None = None) -> None:
        super().__init__(message)
        self.type = type
        self.message = message
        self.line = line


def syntax_error(message: str, line: int | None = None) -> RubyError:
    return RubyError("SyntaxError", message, line)


def arg_error(message: str) -> RubyError:
    return RubyError("ArgumentError", message)


def type_error(message: str) -> RubyError:
    return RubyError("TypeError", message)


def range_error(message: str) -> RubyError:
    return RubyError("RangeError", message)


def argc_error(given: int, expected: str) -> RubyError:
    return RubyError("ArgumentError", f"wrong number of arguments (given {given}, expected {expected})")
