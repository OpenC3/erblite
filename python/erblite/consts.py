"""Built-in constants (§9). Only these names resolve; anything else is a NameError."""

from __future__ import annotations

import math
import sys

from .values import RClass, Value

_CLASS_NAMES = [
    "Integer",
    "Float",
    "String",
    "Symbol",
    "Array",
    "Hash",
    "Range",
    "NilClass",
    "TrueClass",
    "FalseClass",
    "Numeric",
    "Object",
    "Enumerator",
    "File",
    "Dir",
]
_MODULE_NAMES = ["Comparable", "Enumerable", "Math", "JSON"]

CONSTANTS: dict[str, Value] = {}
for _n in _CLASS_NAMES:
    CONSTANTS[_n] = RClass(_n)
for _n in _MODULE_NAMES:
    CONSTANTS[_n] = RClass(_n, is_module=True)

SCOPED: dict[str, Value] = {
    "Math::PI": math.pi,
    "Math::E": math.e,
    "Math::DomainError": RClass("Math::DomainError"),
    "Float::INFINITY": math.inf,
    "Float::NAN": math.nan,
    "Float::EPSILON": sys.float_info.epsilon,
    "Float::MAX": sys.float_info.max,
    "Float::MIN": sys.float_info.min,
    "JSON::ParserError": RClass("JSON::ParserError"),
    "JSON::GeneratorError": RClass("JSON::GeneratorError"),
}


def class_value(name: str) -> RClass:
    v = CONSTANTS.get(name)
    if isinstance(v, RClass):
        return v
    return RClass(name)
