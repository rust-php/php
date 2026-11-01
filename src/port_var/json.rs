//! Port of ext/json: json_encoder.c, json_scanner.re, json_parser.y, json.c.
use super::dump::is_enum;
use crate::builtins::arg;
use crate::object::{Extra, Obj};
use crate::value::*;
use crate::vm::{Flow, Vm};
use std::cell::Cell;
use std::rc::Rc;

type R<T> = Result<T, Flow>;

pub const HEX_TAG: i64 = 1;
pub const HEX_AMP: i64 = 2;
pub const HEX_APOS: i64 = 4;
pub const HEX_QUOT: i64 = 8;
pub const FORCE_OBJECT: i64 = 16;
pub const NUMERIC_CHECK: i64 = 32;
pub const UNESCAPED_SLASHES: i64 = 64;
pub const PRETTY_PRINT: i64 = 128;
pub const UNESCAPED_UNICODE: i64 = 256;
pub const PARTIAL_OUTPUT_ON_ERROR: i64 = 512;
pub const PRESERVE_ZERO_FRACTION: i64 = 1024;
pub const UNESCAPED_LINE_TERMINATORS: i64 = 2048;
pub const OBJECT_AS_ARRAY: i64 = 1;
pub const BIGINT_AS_STRING: i64 = 2;
pub const INVALID_UTF8_IGNORE: i64 = 1048576;
pub const INVALID_UTF8_SUBSTITUTE: i64 = 2097152;
pub const THROW_ON_ERROR: i64 = 4194304;

pub const E_NONE: i64 = 0;
pub const E_DEPTH: i64 = 1;
pub const E_STATE_MISMATCH: i64 = 2;
pub const E_CTRL_CHAR: i64 = 3;
pub const E_SYNTAX: i64 = 4;
pub const E_UTF8: i64 = 5;
pub const E_RECURSION: i64 = 6;
pub const E_INF_OR_NAN: i64 = 7;
pub const E_UNSUPPORTED_TYPE: i64 = 8;
pub const E_INVALID_PROPERTY_NAME: i64 = 9;
pub const E_UTF16: i64 = 10;
pub const E_NON_BACKED_ENUM: i64 = 11;

thread_local! {
    /// JSON_G(error_details): (code, line, column)
    static ERROR: Cell<(i64, u64, u64)> = const { Cell::new((0, 0, 0)) };
}

pub fn constant(name: &str) -> Option<Value> {
    let v = match name {
        "JSON_HEX_TAG" => HEX_TAG,
        "JSON_HEX_AMP" => HEX_AMP,
        "JSON_HEX_APOS" => HEX_APOS,
        "JSON_HEX_QUOT" => HEX_QUOT,
        "JSON_FORCE_OBJECT" => FORCE_OBJECT,
        "JSON_NUMERIC_CHECK" => NUMERIC_CHECK,
        "JSON_UNESCAPED_SLASHES" => UNESCAPED_SLASHES,
        "JSON_PRETTY_PRINT" => PRETTY_PRINT,
        "JSON_UNESCAPED_UNICODE" => UNESCAPED_UNICODE,
        "JSON_PARTIAL_OUTPUT_ON_ERROR" => PARTIAL_OUTPUT_ON_ERROR,
        "JSON_PRESERVE_ZERO_FRACTION" => PRESERVE_ZERO_FRACTION,
        "JSON_UNESCAPED_LINE_TERMINATORS" => UNESCAPED_LINE_TERMINATORS,
        "JSON_OBJECT_AS_ARRAY" => OBJECT_AS_ARRAY,
        "JSON_BIGINT_AS_STRING" => BIGINT_AS_STRING,
        "JSON_INVALID_UTF8_IGNORE" => INVALID_UTF8_IGNORE,
        "JSON_INVALID_UTF8_SUBSTITUTE" => INVALID_UTF8_SUBSTITUTE,
        "JSON_THROW_ON_ERROR" => THROW_ON_ERROR,
        "JSON_ERROR_NONE" => E_NONE,
        "JSON_ERROR_DEPTH" => E_DEPTH,
        "JSON_ERROR_STATE_MISMATCH" => E_STATE_MISMATCH,
        "JSON_ERROR_CTRL_CHAR" => E_CTRL_CHAR,
        "JSON_ERROR_SYNTAX" => E_SYNTAX,
        "JSON_ERROR_UTF8" => E_UTF8,
        "JSON_ERROR_RECURSION" => E_RECURSION,
        "JSON_ERROR_INF_OR_NAN" => E_INF_OR_NAN,
        "JSON_ERROR_UNSUPPORTED_TYPE" => E_UNSUPPORTED_TYPE,
        "JSON_ERROR_INVALID_PROPERTY_NAME" => E_INVALID_PROPERTY_NAME,
        "JSON_ERROR_UTF16" => E_UTF16,
        "JSON_ERROR_NON_BACKED_ENUM" => E_NON_BACKED_ENUM,
        _ => return None,
    };
    Some(Value::Int(v))
}

