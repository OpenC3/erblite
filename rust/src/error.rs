// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! Public error type and the internal control-flow type used by the interpreter.

use crate::value::Value;
use std::fmt;

/// An error raised while compiling or rendering a template.
///
/// `kind` is the Ruby-style error class name (`NoMethodError`, `SyntaxError`, ...).
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub kind: String,
    pub message: String,
    pub filename: String,
    pub line: usize,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}: {}", self.filename, self.line, self.kind, self.message)
    }
}

impl std::error::Error for Error {}

/// Error data carried through the interpreter before it is turned into a public [`Error`].
#[derive(Debug, Clone)]
pub struct RtError {
    pub kind: &'static str,
    pub message: String,
    pub line: Option<u32>,
    pub filename: Option<String>,
}

/// Non-local exits: errors, `break` and `next`.
#[derive(Debug)]
pub enum Flow {
    Err(Box<RtError>),
    Break(Value),
    Next(Value),
}

pub type R<T> = Result<T, Flow>;

/// Builds an error `Flow` of the given kind.
pub fn rt_err(kind: &'static str, message: impl Into<String>) -> Flow {
    Flow::Err(Box::new(RtError {
        kind,
        message: message.into(),
        line: None,
        filename: None,
    }))
}

/// Shorthand for `Err(rt_err(..))`.
pub fn fail<T>(kind: &'static str, message: impl Into<String>) -> R<T> {
    Err(rt_err(kind, message))
}

pub const SYNTAX_ERROR: &str = "SyntaxError";
pub const NAME_ERROR: &str = "NameError";
pub const NO_METHOD_ERROR: &str = "NoMethodError";
pub const ARGUMENT_ERROR: &str = "ArgumentError";
pub const TYPE_ERROR: &str = "TypeError";
pub const ZERO_DIV_ERROR: &str = "ZeroDivisionError";
pub const INDEX_ERROR: &str = "IndexError";
pub const KEY_ERROR: &str = "KeyError";
pub const RANGE_ERROR: &str = "RangeError";
pub const FLOAT_DOMAIN_ERROR: &str = "FloatDomainError";
pub const RUNTIME_ERROR: &str = "RuntimeError";
pub const MATH_DOMAIN_ERROR: &str = "Math::DomainError";
pub const JSON_PARSER_ERROR: &str = "JSON::ParserError";
pub const JSON_GENERATOR_ERROR: &str = "JSON::GeneratorError";
pub const JSON_NESTING_ERROR: &str = "JSON::NestingError";
pub const IO_ERROR: &str = "IOError";
pub const SECURITY_ERROR: &str = "SecurityError";
pub const LIMIT_ERROR: &str = "LimitError";
pub const FROZEN_ERROR: &str = "FrozenError";
