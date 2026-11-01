//! serialize() / unserialize() (port of ext/standard/var.c and var_unserializer.re).
use super::dump::{enum_case, is_enum};
use crate::ast::ClassKind;
use crate::builtins::arg;
use crate::object::{Class, Extra, Obj};
use crate::port_var::diag;
use crate::value::*;
use crate::vm::{Flow, Vm};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

type R<T> = Result<T, Flow>;

// ------------------------------------------------------------------ serialize

struct Ser {
    buf: String,
    n: i64,
    /// pointer -> slot index (-1: failed object)
    seen: HashMap<usize, i64>,
    /// objects kept alive so addresses are not reused
    keep: Vec<Value>,
}

fn mangle(vis: u8, cls: &str, name: &str) -> String {
    match vis {
        1 => format!("\0*\0{name}"),
        2 => format!("\0{cls}\0{name}"),
        _ => name.to_string(),
    }
}

fn ser_string(buf: &mut String, s: &str) {
    buf.push_str(&format!("s:{}:\"{}\";", s.len(), s));
}

fn ser_key(buf: &mut String, k: &Key) {
    match k {
        Key::Int(i) => buf.push_str(&format!("i:{i};")),
        Key::Str(s) => ser_string(buf, s),
    }
}

impl Ser {
    /// php_add_var_hash. `shared` says whether a reference cell is really shared.
    fn add_var_hash(&mut self, v: &Value, shared_ref: bool) -> i64 {
        self.n += 1;
        let (key, is_ref) = match v {
            Value::Ref(r) if shared_ref => match &*r.borrow() {
                Value::Object(o) => (Rc::as_ptr(o) as usize, true),
                _ => (Rc::as_ptr(r) as *const u8 as usize, true),
            },
            Value::Object(o) => (Rc::as_ptr(o) as usize, false),
            _ => return 0,
        };
        if let Some(&idx) = self.seen.get(&key) {
            if is_ref && idx != -1 {
                self.n -= 1;
            }
            return idx;
        }
        self.seen.insert(key, self.n);
        self.keep.push(v.clone());
        0
    }

    fn mark_failed(&mut self, o: &Rc<Obj>) {
        self.seen.insert(Rc::as_ptr(o) as usize, -1);
    }
}

fn not_allowed(vm: &mut Vm<'_>, name: &str) -> Flow {
    match vm.make_throwable("Exception", &format!("Serialization of '{name}' is not allowed")) {
        Ok(e) => Flow::Throw(e),
        Err(f) => f,
    }
}

/// `extra_shared`: number of extra strong refs held by the caller's snapshot of this value.
fn ser_intern(vm: &mut Vm<'_>, s: &mut Ser, v: &Value, extra: usize) -> R<()> {
    // unwrap refcount==1 references (done by callers in C before the call)
    let shared = matches!(v, Value::Ref(r) if Rc::strong_count(r) > 1 + extra);
    let v = match v {
        Value::Ref(r) if !shared => r.borrow().clone(),
        _ => v.clone(),
    };
    let already = s.add_var_hash(&v, shared);
    if already != 0 {
        if already == -1 {
            s.buf.push_str("N;");
            return Ok(());
        } else if matches!(v, Value::Ref(_)) {
            s.buf.push_str(&format!("R:{already};"));
            return Ok(());
        } else if matches!(v, Value::Object(_)) {
            s.buf.push_str(&format!("r:{already};"));
            return Ok(());
        }
    }
    let mut cur = v;
    while let Value::Ref(r) = &cur {
        let inner = r.borrow().clone();
        cur = inner;
    }
    match &cur {
        Value::Null => s.buf.push_str("N;"),
        Value::Bool(b) => s.buf.push_str(&format!("b:{};", *b as i32)),
        Value::Int(i) => s.buf.push_str(&format!("i:{i};")),
        Value::Float(f) => s.buf.push_str(&format!("d:{};", fmt_float(*f, None))),
        Value::Str(t) => ser_string(&mut s.buf, t),
        Value::Resource(_) => s.buf.push_str("i:0;"),
        Value::Array(a) => {
            s.buf.push_str(&format!("a:{}:{{", a.len()));
            let a = a.clone();
            for (k, x) in a.iter() {
                ser_key(&mut s.buf, k);
                ser_intern(vm, s, x, 0)?;
            }
            s.buf.push('}');
        }
        Value::Object(o) => ser_object(vm, s, &cur, o)?,
        Value::Ref(_) => unreachable!(),
    }
    Ok(())
}