fn error_msg(code: i64) -> &'static str {
    match code {
        E_NONE => "No error",
        E_DEPTH => "Maximum stack depth exceeded",
        E_STATE_MISMATCH => "State mismatch (invalid or malformed JSON)",
        E_CTRL_CHAR => "Control character error, possibly incorrectly encoded",
        E_SYNTAX => "Syntax error",
        E_UTF8 => "Malformed UTF-8 characters, possibly incorrectly encoded",
        E_RECURSION => "Recursion detected",
        E_INF_OR_NAN => "Inf and NaN cannot be JSON encoded",
        E_UNSUPPORTED_TYPE => "Type is not supported",
        E_INVALID_PROPERTY_NAME => "The decoded property name is invalid",
        E_UTF16 => "Single unpaired UTF-16 surrogate in unicode escape",
        E_NON_BACKED_ENUM => "Non-backed enums have no default serialization",
        _ => "Unknown error",
    }
}

fn error_msg_with_location(d: (i64, u64, u64)) -> String {
    let base = error_msg(d.0);
    if d.1 > 0 && d.2 > 0 {
        format!("{base} near location {}:{}", d.1, d.2)
    } else {
        base.to_string()
    }
}

// ---------------------------------------------------------------- encoder

struct Encoder {
    depth: i64,
    max_depth: i64,
    error: i64,
    /// JSON recursion guard on objects being serialized via jsonSerialize()
    guard: Vec<usize>,
    /// GC_PROTECT_RECURSION on arrays / objects
    rec: Vec<usize>,
}

fn pretty_char(buf: &mut String, o: i64, c: char) {
    if o & PRETTY_PRINT != 0 {
        buf.push(c);
    }
}

fn pretty_indent(buf: &mut String, o: i64, e: &Encoder) {
    if o & PRETTY_PRINT != 0 {
        for _ in 0..e.depth {
            buf.push_str("    ");
        }
    }
}

fn encode_double(buf: &mut String, d: f64, o: i64) {
    let mut num = fmt_float(d, None).replace('E', "e");
    if o & PRESERVE_ZERO_FRACTION != 0 && !num.contains('.') {
        num.push_str(".0");
    }
    buf.push_str(&num);
}

