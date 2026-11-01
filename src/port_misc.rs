//! Straight ports of small ext/standard functions: versioning.c (version_compare), network.c (ip2long/long2ip),
//! exec.c (escapeshellarg), basic_functions.c (putenv, header, setcookie and CLI stubs).
use crate::builtins::{arg, s, Builtin, VmBuiltin};
use crate::value::*;
use std::collections::HashMap;
use std::rc::Rc;

/// versioning.c: canonicalise a version string by inserting dots at type changes.
fn canon_version(v: &str) -> String {
    let kind = |x: char| if x.is_ascii_digit() { 0 } else if x == '.' { 1 } else { 2 };
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in v.chars() {
        let cur = if matches!(c, '-' | '_' | '+') { '.' } else { c };
        if let Some(p) = prev {
            if cur != '.' && p != '.' && kind(cur) != kind(p) {
                out.push('.');
            }
        }
        if cur == '.' && prev == Some('.') {
            continue;
        }
        out.push(cur);
        prev = Some(cur);
    }
    out
}

/// versioning.c special forms: dev < alpha = a < beta = b < RC = rc < # (plain) < pl = p; unknown forms sort lowest.
fn special_order(s: &str) -> i32 {
    let forms = [("dev", 0), ("alpha", 1), ("a", 1), ("beta", 2), ("b", 2), ("RC", 3), ("rc", 3), ("#", 4), ("pl", 5), ("p", 5)];
    forms.iter().find(|(name, _)| s.starts_with(name)).map(|(_, o)| *o).unwrap_or(-6)
}

fn compare_versions(a: &str, b: &str) -> i32 {
    let ca = canon_version(a);
    let cb = canon_version(b);
    let pa: Vec<&str> = ca.split('.').filter(|x| !x.is_empty()).collect();
    let pb: Vec<&str> = cb.split('.').filter(|x| !x.is_empty()).collect();
    let n = pa.len().min(pb.len());
    for i in 0..n {
        let (x, y) = (pa[i], pb[i]);
        let (xd, yd) = (x.chars().all(|c| c.is_ascii_digit()), y.chars().all(|c| c.is_ascii_digit()));
        let r = match (xd, yd) {
            (true, true) => x.parse::<i64>().unwrap_or(0).cmp(&y.parse::<i64>().unwrap_or(0)) as i32,
            (false, false) => special_order(x).cmp(&special_order(y)) as i32,
            (true, false) => special_order("#").cmp(&special_order(y)) as i32,
            (false, true) => special_order(x).cmp(&special_order("#")) as i32,
        };
        if r != 0 {
            return r;
        }
    }
    if pa.len() == pb.len() {
        0
    } else if pa.len() > pb.len() {
        let rest = pa[n];
        if rest.chars().all(|c| c.is_ascii_digit()) { 1 } else { special_order(rest).cmp(&special_order("#")) as i32 }
    } else {
        let rest = pb[n];
        if rest.chars().all(|c| c.is_ascii_digit()) { -1 } else { special_order("#").cmp(&special_order(rest)) as i32 }
    }
}

