//! Built-in functions and constants.
use crate::value::*;
use crate::vm::Vm;
use std::{collections::HashMap, rc::Rc, sync::OnceLock};

type Res = Result<Value, String>;
type Builtin = fn(&mut Vm<'_>, Vec<Value>) -> Res;

fn arg(a: &[Value], i: usize) -> Value {
    a.get(i).cloned().unwrap_or(Value::Null)
}

fn s(v: &str) -> Value {
    Value::Str(v.into())
}

fn arr(items: Vec<Value>) -> Value {
    Value::Array(Rc::new(PhpArray::from_list(items)))
}

fn need_arr(a: &[Value], i: usize, f: &str) -> Result<Rc<PhpArray>, String> {
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
        "STR_PAD_RIGHT" => Value::Int(1),
        "STR_PAD_LEFT" => Value::Int(0),
        "STR_PAD_BOTH" => Value::Int(2),
        "E_ALL" => Value::Int(32767),
        "E_ERROR" => Value::Int(1),
        "E_WARNING" => Value::Int(2),
        "E_NOTICE" => Value::Int(8),
        "E_STRICT" => Value::Int(2048),
        _ => return None,
    })
}

fn table() -> &'static HashMap<&'static str, Builtin> {
    static T: OnceLock<HashMap<&'static str, Builtin>> = OnceLock::new();
    T.get_or_init(|| {
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
                vm.out.push_str(&r);
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
                vm.out.push_str(&out);
                Ok(Value::Bool(true))
            }),
            ("var_export", |vm, a| {
                let mut out = String::new();
                var_export(&arg(&a, 0), 0, &mut out);
                if a.get(1).is_some_and(Value::to_bool) {
                    return Ok(s(&out));
                }
                vm.out.push_str(&out);
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
        list.iter().copied().collect()
    })
}

pub fn exists(name: &str) -> bool {
    table().contains_key(name)
}

pub fn call(vm: &mut Vm<'_>, name: &str, args: Vec<Value>) -> Option<Res> {
    table().get(name).map(|f| f(vm, args))
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

fn sprintf(a: &[Value]) -> Result<String, String> {
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
        Value::Null => out.push_str(&format!("{pad}NULL\n")),
        Value::Bool(b) => out.push_str(&format!("{pad}bool({b})\n")),
        Value::Int(i) => out.push_str(&format!("{pad}int({i})\n")),
        Value::Float(f) => out.push_str(&format!("{pad}float({})\n", fmt_float(*f, None))),
        Value::Str(t) => out.push_str(&format!("{pad}string({}) \"{t}\"\n", t.len())),
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