/// php_json_escape_string. Returns false on FAILURE.
fn escape_string(buf: &mut String, s: &str, o: i64, e: &mut Encoder) -> bool {
    if s.is_empty() {
        buf.push_str("\"\"");
        return true;
    }
    if o & NUMERIC_CHECK != 0 {
        if let Some((n, true)) = parse_numeric(s) {
            match n {
                Num::I(i) => {
                    buf.push_str(&i.to_string());
                    return true;
                }
                Num::F(d) if d.is_finite() => {
                    encode_double(buf, d, o);
                    return true;
                }
                _ => {}
            }
        }
    }
    buf.push('"');
    for c in s.chars() {
        let us = c as u32;
        if us >= 0x80 {
            if o & UNESCAPED_UNICODE != 0 && (o & UNESCAPED_LINE_TERMINATORS != 0 || !(0x2028..=0x2029).contains(&us)) {
                buf.push(c);
            } else {
                let mut b = [0u16; 2];
                for u in c.encode_utf16(&mut b) {
                    buf.push_str(&format!("\\u{:04x}", u));
                }
            }
            continue;
        }
        match c {
            '"' => buf.push_str(if o & HEX_QUOT != 0 { "\\u0022" } else { "\\\"" }),
            '\\' => buf.push_str("\\\\"),
            '/' => buf.push_str(if o & UNESCAPED_SLASHES != 0 { "/" } else { "\\/" }),
            '\u{8}' => buf.push_str("\\b"),
            '\u{c}' => buf.push_str("\\f"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            '<' => buf.push_str(if o & HEX_TAG != 0 { "\\u003C" } else { "<" }),
            '>' => buf.push_str(if o & HEX_TAG != 0 { "\\u003E" } else { ">" }),
            '&' => buf.push_str(if o & HEX_AMP != 0 { "\\u0026" } else { "&" }),
            '\'' => buf.push_str(if o & HEX_APOS != 0 { "\\u0027" } else { "'" }),
            _ if us < 0x20 => buf.push_str(&format!("\\u00{:02x}", us)),
            c => buf.push(c),
        }
    }
    buf.push('"');
    let _ = e;
    true
}

fn is_list(a: &PhpArray) -> bool {
    a.iter().enumerate().all(|(i, (k, _))| *k == Key::Int(i as i64))
}

/// php_json_encode_array. `val` is an Array or Object value. Ok(false) = FAILURE.
fn encode_array(vm: &mut Vm<'_>, buf: &mut String, val: &Value, o: i64, e: &mut Encoder) -> R<bool> {
    let mut as_object = o & FORCE_OBJECT != 0;
    let items: Vec<(Key, Value)>;
    let rc_ptr: usize;
    match val {
        Value::Array(a) => {
            rc_ptr = Rc::as_ptr(a) as usize;
            as_object = as_object || !is_list(a);
            items = a.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        }
        Value::Object(ob) => {
            rc_ptr = Rc::as_ptr(ob) as usize;
            as_object = true;
            items = ob.props.borrow().iter().filter(|(k, _)| match k {
                Key::Str(n) => !n.starts_with('\0') && !matches!(ob.class.prop_vis(n), Some((v, _)) if v != 0),
                _ => true,
            }).map(|(k, v)| (k.clone(), v.clone())).collect();
        }
        _ => return Ok(false),
    }
    if e.rec.contains(&rc_ptr) {
        e.error = E_RECURSION;
        buf.push_str("null");
        return Ok(false);
    }
    e.rec.push(rc_ptr);
    buf.push(if as_object { '{' } else { '[' });
    e.depth += 1;
    let mut empty = true;
    let obj = matches!(val, Value::Object(_));
    for (k, data) in &items {
        let _ = obj;
        pretty_char(buf, o, '\n');
        pretty_indent(buf, o, e);
        if as_object {
            match k {
                Key::Str(name) => {
                    let start = buf.len();
                    if !escape_string(buf, name, o & !NUMERIC_CHECK, e) {
                        if o & PARTIAL_OUTPUT_ON_ERROR == 0 {
                            e.rec.pop();
                            return Ok(false);
                        }
                        buf.truncate(start);
                        buf.push_str("\"\"");
                    }
                }
                Key::Int(i) => {
                    buf.push('"');
                    buf.push_str(&i.to_string());
                    buf.push('"');
                }
            }
            buf.push(':');
            pretty_char(buf, o, ' ');
        }
        if !encode_zval(vm, buf, data, o, e)? && o & PARTIAL_OUTPUT_ON_ERROR == 0 {
            e.rec.pop();
            return Ok(false);
        }
        buf.push(',');
        empty = false;
    }
    if !empty {
        buf.pop();
    }
    e.rec.pop();
    if e.depth > e.max_depth {
        e.error = E_DEPTH;
        if o & PARTIAL_OUTPUT_ON_ERROR == 0 {
            return Ok(false);
        }
    }
    e.depth -= 1;
    if !empty {
        pretty_char(buf, o, '\n');
        pretty_indent(buf, o, e);
    }
    buf.push(if as_object { '}' } else { ']' });
    Ok(true)
}

fn encode_serializable_object(vm: &mut Vm<'_>, buf: &mut String, val: &Value, ob: &Rc<Obj>, o: i64, e: &mut Encoder) -> R<bool> {
    let p = Rc::as_ptr(ob) as usize;
    if e.guard.contains(&p) {
        e.error = E_RECURSION;
        if o & PARTIAL_OUTPUT_ON_ERROR != 0 {
            buf.push_str("null");
        }
        return Ok(false);
    }
    e.guard.push(p);
    let ret = match vm.call_method_named(val, "jsonSerialize", vec![]) {
        Ok(r) => r,
        Err(f) => {
            e.guard.pop();
            return Err(f);
        }
    };
    let r = match &ret {
        Value::Object(r2) if Rc::ptr_eq(r2, ob) => {
            e.guard.pop();
            encode_array(vm, buf, &ret, o, e)
        }
        _ => {
            let r = encode_zval(vm, buf, &ret, o, e);
            e.guard.pop();
            r
        }
    };
    r
}

fn encode_zval(vm: &mut Vm<'_>, buf: &mut String, val: &Value, o: i64, e: &mut Encoder) -> R<bool> {
    match val {
        Value::Null => buf.push_str("null"),
        Value::Bool(true) => buf.push_str("true"),
        Value::Bool(false) => buf.push_str("false"),
        Value::Int(i) => buf.push_str(&i.to_string()),
        Value::Float(d) => {
            if d.is_finite() {
                encode_double(buf, *d, o);
            } else {
                e.error = E_INF_OR_NAN;
                buf.push('0');
            }
        }
        Value::Str(s) => return Ok(escape_string(buf, s, o, e)),
        Value::Object(ob) => {
            if ob.class.is_a("jsonserializable") {
                return encode_serializable_object(vm, buf, val, ob, o, e);
            }
            if is_enum(ob) {
                if !ob.class.backed {
                    e.error = E_NON_BACKED_ENUM;
                    buf.push('0');
                    return Ok(false);
                }
                let v = ob.props.borrow().get(&Key::from_str("value")).cloned().unwrap_or(Value::Null);
                return encode_zval(vm, buf, &v, o, e);
            }
            return encode_array(vm, buf, val, o, e);
        }
        Value::Array(_) => return encode_array(vm, buf, val, o, e),
        Value::Ref(r) => {
            let p = Rc::as_ptr(r) as usize;
            if e.rec.contains(&p) {
                e.error = E_RECURSION;
                buf.push_str("null");
                return Ok(false);
            }
            e.rec.push(p);
            let inner = r.borrow().clone();
            let res = encode_zval(vm, buf, &inner, o, e);
            e.rec.pop();
            return res;
        }
        Value::Resource(_) => {
            e.error = E_UNSUPPORTED_TYPE;
            if o & PARTIAL_OUTPUT_ON_ERROR != 0 {
                buf.push_str("null");
            }
            return Ok(false);
        }
    }
    Ok(true)
}

fn throw_json(vm: &mut Vm<'_>, msg: &str, code: i64) -> Flow {
    match vm.make_throwable("JsonException", msg) {
        Ok(ex) => {
            if let Value::Object(ob) = &ex {
                ob.props.borrow_mut().set(Key::from_str("code"), Value::Int(code));
            }
            Flow::Throw(ex)
        }
        Err(f) => f,
    }
}

// ---------------------------------------------------------------- scanner / parser

#[derive(Debug)]
enum Tok {
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    Comma,
    Nul,
    True,
    False,
    Int(i64),
    Double(f64),
    Str(String),
    Eoi,
    Error,
}

struct Scanner<'a> {
    b: &'a [u8],
    cur: usize,
    token: usize,
    line: u64,
    line_start: usize,
    options: i64,
    errcode: i64,
}

