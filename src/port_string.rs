//! Straight ports of php-src C functions (see docs/PORTING.md).
//! String functions: ext/standard/{string,formatted_print,html,url,...}.c, ext/ctype, ext/mbstring (UTF-8 only).
//! NOTE: the engine's strings are UTF-8 `Rc<str>`; bytes >= 0x80 that do not form valid UTF-8 are replaced lossily.
use crate::builtins::{Builtin, VmBuiltin};
use std::collections::HashMap;

#[path = "ps/util.rs"]
pub mod util;
#[path = "ps/ctype.rs"]
mod ctype;
#[path = "ps/core1.rs"]
mod core1;
#[path = "ps/fmt.rs"]
mod fmt;
#[path = "ps/consts.rs"]
mod consts;

pub fn constant(name: &str) -> Option<crate::value::Value> {
    consts::constant(name)
}

/// Plain builtins (no VM access). Entries registered here override older reimplementations.
pub fn register(_m: &mut HashMap<&'static str, Builtin>) {}

/// Builtins that need the VM (callbacks, output, exceptions).
pub fn register_vm(m: &mut HashMap<&'static str, VmBuiltin>) {
    ctype::register(m);
    fmt::register(m);
    core1::register(m);
}
