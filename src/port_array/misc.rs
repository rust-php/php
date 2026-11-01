//! Remaining plain array builtins ported from ext/standard/array.c.
use super::args::*;
use super::mt;
use super::setops::{check_callback, is_callable, callback_error};
use super::sort::*;
use crate::builtins::VmBuiltin;
use crate::value::*;
use crate::vm::Vm;
use std::collections::HashMap;
use std::rc::Rc;

fn arr_arg(vm: &mut Vm<'_>, f: &str, n: usize, pname: &str, a: &[Value]) -> R<Rc<PhpArray>> {
    match a.get(n - 1).map(Value::deref) {
        Some(Value::Array(x)) => Ok(x),
        Some(v) => arg_type_err(vm, f, n, pname, "array", &v),
        None => argc_err(vm, f, n, n, a.len()),
    }
}

fn mk(a: PhpArray) -> Value {
    Value::Array(Rc::new(a))
}

/// zend_symtable-style key from a value converted to string (array_fill_keys, array_combine).
fn sym_key(v: &Value) -> Key {
    match v.deref() {
        Value::Int(i) => Key::Int(i),
        v => Key::from_str(&v.to_str()),
    }
}

fn obj_to_array(v: &Value) -> PhpArray {
    match v {
        Value::Object(o) => o.props.borrow().clone(),
        Value::Null => PhpArray::new(),
        v => PhpArray::from_list(vec![v.clone()]),
    }
}

fn f_count(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "count", &a, 1, 2)?;
    let mode = if a.len() > 1 { p_long(vm, "count", 2, "mode", &a[1])? } else { 0 };
    if mode != 0 && mode != 1 {
        return value_err(vm, "count", 2, "mode", "must be either COUNT_NORMAL or COUNT_RECURSIVE");
    }
    match a[0].deref() {
        Value::Array(x) => {
            if mode == 0 {
                return Ok(Value::Int(x.len() as i64));
            }
            fn rec(x: &PhpArray, depth: usize) -> i64 {
                let mut c = x.len() as i64;
                if depth > 512 {
                    return c;
                }
                for (_, v) in x.iter() {
                    if let Value::Array(s) = v.deref() {
                        c += rec(&s, depth + 1);
                    }
                }
                c
            }
            Ok(Value::Int(rec(&x, 0)))
        }
        v @ Value::Object(_) => {
            let Value::Object(o) = &v else { unreachable!() };
            if let Some(m) = o.class.find_method("count") {
                let r = vm.call_method(&v, &m, vec![])?;
                return Ok(Value::Int(r.to_int()));
            }
            arg_type_err(vm, "count", 1, "value", "Countable|array", &v)
        }
        v => arg_type_err(vm, "count", 1, "value", "Countable|array", &v),
    }
}

fn f_current(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "current", &a, 1, 1)?;
    match a[0].deref() {
        Value::Array(x) => Ok(x.entry_at(x.ptr).map(|(_, _, v)| v.deref()).unwrap_or(Value::Bool(false))),
        v => arg_type_err(vm, "current", 1, "array", "array|object", &v),
    }
}

fn f_key(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "key", &a, 1, 1)?;
    match a[0].deref() {
        Value::Array(x) => Ok(x.entry_at(x.ptr).map(|(_, k, _)| k.to_value()).unwrap_or(Value::Null)),
        v => arg_type_err(vm, "key", 1, "array", "array|object", &v),
    }
}

fn minmax(vm: &mut Vm<'_>, name: &str, a: Vec<Value>, is_max: bool) -> R<Value> {
    if a.is_empty() {
        return argc_err(vm, name, 1, usize::MAX, 0);
    }
    let better = |x: &Value, cur: &Value| {
        let o = loose_cmp(x, cur);
        if is_max { o.is_gt() } else { o.is_lt() }
    };
    if a.len() == 1 {
        let Value::Array(x) = a[0].deref() else {
            return arg_type_err(vm, name, 1, "value", "array", &a[0]);
        };
        let mut it = x.iter();
        let Some((_, first)) = it.next() else {
            return value_err(vm, name, 1, "value", "must contain at least one element");
        };
        let mut best = first.deref();
        for (_, v) in it {
            let v = v.deref();
            if better(&v, &best) {
                best = v;
            }
        }
        return Ok(best);
    }
    let mut best = a[0].deref();
    for v in &a[1..] {
        let v = v.deref();
        if better(&v, &best) {
            best = v;
        }
    }
    Ok(best)
}

