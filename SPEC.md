# ERBLite Language Specification (v0.2)

ERBLite is a template language that uses Ruby's ERB syntax and runs a **small,
sandboxed subset of Ruby**. It has native implementations in Ruby, Python and
Rust. None of them evals host-language code, and none needs Ruby installed.

**Main rule: be Ruby-compatible.** If an ERBLite template renders without error,
it must give byte-for-byte the same output as Ruby 3.4
`ERB.new(src, trim_mode: "-").result_with_hash(vars)` (with `json` loaded).
The only exceptions are the features marked **[ERBLite]** below. If this spec
and Ruby disagree on a feature not marked **[ERBLite]**, Ruby is correct and
this spec has a bug.

The shared conformance suite in `conformance/` decides what each
implementation must do. Most of its expected outputs come from running real
Ruby (`conformance/generate.rb`).

### 0. Security model (main requirement)

* **No implementation may run host-language code taken from a template.** That
  rules out `eval`, `instance_eval`, `exec`, `compile`, `Function`, `__import__`,
  `getattr`/`send`/`public_send` with template-controlled names, and every
  other form of dynamic dispatch or code loading.
* Templates are tokenized, parsed into an AST, and run by a tree-walking
  interpreter. A method call on a value is looked up by type and method name
  in a **fixed allowlist** that the implementation writes out explicitly. A
  name that is not in that allowlist raises `NoMethodError`.
* Templates cannot reach the host environment: no `require`, `ENV`, system
  calls, sockets, process control, globals, `Object`/`Kernel` reflection,
  `instance_variable_get`, `method`, `define_method`, or constants other than
  those in §9.
* The only I/O is reading files inside the sandbox (§9.1).
* Untrusted templates can stop runaway loops with `max_steps` (§12).

---

## 1. Template syntax

| Tag            | Meaning |
|----------------|---------|
| `<% code %>`   | Runs code and outputs nothing. |
| `<%= expr %>`  | Evaluates `expr` and appends `expr.to_s` to the output. No HTML escaping. |
| `<%# text %>`  | Comment. Ignored. |
| `<%%`          | In text, outputs a literal `<%`. |
| `<%- …`        | Same as `<%`. Also removes the spaces and tabs before the tag, but only when nothing else comes before the tag on its line. |
| `… -%>`        | Closes any tag. If a `\n` or `\r\n` comes right after it, that line break is removed. |

* A tag always ends at the first `%>`, even when the `%>` is inside a string
  literal (the same as ERB).
* `<%=` and `<%#` cannot be combined with the leading `-`. Trimming is always on.
  It is the same as Ruby `trim_mode: "-"`.
* The code in all the tags of a template forms **one program**. A statement can
  span several tags, and text between tags becomes an output statement in the
  enclosing block:

  ```erb
  <% items.each do |item| -%>
    - <%= item %>
  <% end -%>
  ```
* The end of a `<% %>` tag works like a newline (statement terminator).
* A `<%= %>` tag holds one or more statements (separated by `;` or newlines)
  and outputs the value of the last one. A block opened inside a `<%= %>` tag
  cannot continue into a later tag.
* An unclosed tag is a `SyntaxError`.

## 2. Lexical structure (inside tags)

* **Comments**: `#` to the end of the line or the end of the tag. **[ERBLite]** In Ruby ERB, a `#` comment in a
  `<% %>` tag also hides the template text that follows the tag on the same generated line (a known ERB pitfall). ERBLite always ends the comment at the tag.
* **Newlines** end statements. A newline does *not* end a statement when it
  comes after a binary operator, `,`, `(`, `[`, `{`, `|`, `.`, `&.`, `=` or `?`
  / `:` in a ternary, or before a line that starts with `.` or `&.`.
* **Identifiers**: `[a-z_][A-Za-z0-9_]*`. A method name can end in `?` or `!`.
  The `?` or `!` is part of the identifier when it directly follows the
  name and is not followed by `=` (`x.nil?`, `a != b`).
* **Constants**: `[A-Z][A-Za-z0-9_]*`.
* **Keywords**: `if elsif else unless then end while until for in do case when
  break next nil true false and or not begin self`. `begin` and `self` are
  reserved but not supported (`SyntaxError`).
* **Integers**: `123`, `1_000`, `0x1F`, `0b1010`, `0o17`, `017` (octal).
* **Floats**: `1.5`, `1e10`, `1.5e-3`, `1_000.5`. A digit must come before `.`
  (`.5` is not a float).
