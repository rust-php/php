//! Core value model: zval-equivalent `Value`, ordered-hash `PhpArray`, conversions and comparisons.
use crate::object::Obj;
use std::{cell::RefCell, cmp::Ordering, collections::HashMap, rc::Rc};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Int(i64),
    Str(Rc<str>),
}

impl Key {
    pub fn from_value(v: &Value) -> Result<Key, String> {
        Ok(match v {
            Value::Ref(r) => return Key::from_value(&r.borrow()),
            Value::Resource(r) => Key::Int(r.id),
            Value::Int(i) => Key::Int(*i),
            Value::Str(s) => Key::from_str(s),
            Value::Bool(b) => Key::Int(*b as i64),
            Value::Float(f) => Key::Int(*f as i64),
            Value::Null => Key::Str("".into()),
            Value::Array(_) | Value::Object(_) => return Err("Illegal offset type".into()),
        })
    }

    pub fn from_str(s: &str) -> Key {
        match canonical_int(s) {
            Some(i) => Key::Int(i),
            None => Key::Str(s.into()),
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            Key::Int(i) => Value::Int(*i),
            Key::Str(s) => Value::Str(s.clone()),
        }
    }
}

fn canonical_int(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 20 {
        return None;
    }
    let digits = if b[0] == b'-' { &b[1..] } else { b };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    if digits[0] == b'0' && (digits.len() > 1 || b[0] == b'-') {
        return None;
    }
    s.parse().ok()
}

/// Insertion-ordered hash map with PHP key semantics.
#[derive(Clone, Debug, Default)]
pub struct PhpArray {
    entries: Vec<Option<(Key, Value)>>,
    index: HashMap<Key, usize>,
    next: i64,
    /// Internal array pointer (slot index), used by current()/next()/reset()/end().
    pub ptr: usize,
}

impl PhpArray {
    pub fn key_at(&self, slot: usize) -> Option<Key> {
        self.entries.get(slot).and_then(|e| e.as_ref()).map(|(k, _)| k.clone())
    }
    pub fn slot_count(&self) -> usize {
        self.entries.len()
    }
    pub fn last_slot(&self) -> Option<usize> {
        (0..self.entries.len()).rev().find(|i| self.entries[*i].is_some())
    }
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.index.len()
    }
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
    pub fn get(&self, k: &Key) -> Option<&Value> {
        self.index.get(k).and_then(|&i| self.entries[i].as_ref()).map(|(_, v)| v)
    }
    pub fn get_mut(&mut self, k: &Key) -> Option<&mut Value> {
        let i = *self.index.get(k)?;
        self.entries[i].as_mut().map(|(_, v)| v)
    }
    pub fn set(&mut self, k: Key, v: Value) {
        if let Some(&i) = self.index.get(&k) {
            self.entries[i].as_mut().unwrap().1 = v;
            return;
        }
        if let Key::Int(n) = k {
            if n >= self.next {
                self.next = n.saturating_add(1);
            }
        }
        self.index.insert(k.clone(), self.entries.len());
        self.entries.push(Some((k, v)));
    }
    pub fn push(&mut self, v: Value) {
        let k = Key::Int(self.next);
        self.set(k, v);
    }
    pub fn remove(&mut self, k: &Key) {
        if let Some(i) = self.index.remove(k) {
            self.entries[i] = None;
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = (&Key, &Value)> {
        self.entries.iter().flatten().map(|(k, v)| (k, v))
    }
    /// First live entry at slot >= pos, with its slot index.
    pub fn entry_at(&self, pos: usize) -> Option<(usize, &Key, &Value)> {
        (pos..self.entries.len()).find_map(|i| self.entries[i].as_ref().map(|(k, v)| (i, k, v)))
    }
    pub fn from_list(items: Vec<Value>) -> Self {
        let mut a = Self::new();
        for v in items {
            a.push(v);
        }
        a
    }
}

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    Array(Rc<PhpArray>),
    Object(Rc<Obj>),
    /// A PHP reference cell. Only ever stored in variable slots, never seen by PHP code.
    Ref(Rc<RefCell<Value>>),
    Resource(Rc<crate::stream::Resource>),
}

#[derive(Clone, Copy, Debug)]
pub enum Num {
    I(i64),
    F(f64),
}

