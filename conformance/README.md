# ERBLite conformance suite

* `src/*.yaml` — hand-written cases (`n` name, `t` template, `v` vars, `x` expected,
  `e` error type, `line` error line, `max_steps`, `oracle: false` for ERBLite-only behavior).
* `cases/*.json` — generated; **every implementation must pass all of them**.
* `fixtures/` — the `base_dir` used for every case (File/JSON/render tests).
* `generate.rb` — regenerates `cases/` using real Ruby ERB as the oracle
  (`ruby conformance/generate.rb`, or `--check` in CI).

## Runner contract

For every case in `cases/*.json`:

1. Render `template` with host variables `vars` (JSON values; object keys become
   String keys), `base_dir = conformance/fixtures`, `filename = "test.erb"`, and
   `max_steps` if present.
2. If the case has `expected`, the output must equal it exactly.
3. If the case has `error`, rendering must fail with that error type. If the case
   also has `line`, the error's line must match.
