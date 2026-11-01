//! Builtins that modify their arguments in place (`sort`, `array_push`, `reset`, ...).
//! They receive the argument vector mutably; the compiler stores the changed values back.
use crate::value::*;
use crate::vm::{Flow, Vm};
use std::{cmp::Ordering, rc::Rc};

type R<T> = Result<T, Flow>;

pub fn ref_positions(name: &str) -> &'static [usize] {
    match name {
        "sort" | "rsort" | "usort" | "uasort" | "uksort" | "asort" | "arsort" | "ksort" | "krsort" | "shuffle" | "array_push" | "array_pop"
        | "array_shift" | "array_unshift" | "array_splice" | "reset" | "end" | "next" | "prev" | "array_walk" | "natsort" | "natcasesort" | "array_walk_recursive" => &[0],
        "array_multisort" => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        "preg_match" | "preg_match_all" => &[2],
        "preg_replace" | "preg_replace_callback" => &[4],
        "str_replace" | "str_ireplace" => &[3],
        "settype" => &[0],
        _ => &[],
    }
}

fn merge_sort<T: Clone>(mut v: Vec<T>, cmp: &mut dyn FnMut(&T, &T) -> R<Ordering>) -> R<Vec<T>> {
    let n = v.len();
    if n <= 1 {
        return Ok(v);
    }
    let right = v.split_off(n / 2);
    let l = merge_sort(v, cmp)?;
    let r = merge_sort(right, cmp)?;
    let mut out = Vec::with_capacity(n);
    let (mut i, mut j) = (0, 0);
    while i < l.len() && j < r.len() {
        if cmp(&r[j], &l[i])?.is_lt() {
            out.push(r[j].clone());
            j += 1;
        } else {
            out.push(l[i].clone());
            i += 1;
        }
    }
    out.extend_from_slice(&l[i..]);
    out.extend_from_slice(&r[j..]);
    Ok(out)
}

fn entries(v: &Value) -> Option<Vec<(Key, Value)>> {
    match v {
        Value::Array(a) => Some(a.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        _ => None,
    }
}

fn build(items: Vec<(Key, Value)>, renumber: bool) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in items {
        match k {
            Key::Int(_) if renumber => a.push(v),
            k => a.set(k, v),
        }
    }
    Value::Array(Rc::new(a))
}

fn flag_cmp(flags: i64) -> impl Fn(&Value, &Value) -> Ordering {
    move |a, b| match flags & 7 {
        1 => a.to_float().partial_cmp(&b.to_float()).unwrap_or(Ordering::Equal),
        2 => a.to_str().as_bytes().cmp(b.to_str().as_bytes()),
        _ => loose_cmp(a, b),
    }
}

fn type_err<T>(vm: &mut Vm<'_>, f: &str, v: &Value) -> R<T> {
    vm.throw_error("TypeError", &format!("{f}(): Argument #1 ($array) must be of type array, {} given", v.type_name()))
}