fn search(vm: &mut Vm<'_>, name: &str, a: &[Value], want_key: bool) -> R<Value> {
    check_argc(vm, name, a, 2, 3)?;
    let h = arr_arg(vm, name, 2, "haystack", a)?;
    let strict = a.get(2).map(|v| v.deref().to_bool()).unwrap_or(false);
    let needle = a[0].deref();
    for (k, v) in h.iter() {
        let v = v.deref();
        let eq = if strict { strict_eq(&needle, &v) } else { loose_eq(&needle, &v) };
        if eq {
            return Ok(if want_key { k.to_value() } else { Value::Bool(true) });
        }
    }
    Ok(Value::Bool(false))
}

fn f_array_fill(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_fill", &a, 3, 3)?;
    let start = p_long(vm, "array_fill", 1, "start_index", &a[0])?;
    let num = p_long(vm, "array_fill", 2, "count", &a[1])?;
    if num > 0 {
        if num > i32::MAX as i64 {
            return value_err(vm, "array_fill", 2, "count", "is too large");
        }
        if start > i64::MAX - num + 1 {
            return vm.throw_error("Error", "Cannot add element to the array as the next element is already occupied");
        }
        let mut out = PhpArray::new();
        out.set(Key::Int(start), a[2].deref());
        for _ in 1..num {
            out.push(a[2].deref());
        }
        Ok(mk(out))
    } else if num == 0 {
        Ok(mk(PhpArray::new()))
    } else {
        value_err(vm, "array_fill", 2, "count", "must be greater than or equal to 0")
    }
}

fn f_array_fill_keys(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_fill_keys", &a, 2, 2)?;
    let keys = arr_arg(vm, "array_fill_keys", 1, "keys", &a)?;
    let mut out = PhpArray::new();
    for (_, k) in keys.iter() {
        let key = sym_key(k);
        if out.get(&key).is_none() {
            out.set(key, a[1].deref());
        }
    }
    Ok(mk(out))
}

fn f_array_keys(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_keys", &a, 1, 3)?;
    let x = arr_arg(vm, "array_keys", 1, "array", &a)?;
    let mut out = PhpArray::new();
    if a.len() > 1 {
        let needle = a[1].deref();
        let strict = a.get(2).map(|v| v.deref().to_bool()).unwrap_or(false);
        for (k, v) in x.iter() {
            let v = v.deref();
            if if strict { strict_eq(&needle, &v) } else { loose_eq(&needle, &v) } {
                out.push(k.to_value());
            }
        }
    } else {
        for (k, _) in x.iter() {
            out.push(k.to_value());
        }
    }
    Ok(mk(out))
}

fn f_array_slice(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_slice", &a, 2, 4)?;
    let x = arr_arg(vm, "array_slice", 1, "array", &a)?;
    let mut offset = p_long(vm, "array_slice", 2, "offset", &a[1])?;
    let num_in = x.len() as i64;
    let mut length = match a.get(2).map(Value::deref) {
        None | Some(Value::Null) => num_in,
        Some(v) => match v {
            Value::Str(ref s) if parse_numeric(s).is_none() => return arg_type_err(vm, "array_slice", 3, "length", "?int", &v),
            Value::Array(_) | Value::Object(_) => return arg_type_err(vm, "array_slice", 3, "length", "?int", &v),
            v => p_long(vm, "array_slice", 3, "length", &v)?,
        },
    };
    let preserve = a.get(3).map(|v| v.deref().to_bool()).unwrap_or(false);
    if offset > num_in {
        return Ok(mk(PhpArray::new()));
    } else if offset < 0 {
        offset += num_in;
        if offset < 0 {
            offset = 0;
        }
    }
    if length < 0 {
        length = num_in - offset + length;
    } else if offset + length > num_in {
        length = num_in - offset;
    }
    if length <= 0 {
        return Ok(mk(PhpArray::new()));
    }
    let mut out = PhpArray::new();
    for (k, v) in x.iter().skip(offset as usize).take(length as usize) {
        match k {
            Key::Int(_) if !preserve => out.push(v.clone()),
            k => out.set(k.clone(), v.clone()),
        }
    }
    Ok(mk(out))
}

