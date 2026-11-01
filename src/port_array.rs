//! Straight ports of php-src C functions (see docs/PORTING.md).
use crate::builtins::{Builtin, VmBuiltin};
use crate::value::Value;
use std::collections::HashMap;

pub mod args;
pub mod math;
pub mod mt;

/// Plain builtins (no VM access). Entries registered here override older reimplementations.
pub fn register(_m: &mut HashMap<&'static str, Builtin>) {}

/// Builtins that need the VM (callbacks, output, exceptions).
pub fn register_vm(m: &mut HashMap<&'static str, VmBuiltin>) {
    math::register_vm(m);
}

/// Constants defined by the array/math/random extensions.
pub fn constant(name: &str) -> Option<Value> {
    math::constant(name)
}
