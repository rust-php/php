//! json_encode/json_decode and serialize/unserialize.
use crate::builtins::{arg, VmBuiltin};
use crate::object::{Extra, Obj};
use crate::value::*;
use crate::vm::{Flow, Vm};
use std::{collections::HashMap, rc::Rc};

type R<T> = Result<T, Flow>;

const PRETTY: i64 = 128;
const UNESCAPED_SLASHES: i64 = 64;
const UNESCAPED_UNICODE: i64 = 256;
const FORCE_OBJECT: i64 = 16;
const PRESERVE_ZERO: i64 = 1024;
const THROW: i64 = 4194304;
const PARTIAL: i64 = 512;
const NUMERIC_CHECK: i64 = 32;

struct JsonErr(i64, &'static str);

fn escape(t: &str, flags: i64, out: &mut String) {
    out.push('"');
    for c in t.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' if flags & UNESCAPED_SLASHES == 0 => out.push_str("\\/"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if !c.is_ascii() && flags & UNESCAPED_UNICODE == 0 => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn encode(vm: &mut Vm<'_>, v: &Value, flags: i64, depth: usize, out: &mut String) -> R<Result<(), JsonErr>> {
    let nl = |out: &mut String, d: usize| {
        if flags & PRETTY != 0 {
            out.push('\n');
            out.push_str(&"    ".repeat(d));
        }
    };
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => {
            if !f.is_finite() {
                return Ok(Err(JsonErr(7, "Inf and NaN cannot be JSON encoded")));
            }
            let t = fmt_float(*f, None);
            out.push_str(&t);
            if flags & PRESERVE_ZERO != 0 && !t.contains(['.', 'E']) {
                out.push_str(".0");
            }
        }
        Value::Str(t) => {
            if flags & NUMERIC_CHECK != 0 {
                if let Some((n, true)) = parse_numeric(t) {
                    return encode(vm, &match n { Num::I(i) => Value::Int(i), Num::F(f) => Value::Float(f) }, flags, depth, out);
                }
            }
            escape(t, flags, out)
        }
        Value::Array(a) => {
            let is_list = flags & FORCE_OBJECT == 0 && a.iter().enumerate().all(|(i, (k, _))| *k == Key::Int(i as i64));
            if a.is_empty() {
                out.push_str(if is_list { "[]" } else { "{}" });
                return Ok(Ok(()));
            }
            out.push(if is_list { '[' } else { '{' });
            for (i, (k, x)) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                nl(out, depth + 1);
                if !is_list {
                    escape(&k.to_value().to_str(), flags, out);
                    out.push_str(if flags & PRETTY != 0 { ": " } else { ":" });
                }
                if let Err(e) = encode(vm, x, flags, depth + 1, out)? {
                    return Ok(Err(e));
                }
            }
            nl(out, depth);
            out.push(if is_list { ']' } else { '}' });
        }
        Value::Object(o) => {
            if o.class.is_a("jsonserializable") {
                let r = vm.call_method_named(v, "jsonSerialize", vec![])?;
                return encode(vm, &r, flags, depth, out);
            }
            if o.class.kind == crate::ast::ClassKind::Enum {
                let val = o.props.borrow().get(&Key::from_str("value")).cloned();
                return match val {
                    Some(x) => encode(vm, &x, flags, depth, out),
                    None => Ok(Err(JsonErr(11, "Non-backed enums have no default serialization in JSON"))),
                };
            }
            let props: Vec<(Key, Value)> = o
                .props
                .borrow()
                .iter()
                .filter(|(k, _)| matches!(k, Key::Int(_)) || o.class.prop_vis(&k.to_value().to_str()).is_none_or(|(vis, _)| vis == 0))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if props.is_empty() {
                out.push_str("{}");
                return Ok(Ok(()));
            }
            out.push('{');
            for (i, (k, x)) in props.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                nl(out, depth + 1);
                escape(&k.to_value().to_str(), flags, out);
                out.push_str(if flags & PRETTY != 0 { ": " } else { ":" });
                if let Err(e) = encode(vm, x, flags, depth + 1, out)? {
                    return Ok(Err(e));
                }
            }
            nl(out, depth);
            out.push('}');
        }
        Value::Ref(r) => return encode(vm, &r.borrow().clone(), flags, depth, out),
    }
    Ok(Ok(()))
}