fn f_array_merge(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    let mut out = PhpArray::new();
    for (i, v) in a.iter().enumerate() {
        match v.deref() {
            Value::Array(x) => {
                for (k, v) in x.iter() {
                    match k {
                        Key::Int(_) => out.push(v.clone()),
                        k => out.set(k.clone(), v.clone()),
                    }
                }
            }
            v => return vm.throw_error("TypeError", &format!("array_merge(): Argument #{} must be of type array, {} given", i + 1, tname(&v))),
        }
    }
    Ok(mk(out))
}

fn merge_recursive(vm: &mut Vm<'_>, dest: &mut PhpArray, src: &PhpArray, depth: usize) -> R<()> {
    if depth > 256 {
        return vm.throw_error("Error", "Recursion detected");
    }
    for (k, sv) in src.iter() {
        let Key::Str(_) = k else {
            dest.push(sv.clone());
            continue;
        };
        let Some(dv) = dest.get(k).cloned() else {
            dest.set(k.clone(), sv.clone());
            continue;
        };
        let dv = dv.deref();
        let mut target = match &dv {
            Value::Array(x) => (**x).clone(),
            Value::Null => PhpArray::from_list(vec![Value::Null]),
            Value::Object(_) => obj_to_array(&dv),
            v => PhpArray::from_list(vec![v.clone()]),
        };
        match sv.deref() {
            Value::Array(s) => merge_recursive(vm, &mut target, &s, depth + 1)?,
            o @ Value::Object(_) => merge_recursive(vm, &mut target, &obj_to_array(&o), depth + 1)?,
            s => target.push(s),
        }
        dest.set(k.clone(), mk(target));
    }
    Ok(())
}

fn f_array_merge_recursive(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    let mut out = PhpArray::new();
    for (i, v) in a.iter().enumerate() {
        match v.deref() {
            Value::Array(x) => merge_recursive(vm, &mut out, &x, 0)?,
            v => return vm.throw_error("TypeError", &format!("array_merge_recursive(): Argument #{} must be of type array, {} given", i + 1, tname(&v))),
        }
    }
    Ok(mk(out))
}

fn replace_recursive(vm: &mut Vm<'_>, dest: &mut PhpArray, src: &PhpArray, depth: usize) -> R<()> {
    if depth > 256 {
        return vm.throw_error("Error", "Recursion detected");
    }
    for (k, sv) in src.iter() {
        if let (Value::Array(s), Some(Value::Array(d))) = (sv.deref(), dest.get(k).map(Value::deref)) {
            let mut t = (*d).clone();
            replace_recursive(vm, &mut t, &s, depth + 1)?;
            dest.set(k.clone(), mk(t));
        } else {
            dest.set(k.clone(), sv.clone());
        }
    }
    Ok(())
}

