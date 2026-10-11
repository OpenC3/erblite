// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! Security, resource-limit and robustness tests. Templates must never crash the host.

use erblite::{render, render_with, Error, Options, Template, Value, Vars};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("conformance")
        .join("fixtures")
}

fn sandboxed() -> Options {
    Options {
        base_dir: Some(fixtures()),
        max_steps: None,
        filename: Some("t.erb".into()),
    }
}

fn run(src: &str) -> Result<String, Error> {
    render_with(src, &Vars::new(), &sandboxed())
}

fn err_kind(src: &str) -> String {
    match run(src) {
        Ok(out) => panic!("expected an error for {:?}, got output {:?}", src, out),
        Err(e) => e.kind,
    }
}

// ----- code execution escapes ----------------------------------------------------------------

#[test]
fn no_dynamic_dispatch_or_eval() {
    for src in [
        "<%= 1.send(:+, 2) %>",
        "<%= 1.public_send(:+, 2) %>",
        "<%= 1.__send__(:+, 2) %>",
        "<%= eval('1') %>",
        "<%= 'x'.instance_eval('1') %>",
        "<%= 'x'.instance_variable_get(:@a) %>",
        "<%= 1.method(:+) %>",
        "<%= system('echo hi') %>",
        "<%= exec('ls') %>",
        "<%= spawn('ls') %>",
        "<%= open('|ls') %>",
        "<%= require 'socket' %>",
        "<%= proc { 1 } %>",
        "<%= lambda { 1 } %>",
        "<%= 1.class.ancestors %>",
        "<%= 'x'.define_singleton_method(:a) { 1 } %>",
        "<%= File.write('x', 'y') %>",
        "<%= File.open('data/lines.txt') %>",
        "<%= File.delete('data/lines.txt') %>",
        "<%= Dir.entries('.') %>",
        "<%= [].instance_variables %>",
        "<%= 'x'.respond_to?(:system) ? 1 : nil.foo %>",
    ] {
        assert_eq!(err_kind(src), "NoMethodError", "{}", src);
    }
}

#[test]
fn bare_unknown_names_are_name_errors() {
    for src in ["<%= binding %>", "<%= puts %>", "<%= exit %>", "<%= self_destruct %>"] {
        assert_eq!(err_kind(src), "NameError", "{}", src);
    }
}

#[test]
fn no_host_constants_or_globals() {
    for src in [
        "<%= ENV['HOME'] %>",
        "<%= Kernel.exit %>",
        "<%= ObjectSpace %>",
        "<%= Process.pid %>",
        "<%= IO %>",
        "<%= Object.const_get(:ENV) %>",
    ] {
        let k = err_kind(src);
        assert!(k == "NameError" || k == "NoMethodError", "{} gave {}", src, k);
    }
    for src in [
        "<%= $0 %>",
        "<%= @x %>",
        "<%= `ls` %>",
        "<%= %x(ls) %>",
        "<% def x; end %>",
        "<%= self %>",
        "<% BEGIN { } %>",
        "<%= ->{ 1 } %>",
        "<%= __FILE__ %>",
    ] {
        assert_eq!(err_kind(src), "SyntaxError", "{}", src);
    }
}

