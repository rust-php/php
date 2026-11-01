//! Built-in functions and constants.
use crate::value::*;
use crate::vm::{Flow, Vm};
use crate::object::Extra;
use std::{collections::HashMap, rc::Rc, sync::OnceLock};

pub use crate::refbuiltins::{call_ref, ref_positions};

pub(crate) type Res = Result<Value, String>;
pub(crate) type Builtin = fn(&mut Vm<'_>, Vec<Value>) -> Res;

pub(crate) fn arg(a: &[Value], i: usize) -> Value {
    a.get(i).cloned().unwrap_or(Value::Null)
}

pub(crate) fn s(v: &str) -> Value {
    Value::Str(v.into())
}

pub(crate) fn arr(items: Vec<Value>) -> Value {
    Value::Array(Rc::new(PhpArray::from_list(items)))
}

pub(crate) fn need_arr(a: &[Value], i: usize, f: &str) -> Result<Rc<PhpArray>, String> {
    match a.get(i) {
        Some(Value::Array(x)) => Ok(x.clone()),
        other => Err(format!(
            "{f}(): Argument #{} ($array) must be of type array, {} given",
            i + 1,
            other.map(Value::type_name).unwrap_or("null")
        )),
    }
}

pub fn constant(name: &str) -> Option<Value> {
    if let Some(v) = crate::port_var::constant(name) {
        return Some(v);
    }
    Some(match name {
        "PHP_EOL" => s("\n"),
        "PHP_INT_MAX" => Value::Int(i64::MAX),
        "PHP_INT_MIN" => Value::Int(i64::MIN),
        "PHP_INT_SIZE" => Value::Int(8),
        "PHP_FLOAT_EPSILON" => Value::Float(f64::EPSILON),
        "PHP_FLOAT_MAX" => Value::Float(f64::MAX),
        "PHP_FLOAT_MIN" => Value::Float(f64::MIN_POSITIVE),
        "PHP_VERSION" => s("8.4.0-rust"),
        "PHP_OS" => s("Linux"),
        "NAN" => Value::Float(f64::NAN),
        "INF" => Value::Float(f64::INFINITY),
        "M_PI" => Value::Float(std::f64::consts::PI),
        "M_E" => Value::Float(std::f64::consts::E),
        "STDIN" => Value::Resource(crate::stream::std_resource(0)),
        "STDOUT" => Value::Resource(crate::stream::std_resource(1)),
        "STDERR" => Value::Resource(crate::stream::std_resource(2)),
        "DIRECTORY_SEPARATOR" => s("/"),
        "PATH_SEPARATOR" => s(":"),
        "PHP_OS_FAMILY" => s("Linux"),
        "FILE_USE_INCLUDE_PATH" => Value::Int(1),
        "LOCK_EX" => Value::Int(2),
        "FILE_APPEND" => Value::Int(8),
        "FILE_IGNORE_NEW_LINES" => Value::Int(2),
        "FILE_SKIP_EMPTY_LINES" => Value::Int(4),
        "LOCK_SH" => Value::Int(1),
        "LOCK_UN" => Value::Int(3),
        "SEEK_SET" => Value::Int(0),
        "SEEK_CUR" => Value::Int(1),
        "SEEK_END" => Value::Int(2),
        "GLOB_BRACE" => Value::Int(1024),
        "GLOB_ONLYDIR" => Value::Int(8192),
        "PATHINFO_DIRNAME" => Value::Int(1),
        "PATHINFO_BASENAME" => Value::Int(2),
        "PATHINFO_EXTENSION" => Value::Int(4),
        "PATHINFO_FILENAME" => Value::Int(8),
        "STR_PAD_RIGHT" => Value::Int(1),
        "STR_PAD_LEFT" => Value::Int(0),
        "STR_PAD_BOTH" => Value::Int(2),
        "SORT_REGULAR" => Value::Int(0),
        "SORT_NUMERIC" => Value::Int(1),
        "SORT_STRING" => Value::Int(2),
        "COUNT_RECURSIVE" => Value::Int(1),
        "ARRAY_FILTER_USE_KEY" => Value::Int(2),
        "ARRAY_FILTER_USE_BOTH" => Value::Int(1),
        "E_ALL" => Value::Int(32767),
        "E_ERROR" => Value::Int(1),
        "E_WARNING" => Value::Int(2),
        "E_NOTICE" => Value::Int(8),
        "E_STRICT" => Value::Int(2048),
        _ => return crate::port_array::constant(name).or_else(|| crate::port_string::constant(name)),
    })
}

fn table() -> &'static HashMap<&'static str, Builtin> {
    static T: OnceLock<HashMap<&'static str, Builtin>> = OnceLock::new();
    T.get_or_init(|| {
        let mut map_all: HashMap<&'static str, Builtin> = HashMap::new();
        crate::stdlib_str::register(&mut map_all);
        crate::stdlib_arr::register(&mut map_all);
        crate::stdlib_file::register_simple(&mut map_all);
        for n in ["preg_quote", "preg_split", "preg_grep", "preg_last_error", "preg_last_error_msg"] {
            map_all.insert(n, match n {
                "preg_quote" => |_, a| crate::stdlib_preg::simple("preg_quote", &a).unwrap(),
                "preg_split" => |_, a| crate::stdlib_preg::simple("preg_split", &a).unwrap(),
                "preg_grep" => |_, a| crate::stdlib_preg::simple("preg_grep", &a).unwrap(),
                "preg_last_error" => |_, a| crate::stdlib_preg::simple("preg_last_error", &a).unwrap(),
                _ => |_, a| crate::stdlib_preg::simple("preg_last_error_msg", &a).unwrap(),
            });
        }
        let list: &[(&str, Builtin)] = &[
            ("strlen", |_, a| Ok(Value::Int(arg(&a, 0).to_str().len() as i64))),
            ("count", |_, a| match a.first() {
                Some(Value::Array(x)) => Ok(Value::Int(x.len() as i64)),
                other => Err(format!(
                    "count(): Argument #1 ($value) must be of type Countable|array, {} given",
                    other.map(Value::type_name).unwrap_or("null")
                )),
            }),
            ("sizeof", |v, a| table()["count"](v, a)),
            ("str_repeat", |_, a| Ok(s(&arg(&a, 0).to_str().repeat(arg(&a, 1).to_int().max(0) as usize)))),
            ("strtoupper", |_, a| Ok(s(&arg(&a, 0).to_str().to_ascii_uppercase()))),
            ("strtolower", |_, a| Ok(s(&arg(&a, 0).to_str().to_ascii_lowercase()))),
            ("ucfirst", |_, a| Ok(s(&map_first(&arg(&a, 0).to_str(), true)))),
            ("lcfirst", |_, a| Ok(s(&map_first(&arg(&a, 0).to_str(), false)))),
            ("ucwords", |_, a| {
                let mut out = String::new();
                let mut up = true;
                for ch in arg(&a, 0).to_str().chars() {
                    out.push(if up { ch.to_ascii_uppercase() } else { ch });
                    up = matches!(ch, ' ' | '\t' | '\r' | '\n' | '\x0c' | '\x0b');
                }
                Ok(s(&out))
            }),
            ("trim", |_, a| Ok(trim(&a, true, true))),
            ("ltrim", |_, a| Ok(trim(&a, true, false))),
            ("rtrim", |_, a| Ok(trim(&a, false, true))),
            ("chop", |_, a| Ok(trim(&a, false, true))),
            ("implode", |_, a| {
                let (sep, items) = match (a.first(), a.get(1)) {
                    (Some(Value::Array(x)), None) => (Rc::from(""), x.clone()),
                    (Some(sep), Some(Value::Array(x))) => (sep.to_str(), x.clone()),
                    (Some(Value::Array(x)), Some(sep)) => (sep.to_str(), x.clone()),
                    _ => return Err("implode(): Argument #2 ($array) must be of type ?array, string given".into()),
                };
                let parts: Vec<Rc<str>> = items.iter().map(|(_, v)| v.to_str()).collect();
                Ok(s(&parts.join(&*sep)))
            }),
            ("join", |v, a| table()["implode"](v, a)),
            ("explode", |_, a| {
                let (sep, text) = (arg(&a, 0).to_str(), arg(&a, 1).to_str());
                if sep.is_empty() {
                    return Err("explode(): Argument #1 ($separator) cannot be empty".into());
                }
                let limit = a.get(2).map(Value::to_int).unwrap_or(i64::MAX);
                let mut parts: Vec<&str> = if limit > 0 {
                    text.splitn(limit as usize, &*sep).collect()
                } else {
                    text.split(&*sep).collect()
                };
                if limit < 0 {
                    let drop = (-limit) as usize;
                    parts.truncate(parts.len().saturating_sub(drop));
                }
                Ok(arr(parts.into_iter().map(s).collect()))
            }),
            ("substr", |_, a| {
                let b = arg(&a, 0).to_str().as_bytes().to_vec();
                let n = b.len() as i64;
                let mut st = arg(&a, 1).to_int();
                if st < 0 {
                    st = (n + st).max(0);
                }
                let st = st.min(n);
                let end = match a.get(2) {
                    None | Some(Value::Null) => n,
                    Some(l) => {
                        let l = l.to_int();
                        if l < 0 { (n + l).max(st) } else { (st + l).min(n) }
                    }
                };
                Ok(s(&String::from_utf8_lossy(&b[st as usize..end as usize])))
            }),
            ("strpos", |_, a| Ok(find(&a, false, false))),
            ("stripos", |_, a| Ok(find(&a, true, false))),
            ("strrpos", |_, a| Ok(find(&a, false, true))),
            ("str_contains", |_, a| Ok(Value::Bool(arg(&a, 0).to_str().contains(&*arg(&a, 1).to_str())))),
            ("str_starts_with", |_, a| Ok(Value::Bool(arg(&a, 0).to_str().starts_with(&*arg(&a, 1).to_str())))),
            ("str_ends_with", |_, a| Ok(Value::Bool(arg(&a, 0).to_str().ends_with(&*arg(&a, 1).to_str())))),
            ("str_replace", |_, a| {
                let mut subject = arg(&a, 2).to_str().to_string();
                let pairs: Vec<(String, String)> = match (arg(&a, 0), arg(&a, 1)) {
                    (Value::Array(se), Value::Array(re)) => {
                        let reps: Vec<_> = re.iter().map(|(_, v)| v.to_str().to_string()).collect();
                        se.iter().enumerate().map(|(i, (_, v))| (v.to_str().to_string(), reps.get(i).cloned().unwrap_or_default())).collect()
                    }
                    (Value::Array(se), r) => se.iter().map(|(_, v)| (v.to_str().to_string(), r.to_str().to_string())).collect(),
                    (se, r) => vec![(se.to_str().to_string(), r.to_str().to_string())],
                };
                for (from, to) in pairs {
                    if !from.is_empty() {
                        subject = subject.replace(&from, &to);
                    }
                }
                Ok(s(&subject))
            }),
            ("strrev", |_, a| Ok(s(&arg(&a, 0).to_str().chars().rev().collect::<String>()))),
            ("str_pad", |_, a| {
                let (text, len) = (arg(&a, 0).to_str(), arg(&a, 1).to_int().max(0) as usize);
                let pad: Vec<char> = a.get(2).map(|p| p.to_str().chars().collect()).unwrap_or_else(|| vec![' ']);
                let kind = a.get(3).map(Value::to_int).unwrap_or(1);
                let cur = text.chars().count();
                if len <= cur || pad.is_empty() {
                    return Ok(Value::Str(text));
                }
                let total = len - cur;
                let (l, r) = match kind {
                    0 => (total, 0),
                    2 => (total / 2, total - total / 2),
                    _ => (0, total),
                };
                let fill = |n: usize| -> String { (0..n).map(|i| pad[i % pad.len()]).collect() };
                Ok(s(&format!("{}{}{}", fill(l), text, fill(r))))
            }),
            ("str_split", |_, a| {
                let n = a.get(1).map(Value::to_int).unwrap_or(1).max(1) as usize;
                let chars: Vec<char> = arg(&a, 0).to_str().chars().collect();
                if chars.is_empty() {
                    return Ok(arr(vec![s("")]));
                }
                Ok(arr(chars.chunks(n).map(|c| s(&c.iter().collect::<String>())).collect()))
            }),
            ("sprintf", |_, a| sprintf(&a).map(|r| s(&r))),
            ("printf", |vm, a| {
                let r = sprintf(&a)?;
                vm.write(&r);
                Ok(Value::Int(r.len() as i64))
            }),
            ("number_format", |_, a| {
                let dec = a.get(1).map(Value::to_int).unwrap_or(0).max(0) as usize;
                let dp = a.get(2).map(|v| v.to_str().to_string()).unwrap_or_else(|| ".".into());
                let ts = a.get(3).map(|v| v.to_str().to_string()).unwrap_or_else(|| ",".into());
                let x = php_round(arg(&a, 0).to_float(), dec as i32);
                let t = format!("{:.*}", dec, x.abs());
                let (ip, fp) = t.split_once('.').map(|(i, f)| (i.to_string(), Some(f.to_string()))).unwrap_or((t.clone(), None));
                let mut grouped = String::new();
                for (i, ch) in ip.chars().enumerate() {
                    if i > 0 && (ip.len() - i) % 3 == 0 {
                        grouped.push_str(&ts);
                    }
                    grouped.push(ch);
                }
                let neg = x < 0.0 && t.chars().any(|c| c.is_ascii_digit() && c != '0');
                Ok(s(&format!("{}{}{}", if neg { "-" } else { "" }, grouped, fp.map(|f| format!("{dp}{f}")).unwrap_or_default())))
            }),
            ("intdiv", |_, a| {
                let (x, y) = (arg(&a, 0).to_int(), arg(&a, 1).to_int());
                if y == 0 {
                    return Err("Division by zero".into());
                }
                x.checked_div(y).map(Value::Int).ok_or_else(|| "Division of PHP_INT_MIN by -1 is not an integer".into())
            }),
            ("abs", |_, a| Ok(match arg(&a, 0) {
                Value::Int(i) => i.checked_abs().map(Value::Int).unwrap_or(Value::Float(-(i as f64))),
                v => Value::Float(v.to_float().abs()),
            })),
            ("max", |_, a| extreme(&a, true)),
            ("min", |_, a| extreme(&a, false)),
            ("floor", |_, a| Ok(Value::Float(arg(&a, 0).to_float().floor()))),
            ("ceil", |_, a| Ok(Value::Float(arg(&a, 0).to_float().ceil()))),
            ("round", |_, a| Ok(Value::Float(php_round(arg(&a, 0).to_float(), a.get(1).map(Value::to_int).unwrap_or(0) as i32)))),
            ("sqrt", |_, a| Ok(Value::Float(arg(&a, 0).to_float().sqrt()))),
            ("pow", |_, a| arith("**", &arg(&a, 0), &arg(&a, 1))),
            ("fmod", |_, a| Ok(Value::Float(arg(&a, 0).to_float() % arg(&a, 1).to_float()))),
            ("pi", |_, _| Ok(Value::Float(std::f64::consts::PI))),
            ("is_int", |_, a| Ok(Value::Bool(matches!(arg(&a, 0), Value::Int(_))))),
            ("is_integer", |_, a| Ok(Value::Bool(matches!(arg(&a, 0), Value::Int(_))))),
            ("is_float", |_, a| Ok(Value::Bool(matches!(arg(&a, 0), Value::Float(_))))),
            ("is_string", |_, a| Ok(Value::Bool(matches!(arg(&a, 0), Value::Str(_))))),
            ("is_bool", |_, a| Ok(Value::Bool(matches!(arg(&a, 0), Value::Bool(_))))),
            ("is_array", |_, a| Ok(Value::Bool(matches!(arg(&a, 0), Value::Array(_))))),
            ("is_null", |_, a| Ok(Value::Bool(matches!(arg(&a, 0), Value::Null)))),
            ("is_numeric", |_, a| Ok(Value::Bool(match arg(&a, 0) {
                Value::Int(_) | Value::Float(_) => true,
                Value::Str(x) => parse_numeric(&x).is_some_and(|(_, w)| w),
                _ => false,
            }))),
            ("gettype", |_, a| Ok(s(match arg(&a, 0) {
                Value::Null => "NULL",
                Value::Bool(_) => "boolean",
                Value::Int(_) => "integer",
                Value::Float(_) => "double",
                Value::Str(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
                Value::Ref(_) => "reference",
                Value::Resource(r) => if r.is_closed() { "resource (closed)" } else { "resource" },
            }))),
            ("intval", |_, a| Ok(Value::Int(arg(&a, 0).to_int()))),
            ("floatval", |_, a| Ok(Value::Float(arg(&a, 0).to_float()))),
            ("doubleval", |_, a| Ok(Value::Float(arg(&a, 0).to_float()))),
            ("strval", |_, a| Ok(Value::Str(arg(&a, 0).to_str()))),
            ("boolval", |_, a| Ok(Value::Bool(arg(&a, 0).to_bool()))),
            ("var_dump", |vm, a| {
                for v in &a {
                    var_dump(v, 0, &mut vm.out);
                }
                Ok(Value::Null)
            }),
            ("print_r", |vm, a| {
                let mut out = String::new();
                print_r(&arg(&a, 0), 0, &mut out);
                if a.get(1).is_some_and(Value::to_bool) {
                    return Ok(s(&out));
                }
                vm.write(&out);
                Ok(Value::Bool(true))
            }),
            ("var_export", |vm, a| {
                let mut out = String::new();
                var_export(&arg(&a, 0), 0, &mut out);
                if a.get(1).is_some_and(Value::to_bool) {
                    return Ok(s(&out));
                }
                vm.write(&out);
                Ok(Value::Null)
            }),
            ("array_keys", |_, a| Ok(arr(need_arr(&a, 0, "array_keys")?.iter().map(|(k, _)| k.to_value()).collect()))),
            ("array_values", |_, a| Ok(arr(need_arr(&a, 0, "array_values")?.iter().map(|(_, v)| v.clone()).collect()))),
            ("array_merge", |_, a| {
                let mut out = PhpArray::new();
                for i in 0..a.len() {
                    for (k, v) in need_arr(&a, i, "array_merge")?.iter() {
                        match k {
                            Key::Int(_) => out.push(v.clone()),
                            k => out.set(k.clone(), v.clone()),
                        }
                    }
                }
                Ok(Value::Array(Rc::new(out)))
            }),
            ("array_sum", |_, a| {
                let mut acc = Value::Int(0);
                for (_, v) in need_arr(&a, 0, "array_sum")?.iter() {
                    acc = arith("+", &acc, &Value::Int(0).clone()).and_then(|x| arith("+", &x, &num_value(v)))?;
                }
                Ok(acc)
            }),
            ("array_product", |_, a| {
                let mut acc = Value::Int(1);
                for (_, v) in need_arr(&a, 0, "array_product")?.iter() {
                    acc = arith("*", &acc, &num_value(v))?;
                }
                Ok(acc)
            }),
            ("in_array", |_, a| {
                let strict = a.get(2).is_some_and(Value::to_bool);
                let needle = arg(&a, 0);
                Ok(Value::Bool(need_arr(&a, 1, "in_array")?.iter().any(|(_, v)| if strict { strict_eq(v, &needle) } else { loose_eq(v, &needle) })))
            }),
            ("array_search", |_, a| {
                let strict = a.get(2).is_some_and(Value::to_bool);
                let needle = arg(&a, 0);
                let h = need_arr(&a, 1, "array_search")?;
                let hit = h.iter().find(|(_, v)| if strict { strict_eq(v, &needle) } else { loose_eq(v, &needle) });
                Ok(hit.map(|(k, _)| k.to_value()).unwrap_or(Value::Bool(false)))
            }),
            ("array_key_exists", |_, a| Ok(Value::Bool(need_arr(&a, 1, "array_key_exists")?.get(&Key::from_value(&arg(&a, 0))?).is_some()))),
            ("array_key_first", |_, a| Ok(need_arr(&a, 0, "array_key_first")?.iter().next().map(|(k, _)| k.to_value()).unwrap_or(Value::Null))),
            ("array_key_last", |_, a| Ok(need_arr(&a, 0, "array_key_last")?.iter().last().map(|(k, _)| k.to_value()).unwrap_or(Value::Null))),
            ("array_reverse", |_, a| {
                let keep = a.get(1).is_some_and(Value::to_bool);
                let src = need_arr(&a, 0, "array_reverse")?;
                let items: Vec<(Key, Value)> = src.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                let mut out = PhpArray::new();
                for (k, v) in items.into_iter().rev() {
                    match k {
                        Key::Int(_) if !keep => out.push(v),
                        k => out.set(k, v),
                    }
                }
                Ok(Value::Array(Rc::new(out)))
            }),
            ("array_slice", |_, a| {
                let src = need_arr(&a, 0, "array_slice")?;
                let n = src.len() as i64;
                let mut off = arg(&a, 1).to_int();
                if off < 0 {
                    off = (n + off).max(0);
                }
                let off = off.min(n);
                let end = match a.get(2) {
                    None | Some(Value::Null) => n,
                    Some(l) => {
                        let l = l.to_int();
                        if l < 0 { (n + l).max(off) } else { (off + l).min(n) }
                    }
                };
                let keep = a.get(3).is_some_and(Value::to_bool);
                let mut out = PhpArray::new();
                for (k, v) in src.iter().skip(off as usize).take((end - off) as usize) {
                    match k {
                        Key::Int(_) if !keep => out.push(v.clone()),
                        k => out.set(k.clone(), v.clone()),
                    }
                }
                Ok(Value::Array(Rc::new(out)))
            }),
            ("array_flip", |_, a| {
                let mut out = PhpArray::new();
                for (k, v) in need_arr(&a, 0, "array_flip")?.iter() {
                    out.set(Key::from_value(v)?, k.to_value());
                }
                Ok(Value::Array(Rc::new(out)))
            }),
            ("array_combine", |_, a| {
                let (ks, vs) = (need_arr(&a, 0, "array_combine")?, need_arr(&a, 1, "array_combine")?);
                if ks.len() != vs.len() {
                    return Err("array_combine(): Argument #1 ($keys) and argument #2 ($values) must have the same number of elements".into());
                }
                let mut out = PhpArray::new();
                for ((_, k), (_, v)) in ks.iter().zip(vs.iter()) {
                    out.set(Key::from_value(k)?, v.clone());
                }
                Ok(Value::Array(Rc::new(out)))
            }),
            ("array_unique", |_, a| {
                let src = need_arr(&a, 0, "array_unique")?;
                let mut out = PhpArray::new();
                let mut seen: Vec<Value> = vec![];
                for (k, v) in src.iter() {
                    if !seen.iter().any(|x| loose_eq(x, v)) {
                        seen.push(v.clone());
                        out.set(k.clone(), v.clone());
                    }
                }
                Ok(Value::Array(Rc::new(out)))
            }),
            ("array_fill", |_, a| {
                let (start, n) = (arg(&a, 0).to_int(), arg(&a, 1).to_int());
                let mut out = PhpArray::new();
                for i in 0..n.max(0) {
                    out.set(Key::Int(start + i), arg(&a, 2));
                }
                Ok(Value::Array(Rc::new(out)))
            }),
            ("range", |_, a| {
                let (lo, hi) = (arg(&a, 0), arg(&a, 1));
                if let (Value::Str(x), Value::Str(y)) = (&lo, &hi) {
                    if x.len() == 1 && y.len() == 1 && parse_numeric(x).is_none() {
                        let (p, q) = (x.as_bytes()[0] as i64, y.as_bytes()[0] as i64);
                        let step = a.get(2).map(Value::to_int).unwrap_or(1).abs().max(1);
                        let mut v = vec![];
                        let mut c = p;
                        while if p <= q { c <= q } else { c >= q } {
                            v.push(s(&(c as u8 as char).to_string()));
                            c += if p <= q { step } else { -step };
                        }
                        return Ok(arr(v));
                    }
                }
                let float = matches!(lo, Value::Float(_)) || matches!(hi, Value::Float(_)) || matches!(a.get(2), Some(Value::Float(f)) if f.fract() != 0.0);
                let step = a.get(2).map(Value::to_float).unwrap_or(1.0).abs();
                if step == 0.0 {
                    return Err("range(): Argument #3 ($step) cannot be 0".into());
                }
                let (p, q) = (lo.to_float(), hi.to_float());
                let count = ((p - q).abs() / step + 1e-9).floor() as i64;
                let dir = if p <= q { 1.0 } else { -1.0 };
                let v = (0..=count)
                    .map(|i| {
                        let x = p + dir * step * i as f64;
                        if float { Value::Float(x) } else { Value::Int(x as i64) }
                    })
                    .collect();
                Ok(arr(v))
            }),
            ("chr", |_, a| Ok(s(&((arg(&a, 0).to_int().rem_euclid(256)) as u8 as char).to_string()))),
            ("ord", |_, a| Ok(Value::Int(arg(&a, 0).to_str().bytes().next().unwrap_or(0) as i64))),
            ("dechex", |_, a| Ok(s(&format!("{:x}", arg(&a, 0).to_int())))),
            ("decbin", |_, a| Ok(s(&format!("{:b}", arg(&a, 0).to_int())))),
            ("decoct", |_, a| Ok(s(&format!("{:o}", arg(&a, 0).to_int())))),
            ("hexdec", |_, a| Ok(Value::Int(i64::from_str_radix(&arg(&a, 0).to_str(), 16).unwrap_or(0)))),
            ("bindec", |_, a| Ok(Value::Int(i64::from_str_radix(&arg(&a, 0).to_str(), 2).unwrap_or(0)))),
            ("time", |_, _| Ok(Value::Int(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)))),
            ("microtime", |_, a| {
                let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64();
                Ok(if a.first().is_some_and(Value::to_bool) { Value::Float(d) } else { s(&format!("{:.8} {}", d.fract(), d as i64)) })
            }),
            ("function_exists", |vm, a| {
                let n = arg(&a, 0).to_str().to_ascii_lowercase();
                Ok(Value::Bool(vm.prog.func_index.contains_key(&n) || exists(&n)))
            }),
            ("error_reporting", |_, _| Ok(Value::Int(32767))),
            ("ini_set", |_, _| Ok(Value::Bool(false))),
            ("set_error_handler", |_, _| Ok(Value::Null)),
        ];
        for (k, f) in list {
            map_all.insert(k, *f);
        }
        crate::port_string::register(&mut map_all);
        crate::port_array::register(&mut map_all);
        crate::port_var::register(&mut map_all);
        crate::port_misc::register(&mut map_all);
        map_all
    })
}

pub fn exists(name: &str) -> bool {
    table().contains_key(name) || vm_table().contains_key(name)
}

pub fn call(vm: &mut Vm<'_>, name: &str, args: Vec<Value>) -> Option<Result<Value, Flow>> {
    if let Some(f) = vm_table().get(name) {
        return Some(f(vm, args));
    }
    table().get(name).map(|f| f(vm, args).map_err(Flow::Fatal))
}

fn num_value(v: &Value) -> Value {
    match v {
        Value::Int(_) | Value::Float(_) => v.clone(),
        Value::Str(x) => match parse_numeric(x) {
            Some((Num::I(i), _)) => Value::Int(i),
            Some((Num::F(f), _)) => Value::Float(f),
            None => Value::Int(0),
        },
        other => Value::Int(other.to_int()),
    }
}

fn map_first(t: &str, up: bool) -> String {
    let mut c = t.chars();
    match c.next() {
        Some(f) => format!("{}{}", if up { f.to_ascii_uppercase() } else { f.to_ascii_lowercase() }, c.as_str()),
        None => String::new(),
    }
}

fn trim(a: &[Value], left: bool, right: bool) -> Value {
    let text = arg(a, 0).to_str();
    let set: Vec<char> = a.get(1).map(|v| v.to_str().chars().collect()).unwrap_or_else(|| " \t\n\r\0\x0b".chars().collect());
    let mut t: &str = &text;
    if left {
        t = t.trim_start_matches(|c| set.contains(&c));
    }
    if right {
        t = t.trim_end_matches(|c| set.contains(&c));
    }
    s(t)
}

fn find(a: &[Value], ci: bool, last: bool) -> Value {
    let (mut h, mut n) = (arg(a, 0).to_str().to_string(), arg(a, 1).to_str().to_string());
    if ci {
        h = h.to_ascii_lowercase();
        n = n.to_ascii_lowercase();
    }
    let off = a.get(2).map(Value::to_int).unwrap_or(0);
    let start = if off < 0 { (h.len() as i64 + off).max(0) as usize } else { off as usize };
    if start > h.len() || !h.is_char_boundary(start) {
        return Value::Bool(false);
    }
    let hit = if last { h[start..].rfind(&n) } else { h[start..].find(&n) };
    hit.map(|i| Value::Int((i + start) as i64)).unwrap_or(Value::Bool(false))
}

fn extreme(a: &[Value], max: bool) -> Res {
    let items: Vec<Value> = match a {
        [Value::Array(x)] => x.iter().map(|(_, v)| v.clone()).collect(),
        _ => a.to_vec(),
    };
    let mut it = items.into_iter();
    let mut best = it.next().ok_or_else(|| format!("{}(): Argument #1 ($value) must contain at least one element", if max { "max" } else { "min" }))?;
    for v in it {
        let c = loose_cmp(&v, &best);
        if (max && c.is_gt()) || (!max && c.is_lt()) {
            best = v;
        }
    }
    Ok(best)
}

pub fn php_round(x: f64, places: i32) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let f = 10f64.powi(places.abs());
    let y = if places >= 0 { x * f } else { x / f };
    // Pre-round to 15 significant digits to absorb binary representation error (1.005 -> 1.01).
    let pre: f64 = format!("{:.14e}", y).parse().unwrap_or(y);
    let r = pre.round();
    if places >= 0 { r / f } else { r * f }
}