struct Parser<'a> {
    c: &'a [char],
    i: usize,
    assoc: bool,
    max_depth: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.c.len() && matches!(self.c[self.i], ' ' | '\t' | '\n' | '\r') {
            self.i += 1;
        }
    }

    fn value(&mut self, vm: &mut Vm<'_>, depth: usize) -> Result<Value, JsonErr> {
        if depth > self.max_depth {
            return Err(JsonErr(1, "Maximum stack depth exceeded"));
        }
        self.ws();
        let syntax = || JsonErr(4, "Syntax error");
        match self.c.get(self.i).copied() {
            Some('{') => {
                self.i += 1;
                let mut props = PhpArray::new();
                self.ws();
                if self.c.get(self.i) == Some(&'}') {
                    self.i += 1;
                } else {
                    loop {
                        self.ws();
                        let Value::Str(k) = self.string()? else { return Err(syntax()) };
                        self.ws();
                        if self.c.get(self.i) != Some(&':') {
                            return Err(syntax());
                        }
                        self.i += 1;
                        let v = self.value(vm, depth + 1)?;
                        props.set(Key::from_str(&k), v);
                        self.ws();
                        match self.c.get(self.i) {
                            Some(',') => self.i += 1,
                            Some('}') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(syntax()),
                        }
                    }
                }
                if self.assoc {
                    Ok(Value::Array(Rc::new(props)))
                } else {
                    let class = vm.find_class("stdClass").map_err(|_| syntax())?;
                    Ok(Value::Object(Obj::new(class, props, Extra::None)))
                }
            }
            Some('[') => {
                self.i += 1;
                let mut items = vec![];
                self.ws();
                if self.c.get(self.i) == Some(&']') {
                    self.i += 1;
                } else {
                    loop {
                        items.push(self.value(vm, depth + 1)?);
                        self.ws();
                        match self.c.get(self.i) {
                            Some(',') => self.i += 1,
                            Some(']') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(syntax()),
                        }
                    }
                }
                Ok(Value::Array(Rc::new(PhpArray::from_list(items))))
            }
            Some('"') => self.string(),
            Some('t') if self.lit("true") => Ok(Value::Bool(true)),
            Some('f') if self.lit("false") => Ok(Value::Bool(false)),
            Some('n') if self.lit("null") => Ok(Value::Null),
            Some(c) if c == '-' || c.is_ascii_digit() => {
                let st = self.i;
                self.i += 1;
                while self.i < self.c.len() && matches!(self.c[self.i], '0'..='9' | '.' | 'e' | 'E' | '+' | '-') {
                    self.i += 1;
                }
                let t: String = self.c[st..self.i].iter().collect();
                if t.contains(['.', 'e', 'E']) {
                    t.parse().map(Value::Float).map_err(|_| syntax())
                } else {
                    Ok(t.parse::<i64>().map(Value::Int).unwrap_or_else(|_| Value::Float(t.parse().unwrap_or(0.0))))
                }
            }
            _ => Err(syntax()),
        }
    }

    fn lit(&mut self, word: &str) -> bool {
        let w: Vec<char> = word.chars().collect();
        if self.c.len() >= self.i + w.len() && self.c[self.i..self.i + w.len()] == w[..] {
            self.i += w.len();
            true
        } else {
            false
        }
    }

    fn string(&mut self) -> Result<Value, JsonErr> {
        if self.c.get(self.i) != Some(&'"') {
            return Err(JsonErr(4, "Syntax error"));
        }
        self.i += 1;
        let mut out = String::new();
        while let Some(&ch) = self.c.get(self.i) {
            self.i += 1;
            match ch {
                '"' => return Ok(Value::Str(out.into())),
                '\\' => {
                    let e = self.c.get(self.i).copied().ok_or(JsonErr(4, "Syntax error"))?;
                    self.i += 1;
                    match e {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        '/' | '\\' | '"' => out.push(e),
                        'u' => {
                            let hex = |p: &Parser<'_>, at: usize| -> Option<u32> { u32::from_str_radix(&p.c.get(at..at + 4)?.iter().collect::<String>(), 16).ok() };
                            let mut cp = hex(self, self.i).ok_or(JsonErr(4, "Syntax error"))?;
                            self.i += 4;
                            if (0xD800..0xDC00).contains(&cp) && self.c.get(self.i) == Some(&'\\') && self.c.get(self.i + 1) == Some(&'u') {
                                if let Some(lo) = hex(self, self.i + 2) {
                                    cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                    self.i += 6;
                                }
                            }
                            out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                        }
                        _ => return Err(JsonErr(4, "Syntax error")),
                    }
                }
                c if (c as u32) < 0x20 => return Err(JsonErr(3, "Control character error, possibly incorrectly encoded")),
                c => out.push(c),
            }
        }
        Err(JsonErr(4, "Syntax error"))
    }
}

// ---- serialize ----

