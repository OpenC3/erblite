// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! Focused behaviour tests. Expected values were checked against Ruby 3.4 ERB.

use erblite::{render, render_with, Options, Template, Value, Vars};

fn r(src: &str) -> String {
    render(src, &Vars::new()).unwrap_or_else(|e| panic!("{}: {}", src, e))
}

fn kind(src: &str) -> String {
    render(src, &Vars::new()).expect_err(src).kind
}

#[test]
fn float_to_s() {
    assert_eq!(
        r("<%= [1e15, 1e16, 1e-4, 1e-5, 1000000000000000.2 + 0.05, 1e23, 5e-324, -0.0, 1.0 / 3].join(' ') %>"),
        "1.0e+15 1.0e+16 0.0001 1.0e-05 1000000000000000.2 1.0e+23 5.0e-324 -0.0 0.3333333333333333"
    );
    // when two shortest candidates tie, Ruby picks the even digit
    assert_eq!(r("<%= 1e15 + 0.3 %>"), "1000000000000000.2");
}

#[test]
fn format_uses_rubys_dtoa() {
    assert_eq!(
        r("<%= format('%.1f %.1f %.1f %.2f %.2f %.3f %.0f %.0f', 0.15, 0.35, 0.45, 2.675, 1.005, 0.0005, 2.5, 3.5) %>"),
        "0.2 0.4 0.4 2.68 1.00 0.001 2 4"
    );
    assert_eq!(
        r("<%= format('%e|%.2e|%g|%g|%#g|%.3g|%10.4g|', 12345.678, 0.000123, 1e-5, 123456.0, 1.0, 3.14159, 123456.0) %>"),
        "1.234568e+04|1.23e-04|1e-05|123456|1.00000|3.14| 1.235e+05|"
    );
    assert_eq!(
        r("<%= format('%2$s %1$s', 'x', 'y') %>|<%= format('%<a>05.1f|%{b}', a: 2.25, b: 1) %>"),
        "y x|002.2|1"
    );
    assert_eq!(kind("<%= format('%5%') %>"), "ArgumentError");
    assert_eq!(kind("<%= format('%x', -1) %>"), "ArgumentError");
    assert_eq!(r("<%= format('%+x', -255) %>"), "-ff");
}

#[test]
fn float_rounding() {
    assert_eq!(
        r("<%= [2.5.round, -2.5.round, 1.005.round(2), 2.675.round(2), 1.15.round(1), 1234.5.round(-2)].inspect %>"),
        "[3, -3, 1.01, 2.68, 1.2, 1200]"
    );
    assert_eq!(
        r("<%= [75.933.ceil(4), 2.675.floor(2), 0.1.round(16), 1.0e-20.round(2)].inspect %>"),
        "[75.9331, 2.67, 0.1, 0.0]"
    );
}

#[test]
fn sum_matches_ruby() {
    assert_eq!(
        r("<%= [0.1, 0.2, 0.3].sum %>,<%= [0.1, 0.2, 0.3].sum(0.0) %>,<%= [1e100, 1.0, -1e100].sum %>"),
        "0.6,0.6000000000000001,1.0"
    );
}

#[test]
fn scoping_and_host_variables() {
    let mut vars = Vars::new();
    vars.insert("total".into(), Value::from(10));
    vars.insert("y".into(), Value::from(10));
    let out = render(
        "<% [1, 2].each { |i| total = total + i } %><%= total %>,<%= y -1 %>",
        &vars,
    )
    .unwrap();
    assert_eq!(out, "13,9");
    assert_eq!(r("<% [1].each { |i| w = i } %><% w = 5 %><%= w %>"), "5");
    assert_eq!(
        r("<% if false %><% q = 0 %><% end %><% [1, 2].each { |i| q = i } %><%= q %>"),
        "2"
    );
    assert_eq!(kind("<%= z %><% z = 1 %>"), "NameError");
    assert_eq!(kind("<%= foo(1) %>"), "NoMethodError");
    // a template is parsed once per set of host variable names
    let t = Template::compile("<%= x -1 %>", "t").unwrap();
    assert_eq!(t.render(&vars_with("x", 5), &Options::default()).unwrap(), "4");
    assert_eq!(
        t.render(&Vars::new(), &Options::default()).unwrap_err().kind,
        "NoMethodError"
    );
}

fn vars_with(k: &str, v: i64) -> Vars {
    let mut vars = Vars::new();
    vars.insert(k.into(), Value::from(v));
    vars
}