fn replace(vm: &mut Vm<'_>, name: &str, a: Vec<Value>, recursive: bool) -> R<Value> {
    if a.is_empty() {
        return argc_err(vm, name, 1, usize::MAX, 0);
    }
    let mut out = match a[0].deref() {
        Value::Array(x) => (*x).clone(),
        v => return arg_type_err(vm, name, 1, "array", "array", &v),
    };
    out.ptr = 0;
    for (i, v) in a.iter().enumerate().skip(1) {
        match v.deref() {
            Value::Array(x) => {
                if recursive {
                    replace_recursive(vm, &mut out, &x, 0)?;
                } else {
                    for (k, v) in x.iter() {
                        out.set(k.clone(), v.clone());
                    }
                }
            }
            v => return vm.throw_error("TypeError", &format!("{name}(): Argument #{} must be of type array, {} given", i + 1, tname(&v))),
        }
    }
    Ok(mk(out))
}

fn f_array_key_exists(vm: &mut Vm<'_>, a: Vec<Value>, name: &str) -> R<Value> {
    check_argc(vm, name, &a, 2, 2)?;
    let x = arr_arg(vm, name, 2, "array", &a)?;
    let key = match a[0].deref() {
        Value::Str(s) => Key::from_str(&s),
        Value::Int(i) => Key::Int(i),
        Value::Null => Key::Str("".into()),
        Value::Float(f) => Key::Int(f as i64),
        Value::Bool(b) => Key::Int(b as i64),
        Value::Resource(r) => Key::Int(r.id),
        _ => return vm.throw_error("TypeError", &format!("{name}(): Argument #1 ($key) must be a valid array offset type")),
    };
    Ok(Value::Bool(x.get(&key).is_some()))
}

fn f_change_key_case(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_change_key_case", &a, 1, 2)?;
    let x = arr_arg(vm, "array_change_key_case", 1, "array", &a)?;
    let case = if a.len() > 1 { p_long(vm, "array_change_key_case", 2, "case", &a[1])? } else { 0 };
    if case != 0 && case != 1 {
        return value_err(vm, "array_change_key_case", 2, "case", "must be either CASE_LOWER or CASE_UPPER");
    }
    let mut out = PhpArray::new();
    for (k, v) in x.iter() {
        match k {
            Key::Int(_) => out.set(k.clone(), v.clone()),
            Key::Str(s) => {
                let n: String = if case == 0 { s.to_ascii_lowercase() } else { s.to_ascii_uppercase() };
                out.set(Key::Str(n.into()), v.clone())
            }
        }
    }
    Ok(mk(out))
}

fn f_count_values(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_count_values", &a, 1, 1)?;
    let x = arr_arg(vm, "array_count_values", 1, "array", &a)?;
    let mut out = PhpArray::new();
    for (_, v) in x.iter() {
        let key = match v.deref() {
            Value::Int(i) => Key::Int(i),
            Value::Str(s) => Key::from_str(&s),
            _ => continue,
        };
        let n = out.get(&key).map(|v| v.to_int()).unwrap_or(0);
        out.set(key, Value::Int(n + 1));
    }
    Ok(mk(out))
}

fn f_combine(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_combine", &a, 2, 2)?;
    let keys = arr_arg(vm, "array_combine", 1, "keys", &a)?;
    let vals = arr_arg(vm, "array_combine", 2, "values", &a)?;
    if keys.len() != vals.len() {
        return vm.throw_error("ValueError", "array_combine(): Argument #1 ($keys) and argument #2 ($values) must have the same number of elements");
    }
    let mut out = PhpArray::new();
    for ((_, k), (_, v)) in keys.iter().zip(vals.iter()) {
        out.set(sym_key(k), v.clone());
    }
    Ok(mk(out))
}

fn f_flip(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_flip", &a, 1, 1)?;
    let x = arr_arg(vm, "array_flip", 1, "array", &a)?;
    let mut out = PhpArray::new();
    for (k, v) in x.iter() {
        match v.deref() {
            Value::Int(i) => out.set(Key::Int(i), k.to_value()),
            Value::Str(s) => out.set(Key::from_str(&s), k.to_value()),
            _ => {}
        }
    }
    Ok(mk(out))
}

