//! var_dump / print_r / var_export (ports of ext/standard/var.c and Zend/zend.c zend_print_zval_r).
use crate::ast::ClassKind;
use crate::object::Obj;
use crate::value::*;
use crate::vm::{Flow, Vm};
use std::rc::Rc;

type R<T> = Result<T, Flow>;

#[derive(Clone, Copy, PartialEq)]
pub enum Vis {
    Public,
    Protected,
    Private,
}

/// zend_unmangle_property_name for our (unmangled, class-annotated) property tables.
pub fn prop_label(o: &Obj, name: &str) -> (String, Vis, String) {
    if let Some(rest) = name.strip_prefix('\0') {
        if let Some(p) = rest.find('\0') {
            let (cls, prop) = (&rest[..p], &rest[p + 1..]);
            return if cls == "*" { (prop.to_string(), Vis::Protected, String::new()) } else { (prop.to_string(), Vis::Private, cls.to_string()) };
        }
    }
    match o.class.prop_vis(name) {
        Some((1, _)) => (name.to_string(), Vis::Protected, String::new()),
        Some((2, c)) => (name.to_string(), Vis::Private, c.to_string()),
        _ => (name.to_string(), Vis::Public, String::new()),
    }
}

pub fn is_enum(o: &Obj) -> bool {
    o.class.kind == ClassKind::Enum
}

pub fn enum_case(o: &Obj) -> String {
    o.props.borrow().get(&Key::from_str("name")).map(|n| n.to_str().to_string()).unwrap_or_default()
}