impl Num {
    pub fn f(self) -> f64 {
        match self {
            Num::I(i) => i as f64,
            Num::F(f) => f,
        }
    }
}

/// Parse a leading numeric string. Returns the number and whether the whole string was numeric.
pub fn parse_numeric(s: &str) -> Option<(Num, bool)> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let start = i;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let ds = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - ds;
    let mut is_float = false;
    if i < b.len() && b[i] == b'.' {
        let mut k = i + 1;
        while k < b.len() && b[k].is_ascii_digit() {
            k += 1;
        }
        if digits > 0 || k > i + 1 {
            is_float = true;
            digits += k - i - 1;
            i = k;
        }
    }
    if digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut k = i + 1;
        if k < b.len() && (b[k] == b'+' || b[k] == b'-') {
            k += 1;
        }
        let es = k;
        while k < b.len() && b[k].is_ascii_digit() {
            k += 1;
        }
        if k > es {
            is_float = true;
            i = k;
        }
    }
    let txt = &s[start..i];
    let whole = b[i..].iter().all(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c));
    let n = if !is_float {
        match txt.parse::<i64>() {
            Ok(v) => Num::I(v),
            Err(_) => Num::F(txt.parse().ok()?),
        }
    } else {
        Num::F(txt.parse().ok()?)
    };
    Some((n, whole))
}

fn parse_numeric_whole(s: &str) -> Option<Num> {
    parse_numeric(s).filter(|(_, w)| *w).map(|(n, _)| n)
}

/// Digits and decimal-point position of a float: value = 0.DIGITS * 10^decpt.
fn float_digits(f: f64, precision: Option<usize>) -> (String, i32) {
    let s = match precision {
        Some(p) => format!("{:.*e}", p - 1, f.abs()),
        None => format!("{:e}", f.abs()),
    };
    let (m, e) = s.split_once('e').unwrap();
    let exp: i32 = e.parse().unwrap();
    let digits: String = m.chars().filter(|c| *c != '.').collect();
    (digits.trim_end_matches('0').to_string(), exp + 1)
}

/// PHP `%.{ndigit}G`-style float formatting (`precision` = None means shortest round-trip).
pub fn fmt_float(f: f64, precision: Option<usize>) -> String {
    if f.is_nan() {
        return "NAN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "INF".into() } else { "-INF".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let (digits, decpt) = float_digits(f, precision);
    let digits = if digits.is_empty() { "0".to_string() } else { digits };
    let ndigit = precision.map(|p| p as i32).unwrap_or(17);
    let sign = if f < 0.0 { "-" } else { "" };
    let n = digits.len() as i32;
    let body = if if decpt < 0 { decpt < -3 } else { decpt > ndigit } {
        let e = decpt - 1;
        let rest = if n > 1 { &digits[1..] } else { "0" };
        format!("{}.{}E{}{}", &digits[..1], rest, if e < 0 { '-' } else { '+' }, e.abs())
    } else if decpt <= 0 {
        format!("0.{}{}", "0".repeat((-decpt) as usize), digits)
    } else if decpt >= n {
        format!("{}{}", digits, "0".repeat((decpt - n) as usize))
    } else {
        format!("{}.{}", &digits[..decpt as usize], &digits[decpt as usize..])
    };
    format!("{sign}{body}")
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(s.into())
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
            Value::Ref(_) => "reference",
            Value::Resource(_) => "resource",
        }
    }
    pub fn deref(&self) -> Value {
        match self {
            Value::Ref(r) => r.borrow().deref(),
            v => v.clone(),
        }
    }
    pub fn to_bool(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !(s.is_empty() || &**s == "0"),
            Value::Array(a) => !a.is_empty(),
            Value::Object(_) => true,
            Value::Ref(r) => r.borrow().to_bool(),
            Value::Resource(_) => true,
        }
    }
    pub fn to_int(&self) -> i64 {
        match self {
            Value::Null => 0,
            Value::Bool(b) => *b as i64,
            Value::Int(i) => *i,
            Value::Float(f) => float_to_int(*f),
            Value::Str(s) => match parse_numeric(s) {
                Some((Num::I(i), _)) => i,
                Some((Num::F(f), _)) => float_to_int(f),
                None => 0,
            },
            Value::Array(a) => !a.is_empty() as i64,
            Value::Object(_) => 1,
            Value::Ref(r) => r.borrow().to_int(),
            Value::Resource(r) => r.id,
        }
    }
    pub fn to_float(&self) -> f64 {
        match self {
            Value::Float(f) => *f,
            Value::Str(s) => parse_numeric(s).map(|(n, _)| n.f()).unwrap_or(0.0),
            v => v.to_int() as f64,
        }
    }
    pub fn to_str(&self) -> Rc<str> {
        match self {
            Value::Str(s) => s.clone(),
            Value::Null => "".into(),
            Value::Bool(true) => "1".into(),
            Value::Bool(false) => "".into(),
            Value::Int(i) => i.to_string().into(),
            Value::Float(f) => fmt_float(*f, Some(14)).into(),
            Value::Array(_) => "Array".into(),
            Value::Object(_) => "Object".into(),
            Value::Ref(r) => r.borrow().to_str(),
            Value::Resource(r) => format!("Resource id #{}", r.id).into(),
        }
    }
}

