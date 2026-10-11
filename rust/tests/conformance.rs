//! Runs the shared conformance suite in `../conformance/cases/*.json`.

use erblite::{vars_from_value, Options, Template, Value, Vars};
use std::path::{Path, PathBuf};

fn field(case: &Value, name: &str) -> Option<Value> {
    match case {
        Value::Hash(h) => h.borrow().get(&Value::str(name)),
        _ => None,
    }
}

fn as_str(v: &Option<Value>) -> Option<String> {
    match v {
        Some(Value::Str(s)) => Some(s.to_string()),
        _ => None,
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("conformance")
}

/// Shows the first differing position of two strings.
fn diff(expected: &str, actual: &str) -> String {
    let e: Vec<char> = expected.chars().collect();
    let a: Vec<char> = actual.chars().collect();
    let i = e.iter().zip(a.iter()).take_while(|(x, y)| x == y).count();
    let ctx = |v: &[char]| -> String { v[i.saturating_sub(20)..(i + 40).min(v.len())].iter().collect() };
    format!(
        "    expected: {:?}\n    actual:   {:?}\n    first difference at char {}: expected ...{:?}... got ...{:?}...",
        expected,
        actual,
        i,
        ctx(&e),
        ctx(&a)
    )
}

#[test]
fn conformance_suite() {
    let dir = root().join("cases");
    let fixtures = root().join("fixtures");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("conformance/cases directory")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut total = 0;
    let mut failures = Vec::new();
    for f in files {
        let group = f.file_stem().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&f).unwrap();
        let cases = Value::from_json(&text).unwrap_or_else(|e| panic!("{}: {}", f.display(), e));
        let Value::Array(cases) = cases else { panic!("{} is not an array", f.display()) };
        for case in cases.borrow().iter() {
            total += 1;
            let name = as_str(&field(case, "name")).unwrap_or_default();
            let template = as_str(&field(case, "template")).unwrap_or_default();
            let vars: Vars = field(case, "vars").and_then(|v| vars_from_value(&v)).unwrap_or_default();
            let max_steps = match field(case, "max_steps") {
                Some(Value::Int(n)) => Some(n as u64),
                _ => None,
            };
            let opts = Options { base_dir: Some(fixtures.clone()), max_steps, filename: Some("test.erb".into()) };
            let result = Template::compile(&template, "test.erb").and_then(|t| t.render(&vars, &opts));
            let id = format!("{}/{}", group, name);
            if let Some(expected) = as_str(&field(case, "expected")) {
                match result {
                    Ok(out) if out == expected => {}
                    Ok(out) => failures.push(format!("{} (template {:?})\n{}", id, template, diff(&expected, &out))),
                    Err(e) => failures.push(format!(
                        "{} (template {:?})\n    expected output {:?}\n    got error {}",
                        id, template, expected, e
                    )),
                }
            } else if let Some(kind) = as_str(&field(case, "error")) {
                let line = match field(case, "line") {
                    Some(Value::Int(n)) => Some(n as usize),
                    _ => None,
                };
                match result {
                    Err(e) if e.kind == kind && line.is_none_or(|l| l == e.line) => {}
                    Err(e) => failures.push(format!(
                        "{} (template {:?})\n    expected error {}{}\n    got error {}",
                        id,
                        template,
                        kind,
                        line.map(|l| format!(" at line {}", l)).unwrap_or_default(),
                        e
                    )),
                    Ok(out) => failures.push(format!(
                        "{} (template {:?})\n    expected error {}\n    got output {:?}",
                        id, template, kind, out
                    )),
                }
            }
        }
    }
    for f in &failures {
        println!("FAIL {}\n", f);
    }
    println!("conformance: {}/{} passed", total - failures.len(), total);
    assert!(failures.is_empty(), "{} of {} conformance cases failed", failures.len(), total);
}