fn f_pad(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_pad", &a, 3, 3)?;
    let x = arr_arg(vm, "array_pad", 1, "array", &a)?;
    let pad_size = p_long(vm, "array_pad", 2, "length", &a[1])?;
    let pad_abs = pad_size.unsigned_abs();
    let n = x.len() as u64;
    if pad_abs > i32::MAX as u64 || pad_abs.saturating_sub(n) > 1048576 {
        return value_err(vm, "array_pad", 2, "length", "must not exceed the maximum allowed array size");
    }
    if n >= pad_abs {
        return Ok(mk((*x).clone()));
    }
    let pads = (pad_abs - n) as usize;
    let mut out = PhpArray::new();
    let push_input = |out: &mut PhpArray| {
        for (k, v) in x.iter() {
            match k {
                Key::Int(_) => out.push(v.clone()),
                k => out.set(k.clone(), v.clone()),
            }
        }
    };
    if pad_size < 0 {
        for _ in 0..pads {
            out.push(a[2].deref());
        }
        push_input(&mut out);
    } else {
        push_input(&mut out);
        for _ in 0..pads {
            out.push(a[2].deref());
        }
    }
    Ok(mk(out))
}

fn f_chunk(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_chunk", &a, 2, 3)?;
    let x = arr_arg(vm, "array_chunk", 1, "array", &a)?;
    let size = p_long(vm, "array_chunk", 2, "length", &a[1])?;
    let preserve = a.get(2).map(|v| v.deref().to_bool()).unwrap_or(false);
    if size < 1 {
        return value_err(vm, "array_chunk", 2, "length", "must be greater than 0");
    }
    let mut out = PhpArray::new();
    let mut cur = PhpArray::new();
    for (k, v) in x.iter() {
        if preserve {
            cur.set(k.clone(), v.clone());
        } else {
            cur.push(v.clone());
        }
        if cur.len() as i64 == size {
            out.push(mk(std::mem::take(&mut cur)));
        }
    }
    if !cur.is_empty() {
        out.push(mk(cur));
    }
    Ok(mk(out))
}

fn f_reverse(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_reverse", &a, 1, 2)?;
    let x = arr_arg(vm, "array_reverse", 1, "array", &a)?;
    let preserve = a.get(1).map(|v| v.deref().to_bool()).unwrap_or(false);
    let mut out = PhpArray::new();
    let items: Vec<_> = x.iter().collect();
    for (k, v) in items.into_iter().rev() {
        match k {
            Key::Int(_) if !preserve => out.push(v.clone()),
            k => out.set(k.clone(), v.clone()),
        }
    }
    Ok(mk(out))
}

fn f_unique(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_unique", &a, 1, 2)?;
    let x = arr_arg(vm, "array_unique", 1, "array", &a)?;
    let flags = if a.len() > 1 { p_long(vm, "array_unique", 2, "flags", &a[1])? } else { SORT_STRING };
    let mut out = PhpArray::new();
    if flags == SORT_STRING {
        let mut seen = std::collections::HashSet::new();
        for (k, v) in x.iter() {
            if seen.insert(v.to_str()) {
                out.set(k.clone(), v.clone());
            }
        }
    } else {
        let cmp = data_cmp(flags);
        let mut kept: Vec<Value> = vec![];
        for (k, v) in x.iter() {
            if !kept.iter().any(|w| cmp(w, v).is_eq()) {
                kept.push(v.clone());
                out.set(k.clone(), v.clone());
            }
        }
    }
    Ok(mk(out))
}

