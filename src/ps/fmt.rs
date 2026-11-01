//! Port of ext/standard/formatted_print.c (+ zend_gcvt, php_conv_fp pieces).
use super::util::*;
use crate::builtins::VmBuiltin;
use crate::value::*;
use crate::vm::Vm;
use std::collections::HashMap;

const ALIGN_LEFT: i32 = 0;
const ALIGN_RIGHT: i32 = 1;
const ADJ_WIDTH: i32 = 1;
const ADJ_PRECISION: i32 = 2;
const FLOAT_PRECISION: i32 = 6;
const MAX_FLOAT_PRECISION: i32 = 53;

/// dtoa digits: (digits without trailing zeros, decpt, negative). mode2 with ndigit significant digits, or shortest if None.
fn dtoa(v: f64, ndigit: Option<usize>) -> (String, i32, bool) {
    let neg = v.is_sign_negative();
    if v == 0.0 {
        return ("0".into(), 1, neg);
    }
    let a = v.abs();
    let s = match ndigit {
        Some(n) => format!("{:.*e}", n.max(1) - 1, a),
        None => format!("{:e}", a),
    };
    let (m, e) = s.split_once('e').unwrap();
    let exp: i32 = e.parse().unwrap();
    let digits: String = m.chars().filter(|c| *c != '.').collect();
    let d = digits.trim_end_matches('0');
    (if d.is_empty() { "0".to_string() } else { d.to_string() }, exp + 1, neg)
}

/// zend_gcvt
pub fn gcvt(value: f64, ndigit: i32, dec_point: char, exponent: char) -> String {
    let (nd, mode0) = if ndigit >= 0 { (ndigit, false) } else { (17, true) };
    let (digits, mut decpt, sign) = dtoa(value, if mode0 { None } else { Some(nd as usize) });
    let mut out = String::new();
    if sign {
        out.push('-');
    }
    let d = digits.as_bytes();
    if (decpt >= 0 && decpt > nd) || decpt < -3 {
        decpt -= 1;
        let neg = decpt < 0;
        if neg {
            decpt = -decpt;
        }
        out.push(d[0] as char);
        out.push(dec_point);
        if d.len() == 1 {
            out.push('0');
        } else {
            out.push_str(&digits[1..]);
        }
        out.push(exponent);
        out.push(if neg { '-' } else { '+' });
        out.push_str(&decpt.to_string());
    } else if decpt < 0 {
        out.push('0');
        out.push(dec_point);
        for _ in 0..(-decpt) {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        let mut src = 0;
        for _ in 0..decpt {
            if src < d.len() {
                out.push(d[src] as char);
                src += 1;
            } else {
                out.push('0');
            }
        }
        if src < d.len() {
            if src == 0 {
                out.push('0');
            }
            out.push(dec_point);
            out.push_str(&digits[decpt as usize..]);
        }
    }
    out
}

/// php_conv_fp for 'F' / 'e' / 'E'; returns (text, is_negative)
fn conv_fp(format: char, num: f64, add_dp: bool, mut precision: i32, dec_point: char) -> (String, bool) {
    const NDIG: i32 = 320;
    if precision >= NDIG - 1 {
        precision = NDIG - 2;
    }
    let neg = num.is_sign_negative() && num != 0.0 || (num == 0.0 && num.is_sign_negative());
    let a = num.abs();
    let mut out = String::new();
    if format == 'F' {
        let s = format!("{:.*}", precision as usize, a);
        match s.split_once('.') {
            Some((i, f)) => {
                out.push_str(i);
                out.push(dec_point);
                out.push_str(f);
            }
            None => {
                out.push_str(&s);
                if add_dp {
                    out.push(dec_point);
                }
            }
        }
    } else {
        let s = format!("{:.*e}", precision as usize, a);
        let (m, e) = s.split_once('e').unwrap();
        let exp: i32 = e.parse().unwrap();
        out.push_str(m);
        if precision == 0 && add_dp {
            out.push('.');
        }
        out.push(format);
        out.push(if exp < 0 { '-' } else { '+' });
        out.push_str(&exp.abs().to_string());
    }
    (out, neg)
}

struct Spec<'a> {
    out: &'a mut Vec<u8>,
}

