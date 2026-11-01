//! Array, math, date and environment builtins.
use crate::builtins::{arg, arr, need_arr, s, Builtin};
use crate::value::*;
use std::{cell::Cell, collections::HashMap, rc::Rc};

thread_local! {
    static RNG: Cell<u64> = const { Cell::new(0x2545F4914F6CDD1D) };
}

fn next_rand() -> u64 {
    RNG.with(|r| {
        let mut x = r.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        r.set(x);
        x
    })
}

fn rand_range(lo: i64, hi: i64) -> i64 {
    if hi <= lo {
        return lo;
    }
    lo + (next_rand() % ((hi - lo) as u64 + 1)) as i64
}

fn f1(a: &[Value], f: fn(f64) -> f64) -> Result<Value, String> {
    Ok(Value::Float(f(arg(a, 0).to_float())))
}

fn days_to_civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// PHP `date()` in UTC.
fn format_date(fmt: &str, ts: i64) -> String {
    let days = ts.div_euclid(86400);
    let secs = ts.rem_euclid(86400);
    let (y, m, d) = days_to_civil(days);
    let (h, mi, sec) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let dow = (days + 4).rem_euclid(7) as usize;
    let doy = (1..m).map(|mm| [31, if is_leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][mm as usize - 1]).sum::<i64>() + d - 1;
    let mut out = String::new();
    let mut esc = false;
    for c in fmt.chars() {
        if esc {
            out.push(c);
            esc = false;
            continue;
        }
        match c {
            '\\' => esc = true,
            'd' => out.push_str(&format!("{d:02}")),
            'j' => out.push_str(&d.to_string()),
            'D' => out.push_str(&DAYS[dow][..3]),
            'l' => out.push_str(DAYS[dow]),
            'N' => out.push_str(&(if dow == 0 { 7 } else { dow }).to_string()),
            'w' => out.push_str(&dow.to_string()),
            'z' => out.push_str(&doy.to_string()),
            'm' => out.push_str(&format!("{m:02}")),
            'n' => out.push_str(&m.to_string()),
            'M' => out.push_str(&MONTHS[m as usize - 1][..3]),
            'F' => out.push_str(MONTHS[m as usize - 1]),
            't' => out.push_str(&[31, if is_leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m as usize - 1].to_string()),
            'L' => out.push(if is_leap(y) { '1' } else { '0' }),
            'Y' | 'o' => out.push_str(&y.to_string()),
            'y' => out.push_str(&format!("{:02}", y % 100)),
            'a' => out.push_str(if h < 12 { "am" } else { "pm" }),
            'A' => out.push_str(if h < 12 { "AM" } else { "PM" }),
            'g' => out.push_str(&(if h % 12 == 0 { 12 } else { h % 12 }).to_string()),
            'G' => out.push_str(&h.to_string()),
            'h' => out.push_str(&format!("{:02}", if h % 12 == 0 { 12 } else { h % 12 })),
            'H' => out.push_str(&format!("{h:02}")),
            'i' => out.push_str(&format!("{mi:02}")),
            's' => out.push_str(&format!("{sec:02}")),
            'u' => out.push_str("000000"),
            'v' => out.push_str("000"),
            'e' | 'T' => out.push_str("UTC"),
            'P' => out.push_str("+00:00"),
            'O' => out.push_str("+0000"),
            'Z' | 'I' => out.push('0'),
            'U' => out.push_str(&ts.to_string()),
            'c' => out.push_str(&format_date("Y-m-d\\TH:i:sP", ts)),
            'r' => out.push_str(&format_date("D, d M Y H:i:s O", ts)),
            c => out.push(c),
        }
    }
    out
}