fn f_column(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_column", &a, 2, 3)?;
    let x = arr_arg(vm, "array_column", 1, "array", &a)?;
    let col = a[1].deref();
    let idx = a.get(2).map(Value::deref).unwrap_or(Value::Null);
    for (n, v, ok) in [(2usize, &col, matches!(col, Value::Null | Value::Int(_) | Value::Str(_) | Value::Float(_) | Value::Bool(_))), (3, &idx, matches!(idx, Value::Null | Value::Int(_) | Value::Str(_) | Value::Float(_) | Value::Bool(_)))] {
        if !ok {
            let ty = if n == 2 { "string|int|null" } else { "string|int|null" };
            return arg_type_err(vm, "array_column", n, if n == 2 { "column_key" } else { "index_key" }, ty, v);
        }
    }
    let fetch = |row: &Value, key: &Value| -> Option<Value> {
        let k = match key {
            Value::Int(i) => Key::Int(*i),
            v => Key::from_str(&v.to_str()),
        };
        match row {
            Value::Array(r) => r.get(&k).map(Value::deref),
            Value::Object(o) => o.props.borrow().get(&k).map(Value::deref),
            _ => None,
        }
    };
    let mut out = PhpArray::new();
    for (_, row) in x.iter() {
        let row = row.deref();
        let val = if matches!(col, Value::Null) { Some(row.clone()) } else { fetch(&row, &col) };
        let Some(val) = val else { continue };
        let key = if matches!(idx, Value::Null) { None } else { fetch(&row, &idx) };
        match key {
            Some(Value::Int(i)) => out.set(Key::Int(i), val),
            Some(Value::Str(s)) => out.set(Key::from_str(&s), val),
            Some(Value::Null) => out.set(Key::Str("".into()), val),
            Some(Value::Bool(b)) => out.set(Key::Int(b as i64), val),
            Some(Value::Float(f)) => out.set(Key::Int(f as i64), val),
            _ => out.push(val),
        }
    }
    Ok(mk(out))
}

fn f_array_rand(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_rand", &a, 1, 2)?;
    let x = arr_arg(vm, "array_rand", 1, "array", &a)?;
    let num = if a.len() > 1 { p_long(vm, "array_rand", 2, "num", &a[1])? } else { 1 };
    let avail = x.len() as i64;
    if avail == 0 {
        return vm.throw_error("ValueError", "array_rand(): Argument #1 ($array) cannot be empty");
    }
    let keys: Vec<Value> = x.iter().map(|(k, _)| k.to_value()).collect();
    if num == 1 {
        let r = mt::mt_rand_range(0, avail - 1);
        return Ok(keys[r as usize].clone());
    }
    if num <= 0 || num > avail {
        return value_err(vm, "array_rand", 2, "num", "must be between 1 and the number of elements in argument #1 ($array)");
    }
    let mut req = num;
    let negative = req > (avail >> 1);
    if negative {
        req = avail - req;
    }
    let mut bits = vec![false; avail as usize];
    let mut i = req;
    while i > 0 {
        let r = mt::mt_rand_range(0, avail - 1) as usize;
        if !bits[r] {
            bits[r] = true;
            i -= 1;
        }
    }
    let mut out = PhpArray::new();
    for (j, k) in keys.into_iter().enumerate() {
        if bits[j] ^ negative {
            out.push(k);
        }
    }
    Ok(mk(out))
}

fn f_first_last(vm: &mut Vm<'_>, a: Vec<Value>, name: &str, last: bool, want_key: bool) -> R<Value> {
    check_argc(vm, name, &a, 1, 1)?;
    let x = arr_arg(vm, name, 1, "array", &a)?;
    let e = if last { x.iter().last() } else { x.iter().next() };
    Ok(match e {
        Some((k, v)) => {
            if want_key {
                k.to_value()
            } else {
                v.deref()
            }
        }
        None => Value::Null,
    })
}

fn cb_check(vm: &mut Vm<'_>, name: &str, n: usize, v: &Value, nullable: bool) -> R<()> {
    if nullable && matches!(v.deref(), Value::Null) {
        return Ok(());
    }
    if is_callable(vm, v) {
        return Ok(());
    }
    let suffix = if nullable { " or null" } else { "" };
    vm.throw_error("TypeError", &format!("{name}(): Argument #{n} ($callback) must be a valid callback{suffix}, {}", callback_error(&v.deref())))
}

