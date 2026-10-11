//! String (§8.5) and Symbol (§8.6) methods. Strings are immutable values.

use std::sync::Arc;
use super::conv::{str_oct, str_to_f, str_to_i};
use super::numeric::{between, comparison_failed};
use super::{argc, block_or_enum, check_str_len, object, range, to_int, to_str};
use crate::error::{fail, R, ARGUMENT_ERROR, TYPE_ERROR};
use crate::format::sprintf;
use crate::interp::{BlockArg, Interp};
use crate::value::Value;

const STRIP_WS: &[char] = &[' ', '\t', '\n', '\x0b', '\x0c', '\r', '\0'];

fn s(v: String) -> R<Value> {
    check_str_len(v.len())?;
    Ok(Value::string(v))
}

/// The text `str << v` appends: a String, or an Integer code point.
pub fn shovel_arg(v: &Value) -> R<String> {
    match v {
        Value::Str(s) => Ok(s.to_string()),
        Value::Int(i) => match u32::try_from(*i).ok().and_then(char::from_u32) {
            Some(c) => Ok(c.to_string()),
            None => fail(crate::error::RANGE_ERROR, format!("{} out of char range", i)),
        },
        other => fail(TYPE_ERROR, format!("no implicit conversion of {} into String", other.type_desc())),
    }
}

fn chars_to_string(c: &[char]) -> String {
    c.iter().collect()
}

/// `String#[]` / `slice`.
fn index(st: &str, args: &[Value]) -> R<Value> {
    argc(args, 1, 2)?;
    let chars: Vec<char> = st.chars().collect();
    let n = chars.len() as i64;
    if args.len() == 2 {
        let start = to_int(&args[0])?;
        let len = to_int(&args[1])?;
        if len < 0 {
            return Ok(Value::Nil);
        }
        let st = if start < 0 { start + n } else { start };
        if st < 0 || st > n {
            return Ok(Value::Nil);
        }
        let e = st.saturating_add(len).min(n);
        return Ok(Value::string(chars_to_string(&chars[st as usize..e as usize])));
    }
    match &args[0] {
        Value::Range(r) => match range::beg_len(r, n)? {
            Some((b, l)) => Ok(Value::string(chars_to_string(&chars[b as usize..(b + l) as usize]))),
            None => Ok(Value::Nil),
        },
        Value::Str(sub) => Ok(if st.contains(&**sub) { Value::Str(sub.clone()) } else { Value::Nil }),
        other => {
            let i = to_int(other)?;
            let i = if i < 0 { i + n } else { i };
            if i < 0 || i >= n {
                Ok(Value::Nil)
            } else {
                Ok(Value::string(chars[i as usize].to_string()))
            }
        }
    }
}

struct CharSet {
    neg: bool,
    chars: Vec<char>,
}

impl CharSet {
    fn parse(spec: &str) -> R<CharSet> {
        let raw: Vec<char> = spec.chars().collect();
        let mut i = 0;
        let mut neg = false;
        if raw.len() > 1 && raw[0] == '^' {
            neg = true;
            i = 1;
        }
        Ok(CharSet { neg, chars: expand_set(&raw[i..])? })
    }

    fn contains(&self, c: char) -> bool {
        self.neg != self.chars.contains(&c)
    }
}

fn expand_set(raw: &[char]) -> R<Vec<char>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let mut c = raw[i];
        if c == '\\' && i + 1 < raw.len() {
            i += 1;
            c = raw[i];
        } else if i + 2 < raw.len() && raw[i + 1] == '-' {
            let hi = raw[i + 2];
            if hi < c {
                return fail(
                    ARGUMENT_ERROR,
                    format!("invalid range \"{}-{}\" in string transliteration", c, hi),
                );
            }
            for cp in (c as u32)..=(hi as u32) {
                if let Some(ch) = char::from_u32(cp) {
                    out.push(ch);
                }
                super::check_array_len(out.len())?;
            }
            i += 3;
            continue;
        }
        out.push(c);
        i += 1;
    }
    Ok(out)
}