impl<'a> Scanner<'a> {
    fn hex(&self, i: usize) -> Option<u32> {
        self.b.get(i).and_then(|c| (*c as char).to_digit(16))
    }

    fn scan(&mut self) -> Tok {
        loop {
            self.token = self.cur;
            let Some(&c) = self.b.get(self.cur) else {
                return Tok::Eoi;
            };
            match c {
                b'{' => { self.cur += 1; return Tok::LBrace; }
                b'}' => { self.cur += 1; return Tok::RBrace; }
                b'[' => { self.cur += 1; return Tok::LBracket; }
                b']' => { self.cur += 1; return Tok::RBracket; }
                b':' => { self.cur += 1; return Tok::Colon; }
                b',' => { self.cur += 1; return Tok::Comma; }
                b' ' | b'\t' | b'\r' => {
                    if c == b'\r' && self.b.get(self.cur + 1) == Some(&b'\n') {
                        self.cur += 2;
                        self.line += 1;
                        self.line_start = self.cur;
                        continue;
                    }
                    self.cur += 1;
                    continue;
                }
                b'\n' => {
                    self.cur += 1;
                    self.line += 1;
                    self.line_start = self.cur;
                    continue;
                }
                b'"' => return self.scan_string(),
                b'-' | b'0'..=b'9' => return self.scan_number(),
                b'n' if self.b[self.cur..].starts_with(b"null") => { self.cur += 4; return Tok::Nul; }
                b't' if self.b[self.cur..].starts_with(b"true") => { self.cur += 4; return Tok::True; }
                b'f' if self.b[self.cur..].starts_with(b"false") => { self.cur += 5; return Tok::False; }
                c if c < 0x20 => {
                    self.cur += 1;
                    self.errcode = E_CTRL_CHAR;
                    return Tok::Error;
                }
                _ => {
                    self.cur += 1;
                    self.errcode = E_SYNTAX;
                    return Tok::Error;
                }
            }
        }
    }