fn f_array_map(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    if a.len() < 2 {
        return argc_err(vm, "array_map", 2, usize::MAX, a.len());
    }
    let cb = a[0].deref();
    let first = arr_arg(vm, "array_map", 2, "array", &a)?;
    for (i, v) in a.iter().enumerate().skip(2) {
        if !matches!(v.deref(), Value::Array(_)) {
            return vm.throw_error("TypeError", &format!("array_map(): Argument #{} must be of type array, {} given", i + 1, tname(v)));
        }
    }
    cb_check(vm, "array_map", 1, &cb, true)?;
    if a.len() == 2 {
        if matches!(cb, Value::Null) {
            return Ok(mk((*first).clone()));
        }
        let mut out = PhpArray::new();
        for (k, v) in first.iter() {
            let r = vm.call_value(&cb, vec![v.clone()])?;
            out.set(k.clone(), r);
        }
        return Ok(mk(out));
    }
    let lists: Vec<Vec<Value>> = a[1..]
        .iter()
        .map(|v| match v.deref() {
            Value::Array(x) => x.iter().map(|(_, v)| v.clone()).collect(),
            _ => vec![],
        })
        .collect();
    let n = lists.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = PhpArray::new();
    for i in 0..n {
        let row: Vec<Value> = lists.iter().map(|l| l.get(i).cloned().unwrap_or(Value::Null)).collect();
        if matches!(cb, Value::Null) {
            out.push(mk(PhpArray::from_list(row)));
        } else {
            out.push(vm.call_value(&cb, row)?);
        }
    }
    Ok(mk(out))
}

fn f_array_filter(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_filter", &a, 1, 3)?;
    let x = arr_arg(vm, "array_filter", 1, "array", &a)?;
    let cb = a.get(1).map(Value::deref).unwrap_or(Value::Null);
    let mode = if a.len() > 2 { p_long(vm, "array_filter", 3, "mode", &a[2])? } else { 0 };
    cb_check(vm, "array_filter", 2, &cb, true)?;
    if !matches!(mode, 0 | 1 | 2) {
        return value_err(vm, "array_filter", 3, "mode", "must be one of ARRAY_FILTER_USE_VALUE, ARRAY_FILTER_USE_KEY, or ARRAY_FILTER_USE_BOTH");
    }
    let mut out = PhpArray::new();
    for (k, v) in x.iter() {
        let keep = if matches!(cb, Value::Null) {
            v.deref().to_bool()
        } else {
            let args = match mode {
                2 => vec![k.to_value()],
                1 => vec![v.clone(), k.to_value()],
                _ => vec![v.clone()],
            };
            vm.call_value(&cb, args)?.to_bool()
        };
        if keep {
            out.set(k.clone(), v.clone());
        }
    }
    Ok(mk(out))
}

fn f_array_reduce(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_reduce", &a, 2, 3)?;
    let x = arr_arg(vm, "array_reduce", 1, "array", &a)?;
    cb_check(vm, "array_reduce", 2, &a[1], false)?;
    let cb = a[1].deref();
    let mut acc = a.get(2).map(Value::deref).unwrap_or(Value::Null);
    for (_, v) in x.iter() {
        acc = vm.call_value(&cb, vec![acc, v.clone()])?;
    }
    Ok(acc)
}

fn find_like(vm: &mut Vm<'_>, name: &str, a: Vec<Value>) -> R<Value> {
    check_argc(vm, name, &a, 2, 2)?;
    let x = arr_arg(vm, name, 1, "array", &a)?;
    cb_check(vm, name, 2, &a[1], false)?;
    let cb = a[1].deref();
    for (k, v) in x.iter() {
        let r = vm.call_value(&cb, vec![v.clone(), k.to_value()])?.to_bool();
        match name {
            "array_find" if r => return Ok(v.deref()),
            "array_find_key" if r => return Ok(k.to_value()),
            "array_any" if r => return Ok(Value::Bool(true)),
            "array_all" if !r => return Ok(Value::Bool(false)),
            _ => {}
        }
    }
    Ok(match name {
        "array_any" => Value::Bool(false),
        "array_all" => Value::Bool(true),
        _ => Value::Null,
    })
}