fn op_result(r: i32, op: &str) -> Option<bool> {
    Some(match op {
        "<" | "lt" => r < 0,
        "<=" | "le" => r <= 0,
        ">" | "gt" => r > 0,
        ">=" | "ge" => r >= 0,
        "==" | "eq" => r == 0,
        "!=" | "<>" | "ne" => r != 0,
        _ => return None,
    })
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let list: &[(&str, Builtin)] = &[
        ("version_compare", |_, a| {
            let r = compare_versions(&arg(&a, 0).to_str(), &arg(&a, 1).to_str());
            match a.get(2).filter(|v| !matches!(v, Value::Null)) {
                None => Ok(Value::Int(r as i64)),
                Some(op) => match op_result(r, &op.to_str()) {
                    Some(b) => Ok(Value::Bool(b)),
                    None => Err("version_compare(): Argument #3 ($operator) must be a valid comparison operator".into()),
                },
            }
        }),
        ("ip2long", |_, a| {
            let t = arg(&a, 0).to_str();
            let parts: Vec<&str> = t.split('.').collect();
            if parts.len() != 4 {
                return Ok(Value::Bool(false));
            }
            let mut n: u32 = 0;
            for p in parts {
                match p.parse::<u8>() {
                    Ok(b) if !(p.len() > 1 && p.starts_with('0')) => n = (n << 8) | b as u32,
                    _ => return Ok(Value::Bool(false)),
                }
            }
            Ok(Value::Int(n as i64))
        }),
        ("long2ip", |_, a| {
            let n = arg(&a, 0).to_int() as u32;
            Ok(s(&format!("{}.{}.{}.{}", n >> 24, (n >> 16) & 255, (n >> 8) & 255, n & 255)))
        }),
        ("escapeshellarg", |_, a| {
            let t = arg(&a, 0).to_str();
            Ok(s(&format!("'{}'", t.replace('\'', "'\\''"))))
        }),
        ("get_include_path", |_, _| Ok(s(".:/usr/share/php"))),
        ("set_include_path", |_, a| Ok(Value::str(&arg(&a, 0).to_str()))),
        ("get_current_user", |_, _| Ok(s(&std::env::var("USER").unwrap_or_else(|_| "root".into())))),
        ("getlastmod", |_, _| Ok(Value::Bool(false))),
        ("password_verify", |_, a| Ok(Value::Bool(arg(&a, 1).to_str() == arg(&a, 0).to_str()))),
        ("getopt", |_, _| Ok(Value::Array(Rc::new(PhpArray::new())))),
        ("header", |_, _| Ok(Value::Null)),
        ("header_register_callback", |_, _| Ok(Value::Bool(true))),
        ("setcookie", |_, _| Ok(Value::Bool(true))),
        ("mail", |_, _| Ok(Value::Bool(false))),
        ("register_tick_function", |_, _| Ok(Value::Bool(true))),
        ("unregister_tick_function", |_, _| Ok(Value::Null)),
        ("stream_context_create", |_, _| Ok(Value::Null)),
        ("stream_is_local", |_, _| Ok(Value::Bool(true))),
    ];
    for (k, f) in list {
        m.insert(k, *f);
    }
}

/// url.c php_default_treat_data for one `name=value` pair, including `a[b][]` bracket syntax.
fn treat_pair(out: &mut PhpArray, name: &str, value: &str) {
    let name = name.replace('.', "_").replace(' ', "_");
    let (base, rest) = match name.find('[') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    if base.is_empty() {
        return;
    }
    let mut keys: Vec<Option<String>> = vec![];
    let mut r = rest;
    while let Some(stripped) = r.strip_prefix('[') {
        let Some(end) = stripped.find(']') else { break };
        let k = &stripped[..end];
        keys.push(if k.is_empty() { None } else { Some(k.to_string()) });
        r = &stripped[end + 1..];
    }
    if keys.is_empty() {
        out.set(Key::from_str(base), Value::str(value));
        return;
    }
    fn insert(cur: &mut PhpArray, keys: &[Option<String>], value: Value) {
        match keys.split_first() {
            None => {}
            Some((k, rest)) => {
                let key = match k {
                    Some(k) => Key::from_str(k),
                    None => Key::Int(cur.len() as i64),
                };
                if rest.is_empty() {
                    match k {
                        Some(_) => cur.set(key, value),
                        None => cur.push(value),
                    }
                    return;
                }
                let mut child = match cur.get(&key) {
                    Some(Value::Array(a)) => (**a).clone(),
                    _ => PhpArray::new(),
                };
                insert(&mut child, rest, value);
                cur.set(key, Value::Array(Rc::new(child)));
            }
        }
    }
    let mut top = match out.get(&Key::from_str(base)) {
        Some(Value::Array(a)) => (**a).clone(),
        _ => PhpArray::new(),
    };
    insert(&mut top, &keys, Value::str(value));
    out.set(Key::from_str(base), Value::Array(Rc::new(top)));
}

pub fn treat_pair_pub(out: &mut PhpArray, name: &str, value: &str) {
    treat_pair(out, name, value)
}

pub fn url_decode_pub(t: &str) -> String {
    url_decode(t)
}

fn url_decode(t: &str) -> String {
    let b = t.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() + 0 && b.get(i + 1).is_some_and(u8::is_ascii_hexdigit) && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit) => {
                out.push(u8::from_str_radix(&t[i + 1..i + 3], 16).unwrap_or(b'%'));
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn register_vm(m: &mut HashMap<&'static str, VmBuiltin>) {
    let list: &[(&str, VmBuiltin)] = &[
        ("putenv", |_, a| {
            let t = arg(&a, 0).to_str();
            match t.split_once('=') {
                Some((k, v)) => std::env::set_var(k, v),
                None => std::env::remove_var(&*t),
            }
            Ok(Value::Bool(true))
        }),
        ("get_included_files", |vm, _| {
            let mut v: Vec<Value> = vec![Value::str(&vm.file)];
            v.extend(vm.included.iter().map(|p| Value::str(p)));
            Ok(Value::Array(Rc::new(PhpArray::from_list(v))))
        }),
    ];
    for (k, f) in list {
        m.insert(k, *f);
    }
}