pub(crate) fn sprintf(a: &[Value]) -> Result<String, String> {
    let fmt: Vec<char> = arg(a, 0).to_str().chars().collect();
    let mut out = String::new();
    let (mut i, mut next_arg) = (0, 1);
    while i < fmt.len() {
        if fmt[i] != '%' {
            out.push(fmt[i]);
            i += 1;
            continue;
        }
        i += 1;
        if fmt.get(i) == Some(&'%') {
            out.push('%');
            i += 1;
            continue;
        }
        let mut explicit = None;
        let save = i;
        let mut n = 0usize;
        while fmt.get(i).is_some_and(char::is_ascii_digit) {
            n = n * 10 + fmt[i].to_digit(10).unwrap() as usize;
            i += 1;
        }
        if i > save && fmt.get(i) == Some(&'$') {
            explicit = Some(n);
            i += 1;
        } else {
            i = save;
        }
        let (mut left, mut plus, mut pad) = (false, false, ' ');
        loop {
            match fmt.get(i) {
                Some('-') => left = true,
                Some('+') => plus = true,
                Some('0') => pad = '0',
                Some(' ') => pad = ' ',
                Some('\'') => {
                    i += 1;
                    pad = *fmt.get(i).ok_or("Missing padding character")?;
                }
                _ => break,
            }
            i += 1;
        }
        let mut width = 0usize;
        while fmt.get(i).is_some_and(char::is_ascii_digit) {
            width = width * 10 + fmt[i].to_digit(10).unwrap() as usize;
            i += 1;
        }
        let mut prec = None;
        if fmt.get(i) == Some(&'.') {
            i += 1;
            let mut p = 0;
            while fmt.get(i).is_some_and(char::is_ascii_digit) {
                p = p * 10 + fmt[i].to_digit(10).unwrap() as usize;
                i += 1;
            }
            prec = Some(p);
        }
        let spec = *fmt.get(i).ok_or("Missing format specifier at end of string")?;
        i += 1;
        let idx = match explicit {
            Some(e) => e,
            None => {
                next_arg += 1;
                next_arg - 1
            }
        };
        let v = a.get(idx).ok_or_else(|| format!("{} arguments are required, {} given", idx + 1, a.len()))?;
        let mut body = match spec {
            'd' | 'i' => {
                let n = v.to_int();
                if plus && n >= 0 { format!("+{n}") } else { n.to_string() }
            }
            'u' => (v.to_int() as u64).to_string(),
            'f' | 'F' => {
                let x = v.to_float();
                let t = format!("{:.*}", prec.unwrap_or(6), php_round(x, prec.unwrap_or(6) as i32));
                if plus && x >= 0.0 { format!("+{t}") } else { t }
            }
            'e' | 'E' => {
                let t = format!("{:.*e}", prec.unwrap_or(6), v.to_float());
                let (m, e) = t.split_once('e').unwrap();
                let e: i32 = e.parse().unwrap();
                let t = format!("{m}e{}{}", if e < 0 { '-' } else { '+' }, e.abs());
                if spec == 'E' { t.to_uppercase() } else { t }
            }
            's' => {
                let t = v.to_str();
                match prec {
                    Some(p) => t.chars().take(p).collect(),
                    None => t.to_string(),
                }
            }
            'x' => format!("{:x}", v.to_int()),
            'X' => format!("{:X}", v.to_int()),
            'o' => format!("{:o}", v.to_int()),
            'b' => format!("{:b}", v.to_int()),
            'c' => ((v.to_int() as u8) as char).to_string(),
            other => return Err(format!("Unknown format specifier \"{other}\"")),
        };
        let len = body.chars().count();
        if len < width {
            let fill = width - len;
            if left {
                body.push_str(&pad.to_string().repeat(fill));
            } else if pad == '0' && matches!(body.chars().next(), Some('-' | '+')) && !matches!(spec, 's') {
                let (sign, rest) = body.split_at(1);
                body = format!("{sign}{}{rest}", "0".repeat(fill));
            } else {
                body = format!("{}{}", pad.to_string().repeat(fill), body);
            }
        }
        out.push_str(&body);
    }
    Ok(out)
}

