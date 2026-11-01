# Porting rule

Port php-src C functions to Rust **straightforwardly**: read the C function (e.g. `php-src/ext/standard/string.c`),
keep its structure, edge cases, argument validation and error messages, and translate it into the Value API.
Do not re-derive behaviour from tests; use phpt only to verify.

* Plain builtin: `fn(&mut Vm, Vec<Value>) -> Result<Value, String>`; `Err` becomes TypeError/ValueError/Error by message.
* VM builtin (callbacks, output, throwing precise exceptions): `fn(&mut Vm, Vec<Value>) -> Result<Value, Flow>`.
* By-reference arguments go through `refbuiltins.rs` (`ref_positions` + `call_ref`).
* Later registrations override earlier ones: put ported functions in `port_*.rs`.
