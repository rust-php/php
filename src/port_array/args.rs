//! Argument parsing helpers mirroring Z_PARAM_* behaviour of zend_API.h.
use crate::value::*;
use crate::vm::{Flow, Vm};

pub type R<T> = Result<T, Flow>;

pub fn tname(v: &Value) -> String {
    match v {
        Value::Object(o) => o.class.name.to_string(),
        Value::Ref(r) => tname(&r.borrow()),
        v => v.type_name().to_string(),
    }
}

pub fn arg_type_err<T>(vm: &mut Vm<'_>, f: &str, n: usize, pname: &str, ty: &str, v: &Value) -> R<T> {
    vm.throw_error("TypeError", &format!("{f}(): Argument #{n} (${pname}) must be of type {ty}, {} given", tname(v)))
}

pub fn value_err<T>(vm: &mut Vm<'_>, f: &str, n: usize, pname: &str, msg: &str) -> R<T> {
    vm.throw_error("ValueError", &format!("{f}(): Argument #{n} (${pname}) {msg}"))
}

/// Z_PARAM_NUMBER: returns Int or Float.
pub fn p_number(vm: &mut Vm<'_>, f: &str, n: usize, pname: &str, v: &Value) -> R<Value> {
    let v = v.deref();
    match &v {
        Value::Int(_) | Value::Float(_) => Ok(v),
        Value::Bool(b) => Ok(Value::Int(*b as i64)),
        Value::Null => Ok(Value::Int(0)),
        Value::Str(s) => match parse_numeric(s) {
            Some((Num::I(i), _)) => Ok(Value::Int(i)),
            Some((Num::F(x), _)) => Ok(Value::Float(x)),
            None => arg_type_err(vm, f, n, pname, "int|float", &v),
        },
        _ => arg_type_err(vm, f, n, pname, "int|float", &v),
    }
}

pub fn p_double(vm: &mut Vm<'_>, f: &str, n: usize, pname: &str, v: &Value) -> R<f64> {
    let v = v.deref();
    match &v {
        Value::Int(i) => Ok(*i as f64),
        Value::Float(x) => Ok(*x),
        Value::Bool(b) => Ok(*b as i64 as f64),
        Value::Null => Ok(0.0),
        Value::Str(s) => match parse_numeric(s) {
            Some((nn, _)) => Ok(nn.f()),
            None => arg_type_err(vm, f, n, pname, "float", &v),
        },
        _ => arg_type_err(vm, f, n, pname, "float", &v),
    }
}

pub fn p_long(vm: &mut Vm<'_>, f: &str, n: usize, pname: &str, v: &Value) -> R<i64> {
    let v = v.deref();
    match &v {
        Value::Int(i) => Ok(*i),
        Value::Float(x) => float_long(vm, f, n, pname, *x, &v),
        Value::Bool(b) => Ok(*b as i64),
        Value::Null => Ok(0),
        Value::Str(s) => match parse_numeric(s) {
            Some((Num::I(i), _)) => Ok(i),
            Some((Num::F(x), _)) => float_long(vm, f, n, pname, x, &v),
            None => arg_type_err(vm, f, n, pname, "int", &v),
        },
        _ => arg_type_err(vm, f, n, pname, "int", &v),
    }
}

fn float_long(vm: &mut Vm<'_>, f: &str, n: usize, pname: &str, x: f64, v: &Value) -> R<i64> {
    if x.is_nan() || x.is_infinite() || !(x >= -9.223372036854775808e18 && x < 9.223372036854775808e18) {
        return arg_type_err(vm, f, n, pname, "int", v);
    }
    Ok(x as i64)
}

pub fn p_str(vm: &mut Vm<'_>, f: &str, n: usize, pname: &str, v: &Value) -> R<std::rc::Rc<str>> {
    let v = v.deref();
    match &v {
        Value::Array(_) | Value::Resource(_) => arg_type_err(vm, f, n, pname, "string", &v),
        Value::Object(o) => {
            if o.class.find_method("__tostring").is_none() {
                return arg_type_err(vm, f, n, pname, "string", &v);
            }
            let s = vm.to_string_val(&v)?;
            Ok(s)
        }
        _ => Ok(v.to_str()),
    }
}

pub fn argc_err<T>(vm: &mut Vm<'_>, f: &str, min: usize, max: usize, given: usize) -> R<T> {
    let (kind, n) = if given < min { ("at least", min) } else { ("at most", max) };
    let kind = if min == max { "exactly" } else { kind };
    vm.throw_error("ArgumentCountError", &format!("{f}() expects {kind} {n} argument{}, {given} given", if n == 1 { "" } else { "s" }))
}

pub fn check_argc(vm: &mut Vm<'_>, f: &str, a: &[Value], min: usize, max: usize) -> R<()> {
    if a.len() < min || a.len() > max {
        return argc_err(vm, f, min, max, a.len());
    }
    Ok(())
}