fn var_dump(v: &Value, ind: usize, out: &mut String) {
    let pad = " ".repeat(ind);
    match v {
        Value::Ref(r) => var_dump(&r.borrow(), ind, out),
        Value::Resource(r) => out.push_str(&format!("{pad}resource({}) of type ({})\n", r.id, if r.is_closed() { "Unknown" } else { r.kind })),
        Value::Null => out.push_str(&format!("{pad}NULL\n")),
        Value::Bool(b) => out.push_str(&format!("{pad}bool({b})\n")),
        Value::Int(i) => out.push_str(&format!("{pad}int({i})\n")),
        Value::Float(f) => out.push_str(&format!("{pad}float({})\n", fmt_float(*f, None))),
        Value::Str(t) => out.push_str(&format!("{pad}string({}) \"{t}\"\n", t.len())),
        Value::Object(o) if o.class.kind == crate::ast::ClassKind::Enum => {
            let case = o.props.borrow().get(&Key::from_str("name")).map(|n| n.to_str().to_string()).unwrap_or_default();
            out.push_str(&format!("{pad}enum({}::{case})\n", o.class.name));
        }
        Value::Object(o) => {
            let props = o.props.borrow();
            out.push_str(&format!("{pad}object({})#{} ({}) {{\n", o.class.name, o.id, props.len()));
            for (k, x) in props.iter() {
                let label = match k {
                    Key::Int(i) => format!("[{i}]"),
                    Key::Str(t) => match o.class.prop_vis(t) {
                        Some((1, _)) => format!("[\"{t}\":protected]"),
                        Some((2, c)) => format!("[\"{t}\":\"{c}\":private]"),
                        _ => format!("[\"{t}\"]"),
                    },
                };
                out.push_str(&format!("{pad}  {label}=>\n"));
                var_dump(x, ind + 2, out);
            }
            out.push_str(&format!("{pad}}}\n"));
        }
        Value::Array(a) => {
            out.push_str(&format!("{pad}array({}) {{\n", a.len()));
            for (k, x) in a.iter() {
                match k {
                    Key::Int(i) => out.push_str(&format!("{pad}  [{i}]=>\n")),
                    Key::Str(t) => out.push_str(&format!("{pad}  [\"{t}\"]=>\n")),
                }
                var_dump(x, ind + 2, out);
            }
            out.push_str(&format!("{pad}}}\n"));
        }
    }
}

