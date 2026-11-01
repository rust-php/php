//! Port of php-src/ext/standard/pack.c: pack() and unpack() with the same format codes and repeat counts.
//! Strings are UTF-8 in this engine, so bytes >= 0x80 produced by pack() are carried as their UTF-8 encoding of
//! the Latin-1 code point only when they are valid; otherwise the output is lossy (documented gap).
use crate::builtins::{arg, s, VmBuiltin};
use crate::value::*;
use crate::vm::{Flow, Vm};
use std::collections::HashMap;
use std::rc::Rc;

type R<T> = Result<T, Flow>;

/// Byte buffer: bytes are stored as chars 0..=255 (Latin-1), then encoded as UTF-8 at the end.
fn bytes_to_value(b: &[u8]) -> Value {
    match std::str::from_utf8(b) {
        Ok(t) => Value::str(t),
        Err(_) => Value::Str(b.iter().map(|&c| c as char).collect::<String>().into()),
    }
}

fn value_bytes(v: &Value) -> Vec<u8> {
    let t = v.to_str();
    t.chars().map(|c| if (c as u32) < 256 { c as u32 as u8 } else { b'?' }).collect()
}

fn int_bytes(v: i64, size: usize, big: bool) -> Vec<u8> {
    let bytes = v.to_le_bytes();
    let mut out: Vec<u8> = bytes[..size].to_vec();
    if big {
        out.reverse();
    }
    out
}

fn fmt_arg(vm: &mut Vm<'_>, code: char, n: usize) -> R<()> {
    vm.throw_error("ValueError", &format!("Type {code}: unknown format code"))?;
    let _ = n;
    Ok(())
}

pub fn pack(vm: &mut Vm<'_>, args: &[Value]) -> R<Value> {
    let fmt: Vec<char> = arg(args, 0).to_str().chars().collect();
    let mut out: Vec<u8> = Vec::new();
    let mut ai = 1usize;
    let mut i = 0usize;
    while i < fmt.len() {
        let code = fmt[i];
        i += 1;
        // repeat count: digits or '*'
        let mut count: Option<usize> = Some(1);
        let mut star = false;
        if i < fmt.len() && fmt[i] == '*' {
            star = true;
            i += 1;
        } else if i < fmt.len() && fmt[i].is_ascii_digit() {
            let mut n = 0usize;
            while i < fmt.len() && fmt[i].is_ascii_digit() {
                n = n * 10 + fmt[i].to_digit(10).unwrap() as usize;
                i += 1;
            }
            count = Some(n);
        }
        match code {
            'a' | 'A' | 'Z' | 'h' | 'H' => {
                let Some(v) = args.get(ai) else {
                    return vm.throw_error("ValueError", &format!("Type {code}: not enough arguments"));
                };
                ai += 1;
                let data = value_bytes(v);
                if code == 'h' || code == 'H' {
                    let digits: Vec<u8> = data.clone();
                    let n = if star { digits.len() } else { count.unwrap_or(1).min(digits.len()) };
                    let mut byte = 0u8;
                    for (k, d) in digits.iter().take(n).enumerate() {
                        let nib = match d {
                            b'0'..=b'9' => d - b'0',
                            b'a'..=b'f' => d - b'a' + 10,
                            b'A'..=b'F' => d - b'A' + 10,
                            _ => 0,
                        };
                        let high_first = code == 'H';
                        if k % 2 == 0 {
                            byte = if high_first { nib << 4 } else { nib };
                        } else {
                            byte |= if high_first { nib } else { nib << 4 };
                            out.push(byte);
                        }
                    }
                    if n % 2 == 1 {
                        out.push(byte);
                    }
                    continue;
                }
                let width = if star { data.len() + usize::from(code == 'Z') } else { count.unwrap_or(1) };
                let pad = if code == 'A' { b' ' } else { 0 };
                let mut field: Vec<u8> = data.iter().take(if code == 'Z' { width.saturating_sub(1) } else { width }).copied().collect();
                field.resize(width, pad);
                out.extend(field);
            }
            'x' => {
                let n = if star { 0 } else { count.unwrap_or(1) };
                out.extend(std::iter::repeat_n(0u8, n));
            }
            'X' => {
                let n = if star { 0 } else { count.unwrap_or(1) };
                out.truncate(out.len().saturating_sub(n));
            }
            '@' => {
                let n = count.unwrap_or(0);
                out.resize(n, 0);
            }
            _ => {
                let (size, big, float) = match code {
                    'c' | 'C' => (1, false, false),
                    's' | 'S' | 'v' => (2, false, false),
                    'n' => (2, true, false),
                    'i' | 'I' | 'l' | 'L' | 'V' => (4, false, false),
                    'N' => (4, true, false),
                    'q' | 'Q' | 'P' => (8, false, false),
                    'J' => (8, true, false),
                    'f' | 'g' => (4, false, true),
                    'G' => (4, true, true),
                    'd' | 'e' => (8, false, true),
                    'E' => (8, true, true),
                    _ => return vm.throw_error("ValueError", &format!("Type {code}: unknown format code")),
                };
                let reps = if star { s_count(args, ai) } else { count.unwrap_or(1) };
                for _ in 0..reps {
                    let Some(v) = args.get(ai) else {
                        return vm.throw_error("ValueError", &format!("Type {code}: too few arguments"));
                    };
                    ai += 1;
                    if float {
                        let f = v.to_float();
                        let bytes: Vec<u8> = if size == 4 {
                            let x = f as f32;
                            if big { x.to_be_bytes().to_vec() } else { x.to_le_bytes().to_vec() }
                        } else if big {
                            f.to_be_bytes().to_vec()
                        } else {
                            f.to_le_bytes().to_vec()
                        };
                        out.extend(bytes);
                    } else {
                        out.extend(int_bytes(v.to_int(), size, big));
                    }
                }
            }
        }
    }
    let _ = fmt_arg;
    Ok(bytes_to_value(&out))
}

