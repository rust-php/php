//! By-reference array builtins (sort family, push/pop/shift/unshift/splice, walk, multisort,
//! internal pointer, shuffle), ported from ext/standard/array.c.
use super::args::*;
use super::mt;
use super::sort::*;
use crate::value::*;
use crate::vm::{Flow, Vm};
use std::cell::RefCell;
use std::rc::Rc;

fn entries(a: &PhpArray) -> Vec<(Key, Value)> {
    a.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
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

fn first_array(vm: &mut Vm<'_>, f: &str, args: &[Value]) -> R<Rc<PhpArray>> {
    match args.first().map(Value::deref) {
        Some(Value::Array(a)) => Ok(a),
        Some(v) => arg_type_err(vm, f, 1, "array", "array", &v),
        None => argc_err(vm, f, 1, 1, 0),
    }
}

fn do_sort(vm: &mut Vm<'_>, name: &str, args: &mut [Value]) -> R<Value> {
    let (by_key, reverse, renumber) = match name {
        "sort" => (false, false, true),
        "rsort" => (false, true, true),
        "asort" => (false, false, false),
        "arsort" => (false, true, false),
        "ksort" => (true, false, false),
        "krsort" => (true, true, false),
        "natsort" => (false, false, false),
        "natcasesort" => (false, false, false),
        _ => unreachable!(),
    };
    let max = if name.starts_with("nat") { 1 } else { 2 };
    check_argc(vm, name, args, 1, max)?;
    let arr = first_array(vm, name, args)?;
    let mut flags = if args.len() > 1 { p_long(vm, name, 2, "flags", &args[1])? } else { 0 };
    if name == "natsort" {
        flags = SORT_NATURAL;
    } else if name == "natcasesort" {
        flags = SORT_NATURAL | SORT_FLAG_CASE;
    }
    let cmp = if by_key { key_cmp(flags) } else { data_cmp(flags) };
    let items = entries(&arr);
    let out = merge_sort::<_, Flow>(items, &mut |a, b| {
        let o = if by_key { cmp(&a.0.to_value(), &b.0.to_value()) } else { cmp(&a.1, &b.1) };
        Ok(if reverse { o.reverse() } else { o })
    })?;
    args[0] = build(out, renumber);
    Ok(Value::Bool(true))
}

fn do_usort(vm: &mut Vm<'_>, name: &str, args: &mut [Value]) -> R<Value> {
    check_argc(vm, name, args, 2, 2)?;
    let arr = first_array(vm, name, args)?;
    let cb = args[1].deref();
    let by_key = name == "uksort";
    let items = entries(&arr);
    let out = merge_sort::<_, Flow>(items, &mut |a, b| {
        let (x, y) = if by_key { (a.0.to_value(), b.0.to_value()) } else { (a.1.clone(), b.1.clone()) };
        let r = vm.call_value(&cb, vec![x, y])?;
        let n = match r {
            Value::Bool(false) => 0,
            r => r.to_int(),
        };
        Ok(n.cmp(&0))
    })?;
    args[0] = build(out, name == "usort");
    Ok(Value::Bool(true))
}

fn do_multisort(vm: &mut Vm<'_>, args: &mut [Value]) -> R<Value> {
    if args.is_empty() {
        return argc_err(vm, "array_multisort", 1, usize::MAX, 0);
    }
    let mut arrays: Vec<usize> = vec![];
    let mut funcs: Vec<(i64, bool)> = vec![];
    let (mut order_ok, mut type_ok) = (false, false);
    let (mut sort_order, mut sort_type) = (SORT_ASC, SORT_REGULAR);
    for i in 0..args.len() {
        match args[i].deref() {
            Value::Array(_) => {
                if i > 0 {
                    funcs.push((sort_type, sort_order != SORT_ASC));
                    sort_order = SORT_ASC;
                    sort_type = SORT_REGULAR;
                }
                arrays.push(i);
                order_ok = true;
                type_ok = true;
            }
            Value::Int(n) => match n & !SORT_FLAG_CASE {
                SORT_ASC | SORT_DESC => {
                    if order_ok {
                        sort_order = if n == SORT_DESC { SORT_DESC } else { SORT_ASC };
                        order_ok = false;
                    } else {
                        return vm.throw_error("TypeError", &format!("array_multisort(): Argument #{} must be an array or a sort flag that has not already been specified", i + 1));
                    }
                }
                SORT_REGULAR | SORT_NUMERIC | SORT_STRING | SORT_NATURAL | SORT_LOCALE_STRING => {
                    if type_ok {
                        sort_type = n;
                        type_ok = false;
                    } else {
                        return vm.throw_error("TypeError", &format!("array_multisort(): Argument #{} must be an array or a sort flag that has not already been specified", i + 1));
                    }
                }
                _ => return vm.throw_error("ValueError", &format!("array_multisort(): Argument #{} must be a valid sort flag", i + 1)),
            },
            _ => return vm.throw_error("TypeError", &format!("array_multisort(): Argument #{} must be an array or a sort flag", i + 1)),
        }
    }
    if arrays.is_empty() {
        return vm.throw_error("TypeError", "array_multisort(): Argument #1 ($array) must be an array or a sort flag");
    }
    funcs.push((sort_type, sort_order != SORT_ASC));
    let cols: Vec<Vec<(Key, Value)>> = arrays
        .iter()
        .map(|&i| match args[i].deref() {
            Value::Array(a) => entries(&a),
            _ => vec![],
        })
        .collect();
    let size = cols[0].len();
    if cols.iter().any(|c| c.len() != size) {
        return vm.throw_error("ValueError", "Array sizes are inconsistent");
    }
    if size < 1 {
        return Ok(Value::Bool(true));
    }
    let cmps: Vec<(Cmp, bool)> = funcs.iter().map(|(t, rev)| (data_cmp(*t), *rev)).collect();
    let idx: Vec<usize> = (0..size).collect();
    let sorted = merge_sort::<_, Flow>(idx, &mut |&a, &b| {
        for (c, (cmp, rev)) in cols.iter().zip(cmps.iter()) {
            let o = cmp(&c[a].1, &c[b].1);
            let o = if *rev { o.reverse() } else { o };
            if o != std::cmp::Ordering::Equal {
                return Ok(o);
            }
        }
        Ok(std::cmp::Ordering::Equal)
    })?;
    for (ci, &ai) in arrays.iter().enumerate() {
        let col = &cols[ci];
        let mut a = PhpArray::new();
        for &k in &sorted {
            match &col[k].0 {
                Key::Int(_) => a.push(col[k].1.clone()),
                key => a.set(key.clone(), col[k].1.clone()),
            }
        }
        args[ai] = Value::Array(Rc::new(a));
    }
    Ok(Value::Bool(true))
}

fn walk(vm: &mut Vm<'_>, arr: &PhpArray, cb: &Value, extra: &Option<Value>, recursive: bool, depth: usize) -> R<PhpArray> {
    let mut out = PhpArray::new();
    if depth > 256 {
        return vm.throw_error("Error", "Recursion detected");
    }
    for (k, v) in arr.iter() {
        let cur = v.deref();
        if recursive {
            if let Value::Array(sub) = &cur {
                let r = walk(vm, sub, cb, extra, recursive, depth + 1)?;
                out.set(k.clone(), Value::Array(Rc::new(r)));
                continue;
            }
        }
        let cell = Rc::new(RefCell::new(cur));
        let mut cargs = vec![Value::Ref(cell.clone()), k.to_value()];
        cargs.extend(extra.clone());
        vm.call_value(cb, cargs)?;
        let nv = cell.borrow().clone();
        out.set(k.clone(), nv);
    }
    Ok(out)
}

fn do_walk(vm: &mut Vm<'_>, name: &str, args: &mut [Value]) -> R<Value> {
    check_argc(vm, name, args, 2, 3)?;
    let arr = match args[0].deref() {
        Value::Array(a) => a,
        v => return arg_type_err(vm, name, 1, "array", "array|object", &v),
    };
    let cb = args[1].deref();
    let extra = args.get(2).map(Value::deref);
    let mut out = walk(vm, &arr, &cb, &extra, name == "array_walk_recursive", 0)?;
    out.ptr = arr.ptr;
    args[0] = Value::Array(Rc::new(out));
    Ok(Value::Bool(true))
}

fn occupied(a: &PhpArray) -> bool {
    a.get(&Key::Int(i64::MAX)).is_some()
}

fn do_push(vm: &mut Vm<'_>, args: &mut [Value]) -> R<Value> {
    if args.is_empty() {
        return argc_err(vm, "array_push", 1, usize::MAX, 0);
    }
    let arr = first_array(vm, "array_push", args)?;
    let mut a = (*arr).clone();
    for v in args.iter().skip(1) {
        if occupied(&a) {
            return vm.throw_error("Error", "Cannot add element to the array as the next element is already occupied");
        }
        a.push(v.deref());
    }
    let n = a.len() as i64;
    args[0] = Value::Array(Rc::new(a));
    Ok(Value::Int(n))
}

fn do_pop(vm: &mut Vm<'_>, args: &mut [Value]) -> R<Value> {
    check_argc(vm, "array_pop", args, 1, 1)?;
    let arr = first_array(vm, "array_pop", args)?;
    let mut items = entries(&arr);
    let Some((_, v)) = items.pop() else { return Ok(Value::Null) };
    args[0] = build(items, false);
    Ok(v)
}

fn do_shift(vm: &mut Vm<'_>, args: &mut [Value]) -> R<Value> {
    check_argc(vm, "array_shift", args, 1, 1)?;
    let arr = first_array(vm, "array_shift", args)?;
    let mut items = entries(&arr);
    if items.is_empty() {
        return Ok(Value::Null);
    }
    let (_, v) = items.remove(0);
    args[0] = build(items, true);
    Ok(v)
}

fn do_unshift(vm: &mut Vm<'_>, args: &mut [Value]) -> R<Value> {
    if args.is_empty() {
        return argc_err(vm, "array_unshift", 1, usize::MAX, 0);
    }
    let arr = first_array(vm, "array_unshift", args)?;
    let mut all: Vec<(Key, Value)> = args.iter().skip(1).map(|v| (Key::Int(0), v.deref())).collect();
    all.extend(entries(&arr));
    let n = all.len() as i64;
    args[0] = build(all, true);
    Ok(Value::Int(n))
}

/// php_splice
fn do_splice(vm: &mut Vm<'_>, args: &mut [Value]) -> R<Value> {
    check_argc(vm, "array_splice", args, 2, 4)?;
    let arr = first_array(vm, "array_splice", args)?;
    let mut offset = p_long(vm, "array_splice", 2, "offset", &args[1])?;
    let num_in = arr.len() as i64;
    let mut length = match args.get(2).map(Value::deref) {
        None | Some(Value::Null) => num_in,
        Some(v) => p_long(vm, "array_splice", 3, "length", &v)?,
    };
    let repl: Vec<Value> = if args.len() == 4 {
        match args[3].deref() {
            Value::Array(a) => a.iter().map(|(_, v)| v.clone()).collect(),
            Value::Null => vec![],
            v => vec![v],
        }
    } else {
        vec![]
    };
    if offset > num_in {
        offset = num_in;
    } else if offset < 0 {
        offset += num_in;
        if offset < 0 {
            offset = 0;
        }
    }
    if length < 0 {
        length = (num_in - offset + length).max(0);
    } else if offset + length > num_in {
        length = num_in - offset;
    }
    let items = entries(&arr);
    let (off, end) = (offset as usize, (offset + length) as usize);
    let mut removed = PhpArray::new();
    for (k, v) in &items[off..end] {
        match k {
            Key::Int(_) => removed.push(v.clone()),
            k => removed.set(k.clone(), v.clone()),
        }
    }
    let mut out: Vec<(Key, Value)> = items[..off].to_vec();
    out.extend(repl.into_iter().map(|v| (Key::Int(0), v)));
    out.extend(items[end..].iter().cloned());
    args[0] = build(out, true);
    Ok(Value::Array(Rc::new(removed)))
}

fn do_pointer(vm: &mut Vm<'_>, name: &str, args: &mut [Value]) -> R<Value> {
    check_argc(vm, name, args, 1, 1)?;
    let mut av = args[0].deref();
    let Value::Array(rc) = &mut av else {
        return arg_type_err(vm, name, 1, "array", "array|object", &av);
    };
    let arr = Rc::make_mut(rc);
    let live: Vec<usize> = (0..arr.slot_count()).filter(|i| arr.key_at(*i).is_some()).collect();
    let cur = live.iter().position(|s| *s >= arr.ptr);
    let target = match name {
        "reset" => live.first().copied(),
        "end" => live.last().copied(),
        "next" => cur.and_then(|c| live.get(c + 1)).copied(),
        _ => cur.and_then(|c| c.checked_sub(1)).and_then(|c| live.get(c)).copied(),
    };
    let r = match target {
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
    };
    args[0] = av;
    Ok(r)
}

/// php_array_data_shuffle (Fisher-Yates using php_mt_rand_range).
fn do_shuffle(vm: &mut Vm<'_>, args: &mut [Value]) -> R<Value> {
    check_argc(vm, "shuffle", args, 1, 1)?;
    let arr = first_array(vm, "shuffle", args)?;
    let mut vals: Vec<Value> = arr.iter().map(|(_, v)| v.clone()).collect();
    let n = vals.len();
    if n > 1 {
        let mut n_left = n - 1;
        while n_left > 0 {
            let rnd = mt::mt_rand_range(0, n_left as i64) as usize;
            if rnd != n_left {
                vals.swap(n_left, rnd);
            }
            n_left -= 1;
        }
    }
    args[0] = Value::Array(Rc::new(PhpArray::from_list(vals)));
    Ok(Value::Bool(true))
}

pub fn call(vm: &mut Vm<'_>, name: &str, args: &mut Vec<Value>) -> Option<R<Value>> {
    Some(match name {
        "sort" | "rsort" | "asort" | "arsort" | "ksort" | "krsort" | "natsort" | "natcasesort" => do_sort(vm, name, args),
        "usort" | "uasort" | "uksort" => do_usort(vm, name, args),
        "array_multisort" => do_multisort(vm, args),
        "array_walk" | "array_walk_recursive" => do_walk(vm, name, args),
        "array_push" => do_push(vm, args),
        "array_pop" => do_pop(vm, args),
        "array_shift" => do_shift(vm, args),
        "array_unshift" => do_unshift(vm, args),
        "array_splice" => do_splice(vm, args),
        "reset" | "end" | "next" | "prev" => do_pointer(vm, name, args),
        "shuffle" => do_shuffle(vm, args),
        _ => return None,
    })
}