fn class_header(buf: &mut String, o: &Obj) -> bool {
    let incomplete = o.class.name == "__PHP_Incomplete_Class";
    let name = if incomplete {
        o.props.borrow().get(&Key::from_str("__PHP_Incomplete_Class_Name")).map(|v| v.to_str().to_string()).unwrap_or_else(|| o.class.name.clone())
    } else {
        o.class.name.clone()
    };
    buf.push_str(&format!("O:{}:\"{}\":", name.len(), name));
    incomplete
}

fn ser_nested(vm: &mut Vm<'_>, s: &mut Ser, items: &[(Key, Value)], skip_incomplete: bool) -> R<()> {
    let count = items.len() - if skip_incomplete && items.iter().any(|(k, _)| matches!(k, Key::Str(n) if &**n == "__PHP_Incomplete_Class_Name")) { 1 } else { 0 };
    s.buf.push_str(&format!("{count}:{{"));
    for (k, x) in items {
        if skip_incomplete && matches!(k, Key::Str(n) if &**n == "__PHP_Incomplete_Class_Name") {
            continue;
        }
        ser_key(&mut s.buf, k);
        ser_intern(vm, s, x, 1)?;
    }
    s.buf.push('}');
    Ok(())
}

fn ser_object(vm: &mut Vm<'_>, s: &mut Ser, v: &Value, o: &Rc<Obj>) -> R<()> {
    if !matches!(&*o.extra.borrow(), Extra::None) {
        return Err(not_allowed(vm, &o.class.name));
    }
    if matches!(o.class.name.as_str(), "Closure" | "Generator" | "Fiber" | "WeakReference" | "WeakMap") {
        return Err(not_allowed(vm, &o.class.name));
    }
    if is_enum(o) {
        let case = enum_case(o);
        s.buf.push_str(&format!("E:{}:\"{}:{}\";", o.class.name.len() + 1 + case.len(), o.class.name, case));
        return Ok(());
    }
    if o.class.find_method("__serialize").is_some() {
        let ret = vm.call_method_named(v, "__serialize", vec![])?;
        let Value::Array(a) = ret else {
            return vm.throw_error("TypeError", &format!("{}::__serialize() must return an array", o.class.name));
        };
        class_header(&mut s.buf, o);
        s.buf.push_str(&format!("{}:{{", a.len()));
        for (k, x) in a.iter() {
            ser_key(&mut s.buf, k);
            ser_intern(vm, s, x, 0)?;
        }
        s.buf.push('}');
        return Ok(());
    }
    if o.class.is_a("serializable") && o.class.find_method("serialize").is_some() {
        match vm.call_method_named(v, "serialize", vec![])? {
            Value::Str(d) => {
                s.buf.push_str(&format!("C:{}:\"{}\":{}:{{{}}}", o.class.name.len(), o.class.name, d.len(), d));
            }
            Value::Null => {
                s.mark_failed(o);
                s.buf.push_str("N;");
            }
            _ => {
                return Err(Flow::Throw(vm.make_throwable("Exception", &format!("{}::serialize() must return a string or NULL", o.class.name))?));
            }
        }
        return Ok(());
    }
    if o.class.find_method("__sleep").is_some() {
        let ret = vm.call_method_named(v, "__sleep", vec![])?;
        let Value::Array(names) = ret else {
            diag(vm, "Warning", &format!("serialize(): {}::__sleep() should return an array only containing the names of instance-variables to serialize", o.class.name));
            s.buf.push_str("N;");
            return Ok(());
        };
        let mut items: Vec<(Key, Value)> = vec![];
        let mut seen_names: Vec<String> = vec![];
        for (_, nv) in names.iter() {
            let nv = nv.deref();
            if !matches!(nv, Value::Str(_)) {
                diag(vm, "Warning", &format!("serialize(): {}::__sleep() should return an array only containing the names of instance-variables to serialize", o.class.name));
            }
            let name = nv.to_str().to_string();
            let found = o.props.borrow().get(&Key::from_str(&name)).cloned();
            let found = found.or_else(|| o.props.borrow().get(&Key::Str(name.as_str().into())).cloned());
            match found {
                Some(val) => {
                    if seen_names.contains(&name) {
                        diag(vm, "Warning", &format!("serialize(): \"{name}\" is returned from __sleep() multiple times"));
                        continue;
                    }
                    seen_names.push(name.clone());
                    let (vis, cls) = o.class.prop_vis(&name).map(|(v, c)| (v, c.to_string())).unwrap_or((0, String::new()));
                    items.push((Key::Str(mangle(vis, &cls, &name).as_str().into()), val));
                }
                None => {
                    // typed uninitialized properties are silently skipped
                    if o.class.prop_info(&name).is_some_and(|p| p.ty.is_some()) {
                        continue;
                    }
                    diag(vm, "Warning", &format!("serialize(): \"{name}\" returned as member variable from __sleep() but does not exist"));
                }
            }
        }
        class_header(&mut s.buf, o);
        return ser_nested(vm, s, &items, false);
    }
    let incomplete = class_header(&mut s.buf, o);
    let items: Vec<(Key, Value)> = o
        .props
        .borrow()
        .iter()
        .map(|(k, x)| match k {
            Key::Str(n) if !n.starts_with('\0') => {
                let (vis, cls) = o.class.prop_vis(n).map(|(v, c)| (v, c.to_string())).unwrap_or((0, String::new()));
                (Key::Str(mangle(vis, &cls, n).as_str().into()), x.clone())
            }
            _ => (k.clone(), x.clone()),
        })
        .collect();
    ser_nested(vm, s, &items, incomplete)
}