/// For `*` on numeric codes PHP packs every remaining argument.
fn s_count(args: &[Value], from: usize) -> usize {
    args.len().saturating_sub(from)
}

pub fn unpack(vm: &mut Vm<'_>, args: &[Value]) -> R<Value> {
    let fmt = arg(args, 0).to_str().to_string();
    let data = value_bytes(&arg(args, 1));
    let mut pos = arg(args, 2).to_int().max(0) as usize;
    let mut out = PhpArray::new();
    for part in fmt.split('/') {
        let chars: Vec<char> = part.chars().collect();
        if chars.is_empty() {
            continue;
        }
        let code = chars[0];
        let mut i = 1usize;
        let mut count: Option<usize> = Some(1);
        let mut star = false;
        if i < chars.len() && chars[i] == '*' {
            star = true;
            i += 1;
        } else if i < chars.len() && chars[i].is_ascii_digit() {
            let mut n = 0usize;
            while i < chars.len() && chars[i].is_ascii_digit() {
                n = n * 10 + chars[i].to_digit(10).unwrap() as usize;
                i += 1;
            }
            count = Some(n);
        }
        let name: String = chars[i..].iter().collect();
        match code {
            'a' | 'A' | 'Z' => {
                let n = if star { data.len().saturating_sub(pos) } else { count.unwrap_or(1) };
                if pos + n > data.len() {
                    return vm.throw_error("ValueError", &format!("Type {code}: not enough input, need {n}, have {}", data.len().saturating_sub(pos)));
                }
                let mut field = data[pos..pos + n].to_vec();
                pos += n;
                match code {
                    'A' => {
                        while matches!(field.last(), Some(b' ' | 0 | b'\t' | b'\r' | b'\n')) {
                            field.pop();
                        }
                    }
                    'Z' => {
                        if let Some(z) = field.iter().position(|b| *b == 0) {
                            field.truncate(z);
                        }
                    }
                    _ => {}
                }
                out.set(Key::from_str(&key_name(&name, 1)), bytes_to_value(&field));
            }
            'h' | 'H' => {
                let nibbles = if star { (data.len() - pos) * 2 } else { count.unwrap_or(1) };
                let bytes = nibbles.div_ceil(2);
                if pos + bytes > data.len() {
                    return vm.throw_error("ValueError", &format!("Type {code}: not enough input"));
                }
                let mut hex = String::new();
                for k in 0..nibbles {
                    let b = data[pos + k / 2];
                    let nib = if (k % 2 == 0) == (code == 'H') { b >> 4 } else { b & 15 };
                    hex.push(std::char::from_digit(nib as u32, 16).unwrap());
                }
                pos += bytes;
                out.set(Key::from_str(&key_name(&name, 1)), s(&hex));
            }
            'x' => pos += if star { 0 } else { count.unwrap_or(1) },
            'X' => pos = pos.saturating_sub(if star { 0 } else { count.unwrap_or(1) }),
            '@' => pos = count.unwrap_or(0),
            _ => {
                let (size, big, float, signed) = match code {
                    'c' => (1, false, false, true),
                    'C' => (1, false, false, false),
                    's' => (2, false, false, true),
                    'S' | 'v' => (2, false, false, false),
                    'n' => (2, true, false, false),
                    'i' | 'l' => (4, false, false, true),
                    'I' | 'L' | 'V' => (4, false, false, false),
                    'N' => (4, true, false, false),
                    'q' => (8, false, false, true),
                    'Q' | 'P' => (8, false, false, false),
                    'J' => (8, true, false, false),
                    'f' | 'g' => (4, false, true, false),
                    'G' => (4, true, true, false),
                    'd' | 'e' => (8, false, true, false),
                    'E' => (8, true, true, false),
                    _ => return vm.throw_error("ValueError", &format!("Type {code}: unknown format code")),
                };
                let reps = if star { (data.len() - pos) / size } else { count.unwrap_or(1) };
                for k in 0..reps {
                    if pos + size > data.len() {
                        return vm.throw_error("ValueError", &format!("Type {code}: not enough input"));
                    }
                    let mut chunk = data[pos..pos + size].to_vec();
                    pos += size;
                    if big {
                        chunk.reverse();
                    }
                    let v = if float {
                        if size == 4 {
                            Value::Float(f32::from_le_bytes(chunk.try_into().unwrap()) as f64)
                        } else {
                            Value::Float(f64::from_le_bytes(chunk.try_into().unwrap()))
                        }
                    } else {
                        let mut buf = [0u8; 8];
                        buf[..size].copy_from_slice(&chunk);
                        let raw = u64::from_le_bytes(buf);
                        let val = if signed && size < 8 && (raw >> (size * 8 - 1)) & 1 == 1 {
                            (raw | (!0u64 << (size * 8))) as i64
                        } else {
                            raw as i64
                        };
                        Value::Int(val)
                    };
                    let key = if reps > 1 || star { format!("{name}{}", k + 1) } else { name.clone() };
                    let key = if name.is_empty() { (k + 1).to_string() } else { key };
                    out.set(Key::from_str(&key), v);
                }
            }
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

fn key_name(name: &str, idx: usize) -> String {
    if name.is_empty() { idx.to_string() } else { name.to_string() }
}

pub fn register_vm(m: &mut HashMap<&'static str, VmBuiltin>) {
    let list: &[(&str, VmBuiltin)] = &[
        ("pack", |vm, a| pack(vm, &a)),
        ("unpack", |vm, a| unpack(vm, &a)),
    ];
    for (k, f) in list {
        m.insert(k, *f);
    }
}