/// zend_get_properties_for(.., DEBUG): honours __debugInfo.
fn debug_props(vm: &mut Vm<'_>, v: &Value, o: &Rc<Obj>) -> R<Vec<(Key, Value)>> {
    if o.class.find_method("__debuginfo").is_some() {
        return match vm.call_method_named(v, "__debugInfo", vec![])? {
            Value::Array(a) => Ok(a.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            Value::Null => Ok(vec![]),
            _ => Err(Flow::Fatal("__debuginfo() must return an array".into())),
        };
    }
    Ok(o.props.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}

fn pad(out: &mut String, n: usize) {
    for _ in 0..n {
        out.push(' ');
    }
}

pub fn var_dump(vm: &mut Vm<'_>, v: &Value, level: usize, stack: &mut Vec<usize>, out: &mut String) -> R<()> {
    if level > 1 {
        pad(out, level - 1);
    }
    let mut is_ref = false;
    let mut cur = v.clone();
    let mut cell = None;
    while let Value::Ref(r) = &cur {
        if Rc::strong_count(r) > 2 {
            is_ref = true;
        }
        let p = Rc::as_ptr(r) as usize;
        if stack.contains(&p) {
            out.push_str("*RECURSION*\n");
            return Ok(());
        }
        cell = Some(p);
        let inner = r.borrow().clone();
        cur = inner;
    }
    if let Some(p) = cell {
        stack.push(p);
        let r = var_dump_inner(vm, &cur, level, stack, out, is_ref);
        stack.pop();
        return r;
    }
    var_dump_inner(vm, &cur, level, stack, out, is_ref)
}

fn var_dump_inner(vm: &mut Vm<'_>, cur: &Value, level: usize, stack: &mut Vec<usize>, out: &mut String, is_ref: bool) -> R<()> {
    let cur = cur.clone();
    let common = if is_ref { "&" } else { "" };
    match &cur {
        Value::Bool(b) => out.push_str(&format!("{common}bool({b})\n")),
        Value::Null => out.push_str(&format!("{common}NULL\n")),
        Value::Int(i) => out.push_str(&format!("{common}int({i})\n")),
        Value::Float(f) => out.push_str(&format!("{common}float({})\n", fmt_float(*f, None))),
        Value::Str(s) => out.push_str(&format!("{common}string({}) \"{s}\"\n", s.len())),
        Value::Array(a) => {
            let p = Rc::as_ptr(a) as usize;
            if stack.contains(&p) {
                out.push_str("*RECURSION*\n");
                return Ok(());
            }
            stack.push(p);
            out.push_str(&format!("{common}array({}) {{\n", a.len()));
            for (k, x) in a.iter() {
                match k {
                    Key::Int(i) => out.push_str(&format!("{}[{i}]=>\n", " ".repeat(level + 1))),
                    Key::Str(t) => out.push_str(&format!("{}[\"{t}\"]=>\n", " ".repeat(level + 1))),
                }
                var_dump(vm, x, level + 2, stack, out)?;
            }
            stack.pop();
            if level > 1 {
                pad(out, level - 1);
            }
            out.push_str("}\n");
        }
        Value::Object(o) => {
            if is_enum(o) && o.class.find_method("__debuginfo").is_none() {
                out.push_str(&format!("{common}enum({}::{})\n", o.class.name, enum_case(o)));
                return Ok(());
            }
            let p = Rc::as_ptr(o) as usize;
            if stack.contains(&p) {
                out.push_str("*RECURSION*\n");
                return Ok(());
            }
            stack.push(p);
            let props = debug_props(vm, &cur, o)?;
            if is_enum(o) {
                out.push_str(&format!("{common}enum({}::{}) ({}) {{\n", o.class.name, enum_case(o), props.len()));
            } else {
                out.push_str(&format!("{common}object({})#{} ({}) {{\n", o.class.name, o.id, props.len()));
            }
            for (k, x) in &props {
                match k {
                    Key::Int(i) => out.push_str(&format!("{}[\"{i}\"]=>\n", " ".repeat(level + 1))),
                    Key::Str(t) => {
                        let (name, vis, cls) = prop_label(o, t);
                        let lab = match vis {
                            Vis::Protected => format!("\"{name}\":protected"),
                            Vis::Private => format!("\"{name}\":\"{cls}\":private"),
                            Vis::Public => format!("\"{name}\""),
                        };
                        out.push_str(&format!("{}[{lab}]=>\n", " ".repeat(level + 1)));
                    }
                }
                var_dump(vm, x, level + 2, stack, out)?;
            }
            if level > 1 {
                pad(out, level - 1);
            }
            out.push_str("}\n");
            stack.pop();
        }
        Value::Resource(r) => {
            let kind = if r.is_closed() { "Unknown" } else { r.kind };
            out.push_str(&format!("{common}resource({}) of type ({kind})\n", r.id));
        }
        Value::Ref(_) => unreachable!(),
    }
    Ok(())
}

/// zend_print_zval_r_to_buf
pub fn print_r(vm: &mut Vm<'_>, v: &Value, indent: usize, stack: &mut Vec<usize>, out: &mut String) -> R<()> {
    match v {
        Value::Ref(r) => {
            let p = Rc::as_ptr(r) as usize;
            if stack.contains(&p) {
                out.push_str("Array\n *RECURSION*");
                return Ok(());
            }
            stack.push(p);
            let inner = r.borrow().clone();
            let res = print_r(vm, &inner, indent, stack, out);
            stack.pop();
            res
        }
        Value::Array(a) => {
            out.push_str("Array\n");
            let p = Rc::as_ptr(a) as usize;
            if stack.contains(&p) {
                out.push_str(" *RECURSION*");
                return Ok(());
            }
            stack.push(p);
            pad(out, indent);
            out.push_str("(\n");
            for (k, x) in a.iter() {
                pad(out, indent + 4);
                match k {
                    Key::Int(i) => out.push_str(&format!("[{i}] => ")),
                    Key::Str(t) => out.push_str(&format!("[{t}] => ")),
                }
                print_r(vm, x, indent + 8, stack, out)?;
                out.push('\n');
            }
            pad(out, indent);
            out.push_str(")\n");
            stack.pop();
            Ok(())
        }
        Value::Object(o) => {
            out.push_str(&o.class.name);
            if !is_enum(o) {
                out.push_str(" Object\n");
            } else {
                out.push_str(" Enum");
                if o.class.backed {
                    match o.props.borrow().get(&Key::from_str("value")) {
                        Some(Value::Int(_)) => out.push_str(":int"),
                        _ => out.push_str(":string"),
                    }
                }
                out.push('\n');
            }
            let p = Rc::as_ptr(o) as usize;
            if stack.contains(&p) {
                out.push_str(" *RECURSION*");
                return Ok(());
            }
            stack.push(p);
            let props = debug_props(vm, v, o)?;
            pad(out, indent);
            out.push_str("(\n");
            for (k, x) in &props {
                pad(out, indent + 4);
                out.push('[');
                match k {
                    Key::Int(i) => out.push_str(&i.to_string()),
                    Key::Str(t) => {
                        let (name, vis, cls) = prop_label(o, t);
                        out.push_str(&name);
                        match vis {
                            Vis::Protected => out.push_str(":protected"),
                            Vis::Private => out.push_str(&format!(":{cls}:private")),
                            Vis::Public => {}
                        }
                    }
                }
                out.push_str("] => ");
                print_r(vm, x, indent + 8, stack, out)?;
                out.push('\n');
            }
            pad(out, indent);
            out.push_str(")\n");
            stack.pop();
            Ok(())
        }
        Value::Float(f) => {
            out.push_str(&fmt_float(*f, Some(14)));
            Ok(())
        }
        other => {
            out.push_str(&other.to_str());
            Ok(())
        }
    }
}

/// php_addcslashes(s, "'\\") followed by NUL -> `' . "\0" . '`
fn export_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\'' => o.push_str("\\'"),
            '\\' => o.push_str("\\\\"),
            '\0' => o.push_str("' . \"\\0\" . '"),
            c => o.push(c),
        }
    }
    o
}

