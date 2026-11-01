//! array_diff / array_intersect families (ext/standard/array.c).
use super::args::*;
use crate::builtins::VmBuiltin;
use crate::value::*;
use crate::vm::Vm;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq)]
enum Data {
    None,
    Str,
    User,
}
#[derive(Clone, Copy, PartialEq)]
enum KeyC {
    Ignore,
    Internal,
    User,
}

struct Spec {
    name: &'static str,
    intersect: bool,
    data: Data,
    key: KeyC,
}

const SPECS: &[Spec] = &[
    Spec { name: "array_diff", intersect: false, data: Data::Str, key: KeyC::Ignore },
    Spec { name: "array_diff_key", intersect: false, data: Data::None, key: KeyC::Internal },
    Spec { name: "array_diff_assoc", intersect: false, data: Data::Str, key: KeyC::Internal },
    Spec { name: "array_diff_ukey", intersect: false, data: Data::None, key: KeyC::User },
    Spec { name: "array_diff_uassoc", intersect: false, data: Data::Str, key: KeyC::User },
    Spec { name: "array_udiff", intersect: false, data: Data::User, key: KeyC::Ignore },
    Spec { name: "array_udiff_assoc", intersect: false, data: Data::User, key: KeyC::Internal },
    Spec { name: "array_udiff_uassoc", intersect: false, data: Data::User, key: KeyC::User },
    Spec { name: "array_intersect", intersect: true, data: Data::Str, key: KeyC::Ignore },
    Spec { name: "array_intersect_key", intersect: true, data: Data::None, key: KeyC::Internal },
    Spec { name: "array_intersect_assoc", intersect: true, data: Data::Str, key: KeyC::Internal },
    Spec { name: "array_intersect_ukey", intersect: true, data: Data::None, key: KeyC::User },
    Spec { name: "array_intersect_uassoc", intersect: true, data: Data::Str, key: KeyC::User },
    Spec { name: "array_uintersect", intersect: true, data: Data::User, key: KeyC::Ignore },
    Spec { name: "array_uintersect_assoc", intersect: true, data: Data::User, key: KeyC::Internal },
    Spec { name: "array_uintersect_uassoc", intersect: true, data: Data::User, key: KeyC::User },
];

pub fn callback_error(v: &Value) -> String {
    match v {
        Value::Str(s) => {
            if let Some((c, _)) = s.split_once("::") {
                format!("class \"{c}\" not found")
            } else {
                format!("function \"{s}\" not found or invalid function name")
            }
        }
        Value::Array(a) if a.len() != 2 => "array callback must have exactly two members".to_string(),
        Value::Array(a) => {
            let c = a.get(&Key::Int(0)).map(|v| v.to_str().to_string()).unwrap_or_default();
            format!("class \"{c}\" not found")
        }
        _ => "no array or string given".to_string(),
    }
}

pub fn is_callable(vm: &mut Vm<'_>, v: &Value) -> bool {
    if let Value::Array(a) = v.deref() {
        if let (Some(Value::Object(o)), Some(_)) = (a.get(&Key::Int(0)).map(Value::deref), a.get(&Key::Int(1))) {
            if o.class.find_method("__call").is_some() {
                return true;
            }
        }
    }
    matches!(vm.call_value(&Value::str("is_callable"), vec![v.deref()]), Ok(Value::Bool(true)))
}

pub fn check_callback(vm: &mut Vm<'_>, f: &str, n: usize, pname: &str, v: &Value) -> R<()> {
    let v = v.deref();
    if is_callable(vm, &v) {
        return Ok(());
    }
    let named = if pname.is_empty() { format!("Argument #{n}") } else { format!("Argument #{n} (${pname})") };
    vm.throw_error("TypeError", &format!("{f}(): {named} must be a valid callback, {}", callback_error(&v)))
}

fn user_eq(vm: &mut Vm<'_>, cb: &Value, a: Value, b: Value) -> R<bool> {
    let r = vm.call_value(cb, vec![a, b])?;
    Ok(match r {
        Value::Bool(b) => !b,
        r => r.to_int() == 0,
    })
}