impl Spec<'_> {
    #[allow(clippy::too_many_arguments)]
    fn append_string(&mut self, add: &[u8], min_width: usize, max_width: usize, padding: u8, alignment: i32, neg: bool, expprec: bool, always_sign: bool) {
        let len = add.len();
        let copy_len = if expprec { max_width.min(len) } else { len };
        let npad = min_width.saturating_sub(copy_len);
        let mut add = &add[..copy_len];
        if alignment == ALIGN_RIGHT {
            if (neg || always_sign) && padding == b'0' && !add.is_empty() {
                self.out.push(if neg { b'-' } else { b'+' });
                add = &add[1..];
            }
            for _ in 0..npad {
                self.out.push(padding);
            }
        }
        self.out.extend_from_slice(add);
        if alignment == ALIGN_LEFT {
            for _ in 0..npad {
                self.out.push(padding);
            }
        }
    }

    fn append_int(&mut self, number: i64, width: usize, mut padding: u8, alignment: i32, always_sign: bool) {
        let neg = number < 0;
        let magn = number.unsigned_abs();
        if alignment == 0 && padding == b'0' {
            padding = b' ';
        }
        let mut s = magn.to_string();
        if neg {
            s.insert(0, '-');
        } else if always_sign {
            s.insert(0, '+');
        }
        self.append_string(s.as_bytes(), width, 0, padding, alignment, neg, false, always_sign);
    }

    fn append_uint(&mut self, number: u64, width: usize, mut padding: u8, alignment: i32) {
        if alignment == 0 && padding == b'0' {
            padding = b' ';
        }
        self.append_string(number.to_string().as_bytes(), width, 0, padding, alignment, false, false, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn append_double(&mut self, vm: &mut Vm<'_>, number: f64, width: usize, padding: u8, alignment: i32, mut precision: i32, adjust: i32, fmt: char, always_sign: bool) {
        if adjust & ADJ_PRECISION == 0 {
            precision = FLOAT_PRECISION;
        } else if precision > MAX_FLOAT_PRECISION {
            notice(vm, &format!("sprintf(): Requested precision of {precision} digits was truncated to PHP maximum of {MAX_FLOAT_PRECISION} digits"));
            precision = MAX_FLOAT_PRECISION;
        }
        if number.is_nan() {
            self.append_string(b"NaN", 3, 0, padding, alignment, false, false, always_sign);
            return;
        }
        if number.is_infinite() {
            let neg = number < 0.0;
            let s: &[u8] = if neg { b"-INF" } else { b"INF" };
            self.append_string(s, s.len(), 0, padding, alignment, neg, false, always_sign);
            return;
        }
        let (s, is_negative);
        match fmt {
            'e' | 'E' | 'f' | 'F' => {
                let (t, n) = conv_fp(if fmt == 'f' { 'F' } else { fmt }, number, false, precision, '.');
                is_negative = n;
                s = if n {
                    format!("-{t}")
                } else if always_sign {
                    format!("+{t}")
                } else {
                    t
                };
            }
            _ => {
                if precision == 0 {
                    precision = 1;
                }
                let exp_char = if fmt == 'G' || fmt == 'H' { 'E' } else { 'e' };
                let t = gcvt(number, precision, '.', exp_char);
                if t.starts_with('-') {
                    is_negative = true;
                    s = t;
                } else {
                    is_negative = false;
                    s = if always_sign { format!("+{t}") } else { t };
                }
            }
        }
        self.append_string(s.as_bytes(), width, 0, padding, alignment, is_negative, false, always_sign);
    }

    #[allow(clippy::too_many_arguments)]
    fn append_2n(&mut self, number: i64, width: usize, padding: u8, alignment: i32, n: u32, table: &[u8], expprec: bool) {
        let mut num = number as u64;
        let andbits = (1u64 << n) - 1;
        let mut buf = vec![];
        loop {
            buf.push(table[(num & andbits) as usize]);
            num >>= n;
            if num == 0 {
                break;
            }
        }
        buf.reverse();
        self.append_string(&buf, width, 0, padding, alignment, false, expprec, false);
    }
}

fn is_digit(c: Option<&u8>) -> bool {
    c.is_some_and(|c| c.is_ascii_digit())
}

/// php_sprintf_getnumber: strtol then range check. Returns -1 on overflow.
fn getnumber(f: &[u8], pos: &mut usize) -> i64 {
    let start = *pos;
    let mut p = start;
    while is_digit(f.get(p)) {
        p += 1;
    }
    *pos = p;
    let s = std::str::from_utf8(&f[start..p]).unwrap_or("0");
    match s.parse::<i64>() {
        Ok(n) if n < i32::MAX as i64 && n >= 0 => n,
        _ => -1,
    }
}

const ARG_NUM_NEXT: i64 = -1;
const ARG_NUM_INVALID: i64 = -2;

fn get_argnum(vm: &mut Vm<'_>, f: &[u8], pos: &mut usize) -> R<i64> {
    let mut t = *pos;
    while is_digit(f.get(t)) {
        t += 1;
    }
    if f.get(t) != Some(&b'$') {
        return Ok(ARG_NUM_NEXT);
    }
    if t == *pos {
        verr::<()>(vm, "Argument number specifier must not be empty")?;
    }
    let n = getnumber(f, pos);
    if n <= 0 {
        verr::<()>(vm, &format!("Argument number specifier must be greater than zero and less than {}", i32::MAX))?;
    }
    *pos += 1;
    Ok(n - 1)
}

/// php_formatted_print. `nb_additional` is -1 for vsprintf-style calls.
pub fn formatted_print(vm: &mut Vm<'_>, format: &str, args: &[Value], nb_additional: i64) -> R<String> {
    let f = format.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    let argc = args.len() as i64;
    let mut pos = 0usize;
    let mut currarg: i64 = 0;
    let mut max_missing: i64 = -1;
    const HEX: &[u8] = b"0123456789abcdef";
    const HEXU: &[u8] = b"0123456789ABCDEF";

    while pos < f.len() {
        let Some(off) = f[pos..].iter().position(|c| *c == b'%') else {
            out.extend_from_slice(&f[pos..]);
            break;
        };
        out.extend_from_slice(&f[pos..pos + off]);
        pos += off + 1;
        if f.get(pos) == Some(&b'%') {
            out.push(b'%');
            pos += 1;
            continue;
        }
        let mut alignment = ALIGN_RIGHT;
        let mut adjusting = 0;
        let mut padding = b' ';
        let mut always_sign = false;
        let mut expprec = false;
        let (mut width, mut precision): (i64, i64);
        let mut argnum: i64;
        let cur = f.get(pos).copied().unwrap_or(0);
        if cur.is_ascii_alphabetic() {
            width = 0;
            precision = 0;
            argnum = ARG_NUM_NEXT;
        } else {
            argnum = get_argnum(vm, f, &mut pos)?;
            loop {
                match f.get(pos) {
                    Some(b' ') | Some(b'0') => padding = f[pos],
                    Some(b'-') => alignment = ALIGN_LEFT,
                    Some(b'+') => always_sign = true,
                    Some(b'\'') => {
                        if pos + 1 < f.len() {
                            pos += 1;
                            padding = f[pos];
                        } else {
                            return verr(vm, "Missing padding character");
                        }
                    }
                    _ => break,
                }
                pos += 1;
            }
            // width
            if f.get(pos) == Some(&b'*') {
                pos += 1;
                let mut wa = get_argnum(vm, f, &mut pos)?;
                if wa == ARG_NUM_NEXT {
                    wa = currarg;
                    currarg += 1;
                }
                if wa >= argc {
                    max_missing = max_missing.max(wa);
                    continue;
                }
                let t = args[wa as usize].deref();
                let Value::Int(w) = t else { return verr(vm, "Width must be an integer") };
                if w < 0 || w > i32::MAX as i64 {
                    return verr(vm, &format!("Width must be between 0 and {}", i32::MAX));
                }
                width = w;
                adjusting |= ADJ_WIDTH;
            } else if is_digit(f.get(pos)) {
                width = getnumber(f, &mut pos);
                if width < 0 {
                    return verr(vm, &format!("Width must be between 0 and {}", i32::MAX));
                }
                adjusting |= ADJ_WIDTH;
            } else {
                width = 0;
            }
            // precision
            if f.get(pos) == Some(&b'.') {
                pos += 1;
                if f.get(pos) == Some(&b'*') {
                    pos += 1;
                    let mut pa = get_argnum(vm, f, &mut pos)?;
                    if pa == ARG_NUM_NEXT {
                        pa = currarg;
                        currarg += 1;
                    }
                    if pa >= argc {
                        max_missing = max_missing.max(pa);
                        continue;
                    }
                    let t = args[pa as usize].deref();
                    let Value::Int(p) = t else { return verr(vm, "Precision must be an integer") };
                    if p < -1 || p > i32::MAX as i64 {
                        return verr(vm, &format!("Precision must be between -1 and {}", i32::MAX));
                    }
                    precision = p;
                    adjusting |= ADJ_PRECISION;
                    expprec = true;
                } else if is_digit(f.get(pos)) {
                    precision = getnumber(f, &mut pos);
                    if precision < 0 {
                        return verr(vm, &format!("Precision must be between 0 and {}", i32::MAX));
                    }
                    adjusting |= ADJ_PRECISION;
                    expprec = true;
                } else {
                    precision = 0;
                    adjusting |= ADJ_PRECISION;
                }
            } else {
                precision = 0;
            }
        }
        if f.get(pos) == Some(&b'l') {
            pos += 1;
        }
        let spec = f.get(pos).copied().unwrap_or(0);
        if argnum == ARG_NUM_NEXT {
            argnum = currarg;
            currarg += 1;
        }
        if argnum >= argc {
            max_missing = max_missing.max(argnum);
            continue;
        }
        if expprec && precision == -1 && !matches!(spec, b'g' | b'G' | b'h' | b'H') {
            return verr(vm, "Precision -1 is only supported for %g, %G, %h and %H");
        }
        let tmp = args[argnum as usize].deref();
        let (w, pr) = (width as usize, precision as usize);
        let mut sp = Spec { out: &mut out };
        match spec {
            b's' => {
                let s = tmp.to_str();
                sp.append_string(s.as_bytes(), w, pr, padding, alignment, false, expprec, false);
            }
            b'd' => sp.append_int(tmp.to_int(), w, padding, alignment, always_sign),
            b'u' => sp.append_uint(tmp.to_int() as u64, w, padding, alignment),
            b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'h' | b'H' => {
                sp.append_double(vm, tmp.to_float(), w, padding, alignment, precision as i32, adjusting, spec as char, always_sign)
            }
            b'c' => sp.out.push(tmp.to_int() as u8),
            b'o' => sp.append_2n(tmp.to_int(), w, padding, alignment, 3, HEX, expprec),
            b'x' => sp.append_2n(tmp.to_int(), w, padding, alignment, 4, HEX, expprec),
            b'X' => sp.append_2n(tmp.to_int(), w, padding, alignment, 4, HEXU, expprec),
            b'b' => sp.append_2n(tmp.to_int(), w, padding, alignment, 1, HEX, expprec),
            b'%' => sp.out.push(b'%'),
            0 if pos >= f.len() => return verr(vm, "Missing format specifier at end of string"),
            c => return verr(vm, &format!("Unknown format specifier \"{}\"", c as char)),
        }
        pos += 1;
    }
    if max_missing >= 0 {
        if nb_additional == -1 {
            return verr(vm, &format!("The arguments array must contain {} items, {} given", max_missing + 1, argc));
        }
        let msg = format!("{} arguments are required, {} given", max_missing + nb_additional + 1, argc + nb_additional);
        return vm.throw_error("ArgumentCountError", &msg);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

fn array_args(a: &PhpArray) -> Vec<Value> {
    a.iter().map(|(_, v)| v.deref()).collect()
}

fn sprintf(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "sprintf", &a, 1, -1)?;
    let fmt = p_str(vm, "sprintf", &a, 0, "format")?;
    formatted_print(vm, &fmt, &a[1..], 1).map(|s| vstr(&s))
}

fn vsprintf(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "vsprintf", &a, 2, 2)?;
    let fmt = p_str(vm, "vsprintf", &a, 0, "format")?;
    let arr = p_array(vm, "vsprintf", &a, 1, "values")?;
    formatted_print(vm, &fmt, &array_args(&arr), -1).map(|s| vstr(&s))
}

fn printf(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "printf", &a, 1, -1)?;
    let fmt = p_str(vm, "printf", &a, 0, "format")?;
    let s = formatted_print(vm, &fmt, &a[1..], 1)?;
    vm.write(&s);
    Ok(Value::Int(s.len() as i64))
}

