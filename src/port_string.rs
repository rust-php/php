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
#[path = "ps/core2.rs"]
mod core2;
#[path = "ps/misc.rs"]
mod misc;
#[path = "ps/strip.rs"]
mod strip;
#[path = "ps/entities.rs"]
mod entities;
#[path = "ps/html.rs"]
mod html;
#[path = "ps/eaw.rs"]
mod eaw;
#[path = "ps/mbenc.rs"]
mod mbenc;
#[path = "ps/mb.rs"]
mod mb;
#[path = "ps/scanf.rs"]
mod scanf;
#[path = "ps/numfmt.rs"]
mod numfmt;
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
    core2::register(m);
    numfmt::register(m);
    scanf::register(m);
    mb::register(m);
    html::register(m);
    misc::register(m);
    strip::register(m);
}