fn run(vm: &mut Vm<'_>, sp: &Spec, mut a: Vec<Value>) -> R<Value> {
    let name = sp.name;
    let ncb = (sp.data == Data::User) as usize + (sp.key == KeyC::User) as usize;
    if a.is_empty() || a.len() < 1 + ncb {
        if a.len() < 1 + ncb && !a.is_empty() && ncb > 0 {
            let given = a.len();
            return vm.throw_error("ArgumentCountError", &format!("At least {} arguments are required, {given} given", 1 + ncb));
        }
        return vm.throw_error("ArgumentCountError", &format!("{name}() expects at least 1 argument, 0 given"));
    }
    let split = a.len() - ncb;
    let cbs: Vec<Value> = a.split_off(split);
    for (i, cb) in cbs.iter().enumerate() {
        check_callback(vm, name, split + i + 1, "", cb)?;
    }
    let (data_cb, key_cb) = match (sp.data == Data::User, sp.key == KeyC::User) {
        (true, true) => (Some(cbs[0].deref()), Some(cbs[1].deref())),
        (true, false) => (Some(cbs[0].deref()), None),
        (false, true) => (None, Some(cbs[0].deref())),
        _ => (None, None),
    };
    let mut arrays: Vec<Rc<PhpArray>> = vec![];
    for (i, v) in a.iter().enumerate() {
        match v.deref() {
            Value::Array(x) => arrays.push(x),
            v => {
                let msg = if i == 0 {
                    format!("{name}(): Argument #1 ($array) must be of type array, {} given", tname(&v))
                } else {
                    format!("{name}(): Argument #{} must be of type array, {} given", i + 1, tname(&v))
                };
                return vm.throw_error("TypeError", &msg);
            }
        }
    }
    let first = arrays[0].clone();
    let rest = &arrays[1..];
    // Precomputed string sets for the plain value comparisons.
    let sets: Vec<HashSet<Rc<str>>> = if sp.data == Data::Str && sp.key == KeyC::Ignore {
        rest.iter().map(|b| b.iter().map(|(_, v)| v.to_str()).collect()).collect()
    } else {
        vec![]
    };
    let mut out = PhpArray::new();
    for (k, v) in first.iter() {
        let mut found_in_all = true;
        let mut found_in_any = false;
        for (bi, b) in rest.iter().enumerate() {
            let found = if sp.data == Data::Str && sp.key == KeyC::Ignore {
                sets[bi].contains(&v.to_str())
            } else if sp.key == KeyC::Internal {
                match b.get(k) {
                    None => false,
                    Some(w) => match sp.data {
                        Data::None => true,
                        Data::Str => v.to_str() == w.to_str(),
                        Data::User => user_eq(vm, data_cb.as_ref().unwrap(), v.clone(), w.clone())?,
                    },
                }
            } else {
                let mut f = false;
                for (k2, w) in b.iter() {
                    let key_ok = match &key_cb {
                        Some(cb) => user_eq(vm, cb, k.to_value(), k2.to_value())?,
                        None => true,
                    };
                    if !key_ok {
                        continue;
                    }
                    let data_ok = match sp.data {
                        Data::None => true,
                        Data::Str => v.to_str() == w.to_str(),
                        Data::User => user_eq(vm, data_cb.as_ref().unwrap(), v.clone(), w.clone())?,
                    };
                    if data_ok {
                        f = true;
                        break;
                    }
                }
                f
            };
            if found {
                found_in_any = true;
            } else {
                found_in_all = false;
            }
            if sp.intersect && !found {
                break;
            }
            if !sp.intersect && found {
                break;
            }
        }
        let keep = if sp.intersect { found_in_all } else { !found_in_any };
        if keep {
            out.set(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

macro_rules! entry {
    ($m:expr, $i:expr) => {
        $m.insert(SPECS[$i].name, (|vm: &mut Vm<'_>, a: Vec<Value>| run(vm, &SPECS[$i], a)) as VmBuiltin);
    };
}

pub fn register_vm(m: &mut HashMap<&'static str, VmBuiltin>) {
    entry!(m, 0);
    entry!(m, 1);
    entry!(m, 2);
    entry!(m, 3);
    entry!(m, 4);
    entry!(m, 5);
    entry!(m, 6);
    entry!(m, 7);
    entry!(m, 8);
    entry!(m, 9);
    entry!(m, 10);
    entry!(m, 11);
    entry!(m, 12);
    entry!(m, 13);
    entry!(m, 14);
    entry!(m, 15);
}