#[test]
fn command_calls() {
    assert_eq!(
        r("<% a = [3] %><% a.push -1 %><%= a.inspect %>,<%= [1].include? (1) %>,<%= [true].include? !false %>"),
        "[3, -1],true,true"
    );
    assert_eq!(r("<%= %w[a b].map &:upcase %>"), "[\"A\", \"B\"]");
    assert_eq!(r("<% x = 5 %><%= x -1 %>"), "4");
}

#[test]
fn strings_are_values() {
    assert_eq!(r("<% s = 'a' %><% s << 'b' << 'c' << 100 %><%= s %>"), "abcd");
    assert_eq!(r("<% s = 'a' %><% t = s %><% t << 'b' %><%= s %>,<%= t %>"), "a,ab");
    assert_eq!(kind("<% s = 'a' %><% s << nil %>"), "TypeError");
    assert_eq!(
        r("<% s = 'a'.freeze %><% s << 'b' %><%= s %>,<%= 'x'.frozen? %>,<%= 'x'.freeze.frozen? %>"),
        "ab,false,true"
    );
    assert_eq!(kind("<% a = [1].freeze %><% a << 2 %>"), "FrozenError");
    assert_eq!(kind("<% h = {}.freeze %><% h[:a] = 1 %>"), "FrozenError");
}

#[test]
fn text_statement_value_is_buffer_copy() {
    assert_eq!(r("<% x = if true %>hi<% end %>[<%= x %>]"), "hi[hi]");
    assert_eq!(
        r("<% r = [1, 2].map { %>a<% } %>|<%= r.inspect %>"),
        "aa|[\"a\", \"aa\"]"
    );
}

#[test]
fn trimming() {
    assert_eq!(r("<% [1, 2].each do |i| -%>\n  <%= i %>\n<% end -%>\n"), "  1\n  2\n");
    assert_eq!(r("  <%- x = 1 -%>\nA"), "A");
    assert_eq!(r("B  <%- x = 1 -%>\nA"), "B  A");
    assert_eq!(r("<%= 1 %>  <%- x = 1 %>y"), "1y");
    assert_eq!(r("a<%% b %%>c<%# note -%>\n"), "a<% b %%>c");
}

#[test]
fn inspect_formats() {
    assert_eq!(
        r("<%= {'a' => 1, b: 2, :\"c d\" => 3, :x= => 4, :+ => 5, nil => [1.0, :@iv]}.inspect %>"),
        "{\"a\" => 1, b: 2, \"c d\": 3, \"x=\": 4, \"+\": 5, nil => [1.0, :@iv]}"
    );
    assert_eq!(
        r("<%= \"\\e\\u0001\\u0085 #{1}\\#{x}\".inspect %>"),
        "\"\\e\\u0001\\u0085 1\\#{x}\""
    );
    assert_eq!(
        r("<%= [1].each.inspect %>,<%= 1.step(10, 3).inspect %>"),
        "#<Enumerator: [1]:each>,(1.step(10, 3))"
    );
}

#[test]
fn json() {
    assert_eq!(
        r("<%= JSON.generate({a: [1, 2.5, nil, 1e20], 'b' => \"q\\\"\\n\\u0001\"}) %>"),
        "{\"a\":[1,2.5,null,1.0e+20],\"b\":\"q\\\"\\n\\u0001\"}"
    );
    assert_eq!(
        r("<%= JSON.parse('/* c */ {\"a\": [1, 2.0, -0, \"\\\\u00e9\"]} // x').inspect %>"),
        "{\"a\" => [1, 2.0, 0, \"é\"]}"
    );
    for bad in ["[1,]", "01", "[\"\\\\ud800\"]", "{a: 1}", "[\"\t\"]", ""] {
        assert_eq!(
            kind(&format!("<%= JSON.parse('{}') %>", bad)),
            "JSON::ParserError",
            "{}",
            bad
        );
    }
}

#[test]
fn partials() {
    let opts = Options {
        base_dir: Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/fixtures")),
        ..Default::default()
    };
    let out = render_with(
        "<%= render 'partials/_greet.erb', name: 'Bo' %>|<%= render('partials/_item.erb', 'items' => [1]) %>",
        &Vars::new(),
        &opts,
    )
    .unwrap();
    assert_eq!(out, "Hello Bo!|* 1\n");
}
