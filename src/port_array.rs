//! Straight ports of php-src C functions (see docs/PORTING.md).
use crate::builtins::{Builtin, VmBuiltin};
use crate::value::Value;
use std::collections::HashMap;

pub mod args;
pub mod math;
pub mod misc;
pub mod mt;
pub mod range;
pub mod refs;
pub mod setops;
pub mod sort;

/// Plain builtins (no VM access). Entries registered here override older reimplementations.
pub fn register(_m: &mut HashMap<&'static str, Builtin>) {}

/// Builtins that need the VM (callbacks, output, exceptions).
pub fn register_vm(m: &mut HashMap<&'static str, VmBuiltin>) {
    math::register_vm(m);
    setops::register_vm(m);
    misc::register_vm(m);
}

/// Constants defined by the array/math/random extensions.
pub fn constant(name: &str) -> Option<Value> {
    if let Some(v) = math::constant(name) {
        return Some(v);
    }
    let i = |x: i64| Some(Value::Int(x));
    match name {
        "SORT_ASC" => i(4),
        "SORT_DESC" => i(3),
        "SORT_LOCALE_STRING" => i(5),
        "SORT_NATURAL" => i(6),
        "SORT_FLAG_CASE" => i(8),
        "COUNT_NORMAL" => i(0),
        "CASE_LOWER" => i(0),
        "CASE_UPPER" => i(1),
        "EXTR_OVERWRITE" => i(0),
        "EXTR_SKIP" => i(1),
        "EXTR_PREFIX_SAME" => i(2),
        "EXTR_PREFIX_ALL" => i(3),
        "EXTR_PREFIX_INVALID" => i(4),
        "EXTR_PREFIX_IF_EXISTS" => i(5),
        "EXTR_IF_EXISTS" => i(6),
        "EXTR_REFS" => i(256),
        _ => None,
    }
}