fn print_r(v: &Value, ind: usize, out: &mut String) {
    match v {
        Value::Object(o) => {
            let pad = " ".repeat(ind);
            let kind = if o.class.kind == crate::ast::ClassKind::Enum {
                match o.props.borrow().get(&Key::from_str("value")) {
                    Some(Value::Int(_)) => "Enum:int".to_string(),
                    Some(_) => "Enum:string".to_string(),
                    None => "Enum".to_string(),
                }
            } else {
                "Object".to_string()
            };
            out.push_str(&format!("{} {kind}\n{pad}(\n", o.class.name));
            for (k, x) in o.props.borrow().iter() {
                let key = match k {
                    Key::Int(i) => i.to_string(),
                    Key::Str(t) => match o.class.prop_vis(t) {
                        Some((1, _)) => format!("{t}:protected"),
                        Some((2, c)) => format!("{t}:{c}:private"),
                        _ => t.to_string(),
                    },
                };
                out.push_str(&format!("{pad}    [{key}] => "));
                print_r(x, ind + 8, out);
                out.push('\n');
            }
            out.push_str(&format!("{pad})\n"));
        }
        Value::Array(a) => {
            let pad = " ".repeat(ind);
            out.push_str(&format!("Array\n{pad}(\n"));
            for (k, x) in a.iter() {
                let key = match k {
                    Key::Int(i) => i.to_string(),
                    Key::Str(t) => t.to_string(),
                };
                out.push_str(&format!("{pad}    [{key}] => "));
                print_r(x, ind + 8, out);
                out.push('\n');
            }
            out.push_str(&format!("{pad})\n"));
        }
        other => out.push_str(&other.to_str()),
    }
}