fn tr(st: &str, from: &str, to: &str) -> R<String> {
    let f = CharSet::parse(from)?;
    let t = expand_set(&to.chars().collect::<Vec<_>>())?;
    let mut out = String::new();
    for c in st.chars() {
        if f.neg {
            if f.contains(c) {
                if let Some(&last) = t.last() {
                    out.push(last);
                }
            } else {
                out.push(c);
            }
        } else if let Some(idx) = f.chars.iter().rposition(|&x| x == c) {
            if let Some(&r) = t.get(idx).or(t.last()) {
                out.push(r);
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

fn sets(args: &[Value]) -> R<Vec<CharSet>> {
    let mut v = Vec::new();
    for a in args {
        v.push(CharSet::parse(&to_str(a)?)?);
    }
    Ok(v)
}

/// Expands `\0 \& \` \' \\ \1-\9` in a sub/gsub replacement string.
fn expand_replacement(rep: &str, src: &str, m: usize, e: usize) -> R<String> {
    let mut out = String::new();
    let chars: Vec<char> = rep.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let n = chars[i + 1];
            match n {
                '0' | '&' => out.push_str(&src[m..e]),
                '`' => out.push_str(&src[..m]),
                '\'' => out.push_str(&src[e..]),
                '\\' => out.push('\\'),
                '1'..='9' => {}
                'k' if chars.get(i + 2) == Some(&'<') => {
                    let name: String = chars[i + 3..].iter().take_while(|c| **c != '>').collect();
                    return fail(crate::error::INDEX_ERROR, format!("undefined group name reference: {}", name));
                }
                other => {
                    out.push('\\');
                    out.push(other);
                }
            }
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    Ok(out)
}

fn sub_gsub(it: &mut Interp, st: &str, args: &[Value], blk: Option<&BlockArg>, global: bool) -> R<Value> {
    argc(args, 1, 2)?;
    let pat = match &args[0] {
        Value::Str(p) => p.clone(),
        other => return fail(TYPE_ERROR, format!("wrong argument type {} (expected Regexp)", other.class_name())),
    };
    if args.len() == 1 && blk.is_none() {
        return fail(ARGUMENT_ERROR, "wrong number of arguments (given 1, expected 2)");
    }
    let mut out = String::new();
    let mut pos = 0usize;
    loop {
        let found = if pat.is_empty() {
            if pos <= st.len() {
                Some(pos)
            } else {
                None
            }
        } else {
            st[pos..].find(&*pat).map(|i| i + pos)
        };
        let Some(m) = found else { break };
        let e = m + pat.len();
        out.push_str(&st[pos..m]);
        let matched = Value::Str(pat.clone());
        let rep = match args.get(1) {
            Some(Value::Str(r)) => expand_replacement(r, st, m, e)?,
            Some(Value::Hash(h)) => {
                let v = {
                    let hb = h.borrow();
                    hb.get(&matched).unwrap_or_else(|| hb.default.clone())
                };
                v.to_s()?
            }
            Some(other) => return fail(TYPE_ERROR, format!("no implicit conversion of {} into String", other.type_desc())),
            None => match blk {
                Some(b) => it.yield1(b, matched)?.to_s()?,
                None => String::new(),
            },
        };
        out.push_str(&rep);
        check_str_len(out.len())?;
        if pat.is_empty() {
            match st[m..].chars().next() {
                Some(c) => {
                    out.push(c);
                    pos = m + c.len_utf8();
                }
                None => {
                    pos = st.len() + 1;
                    break;
                }
            }
        } else {
            pos = e;
        }
        if !global {
            break;
        }
    }
    if pos <= st.len() {
        out.push_str(&st[pos..]);
    }
    s(out)
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

fn split(st: &str, args: &[Value]) -> R<Value> {
    argc(args, 0, 2)?;
    let sep = match args.first() {
        None | Some(Value::Nil) => None,
        Some(Value::Str(x)) => Some(x.clone()),
        Some(other) => return fail(TYPE_ERROR, format!("wrong argument type {} (expected Regexp)", other.class_name())),
    };
    let limit = match args.get(1) {
        Some(v) => to_int(v)?,
        None => 0,
    };
    // Ruby returns the receiver untouched (no whitespace skipping) for limit 1.
    if limit == 1 {
        let parts = if st.is_empty() { vec![] } else { vec![Value::string(st.to_string())] };
        return Ok(Value::array(parts));
    }
    let mut parts: Vec<String> = Vec::new();
    let awk = sep.as_deref().is_none_or(|x| x == " ");
    if awk {
        let chars: Vec<char> = st.chars().collect();
        let mut i = 0;
        while i < chars.len() && is_ws(chars[i]) {
            i += 1;
        }
        while i < chars.len() {
            if limit > 0 && parts.len() as i64 == limit - 1 {
                parts.push(chars[i..].iter().collect());
                break;
            }
            let start = i;
            while i < chars.len() && !is_ws(chars[i]) {
                i += 1;
            }
            parts.push(chars[start..i].iter().collect());
            if i < chars.len() {
                // skip the whitespace run
                while i < chars.len() && is_ws(chars[i]) {
                    i += 1;
                }
                if i == chars.len() && limit != 0 {
                    parts.push(String::new());
                }
            }
        }
    } else if let Some(sep) = sep.as_deref() {
        if sep.is_empty() {
            let chars: Vec<char> = st.chars().collect();
            for (i, c) in chars.iter().enumerate() {
                if limit > 0 && parts.len() as i64 == limit - 1 {
                    parts.push(chars[i..].iter().collect());
                    break;
                }
                parts.push(c.to_string());
            }
        } else {
            let mut rest = st;
            loop {
                if limit > 0 && parts.len() as i64 == limit - 1 {
                    parts.push(rest.to_string());
                    break;
                }
                match rest.find(sep) {
                    Some(i) => {
                        parts.push(rest[..i].to_string());
                        rest = &rest[i + sep.len()..];
                    }
                    None => {
                        parts.push(rest.to_string());
                        break;
                    }
                }
            }
            if st.is_empty() {
                parts.clear();
            }
        }
    }
    if limit == 0 {
        while parts.last().is_some_and(|p| p.is_empty()) {
            parts.pop();
        }
    }
    Ok(Value::array(parts.into_iter().map(Value::string).collect()))
}

fn justify(st: &str, args: &[Value], mode: char) -> R<Value> {
    argc(args, 1, 2)?;
    let w = to_int(&args[0])?;
    let pad: Vec<char> = match args.get(1) {
        Some(p) => to_str(p)?.chars().collect(),
        None => vec![' '],
    };
    if pad.is_empty() {
        return fail(ARGUMENT_ERROR, "zero width padding");
    }
    let n = st.chars().count() as i64;
    if w <= n {
        return Ok(Value::str(st));
    }
    let total = (w - n) as usize;
    check_str_len(total.saturating_mul(4))?;
    let fill = |k: usize| -> String { pad.iter().cycle().take(k).collect() };
    let out = match mode {
        'l' => format!("{}{}", st, fill(total)),
        'r' => format!("{}{}", fill(total), st),
        _ => {
            let left = total / 2;
            format!("{}{}{}", fill(left), st, fill(total - left))
        }
    };
    s(out)
}

fn chomp(st: &str, args: &[Value]) -> R<Value> {
    argc(args, 0, 1)?;
    let out = match args.first() {
        None => default_chomp(st),
        Some(Value::Nil) => st,
        Some(v) => {
            let suf = to_str(v)?;
            if suf.is_empty() {
                let mut t = st;
                loop {
                    if let Some(x) = t.strip_suffix("\r\n") {
                        t = x;
                    } else if let Some(x) = t.strip_suffix('\n') {
                        t = x;
                    } else {
                        break;
                    }
                }
                t
            } else if &*suf == "\n" {
                default_chomp(st)
            } else {
                st.strip_suffix(&*suf).unwrap_or(st)
            }
        }
    };
    Ok(Value::str(out))
}

fn default_chomp(st: &str) -> &str {
    if let Some(x) = st.strip_suffix("\r\n") {
        x
    } else if let Some(x) = st.strip_suffix('\n') {
        x
    } else if let Some(x) = st.strip_suffix('\r') {
        x
    } else {
        st
    }
}

fn lines(st: &str, sep: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if sep.is_empty() {
        out.push(Value::str(st));
        return out;
    }
    let mut rest = st;
    while !rest.is_empty() {
        match rest.find(sep) {
            Some(i) => {
                out.push(Value::str(&rest[..i + sep.len()]));
                rest = &rest[i + sep.len()..];
            }
            None => {
                out.push(Value::str(rest));
                break;
            }
        }
    }
    out
}

fn line_sep(args: &[Value]) -> R<Arc<str>> {
    argc(args, 0, 1)?;
    match args.first() {
        Some(v) => to_str(v),
        None => Ok(Arc::from("\n")),
    }
}

/// Unicode titlecase of a character (differs from uppercase for digraphs and ligatures).
fn titlecase(c: char) -> String {
    match c {
        '\u{01C4}'..='\u{01C6}' => "\u{01C5}".into(),
        '\u{01C7}'..='\u{01C9}' => "\u{01C8}".into(),
        '\u{01CA}'..='\u{01CC}' => "\u{01CB}".into(),
        '\u{01F1}'..='\u{01F3}' => "\u{01F2}".into(),
        'ß' => "Ss".into(),
        'ﬀ' => "Ff".into(),
        'ﬁ' => "Fi".into(),
        'ﬂ' => "Fl".into(),
        'ﬃ' => "Ffi".into(),
        'ﬄ' => "Ffl".into(),
        'ﬅ' | 'ﬆ' => "St".into(),
        other => other.to_uppercase().collect(),
    }
}

fn capitalize(st: &str) -> String {
    let mut cs = st.chars();
    match cs.next() {
        Some(f) => {
            let mut out = titlecase(f);
            out.push_str(&cs.as_str().to_lowercase());
            out
        }
        None => String::new(),
    }
}

fn swapcase(st: &str) -> String {
    let mut out = String::new();
    for c in st.chars() {
        if c.is_uppercase() {
            out.extend(c.to_lowercase());
        } else if c.is_lowercase() {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn char_index_of_byte(st: &str, b: usize) -> i64 {
    st[..b].chars().count() as i64
}

fn byte_of_char(st: &str, ci: usize) -> usize {
    st.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(st.len())
}

fn str_cmp_arg(recv: &Value, arg: &Value) -> R<std::cmp::Ordering> {
    recv.compare(arg).ok_or_else(|| comparison_failed(recv, arg))
}

pub fn call_str(it: &mut Interp, st: &Arc<str>, recv: &Value, name: &str, args: Vec<Value>, blk: Option<&BlockArg>) -> R<Value> {
    match name {
        "+" => {
            argc(&args, 1, 1)?;
            let o = to_str(&args[0])?;
            check_str_len(st.len() + o.len())?;
            s(format!("{}{}", st, o))
        }
        "<<" => {
            argc(&args, 1, 1)?;
            let o = shovel_arg(&args[0])?;
            check_str_len(st.len() + o.len())?;
            s(format!("{}{}", st, o))
        }
        "*" => {
            argc(&args, 1, 1)?;
            let n = to_int(&args[0])?;
            if n < 0 {
                return fail(ARGUMENT_ERROR, "negative argument");
            }
            check_str_len(st.len().saturating_mul(n as usize))?;
            s(st.repeat(n as usize))
        }
        "%" => {
            argc(&args, 1, 1)?;
            let fargs = match &args[0] {
                Value::Array(a) => a.borrow().clone(),
                other => vec![other.clone()],
            };
            s(sprintf(st, &fargs)?)
        }
        "<=>" => {
            argc(&args, 1, 1)?;
            Ok(match &args[0] {
                Value::Str(_) => Value::Int(recv.compare(&args[0]).map(|o| o as i64).unwrap_or(0)),
                _ => Value::Nil,
            })
        }
        "<" | "<=" | ">" | ">=" => {
            argc(&args, 1, 1)?;
            let o = str_cmp_arg(recv, &args[0])?;
            use std::cmp::Ordering::*;
            Ok(Value::Bool(match name {
                "<" => o == Less,
                "<=" => o != Greater,
                ">" => o == Greater,
                _ => o != Less,
            }))
        }
        "[]" | "slice" => index(st, &args),
        "length" | "size" => {
            argc(&args, 0, 0)?;
            Ok(Value::Int(st.chars().count() as i64))
        }
        "bytesize" => {
            argc(&args, 0, 0)?;
            Ok(Value::Int(st.len() as i64))
        }
        "empty?" => {
            argc(&args, 0, 0)?;
            Ok(Value::Bool(st.is_empty()))
        }
        "upcase" | "downcase" | "capitalize" | "swapcase" => {
            argc(&args, 0, 0)?;
            s(match name {
                "upcase" => st.to_uppercase(),
                "downcase" => st.to_lowercase(),
                "capitalize" => capitalize(st),
                _ => swapcase(st),
            })
        }
        "strip" | "lstrip" | "rstrip" => {
            argc(&args, 0, 0)?;
            Ok(Value::str(match name {
                "strip" => st.trim_matches(STRIP_WS),
                "lstrip" => st.trim_start_matches(STRIP_WS),
                _ => st.trim_end_matches(STRIP_WS),
            }))
        }
        "chomp" => chomp(st, &args),
        "chop" => {
            argc(&args, 0, 0)?;
            if let Some(x) = st.strip_suffix("\r\n") {
                return Ok(Value::str(x));
            }
            let mut cs = st.chars();
            cs.next_back();
            Ok(Value::str(cs.as_str()))
        }
        "chars" => {
            argc(&args, 0, 0)?;
            Ok(Value::array(st.chars().map(|c| Value::string(c.to_string())).collect()))
        }
        "bytes" => {
            argc(&args, 0, 0)?;
            Ok(Value::array(st.bytes().map(|b| Value::Int(b as i64)).collect()))
        }
        "lines" => {
            let sep = line_sep(&args)?;
            Ok(Value::array(lines(st, &sep)))
        }
        "each_line" => {
            let sep = line_sep(&args)?;
            let b = block_or_enum!(blk, recv, name, args);
            for l in lines(st, &sep) {
                it.yield1(b, l)?;
            }
            Ok(recv.clone())
        }
        "each_char" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            for c in st.chars() {
                it.yield1(b, Value::string(c.to_string()))?;
            }
            Ok(recv.clone())
        }
        "reverse" => {
            argc(&args, 0, 0)?;
            Ok(Value::string(st.chars().rev().collect()))
        }
        "include?" => {
            argc(&args, 1, 1)?;
            Ok(Value::Bool(st.contains(&*to_str(&args[0])?)))
        }
        "start_with?" | "end_with?" => {
            let mut hit = false;
            for a in &args {
                let p = to_str(a)?;
                if (name == "start_with?" && st.starts_with(&*p)) || (name == "end_with?" && st.ends_with(&*p)) {
                    hit = true;
                }
            }
            Ok(Value::Bool(hit))
        }
        "index" | "rindex" => {
            argc(&args, 1, 2)?;
            let sub = to_str(&args[0])?;
            let chars: Vec<char> = st.chars().collect();
            let n = chars.len() as i64;
            let start = match args.get(1) {
                Some(v) => {
                    let x = to_int(v)?;
                    let x = if x < 0 { x + n } else { x };
                    if x < 0 || (name == "index" && x > n) {
                        return Ok(Value::Nil);
                    }
                    x.min(n)
                }
                None => {
                    if name == "index" {
                        0
                    } else {
                        n
                    }
                }
            };
            if name == "index" {
                let b = byte_of_char(st, start as usize);
                return Ok(match st[b..].find(&*sub) {
                    Some(i) => Value::Int(char_index_of_byte(st, b + i)),
                    None => Value::Nil,
                });
            }
            let subc: Vec<char> = sub.chars().collect();
            let mut i = start;
            while i >= 0 {
                let iu = i as usize;
                if iu + subc.len() <= chars.len() && chars[iu..iu + subc.len()] == subc[..] {
                    return Ok(Value::Int(i));
                }
                i -= 1;
            }
            Ok(Value::Nil)
        }
        "count" => {
            argc(&args, 1, 1)?;
            let sub = to_str(&args[0])?;
            if sub.is_empty() {
                return Ok(Value::Int(0));
            }
            Ok(Value::Int(st.matches(&*sub).count() as i64))
        }
        "sub" => sub_gsub(it, st, &args, blk, false),
        "gsub" => sub_gsub(it, st, &args, blk, true),
        "tr" => {
            argc(&args, 2, 2)?;
            let from = to_str(&args[0])?;
            let to = to_str(&args[1])?;
            if to.is_empty() {
                let set = CharSet::parse(&from)?;
                return Ok(Value::string(st.chars().filter(|c| !set.contains(*c)).collect()));
            }
            s(tr(st, &from, &to)?)
        }
        "delete" => {
            argc(&args, 1, usize::MAX)?;
            let sets = sets(&args)?;
            Ok(Value::string(st.chars().filter(|c| !sets.iter().all(|set| set.contains(*c))).collect()))
        }
        "squeeze" => {
            let sets = sets(&args)?;
            let mut out = String::new();
            let mut last: Option<char> = None;
            for c in st.chars() {
                if last == Some(c) && sets.iter().all(|set| set.contains(c)) {
                    continue;
                }
                out.push(c);
                last = Some(c);
            }
            Ok(Value::string(out))
        }
        "center" => justify(st, &args, 'c'),
        "ljust" => justify(st, &args, 'l'),
        "rjust" => justify(st, &args, 'r'),
        "to_i" => {
            argc(&args, 0, 1)?;
            let base = match args.first() {
                Some(b) => to_int(b)?,
                None => 10,
            };
            if !(2..=36).contains(&base) {
                return fail(ARGUMENT_ERROR, format!("invalid radix {}", base));
            }
            Ok(Value::Int(str_to_i(st, base as u32)?))
        }
        "to_f" => {
            argc(&args, 0, 0)?;
            Ok(Value::Float(str_to_f(st)))
        }
        "to_str" => {
            argc(&args, 0, 0)?;
            Ok(recv.clone())
        }
        "to_sym" => {
            argc(&args, 0, 0)?;
            Ok(Value::Sym(st.clone()))
        }
        "ord" => {
            argc(&args, 0, 0)?;
            match st.chars().next() {
                Some(c) => Ok(Value::Int(c as i64)),
                None => fail(ARGUMENT_ERROR, "empty string"),
            }
        }
        "hex" => {
            argc(&args, 0, 0)?;
            Ok(Value::Int(str_to_i(st, 16)?))
        }
        "oct" => {
            argc(&args, 0, 0)?;
            Ok(Value::Int(str_oct(st)?))
        }
        "split" => split(st, &args),
        "partition" | "rpartition" => {
            argc(&args, 1, 1)?;
            let sep = match &args[0] {
                Value::Str(x) => x.clone(),
                other => return fail(TYPE_ERROR, format!("wrong argument type {} (expected Regexp)", other.class_name())),
            };
            let found = if name == "partition" { st.find(&*sep) } else { st.rfind(&*sep) };
            let parts = match found {
                Some(i) => [&st[..i], &st[i..i + sep.len()], &st[i + sep.len()..]],
                None if name == "partition" => [&st[..], "", ""],
                None => ["", "", &st[..]],
            };
            Ok(Value::array(parts.iter().map(|p| Value::str(p)).collect()))
        }
        "casecmp" => {
            argc(&args, 1, 1)?;
            let o = to_str(&args[0])?;
            let a = st.to_ascii_lowercase();
            let b = o.to_ascii_lowercase();
            Ok(Value::Int(a.as_bytes().cmp(b.as_bytes()) as i64))
        }
        "casecmp?" => {
            argc(&args, 1, 1)?;
            let o = to_str(&args[0])?;
            Ok(Value::Bool(st.to_lowercase() == o.to_lowercase()))
        }
        "between?" => between(recv, &args),
        "ascii_only?" => {
            argc(&args, 0, 0)?;
            Ok(Value::Bool(st.is_ascii()))
        }
        _ => object::call(it, recv, name, args, blk),
    }
}

pub fn call_sym(it: &mut Interp, sy: &Arc<str>, recv: &Value, name: &str, args: Vec<Value>, blk: Option<&BlockArg>) -> R<Value> {
    match name {
        "id2name" | "name" => {
            argc(&args, 0, 0)?;
            Ok(Value::Str(sy.clone()))
        }
        "to_sym" => {
            argc(&args, 0, 0)?;
            Ok(recv.clone())
        }
        "length" | "size" => {
            argc(&args, 0, 0)?;
            Ok(Value::Int(sy.chars().count() as i64))
        }
        "upcase" | "downcase" | "capitalize" => {
            argc(&args, 0, 0)?;
            let out = match name {
                "upcase" => sy.to_uppercase(),
                "downcase" => sy.to_lowercase(),
                _ => capitalize(sy),
            };
            Ok(Value::sym(&out))
        }
        "<=>" => {
            argc(&args, 1, 1)?;
            Ok(match &args[0] {
                Value::Sym(o) => Value::Int(sy.as_bytes().cmp(o.as_bytes()) as i64),
                _ => Value::Nil,
            })
        }
        "start_with?" | "end_with?" => {
            let mut hit = false;
            for a in &args {
                let p = to_str(a)?;
                if (name == "start_with?" && sy.starts_with(&*p)) || (name == "end_with?" && sy.ends_with(&*p)) {
                    hit = true;
                }
            }
            Ok(Value::Bool(hit))
        }
        "empty?" => {
            argc(&args, 0, 0)?;
            Ok(Value::Bool(sy.is_empty()))
        }
        _ => object::call(it, recv, name, args, blk),
    }
}