pub fn call_ref(vm: &mut Vm<'_>, name: &str, args: &mut Vec<Value>) -> Option<R<Value>> {
    if matches!(name, "preg_match" | "preg_match_all" | "preg_replace" | "preg_replace_callback") {
        return crate::stdlib_preg::call(vm, name, args);
    }
    if matches!(name, "settype" | "str_replace" | "str_ireplace") {
        return call_other(vm, name, args);
    }
    if let Some(r) = crate::port_array::refs::call(vm, name, args) {
        return Some(r);
    }
    let first = args.first().cloned().unwrap_or(Value::Null);
    let Some(items) = entries(&first) else {
        return Some(match name {
            "settype" | "preg_match" | "preg_match_all" | "str_replace" | "str_ireplace" => return call_other(vm, name, args),
            _ => type_err(vm, name, &first),
        });
    };
    let flags = args.get(1).map(Value::to_int).unwrap_or(0);
    let cb = args.get(1).cloned().unwrap_or(Value::Null);
    let sorted = |renumber: bool, by_key: bool, reverse: bool, items: Vec<(Key, Value)>| -> R<Value> {
        let cmp = flag_cmp(flags);
        let out = merge_sort(items, &mut |a, b| {
            let (x, y) = if by_key { (a.0.to_value(), b.0.to_value()) } else { (a.1.clone(), b.1.clone()) };
            let o = cmp(&x, &y);
            Ok(if reverse { o.reverse() } else { o })
        })?;
        Ok(build(out, renumber))
    };
    let user_sorted = |vm: &mut Vm<'_>, renumber: bool, by_key: bool, items: Vec<(Key, Value)>| -> R<Value> {
        let out = merge_sort(items, &mut |a, b| {
            let (x, y) = if by_key { (a.0.to_value(), b.0.to_value()) } else { (a.1.clone(), b.1.clone()) };
            Ok(vm.call_value(&cb, vec![x, y])?.to_int().cmp(&0))
        })?;
        Ok(build(out, renumber))
    };
    let r: R<Value> = (|| {
        match name {
            "sort" => args[0] = sorted(true, false, false, items)?,
            "rsort" => args[0] = sorted(true, false, true, items)?,
            "asort" => args[0] = sorted(false, false, false, items)?,
            "arsort" => args[0] = sorted(false, false, true, items)?,
            "ksort" => args[0] = sorted(false, true, false, items)?,
            "krsort" => args[0] = sorted(false, true, true, items)?,
            "usort" => args[0] = user_sorted(vm, true, false, items)?,
            "uasort" => args[0] = user_sorted(vm, false, false, items)?,
            "uksort" => args[0] = user_sorted(vm, false, true, items)?,
            "natsort" | "natcasesort" => args[0] = sorted(false, false, false, items)?,
            "shuffle" => {
                let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1;
                let mut vals: Vec<Value> = items.into_iter().map(|(_, v)| v).collect();
                for i in (1..vals.len()).rev() {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    vals.swap(i, (seed % (i as u64 + 1)) as usize);
                }
                args[0] = Value::Array(Rc::new(PhpArray::from_list(vals)));
            }
            "array_push" => {
                let mut a = PhpArray::new();
                for (k, v) in items {
                    a.set(k, v);
                }
                for v in args.iter().skip(1) {
                    a.push(v.clone());
                }
                let n = a.len() as i64;
                args[0] = Value::Array(Rc::new(a));
                return Ok(Value::Int(n));
            }
            "array_pop" => {
                let mut items = items;
                let last = items.pop();
                // Reset the next free index like PHP does after a pop.
                args[0] = build(items, false);
                return Ok(last.map(|(_, v)| v).unwrap_or(Value::Null));
            }
            "array_shift" => {
                let mut items = items;
                if items.is_empty() {
                    return Ok(Value::Null);
                }
                let (_, v) = items.remove(0);
                args[0] = build(items, true);
                return Ok(v);
            }
            "array_unshift" => {
                let mut all: Vec<(Key, Value)> = args.iter().skip(1).map(|v| (Key::Int(0), v.clone())).collect();
                all.extend(items);
                let n = all.len() as i64;
                args[0] = build(all, true);
                return Ok(Value::Int(n));
            }
            "array_splice" => {
                let n = items.len() as i64;
                let mut off = args.get(1).map(Value::to_int).unwrap_or(0);
                if off < 0 {
                    off = (n + off).max(0);
                }
                let off = off.min(n) as usize;
                let end = match args.get(2) {
                    None | Some(Value::Null) => n as usize,
                    Some(l) => {
                        let l = l.to_int();
                        if l < 0 { ((n + l).max(off as i64)) as usize } else { (off + l as usize).min(n as usize) }
                    }
                };
                let repl: Vec<Value> = match args.get(3) {
                    Some(Value::Array(a)) => a.iter().map(|(_, v)| v.clone()).collect(),
                    Some(Value::Null) | None => vec![],
                    Some(v) => vec![v.clone()],
                };
                let removed: Vec<Value> = items[off..end].iter().map(|(_, v)| v.clone()).collect();
                let mut out: Vec<(Key, Value)> = items[..off].to_vec();
                out.extend(repl.into_iter().map(|v| (Key::Int(0), v)));
                out.extend(items[end..].iter().cloned());
                args[0] = build(out, true);
                return Ok(Value::Array(Rc::new(PhpArray::from_list(removed))));
            }
            "reset" | "end" | "next" | "prev" => {
                let Value::Array(rc) = &mut args[0] else { unreachable!() };
                let arr = Rc::make_mut(rc);
                let live: Vec<usize> = (0..).take_while(|i| arr.key_at(*i).is_some() || *i < arr.slot_count()).filter(|i| arr.key_at(*i).is_some()).collect();
                let cur = live.iter().position(|s| *s == arr.ptr);
                let target = match name {
                    "reset" => live.first().copied(),
                    "end" => live.last().copied(),
                    "next" => cur.and_then(|c| live.get(c + 1)).copied(),
                    _ => cur.and_then(|c| c.checked_sub(1)).and_then(|c| live.get(c)).copied(),
                };
                return Ok(match target {
                    Some(t) => {
                        arr.ptr = t;
                        arr.entry_at(t).map(|(_, _, v)| v.clone()).unwrap_or(Value::Bool(false))
                    }
                    None => {
                        if matches!(name, "next" | "prev") {
                            arr.ptr = arr.slot_count();
                        }
                        Value::Bool(false)
                    }
                });
            }
            "array_walk" => {
                let cb = args.get(1).cloned().unwrap_or(Value::Null);
                let extra = args.get(2).cloned();
                let mut out = PhpArray::new();
                for (k, v) in items {
                    let cell = Rc::new(std::cell::RefCell::new(v));
                    let mut cargs = vec![Value::Ref(cell.clone()), k.to_value()];
                    cargs.extend(extra.clone());
                    vm.call_value(&cb, cargs)?;
                    out.set(k, cell.borrow().clone());
                }
                args[0] = Value::Array(Rc::new(out));
                return Ok(Value::Bool(true));
            }
            _ => return Err(Flow::Fatal(format!("Call to undefined function {name}()"))),
        }
        Ok(Value::Bool(true))
    })();
    Some(r)
}

