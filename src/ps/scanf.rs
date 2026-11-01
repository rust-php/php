//! Port of ext/standard/scanf.c (sscanf, array-return mode; by-reference variables are not supported).
use super::util::*;
use crate::builtins::VmBuiltin;
use crate::value::*;
use crate::vm::Vm;
use std::collections::HashMap;

const SCAN_NOSKIP: u32 = 0x1;
const SCAN_SUPPRESS: u32 = 0x2;
const SCAN_UNSIGNED: u32 = 0x4;
const SCAN_SIGNOK: u32 = 0x10;
const SCAN_NODIGITS: u32 = 0x20;
const SCAN_NOZERO: u32 = 0x40;
const SCAN_XOK: u32 = 0x80;
const SCAN_PTOK: u32 = 0x100;
const SCAN_EXPOK: u32 = 0x200;

struct CharSet {
    exclude: bool,
    chars: Vec<u8>,
    ranges: Vec<(u8, u8)>,
}

fn at(b: &[u8], i: usize) -> u8 {
    b.get(i).copied().unwrap_or(0)
}

/// BuildCharSet: `fi` points after '['. Returns the index after the closing ']'.
fn build_charset(f: &[u8], mut fi: usize) -> (CharSet, usize) {
    let mut cs = CharSet { exclude: false, chars: vec![], ranges: vec![] };
    let mut ch = at(f, fi);
    if ch == b'^' {
        cs.exclude = true;
        fi += 1;
        ch = at(f, fi);
    }
    // ch = *format; format points to first char
    let mut p = fi; // index of ch
    let mut start = ch;
    let mut next = p + 1;
    if ch == b']' || ch == b'-' {
        cs.chars.push(ch);
        p = next;
        next += 1;
        ch = at(f, p);
    }
    let _ = &mut start;
    let mut ch = ch;
    let mut start = start;
    while ch != b']' && ch != 0 {
        if at(f, next) == b'-' {
            start = ch;
        } else if ch == b'-' {
            if at(f, next) == b']' {
                cs.chars.push(start);
                cs.chars.push(ch);
            } else {
                p = next;
                next += 1;
                ch = at(f, p);
                if start < ch {
                    cs.ranges.push((start, ch));
                } else {
                    cs.ranges.push((ch, start));
                }
            }
        } else {
            cs.chars.push(ch);
        }
        p = next;
        next += 1;
        ch = at(f, p);
    }
    (cs, next)
}

fn in_set(cs: &CharSet, c: u8) -> bool {
    let m = cs.chars.contains(&c) || cs.ranges.iter().any(|r| r.0 <= c && c <= r.1);
    if cs.exclude {
        !m
    } else {
        m
    }
}

