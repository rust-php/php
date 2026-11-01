//! Straight ports of php-src C functions (see docs/PORTING.md).
use crate::builtins::{Builtin, VmBuiltin};
use std::collections::HashMap;

/// Plain builtins (no VM access). Entries registered here override older reimplementations.
pub fn register(_m: &mut HashMap<&'static str, Builtin>) {}

/// Builtins that need the VM (callbacks, output, exceptions).
pub fn register_vm(_m: &mut HashMap<&'static str, VmBuiltin>) {}