fn float_to_int(f: f64) -> i64 {
    if f.is_nan() || f.is_infinite() {
        0
    } else if f >= -9.223372036854775808e18 && f < 9.223372036854775808e18 {
        f as i64
    } else {
        // Out of range: wrap modulo 2^64 like PHP on 64-bit platforms.
        let m = f.trunc() % 18446744073709551616.0;
        (m as i128 as u128 as u64) as i64
    }
}

fn num_of(v: &Value, op: &str, other: &Value, left: bool) -> Result<Num, String> {
    let bad = || {
        let (a, b) = if left { (v, other) } else { (other, v) };
        format!("Unsupported operand types: {} {} {}", a.type_name(), op, b.type_name())
    };
    match v {
        Value::Null => Ok(Num::I(0)),
        Value::Bool(b) => Ok(Num::I(*b as i64)),
        Value::Int(i) => Ok(Num::I(*i)),
        Value::Float(f) => Ok(Num::F(*f)),
        Value::Str(s) => parse_numeric(s).map(|(n, _)| n).ok_or_else(bad),
        Value::Array(_) | Value::Object(_) => Err(bad()),
        Value::Ref(r) => num_of(&r.borrow(), op, other, left),
        Value::Resource(r) => Ok(Num::I(r.id)),
    }
}

pub fn arith(op: &str, a: &Value, b: &Value) -> Result<Value, String> {
    if op == "+" {
        if let (Value::Array(x), Value::Array(y)) = (a, b) {
            let mut r = (**x).clone();
            for (k, v) in y.iter() {
                if r.get(k).is_none() {
                    r.set(k.clone(), v.clone());
                }
            }
            return Ok(Value::Array(Rc::new(r)));
        }
    }
    if op == "%" {
        let (x, y) = (int_operand(a, b, true)?, int_operand(b, a, false)?);
        if y == 0 {
            return Err("Modulo by zero".into());
        }
        return Ok(Value::Int(x.wrapping_rem(y)));
    }
    let (x, y) = (num_of(a, op, b, true)?, num_of(b, op, a, false)?);
    if let (Num::I(p), Num::I(q)) = (x, y) {
        let r = match op {
            "+" => p.checked_add(q),
            "-" => p.checked_sub(q),
            "*" => p.checked_mul(q),
            "/" => {
                if q == 0 {
                    return Err("Division by zero".into());
                }
                if p % q == 0 { p.checked_div(q) } else { None }
            }
            "**" if q >= 0 => u32::try_from(q).ok().and_then(|e| p.checked_pow(e)),
            _ => None,
        };
        if let Some(r) = r {
            return Ok(Value::Int(r));
        }
    }
    let (p, q) = (x.f(), y.f());
    Ok(Value::Float(match op {
        "+" => p + q,
        "-" => p - q,
        "*" => p * q,
        "/" => {
            if q == 0.0 {
                return Err("Division by zero".into());
            }
            p / q
        }
        "**" => p.powf(q),
        _ => unreachable!("arith op {op}"),
    }))
}

fn int_operand(v: &Value, other: &Value, left: bool) -> Result<i64, String> {
    num_of(v, "%", other, left).map(|n| match n {
        Num::I(i) => i,
        Num::F(f) => float_to_int(f),
    })
}