    fn scan_number(&mut self) -> Tok {
        let b = self.b;
        let start = self.cur;
        let mut i = start;
        let neg = b[i] == b'-';
        if neg {
            i += 1;
        }
        let ds = i;
        match b.get(i) {
            Some(b'0') => i += 1,
            Some(b'1'..=b'9') => {
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            _ => {
                // lone '-' : ANY rule -> syntax error
                self.cur = start + 1;
                self.errcode = E_SYNTAX;
                return Tok::Error;
            }
        }
        let int_end = i;
        let mut is_float = false;
        if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            is_float = true;
        }
        if matches!(b.get(i), Some(b'e' | b'E')) {
            let mut k = i + 1;
            if matches!(b.get(k), Some(b'+' | b'-')) {
                k += 1;
            }
            if b.get(k).is_some_and(u8::is_ascii_digit) {
                while k < b.len() && b[k].is_ascii_digit() {
                    k += 1;
                }
                i = k;
                is_float = true;
            }
        }
        if is_float {
            self.cur = i;
            let t = std::str::from_utf8(&b[start..i]).unwrap_or("0");
            return Tok::Double(t.parse().unwrap_or(0.0));
        }
        self.cur = int_end;
        let t = std::str::from_utf8(&b[start..int_end]).unwrap_or("0");
        let digits = int_end - ds;
        let mut bigint = false;
        if digits >= 19 {
            if digits == 19 {
                let cmp = b[ds..int_end].cmp(b"9223372036854775808");
                if !(cmp == std::cmp::Ordering::Less || (cmp == std::cmp::Ordering::Equal && neg)) {
                    bigint = true;
                }
            } else {
                bigint = true;
            }
        }
        if !bigint {
            Tok::Int(t.parse().unwrap_or(0))
        } else if self.options & BIGINT_AS_STRING != 0 {
            Tok::Str(t.to_string())
        } else {
            Tok::Double(t.parse().unwrap_or(0.0))
        }
    }