fn set_op(a: &[Value], name: &str, by_key: bool, keep_if_found: bool, assoc: bool) -> Result<Value, String> {
    let base = need_arr(a, 0, name)?;
    let others: Vec<Rc<PhpArray>> = (1..a.len()).map(|i| need_arr(a, i, name)).collect::<Result<_, _>>()?;
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        let found_in = |o: &Rc<PhpArray>| -> bool {
            if by_key {
                o.get(k).is_some_and(|x| !assoc || x.to_str() == v.to_str())
            } else if assoc {
                o.get(k).is_some_and(|x| x.to_str() == v.to_str())
            } else {
                o.iter().any(|(_, x)| x.to_str() == v.to_str())
            }
        };
        let keep = if keep_if_found { others.iter().all(found_in) } else { !others.iter().any(found_in) };
        if keep {
            out.set(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let list: &[(&str, Builtin)] = &[
        ("array_diff", |_, a| set_op(&a, "array_diff", false, false, false)),
        ("array_diff_key", |_, a| set_op(&a, "array_diff_key", true, false, false)),
        ("array_diff_assoc", |_, a| set_op(&a, "array_diff_assoc", true, false, true)),
        ("array_intersect", |_, a| set_op(&a, "array_intersect", false, true, false)),
        ("array_intersect_key", |_, a| set_op(&a, "array_intersect_key", true, true, false)),
        ("array_intersect_assoc", |_, a| set_op(&a, "array_intersect_assoc", true, true, true)),
        ("array_replace", |_, a| {
            let mut out = (*need_arr(&a, 0, "array_replace")?).clone();
            for i in 1..a.len() {
                for (k, v) in need_arr(&a, i, "array_replace")?.iter() {
                    out.set(k.clone(), v.clone());
                }
            }
            Ok(Value::Array(Rc::new(out)))
        }),
        ("array_column", |_, a| {
            let col = arg(&a, 1);
            let idx = arg(&a, 2);
            let mut out = PhpArray::new();
            for (_, row) in need_arr(&a, 0, "array_column")?.iter() {
                let get = |key: &Value| -> Option<Value> {
                    match row {
                        Value::Array(r) => Key::from_value(key).ok().and_then(|k| r.get(&k).cloned()),
                        Value::Object(o) => Key::from_value(key).ok().and_then(|k| o.props.borrow().get(&k).cloned()),
                        _ => None,
                    }
                };
                let v = if matches!(col, Value::Null) { Some(row.clone()) } else { get(&col) };
                if let Some(v) = v {
                    match (&idx, get(&idx)) {
                        (Value::Null, _) | (_, None) => out.push(v),
                        (_, Some(k)) => out.set(Key::from_value(&k).map_err(|e| e.to_string())?, v),
                    }
                }
            }
            Ok(Value::Array(Rc::new(out)))
        }),
        ("array_chunk", |_, a| {
            let n = arg(&a, 1).to_int();
            if n < 1 {
                return Err("array_chunk(): Argument #2 ($length) must be greater than 0".into());
            }
            let keep = arg(&a, 2).to_bool();
            let items: Vec<(Key, Value)> = need_arr(&a, 0, "array_chunk")?.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let chunks = items
                .chunks(n as usize)
                .map(|c| {
                    let mut p = PhpArray::new();
                    for (k, v) in c {
                        if keep { p.set(k.clone(), v.clone()) } else { p.push(v.clone()) }
                    }
                    Value::Array(Rc::new(p))
                })
                .collect();
            Ok(arr(chunks))
        }),
        ("array_pad", |_, a| {
            let src: Vec<Value> = need_arr(&a, 0, "array_pad")?.iter().map(|(_, v)| v.clone()).collect();
            let n = arg(&a, 1).to_int();
            let missing = (n.unsigned_abs() as usize).saturating_sub(src.len());
            let pad = vec![arg(&a, 2); missing];
            Ok(arr(if n >= 0 { [src, pad].concat() } else { [pad, src].concat() }))
        }),
        ("array_rand", |_, a| {
            let src = need_arr(&a, 0, "array_rand")?;
            if src.is_empty() {
                return Err("array_rand(): Argument #1 ($array) cannot be empty".into());
            }
            let i = rand_range(0, src.len() as i64 - 1) as usize;
            let key = src.iter().nth(i).map(|(k, _)| k.to_value()).unwrap();
            Ok(key)
        }),
        ("array_count_values", |_, a| {
            let mut out = PhpArray::new();
            for (_, v) in need_arr(&a, 0, "array_count_values")?.iter() {
                let k = Key::from_value(v)?;
                let n = out.get(&k).map(Value::to_int).unwrap_or(0);
                out.set(k, Value::Int(n + 1));
            }
            Ok(Value::Array(Rc::new(out)))
        }),
        ("array_is_list", |_, a| {
            let src = need_arr(&a, 0, "array_is_list")?;
            let ok = src.iter().enumerate().all(|(i, (k, _))| *k == Key::Int(i as i64));
            Ok(Value::Bool(ok))
        }),
        ("array_merge_recursive", |_, a| {
            fn merge(dst: &mut PhpArray, src: &PhpArray) {
                for (k, v) in src.iter() {
                    match k {
                        Key::Int(_) => dst.push(v.clone()),
                        k => match (dst.get(k).cloned(), v) {
                            (Some(Value::Array(x)), Value::Array(y)) => {
                                let mut inner = (*x).clone();
                                merge(&mut inner, y);
                                dst.set(k.clone(), Value::Array(Rc::new(inner)));
                            }
                            (Some(old), new) => {
                                let mut inner = match old {
                                    Value::Array(x) => (*x).clone(),
                                    other => PhpArray::from_list(vec![other]),
                                };
                                match new {
                                    Value::Array(y) => merge(&mut inner, y),
                                    other => inner.push(other.clone()),
                                }
                                dst.set(k.clone(), Value::Array(Rc::new(inner)));
                            }
                            (None, new) => dst.set(k.clone(), new.clone()),
                        },
                    }
                }
            }
            let mut out = PhpArray::new();
            for i in 0..a.len() {
                let src = need_arr(&a, i, "array_merge_recursive")?;
                merge(&mut out, &src);
            }
            Ok(Value::Array(Rc::new(out)))
        }),
        ("iterator_count", |vm, a| match arg(&a, 0) {
            Value::Array(x) => Ok(Value::Int(x.len() as i64)),
            _ => {
                let _ = vm;
                Err("iterator_count(): unsupported argument".into())
            }
        }),
        ("exp", |_, a| f1(&a, f64::exp)),
        ("log10", |_, a| f1(&a, f64::log10)),
        ("log2", |_, a| f1(&a, f64::log2)),
        ("log", |_, a| {
            let x = arg(&a, 0).to_float();
            Ok(Value::Float(match a.get(1) {
                Some(b) => x.ln() / b.to_float().ln(),
                None => x.ln(),
            }))
        }),
        ("sin", |_, a| f1(&a, f64::sin)),
        ("cos", |_, a| f1(&a, f64::cos)),
        ("tan", |_, a| f1(&a, f64::tan)),
        ("asin", |_, a| f1(&a, f64::asin)),
        ("acos", |_, a| f1(&a, f64::acos)),
        ("atan", |_, a| f1(&a, f64::atan)),
        ("sinh", |_, a| f1(&a, f64::sinh)),
        ("cosh", |_, a| f1(&a, f64::cosh)),
        ("tanh", |_, a| f1(&a, f64::tanh)),
        ("atan2", |_, a| Ok(Value::Float(arg(&a, 0).to_float().atan2(arg(&a, 1).to_float())))),
        ("hypot", |_, a| Ok(Value::Float(arg(&a, 0).to_float().hypot(arg(&a, 1).to_float())))),
        ("deg2rad", |_, a| f1(&a, f64::to_radians)),
        ("rad2deg", |_, a| f1(&a, f64::to_degrees)),
        ("is_nan", |_, a| Ok(Value::Bool(arg(&a, 0).to_float().is_nan()))),
        ("is_finite", |_, a| Ok(Value::Bool(arg(&a, 0).to_float().is_finite()))),
        ("is_infinite", |_, a| Ok(Value::Bool(arg(&a, 0).to_float().is_infinite()))),
        ("fdiv", |_, a| Ok(Value::Float(arg(&a, 0).to_float() / arg(&a, 1).to_float()))),
        ("octdec", |_, a| Ok(Value::Int(i64::from_str_radix(&arg(&a, 0).to_str(), 8).unwrap_or(0)))),
        ("base_convert", |_, a| {
            let n = u64::from_str_radix(&arg(&a, 0).to_str().to_lowercase(), arg(&a, 1).to_int() as u32).unwrap_or(0);
            let to = arg(&a, 2).to_int() as u64;
            if n == 0 {
                return Ok(s("0"));
            }
            let (mut n, mut out) = (n, Vec::new());
            while n > 0 {
                out.push(std::char::from_digit((n % to) as u32, to as u32).unwrap());
                n /= to;
            }
            Ok(s(&out.into_iter().rev().collect::<String>()))
        }),
        ("mt_rand", |_, a| Ok(Value::Int(if a.len() >= 2 { rand_range(arg(&a, 0).to_int(), arg(&a, 1).to_int()) } else { (next_rand() >> 33) as i64 }))),
        ("rand", |_, a| Ok(Value::Int(if a.len() >= 2 { rand_range(arg(&a, 0).to_int(), arg(&a, 1).to_int()) } else { (next_rand() >> 33) as i64 }))),
        ("random_int", |_, a| Ok(Value::Int(rand_range(arg(&a, 0).to_int(), arg(&a, 1).to_int())))),
        ("mt_srand", |_, a| {
            RNG.with(|r| r.set((arg(&a, 0).to_int() as u64).wrapping_mul(0x9E3779B97F4A7C15) | 1));
            Ok(Value::Null)
        }),
        ("srand", |_, a| {
            RNG.with(|r| r.set((arg(&a, 0).to_int() as u64).wrapping_mul(0x9E3779B97F4A7C15) | 1));
            Ok(Value::Null)
        }),
        ("mt_getrandmax", |_, _| Ok(Value::Int(2147483647))),
        ("getrandmax", |_, _| Ok(Value::Int(2147483647))),
        ("php_sapi_name", |_, _| Ok(s("cli"))),
        ("phpversion", |_, _| Ok(s("8.4.0"))),
        ("php_uname", |_, _| Ok(s("Linux"))),
        ("memory_get_usage", |_, _| Ok(Value::Int(2097152))),
        ("memory_get_peak_usage", |_, _| Ok(Value::Int(2097152))),
        ("set_time_limit", |_, _| Ok(Value::Bool(true))),
        ("sleep", |_, _| Ok(Value::Int(0))),
        ("usleep", |_, _| Ok(Value::Null)),
        ("error_log", |_, _| Ok(Value::Bool(true))),
        ("ini_get", |_, a| {
            Ok(match &*arg(&a, 0).to_str() {
                "precision" => s("14"),
                "serialize_precision" => s("-1"),
                "memory_limit" => s("128M"),
                "display_errors" => s("1"),
                "error_reporting" => s("32767"),
                "max_execution_time" => s("0"),
                _ => Value::Bool(false),
            })
        }),
        ("getenv", |_, a| {
            Ok(match a.first() {
                Some(n) => std::env::var(&*n.to_str()).map(|v| s(&v)).unwrap_or(Value::Bool(false)),
                None => Value::Array(Rc::new(PhpArray::new())),
            })
        }),
        ("hrtime", |_, a| {
            let ns = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos() as i64;
            Ok(if arg(&a, 0).to_bool() { Value::Int(ns) } else { arr(vec![Value::Int(ns / 1_000_000_000), Value::Int(ns % 1_000_000_000)]) })
        }),
        ("date", |_, a| {
            let ts = a.get(1).filter(|v| !matches!(v, Value::Null)).map(Value::to_int).unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0));
            Ok(s(&format_date(&arg(&a, 0).to_str(), ts)))
        }),
        ("gmdate", |_, a| {
            let ts = a.get(1).filter(|v| !matches!(v, Value::Null)).map(Value::to_int).unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0));
            Ok(s(&format_date(&arg(&a, 0).to_str(), ts)))
        }),
        ("date_default_timezone_set", |_, _| Ok(Value::Bool(true))),
        ("date_default_timezone_get", |_, _| Ok(s("UTC"))),
        ("checkdate", |_, a| {
            let (m, d, y) = (arg(&a, 0).to_int(), arg(&a, 1).to_int(), arg(&a, 2).to_int());
            let dim = [31, if is_leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            Ok(Value::Bool((1..=12).contains(&m) && d >= 1 && d <= dim[(m - 1) as usize]))
        }),
        ("mktime", |_, a| {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
            let (cy, cm, cd) = days_to_civil(now.div_euclid(86400));
            let get = |i: usize, dflt: i64| a.get(i).filter(|v| !matches!(v, Value::Null)).map(Value::to_int).unwrap_or(dflt);
            let (h, mi, sec, mo, d, y) = (get(0, now % 86400 / 3600), get(1, now % 3600 / 60), get(2, now % 60), get(3, cm), get(4, cd), get(5, cy));
            // days from civil
            let (yy, mm) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
            let era = yy.div_euclid(400);
            let yoe = yy.rem_euclid(400);
            let doy = (153 * mm + 2) / 5 + d - 1;
            let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
            let days = era * 146097 + doe - 719468;
            Ok(Value::Int(days * 86400 + h * 3600 + mi * 60 + sec))
        }),
    ];
    for (k, f) in list {
        m.insert(k, *f);
    }
}