fn f_is_list(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "array_is_list", &a, 1, 1)?;
    let x = arr_arg(vm, "array_is_list", 1, "array", &a)?;
    let r = x.iter().enumerate().all(|(i, (k, _))| *k == Key::Int(i as i64));
    Ok(Value::Bool(r))
}

fn f_clamp(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "clamp", &a, 3, 3)?;
    let (v, min, max) = (a[0].deref(), a[1].deref(), a[2].deref());
    if matches!(min, Value::Float(f) if f.is_nan()) {
        return value_err(vm, "clamp", 2, "min", "must not be NAN");
    }
    if matches!(max, Value::Float(f) if f.is_nan()) {
        return value_err(vm, "clamp", 3, "max", "must not be NAN");
    }
    if loose_cmp(&max, &min).is_lt() {
        return value_err(vm, "clamp", 2, "min", "must be smaller than or equal to argument #3 ($max)");
    }
    if loose_cmp(&max, &v).is_lt() {
        return Ok(max);
    }
    if loose_cmp(&v, &min).is_lt() {
        return Ok(min);
    }
    Ok(v)
}

fn check_cb_fn(vm: &mut Vm<'_>, a: &[Value]) -> R<()> {
    let _ = check_callback;
    let _ = vm;
    let _ = a;
    Ok(())
}

pub fn register_vm(m: &mut HashMap<&'static str, VmBuiltin>) {
    let _ = check_cb_fn;
    let list: &[(&str, VmBuiltin)] = &[
        ("count", f_count),
        ("sizeof", f_count),
        ("current", f_current),
        ("pos", f_current),
        ("key", f_key),
        ("min", |vm, a| minmax(vm, "min", a, false)),
        ("max", |vm, a| minmax(vm, "max", a, true)),
        ("in_array", |vm, a| search(vm, "in_array", &a, false)),
        ("array_search", |vm, a| search(vm, "array_search", &a, true)),
        ("array_fill", f_array_fill),
        ("array_fill_keys", f_array_fill_keys),
        ("array_keys", f_array_keys),
        ("array_slice", f_array_slice),
        ("array_merge", f_array_merge),
        ("array_merge_recursive", f_array_merge_recursive),
        ("array_replace", |vm, a| replace(vm, "array_replace", a, false)),
        ("array_replace_recursive", |vm, a| replace(vm, "array_replace_recursive", a, true)),
        ("array_key_exists", |vm, a| f_array_key_exists(vm, a, "array_key_exists")),
        ("key_exists", |vm, a| f_array_key_exists(vm, a, "key_exists")),
        ("array_change_key_case", f_change_key_case),
        ("array_count_values", f_count_values),
        ("array_combine", f_combine),
        ("array_flip", f_flip),
        ("array_pad", f_pad),
        ("array_chunk", f_chunk),
        ("array_reverse", f_reverse),
        ("array_unique", f_unique),
        ("array_column", f_column),
        ("array_rand", f_array_rand),
        ("array_key_first", |vm, a| f_first_last(vm, a, "array_key_first", false, true)),
        ("array_key_last", |vm, a| f_first_last(vm, a, "array_key_last", true, true)),
        ("array_first", |vm, a| f_first_last(vm, a, "array_first", false, false)),
        ("array_last", |vm, a| f_first_last(vm, a, "array_last", true, false)),
        ("array_map", f_array_map),
        ("array_filter", f_array_filter),
        ("array_reduce", f_array_reduce),
        ("array_find", |vm, a| find_like(vm, "array_find", a)),
        ("array_find_key", |vm, a| find_like(vm, "array_find_key", a)),
        ("array_any", |vm, a| find_like(vm, "array_any", a)),
        ("array_all", |vm, a| find_like(vm, "array_all", a)),
        ("array_is_list", f_is_list),
        ("clamp", f_clamp),
        ("range", super::range::f_range),
    ];
    for (k, f) in list {
        m.insert(k, *f);
    }
}