fn var_export(v: &Value, ind: usize, out: &mut String) {
    let pad = " ".repeat(ind);
    match v {
        Value::Ref(r) => var_export(&r.borrow(), ind, out),
        Value::Resource(_) => out.push_str("NULL"),
        Value::Null => out.push_str("NULL"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => {
            let t = fmt_float(*f, None);
            out.push_str(&t);
            if f.is_finite() && !t.contains(['.', 'E']) {
                out.push_str(".0");
            }
        }
        Value::Str(t) => out.push_str(&format!("'{}'", t.replace('\\', "\\\\").replace('\'', "\\'"))),
        Value::Object(o) if o.class.kind == crate::ast::ClassKind::Enum => {
            let case = o.props.borrow().get(&Key::from_str("name")).map(|n| n.to_str().to_string()).unwrap_or_default();
            out.push_str(&format!("\\{}::{case}", o.class.name));
        }
        Value::Object(o) => {
            out.push_str(&format!("\\{}::__set_state(array(\n", o.class.name));
            for (k, x) in o.props.borrow().iter() {
                match k {
                    Key::Int(i) => out.push_str(&format!("{pad}   {i} => ")),
                    Key::Str(t) => out.push_str(&format!("{pad}   '{t}' => ")),
                }
                var_export(x, ind + 2, out);
                out.push_str(",\n");
            }
            out.push_str(&format!("{pad}))"));
        }
        Value::Array(a) => {
            out.push_str("array (\n");
            for (k, x) in a.iter() {
                match k {
                    Key::Int(i) => out.push_str(&format!("{pad}  {i} => ")),
                    Key::Str(t) => out.push_str(&format!("{pad}  '{}' => ", t.replace('\\', "\\\\").replace('\'', "\\'"))),
                }
                if matches!(x, Value::Array(_)) {
                    out.push_str(&format!("\n{pad}  "));
                }
                var_export(x, ind + 2, out);
                out.push_str(",\n");
            }
            out.push_str(&format!("{pad})"));
        }
    }
}

pub(crate) type VmBuiltin = fn(&mut Vm<'_>, Vec<Value>) -> Result<Value, Flow>;

pub(crate) fn bool_v(b: bool) -> Result<Value, Flow> {
    Ok(Value::Bool(b))
}

fn class_name_arg(vm: &Vm<'_>, v: &Value) -> Option<std::rc::Rc<crate::object::Class>> {
    vm.class_of(v).ok()
}

fn vm_table() -> &'static HashMap<&'static str, VmBuiltin> {
    static T: OnceLock<HashMap<&'static str, VmBuiltin>> = OnceLock::new();
    T.get_or_init(|| {
        let mut all: HashMap<&'static str, VmBuiltin> = HashMap::new();
        crate::stdlib_data::register(&mut all);
        crate::stdlib_file::register_vm(&mut all);
        let list: &[(&str, VmBuiltin)] = &[
            ("count", |vm, a| match a.first() {
                Some(Value::Array(x)) => Ok(Value::Int(x.len() as i64)),
                Some(o @ Value::Object(ob)) if ob.class.find_method("count").is_some() => {
                    let m = ob.class.find_method("count").unwrap();
                    Ok(Value::Int(vm.call_method(o, &m, vec![])?.to_int()))
                }
                other => vm.throw_error(
                    "TypeError",
                    &format!("count(): Argument #1 ($value) must be of type Countable|array, {} given", other.map(Value::type_name).unwrap_or("null")),
                ),
            }),
            ("sizeof", |vm, a| vm_table()["count"](vm, a)),
            ("call_user_func", |vm, a| {
                let mut it = a.into_iter();
                let f = it.next().unwrap_or(Value::Null);
                vm.call_value(&f, it.collect())
            }),
            ("call_user_func_array", |vm, a| {
                let f = arg(&a, 0);
                let args = match arg(&a, 1) {
                    Value::Array(x) => x.iter().map(|(_, v)| v.clone()).collect(),
                    _ => vec![],
                };
                vm.call_value(&f, args)
            }),
            ("is_callable", |vm, a| {
                let ok = match arg(&a, 0) {
                    Value::Object(o) => matches!(&*o.extra.borrow(), Extra::Closure(_)) || o.class.find_method("__invoke").is_some(),
                    Value::Str(s) => s.contains("::") || vm.prog.func_index.contains_key(&s.to_ascii_lowercase()) || exists(&s.to_ascii_lowercase()),
                    Value::Array(x) if x.len() == 2 => match (x.get(&Key::Int(0)), x.get(&Key::Int(1))) {
                        (Some(c), Some(m)) => class_name_arg(vm, c).is_some_and(|c| c.find_method(&m.to_str().to_ascii_lowercase()).is_some()),
                        _ => false,
                    },
                    _ => false,
                };
                bool_v(ok)
            }),
            ("array_map", |vm, a| {
                let cb = arg(&a, 0);
                let first = need_arr(&a, 1, "array_map")?;
                if a.len() <= 2 {
                    let mut out = PhpArray::new();
                    let list_like = first.iter().all(|(k, _)| matches!(k, Key::Int(_)));
                    for (k, v) in first.iter() {
                        let r = if matches!(cb, Value::Null) { v.clone() } else { vm.call_value(&cb, vec![v.clone()])? };
                        if list_like || true {
                            out.set(k.clone(), r);
                        }
                    }
                    return Ok(Value::Array(Rc::new(out)));
                }
                let lists: Vec<Vec<Value>> = (1..a.len()).map(|i| need_arr(&a, i, "array_map").map(|x| x.iter().map(|(_, v)| v.clone()).collect())).collect::<Result<_, _>>()?;
                let n = lists.iter().map(Vec::len).max().unwrap_or(0);
                let mut out = vec![];
                for i in 0..n {
                    let row: Vec<Value> = lists.iter().map(|l| l.get(i).cloned().unwrap_or(Value::Null)).collect();
                    out.push(if matches!(cb, Value::Null) { arr(row) } else { vm.call_value(&cb, row)? });
                }
                Ok(arr(out))
            }),
            ("array_filter", |vm, a| {
                let src = need_arr(&a, 0, "array_filter")?;
                let mode = a.get(2).map(Value::to_int).unwrap_or(0);
                let mut out = PhpArray::new();
                for (k, v) in src.iter() {
                    let keep = match a.get(1) {
                        None | Some(Value::Null) => v.to_bool(),
                        Some(cb) => {
                            let args = match mode {
                                2 => vec![k.to_value()],
                                1 => vec![v.clone(), k.to_value()],
                                _ => vec![v.clone()],
                            };
                            vm.call_value(cb, args)?.to_bool()
                        }
                    };
                    if keep {
                        out.set(k.clone(), v.clone());
                    }
                }
                Ok(Value::Array(Rc::new(out)))
            }),
            ("array_reduce", |vm, a| {
                let src = need_arr(&a, 0, "array_reduce")?;
                let cb = arg(&a, 1);
                let mut acc = arg(&a, 2);
                for (_, v) in src.iter() {
                    acc = vm.call_value(&cb, vec![acc, v.clone()])?;
                }
                Ok(acc)
            }),
            ("iterator_to_array", |vm, a| {
                let keep = a.get(1).map(Value::to_bool).unwrap_or(true);
                let src = arg(&a, 0);
                if let Value::Array(_) = src {
                    return Ok(src);
                }
                let mut it = src;
                while matches!(&it, Value::Object(o) if o.class.is_a("iteratoraggregate")) {
                    it = vm.call_method_named(&it, "getIterator", vec![])?;
                }
                let mut out = PhpArray::new();
                vm.call_method_named(&it, "rewind", vec![])?;
                while vm.call_method_named(&it, "valid", vec![])?.to_bool() {
                    let v = vm.call_method_named(&it, "current", vec![])?;
                    if keep {
                        let k = vm.call_method_named(&it, "key", vec![])?;
                        out.set(Key::from_value(&k)?, v);
                    } else {
                        out.push(v);
                    }
                    vm.call_method_named(&it, "next", vec![])?;
                }
                Ok(Value::Array(Rc::new(out)))
            }),
            ("current", |_, a| match arg(&a, 0) {
                Value::Array(x) => Ok(x.entry_at(x.ptr).map(|(_, _, v)| v.clone()).unwrap_or(Value::Bool(false))),
                _ => Ok(Value::Bool(false)),
            }),
            ("key", |_, a| match arg(&a, 0) {
                Value::Array(x) => Ok(x.entry_at(x.ptr).map(|(_, k, _)| k.to_value()).unwrap_or(Value::Null)),
                _ => Ok(Value::Null),
            }),
            ("array_fill_keys", |_, a| {
                let mut out = PhpArray::new();
                for (_, k) in need_arr(&a, 0, "array_fill_keys")?.iter() {
                    out.set(Key::from_value(k)?, arg(&a, 1));
                }
                Ok(Value::Array(Rc::new(out)))
            }),
            ("assert", |_, _| bool_v(true)),
            ("spl_autoload_register", |_, _| bool_v(true)),
            ("get_called_class", |_, _| bool_v(false)),
            ("ob_start", |vm, _| {
                vm.ob.push(String::new());
                bool_v(true)
            }),
            ("ob_get_clean", |vm, _| Ok(vm.ob.pop().map(|b| s(&b)).unwrap_or(Value::Bool(false)))),
            ("ob_get_contents", |vm, _| Ok(vm.ob.last().map(|b| s(b)).unwrap_or(Value::Bool(false)))),
            ("ob_get_length", |vm, _| Ok(vm.ob.last().map(|b| Value::Int(b.len() as i64)).unwrap_or(Value::Bool(false)))),
            ("ob_get_level", |vm, _| Ok(Value::Int(vm.ob.len() as i64))),
            ("ob_end_clean", |vm, _| bool_v(vm.ob.pop().is_some())),
            ("ob_clean", |vm, _| {
                if let Some(b) = vm.ob.last_mut() {
                    b.clear();
                }
                bool_v(true)
            }),
            ("ob_end_flush", |vm, _| match vm.ob.pop() {
                Some(b) => {
                    vm.write(&b);
                    bool_v(true)
                }
                None => bool_v(false),
            }),
            ("ob_get_flush", |vm, _| match vm.ob.pop() {
                Some(b) => {
                    vm.write(&b);
                    Ok(s(&b))
                }
                None => bool_v(false),
            }),
            ("ob_flush", |vm, _| {
                if let Some(b) = vm.ob.pop() {
                    vm.write(&b);
                    vm.ob.push(String::new());
                }
                bool_v(true)
            }),
            ("flush", |_, _| Ok(Value::Null)),
            ("ob_implicit_flush", |_, _| Ok(Value::Null)),
            ("get_declared_classes", |vm, _| {
                let mut names: Vec<String> = vm.classes.values().filter(|c| c.kind == crate::ast::ClassKind::Class).map(|c| c.name.clone()).collect();
                names.sort();
                Ok(arr(names.iter().map(|n| s(n)).collect()))
            }),
            ("get_declared_traits", |vm, _| {
                let mut names: Vec<String> = vm.classes.values().filter(|c| c.kind == crate::ast::ClassKind::Trait).map(|c| c.name.clone()).collect();
                names.sort();
                Ok(arr(names.iter().map(|n| s(n)).collect()))
            }),
            ("trait_exists", |vm, a| bool_v(vm.classes.get(&arg(&a, 0).to_str().to_ascii_lowercase()).is_some_and(|c| c.kind == crate::ast::ClassKind::Trait))),
            ("debug_backtrace", |_, _| Ok(arr(vec![]))),
            ("debug_print_backtrace", |_, _| Ok(Value::Null)),
            ("gc_collect_cycles", |_, _| Ok(Value::Int(0))),
            ("gc_enable", |_, _| Ok(Value::Null)),
            ("gc_disable", |_, _| Ok(Value::Null)),
            ("gc_enabled", |_, _| bool_v(true)),
            ("gc_mem_caches", |_, _| Ok(Value::Int(0))),
            ("class_alias", |vm, a| {
                let orig = vm.find_class(&arg(&a, 0).to_str())?;
                vm.classes.insert(arg(&a, 1).to_str().to_ascii_lowercase(), orig);
                bool_v(true)
            }),
            ("set_exception_handler", |vm, a| {
                let old = vm.exception_handler.take().unwrap_or(Value::Null);
                vm.exception_handler = match arg(&a, 0) {
                    Value::Null => None,
                    h => Some(h),
                };
                Ok(old)
            }),
            ("restore_exception_handler", |vm, _| {
                vm.exception_handler = None;
                bool_v(true)
            }),
            ("set_error_handler", |vm, a| {
                let old = vm.error_handler.take().unwrap_or(Value::Null);
                vm.error_handler = match arg(&a, 0) {
                    Value::Null => None,
                    h => Some(h),
                };
                Ok(old)
            }),
            ("restore_error_handler", |vm, _| {
                vm.error_handler = None;
                bool_v(true)
            }),
            ("register_shutdown_function", |vm, a| {
                let mut it = a.into_iter();
                let f = it.next().unwrap_or(Value::Null);
                vm.shutdown.push((f, it.collect()));
                Ok(Value::Null)
            }),
            ("get_class_vars", |vm, a| {
                let class = vm.find_class(&arg(&a, 0).to_str())?;
                let mut out = PhpArray::new();
                for p in class.props.iter().filter(|p| p.vis == 0) {
                    let v = match p.thunk {
                        Some(t) => {
                            let owner = vm.find_class(&p.class).unwrap_or_else(|_| class.clone());
                            vm.call_thunk(t, &owner)?
                        }
                        None => Value::Null,
                    };
                    out.set(Key::from_str(&p.name), v);
                }
                Ok(Value::Array(Rc::new(out)))
            }),
            ("get_class_methods", |vm, a| {
                let class = vm.class_of(&arg(&a, 0))?;
                let mut names: Vec<(usize, String)> = class.methods.values().filter(|m| m.vis == 0).map(|m| (m.func, m.name.clone())).collect();
                names.sort();
                Ok(arr(names.into_iter().map(|(_, n)| s(&n)).collect()))
            }),
            ("is_iterable", |_, a| bool_v(match arg(&a, 0) {
                Value::Array(_) => true,
                Value::Object(o) => o.class.is_a("traversable"),
                _ => false,
            })),
            ("is_countable", |_, a| bool_v(match arg(&a, 0) {
                Value::Array(_) => true,
                Value::Object(o) => o.class.is_a("countable"),
                _ => false,
            })),
            ("is_object", |_, a| bool_v(matches!(arg(&a, 0), Value::Object(_)))),
            ("get_class", |vm, a| match arg(&a, 0) {
                Value::Object(o) => Ok(s(&o.class.name)),
                _ => vm.throw_error("TypeError", "get_class(): Argument #1 ($object) must be of type object"),
            }),
            ("get_parent_class", |vm, a| {
                Ok(class_name_arg(vm, &arg(&a, 0)).and_then(|c| c.parent.clone()).map(|p| s(&p.name)).unwrap_or(Value::Bool(false)))
            }),
            ("method_exists", |vm, a| {
                bool_v(class_name_arg(vm, &arg(&a, 0)).is_some_and(|c| c.find_method(&arg(&a, 1).to_str().to_ascii_lowercase()).is_some()))
            }),
            ("property_exists", |vm, a| {
                let name = arg(&a, 1).to_str();
                if let Value::Object(o) = arg(&a, 0) {
                    if o.props.borrow().get(&Key::from_str(&name)).is_some() {
                        return bool_v(true);
                    }
                }
                bool_v(class_name_arg(vm, &arg(&a, 0)).is_some_and(|c| c.props.iter().any(|p| *p.name == *name) || c.static_thunks.iter().any(|(n, _)| **n == *name)))
            }),
            ("class_exists", |vm, a| bool_v(vm.classes.get(&arg(&a, 0).to_str().to_ascii_lowercase()).is_some_and(|c| c.kind == crate::ast::ClassKind::Class))),
            ("interface_exists", |vm, a| bool_v(vm.classes.get(&arg(&a, 0).to_str().to_ascii_lowercase()).is_some_and(|c| c.kind == crate::ast::ClassKind::Interface))),
            ("is_a", |vm, a| {
                let target = arg(&a, 1).to_str().to_ascii_lowercase();
                let ok = match arg(&a, 0) {
                    Value::Object(o) => o.class.is_a(&target),
                    Value::Str(_) if a.get(2).is_some_and(Value::to_bool) => class_name_arg(vm, &arg(&a, 0)).is_some_and(|c| c.is_a(&target)),
                    _ => false,
                };
                bool_v(ok)
            }),
            ("is_subclass_of", |vm, a| {
                let target = arg(&a, 1).to_str().to_ascii_lowercase();
                bool_v(class_name_arg(vm, &arg(&a, 0)).is_some_and(|c| !c.name.eq_ignore_ascii_case(&target) && c.is_a(&target)))
            }),
            ("spl_object_id", |_, a| match arg(&a, 0) {
                Value::Object(o) => Ok(Value::Int(o.id as i64)),
                _ => Ok(Value::Int(0)),
            }),
            ("spl_object_hash", |_, a| match arg(&a, 0) {
                Value::Object(o) => Ok(s(&format!("{:032x}", o.id))),
                _ => Ok(s("")),
            }),
            ("get_object_vars", |_, a| match arg(&a, 0) {
                Value::Object(o) => Ok(Value::Array(Rc::new(o.props.borrow().clone()))),
                _ => Ok(Value::Null),
            }),
            ("define", |vm, a| {
                vm.constants.insert(arg(&a, 0).to_str().to_string(), arg(&a, 1));
                bool_v(true)
            }),
            ("defined", |vm, a| {
                let n = arg(&a, 0).to_str();
                bool_v(vm.constants.contains_key(&*n) || constant(&n).is_some())
            }),
            ("constant", |vm, a| {
                let n = arg(&a, 0).to_str();
                if let Some((c, k)) = n.split_once("::") {
                    let class = vm.find_class(c)?;
                    return vm.class_const_pub(&class, k);
                }
                match vm.constants.get(&*n).cloned().or_else(|| constant(&n)) {
                    Some(v) => Ok(v),
                    None => vm.throw_error("Error", &format!("Undefined constant \"{n}\"")),
                }
            }),
        ];
        for (k, f) in list {
            all.insert(k, *f);
        }
        crate::port_string::register_vm(&mut all);
        crate::port_array::register_vm(&mut all);
        crate::port_var::register_vm(&mut all);
        crate::port_hash::register_vm(&mut all);
        crate::port_misc::register_vm(&mut all);
        all
    })
}