pub fn serialize(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    if a.len() != 1 {
        return vm.throw_error("ArgumentCountError", &format!("serialize() expects exactly 1 argument, {} given", a.len()));
    }
    let mut s = Ser { buf: String::new(), n: 0, seen: HashMap::new(), keep: vec![] };
    ser_intern(vm, &mut s, &a[0], 0)?;
    Ok(Value::str(&s.buf))
}

// ---------------------------------------------------------------- unserialize

enum Slot {
    Val(Value),
    Arr(Vec<(Key, Rc<Node>)>),
    Obj(Rc<Obj>),
    Copy(Rc<Node>),
}

struct Node {
    slot: RefCell<Slot>,
    is_ref: Cell<bool>,
    fin: RefCell<Option<Value>>,
}

fn node(s: Slot) -> Rc<Node> {
    Rc::new(Node { slot: RefCell::new(s), is_ref: Cell::new(false), fin: RefCell::new(None) })
}

enum Pending {
    Props(Rc<Obj>, Vec<(Key, Rc<Node>)>, bool),
    Unser(Rc<Obj>, Vec<(Key, Rc<Node>)>),
}

struct Unser<'a> {
    b: &'a [u8],
    pos: usize,
    err: usize,
    vars: Vec<Rc<Node>>,
    pending: Vec<Pending>,
    allowed: Option<Vec<String>>, // None: all allowed; Some(list): only those (empty = none)
    max_depth: i64,
    depth: i64,
}

struct Fail;

fn is_class_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'\\' || c >= 0x80
}

