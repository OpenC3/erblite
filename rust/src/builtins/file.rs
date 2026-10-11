//! Sandboxed `File` and `Dir` functions (§9.1) and the pure path helpers.
//!
//! Every path is resolved against the canonical `base_dir`. The lexically normalized path
//! and the fully resolved (symlink-free) path must both stay inside `base_dir`.

use super::{argc, check_str_len, to_str, MAX_ARRAY_LEN};
use crate::error::{fail, R, IO_ERROR, LIMIT_ERROR, SECURITY_ERROR, TYPE_ERROR};
use crate::interp::Interp;
use crate::value::Value;
use std::path::{Component, Path, PathBuf};

fn base(it: &Interp) -> R<PathBuf> {
    match &it.ctx.base_dir {
        Some(b) => Ok(b.clone()),
        None => fail(SECURITY_ERROR, "file access is disabled (no base_dir)"),
    }
}

fn outside(path: &str) -> R<PathBuf> {
    fail(SECURITY_ERROR, format!("access outside the sandbox: {}", path))
}

/// Joins and normalizes `.` / `..` without touching the file system.
fn lexical(base: &Path, path: &str) -> PathBuf {
    let joined = if Path::new(path).is_absolute() { PathBuf::from(path) } else { base.join(path) };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Error kinds from `resolve`: outside the sandbox, or missing.
pub enum Resolve {
    Found(PathBuf),
    Missing,
}

/// Resolves `path` inside the sandbox. Raises SecurityError when it escapes.
pub fn resolve(it: &Interp, path: &str) -> R<Resolve> {
    let base = base(it)?;
    if path.contains('\0') {
        return fail(SECURITY_ERROR, "path contains a NUL byte");
    }
    let lex = lexical(&base, path);
    if !lex.starts_with(&base) {
        return outside(path).map(Resolve::Found);
    }
    match lex.canonicalize() {
        Ok(real) => {
            if real.starts_with(&base) {
                Ok(Resolve::Found(real))
            } else {
                outside(path).map(Resolve::Found)
            }
        }
        Err(_) => {
            // Missing: make sure the deepest existing ancestor is inside the sandbox.
            let mut anc = lex.as_path();
            while let Some(p) = anc.parent() {
                anc = p;
                if let Ok(real) = anc.canonicalize() {
                    if !real.starts_with(&base) {
                        return outside(path).map(Resolve::Found);
                    }
                    break;
                }
            }
            Ok(Resolve::Missing)
        }
    }
}

/// Reads a file inside the sandbox as UTF-8 text.
pub fn read(it: &Interp, path: &str) -> R<String> {
    match resolve(it, path)? {
        Resolve::Found(p) => {
            if p.is_dir() {
                return fail(IO_ERROR, format!("Is a directory @ io_fread - {}", path));
            }
            let meta = std::fs::metadata(&p).map_err(|e| crate::error::rt_err(IO_ERROR, format!("{} - {}", e, path)))?;
            if meta.len() > super::MAX_STR_LEN as u64 {
                return fail(LIMIT_ERROR, "file too large");
            }
            match std::fs::read(&p) {
                Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
                Err(e) => fail(IO_ERROR, format!("{} - {}", e, path)),
            }
        }
        Resolve::Missing => fail(IO_ERROR, format!("No such file or directory @ rb_sysopen - {}", path)),
    }
}

fn exists_check(it: &Interp, path: &str, check: fn(&Path) -> bool) -> R<bool> {
    base(it)?;
    match resolve(it, path) {
        Ok(Resolve::Found(p)) => Ok(check(&p)),
        Ok(Resolve::Missing) => Ok(false),
        Err(_) => Ok(false),
    }
}

/// Ruby `File.basename`.
pub fn basename(p: &str, suffix: Option<&str>) -> String {
    if p.is_empty() {
        return String::new();
    }
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return "/".into();
    }
    let name = t.rsplit('/').next().unwrap_or(t);
    match suffix {
        Some(".*") => match name.rfind('.') {
            Some(i) if i > 0 => name[..i].to_string(),
            _ => name.to_string(),
        },
        Some(s) if !s.is_empty() && name.ends_with(s) && name != s => name[..name.len() - s.len()].to_string(),
        _ => name.to_string(),
    }
}

/// Ruby `File.dirname`.
pub fn dirname(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return if p.starts_with('/') { "/".into() } else { ".".into() };
    }
    match t.rfind('/') {
        None => ".".into(),
        Some(i) => {
            let d = t[..i].trim_end_matches('/');
            if d.is_empty() {
                "/".into()
            } else {
                d.to_string()
            }
        }
    }
}

/// Ruby `File.extname`.
pub fn extname(p: &str) -> String {
    let name = basename(p, None);
    let trimmed = name.trim_start_matches('.');
    match trimmed.rfind('.') {
        Some(i) if i + 1 < trimmed.len() => trimmed[i..].to_string(),
        Some(_) => ".".into(),
        None => String::new(),
    }
}

fn join_parts(args: &[Value], out: &mut Vec<String>, depth: usize) -> R<()> {
    if depth > 64 {
        return fail(crate::error::ARGUMENT_ERROR, "recursive array");
    }
    for a in args {
        match a {
            Value::Array(x) => {
                let inner = x.borrow().clone();
                join_parts(&inner, out, depth + 1)?;
            }
            Value::Str(s) => out.push(s.to_string()),
            other => return fail(TYPE_ERROR, format!("no implicit conversion of {} into String", other.type_desc())),
        }
    }
    Ok(())
}

/// Ruby `File.join`.
pub fn join(args: &[Value]) -> R<String> {
    let mut parts = Vec::new();
    join_parts(args, &mut parts, 0)?;
    let mut out = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            out.push_str(p);
            continue;
        }
        let left = out.trim_end_matches('/').len();
        out.truncate(left);
        out.push('/');
        out.push_str(p.trim_start_matches('/'));
        check_str_len(out.len())?;
    }
    Ok(out)
}

