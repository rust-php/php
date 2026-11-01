//! Port of ext/standard/type.c (intval with base, is_scalar, get_debug_type, ...).
use crate::value::*;
use crate::vm::{Flow, Vm};

type R<T> = Result<T, Flow>;

/// C strtol(): leading whitespace, sign, optional base prefix, saturating.
pub fn strtol(s: &str, mut base: u32) -> i64 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let mut neg = false;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        neg = b[i] == b'-';
        i += 1;
    }
    let has_hex = i + 1 < b.len() && b[i] == b'0' && (b[i + 1] == b'x' || b[i + 1] == b'X');
    if (base == 0 || base == 16) && has_hex && b.get(i + 2).is_some_and(|c| (*c as char).is_digit(16)) {
        i += 2;
        base = 16;
    } else if base == 0 {
        base = if b.get(i) == Some(&b'0') { 8 } else { 10 };
    }
    let mut acc: i128 = 0;
    while i < b.len() {
        let Some(d) = (b[i] as char).to_digit(base) else { break };
        acc = acc * base as i128 + d as i128;
        if acc > (i64::MAX as i128) + 1 {
            acc = (i64::MAX as i128) + 1;
        }
        i += 1;
    }
    let v = if neg { -acc } else { acc };
    v.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn check_count(vm: &mut Vm<'_>, name: &str, a: &[Value], min: usize, max: usize) -> R<()> {
    if a.len() < min || a.len() > max {
        let (w, k) = if a.len() < min { ("at least", min) } else { ("at most", max) };
        let (w, k) = if min == max { ("exactly", min) } else { (w, k) };
        return vm.throw_error("ArgumentCountError", &format!("{name}() expects {w} {k} argument{}, {} given", if k == 1 { "" } else { "s" }, a.len()));
    }
    Ok(())
}

pub fn intval(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_count(vm, "intval", &a, 1, 2)?;
    let num = a[0].deref();
    let base = a.get(1).map(Value::to_int).unwrap_or(10);
    let Value::Str(s) = &num else { return Ok(Value::Int(num.to_int())) };
    if base == 10 {
        return Ok(Value::Int(num.to_int()));
    }
    if base == 0 || base == 2 {
        let t = s.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '\x0b');
        if t.len() > 2 {
            let offset = if t.starts_with(['-', '+']) { 1 } else { 0 };
            let tb = t.as_bytes();
            if tb[offset] == b'0' && matches!(tb.get(offset + 1), Some(b'b' | b'B')) {
                let digits = &t[offset + 2..];
                let v = strtol(digits, 2);
                return Ok(Value::Int(if t.starts_with('-') { v.wrapping_neg() } else { v }));
            }
        }
    }
    Ok(Value::Int(strtol(s, base as u32)))
}

pub fn is_scalar(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_count(vm, "is_scalar", &a, 1, 1)?;
    Ok(Value::Bool(matches!(a[0].deref(), Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_))))
}

pub fn get_debug_type(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_count(vm, "get_debug_type", &a, 1, 1)?;
    Ok(Value::str(&match a[0].deref() {
        Value::Null => "null".to_string(),
        Value::Bool(_) => "bool".to_string(),
        Value::Int(_) => "int".to_string(),
        Value::Float(_) => "float".to_string(),
        Value::Str(_) => "string".to_string(),
        Value::Array(_) => "array".to_string(),
        Value::Object(o) => {
            if o.class.name.contains("@anonymous") {
                match &o.class.parent {
                    Some(p) => format!("{}@anonymous", p.name),
                    None => "class@anonymous".to_string(),
                }
            } else {
                o.class.name.clone()
            }
        }
        Value::Resource(r) => {
            if r.is_closed() {
                "resource (closed)".to_string()
            } else {
                format!("resource ({})", r.kind)
            }
        }
        Value::Ref(_) => "reference".to_string(),
    }))
}

pub fn gettype(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_count(vm, "gettype", &a, 1, 1)?;
    Ok(Value::str(match a[0].deref() {
        Value::Null => "NULL",
        Value::Bool(_) => "boolean",
        Value::Int(_) => "integer",
        Value::Float(_) => "double",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Ref(_) => "unknown type",
        Value::Resource(r) => {
            if r.is_closed() {
                "resource (closed)"
            } else {
                "resource"
            }
        }
    }))
}
