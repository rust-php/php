//! Straight ports of php-src C functions (see docs/PORTING.md).
//! Variable dumping, serialization, JSON and type functions.
use crate::builtins::{arg, Builtin, VmBuiltin};
use crate::value::*;
use crate::vm::{Flow, Vm};
use std::collections::HashMap;

mod dump;
mod json;
mod ser;
mod types;

type R<T> = Result<T, Flow>;

/// Constants defined by this port (JSON_*). The caller wires this into constant lookup.
pub fn constant(name: &str) -> Option<Value> {
    json::constant(name)
}

/// php_error_docref-style diagnostic (no line information available in the VM).
/// Off unless PHP_VAR_DIAG is set: the VM has no @-silence / error_reporting support yet, so
/// printing would break tests that suppress these diagnostics.
pub(crate) fn diag(vm: &mut Vm<'_>, level: &str, msg: &str) {
    let text = format!("\n{level}: {msg} in {} on line 0\n", vm.file);
    if std::env::var_os("PHP_VAR_DIAG").is_some() { vm.write(&text); }
}

/// Plain builtins (no VM access). Entries registered here override older reimplementations.
pub fn register(_m: &mut HashMap<&'static str, Builtin>) {}

fn var_dump(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    if a.is_empty() {
        return vm.throw_error("ArgumentCountError", "var_dump() expects at least 1 argument, 0 given");
    }
    for v in &a {
        let mut out = String::new();
        dump::var_dump(vm, v, 1, &mut vec![], &mut out)?;
        vm.write(&out);
    }
    Ok(Value::Null)
}

fn print_r(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    if a.is_empty() || a.len() > 2 {
        return vm.throw_error("ArgumentCountError", &format!("print_r() expects {} {} argument{}, {} given", if a.is_empty() { "at least" } else { "at most" }, if a.is_empty() { 1 } else { 2 }, if a.is_empty() { "" } else { "s" }, a.len()));
    }
    let mut out = String::new();
    dump::print_r(vm, &a[0], 0, &mut vec![], &mut out)?;
    if arg(&a, 1).to_bool() {
        Ok(Value::str(&out))
    } else {
        vm.write(&out);
        Ok(Value::Bool(true))
    }
}

fn var_export(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    if a.is_empty() || a.len() > 2 {
        return vm.throw_error("ArgumentCountError", &format!("var_export() expects {} {} argument{}, {} given", if a.is_empty() { "at least" } else { "at most" }, if a.is_empty() { 1 } else { 2 }, if a.is_empty() { "" } else { "s" }, a.len()));
    }
    let mut out = String::new();
    dump::var_export(vm, &a[0], 1, &mut vec![], &mut out)?;
    if arg(&a, 1).to_bool() {
        Ok(Value::str(&out))
    } else {
        vm.write(&out);
        Ok(Value::Null)
    }
}

/// Builtins that need the VM (callbacks, output, exceptions).
pub fn register_vm(m: &mut HashMap<&'static str, VmBuiltin>) {
    m.insert("var_dump", var_dump);
    m.insert("print_r", print_r);
    m.insert("var_export", var_export);
    m.insert("intval", types::intval);
    m.insert("is_scalar", types::is_scalar);
    m.insert("get_debug_type", types::get_debug_type);
    m.insert("gettype", types::gettype);
    m.insert("serialize", ser::serialize);
    m.insert("unserialize", ser::unserialize);
    m.insert("json_encode", json::json_encode);
    m.insert("json_decode", json::json_decode);
    m.insert("json_validate", json::json_validate);
    m.insert("json_last_error", json::json_last_error);
    m.insert("json_last_error_msg", json::json_last_error_msg);
}
