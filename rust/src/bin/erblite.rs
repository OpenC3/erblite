#![forbid(unsafe_code)]
// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! Command line renderer: `erblite template.erb [--vars vars.json] [--base-dir DIR] [--max-steps N]`.

use erblite::{vars_from_value, Options, Template, Value, Vars};
use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!("usage: erblite TEMPLATE [--vars VARS.json] [--base-dir DIR] [--max-steps N]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut template: Option<String> = None;
    let mut vars_path: Option<String> = None;
    let mut base_dir: Option<PathBuf> = None;
    let mut max_steps: Option<u64> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--vars" | "--base-dir" | "--max-steps" => {
                let Some(v) = args.get(i + 1) else { return usage() };
                match args[i].as_str() {
                    "--vars" => vars_path = Some(v.clone()),
                    "--base-dir" => base_dir = Some(PathBuf::from(v)),
                    _ => match v.parse() {
                        Ok(n) => max_steps = Some(n),
                        Err(_) => return usage(),
                    },
                }
                i += 2;
            }
            "-h" | "--help" => return usage(),
            other if template.is_none() => {
                template = Some(other.to_string());
                i += 1;
            }
            _ => return usage(),
        }
    }
    let Some(template) = template else { return usage() };
    let source = match std::fs::read_to_string(&template) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("erblite: {}: {}", template, e);
            return ExitCode::FAILURE;
        }
    };
    let vars: Vars = match vars_path {
        None => Vars::new(),
        Some(p) => {
            let text = match std::fs::read_to_string(&p) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("erblite: {}: {}", p, e);
                    return ExitCode::FAILURE;
                }
            };
            let v = match Value::from_json(&text) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("erblite: {}: {}", p, e);
                    return ExitCode::FAILURE;
                }
            };
            match vars_from_value(&v) {
                Some(vars) => vars,
                None => {
                    eprintln!("erblite: {}: variables must be a JSON object", p);
                    return ExitCode::FAILURE;
                }
            }
        }
    };
    let base_dir = base_dir.or_else(|| {
        PathBuf::from(&template).parent().map(|p| {
            if p.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                p.to_path_buf()
            }
        })
    });
    let opts = Options {
        base_dir,
        max_steps,
        filename: Some(template.clone()),
    };
    let result = Template::compile(&source, &template).and_then(|t| t.render(&vars, &opts));
    match result {
        Ok(out) => {
            print!("{}", out);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e);
            ExitCode::FAILURE
        }
    }
}
