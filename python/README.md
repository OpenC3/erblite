# ERBLite for Python

Sandboxed ERB-syntax templates (a subset of Ruby), with no runtime dependencies. See `../SPEC.md`.

```python
import erblite
erblite.render("Hi <%= name.upcase %>", {"name": "bob"})          # "Hi BOB"
erblite.render_file("cfg.erb", {"n": 1})                          # base_dir defaults to the file's dir
t = erblite.Template(src, filename="x.erb"); t.render(vars, base_dir="cfg", max_steps=100_000)
```

CLI: `python -m erblite template.erb [--vars vars.json] [--base-dir DIR] [--max-steps N]`

Tests: `python3 -m venv .venv && .venv/bin/pip install -e . pytest ruff && .venv/bin/pytest && .venv/bin/ruff check erblite tests`
