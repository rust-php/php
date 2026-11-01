//! Shared helpers for the string ports: ZPP-like argument parsing and diagnostics.
use crate::builtins::arg;
use crate::value::*;
use crate::vm::{Flow, Vm};
use std::rc::Rc;

pub type R<T> = Result<T, Flow>;

pub fn diag(vm: &mut Vm<'_>, level: &str, msg: &str) {
    let f = vm.file.clone();
    vm.write(&format!("\n{level}: {msg} in {f} on line 0\n"));
}
pub fn warn(vm: &mut Vm<'_>, msg: &str) {
    diag(vm, "Warning", msg)
}
pub fn notice(vm: &mut Vm<'_>, msg: &str) {
    diag(vm, "Notice", msg)
}
pub fn deprecated(vm: &mut Vm<'_>, msg: &str) {
    diag(vm, "Deprecated", msg)
}

pub fn zname(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        _ => v.type_name(),
    }
}

/// Check argument count like ZEND_PARSE_PARAMETERS_START(min, max); max < 0 means variadic.
pub fn argc(vm: &mut Vm<'_>, f: &str, a: &[Value], min: usize, max: i64) -> R<()> {
    let n = a.len();
    if n < min || (max >= 0 && n > max as usize) {
        let (kind, exp) = if n < min { ("at least", min) } else { ("at most", max as usize) };
        let kind = if min as i64 == max { "exactly" } else { kind };
        let msg = format!("{f}() expects {kind} {exp} argument{}, {n} given", if exp == 1 { "" } else { "s" });
        return vm.throw_error("ArgumentCountError", &msg);
    }
    Ok(())
}

fn terr<T>(vm: &mut Vm<'_>, f: &str, i: usize, p: &str, ty: &str, v: &Value) -> R<T> {
    let msg = format!("{f}(): Argument #{} (${p}) must be of type {ty}, {} given", i + 1, zname(v));
    vm.throw_error("TypeError", &msg)
}

fn null_dep(vm: &mut Vm<'_>, f: &str, i: usize, p: &str, ty: &str) {
    deprecated(vm, &format!("{f}(): Passing null to parameter #{} (${p}) of type {ty} is deprecated", i + 1));
}

pub fn p_str(vm: &mut Vm<'_>, f: &str, a: &[Value], i: usize, p: &str) -> R<Rc<str>> {
    let v = arg(a, i).deref();
    match &v {
        Value::Str(s) => Ok(s.clone()),
        Value::Array(_) => terr(vm, f, i, p, "string", &v),
        Value::Null => {
            null_dep(vm, f, i, p, "string");
            Ok("".into())
        }
        _ => Ok(v.to_str()),
    }
}

pub fn p_str_opt(vm: &mut Vm<'_>, f: &str, a: &[Value], i: usize, p: &str, def: &str) -> R<Rc<str>> {
    if i >= a.len() {
        return Ok(def.into());
    }
    p_str(vm, f, a, i, p)
}

pub fn p_int(vm: &mut Vm<'_>, f: &str, a: &[Value], i: usize, p: &str) -> R<i64> {
    let v = arg(a, i).deref();
    match &v {
        Value::Int(n) => Ok(*n),
        Value::Bool(b) => Ok(*b as i64),
        Value::Float(x) => {
            if x.is_nan() || x.is_infinite() {
                return terr(vm, f, i, p, "int", &v);
            }
            if x.fract() != 0.0 {
                deprecated(vm, &format!("Implicit conversion from float {} to int loses precision", fmt_float(*x, None)));
            }
            Ok(*x as i64)
        }
        Value::Null => {
            null_dep(vm, f, i, p, "int");
            Ok(0)
        }
        Value::Str(s) => match parse_numeric(s) {
            Some((Num::I(n), _)) => Ok(n),
            Some((Num::F(x), _)) => Ok(x as i64),
            None => terr(vm, f, i, p, "int", &v),
        },
        _ => terr(vm, f, i, p, "int", &v),
    }
}

pub fn p_int_opt(vm: &mut Vm<'_>, f: &str, a: &[Value], i: usize, p: &str, def: i64) -> R<i64> {
    if i >= a.len() {
        return Ok(def);
    }
    p_int(vm, f, a, i, p)
}

/// ?int parameter
pub fn p_int_null(vm: &mut Vm<'_>, f: &str, a: &[Value], i: usize, p: &str) -> R<Option<i64>> {
    match a.get(i) {
        None | Some(Value::Null) => Ok(None),
        _ => p_int(vm, f, a, i, p).map(Some),
    }
}

pub fn p_bool(a: &[Value], i: usize, def: bool) -> bool {
    a.get(i).map(|v| v.to_bool()).unwrap_or(def)
}

pub fn p_float(vm: &mut Vm<'_>, f: &str, a: &[Value], i: usize, p: &str) -> R<f64> {
    let v = arg(a, i).deref();
    match &v {
        Value::Int(n) => Ok(*n as f64),
        Value::Float(x) => Ok(*x),
        Value::Bool(b) => Ok(*b as i64 as f64),
        Value::Null => {
            null_dep(vm, f, i, p, "float");
            Ok(0.0)
        }
        Value::Str(s) => match parse_numeric(s) {
            Some((n, _)) => Ok(n.f()),
            None => terr(vm, f, i, p, "float", &v),
        },
        _ => terr(vm, f, i, p, "float", &v),
    }
}

pub fn p_array(vm: &mut Vm<'_>, f: &str, a: &[Value], i: usize, p: &str) -> R<Rc<PhpArray>> {
    match a.get(i).map(|v| v.deref()) {
        Some(Value::Array(x)) => Ok(x),
        other => {
            let v = other.unwrap_or(Value::Null);
            terr(vm, f, i, p, "array", &v)
        }
    }
}

pub fn vstr(s: &str) -> Value {
    Value::Str(s.into())
}

pub fn list(items: Vec<Value>) -> Value {
    Value::Array(Rc::new(PhpArray::from_list(items)))
}

pub fn verr<T>(vm: &mut Vm<'_>, msg: &str) -> R<T> {
    vm.throw_error("ValueError", msg)
}

/// Binary-ish view of a string: UTF-8 bytes.
pub fn bytes(s: &str) -> &[u8] {
    s.as_bytes()
}

/// Build a string from bytes (lossy when not valid UTF-8; the engine has no binary strings yet).
pub fn from_bytes(b: &[u8]) -> Value {
    Value::Str(String::from_utf8_lossy(b).as_ref().into())
}