impl<'a> Unser<'a> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn expect(&mut self, c: u8) -> Result<(), Fail> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(Fail)
        }
    }

    fn read_int(&mut self, allow_sign: bool) -> Result<i64, Fail> {
        let start = self.pos;
        if allow_sign && matches!(self.peek(), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        let ds = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == ds {
            self.pos = start;
            return Err(Fail);
        }
        std::str::from_utf8(&self.b[start..self.pos]).ok().and_then(|t| t.trim_start_matches('+').parse::<i64>().ok()).ok_or(Fail)
    }

    fn read_len_str(&mut self) -> Result<String, Fail> {
        // after "X:" : len ':' '"' bytes '"'
        let len = self.read_int(false)?;
        self.expect(b':')?;
        self.expect(b'"')?;
        let len = len as usize;
        if self.pos + len > self.b.len() {
            return Err(Fail);
        }
        let s = std::str::from_utf8(&self.b[self.pos..self.pos + len]).map_err(|_| Fail)?.to_string();
        self.pos += len;
        self.expect(b'"')?;
        Ok(s)
    }

    fn key(&mut self) -> Result<Key, Fail> {
        let start = self.pos;
        let r = (|| -> Result<Key, Fail> {
            match self.peek() {
                Some(b'i') => {
                    self.pos += 1;
                    self.expect(b':')?;
                    let i = self.read_int(true)?;
                    self.expect(b';')?;
                    Ok(Key::Int(i))
                }
                Some(b's') => {
                    self.pos += 1;
                    self.expect(b':')?;
                    let s = self.read_len_str()?;
                    self.expect(b';')?;
                    Ok(Key::from_str(&s))
                }
                _ => Err(Fail),
            }
        })();
        if r.is_err() {
            self.err = start;
        }
        r
    }

    fn push(&mut self, n: &Rc<Node>) {
        self.vars.push(n.clone());
    }

    fn value(&mut self, vm: &mut Vm<'_>) -> R<Result<Rc<Node>, Fail>> {
        let start = self.pos;
        match self.value_inner(vm, start) {
            Ok(Ok(n)) => Ok(Ok(n)),
            Ok(Err(f)) => {
                if self.err == usize::MAX {
                    self.err = start;
                }
                Ok(Err(f))
            }
            Err(e) => Err(e),
        }
    }

    fn value_inner(&mut self, vm: &mut Vm<'_>, start: usize) -> R<Result<Rc<Node>, Fail>> {
        macro_rules! t {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(f) => return Ok(Err(f)),
                }
            };
        }
        let Some(tag) = self.peek() else { return Ok(Err(Fail)) };
        if self.b.get(self.pos + 1) != Some(&b':') && !(tag == b'N' && self.b.get(self.pos + 1) == Some(&b';')) {
            return Ok(Err(Fail));
        }
        let simple = |s: &mut Self, v: Value| -> Rc<Node> {
            let n = node(Slot::Val(v));
            s.push(&n);
            n
        };
        match tag {
            b'N' => {
                self.pos += 2;
                Ok(Ok(simple(self, Value::Null)))
            }
            b'b' => {
                self.pos += 2;
                let v = match self.peek() {
                    Some(b'0') => false,
                    Some(b'1') => true,
                    _ => return Ok(Err(Fail)),
                };
                self.pos += 1;
                t!(self.expect(b';'));
                Ok(Ok(simple(self, Value::Bool(v))))
            }
            b'i' => {
                self.pos += 2;
                let i = t!(self.read_int(true));
                t!(self.expect(b';'));
                Ok(Ok(simple(self, Value::Int(i))))
            }
            b'd' => {
                self.pos += 2;
                let rest = &self.b[self.pos..];
                let end = rest.iter().position(|&c| c == b';');
                let Some(end) = end else { return Ok(Err(Fail)) };
                let txt = std::str::from_utf8(&rest[..end]).unwrap_or("");
                let f = match txt {
                    "NAN" => f64::NAN,
                    "INF" => f64::INFINITY,
                    "-INF" => f64::NEG_INFINITY,
                    t => {
                        let ok = !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.' | b'e' | b'E'));
                        if !ok {
                            return Ok(Err(Fail));
                        }
                        match t.parse::<f64>() {
                            Ok(f) => f,
                            Err(_) => return Ok(Err(Fail)),
                        }
                    }
                };
                self.pos += end + 1;
                Ok(Ok(simple(self, Value::Float(f))))
            }
            b's' => {
                self.pos += 2;
                let s = t!(self.read_len_str());
                t!(self.expect(b';'));
                Ok(Ok(simple(self, Value::Str(s.as_str().into()))))
            }
            b'S' => Ok(Err(Fail)),
            b'a' => {
                self.pos += 2;
                let n = t!(self.read_int(false));
                t!(self.expect(b':'));
                t!(self.expect(b'{'));
                if n as usize > self.b.len() - self.pos {
                    return Ok(Err(Fail));
                }
                let nd = node(Slot::Arr(vec![]));
                self.push(&nd);
                self.depth += 1;
                let mut items: Vec<(Key, Rc<Node>)> = vec![];
                let r = self.elements(vm, n, &mut items, false)?;
                self.depth -= 1;
                t!(r);
                t!(self.expect(b'}'));
                *nd.slot.borrow_mut() = Slot::Arr(items);
                Ok(Ok(nd))
            }
            b'r' | b'R' => {
                self.pos += 2;
                let id = t!(self.read_int(false));
                t!(self.expect(b';'));
                if id < 1 || id as usize > self.vars.len() {
                    return Ok(Err(Fail));
                }
                let target = self.vars[id as usize - 1].clone();
                if tag == b'R' {
                    target.is_ref.set(true);
                    Ok(Ok(target))
                } else {
                    let n = if let Slot::Obj(_) = &*target.slot.borrow() { node(Slot::Copy(target.clone())) } else { node(Slot::Copy(target.clone())) };
                    self.push(&n);
                    Ok(Ok(n))
                }
            }
            b'E' => {
                self.pos += 2;
                let s = t!(self.read_len_str());
                t!(self.expect(b';'));
                let Some((cn, case)) = s.split_once(':') else { return Ok(Err(Fail)) };
                let Some(cls) = vm.classes.get(&cn.to_ascii_lowercase()).cloned() else {
                    diag(vm, "Warning", &format!("unserialize(): Class '{cn}' not found"));
                    return Ok(Err(Fail));
                };
                if cls.kind != ClassKind::Enum {
                    diag(vm, "Warning", &format!("unserialize(): Class '{cn}' is not an enum"));
                    return Ok(Err(Fail));
                }
                let vals = vm.enum_values(&cls)?;
                match vals.into_iter().find(|(n, _)| n == case) {
                    Some((_, v)) => Ok(Ok(simple(self, v))),
                    None => {
                        diag(vm, "Warning", &format!("unserialize(): Undefined constant {cn}::{case}"));
                        Ok(Err(Fail))
                    }
                }
            }
            b'O' | b'C' => {
                let _ = start;
                self.object(vm, tag)
            }
            _ => Ok(Err(Fail)),
        }
    }

    fn elements(&mut self, vm: &mut Vm<'_>, n: i64, items: &mut Vec<(Key, Rc<Node>)>, _obj: bool) -> R<Result<(), Fail>> {
        for _ in 0..n {
            let k = match self.key() {
                Ok(k) => k,
                Err(f) => return Ok(Err(f)),
            };
            let v = match self.value(vm)? {
                Ok(v) => v,
                Err(f) => return Ok(Err(f)),
            };
            if let Some(slot) = items.iter_mut().find(|(ek, _)| *ek == k) {
                slot.1 = v;
            } else {
                items.push((k, v));
            }
        }
        Ok(Ok(()))
    }

    fn object(&mut self, vm: &mut Vm<'_>, tag: u8) -> R<Result<Rc<Node>, Fail>> {
        macro_rules! t {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(f) => return Ok(Err(f)),
                }
            };
        }
        self.pos += 2;
        let len = t!(self.read_int(false)) as usize;
        t!(self.expect(b':'));
        t!(self.expect(b'"'));
        if self.pos + len > self.b.len() {
            return Ok(Err(Fail));
        }
        let name = t!(std::str::from_utf8(&self.b[self.pos..self.pos + len]).map_err(|_| Fail)).to_string();
        if !name.bytes().all(is_class_char) || name.is_empty() {
            return Ok(Err(Fail));
        }
        self.pos += len;
        t!(self.expect(b'"'));
        t!(self.expect(b':'));
        let allowed = self.allowed.as_ref().is_none_or(|l| l.iter().any(|a| a.eq_ignore_ascii_case(&name)));
        let class = if allowed { vm.classes.get(&name.to_ascii_lowercase()).cloned() } else { None };
        let (class, incomplete) = match class {
            Some(c) => (c, false),
            None => (vm.find_class("__PHP_Incomplete_Class")?, true),
        };
        if tag == b'C' {
            let dlen = t!(self.read_int(false)) as usize;
            t!(self.expect(b':'));
            t!(self.expect(b'{'));
            if self.pos + dlen > self.b.len() {
                return Ok(Err(Fail));
            }
            let data = t!(std::str::from_utf8(&self.b[self.pos..self.pos + dlen]).map_err(|_| Fail)).to_string();
            self.pos += dlen;
            t!(self.expect(b'}'));
            if incomplete || !class.is_a("serializable") || class.find_method("unserialize").is_none() {
                diag(vm, "Warning", &format!("Class {name} has no unserializer"));
                let o = new_object(vm, &class)?;
                let n = node(Slot::Obj(o));
                self.push(&n);
                return Ok(Ok(n));
            }
            let o = new_object(vm, &class)?;
            let ov = Value::Object(o.clone());
            let n = node(Slot::Obj(o));
            self.push(&n);
            vm.call_method_named(&ov, "unserialize", vec![Value::str(&data)])?;
            return Ok(Ok(n));
        }
        let cnt = t!(self.read_int(false));
        t!(self.expect(b':'));
        t!(self.expect(b'{'));
        if cnt as usize > self.b.len() - self.pos {
            return Ok(Err(Fail));
        }
        if class.kind != ClassKind::Class || class.is_abstract {
            return Ok(Err(Fail));
        }
        if matches!(class.name.as_str(), "Closure" | "Generator" | "Fiber" | "WeakReference" | "WeakMap") {
            return Err(Flow::Throw(vm.make_throwable("Exception", &format!("Unserialization of '{}' is not allowed", class.name))?));
        }
        let o = new_object(vm, &class)?;
        let n = node(Slot::Obj(o.clone()));
        self.push(&n);
        let mut items: Vec<(Key, Rc<Node>)> = vec![];
        if incomplete {
            items.push((Key::from_str("__PHP_Incomplete_Class_Name"), node(Slot::Val(Value::str(&name)))));
        }
        self.depth += 1;
        let r = self.elements(vm, cnt, &mut items, true)?;
        self.depth -= 1;
        t!(r);
        t!(self.expect(b'}'));
        if class.find_method("__unserialize").is_some() {
            self.pending.push(Pending::Unser(o, items));
        } else {
            let wake = class.find_method("__wakeup").is_some();
            self.pending.push(Pending::Props(o, items, wake));
        }
        Ok(Ok(n))
    }
}