fn call_other(vm: &mut Vm<'_>, name: &str, args: &mut Vec<Value>) -> Option<R<Value>> {
    match name {
        "settype" => {
            let ty = args.get(1).map(|t| t.to_str().to_ascii_lowercase()).unwrap_or_default();
            let v = args[0].clone();
            args[0] = match ty.as_str() {
                "int" | "integer" => Value::Int(v.to_int()),
                "float" | "double" => Value::Float(v.to_float()),
                "string" => Value::Str(v.to_str()),
                "bool" | "boolean" => Value::Bool(v.to_bool()),
                "null" => Value::Null,
                "array" => match v {
                    Value::Array(_) => v,
                    Value::Null => Value::Array(Rc::new(PhpArray::new())),
                    other => Value::Array(Rc::new(PhpArray::from_list(vec![other]))),
                },
                _ => return Some(Ok(Value::Bool(false))),
            };
            Some(Ok(Value::Bool(true)))
        }
        "str_replace" | "str_ireplace" => {
            let _ = vm;
            let ci = name == "str_ireplace";
            let mut count = 0i64;
            let search = args.first().cloned().unwrap_or(Value::Null);
            let repl = args.get(1).cloned().unwrap_or(Value::Null);
            let subject = args.get(2).cloned().unwrap_or(Value::Null);
            let pairs: Vec<(String, String)> = match (&search, &repl) {
                (Value::Array(se), Value::Array(re)) => {
                    let reps: Vec<_> = re.iter().map(|(_, v)| v.to_str().to_string()).collect();
                    se.iter().enumerate().map(|(i, (_, v))| (v.to_str().to_string(), reps.get(i).cloned().unwrap_or_default())).collect()
                }
                (Value::Array(se), r) => se.iter().map(|(_, v)| (v.to_str().to_string(), r.to_str().to_string())).collect(),
                (se, r) => vec![(se.to_str().to_string(), r.to_str().to_string())],
            };
            let mut apply = |text: String| -> String {
                let mut t = text;
                for (from, to) in &pairs {
                    if from.is_empty() {
                        continue;
                    }
                    if ci {
                        let (lt, lf) = (t.to_ascii_lowercase(), from.to_ascii_lowercase());
                        let mut out = String::new();
                        let mut last = 0;
                        while let Some(i) = lt[last..].find(&lf) {
                            out.push_str(&t[last..last + i]);
                            out.push_str(to);
                            last += i + from.len();
                            count += 1;
                        }
                        out.push_str(&t[last..]);
                        t = out;
                    } else {
                        count += t.matches(from.as_str()).count() as i64;
                        t = t.replace(from.as_str(), to);
                    }
                }
                t
            };
            let result = match &subject {
                Value::Array(a) => {
                    let mut out = PhpArray::new();
                    for (k, v) in a.iter() {
                        out.set(k.clone(), Value::str(&apply(v.to_str().to_string())));
                    }
                    Value::Array(Rc::new(out))
                }
                other => Value::str(&apply(other.to_str().to_string())),
            };
            while args.len() < 4 {
                args.push(Value::Null);
            }
            args[3] = Value::Int(count);
            Some(Ok(result))
        }
        _ => Some(Err(Flow::Fatal(format!("Call to undefined function {name}()")))),
    }
}