fn export_double(f: f64) -> String {
    let mut t = fmt_float(f, None);
    if f.is_finite() && !t.contains(['.', 'e', 'E']) {
        t.push_str(".0");
    }
    t
}

/// php_var_export_ex
pub fn var_export(vm: &mut Vm<'_>, v: &Value, level: usize, stack: &mut Vec<usize>, out: &mut String) -> R<()> {
    match v {
        Value::Ref(r) => {
            let p = Rc::as_ptr(r) as usize;
            if stack.contains(&p) {
                out.push_str("NULL");
                crate::port_var::diag(vm, "Warning", "var_export does not handle circular references");
                return Ok(());
            }
            stack.push(p);
            let inner = r.borrow().clone();
            let res = var_export(vm, &inner, level, stack, out);
            stack.pop();
            res
        }
        Value::Bool(b) => {
            out.push_str(if *b { "true" } else { "false" });
            Ok(())
        }
        Value::Null | Value::Resource(_) => {
            out.push_str("NULL");
            Ok(())
        }
        Value::Int(i) => {
            if *i == i64::MIN {
                out.push_str(&format!("{}-1", i + 1));
            } else {
                out.push_str(&i.to_string());
            }
            Ok(())
        }
        Value::Float(f) => {
            out.push_str(&export_double(*f));
            Ok(())
        }
        Value::Str(s) => {
            out.push('\'');
            out.push_str(&export_str(s));
            out.push('\'');
            Ok(())
        }
        Value::Array(a) => {
            let p = Rc::as_ptr(a) as usize;
            if stack.contains(&p) {
                out.push_str("NULL");
                crate::port_var::diag(vm, "Warning", "var_export does not handle circular references");
                return Ok(());
            }
            stack.push(p);
            if level > 1 {
                out.push('\n');
                pad(out, level - 1);
            }
            out.push_str("array (\n");
            for (k, x) in a.iter() {
                match k {
                    Key::Int(i) => {
                        pad(out, level + 1);
                        out.push_str(&format!("{i} => "));
                    }
                    Key::Str(t) => {
                        pad(out, level + 1);
                        out.push_str(&format!("'{}' => ", export_str(t)));
                    }
                }
                var_export(vm, x, level + 2, stack, out)?;
                out.push_str(",\n");
            }
            stack.pop();
            if level > 1 {
                pad(out, level - 1);
            }
            out.push(')');
            Ok(())
        }
        Value::Object(o) => {
            let p = Rc::as_ptr(o) as usize;
            if stack.contains(&p) {
                out.push_str("NULL");
                crate::port_var::diag(vm, "Warning", "var_export does not handle circular references");
                return Ok(());
            }
            stack.push(p);
            if level > 1 {
                out.push('\n');
                pad(out, level - 1);
            }
            let en = is_enum(o);
            let std = o.class.name == "stdClass";
            if std {
                out.push_str("(object) array(\n");
            } else {
                out.push('\\');
                out.push_str(&o.class.name);
                if en {
                    out.push_str("::");
                    out.push_str(&enum_case(o));
                } else {
                    out.push_str("::__set_state(array(\n");
                }
            }
            if !en {
                let props: Vec<(Key, Value)> = o.props.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                for (k, x) in &props {
                    pad(out, level + 2);
                    match k {
                        Key::Str(t) => {
                            let (name, _, _) = prop_label(o, t);
                            out.push_str(&format!("'{}'", export_str(&name)));
                        }
                        Key::Int(i) => out.push_str(&i.to_string()),
                    }
                    out.push_str(" => ");
                    var_export(vm, x, level + 2, stack, out)?;
                    out.push_str(",\n");
                }
            }
            stack.pop();
            if level > 1 && !en {
                pad(out, level - 1);
            }
            if std {
                out.push(')');
            } else if !en {
                out.push_str("))");
            }
            Ok(())
        }
    }
}