pub fn bitop(op: &str, a: &Value, b: &Value) -> Result<Value, String> {
    let (x, y) = (int_operand(a, b, true)?, int_operand(b, a, false)?);
    Ok(Value::Int(match op {
        "&" => x & y,
        "|" => x | y,
        "^" => x ^ y,
        "<<" | ">>" => {
            if y < 0 {
                return Err("Bit shift by negative number".into());
            }
            if op == "<<" {
                if y >= 64 { 0 } else { x.wrapping_shl(y as u32) }
            } else if y >= 64 {
                if x < 0 { -1 } else { 0 }
            } else {
                x >> y
            }
        }
        _ => unreachable!(),
    }))
}

fn num_cmp(a: Num, b: Num) -> Ordering {
    match (a, b) {
        (Num::I(x), Num::I(y)) => x.cmp(&y),
        _ => a.f().partial_cmp(&b.f()).unwrap_or(Ordering::Greater),
    }
}

fn as_num(v: &Value) -> Num {
    match v {
        Value::Int(i) => Num::I(*i),
        Value::Float(f) => Num::F(*f),
        _ => Num::I(0),
    }
}

pub fn loose_cmp(a: &Value, b: &Value) -> Ordering {
    use Value::*;
    match (a, b) {
        (Ref(r), _) => loose_cmp(&r.borrow(), b),
        (Resource(r), Resource(q)) => r.id.cmp(&q.id),
        (Resource(r), _) => loose_cmp(&Int(r.id), b),
        (_, Resource(r)) => loose_cmp(a, &Int(r.id)),
        (_, Ref(r)) => loose_cmp(a, &r.borrow()),
        (Int(x), Int(y)) => x.cmp(y),
        (Int(_) | Float(_), Int(_) | Float(_)) => num_cmp(as_num(a), as_num(b)),
        (Str(x), Str(y)) => match (parse_numeric_whole(x), parse_numeric_whole(y)) {
            (Some(m), Some(n)) => num_cmp(m, n),
            _ => x.as_bytes().cmp(y.as_bytes()),
        },
        (Null, Str(y)) => if y.is_empty() { Ordering::Equal } else { Ordering::Less },
        (Str(x), Null) => if x.is_empty() { Ordering::Equal } else { Ordering::Greater },
        (Bool(_) | Null, _) | (_, Bool(_) | Null) => a.to_bool().cmp(&b.to_bool()),
        (Int(_) | Float(_), Str(s)) => match parse_numeric_whole(s) {
            Some(n) => num_cmp(as_num(a), n),
            None => a.to_str().as_bytes().cmp(s.as_bytes()),
        },
        (Str(_), Int(_) | Float(_)) => loose_cmp(b, a).reverse(),
        (Array(x), Array(y)) => {
            if x.len() != y.len() {
                return x.len().cmp(&y.len());
            }
            for (k, v) in x.iter() {
                match y.get(k) {
                    None => return Ordering::Greater,
                    Some(w) => match loose_cmp(v, w) {
                        Ordering::Equal => {}
                        o => return o,
                    },
                }
            }
            Ordering::Equal
        }
        (Object(x), Object(y)) => {
            if Rc::ptr_eq(x, y) {
                Ordering::Equal
            } else if Rc::ptr_eq(&x.class, &y.class) {
                loose_cmp(&Array(Rc::new(x.props.borrow().clone())), &Array(Rc::new(y.props.borrow().clone())))
            } else {
                Ordering::Greater
            }
        }
        (Array(_) | Object(_), _) => Ordering::Greater,
        (_, Array(_) | Object(_)) => Ordering::Less,
    }
}

pub fn loose_eq(a: &Value, b: &Value) -> bool {
    // NaN is never equal to anything, which Greater from partial_cmp already encodes.
    loose_cmp(a, b) == Ordering::Equal
}

pub fn strict_eq(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (Ref(r), _) => strict_eq(&r.borrow(), b),
        (_, Ref(r)) => strict_eq(a, &r.borrow()),
        (Null, Null) => true,
        (Bool(x), Bool(y)) => x == y,
        (Int(x), Int(y)) => x == y,
        (Float(x), Float(y)) => x == y,
        (Str(x), Str(y)) => x == y,
        (Array(x), Array(y)) => {
            x.len() == y.len()
                && x.iter().zip(y.iter()).all(|((k1, v1), (k2, v2))| k1 == k2 && strict_eq(v1, v2))
        }
        (Object(x), Object(y)) => Rc::ptr_eq(x, y),
        (Resource(x), Resource(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}