fn wildcard_match(pat: &[char], name: &[char]) -> bool {
    // iterative glob match with backtracking for '*'
    let (mut p, mut n) = (0, 0);
    let (mut star_p, mut star_n) = (usize::MAX, 0);
    while n < name.len() {
        if p < pat.len() && (pat[p] == '?' || pat[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pat.len() && pat[p] == '*' {
            star_p = p;
            star_n = n;
            p += 1;
        } else if star_p != usize::MAX {
            p = star_p + 1;
            star_n += 1;
            n = star_n;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == '*' {
        p += 1;
    }
    p == pat.len()
}

fn has_wild(s: &str) -> bool {
    s.contains(['*', '?'])
}

struct Globber<'a> {
    base: &'a Path,
    out: Vec<String>,
}

impl Globber<'_> {
    fn inside(&self, p: &Path) -> bool {
        p.canonicalize().map(|r| r.starts_with(self.base)).unwrap_or(false)
    }

    fn sorted_entries(&self, dir: &Path) -> Vec<(String, PathBuf, bool, bool)> {
        let mut v = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let ft = e.file_type().ok();
                let is_link = ft.map(|f| f.is_symlink()).unwrap_or(false);
                let path = e.path();
                let is_dir = path.is_dir();
                v.push((name, path, is_dir, is_link));
            }
        }
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    fn walk(&mut self, dir: &Path, prefix: &str, comps: &[&str], depth: usize) -> R<()> {
        if self.out.len() > MAX_ARRAY_LEN || depth > 64 {
            return fail(LIMIT_ERROR, "glob too large");
        }
        let Some((&c, rest)) = comps.split_first() else { return Ok(()) };
        if c == "**" {
            self.walk(dir, prefix, rest, depth + 1)?;
            for (name, path, is_dir, is_link) in self.sorted_entries(dir) {
                if is_dir && !is_link && !name.starts_with('.') && self.inside(&path) {
                    self.walk(&path, &format!("{}{}/", prefix, name), comps, depth + 1)?;
                }
            }
            return Ok(());
        }
        if has_wild(c) {
            let pat: Vec<char> = c.chars().collect();
            for (name, path, is_dir, _) in self.sorted_entries(dir) {
                if name.starts_with('.') && !c.starts_with('.') {
                    continue;
                }
                let nc: Vec<char> = name.chars().collect();
                if !wildcard_match(&pat, &nc) || !self.inside(&path) {
                    continue;
                }
                if rest.is_empty() {
                    self.out.push(format!("{}{}", prefix, name));
                } else if is_dir {
                    self.walk(&path, &format!("{}{}/", prefix, name), rest, depth + 1)?;
                }
            }
            return Ok(());
        }
        let path = dir.join(c);
        if !path.exists() {
            return Ok(());
        }
        if !self.inside(&path) {
            return fail(SECURITY_ERROR, format!("glob outside the sandbox: {}{}", prefix, c));
        }
        if rest.is_empty() {
            self.out.push(format!("{}{}", prefix, c));
        } else if path.is_dir() {
            self.walk(&path, &format!("{}{}/", prefix, c), rest, depth + 1)?;
        }
        Ok(())
    }
}

/// Sandboxed `Dir.glob(pattern)`: sorted paths relative to the base dir.
pub fn glob(it: &Interp, pattern: &str) -> R<Vec<String>> {
    let base = base(it)?;
    if pattern.contains('\0') {
        return fail(SECURITY_ERROR, "pattern contains a NUL byte");
    }
    // Reject patterns whose fixed part leaves the sandbox.
    let fixed: Vec<&str> = pattern.split('/').take_while(|c| !has_wild(c)).collect();
    let lex = lexical(&base, &fixed.join("/"));
    if Path::new(pattern).is_absolute() || !lex.starts_with(&base) {
        return fail(SECURITY_ERROR, format!("glob outside the sandbox: {}", pattern));
    }
    let comps: Vec<&str> = pattern.split('/').filter(|c| !c.is_empty()).collect();
    let mut g = Globber { base: &base, out: Vec::new() };
    g.walk(&base, "", &comps, 0)?;
    let mut out = g.out;
    out.sort();
    out.dedup();
    Ok(out)
}

pub fn call_file(it: &mut Interp, name: &str, args: &[Value]) -> R<Value> {
    match name {
        "read" => {
            argc(args, 1, 1)?;
            let p = to_str(&args[0])?;
            Ok(Value::string(read(it, &p)?))
        }
        "exist?" | "file?" | "directory?" => {
            argc(args, 1, 1)?;
            let p = to_str(&args[0])?;
            let check: fn(&Path) -> bool = match name {
                "exist?" => |p| p.exists(),
                "file?" => |p| p.is_file(),
                _ => |p| p.is_dir(),
            };
            Ok(Value::Bool(exists_check(it, &p, check)?))
        }
        "basename" => {
            argc(args, 1, 2)?;
            let p = to_str(&args[0])?;
            let suf = match args.get(1) {
                Some(s) => Some(to_str(s)?),
                None => None,
            };
            Ok(Value::string(basename(&p, suf.as_deref())))
        }
        "dirname" => {
            argc(args, 1, 1)?;
            Ok(Value::string(dirname(&to_str(&args[0])?)))
        }
        "extname" => {
            argc(args, 1, 1)?;
            Ok(Value::string(extname(&to_str(&args[0])?)))
        }
        "join" => Ok(Value::string(join(args)?)),
        _ => super::no_method(&Value::Class(crate::value::ClassId::File), name),
    }
}
