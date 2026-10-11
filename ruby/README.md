# ERBLite for Ruby

Sandboxed ERB-syntax templates. The template language is a subset of Ruby, run by
an interpreter (no `eval`, no `send`) with no runtime dependencies. See `../SPEC.md`.

```ruby
require "erblite"
ERBLite.render("Hi <%= name.upcase %>", { name: "bob" })            # => "Hi BOB"
ERBLite.render_file("cfg.erb", { n: 1 })                            # base_dir defaults to the file's dir
t = ERBLite::Template.new(src, filename: "x.erb")
t.render({ n: 1 }, base_dir: "cfg", max_steps: 100_000)
```

Errors raise `ERBLite::Error` with `#type` (e.g. `"NoMethodError"`), `#message`,
`#filename` and `#line`.

CLI: `bin/erblite template.erb [--vars vars.json] [--base-dir DIR] [--max-steps N]`

Tests: `bundle install && bundle exec rake spec`