    fn scan_string(&mut self) -> Tok {
        let b = self.b;
        let mut i = self.cur + 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = b.get(i) else {
                self.cur = i;
                self.errcode = E_SYNTAX;
                return Tok::Error;
            };
            match c {
                b'"' => {
                    self.cur = i + 1;
                    return Tok::Str(String::from_utf8_lossy(&out).into_owned());
                }
                c if c < 0x20 => {
                    self.cur = i + 1;
                    self.errcode = E_CTRL_CHAR;
                    return Tok::Error;
                }
                b'\\' => {
                    let Some(&n) = b.get(i + 1) else {
                        self.cur = i + 1;
                        self.errcode = E_SYNTAX;
                        return Tok::Error;
                    };
                    match n {
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'\\' | b'/' | b'"' => out.push(n),
                        b'u' => {
                            let h: Option<Vec<u32>> = (0..4).map(|k| self.hex(i + 2 + k)).collect();
                            let Some(h) = h else {
                                self.cur = i + 1;
                                self.errcode = E_SYNTAX;
                                return Tok::Error;
                            };
                            let u = (h[0] << 12) | (h[1] << 8) | (h[2] << 4) | h[3];
                            if (0xD800..0xDC00).contains(&u) {
                                // UTF16_4: high surrogate followed by \u low surrogate
                                let lo = if b.get(i + 6) == Some(&b'\\') && b.get(i + 7) == Some(&b'u') {
                                    let l: Option<Vec<u32>> = (0..4).map(|k| self.hex(i + 8 + k)).collect();
                                    l.map(|l| (l[0] << 12) | (l[1] << 8) | (l[2] << 4) | l[3]).filter(|v| (0xDC00..0xE000).contains(v))
                                } else {
                                    None
                                };
                                match lo {
                                    Some(lo) => {
                                        let cp = (((u & 0x3FF) << 10) | (lo & 0x3FF)) + 0x10000;
                                        let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                                        let mut t = [0u8; 4];
                                        out.extend_from_slice(ch.encode_utf8(&mut t).as_bytes());
                                        i += 12;
                                        continue;
                                    }
                                    None => {
                                        self.cur = i + 6;
                                        self.errcode = E_UTF16;
                                        return Tok::Error;
                                    }
                                }
                            } else if (0xDC00..0xE000).contains(&u) {
                                self.cur = i + 6;
                                self.errcode = E_UTF16;
                                return Tok::Error;
                            }
                            let ch = char::from_u32(u).unwrap_or('\u{fffd}');
                            let mut t = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut t).as_bytes());
                            i += 6;
                            continue;
                        }
                        _ => {
                            self.cur = i + 1;
                            self.errcode = E_SYNTAX;
                            return Tok::Error;
                        }
                    }
                    i += 2;
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
    }
}

struct Parser<'a> {
    s: Scanner<'a>,
    tok: Tok,
    depth: i64,
    max_depth: i64,
    assoc: bool,
    validate: bool,
    stdclass: Option<Rc<crate::object::Class>>,
}

struct PErr;

impl<'a> Parser<'a> {
    fn next(&mut self) {
        self.tok = self.s.scan();
    }

    fn syntax(&mut self) -> PErr {
        if self.s.errcode == 0 {
            self.s.errcode = E_SYNTAX;
        }
        PErr
    }

    fn depth_inc(&mut self) -> Result<(), PErr> {
        if self.max_depth != 0 && self.depth >= self.max_depth {
            self.s.errcode = E_DEPTH;
            return Err(PErr);
        }
        self.depth += 1;
        Ok(())
    }

    fn new_object(&mut self, props: PhpArray) -> Value {
        if self.assoc {
            Value::Array(Rc::new(props))
        } else {
            let c = self.stdclass.clone().expect("stdClass");
            Value::Object(Obj::new(c, props, Extra::None))
        }
    }

