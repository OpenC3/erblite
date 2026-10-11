# ERBLite for Rust

Sandboxed ERB-syntax templates. The template language is a subset of Ruby, run by
a tree-walking interpreter. The crate has no dependencies and uses
`#![forbid(unsafe_code)]`. See `../SPEC.md`.

```rust
use erblite::{Options, Template, Value, Vars};

let mut vars = Vars::new();
vars.insert("name".into(), Value::from("bob"));
assert_eq!(erblite::render("Hi <%= name.upcase %>", &vars)?, "Hi BOB");

let t = Template::compile(src, "x.erb")?;
let out = t.render(&vars, &Options { base_dir: Some("cfg".into()), max_steps: Some(100_000), ..Default::default() })?;
```

`Value::from_json(&str)` builds variables from JSON. Errors are `erblite::Error`
with `kind` (e.g. `"NoMethodError"`), `message`, `filename` and `line`.

CLI: `cargo run --release --bin erblite -- template.erb [--vars vars.json] [--base-dir DIR] [--max-steps N]`

Tests: `cargo test` (the conformance runner is `tests/conformance.rs`)
