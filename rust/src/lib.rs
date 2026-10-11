// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! ERBLite: a sandboxed template engine with ERB syntax and a safe subset of Ruby.
//!
//! Templates are tokenized, parsed into an AST and run by a tree-walking interpreter.
//! Method calls go through an explicit allowlist; nothing a template contains is ever
//! executed as host code. The only I/O is reading files inside an optional sandbox
//! directory (`Options::base_dir`).
//!
//! ```
//! use erblite::{render, Value, Vars};
//! let mut vars = Vars::new();
//! vars.insert("name".into(), Value::from("World"));
//! assert_eq!(render("Hello <%= name %>!", &vars).unwrap(), "Hello World!");
//! ```
#![forbid(unsafe_code)]

mod ast;
mod builtins;
mod error;
mod format;
mod interp;
mod json;
mod lexer;
mod parser;
mod stack;
mod value;

pub use error::Error;
pub use value::{ClassId, RHash, Value};

use error::Flow;
use interp::{flow_to_error, Ctx, Interp};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// Host variables: the template's starting local variables.
pub type Vars = HashMap<String, Value>;

/// Rendering options.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Sandbox root for `File`, `Dir` and `render`. `None` disables file access.
    pub base_dir: Option<PathBuf>,
    /// Maximum number of evaluation steps (`None` = unlimited).
    pub max_steps: Option<u64>,
    /// File name used in error messages (default `"(erblite)"`).
    pub filename: Option<String>,
}

/// A compiled template that can be rendered many times.
///
/// Host variable names are locals declared before the template starts (Ruby's
/// `result_with_hash`), so a template is parsed once per distinct set of names.
pub struct Template {
    source: String,
    filename: String,
    parsed: RefCell<Vec<ParsedFor>>,
}

/// A parse of the template for one set of host variable names.
type ParsedFor = (Vec<String>, Rc<Vec<ast::Node>>);

/// Stack size of the helper thread used to parse very deeply nested templates.
const PARSE_STACK: usize = 64 << 20;

fn parse_here(source: &str, names: &[String]) -> Result<Vec<ast::Node>, Flow> {
    let toks = lexer::tokenize_template(source)?;
    parser::parse_program(toks, names)
}

fn is_stack_limit(f: &Flow) -> bool {
    matches!(f, Flow::Err(e) if e.message == stack::STACK_LIMIT_MESSAGE)
}

/// Parses a template. Deep templates that exhaust the caller's stack budget are parsed
/// again on a helper thread with a large stack (the AST is `Send`; values are not).
pub(crate) fn compile_program(source: &str, names: &[String]) -> Result<Vec<ast::Node>, Flow> {
    let first = {
        let _guard = stack::enter();
        parse_here(source, names)
    };
    match first {
        Err(f) if is_stack_limit(&f) => {}
        other => return other,
    }
    let result = std::thread::scope(|scope| {
        let handle = std::thread::Builder::new()
            .stack_size(PARSE_STACK)
            .spawn_scoped(scope, || {
                let _guard = stack::enter_with(PARSE_STACK - (4 << 20));
                parse_here(source, names).map_err(|f| match f {
                    Flow::Err(e) => e,
                    _ => Box::new(error::RtError {
                        kind: error::SYNTAX_ERROR,
                        message: "invalid break/next".into(),
                        line: None,
                        filename: None,
                    }),
                })
            });
        handle.ok().map(|h| h.join())
    });
    match result {
        Some(Ok(r)) => r.map_err(Flow::Err),
        Some(Err(_)) => Err(error::rt_err(error::RUNTIME_ERROR, "internal error while parsing")),
        None => Err(error::rt_err(error::LIMIT_ERROR, stack::STACK_LIMIT_MESSAGE)),
    }
}

impl Template {
    /// Parses a template. Syntax errors are reported here.
    pub fn compile(source: &str, filename: &str) -> Result<Template, Error> {
        match compile_program(source, &[]) {
            Ok(p) => Ok(Template {
                source: source.to_string(),
                filename: filename.to_string(),
                parsed: RefCell::new(vec![(Vec::new(), Rc::new(p))]),
            }),
            Err(f) => Err(flow_to_error(f, filename)),
        }
    }

    /// The parse for this set of host variable names (cached).
    fn program_for(&self, names: &[String]) -> Result<Rc<Vec<ast::Node>>, Error> {
        if let Some((_, p)) = self.parsed.borrow().iter().find(|(n, _)| n == names) {
            return Ok(p.clone());
        }
        let p = Rc::new(compile_program(&self.source, names).map_err(|f| flow_to_error(f, &self.filename))?);
        let mut cache = self.parsed.borrow_mut();
        if cache.len() >= 16 {
            cache.remove(1);
        }
        cache.push((names.to_vec(), p.clone()));
        Ok(p)
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// Renders the template with the given host variables.
    pub fn render(&self, vars: &Vars, opts: &Options) -> Result<String, Error> {
        let base_dir = match &opts.base_dir {
            Some(b) => match b.canonicalize() {
                Ok(c) => Some(c),
                Err(e) => {
                    return Err(Error {
                        kind: "IOError".into(),
                        message: format!("base_dir {}: {}", b.display(), e),
                        filename: self.filename.clone(),
                        line: 1,
                    })
                }
            },
            None => None,
        };
        let ctx = Rc::new(Ctx::new(base_dir, opts.max_steps));
        let mut names: Vec<(&String, &Value)> = vars.iter().collect();
        names.sort_by(|a, b| a.0.cmp(b.0));
        let vars: Vec<(Arc<str>, Value)> = names
            .into_iter()
            .map(|(k, v)| (Arc::from(k.as_str()), v.clone()))
            .collect();
        let host_names: Vec<String> = vars.iter().map(|(k, _)| k.to_string()).collect();
        let program = self.program_for(&host_names)?;
        let _guard = stack::enter();
        let mut it = Interp::new(ctx, &self.filename, vars, 0);
        it.run(&program).map_err(|f| flow_to_error(f, &self.filename))
    }
}

/// Renders a template with default options (no file access, no step limit).
pub fn render(source: &str, vars: &Vars) -> Result<String, Error> {
    render_with(source, vars, &Options::default())
}

/// Renders a template with the given options.
pub fn render_with(source: &str, vars: &Vars, opts: &Options) -> Result<String, Error> {
    let filename = opts.filename.clone().unwrap_or_else(|| "(erblite)".to_string());
    Template::compile(source, &filename)?.render(vars, opts)
}

/// Converts a Hash value with String or Symbol keys into host variables.
pub fn vars_from_value(v: &Value) -> Option<Vars> {
    let Value::Hash(h) = v else { return None };
    let mut out = Vars::new();
    for (k, val) in h.borrow().iter() {
        match k {
            Value::Str(s) | Value::Sym(s) => {
                out.insert(s.to_string(), val.clone());
            }
            _ => return None,
        }
    }
    Some(out)
}