/// object_init_ex without running the constructor.
fn new_object(vm: &mut Vm<'_>, class: &Rc<Class>) -> R<Rc<Obj>> {
    let mut props = PhpArray::new();
    for p in &class.props {
        if p.thunk.is_none() && p.ty.is_some() {
            continue;
        }
        let v = match p.thunk {
            Some(t) => {
                let owner = vm.classes.get(&p.class.to_ascii_lowercase()).cloned().unwrap_or_else(|| class.clone());
                vm.call_thunk(t, &owner)?
            }
            None => Value::Null,
        };
        props.set(Key::from_str(&p.name), v);
    }
    Ok(Obj::new(class.clone(), props, Extra::None))
}

fn finalize(n: &Rc<Node>) -> Value {
    if let Some(v) = n.fin.borrow().clone() {
        return v;
    }
    if n.is_ref.get() {
        let cell = Rc::new(RefCell::new(Value::Null));
        *n.fin.borrow_mut() = Some(Value::Ref(cell.clone()));
        let inner = finalize_inner(n);
        *cell.borrow_mut() = inner;
        return Value::Ref(cell);
    }
    let v = finalize_inner(n);
    *n.fin.borrow_mut() = Some(v.clone());
    v
}

fn finalize_inner(n: &Rc<Node>) -> Value {
    let slot = n.slot.borrow();
    match &*slot {
        Slot::Val(v) => v.clone(),
        Slot::Obj(o) => Value::Object(o.clone()),
        Slot::Copy(t) => finalize(t).deref(),
        Slot::Arr(items) => {
            let mut a = PhpArray::new();
            for (k, c) in items {
                a.set(k.clone(), finalize(c));
            }
            Value::Array(Rc::new(a))
        }
    }
}