    fn value(&mut self) -> Result<Value, PErr> {
        match std::mem::replace(&mut self.tok, Tok::Error) {
            Tok::LBrace => {
                self.depth_inc()?;
                self.next();
                let mut props = PhpArray::new();
                let mut created: Option<Rc<Obj>> = None;
                if matches!(self.tok, Tok::RBrace) {
                    self.next();
                    self.depth -= 1;
                    return Ok(self.new_object(props));
                }
                let mut first = true;
                loop {
                    let key = match std::mem::replace(&mut self.tok, Tok::Error) {
                        Tok::Str(k) => k,
                        Tok::RBracket if first => {
                            self.s.errcode = E_STATE_MISMATCH;
                            return Err(PErr);
                        }
                        _ => return Err(self.syntax()),
                    };
                    self.next();
                    if !matches!(self.tok, Tok::Colon) {
                        return Err(self.syntax());
                    }
                    self.next();
                    let v = self.value()?;
                    if !self.assoc && key.starts_with('\0') {
                        self.s.errcode = E_INVALID_PROPERTY_NAME;
                        return Err(PErr);
                    }
                    if self.validate {
                        // nothing stored
                    } else if self.assoc {
                        props.set(Key::from_str(&key), v);
                    } else {
                        props.set(Key::from_str(&key), v);
                    }
                    if first {
                        first = false;
                        if !self.assoc {
                            let c = self.stdclass.clone().expect("stdClass");
                            created = Some(Obj::new(c, PhpArray::new(), Extra::None));
                        }
                    }
                    match self.tok {
                        Tok::Comma => {
                            self.next();
                        }
                        Tok::RBrace => {
                            self.next();
                            break;
                        }
                        Tok::RBracket => {
                            self.s.errcode = E_STATE_MISMATCH;
                            return Err(PErr);
                        }
                        _ => return Err(self.syntax()),
                    }
                }
                self.depth -= 1;
                match created {
                    Some(ob) => {
                        *ob.props.borrow_mut() = props;
                        Ok(Value::Object(ob))
                    }
                    None => Ok(self.new_object(props)),
                }
            }
            Tok::LBracket => {
                self.depth_inc()?;
                self.next();
                let mut items = PhpArray::new();
                if matches!(self.tok, Tok::RBracket) {
                    self.next();
                    self.depth -= 1;
                    return Ok(Value::Array(Rc::new(items)));
                }
                if matches!(self.tok, Tok::RBrace) {
                    self.s.errcode = E_STATE_MISMATCH;
                    return Err(PErr);
                }
                loop {
                    let v = self.value()?;
                    if !self.validate {
                        items.push(v);
                    }
                    match self.tok {
                        Tok::Comma => {
                            self.next();
                        }
                        Tok::RBracket => {
                            self.next();
                            break;
                        }
                        Tok::RBrace => {
                            self.s.errcode = E_STATE_MISMATCH;
                            return Err(PErr);
                        }
                        _ => return Err(self.syntax()),
                    }
                }
                self.depth -= 1;
                Ok(Value::Array(Rc::new(items)))
            }
            Tok::Str(s) => {
                self.next();
                Ok(Value::Str(s.as_str().into()))
            }
            Tok::Int(i) => {
                self.next();
                Ok(Value::Int(i))
            }
            Tok::Double(d) => {
                self.next();
                Ok(Value::Float(d))
            }
            Tok::Nul => {
                self.next();
                Ok(Value::Null)
            }
            Tok::True => {
                self.next();
                Ok(Value::Bool(true))
            }
            Tok::False => {
                self.next();
                Ok(Value::Bool(false))
            }
            _ => Err(self.syntax()),
        }
    }

    fn parse(&mut self) -> Result<Value, PErr> {
        self.next();
        let v = self.value()?;
        if !matches!(self.tok, Tok::Eoi) {
            return Err(self.syntax());
        }
        Ok(v)
    }

    fn error_details(&self) -> (i64, u64, u64) {
        let mut col = 1u64;
        for &c in &self.s.b[self.s.line_start..self.s.token.min(self.s.b.len())] {
            if c & 0xC0 != 0x80 {
                col += 1;
            }
        }
        (self.s.errcode, self.s.line, col)
    }
}

fn decode(vm: &mut Vm<'_>, s: &str, options: i64, depth: i64, validate: bool) -> R<Result<Value, (i64, u64, u64)>> {
    let stdclass = if validate || options & OBJECT_AS_ARRAY != 0 { None } else { Some(vm.find_class("stdClass")?) };
    let mut p = Parser {
        s: Scanner { b: s.as_bytes(), cur: 0, token: 0, line: 1, line_start: 0, options, errcode: 0 },
        tok: Tok::Eoi,
        depth: 1,
        max_depth: depth,
        assoc: options & OBJECT_AS_ARRAY != 0,
        validate,
        stdclass,
    };
    Ok(match p.parse() {
        Ok(v) => Ok(v),
        Err(PErr) => Err(p.error_details()),
    })
}

// ---------------------------------------------------------------- functions

fn count_check(vm: &mut Vm<'_>, name: &str, a: &[Value], min: usize, max: usize) -> R<()> {
    let n = a.len();
    if n < min || n > max {
        let (word, k) = if n < min { ("at least", min) } else { ("at most", max) };
        let word = if min == max { "exactly" } else { word };
        let k = if min == max { min } else { k };
        return vm.throw_error("ArgumentCountError", &format!("{name}() expects {word} {k} argument{}, {n} given", if k == 1 { "" } else { "s" }));
    }
    Ok(())
}