fn ser(vm: &mut Vm<'_>, v: &Value, out: &mut String) -> R<()> {
    match v {
        Value::Null => out.push_str("N;"),
        Value::Bool(b) => out.push_str(&format!("b:{};", *b as i32)),
        Value::Int(i) => out.push_str(&format!("i:{i};")),
        Value::Float(f) => out.push_str(&format!("d:{};", fmt_float(*f, None))),
        Value::Str(t) => out.push_str(&format!("s:{}:\"{}\";", t.len(), t)),
        Value::Array(a) => {
            out.push_str(&format!("a:{}:{{", a.len()));
            for (k, x) in a.iter() {
                ser(vm, &k.to_value(), out)?;
                ser(vm, x, out)?;
            }
            out.push('}');
        }
        Value::Object(o) => {
            if o.class.kind == crate::ast::ClassKind::Enum {
                let case = o.props.borrow().get(&Key::from_str("name")).map(|n| n.to_str().to_string()).unwrap_or_default();
                let body = format!("{}:{}", o.class.name, case);
                out.push_str(&format!("E:{}:\"{}\";", body.len(), body));
                return Ok(());
            }
            let props: Vec<(Key, Value)> = if o.class.find_method("__serialize").is_some() {
                match vm.call_method_named(v, "__serialize", vec![])? {
                    Value::Array(a) => a.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                    _ => vec![],
                }
            } else {
                let mut all: Vec<(Key, Value)> = o.props.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                if o.class.find_method("__sleep").is_some() {
                    if let Value::Array(names) = vm.call_method_named(v, "__sleep", vec![])? {
                        let wanted: Vec<String> = names.iter().map(|(_, n)| n.to_str().to_string()).collect();
                        all.retain(|(k, _)| wanted.contains(&k.to_value().to_str().to_string()));
                    }
                }
                all.into_iter()
                    .map(|(k, x)| {
                        let name = k.to_value().to_str();
                        let mangled = match o.class.prop_vis(&name) {
                            Some((1, _)) => format!("\0*\0{name}"),
                            Some((2, c)) => format!("\0{c}\0{name}"),
                            _ => name.to_string(),
                        };
                        (Key::Str(mangled.into()), x)
                    })
                    .collect()
            };
            out.push_str(&format!("O:{}:\"{}\":{}:{{", o.class.name.len(), o.class.name, props.len()));
            for (k, x) in &props {
                ser(vm, &k.to_value(), out)?;
                ser(vm, x, out)?;
            }
            out.push('}');
        }
        Value::Ref(r) => ser(vm, &r.borrow().clone(), out)?,
    }
    Ok(())
}

struct Unser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Unser<'_> {
    fn num_until(&mut self, end: u8) -> Option<String> {
        let st = self.i;
        while *self.b.get(self.i)? != end {
            self.i += 1;
        }
        let t = String::from_utf8_lossy(&self.b[st..self.i]).to_string();
        self.i += 1;
        Some(t)
    }

    fn value(&mut self, vm: &mut Vm<'_>) -> R<Option<Value>> {
        let Some(&tag) = self.b.get(self.i) else { return Ok(None) };
        self.i += 1;
        if tag == b'N' {
            self.i += 1;
            return Ok(Some(Value::Null));
        }
        self.i += 1; // ':'
        Ok(match tag {
            b'b' => self.num_until(b';').map(|t| Value::Bool(t == "1")),
            b'i' => self.num_until(b';').and_then(|t| t.parse().ok()).map(Value::Int),
            b'd' => self.num_until(b';').and_then(|t| match t.as_str() {
                "INF" => Some(f64::INFINITY),
                "-INF" => Some(f64::NEG_INFINITY),
                "NAN" => Some(f64::NAN),
                t => t.parse().ok(),
            }).map(Value::Float),
            b's' | b'E' => {
                let Some(len) = self.num_until(b':').and_then(|t| t.parse::<usize>().ok()) else { return Ok(None) };
                self.i += 1;
                let Some(raw) = self.b.get(self.i..self.i + len) else { return Ok(None) };
                let text = String::from_utf8_lossy(raw).to_string();
                self.i += len + 2;
                if tag == b'E' {
                    let (c, case) = text.split_once(':').unwrap_or((&text, ""));
                    let class = vm.find_class(c)?;
                    let consts = vm.class_const_pub(&class, case)?;
                    return Ok(Some(consts));
                }
                Some(Value::Str(text.into()))
            }
            b'a' => {
                let Some(n) = self.num_until(b':').and_then(|t| t.parse::<usize>().ok()) else { return Ok(None) };
                self.i += 1;
                let mut arr = PhpArray::new();
                for _ in 0..n {
                    let (Some(k), Some(v)) = (self.value(vm)?, self.value(vm)?) else { return Ok(None) };
                    arr.set(Key::from_value(&k)?, v);
                }
                self.i += 1;
                Some(Value::Array(Rc::new(arr)))
            }
            b'O' => {
                let Some(_) = self.num_until(b':') else { return Ok(None) };
                self.i += 1;
                let Some(name) = self.num_until(b'"') else { return Ok(None) };
                self.i += 1;
                let Some(n) = self.num_until(b':').and_then(|t| t.parse::<usize>().ok()) else { return Ok(None) };
                self.i += 1;
                let class = match vm.find_class(&name) {
                    Ok(c) => c,
                    Err(_) => vm.find_class("__PHP_Incomplete_Class")?,
                };
                let mut props = PhpArray::new();
                for _ in 0..n {
                    let (Some(k), Some(v)) = (self.value(vm)?, self.value(vm)?) else { return Ok(None) };
                    let key = k.to_str();
                    let plain = key.rsplit('\0').next().unwrap_or(&key).to_string();
                    props.set(Key::from_str(&plain), v);
                }
                self.i += 1;
                let obj = Value::Object(Obj::new(class.clone(), PhpArray::new(), Extra::None));
                if let Value::Object(o) = &obj {
                    if class.find_method("__unserialize").is_some() {
                        vm.call_method_named(&obj, "__unserialize", vec![Value::Array(Rc::new(props))])?;
                    } else {
                        *o.props.borrow_mut() = props;
                        if class.find_method("__wakeup").is_some() {
                            vm.call_method_named(&obj, "__wakeup", vec![])?;
                        }
                    }
                }
                Some(obj)
            }
            _ => None,
        })
    }
}