fn unmangle(k: &Key) -> Key {
    if let Key::Str(s) = k {
        if let Some(rest) = s.strip_prefix('\0') {
            if let Some(p) = rest.find('\0') {
                return Key::Str(rest[p + 1..].into());
            }
        }
    }
    k.clone()
}

pub fn unserialize(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    if a.is_empty() || a.len() > 2 {
        let (w, k) = if a.is_empty() { ("at least", 1) } else { ("at most", 2) };
        return vm.throw_error("ArgumentCountError", &format!("unserialize() expects {w} {k} argument{}, {} given", if k == 1 { "" } else { "s" }, a.len()));
    }
    let text = a[0].to_str();
    let mut allowed: Option<Vec<String>> = None;
    let mut max_depth = 4096i64;
    if let Some(opts) = a.get(1) {
        let Value::Array(opts) = opts.deref() else {
            return vm.throw_error("TypeError", &format!("unserialize(): Argument #2 ($options) must be of type array, {} given", opts.deref().type_name()));
        };
        if let Some(ac) = opts.get(&Key::from_str("allowed_classes")) {
            match ac.deref() {
                Value::Bool(true) => {}
                Value::Bool(false) => allowed = Some(vec![]),
                Value::Array(l) => allowed = Some(l.iter().map(|(_, v)| v.to_str().to_string()).collect()),
                other => {
                    return vm.throw_error("TypeError", &format!("unserialize(): Option \"allowed_classes\" must be of type array|bool, {} given", other.type_name()));
                }
            }
        }
        if let Some(md) = opts.get(&Key::from_str("max_depth")) {
            match md.deref() {
                Value::Int(i) => {
                    if i < 0 {
                        return vm.throw_error("ValueError", "unserialize(): Option \"max_depth\" must be greater than or equal to 0");
                    }
                    max_depth = i;
                }
                other => {
                    return vm.throw_error("TypeError", &format!("unserialize(): Option \"max_depth\" must be of type int, {} given", other.type_name()));
                }
            }
        }
    }
    if text.is_empty() {
        return Ok(Value::Bool(false));
    }
    let mut u = Unser { b: text.as_bytes(), pos: 0, err: usize::MAX, vars: vec![], pending: vec![], allowed, max_depth, depth: 0 };
    let _ = u.max_depth;
    let root = match u.value(vm)? {
        Ok(r) => r,
        Err(_) => {
            diag(vm, "Notice", &format!("unserialize(): Error at offset {} of {} bytes", if u.err == usize::MAX { u.pos } else { u.err }, text.len()));
            return Ok(Value::Bool(false));
        }
    };
    let result = finalize(&root).deref();
    // populate objects, then run delayed __wakeup / __unserialize calls in order
    let pending = std::mem::take(&mut u.pending);
    let mut calls: Vec<(Value, &'static str, Vec<Value>)> = vec![];
    for p in &pending {
        match p {
            Pending::Props(o, items, wake) => {
                let mut props = o.props.borrow_mut();
                for (k, c) in items {
                    let key = unmangle(k);
                    let key = match &key {
                        Key::Int(i) => Key::Str(i.to_string().as_str().into()),
                        k => k.clone(),
                    };
                    props.set(key, finalize(c));
                }
                drop(props);
                if *wake {
                    calls.push((Value::Object(o.clone()), "__wakeup", vec![]));
                }
            }
            Pending::Unser(o, items) => {
                let mut arr = PhpArray::new();
                for (k, c) in items {
                    arr.set(k.clone(), finalize(c));
                }
                calls.push((Value::Object(o.clone()), "__unserialize", vec![Value::Array(Rc::new(arr))]));
            }
        }
    }
    for (o, m, args) in calls {
        vm.call_method_named(&o, m, args)?;
    }
    if u.pos < u.b.len() {
        diag(vm, "Warning", &format!("unserialize(): Extra data starting at offset {} of {} bytes", u.pos, text.len()));
    }
    let _ = arg;
    Ok(result)
}