pub fn json_encode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    count_check(vm, "json_encode", &a, 1, 3)?;
    let options = a.get(1).map(Value::to_int).unwrap_or(0);
    let depth = a.get(2).map(Value::to_int).unwrap_or(512);
    if depth > i32::MAX as i64 || depth < i32::MIN as i64 {
        return vm.throw_error("ValueError", &format!("json_encode(): Argument #3 ($depth) must be between {} and {}", i32::MIN, i32::MAX));
    }
    let mut e = Encoder { depth: 0, max_depth: depth, error: E_NONE, guard: vec![], rec: vec![] };
    let mut buf = String::new();
    encode_zval(vm, &mut buf, &arg(&a, 0), options, &mut e)?;
    if options & THROW_ON_ERROR == 0 || options & PARTIAL_OUTPUT_ON_ERROR != 0 {
        ERROR.with(|c| c.set((e.error, 0, 0)));
        if e.error != E_NONE && options & PARTIAL_OUTPUT_ON_ERROR == 0 {
            return Ok(Value::Bool(false));
        }
    } else if e.error != E_NONE {
        return Err(throw_json(vm, error_msg(e.error), e.error));
    }
    Ok(Value::str(&buf))
}

pub fn json_decode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    count_check(vm, "json_decode", &a, 1, 4)?;
    let text = a[0].to_str();
    let assoc_null = matches!(a.get(1), None | Some(Value::Null));
    let assoc = a.get(1).map(Value::to_bool).unwrap_or(false);
    let depth = a.get(2).map(Value::to_int).unwrap_or(512);
    let mut options = a.get(3).map(Value::to_int).unwrap_or(0);
    if options & THROW_ON_ERROR == 0 {
        ERROR.with(|c| c.set((0, 0, 0)));
    }
    if text.is_empty() {
        if options & THROW_ON_ERROR == 0 {
            ERROR.with(|c| c.set((E_SYNTAX, 0, 0)));
        } else {
            return Err(throw_json(vm, error_msg(E_SYNTAX), E_SYNTAX));
        }
        return Ok(Value::Null);
    }
    if depth <= 0 {
        return vm.throw_error("ValueError", "json_decode(): Argument #3 ($depth) must be greater than 0");
    }
    if depth > i32::MAX as i64 {
        return vm.throw_error("ValueError", &format!("json_decode(): Argument #3 ($depth) must be less than {}", i32::MAX));
    }
    if !assoc_null {
        if assoc {
            options |= OBJECT_AS_ARRAY;
        } else {
            options &= !OBJECT_AS_ARRAY;
        }
    }
    match decode(vm, &text, options, depth, false)? {
        Ok(v) => Ok(v),
        Err(d) => {
            if options & THROW_ON_ERROR == 0 {
                ERROR.with(|c| c.set(d));
            } else {
                return Err(throw_json(vm, &error_msg_with_location(d), d.0));
            }
            Ok(Value::Null)
        }
    }
}

pub fn json_validate(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    count_check(vm, "json_validate", &a, 1, 3)?;
    let text = a[0].to_str();
    let depth = a.get(1).map(Value::to_int).unwrap_or(512);
    let options = a.get(2).map(Value::to_int).unwrap_or(0);
    if options != 0 && options != INVALID_UTF8_IGNORE {
        return vm.throw_error("ValueError", "json_validate(): Argument #3 ($flags) must be a valid flag (allowed flags: JSON_INVALID_UTF8_IGNORE)");
    }
    if text.is_empty() {
        ERROR.with(|c| c.set((E_SYNTAX, 0, 0)));
        return Ok(Value::Bool(false));
    }
    ERROR.with(|c| c.set((0, 0, 0)));
    if depth <= 0 {
        return vm.throw_error("ValueError", "json_validate(): Argument #2 ($depth) must be greater than 0");
    }
    if depth > i32::MAX as i64 {
        return vm.throw_error("ValueError", &format!("json_validate(): Argument #2 ($depth) must be less than {}", i32::MAX));
    }
    Ok(match decode(vm, &text, options, depth, true)? {
        Ok(_) => Value::Bool(true),
        Err(d) => {
            ERROR.with(|c| c.set(d));
            Value::Bool(false)
        }
    })
}

pub fn json_last_error(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    count_check(vm, "json_last_error", &a, 0, 0)?;
    Ok(Value::Int(ERROR.with(|c| c.get()).0))
}

pub fn json_last_error_msg(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    count_check(vm, "json_last_error_msg", &a, 0, 0)?;
    Ok(Value::str(&error_msg_with_location(ERROR.with(|c| c.get()))))
}