#[test]
fn respond_to_matches_dispatch_tables() {
    // every name respond_to? reports must be dispatchable (never NoMethodError)
    let samples = [
        "nil", "true", "1", "1.5", "'s'", ":s", "[1, 2]", "{a: 1}", "(1..3)", "[1].each", "Integer", "Math", "JSON",
        "File", "Dir", "Array", "Hash",
    ];
    let names = [
        "==",
        "!=",
        "!",
        "===",
        "eql?",
        "nil?",
        "to_s",
        "inspect",
        "class",
        "frozen?",
        "dup",
        "freeze",
        "to_json",
        "abs",
        "size",
        "length",
        "first",
        "last",
        "to_a",
        "to_h",
        "to_i",
        "to_f",
        "keys",
        "values",
        "upcase",
        "name",
        "each",
        "map",
        "sum",
        "min",
        "max",
        "sort",
        "uniq",
        "reverse",
        "empty?",
        "succ",
        "chars",
        "begin",
        "end",
        "send",
        "eval",
        "system",
        "instance_eval",
        "method",
        "__send__",
        "object_id",
        "fetch",
        "concat",
        "<<",
        "+",
        "-",
        "*",
        "/",
        "%",
    ];
    for s in samples {
        for n in names {
            let responds = run(&format!("<%= {}.respond_to?({:?}) %>", s, n)).unwrap() == "true";
            let call = run(&format!("<% {}.{}() %>", s, n));
            match call {
                Err(e) if e.kind == "NoMethodError" => {
                    assert!(
                        !responds,
                        "{}.respond_to?({}) is true but calling it raised NoMethodError",
                        s, n
                    )
                }
                Err(e) if e.kind == "SyntaxError" => {}
                _ => assert!(responds, "{}.{} works but respond_to? is false", s, n),
            }
        }
        for forbidden in [
            "send",
            "eval",
            "system",
            "instance_eval",
            "method",
            "__send__",
            "object_id",
            "instance_variable_get",
        ] {
            assert_eq!(
                run(&format!("<%= {}.respond_to?(:{}) %>", s, forbidden)).unwrap(),
                "false"
            );
        }
    }
}

// ----- file sandbox ---------------------------------------------------------------------------

#[test]
fn sandbox_blocks_traversal() {
    for src in [
        "<%= File.read('../outside.txt') %>",
        "<%= File.read('data/../../outside.txt') %>",
        "<%= File.read('/etc/passwd') %>",
        "<%= File.read('./../outside.txt') %>",
        "<%= File.read('data/../../conformance/outside.txt') %>",
        "<%= render('../outside.txt') %>",
        "<%= Dir.glob('../*') %>",
        "<%= Dir.glob('/etc/*') %>",
        "<%= Dir.glob('data/../../*') %>",
        "<%= File.read(\"data/\\0x\") %>",
    ] {
        assert_eq!(err_kind(src), "SecurityError", "{}", src);
    }
    assert_eq!(
        run("<%= File.exist?('../outside.txt') %>,<%= File.file?('/etc/passwd') %>").unwrap(),
        "false,false"
    );
    assert_eq!(run("<%= File.read('data/../data/lines.txt').size %>").unwrap(), "18");
}

#[test]
fn sandbox_disabled_without_base_dir() {
    let opts = Options::default();
    for src in [
        "<%= File.read('x') %>",
        "<%= File.exist?('x') %>",
        "<%= Dir.glob('*') %>",
        "<%= render('x') %>",
    ] {
        let e = render_with(src, &Vars::new(), &opts).unwrap_err();
        assert_eq!(e.kind, "SecurityError", "{}", src);
    }
    // pure path helpers still work
    assert_eq!(render("<%= File.join('a', 'b') %>", &Vars::new()).unwrap(), "a/b");
}

