//! Port of ext/ctype/ctype.c
use super::util::*;
use crate::builtins::VmBuiltin;
use crate::value::Value;
use crate::vm::Vm;
use std::collections::HashMap;

fn ctype_impl(vm: &mut Vm<'_>, name: &str, a: &[Value], iswhat: fn(u8) -> bool, allow_digits: bool, allow_minus: bool) -> R<Value> {
    argc(vm, name, a, 1, 1)?;
    let c = a[0].deref();
    if let Value::Str(s) = &c {
        if s.is_empty() {
            return Ok(Value::Bool(false));
        }
        return Ok(Value::Bool(s.bytes().all(iswhat)));
    }
    // ctype_fallback
    deprecated(vm, &format!("{name}(): Argument of type {} will be interpreted as string in the future", zname(&c)));
    if let Value::Int(n) = c {
        let r = if (0..=255).contains(&n) {
            iswhat(n as u8)
        } else if (-128..0).contains(&n) {
            iswhat((n + 256) as u8)
        } else if n >= 0 {
            allow_digits
        } else {
            allow_minus
        };
        return Ok(Value::Bool(r));
    }
    Ok(Value::Bool(false))
}

macro_rules! ct {
    ($m:expr, $name:literal, $f:expr, $d:expr, $mi:expr) => {
        $m.insert($name, |vm, a| ctype_impl(vm, $name, &a, $f, $d, $mi));
    };
}

pub fn register(m: &mut HashMap<&'static str, VmBuiltin>) {
    ct!(m, "ctype_alnum", |c| c.is_ascii_alphanumeric(), true, false);
    ct!(m, "ctype_alpha", |c| c.is_ascii_alphabetic(), false, false);
    ct!(m, "ctype_cntrl", |c| c.is_ascii_control(), false, false);
    ct!(m, "ctype_digit", |c| c.is_ascii_digit(), true, false);
    ct!(m, "ctype_lower", |c| c.is_ascii_lowercase(), false, false);
    ct!(m, "ctype_graph", |c| c.is_ascii_graphic(), true, true);
    ct!(m, "ctype_print", |c| c.is_ascii_graphic() || c == b' ', true, true);
    ct!(m, "ctype_punct", |c| c.is_ascii_punctuation(), false, false);
    ct!(m, "ctype_space", |c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c), false, false);
    ct!(m, "ctype_upper", |c| c.is_ascii_uppercase(), false, false);
    ct!(m, "ctype_xdigit", |c| c.is_ascii_hexdigit(), true, false);
}