pub fn register(m: &mut HashMap<&'static str, VmBuiltin>) {
    let list: &[(&str, VmBuiltin)] = &[
        ("json_encode", |vm, a| {
            let flags = arg(&a, 1).to_int();
            let mut out = String::new();
            match encode(vm, &arg(&a, 0), flags, 0, &mut out)? {
                Ok(()) => {
                    vm.json_error = (0, "No error");
                    Ok(Value::str(&out))
                }
                Err(JsonErr(code, msg)) => {
                    if flags & THROW != 0 {
                        let ex = vm.make_throwable("JsonException", msg)?;
                        if let Value::Object(o) = &ex {
                            o.props.borrow_mut().set(Key::from_str("code"), Value::Int(code));
                        }
                        return Err(Flow::Throw(ex));
                    }
                    vm.json_error = (code, msg);
                    Ok(if flags & PARTIAL != 0 { Value::str(&out) } else { Value::Bool(false) })
                }
            }
        }),
        ("json_decode", |vm, a| {
            let text: Vec<char> = arg(&a, 0).to_str().chars().collect();
            let flags = arg(&a, 3).to_int();
            let assoc = arg(&a, 1).to_bool() || flags & 1 != 0;
            let depth = a.get(2).map(Value::to_int).unwrap_or(512).max(1) as usize;
            let mut p = Parser { c: &text, i: 0, assoc, max_depth: depth };
            let r = p.value(vm, 1).and_then(|v| {
                p.ws();
                if p.i == p.c.len() { Ok(v) } else { Err(JsonErr(4, "Syntax error")) }
            });
            match r {
                Ok(v) => {
                    vm.json_error = (0, "No error");
                    Ok(v)
                }
                Err(JsonErr(code, msg)) => {
                    if flags & THROW != 0 {
                        let ex = vm.make_throwable("JsonException", msg)?;
                        if let Value::Object(o) = &ex {
                            o.props.borrow_mut().set(Key::from_str("code"), Value::Int(code));
                        }
                        return Err(Flow::Throw(ex));
                    }
                    vm.json_error = (code, msg);
                    Ok(Value::Null)
                }
            }
        }),
        ("json_last_error", |vm, _| Ok(Value::Int(vm.json_error.0))),
        ("json_last_error_msg", |vm, _| Ok(Value::str(vm.json_error.1))),
        ("serialize", |vm, a| {
            let mut out = String::new();
            ser(vm, &arg(&a, 0), &mut out)?;
            Ok(Value::str(&out))
        }),
        ("unserialize", |vm, a| {
            let text = arg(&a, 0).to_str();
            let mut u = Unser { b: text.as_bytes(), i: 0 };
            Ok(u.value(vm)?.unwrap_or(Value::Bool(false)))
        }),
    ];
    for (k, f) in list {
        m.insert(k, *f);
    }
}