#[cfg(unix)]
#[test]
fn sandbox_blocks_symlink_escape() {
    let dir = std::env::temp_dir().join(format!("erblite-sandbox-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("base/sub")).unwrap();
    std::fs::write(dir.join("secret.txt"), "secret").unwrap();
    std::fs::write(dir.join("base/ok.txt"), "ok").unwrap();
    std::os::unix::fs::symlink(dir.join("secret.txt"), dir.join("base/link.txt")).unwrap();
    std::os::unix::fs::symlink(&dir, dir.join("base/sub/up")).unwrap();
    std::os::unix::fs::symlink(dir.join("base/ok.txt"), dir.join("base/inner.txt")).unwrap();
    let opts = Options {
        base_dir: Some(dir.join("base")),
        ..Default::default()
    };
    let r = |s: &str| render_with(s, &Vars::new(), &opts);
    assert_eq!(r("<%= File.read('link.txt') %>").unwrap_err().kind, "SecurityError");
    assert_eq!(
        r("<%= File.read('sub/up/secret.txt') %>").unwrap_err().kind,
        "SecurityError"
    );
    assert_eq!(r("<%= render('link.txt') %>").unwrap_err().kind, "SecurityError");
    assert_eq!(r("<%= File.exist?('link.txt') %>").unwrap(), "false");
    assert_eq!(r("<%= File.read('inner.txt') %>").unwrap(), "ok");
    assert_eq!(r("<%= File.read('missing.txt') %>").unwrap_err().kind, "IOError");
    assert_eq!(
        r("<%= File.read('sub/up/missing.txt') %>").unwrap_err().kind,
        "SecurityError"
    );
    let globbed = r("<%= Dir.glob('**/*.txt') %>").unwrap();
    assert!(!globbed.contains("secret"), "{}", globbed);
    let _ = std::fs::remove_dir_all(&dir);
}

// ----- resource limits ------------------------------------------------------------------------

#[test]
fn step_limit_stops_runaway_loops() {
    let opts = Options {
        max_steps: Some(10_000),
        ..Default::default()
    };
    for src in [
        "<% loop { } %>",
        "<% while true; end %>",
        "<% x = 0 %><% x += 1 until false %>",
        "<% (1..).each { } %>",
        "<% 1.step(nil, 1) { } %>",
        "<% (10**18).times { } %>",
        "<% [1].cycle { } rescue 1 %>",
        "<% (0..10**12).sum { |x| x } %>",
        "<% (1..).each(&:to_s) %>",
        "<% 1.step(nil, 1).each(&:to_s) %>",
    ] {
        match render_with(src, &Vars::new(), &opts) {
            Err(e) => assert!(
                e.kind == "LimitError" || e.kind == "SyntaxError" || e.kind == "NoMethodError",
                "{}: {}",
                src,
                e
            ),
            Ok(o) => panic!("{} should not finish: {:?}", src, o),
        }
    }
}

#[test]
fn allocation_limits() {
    for src in [
        "<%= 'a' * (2**40) %>",
        "<%= [1] * (2**40) %>",
        "<%= Array.new(2**40) %>",
        "<%= 'a'.ljust(2**40) %>",
        "<% a = [] %><% a[2**40] = 1 %>",
        "<%= (1..2**40).to_a.size %>",
        "<%= format('%2000000000d', 1) %>",
        "<% s = 'ab' %><% 40.times { s += s } %>",
        "<% a = (1..10).to_a %><%= a.product(a, a, a, a, a, a, a).size %>",
    ] {
        let k = err_kind(src);
        assert!(
            k == "LimitError" || k == "RangeError" || k == "ArgumentError",
            "{}: {}",
            src,
            k
        );
    }
}

#[test]
fn render_nesting_limit() {
    assert_eq!(err_kind("<%= render('partials/_recurse.erb', n: 1) %>"), "LimitError");
}

/// Runs `f` on a thread with a small stack, as a host might.
fn on_small_stack<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn deep_nesting_does_not_overflow() {
    on_small_stack(|| {
        let n = 5000;
        let cases = vec![
            format!("<%= {}1{} %>", "(".repeat(n), ")".repeat(n)),
            format!("<%= {}1{} %>", "[".repeat(n), "]".repeat(n)),
            format!("<%= {}1{} %>", "{a: ".repeat(n), "}".repeat(n)),
            format!("<%= {}true %>", "!".repeat(n)),
            format!("<%= {}1 %>", "- ".repeat(n)),
            format!("<%= {}1 %>", "not ".repeat(n)),
            format!("<%= 1{} %>", " ** 1".repeat(n)),
            format!("<%= x = {}1 %>", "y = ".repeat(n)),
            format!("<%= {}1{} %>", "\"#{".repeat(n), "}\"".repeat(n)),
            format!("<%= {} 1 {} %>", "if true then".repeat(n), " end".repeat(n)),
            format!("<% {} %><% {} %>", "[1].each do |x|".repeat(n), "end;".repeat(n)),
            format!("<%= {}1{} %>", "[1].map { |x| ".repeat(n), " }".repeat(n)),
            format!("<%= 1{} %>", ".abs".repeat(n * 4)),
            format!("<%= 1{} %>", " + 1".repeat(n * 4)),
        ];
        for src in cases {
            match render(&src, &Vars::new()) {
                Ok(_) => {}
                Err(e) => assert!(e.kind == "LimitError" || e.kind == "SyntaxError", "{}", e),
            }
        }
        // moderate nesting works even in debug builds (release builds handle 256+ levels)
        let src = format!("<%= {}1{} %>", "[".repeat(60), "]".repeat(60));
        assert!(render(&src, &Vars::new()).is_ok());
        let src = format!("<%= {}1{} %>", "[1].map { |x| ".repeat(20), " }".repeat(20));
        assert!(render(&src, &Vars::new()).is_ok());
    });
}

#[test]
fn deeply_nested_data_is_safe() {
    on_small_stack(|| {
        // building 100k levels of nesting, then inspecting, comparing, serializing and dropping it
        for tail in [
            "<%= a.inspect.size %>",
            "<%= a == a.dup %>",
            "<%= a.to_json %>",
            "<%= a.flatten.size %>",
            "<%= {a => 1}.size %>",
            "<%= a.hash rescue 1 %>",
            "",
        ] {
            let src = format!("<% a = [] %><% 100000.times {{ a = [a] }} %>{}", tail);
            let _ = render(&src, &Vars::new());
        }
        let _ = render(
            "<% h = {} %><% 100000.times { h = {k: h} } %><%= h.inspect.size %>",
            &Vars::new(),
        );
        let _ = render(
            "<% e = [1].each %><% 100000.times { e = e.with_index } %><%= e.inspect.size %>",
            &Vars::new(),
        );
        // self-referencing structures
        assert_eq!(
            render("<% a = [1] %><% a << a %><%= a.inspect %>", &Vars::new()).unwrap(),
            "[1, [...]]"
        );
        assert_eq!(
            render("<% h = {} %><% h[:h] = h %><%= h.inspect %>", &Vars::new()).unwrap(),
            "{h: {...}}"
        );
        assert_eq!(
            render("<% a = [] %><% a << a %><%= a.flatten rescue 1 %>", &Vars::new())
                .unwrap_err()
                .kind,
            "SyntaxError"
        );
        assert_eq!(
            render("<% a = [] %><% a << a %><%= a.flatten %>", &Vars::new())
                .unwrap_err()
                .kind,
            "ArgumentError"
        );
        assert_eq!(
            render("<% a = [] %><% a << a %><%= a.join %>", &Vars::new())
                .unwrap_err()
                .kind,
            "ArgumentError"
        );
        assert!(render("<% a = [] %><% a << a %><%= a == a %>", &Vars::new()).is_ok());
        assert!(render("<% a = [] %><% a << a %><%= a.to_json %>", &Vars::new()).is_err());
    });
}

#[test]
fn integer_overflow_raises() {
    for src in [
        "<%= 9223372036854775807 + 1 %>",
        "<%= -9223372036854775808 - 1 %>",
        "<%= 4611686018427387904 * 2 %>",
        "<%= (-9223372036854775807 - 1) / -1 %>",
        "<%= (-9223372036854775807 - 1).abs %>",
        "<%= 3 ** 40 %>",
        "<%= 1 << 63 %>",
        "<%= 9223372036854775807.succ %>",
        "<%= [9223372036854775807, 1].sum %>",
        "<%= (1..9223372036854775807).sum %>",
        "<%= 9223372036854775807.round(-1) %>",
        "<%= '99999999999999999999'.to_i %>",
        "<%= Integer('99999999999999999999') %>",
        "<%= 1e19.to_i %>",
        "<%= (10.0**20).round %>",
    ] {
        let k = err_kind(src);
        assert!(k == "RangeError", "{}: {}", src, k);
    }
    assert_eq!(run("<%= (-9223372036854775807 - 1) % -1 %>").unwrap(), "0");
}

// ----- API ------------------------------------------------------------------------------------

#[test]
fn public_api() {
    let mut vars = Vars::new();
    vars.insert("name".into(), Value::from("World"));
    vars.insert("n".into(), Value::from(3));
    vars.insert("list".into(), Value::from(vec![1, 2, 3]));
    vars.insert(
        "cfg".into(),
        Value::from_json(r#"{"a": {"b": [1, 2.5, null]}}"#).unwrap(),
    );
    let t = Template::compile(
        "<%= name %>:<%= n * 2 %>:<%= list.sum %>:<%= cfg['a']['b'].inspect %>",
        "x.erb",
    )
    .unwrap();
    assert_eq!(t.render(&vars, &Options::default()).unwrap(), "World:6:6:[1, 2.5, nil]");
    // templates can be rendered repeatedly; values are not shared between renders
    let t = Template::compile("<% list << 4 %><%= list.size %>", "y.erb").unwrap();
    assert_eq!(t.render(&vars, &Options::default()).unwrap(), "4");

    let e = Template::compile("line1\n<% if %>", "bad.erb").err().unwrap();
    assert_eq!(
        (e.kind.as_str(), e.line, e.filename.as_str()),
        ("SyntaxError", 2, "bad.erb")
    );
    assert!(e.to_string().starts_with("bad.erb:2: SyntaxError: "));

    let e = render_with(
        "\n\n<%= nil.upcase %>",
        &Vars::new(),
        &Options {
            filename: Some("f.erb".into()),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        e.to_string(),
        "f.erb:3: NoMethodError: undefined method 'upcase' for nil"
    );
    let _: &dyn std::error::Error = &e;
}

#[test]
fn errors_in_partials_report_partial_location() {
    let e = run("<%= render('partials/_noscope.erb') %>").unwrap_err();
    assert_eq!(
        (e.kind.as_str(), e.filename.as_str(), e.line),
        ("NameError", "partials/_noscope.erb", 1)
    );
}

// ----- fuzzing --------------------------------------------------------------------------------

/// Small deterministic PRNG (xorshift) so the test needs no dependencies.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const SEEDS: &[&str] = &[
    "<% [1, 2, 3].each_with_index do |x, i| %><%= x * i %>,<% end %>",
    "<%= {a: 1, 'b' => [2, 3.5]}.map { |k, v| \"#{k}=#{v}\" }.join('&') %>",
    "<% x = 5 %><%= x > 3 ? format('%05.2f', x) : x.to_s.rjust(4, '0') %>",
    "<%- if cond -%>\nyes <%= 'a'.upcase %>\n<%- else -%>\nno\n<%- end -%>",
    "<%= (1..10).step(3).map { |i| i ** 2 }.select(&:even?).sum %>",
    "<% h = Hash.new(0) %><% %w[a b a].each { |w| h[w] += 1 } %><%= h.sort_by { |k, v| -v }.to_h %>",
    "<%= JSON.generate({x: [1, nil, 'q\"'], y: 1.5e-7}) %><%= JSON.parse('[1, {\"a\": 2}]') %>",
    "<% case 7 when 1..5 then %>low<% when Integer then %>int<% end %>",
    "<%= 'Hello World'.gsub('o', '0').split(' ').map(&:reverse).inspect %>",
    "<% i = 0 %><% while i < 3 do i += 1; next if i == 2 %><%= i %><% end %>",
    "<%= [3, 1, 2].sort.reverse.each_slice(2).to_a.flatten.uniq.tally %>",
    "<%= render('partials/_greet.erb', name: 'x') %><%= File.read('data/lines.txt').lines.size %>",
    "<% a, b = [1, [2, 3]] %><%= a %><%= b.dig(1) %><%= -2 ** 2 %><%= 7 / -2 %><%= 7 % -3 %><%= 1 <=> 2 %>",
    "<%= \"#{1}#{nil}#{[1]}\" %><%= :\"a b\".inspect %><%= %w[x y].inspect %><%= 0x1f + 0b11 + 0o7 + 1_000 %>",
    "<%= Math.sqrt(2).round(3) %><%= 10.fdiv(3).floor(2) %><%= 1e20.to_s %><%= 2.5.round %><%= Float::INFINITY %>",
];

const FRAGMENTS: &[&str] = &[
    "<%",
    "%>",
    "<%=",
    "-%>",
    "<%-",
    "<%#",
    "<%%",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    "|",
    "||",
    "&&",
    "do",
    "end",
    "if",
    "else",
    "elsif",
    "while",
    "until",
    "for",
    "in",
    "case",
    "when",
    "then",
    "break",
    "next",
    "x",
    "nil",
    "1",
    "-1",
    "0.5",
    "'s'",
    "\"#{",
    "\"",
    "'",
    ":",
    "::",
    "?",
    ",",
    ".",
    "&.",
    "..",
    "...",
    "=",
    "==",
    "+",
    "-",
    "*",
    "**",
    "/",
    "%",
    "<<",
    ">>",
    "<=>",
    "!",
    "not",
    "and",
    "or",
    "\n",
    ";",
    " ",
    "#",
    "\\",
    "a:",
    "=>",
    "&:upcase",
    "x.each",
    ".map",
    ".sum",
    ".inspect",
    "%w[",
    "@",
    "$",
    "`",
    "9223372036854775807",
    "1e308",
    "Float::NAN",
    "[1, [2]]",
    "{a: {b: 1}}",
    ".each_slice(0)",
    ".flatten",
    ".sort",
    "loop",
    "render",
    "File.read",
    "'..'",
    "Integer('z')",
    "format('%",
    "%d",
    "%-5.2f",
    "%<a>s",
    "\u{1F600}",
    "é",
    "\0",
    "\r\n",
];

#[test]
fn fuzz_no_panics() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let opts = Options {
        base_dir: Some(fixtures()),
        max_steps: Some(20_000),
        filename: None,
    };
    let mut vars = Vars::new();
    vars.insert("cond".into(), Value::from(true));
    vars.insert(
        "x".into(),
        Value::from(vec![Value::from(1), Value::from("two"), Value::Nil]),
    );
    let iterations = 20_000;
    for i in 0..iterations {
        let mut src: Vec<char> = SEEDS[rng.below(SEEDS.len())].chars().collect();
        let edits = 1 + rng.below(6);
        for _ in 0..edits {
            match rng.below(4) {
                0 if !src.is_empty() => {
                    let p = rng.below(src.len());
                    let l = 1 + rng.below(5.min(src.len() - p));
                    src.drain(p..p + l);
                }
                1 => {
                    let p = rng.below(src.len() + 1);
                    let frag: Vec<char> = FRAGMENTS[rng.below(FRAGMENTS.len())].chars().collect();
                    src.splice(p..p, frag);
                }
                2 if !src.is_empty() => {
                    let p = rng.below(src.len());
                    src[p] = char::from_u32(rng.below(128) as u32).unwrap_or(' ');
                }
                _ => {
                    // splice in a chunk of another seed
                    let other: Vec<char> = SEEDS[rng.below(SEEDS.len())].chars().collect();
                    let a = rng.below(other.len());
                    let b = a + rng.below(other.len() - a + 1);
                    let p = rng.below(src.len() + 1);
                    src.splice(p..p, other[a..b].iter().cloned());
                }
            }
        }
        let text: String = src.into_iter().collect();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = render_with(&text, &vars, &opts);
        }));
        assert!(result.is_ok(), "panic on iteration {} with template {:?}", i, text);
    }
}

#[test]
fn fuzz_random_bytes_no_panics() {
    let mut rng = Rng(12345);
    for _ in 0..5000 {
        let len = rng.below(60);
        let s: String = (0..len)
            .map(|_| {
                let pool = "<%=>-#{}[]()|&.:'\"\\ \n0123456789abcxyzAZ_?!@$,;+*/^~";
                pool.chars().nth(rng.below(pool.chars().count())).unwrap_or(' ')
            })
            .collect();
        let r = std::panic::catch_unwind(|| {
            let _ = render_with(
                &s,
                &Vars::new(),
                &Options {
                    max_steps: Some(5_000),
                    ..Default::default()
                },
            );
        });
        assert!(r.is_ok(), "panic with {:?}", s);
    }
}