* A `-` directly before a numeric literal makes a negative literal when the
  previous token does not end a value. A token ends a value if it is a literal,
  identifier, constant, `)`, `]`, `}`, `nil`, `true`, `false` or `end`. So
  `-2.abs` is `2`, and `-2 ** 2` is `-4` (as in Ruby, `**` binds before the
  literal's sign).
* **Strings**:
  * `'single'`: only `\\` and `\'` are escapes. Any other backslash is kept
    as written.
  * `"double"`: escapes are `\n \t \r \0 \s \e \a \b \f \v \\ \" \#`,
    `\uXXXX`, `\u{X…}`, `\xHH`. Any other `\c` gives `c`. Interpolation is
    `#{expr}`, and it inserts `expr.to_s`.
  * `%w[a b c]` and `%i[a b c]` work with the delimiters `[] () {} <>`.
* **Symbols**: `:name`, `:name?`, `:name!`, `:name=`, `:"quoted"`. In a hash
  literal or an argument list, `name: value` means the key `:name`.

## 3. Grammar

```
program    := statements
statements := { statement terminator }            terminator: newline | ';' | tag end
statement  := TEXT | '<%=' expr '%>'
            | simple { ('if'|'unless'|'while'|'until') expr }   # modifiers, left to right
simple     := 'break' [expr] | 'next' [expr]
            | mlhs '=' rhs_list                    # a, b = 1, 2   /   a, b = pair
            | expr
expr       := not_expr { ('and'|'or') not_expr }   # same precedence, left-assoc
not_expr   := 'not' not_expr | assign
assign     := target ('='|'+='|'-='|'*='|'/='|'%='|'**='|'||='|'&&='|'<<='|'|='|'&=') assign
            | ternary
target     := IDENT | postfix '[' args ']' 
ternary    := range [ '?' ternary ':' ternary ]
range      := oror [ ('..'|'...') oror ]
oror       := andand { '||' andand }
andand     := not { '&&' not }
equality   := compare [ ('=='|'!='|'<=>'|'===') compare ]      # non-associative
compare    := bor { ('<'|'<='|'>'|'>=') bor }
bor        := band { ('|'|'^') band }
band       := shift { '&' shift }
shift      := add { ('<<'|'>>') add }
add        := mul { ('+'|'-') mul }
mul        := uminus { ('*'|'/'|'%') uminus }
uminus     := '-' uminus | pow
pow        := unary [ '**' uminus ]                 # right-assoc
unary      := '!' unary | '~' unary | '+' unary | postfix   # '!' binds tighter than any binary operator
postfix    := primary { '.' NAME [call_args] [block]
                      | '&.' NAME [call_args] [block]
                      | '::' CONST
                      | '[' args ']' }
primary    := literal | array | hash | '(' statements ')'
            | IDENT [call_args] [block] | CONST [call_args]
            | if | unless | while | until | for | case
call_args  := '(' args ')'                          # no space before '('
            | command_args                          # see §3.1
args       := [ arg { ',' arg } [','] ]
arg        := expr | NAME ':' expr | expr '=>' expr | '&' SYMBOL
block      := '{' [params] statements '}' | 'do' [params] statements 'end'
params     := '|' param { ',' param } '|'
param      := IDENT | '(' param { ',' param } ')'
if         := 'if' expr then statements { 'elsif' expr then statements } [ 'else' statements ] 'end'
unless     := 'unless' expr then statements [ 'else' statements ] 'end'
while      := ('while'|'until') expr ['do'] terminator statements 'end'
for        := 'for' IDENT {',' IDENT} 'in' expr ['do'] statements 'end'
case       := 'case' [expr] { 'when' expr {',' expr} then statements } [ 'else' statements ] 'end'
then       := terminator | 'then' | terminator 'then'
```

Notes:
* `!=` is `!(a == b)`. `===` exists only so that `case` can use it.
* In a `while`, `until` or `for` header, `do` belongs to the loop. It is not a
  block for the condition.
* A `{` right after a method call is a block. Anywhere else, `{` starts a hash
  literal.
* Arguments written as `key: value` or `k => v` at the end of an argument list
  are collected into one Hash. That Hash is passed as the last argument.
* `&:sym` passes a block that calls method `sym` on its first argument
  (`map(&:upcase)`).
* `&.` returns `nil` when the receiver is `nil`. Otherwise it calls the method
  normally.

### 3.1 Calls without parentheses

A method call with no parentheses can take arguments (`render "x", a: 1`,
`list.include? item`) when **all** of these are true:
1. The callee is a method call. That means `recv.name`, or a bare identifier that
   has not yet been assigned as a local variable when the parser reaches it.
2. The next token is on the same line and has whitespace before it.
3. The next token is one of: an integer, float, string, symbol, label (`a:`),
   identifier (not a keyword), constant, `nil`, `true` or `false`.

The arguments run to the end of the line, the end of the tag, a closing
bracket, or a modifier keyword (`if`, `unless`, `while`, `until`, `do`, `then`,
`and`, `or`). Arguments without parentheses cannot contain another call
without parentheses.

## 4. Values

| Type        | Notes |
|-------------|-------|
| `nil`       | |
| `true`/`false` | Only `nil` and `false` are falsy. |
| `Integer`   | **[ERBLite]** Signed 64-bit. Any result outside `[-2^63, 2^63-1]` raises `RangeError` ("integer overflow"). Ruby would promote it to a Bignum. |
| `Float`     | IEEE-754 double. |
| `String`    | Unicode text. Indexing and length count code points. |
| `Symbol`    | Interned name. Not equal to the String with the same name. |
| `Array`     | Mutable, ordered, shared by reference. |
| `Hash`      | Mutable, keeps insertion order, shared by reference, has an optional default value. Keys compare with Ruby `eql?`, so `1` and `1.0` are different keys. Strings are copied when used as keys. |
| `Range`     | `a..b` / `a...b`. Integer endpoints are needed to iterate. Float endpoints are allowed for `include?`/`===`/`cover?`. |
| `Class`     | Only the built-in constants listed in §9. |
| `Enumerator`| What an iterator method returns when called without a block (§8.7). |

**Strings act as values [ERBLite].** No method changes a string in place, and
none can. `s << x` where `s` is a String is treated as `s = s + x.to_s`, but only
when the left side can be assigned to (a variable, `a[i]`, …). Otherwise it
returns the new string. In real Ruby, `<<` changes the string in place, so the
two can differ only when one String object is reachable through two names.

**Equality (`==`)**: Numbers are equal across types (`1 == 1.0`). Arrays are
equal when their elements are equal one by one. Hashes are equal when they have
the same keys (`eql?`) and equal values (`==`). Ranges are equal when their
endpoints and exclusivity match. Strings are equal by value. Symbols are equal
by name. Other values are equal when they are the same object or the same
constant.

**Comparison (`<=>`)**: Numbers compare with numbers. Strings compare with
strings, by code point. Arrays compare element by element. All other pairs give
`nil`. `<`, `<=`, `>` and `>=` raise `ArgumentError`
(`comparison of X with Y failed`) when `<=>` gives `nil`. `sort`, `min` and
`max` raise the same error.

## 5. Variables and scope

* The host's variables are the template's starting local variables.
* Assigning to a name declares a local variable in the current scope, unless
  an enclosing scope already declares that name. The parser decides which
  scope a name belongs to as it reads the template, the same way Ruby does.
* A name that the parser has seen assigned, but whose assignment has not run
  yet, evaluates to `nil` (as in Ruby).
* A bare identifier that is not a known local is looked up in this order:
  host variable, then built-in function (§9). If neither exists, it raises
  `NameError`.
* Blocks (`{ }` / `do end`) open a new scope. Block parameters always shadow
  outer variables. Variables first assigned inside a block are local to that
  block.
* `if`, `while`, `until`, `for` and `case` do **not** open a scope. A `for` loop
  variable stays visible after the loop.
* Partials (`render`) get a new top-level scope. Its host variables are only
  the locals passed to `render`.

### 5.1 Blocks and yield

* When a method yields **one** value and the block has **more than one**
  parameter, an Array value is spread across the parameters (auto-splat).
  Missing parameters are `nil`. Extra values are dropped.
* A parameter written as `(a, b)` destructures one yielded Array.
* `next [v]` ends the block call. The block returns `v`, or `nil` when there
  is no value.
* `break [v]` ends the method that called the block (or the innermost
  `while`/`until`/`for`/`loop`). That method then returns `v` (default `nil`).
* `break` or `next` outside a loop or block is a `SyntaxError`.

## 6. Operators

| Operator | Integer | Float | Other |
|----------|---------|-------|-------|
| `+ - *`  | Integer, or Float if either side is a Float. Overflow raises `RangeError`. | IEEE | `String + String`, `String * Integer`, `Array + Array`, `Array - Array`, `Array * Integer`, `Array * String` (= `join`) |
| `/`      | Floor division. Dividing by 0 raises `ZeroDivisionError`. | IEEE (`1.0/0` is `Infinity`) | |
| `%`      | Floor modulo (sign follows the divisor). Modulo by 0 raises `ZeroDivisionError`. | `fmod`, then the divisor is added if the signs differ | `String % arg_or_array` (= `format`) |
| `**`     | Integer exponent ≥ 0 gives an Integer. **[ERBLite]** A negative exponent raises `RangeError` (Ruby returns a Rational). | `pow` | |
| `& \| ^ ~ << >>` | Bitwise. A negative shift reverses direction. `<<` overflow raises `RangeError`. | | `true & x`, `false \| x`, `Array & Array` (intersection), `Array \| Array` (union), `Array << v` (push in place, returns the array) |
| `<=> < <= > >= == !=` | see §4 | | |
| `! not && \|\| and or` | Short-circuit. `&&`/`||` return an operand, not a boolean. | | |
| `.. ...` | Range | | |
| `a ? b : c` | | | |

Compound assignment `a op= b` is `a = a op b`. `a ||= b` is `a || (a = b)`, and
`&&=` works the same way. `h[k] += 1` evaluates `h` and `k` once.

## 7. String conversion

### 7.1 `to_s`
* `nil` → `""`. `true` → `"true"`. `false` → `"false"`. An Integer prints in
  decimal. A Symbol prints its name. `Range` prints `"#{a.to_s}..#{b.to_s}"`
  (or `...`).
* An Array or Hash prints the same as `inspect`. A Class prints its name.
* **Float** (same as Ruby 3.4's `Float#to_s`):
  * Special values print as `Infinity`, `-Infinity` and `NaN`. `-0.0` prints
    as `-0.0`.
  * Otherwise, take the shortest round-trip decimal digits `D` (`n = len(D)`)
    and decimal exponent `e`, so that value = `0.D × 10^e`. Then:
    * if `0 < e ≤ 16` and `n > e`, print `D[0,e] + "." + D[e..]`
    * else if `0 < e ≤ 15`, print `D + "0"*(e-n) + ".0"`
    * else if `-4 < e ≤ 0`, print `"0." + "0"*(-e) + D`
    * else print `D[0] + "." + (D[1..] or "0") + "e" + sign + exponent`. The
      exponent is `e-1`, written with a sign and at least 2 digits
      (`1.0e+16`, `1.5e-05`).

### 7.2 `inspect`
* A String is wrapped in `"` with these escapes: `\"`, `\\`, `\n`, `\t`, `\r`,
  `\e`, `\a`, `\b`, `\f`, `\v`. `#{`, `#$` and `#@` become `\#{`, `\#$` and
  `\#@`. Any other character below U+0020, and U+007F, becomes `\uXXXX`
  (4 uppercase hex digits), except U+0000, which becomes `\u0000`.
  All other characters are written unchanged.
* A Symbol prints as `:name` when the name matches
  `/\A[A-Za-z_][A-Za-z0-9_]*[?!=]?\z/` or is an operator name
  (`+ - * / % ** == != < > <= >= <=> ! [] []= << >> & | ^ ~ +@ -@ ===`).
  Otherwise it prints as `:"…"`, escaped like a String.
* Array → `[a, b]`. Hash → `{}` when empty. Otherwise `{k => v, sym: v}`:
  a key that is a Symbol which prints as `:name` is written `name: v`; any
  other Symbol key is written `"quoted": v`; any other key is written
  `k.inspect => v.inspect`.
* A Range prints as `a.inspect..b.inspect`. `nil` prints as `nil`. Numbers
  print the same as `to_s`.
* An Enumerator prints as `#<Enumerator: RECV.inspect:METHOD>`, or
  `#<Enumerator: RECV.inspect:METHOD(ARG, …)>` when it has arguments, with
  each ARG printed by `inspect`.

## 8. Built-in methods

If a method is not listed here, calling it raises `NoMethodError`. If an
argument has the wrong type, the call raises `TypeError`, unless a different
error is stated. If the number of arguments is wrong, the call raises
`ArgumentError`. "Enumerable" means the shared method set in §8.8.

### 8.1 All values
`==`, `!=`, `!`, `nil?`, `to_s`, `inspect`, `class`, `is_a?(C)`,
`kind_of?(C)`, `instance_of?(C)`, `respond_to?(sym_or_str)`, `dup`, `freeze`,
`frozen?` (both follow Ruby; see §14), `eql?`, `===`, `<=>`, `to_json`,
`then { |x| }`, `tap { |x| }`.

### 8.2 nil / booleans
nil: `to_a` → `[]`, `to_i` → `0`, `to_f` → `0.0`, `to_h` → `{}`.
true/false: `&`, `|`, `^`.

### 8.3 Integer
`+ - * / % ** -@ ~ & | ^ << >> <=> < <= > >= ==`, `abs`, `succ`/`next`, `pred`,
`to_i`, `to_f`, `to_s(base=10)` (base 2–36), `chr` (0–255, else `RangeError`),
`ord`, `times`, `upto(n)`, `downto(n)`, `step(limit, step=1)`, `even?`,
`odd?`, `zero?`, `positive?`, `negative?`, `between?(a,b)`, `clamp(a,b)`,
`divmod(n)`, `fdiv(n)`, `gcd(n)`, `lcm(n)`, `digits`, `bit_length`,
`floor`/`ceil`/`truncate` (no args; return self), `round(ndigits=0)` (with
`ndigits < 0`, round half away from zero to a multiple of `10^-ndigits`;
otherwise return self).

### 8.4 Float
`+ - * / % ** -@ <=> < <= > >= ==`, `abs`, `to_f`, `to_s`, `nan?`, `finite?`,
`infinite?` (`1`, `-1` or `nil`), `zero?`, `positive?`, `negative?`,
`between?`, `clamp`, `divmod`, `fdiv`.
* `to_i`/`truncate`: round toward zero. NaN/Infinity → `FloatDomainError`. Out of
  range → `RangeError`.
* `floor`, `ceil`: return an Integer.
* `round(ndigits=0)`: Ruby's algorithm.
  * `ndigits == 0`: round half away from zero, return an Integer.
  * `ndigits < 0`: `self.to_i.round(ndigits)`.
  * `ndigits > 0`: return a Float. If `self == 0`, return `self`. Let
    `(_, binexp) = frexp(self)`. If `ndigits >= 17 - (binexp > 0 ? binexp/4 : binexp/3 - 1)`
    (C integer division, truncating), return `self` unchanged. If
    `ndigits > 14`, use exact rational rounding (implementations may raise
    `ArgumentError` for `ndigits > 14` **[ERBLite]**). Otherwise
    `s = 10.0**ndigits`, `x = round_half_up(self, s)`, and return `x / s`, where:
    ```
    round_half_up(x, s): xs = x*s; f = round(xs)   # C round(): half away from 0
                         if x > 0 and (f + 0.5)/s <= x: f += 1
                         if x < 0 and (f - 0.5)/s >= x: f -= 1
                         return f
    ```

### 8.5 String
`+`, `*`, `%`, `==`, `<=>`, `< <= > >=`,
`[]` (`i`, `start, len`, range; out of range → `nil`), `length`/`size`,
`bytesize`, `empty?`, `upcase`, `downcase`, `capitalize`, `swapcase`, `strip`,
`lstrip`, `rstrip`, `chomp(suffix=nil)`, `chop`, `chars`, `lines`, `bytes`,
`reverse`, `include?(s)`, `start_with?(s, …)`, `end_with?(s, …)`,
`index(s, start=0)`, `rindex(s)`, `count(s)` (counts non-overlapping occurrences
of the substring **[ERBLite]**; Ruby counts characters from a character set,
so the two agree when `s` is one character), `sub(pat, rep)`, `gsub(pat, rep)`,
`sub(pat) { |m| }`, `gsub(pat) { |m| }`, `gsub(pat, hash)`, `tr(from, to)`,
`delete(s)`, `squeeze`, `center(w, pad=" ")`, `ljust`, `rjust`, `to_i(base=10)`,
`to_f`, `to_s`, `to_str`, `to_sym`, `ord`, `hex`, `oct`, `each_char`,
`each_line`, `split(sep=nil, limit=0)`, `partition(s)`, `rpartition(s)`,
`casecmp(s)`, `casecmp?(s)`, `between?`, `ascii_only?`, `slice` (same as `[]`).
There is no regex support anywhere (`=~`, `match` and Regexp literals do not exist).

* Whitespace for `strip`, `lstrip`, `rstrip` and `split(nil)` is
  `" \t\n\v\f\r"` plus `\0`.
* `split(nil)` or `split(" ")`: splits on runs of whitespace and drops leading
  empty fields. `split("")`: splits into characters. `split(s)`: splits on the
  literal `s`. When `limit == 0`, trailing empty strings are removed. When
  `limit > 0`, the result has at most `limit` fields. When `limit < 0`, the
  trailing empty strings are kept.
* `sub`/`gsub` patterns are **literal strings** **[ERBLite: no Regexp]**. In the
  replacement string, `\0` and `\&` insert the match, `` \` `` inserts the text
  before the match, `\'` inserts the text after the match, `\\` inserts a
  backslash, and `\1`–`\9` insert nothing. Any other backslash is kept as written.
* `tr`/`delete` character sets accept `a-z` ranges and a leading `^`
  (negation). With `tr`, if `to` is shorter than `from`, the last character of
  `to` is repeated.
* `to_i`: skips leading whitespace, then reads an optional sign and the longest
  valid run of digits (single `_` allowed between digits). With base 16 a
  `0x` prefix is allowed, with base 2 `0b`, with base 8 `0o`/`0`. No digits →
  `0`. `to_f` reads `[sign]digits[.digits][e[sign]digits]` the same way, and also
  accepts a leading `.digits`. `hex` = `to_i(16)`. `oct` = `to_i(8)`.

### 8.6 Symbol
`to_s`/`id2name`/`name`, `to_sym`,
`length`/`size`, `upcase`, `downcase`, `capitalize` (each returns a Symbol),
`<=>`, `==`, `start_with?`, `end_with?`, `empty?`.

### 8.7 Array
Literal syntax, `[]`/`slice` (`i`, `start, len`, range), `[]=` (`i`, or
`start, len`; writing past the end fills the gap with `nil`), `+`, `-`, `*`,
`&`, `|`, `<<`, `==`, `<=>`, `length`/`size`, `empty?`, `first(n=nil)`,
`last(n=nil)`, `push`/`append(*v)`, `pop`, `shift`, `unshift`/`prepend(*v)`,
`insert(i, *v)`, `concat(*arrays)`, `delete(v)`, `delete_at(i)`,
`delete_if {}`, `clear`, `fill(v)`, `index(v)`/`find_index(v)` (or a block),
`rindex(v)`, `include?`, `join(sep="")` (nested arrays are joined
recursively; elements use `to_s`), `reverse`, `rotate(n=1)`, `sort`,
`sort { |a,b| }`, `sort_by`, `uniq` (also with a block), `flatten(depth=nil)`,
`compact`, `transpose`, `values_at(*i)`,
`dig(*keys)`, `assoc(k)`, `product(*a)`, `combination(n)`, `permutation(n)`
(the last three need a block or return an Array **[ERBLite]**), `to_a`,
`to_h` (from pairs, or with a block), `each`,
`each_index`, `reverse_each`, plus Enumerable.

* `sum(init=0)`: if every element is numeric, integers add exactly. Once a
  Float appears, it switches to Kahan-Babuska summation (the same as Ruby
  `ary_sum`). Otherwise it adds with `+`.
* Methods that change the array (`<<`, `push`, `pop`, `[]=`, `delete`, …) do so in
  place.

**Enumerator** (from calling `each`, `map`, `each_with_index`, `select`,
`each_slice(n)`, `each_cons(n)`, `times`, `upto`, `downto`, `step`, `each_char`,
`reject`, `filter_map`, `flat_map` or `each_pair` without a block):
* `with_index(offset=0) { |x, i| }` / `each_with_index { |x, i| }`: calls the
  original method with a block that passes `(yielded, index)` to the user's block.
  It returns whatever the original method returns. For example,
  `map.with_index { … }` returns the mapped Array.
* `with_object(o)`: similar to `with_index`, but returns `o`.
* Every other method works on `to_a` (all yielded values, collected) through
  Array/Enumerable. `size` gives the count.

### 8.8 Enumerable (Array, Hash, Range, Enumerator)
`each_with_index`, `each_with_object(o)`, `each_slice(n)`, `each_cons(n)`,
`map`/`collect`, `flat_map`/`collect_concat`, `filter_map`,
`select`/`filter`, `reject`, `find`/`detect`, `find_index`, `find_all`,
`any?`, `all?`, `none?`, `one?` (each takes an optional argument that is
compared with `===`), `count(v=nil)`, `include?`/`member?`,
`min`/`max(n=nil)`, `min_by`/`max_by`, `minmax`, `sum`, `inject`/`reduce`
(forms: `(init) { }`, `{ }`, `(:sym)`, `(init, :sym)`), `group_by`,
`partition`, `chunk_while`, `slice_when`, `tally`, `zip(*others)`, `take(n)`,
`drop(n)`, `take_while`, `drop_while`, `first(n)`, `sort`, `sort_by`, `uniq`,
`to_a`/`entries`, `to_h`.

### 8.9 Hash
`Hash.new(default=nil)`, `[]` (a missing key gives the default value),
`[]=`, `store`, `fetch(k, default)` and `fetch(k) { |k| }` (a missing key
without a default raises `KeyError`), `dig(*keys)`,
`key?`/`has_key?`/`include?`/`member?`, `value?`/`has_value?`, `key(v)`,
`keys`, `values`, `values_at`, `fetch_values`, `length`/`size`, `empty?`,
`delete(k)`, `delete_if`, `clear`, `merge(*others)`
(also with a block `{ |k, old, new| }`), `merge!`/`update`, `to_a`, `to_h`
(also with a block), `invert`, `transform_values`, `transform_keys`,
`select`/`filter`, `reject`, `filter_map`, `min_by`, `max_by`, `sort_by`,
`group_by`, `partition`, `find`/`detect`, `any?`, `all?`, `none?`, `count`,
`sum`, `sort`, `each`/`each_pair`, `each_key`, `each_value`, `each_with_index`,
`each_with_object`, `inject`/`reduce`, `map`/`collect`, `flat_map`, `slice(*keys)`,
`except(*keys)`, `compact`, `default`, `default=`, `first(n=nil)`, `sum`,
`uniq`, `zip`, `tally`, `min_by`, `max_by`, `to_json`.

* `select`, `filter`, `reject`, `filter_map`, `delete_if`, `transform_*`,
  `each_key` and `each_value` yield `k, v` as two values. Every other iterator
  yields one `[k, v]` Array, which the block can auto-splat. `select`,
  `filter` and `reject` return a Hash.

### 8.10 Range
`first(n=nil)`/`begin`, `last(n=nil)`/`end`, `min`, `max`, `size`/`count`
(Integer ranges), `sum` (Integer ranges use the closed formula), `to_a`/`entries`,
`each`, `reverse_each`, `step(n)`, `include?`/`member?`/
`cover?`/`===`, `exclude_end?`, `to_s`, `inspect`, plus Enumerable (through `to_a`).

### 8.11 Class values
`name`, `to_s`, `inspect`, `==`, `===` (checks `is_a?`). Class hierarchy:
`Integer < Numeric`, `Float < Numeric`. Every class is `< Object`. `Comparable` and
`Enumerable` exist as module constants for `is_a?`. Comparable: Integer, Float,
String. Enumerable: Array, Hash, Range, Enumerator.

## 9. Built-in functions and constants

| Name | Behavior |
|------|----------|
| `format(fmt, *args)` / `sprintf` | §10 |
| `Integer(x, base=nil)` | Strict conversion. A String must be a valid literal: optional sign, optional `0x`/`0b`/`0o`/`0` prefix, `_` only between digits, surrounding whitespace allowed. Otherwise `ArgumentError`. A Float is truncated. `nil` → `TypeError`. |
| `Float(x)` | Strict conversion. A String must be a valid decimal float or integer (`_` allowed between digits). Otherwise `ArgumentError`. `nil` → `TypeError`. |
| `String(x)` | `x.to_s` |
| `Array(x)` | `nil` → `[]`. An Array stays as is. A Hash → `to_a`. Anything else → `[x]`. |
| `raise(msg)` | Raises `RuntimeError` with `msg`. |
| `loop { }` | Repeats until `break`. Returns the break value. |
| `render(path, locals = {})` | **[ERBLite]** Reads `path` (the same sandbox as `File.read`), renders it as a template with `locals` (Symbol or String keys) as its host variables, and returns the result String. Nesting deeper than 32 raises `LimitError`. |
| `File.read(path)` | **[ERBLite sandbox]** Returns the file's contents as a String (UTF-8). |
| `File.exist?(path)` / `File.file?` / `File.directory?` | Sandboxed. Returns `false` when the path is outside the sandbox. |
| `File.basename(p, suffix=nil)`, `File.dirname(p)`, `File.extname(p)`, `File.join(*parts)` | Pure string functions with Ruby semantics. They work even without a sandbox. |
| `Dir.glob(pattern)` | Sandboxed. Returns sorted paths relative to the base dir. Supports `*`, `**` and `?`. |
| `JSON.parse(str, symbolize_names: false)` | JSON objects → Hash, arrays → Array, integers → Integer (out of i64 range → `RangeError`), numbers with `.` or an exponent → Float. Invalid JSON → `JSON::ParserError`. |
| `JSON.generate(obj)` / `obj.to_json` | Compact output (`{"a":1,"b":[1,2]}`). Symbols and other keys are converted with `to_s`. **[ERBLite]** Floats use `Float#to_s` (`1.0e+20`). This matches Ruby 3.4's bundled json 2.9.1; newer json gems print floats differently. NaN/Infinity → `JSON::GeneratorError`. Strings escape `"`, `\`, and control characters (`\b \f \n \r \t`, others `\u00XX` lowercase hex). Non-ASCII and `/` are written unchanged. |
| `JSON.pretty_generate(obj)` | 2-space indent, `"key": value`, one element per line. Empty `[]`/`{}` are written inline. |
| `Math.sqrt cbrt sin cos tan asin acos atan atan2 exp log(x, base=nil) log2 log10 hypot`, `Math::PI`, `Math::E` | Always return a Float. A domain error raises `Math::DomainError`. |
| `Float::INFINITY`, `Float::NAN`, `Float::EPSILON`, `Float::MAX`, `Float::MIN` | |
| `Integer Float String Symbol Array Hash Range NilClass TrueClass FalseClass Numeric Object Comparable Enumerable Enumerator` | Class constants. `Array.new(n=0, v=nil)`, `Array.new(n) { \|i\| }`, `Hash.new(default=nil)`. |

### 9.1 File sandbox [ERBLite]
* File access works only when the host passes a `base_dir`. With no
  `base_dir`, every sandboxed `File`/`Dir`/`render` call raises `SecurityError`.
* A relative path is resolved against `base_dir`. A path that is absolute or
  contains `..` is allowed only if, after normalizing and resolving symlinks,
  it is still inside the real path of `base_dir`. Otherwise the call raises
  `SecurityError`.
* A missing file raises `IOError`.

## 10. `format` / `String#%`

`%[flags][width][.precision]type`, where flags are `-`, `+`, space, `0` and
`#`. Width or precision can be `*` (taken from the arguments). Named
references `%<name>d` and `%{name}` take values from a Hash argument. Types:

| Type | Meaning |
|------|---------|
| `d i u` | Integer. A Float argument is truncated toward zero (`format("%d", -3.99)` → `-3`). A String argument goes through `Integer()`. |
| `f` | Fixed point. Precision defaults to 6. Uses correct rounding (ties to even on the exact binary value, the same as C printf). |
| `e E` | `d.dddddde+XX` (exponent has at least 2 digits) |
| `g G` | C `%g` rules. Trailing zeros are removed unless `#` is given. |
| `x X o b B` | Lowercase hex, uppercase hex, octal, binary. `#` adds a `0x`/`0X`/`0`/`0b`/`0B` prefix. **[ERBLite]** A negative argument without the `+` or space flag raises `ArgumentError` (Ruby prints two's-complement `..f`). With `+` or space, prints `-ff`. |
| `s` | `to_s`. Precision cuts the string. |
| `p` | `inspect` |
| `c` | An Integer code point, or the first character of a String |
| `%` | A literal `%` |

Too few arguments → `ArgumentError`. Unknown type → `ArgumentError`.
`Float` NaN/Inf with `f`/`e`/`g` prints `NaN`/`Inf`/`-Inf`.

## 11. Errors

Every error has a **type** name, a **message**, the template **filename**,
and a 1-based **line**. Its display form is `"#{filename}:#{line}: #{type}: #{message}"`.
Type names:

`SyntaxError`, `NameError`, `NoMethodError`, `ArgumentError`, `TypeError`,
`ZeroDivisionError`, `IndexError`, `KeyError`, `RangeError`,
`FloatDomainError`, `RuntimeError`, `Math::DomainError`, `JSON::ParserError`,
`JSON::GeneratorError`, `IOError`, `SecurityError`, `LimitError`
**[ERBLite]**.

Messages are meant for people to read. They should follow Ruby's wording, but
the conformance suite only checks the type and (where given) the line.

## 12. Resource limits [ERBLite]

* `max_steps` (host option, default unlimited): every expression evaluation,
  loop iteration and block call counts as one step. Going over the limit
  raises `LimitError`.
* The `render` nesting limit is 32. Nesting expressions or blocks deeper than
  256 levels may raise `LimitError`.

## 13. Host API (all implementations)

* **Python**: `erblite.render(source, variables=None, *, base_dir=None, filename="(erblite)", max_steps=None) -> str`,
  `erblite.render_file(path, variables=None, *, base_dir=None, ...)` (`base_dir` defaults to the file's directory),
  `erblite.Template(source, filename=...)` with `.render(variables=None, *, base_dir=None, max_steps=None)`,
  exception `erblite.Error` with `.type`, `.message`, `.filename`, `.line`.
  Variables: `dict` → Hash (str keys stay Strings), `list`/`tuple` → Array,
  `int`, `float`, `str`, `bool`, `None`, `erblite.Symbol("x")`.
* **Ruby**: `ERBLite.render(source, vars = {}, base_dir: nil, filename: "(erblite)", max_steps: nil)`,
  `ERBLite.render_file(path, vars = {}, ...)`, `ERBLite::Template.new(source, filename:)`
  with `#render(vars = {}, ...)`, exception `ERBLite::Error` with `#type`, `#message`, `#filename`, `#line`.
  Native Ruby values are used. Hash keys stay as given.
* **Rust**: `erblite::render(source, &Vars) -> Result<String, Error>`,
  `erblite::Template::compile(source, filename)?.render(&vars, &Options)`,
  `erblite::Value` (with `From` impls and `Value::from_json(&str)`),
  `erblite::Error { kind: String, message, filename, line }`. No required dependencies.

## 14. Clarifications (v0.2)

These rules came out of building three implementations side by side, and each
one is pinned by conformance cases (`conformance/src/alignment.yaml`). Where
they conflict with an earlier section, these rules win.

### Syntax
* **Trimming with `<%-`** works like ERB's scanner. Spaces and tabs before
  `<%-` are removed when they start a line, and also when they come right after
  a previous `%>` or `<%%` on the same line: `<%= 1 %>  <%- x %>y` gives `1y`.
* **`<%= %>` holds a statement**, so modifiers are allowed:
  `<%= x if y %>` outputs `""` when `y` is false. An empty `<%= %>` outputs `""`.
* **The value of a text or `<%= %>` statement** is a String copy of the output
  buffer at that moment. So `<% x = if true %>hi<% end %>` sets `x` to `"hi"`.
  **[ERBLite]** In Ruby the value is the live `_erbout` buffer, which keeps
  growing as output is added.
* **Double-quoted strings** support octal escapes `\NNN`. Writing `#@x` or
  `#$x` inside one is a `SyntaxError`.
* **Arguments without parentheses** (§3.1) can also start with `(`, `[`,
  `!`, `&:sym`, or a `-` directly followed by a digit, when there is a space
  before it and the callee is a method. So `a.push -1` is `a.push(-1)` and
  `x.round (2)` is `x.round(2)`. When the callee is a local variable, `x -1`
  is subtraction.
* **Host variable names count as locals that are already declared when parsing
  starts** (the same as `result_with_hash`). An implementation may cache one
  parse for each set of host variable names.
* Also allowed: `a = 1, 2` (assigns an Array); operator method calls
  (`1.+(2)`, `a.[](0)`); and endless or beginless ranges (`s[2..]`, `s[..1]`)
  for indexing.

### Names and calls
* A bare identifier that is not a local, not a host variable and not a
  built-in function raises `NameError`. A call with parentheses or arguments
  to an unknown function raises `NoMethodError`.
* Every value supports `eql?`, `===` and `<=>`.

### Strings, freezing
* `str << Integer` appends that code point. `str << nil` (or any other value
  that is not a String) raises `TypeError`.
* `freeze` is real: `frozen?` follows Ruby. Changing a frozen Array or Hash
  raises `FrozenError`. **[ERBLite]** `s << x` on a frozen String still works
  (rebinding), because strings act as values.
* `String#inspect` also escapes C1 controls (U+0080–U+009F) and U+2028/U+2029
  as `\uXXXX`. `split(nil)` does not treat `\0` as whitespace. `strip` does.
  Symbol and hash-key inspect follow Ruby. Non-ASCII identifiers and `@`/`$`
  names print bare as symbols, but only identifier-like names (optionally
  ending in `?` or `!`) print bare as hash keys.

### Numbers
* `Float#floor(n)` and `Float#ceil(n)` accept a digit count and follow Ruby.
* `Float#round(n)` for `n > 14` must match Ruby exactly: exact rational
  rounding, half away from zero.
* `format` with `%f`/`%e`/`%g` follows Ruby's own BSD `__dtoa`, **not** correctly
  rounded printf: `format('%.1f', 0.35)` is `0.4` and `format('%.2f', 2.675)` is
  `2.68`. Positional arguments `%1$s` are supported. A `%%` written with flags,
  width or precision raises `ArgumentError`.
* Any intermediate result outside the 64-bit range raises `RangeError`, even
  when the final value would fit (`-2 ** 63`).
* `Integer#chr` for 128–255 returns U+0080–U+00FF **[ERBLite]**.

### Hash block arity (same as Ruby)
`select`, `filter`, `reject`, `delete_if`, `each_key` (k), `each_value` (v),
`transform_values` (v) and `transform_keys` (k) yield their listed values.
Everything else, including `filter_map`, `min_by`, `count`, `map` and `each`,
yields one `[k, v]` Array.

### JSON
* `JSON.parse` accepts `//` and `/* */` comments. It rejects trailing commas,
  leading zeros, raw control characters and lone surrogates
  (`JSON::ParserError`). `1e400` parses to `Infinity`.

### Size limits [ERBLite]
* Any String, Array or Hash larger than 10,000,000 elements or characters,
  including a materialized Range, raises `LimitError`. Integer Range
  `sum`/`size`/`include?` use closed forms and never materialize the range.

### Error types
* `FrozenError` is added to §11.