fn vprintf(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "vprintf", &a, 2, 2)?;
    let fmt = p_str(vm, "vprintf", &a, 0, "format")?;
    let arr = p_array(vm, "vprintf", &a, 1, "values")?;
    let s = formatted_print(vm, &fmt, &array_args(&arr), -1)?;
    vm.write(&s);
    Ok(Value::Int(s.len() as i64))
}

fn fprintf(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "fprintf", &a, 2, -1)?;
    let fmt = p_str(vm, "fprintf", &a, 1, "format")?;
    let s = formatted_print(vm, &fmt, &a[2..], 2)?;
    vm.call_function_named("fwrite", vec![a[0].clone(), vstr(&s)])?;
    Ok(Value::Int(s.len() as i64))
}

fn vfprintf(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "vfprintf", &a, 3, 3)?;
    let fmt = p_str(vm, "vfprintf", &a, 1, "format")?;
    let arr = p_array(vm, "vfprintf", &a, 2, "values")?;
    let s = formatted_print(vm, &fmt, &array_args(&arr), -1)?;
    vm.call_function_named("fwrite", vec![a[0].clone(), vstr(&s)])?;
    Ok(Value::Int(s.len() as i64))
}

pub fn register(m: &mut HashMap<&'static str, VmBuiltin>) {
    m.insert("sprintf", sprintf);
    m.insert("vsprintf", vsprintf);
    m.insert("printf", printf);
    m.insert("vprintf", vprintf);
    m.insert("fprintf", fprintf);
    m.insert("vfprintf", vfprintf);
}