fn strtol(s: &[u8], base: u32) -> i64 {
    // strtol(buf, NULL, base) semantics for the restricted buffers produced above
    let mut i = 0;
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut base = base;
    if (base == 0 || base == 16) && i + 1 < s.len() && s[i] == b'0' && (s[i + 1] == b'x' || s[i + 1] == b'X') {
        i += 2;
        base = 16;
    } else if base == 0 {
        base = if i < s.len() && s[i] == b'0' { 8 } else { 10 };
    }
    let mut v: i128 = 0;
    while i < s.len() {
        let Some(d) = (s[i] as char).to_digit(base) else { break };
        v = v * base as i128 + d as i128;
        if v > u64::MAX as i128 {
            v = u64::MAX as i128;
        }
        i += 1;
    }
    if neg {
        v = -v;
    }
    v.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn validate_format(vm: &mut Vm<'_>, f: &[u8], num_vars: usize) -> R<Option<usize>> {
    let mut i = 0;
    let (mut got_xpg, mut got_seq) = (false, false);
    let mut obj_index = 0usize;
    let mut xpg_size = 0usize;
    let mut nassign: Vec<i32> = vec![0; num_vars.max(16)];
    while i < f.len() {
        let mut ch = f[i];
        i += 1;
        let mut flags = 0u32;
        if ch != b'%' {
            continue;
        }
        ch = at(f, i);
        i += 1;
        if ch == b'%' {
            continue;
        }
        let mut xpg_done = false;
        if ch == b'*' {
            flags |= SCAN_SUPPRESS;
            ch = at(f, i);
            i += 1;
            xpg_done = true;
        } else if ch.is_ascii_digit() {
            let mut e = i - 1;
            while at(f, e).is_ascii_digit() {
                e += 1;
            }
            if at(f, e) == b'$' {
                let value: usize = std::str::from_utf8(&f[i - 1..e]).unwrap_or("0").parse().unwrap_or(0);
                i = e + 1;
                ch = at(f, i);
                i += 1;
                got_xpg = true;
                if got_seq {
                    return verr(vm, "cannot mix \"%\" and \"%n$\" conversion specifiers");
                }
                if value < 1 || (num_vars > 0 && value > num_vars) || (num_vars == 0 && value > 255) {
                    return verr(vm, "\"%n$\" argument index out of range");
                }
                if num_vars == 0 {
                    xpg_size = xpg_size.max(value);
                }
                obj_index = value - 1;
                xpg_done = true;
            }
        }
        if !xpg_done {
            got_seq = true;
            if got_xpg {
                return verr(vm, "cannot mix \"%\" and \"%n$\" conversion specifiers");
            }
        }
        if ch.is_ascii_digit() {
            let mut e = i - 1;
            while at(f, e).is_ascii_digit() {
                e += 1;
            }
            flags |= 8;
            i = e;
            ch = at(f, i);
            i += 1;
        }
        if ch == b'l' || ch == b'L' || ch == b'h' {
            ch = at(f, i);
            i += 1;
        }
        if flags & SCAN_SUPPRESS == 0 && num_vars > 0 && obj_index >= num_vars {
            return verr(vm, if got_xpg { "\"%n$\" argument index out of range" } else { "Different numbers of variable names and field specifiers" });
        }
        match ch {
            b'n' | b'd' | b'D' | b'i' | b'o' | b'x' | b'X' | b'u' | b'f' | b'e' | b'E' | b'g' | b's' => {}
            b'c' => {
                if flags & 8 != 0 {
                    warn(vm, "sscanf(): Field width may not be specified in %c conversion");
                    return Ok(None);
                }
            }
            b'[' => {
                if at(f, i) == 0 {
                    return verr(vm, "Unmatched [ in format string");
                }
                ch = f[i];
                i += 1;
                if ch == b'^' {
                    if at(f, i) == 0 {
                        return verr(vm, "Unmatched [ in format string");
                    }
                    ch = f[i];
                    i += 1;
                }
                if ch == b']' {
                    if at(f, i) == 0 {
                        return verr(vm, "Unmatched [ in format string");
                    }
                    ch = f[i];
                    i += 1;
                }
                while ch != b']' {
                    if at(f, i) == 0 {
                        return verr(vm, "Unmatched [ in format string");
                    }
                    ch = f[i];
                    i += 1;
                }
            }
            c => return verr(vm, &format!("Bad scan conversion character \"{}\"", c as char)),
        }
        if flags & SCAN_SUPPRESS == 0 {
            if obj_index >= nassign.len() {
                nassign.resize(obj_index + 16, 0);
            }
            nassign[obj_index] += 1;
            obj_index += 1;
        }
    }
    let total = if num_vars == 0 {
        if xpg_size > 0 {
            xpg_size
        } else {
            obj_index
        }
    } else {
        num_vars
    };
    for n in nassign.iter().take(total) {
        if *n > 1 {
            return verr(vm, "Variable is assigned by multiple \"%n$\" conversion specifiers");
        } else if xpg_size == 0 && *n == 0 {
            return verr(vm, "Variable is not assigned by any conversion specifiers");
        }
    }
    Ok(Some(total))
}

fn sscanf(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "sscanf", &a, 2, -1)?;
    let s = p_str(vm, "sscanf", &a, 0, "string")?;
    let fmt = p_str(vm, "sscanf", &a, 1, "format")?;
    let num_vars = a.len() - 2;
    let (st, f) = (s.as_bytes(), fmt.as_bytes());
    let Some(total) = validate_format(vm, f, num_vars)? else {
        return Ok(if num_vars > 0 { Value::Int(-1) } else { Value::Null });
    };
    let mut res: Vec<Value> = if num_vars == 0 { vec![Value::Null; total] } else { vec![] };
    let mut obj_index = 0usize;
    let mut si = 0usize;
    let mut fi = 0usize;
    let mut nconv = 0i64;
    let mut underflow = false;
    let set = |res: &mut Vec<Value>, idx: usize, v: Value| {
        if idx < res.len() {
            res[idx] = v;
        } else if num_vars == 0 {
            res.resize(idx + 1, Value::Null);
            res[idx] = v;
        }
    };
    'main: while fi < f.len() {
        let mut ch = f[fi];
        fi += 1;
        let mut flags = 0u32;
        if matches!(ch, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            while si < st.len() && matches!(st[si], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
                si += 1;
            }
            continue;
        }
        let mut literal = ch != b'%';
        if !literal {
            ch = at(f, fi);
            fi += 1;
            if ch == b'%' {
                literal = true;
            }
        }
        if literal {
            if si >= st.len() {
                underflow = true;
                break 'main;
            }
            let sch = st[si];
            si += 1;
            if ch != sch {
                break 'main;
            }
            continue;
        }
        if ch == b'*' {
            flags |= SCAN_SUPPRESS;
            ch = at(f, fi);
            fi += 1;
        } else if ch.is_ascii_digit() {
            let mut e = fi - 1;
            while at(f, e).is_ascii_digit() {
                e += 1;
            }
            if at(f, e) == b'$' {
                let v: usize = std::str::from_utf8(&f[fi - 1..e]).unwrap_or("1").parse().unwrap_or(1);
                fi = e + 1;
                ch = at(f, fi);
                fi += 1;
                obj_index = v - 1;
            }
        }
        let mut width: usize = 0;
        if ch.is_ascii_digit() {
            let mut e = fi - 1;
            while at(f, e).is_ascii_digit() {
                e += 1;
            }
            width = std::str::from_utf8(&f[fi - 1..e]).unwrap_or("0").parse().unwrap_or(0);
            fi = e;
            ch = at(f, fi);
            fi += 1;
        }
        if ch == b'l' || ch == b'L' || ch == b'h' {
            ch = at(f, fi);
            fi += 1;
        }
        let mut op = 0u8;
        let mut base: u32 = 0;
        let mut unsigned = false;
        match ch {
            b'n' => {
                if flags & SCAN_SUPPRESS == 0 {
                    set(&mut res, obj_index, Value::Int(si as i64));
                    obj_index += 1;
                }
                nconv += 1;
                continue;
            }
            b'd' | b'D' => {
                op = b'i';
                base = 10;
            }
            b'i' => {
                op = b'i';
                base = 0;
            }
            b'o' => {
                op = b'i';
                base = 8;
            }
            b'x' | b'X' => {
                op = b'i';
                base = 16;
            }
            b'u' => {
                op = b'i';
                base = 10;
                unsigned = true;
            }
            b'f' | b'e' | b'E' | b'g' => op = b'f',
            b's' => op = b's',
            b'c' => {
                op = b's';
                flags |= SCAN_NOSKIP;
                if width == 0 {
                    width = 1;
                }
            }
            b'[' => {
                op = b'[';
                flags |= SCAN_NOSKIP;
            }
            _ => {}
        }
        if si >= st.len() {
            underflow = true;
            break 'main;
        }
        if flags & SCAN_NOSKIP == 0 {
            while si < st.len() && matches!(st[si], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
                si += 1;
            }
            if si >= st.len() {
                underflow = true;
                break 'main;
            }
        }
        match op {
            b's' => {
                let mut w = if width == 0 { usize::MAX } else { width };
                let mut end = si;
                while end < st.len() {
                    if matches!(st[end], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
                        break;
                    }
                    end += 1;
                    w -= 1;
                    if w == 0 {
                        break;
                    }
                }
                if flags & SCAN_SUPPRESS == 0 {
                    set(&mut res, obj_index, from_bytes(&st[si..end]));
                    obj_index += 1;
                }
                si = end;
            }
            b'[' => {
                let mut w = if width == 0 { usize::MAX } else { width };
                let (cs, nf) = build_charset(f, fi);
                fi = nf;
                let mut end = si;
                while end < st.len() {
                    if !in_set(&cs, st[end]) {
                        break;
                    }
                    end += 1;
                    w -= 1;
                    if w == 0 {
                        break;
                    }
                }
                if si == end {
                    break 'main;
                }
                if flags & SCAN_SUPPRESS == 0 {
                    set(&mut res, obj_index, from_bytes(&st[si..end]));
                    obj_index += 1;
                }
                si = end;
            }
            b'i' => {
                let mut buf: Vec<u8> = Vec::new();
                let mut w = if width == 0 || width > 63 { 63 } else { width };
                flags |= SCAN_SIGNOK | SCAN_NODIGITS | SCAN_NOZERO;
                while w > 0 {
                    let c = at(st, si);
                    let mut add = false;
                    match c {
                        b'0' => {
                            if base == 16 {
                                flags |= SCAN_XOK;
                            }
                            if base == 0 {
                                base = 8;
                                flags |= SCAN_XOK;
                            }
                            if flags & SCAN_NOZERO != 0 {
                                flags &= !(SCAN_SIGNOK | SCAN_NODIGITS | SCAN_NOZERO);
                            } else {
                                flags &= !(SCAN_SIGNOK | SCAN_XOK | SCAN_NODIGITS);
                            }
                            add = true;
                        }
                        b'1'..=b'7' => {
                            if base == 0 {
                                base = 10;
                            }
                            flags &= !(SCAN_SIGNOK | SCAN_XOK | SCAN_NODIGITS);
                            add = true;
                        }
                        b'8' | b'9' => {
                            if base == 0 {
                                base = 10;
                            }
                            if base > 8 {
                                flags &= !(SCAN_SIGNOK | SCAN_XOK | SCAN_NODIGITS);
                                add = true;
                            }
                        }
                        b'A'..=b'F' | b'a'..=b'f' => {
                            if base > 10 {
                                flags &= !(SCAN_SIGNOK | SCAN_XOK | SCAN_NODIGITS);
                                add = true;
                            }
                        }
                        b'+' | b'-' => {
                            if flags & SCAN_SIGNOK != 0 {
                                flags &= !SCAN_SIGNOK;
                                add = true;
                            }
                        }
                        b'x' | b'X' => {
                            if flags & SCAN_XOK != 0 && buf.len() == 1 {
                                base = 16;
                                flags &= !SCAN_XOK;
                                add = true;
                            }
                        }
                        _ => {}
                    }
                    if !add {
                        break;
                    }
                    buf.push(c);
                    si += 1;
                    if si >= st.len() {
                        break;
                    }
                    w -= 1;
                }
                if flags & SCAN_NODIGITS != 0 {
                    if si >= st.len() {
                        underflow = true;
                    }
                    break 'main;
                } else if matches!(buf.last(), Some(b'x') | Some(b'X')) {
                    buf.pop();
                    si -= 1;
                }
                if flags & SCAN_SUPPRESS == 0 {
                    let value = strtol(&buf, base);
                    let v = if unsigned && value < 0 { vstr(&(value as u64).to_string()) } else { Value::Int(value) };
                    set(&mut res, obj_index, v);
                    obj_index += 1;
                }
                let _ = SCAN_UNSIGNED;
            }
            b'f' => {
                let mut buf: Vec<u8> = Vec::new();
                let mut w = if width == 0 || width > 63 { 63 } else { width };
                flags |= SCAN_SIGNOK | SCAN_NODIGITS | SCAN_PTOK | SCAN_EXPOK;
                while w > 0 {
                    let c = at(st, si);
                    let mut add = false;
                    match c {
                        b'0'..=b'9' => {
                            flags &= !(SCAN_SIGNOK | SCAN_NODIGITS);
                            add = true;
                        }
                        b'+' | b'-' => {
                            if flags & SCAN_SIGNOK != 0 {
                                flags &= !SCAN_SIGNOK;
                                add = true;
                            }
                        }
                        b'.' => {
                            if flags & SCAN_PTOK != 0 {
                                flags &= !(SCAN_SIGNOK | SCAN_PTOK);
                                add = true;
                            }
                        }
                        b'e' | b'E' => {
                            if flags & (SCAN_NODIGITS | SCAN_EXPOK) == SCAN_EXPOK {
                                flags = (flags & !(SCAN_EXPOK | SCAN_PTOK)) | SCAN_SIGNOK | SCAN_NODIGITS;
                                add = true;
                            }
                        }
                        _ => {}
                    }
                    if !add {
                        break;
                    }
                    buf.push(c);
                    si += 1;
                    if si >= st.len() {
                        break;
                    }
                    w -= 1;
                }
                if flags & SCAN_NODIGITS != 0 {
                    if flags & SCAN_EXPOK != 0 {
                        if si >= st.len() {
                            underflow = true;
                        }
                        break 'main;
                    }
                    let last = buf.pop();
                    si -= 1;
                    if !matches!(last, Some(b'e') | Some(b'E')) {
                        buf.pop();
                        si -= 1;
                    }
                }
                if flags & SCAN_SUPPRESS == 0 {
                    let d: f64 = std::str::from_utf8(&buf).ok().and_then(|x| x.parse().ok()).unwrap_or(0.0);
                    set(&mut res, obj_index, Value::Float(d));
                    obj_index += 1;
                }
            }
            _ => {}
        }
        nconv += 1;
    }
    if underflow && nconv == 0 {
        return Ok(if num_vars > 0 { Value::Int(-1) } else { Value::Null });
    }
    if num_vars > 0 {
        return Ok(Value::Int(nconv));
    }
    Ok(list(res))
}

pub fn register(m: &mut HashMap<&'static str, VmBuiltin>) {
    m.insert("sscanf", sscanf);
}
