//! Ports of url.c, base64.c, uuencode.c, quot_print.c, soundex.c, metaphone.c, levenshtein.c, strcoll.
use super::core1::ve;
use super::util::*;
use crate::builtins::VmBuiltin;
use crate::value::*;
use crate::vm::Vm;
use std::collections::HashMap;
use std::rc::Rc;

fn dep_fn(vm: &mut Vm<'_>, f: &str, since: &str, msg: &str) {
    deprecated(vm, &format!("Function {f}() is deprecated since {since}, {msg}"));
}

// ---------- url.c ----------
fn url_encode_helper(s: &[u8], raw: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for &c in s {
        if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'.' || (raw && c == b'~') {
            out.push(c as char);
        } else if !raw && c == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

fn url_decode_ex(s: &[u8], raw: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if !raw && c == b'+' {
            out.push(b' ');
        } else if c == b'%' && i + 2 < s.len() + 0 && s[i + 1].is_ascii_hexdigit() && s[i + 2].is_ascii_hexdigit() {
            out.push(((s[i + 1] as char).to_digit(16).unwrap() * 16 + (s[i + 2] as char).to_digit(16).unwrap()) as u8);
            i += 2;
        } else if c == b'%' && i + 2 == s.len() - 0 && false {
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

fn url_fn(vm: &mut Vm<'_>, f: &str, a: Vec<Value>, enc: bool, raw: bool) -> R<Value> {
    argc(vm, f, &a, 1, 1)?;
    let s = p_str(vm, f, &a, 0, "string")?;
    if enc {
        Ok(vstr(&url_encode_helper(s.as_bytes(), raw)))
    } else {
        Ok(from_bytes(&url_decode_ex(s.as_bytes(), raw)))
    }
}

#[derive(Default)]
struct Url {
    scheme: Option<Vec<u8>>,
    host: Option<Vec<u8>>,
    port: u16,
    user: Option<Vec<u8>>,
    pass: Option<Vec<u8>>,
    path: Option<Vec<u8>>,
    query: Option<Vec<u8>>,
    fragment: Option<Vec<u8>>,
    has_port: bool,
}

fn rcc(b: &[u8]) -> Vec<u8> {
    b.iter().map(|&c| if c < 32 || c == 127 { b'_' } else { c }).collect()
}

fn strcspn_pos(b: &[u8], from: usize, to: usize, set: &[u8]) -> usize {
    (from..to).find(|&i| set.contains(&b[i])).unwrap_or(to)
}

fn memrchr(b: &[u8], from: usize, to: usize, c: u8) -> Option<usize> {
    (from..to).rev().find(|&i| b[i] == c)
}

fn parse_port_str(b: &[u8]) -> Option<u16> {
    let s = std::str::from_utf8(b).ok()?;
    let n = s.parse::<i64>().ok()?;
    if (0..=65535).contains(&n) {
        Some(n as u16)
    } else {
        None
    }
}

#[derive(PartialEq)]
enum St {
    Start,
    ParsePort,
    ParseHost,
    JustPath,
}

/// php_url_parse_ex2
fn url_parse(b: &[u8]) -> Option<Url> {
    let mut ret = Url::default();
    let ue = b.len();
    let mut s = 0usize;
    let mut e: Option<usize> = b.iter().position(|&c| c == b':');
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let mut st = St::Start;
    if let Some(ee) = e {
        if ee != 0 {
            // validate scheme
            let mut p = s;
            let mut resolved = false;
            while p < ee {
                let c = b[p];
                if !c.is_ascii_alphanumeric() && c != b'+' && c != b'.' && c != b'-' {
                    if ee + 1 < ue && ee < strcspn_pos(b, s, ue, b"?#") {
                        st = St::ParsePort;
                    } else if s + 1 < ue && b[s] == b'/' && b[s + 1] == b'/' {
                        s += 2;
                        e = None;
                        st = St::ParseHost;
                    } else {
                        st = St::JustPath;
                    }
                    resolved = true;
                    break;
                }
                p += 1;
            }
            if !resolved {
                if ee + 1 == ue {
                    ret.scheme = Some(rcc(&b[s..ee]));
                    return Some(ret);
                }
                if at(ee + 1) != b'/' {
                    let mut p = ee + 1;
                    while p < ue && b[p].is_ascii_digit() {
                        p += 1;
                    }
                    if (p == ue || b[p] == b'/') && (p - ee) < 7 {
                        st = St::ParsePort;
                    } else {
                        ret.scheme = Some(rcc(&b[s..ee]));
                        s = ee + 1;
                        st = St::JustPath;
                    }
                } else {
                    ret.scheme = Some(rcc(&b[s..ee]));
                    if ee + 2 < ue && b[ee + 2] == b'/' {
                        s = ee + 3;
                        st = St::ParseHost;
                        if ret.scheme.as_ref().unwrap().eq_ignore_ascii_case(b"file") && ee + 3 < ue && b[ee + 3] == b'/' {
                            if ee + 5 < ue && b[ee + 5] == b':' {
                                s = ee + 4;
                            }
                            st = St::JustPath;
                        }
                    } else {
                        s = ee + 1;
                        st = St::JustPath;
                    }
                }
            }
        } else {
            st = St::ParsePort;
        }
    } else if s + 1 < ue && b[s] == b'/' && b[s + 1] == b'/' {
        s += 2;
        st = St::ParseHost;
    } else {
        st = St::JustPath;
    }

    if st == St::ParsePort {
        let ee = e.unwrap();
        let p = ee + 1;
        let mut pp = p;
        while pp < ue && pp - p < 6 && b[pp].is_ascii_digit() {
            pp += 1;
        }
        if pp - p > 0 && pp - p < 6 && (pp == ue || b[pp] == b'/') {
            match parse_port_str(&b[p..pp]) {
                Some(port) => {
                    ret.has_port = true;
                    ret.port = port;
                    if s + 1 < ue && b[s] == b'/' && b[s + 1] == b'/' {
                        s += 2;
                    }
                }
                None => return None,
            }
            st = St::ParseHost;
        } else if p == pp && pp == ue {
            return None;
        } else if s + 1 < ue && b[s] == b'/' && b[s + 1] == b'/' {
            s += 2;
            st = St::ParseHost;
        } else {
            st = St::JustPath;
        }
    }

    if st == St::ParseHost {
        let e2 = strcspn_pos(b, s, ue, b"/?#");
        if let Some(p) = memrchr(b, s, e2, b'@') {
            if let Some(pp) = (s..p).find(|&i| b[i] == b':') {
                ret.user = Some(rcc(&b[s..pp]));
                ret.pass = Some(rcc(&b[pp + 1..p]));
            } else {
                ret.user = Some(rcc(&b[s..p]));
            }
            s = p + 1;
        }
        let colon = if s < ue && b[s] == b'[' && e2 > 0 && b[e2 - 1] == b']' { None } else { memrchr(b, s, e2, b':') };
        let hend;
        if let Some(p) = colon {
            if ret.port == 0 {
                let p1 = p + 1;
                if e2 - p1 > 5 {
                    return None;
                } else if e2 > p1 {
                    match parse_port_str(&b[p1..e2]) {
                        Some(port) => {
                            ret.has_port = true;
                            ret.port = port;
                        }
                        None => return None,
                    }
                }
            }
            hend = p;
        } else {
            hend = e2;
        }
        if hend < s + 1 {
            return None;
        }
        ret.host = Some(rcc(&b[s..hend]));
        if e2 == ue {
            return Some(ret);
        }
        s = e2;
    }

    // just_path
    let mut e3 = ue;
    if let Some(p) = (s..e3).find(|&i| b[i] == b'#') {
        let p1 = p + 1;
        ret.fragment = Some(if p1 < e3 { rcc(&b[p1..e3]) } else { vec![] });
        e3 = p1 - 1;
    }
    if let Some(p) = (s..e3).find(|&i| b[i] == b'?') {
        let p1 = p + 1;
        ret.query = Some(if p1 < e3 { rcc(&b[p1..e3]) } else { vec![] });
        e3 = p1 - 1;
    }
    if s < e3 || s == ue {
        ret.path = Some(rcc(&b[s..e3]));
    }
    Some(ret)
}

fn parse_url(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "parse_url", &a, 1, 2)?;
    let s = p_str(vm, "parse_url", &a, 0, "url")?;
    let key = p_int_opt(vm, "parse_url", &a, 1, "component", -1)?;
    let Some(u) = url_parse(s.as_bytes()) else { return Ok(Value::Bool(false)) };
    let sv = |o: &Option<Vec<u8>>| o.as_ref().map(|b| from_bytes(b)).unwrap_or(Value::Null);
    if key > -1 {
        return Ok(match key {
            0 => sv(&u.scheme),
            1 => sv(&u.host),
            2 => {
                if u.has_port {
                    Value::Int(u.port as i64)
                } else {
                    Value::Null
                }
            }
            3 => sv(&u.user),
            4 => sv(&u.pass),
            5 => sv(&u.path),
            6 => sv(&u.query),
            7 => sv(&u.fragment),
            _ => return verr(vm, &format!("parse_url(): Argument #2 ($component) must be a valid URL component identifier, {key} given")),
        });
    }
    let mut r = PhpArray::new();
    let mut put = |k: &str, v: Value| {
        if !matches!(v, Value::Null) {
            r.set(Key::from_str(k), v)
        }
    };
    put("scheme", sv(&u.scheme));
    put("host", sv(&u.host));
    if u.has_port {
        put("port", Value::Int(u.port as i64));
    }
    put("user", sv(&u.user));
    put("pass", sv(&u.pass));
    put("path", sv(&u.path));
    put("query", sv(&u.query));
    put("fragment", sv(&u.fragment));
    Ok(Value::Array(Rc::new(r)))
}

// ---------- base64 ----------
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "base64_encode", &a, 1, 1)?;
    let s = p_str(vm, "base64_encode", &a, 0, "string")?;
    let mut out = String::new();
    for c in s.as_bytes().chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    Ok(vstr(&out))
}

fn b64_rev(c: u8) -> i32 {
    match c {
        b'A'..=b'Z' => (c - b'A') as i32,
        b'a'..=b'z' => (c - b'a') as i32 + 26,
        b'0'..=b'9' => (c - b'0') as i32 + 52,
        b'+' => 62,
        b'/' => 63,
        9 | 10 | 13 | b' ' => -1,
        _ => -2,
    }
}

fn base64_decode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "base64_decode", &a, 1, 2)?;
    let s = p_str(vm, "base64_decode", &a, 0, "string")?;
    let strict = p_bool(&a, 1, false);
    let mut out: Vec<u8> = Vec::new();
    let (mut i, mut padding) = (0usize, 0usize);
    for &c in s.as_bytes() {
        if c == b'=' {
            padding += 1;
            continue;
        }
        let ch = b64_rev(c);
        if !strict {
            if ch < 0 {
                continue;
            }
        } else {
            if ch == -1 {
                continue;
            }
            if ch == -2 || padding > 0 {
                return Ok(Value::Bool(false));
            }
        }
        let ch = ch as u8;
        match i % 4 {
            0 => out.push(ch << 2),
            1 => {
                *out.last_mut().unwrap() |= ch >> 4;
                out.push((ch & 0x0f) << 4);
            }
            2 => {
                *out.last_mut().unwrap() |= ch >> 2;
                out.push((ch & 0x03) << 6);
            }
            _ => *out.last_mut().unwrap() |= ch,
        }
        i += 1;
    }
    if strict && i % 4 == 1 {
        return Ok(Value::Bool(false));
    }
    if strict && padding > 0 && (padding > 2 || (i + padding) % 4 != 0) {
        return Ok(Value::Bool(false));
    }
    // drop the partially filled trailing byte
    if i % 4 != 0 {
        out.pop();
    } else if !out.is_empty() && i > 0 {
        // after a full group the pushed slot is only started when i%4 in 1,2; nothing pending
    }
    Ok(from_bytes(&out))
}

// ---------- uuencode ----------
fn uu_enc(c: u8) -> u8 {
    if c != 0 {
        (c & 0o77) + b' '
    } else {
        b'`'
    }
}
fn uu_dec(c: u8) -> u8 {
    c.wrapping_sub(b' ') & 0o77
}

fn php_uuencode(src: &[u8]) -> Vec<u8> {
    let g = |i: usize| src.get(i).copied().unwrap_or(0);
    let mut out = Vec::new();
    let mut len = 45usize;
    let mut s = 0usize;
    let e = src.len();
    let c2 = |s: usize| uu_enc(((g(s) << 4) & 0o60) | ((g(s + 1) >> 4) & 0o17));
    let c3 = |s: usize| uu_enc(((g(s + 1) << 2) & 0o74) | ((g(s + 2) >> 6) & 0o3));
    while s + 3 < e {
        let mut ee = s + len;
        if ee > e {
            ee = e;
            len = ee - s;
            if len % 3 != 0 {
                ee = s + (len / 3) * 3;
            }
        }
        out.push(uu_enc(len as u8));
        while s < ee {
            out.push(uu_enc(g(s) >> 2));
            out.push(c2(s));
            out.push(c3(s));
            out.push(uu_enc(g(s + 2) & 0o77));
            s += 3;
        }
        if len == 45 {
            out.push(b'\n');
        }
    }
    if s < e {
        if len == 45 {
            out.push(uu_enc((e - s) as u8));
            len = 0;
        }
        out.push(uu_enc(g(s) >> 2));
        out.push(c2(s));
        out.push(if e - s > 1 { c3(s) } else { uu_enc(0) });
        out.push(if e - s > 2 { uu_enc(g(s + 2) & 0o77) } else { uu_enc(0) });
    }
    if len < 45 {
        out.push(b'\n');
    }
    out.push(uu_enc(0));
    out.push(b'\n');
    out
}

fn php_uudecode(src: &[u8]) -> Option<Vec<u8>> {
    if src.is_empty() {
        return None;
    }
    let g = |i: usize| src.get(i).copied().unwrap_or(0);
    let mut out: Vec<u8> = Vec::new();
    let mut s = 0usize;
    let e = src.len();
    let mut total_len = 0usize;
    while s < e {
        let len = uu_dec(src[s]) as usize;
        s += 1;
        if len == 0 {
            break;
        }
        if len > e {
            return None;
        }
        total_len += len;
        let ee = s + if len == 45 { 60 } else { (len as f64 * 1.33).floor() as usize };
        if ee > e {
            return None;
        }
        while s < ee {
            if s + 4 > e {
                return None;
            }
            out.push(uu_dec(src[s]) << 2 | uu_dec(src[s + 1]) >> 4);
            out.push(uu_dec(src[s + 1]) << 4 | uu_dec(src[s + 2]) >> 2);
            out.push(uu_dec(src[s + 2]) << 6 | uu_dec(src[s + 3]));
            s += 4;
        }
        if len < 45 {
            break;
        }
        s += 1;
    }
    if total_len > out.len() {
        out.push(uu_dec(g(s)) << 2 | uu_dec(g(s + 1)) >> 4);
        if total_len > 1 {
            out.push(uu_dec(g(s + 1)) << 4 | uu_dec(g(s + 2)) >> 2);
            if total_len > 2 {
                out.push(uu_dec(g(s + 2)) << 6 | uu_dec(g(s + 3)));
            }
        }
    }
    out.truncate(total_len);
    Some(out)
}

// ---------- quoted printable ----------
fn qp_decode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "quoted_printable_decode", &a, 1, 1)?;
    let s = p_str(vm, "quoted_printable_decode", &a, 0, "string")?;
    let b = s.as_bytes();
    let g = |i: usize| b.get(i).copied().unwrap_or(0);
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() && b[i] != 0 {
        if b[i] == b'=' {
            if g(i + 1) != 0 && g(i + 2) != 0 && g(i + 1).is_ascii_hexdigit() && g(i + 2).is_ascii_hexdigit() {
                out.push(((g(i + 1) as char).to_digit(16).unwrap() * 16 + (g(i + 2) as char).to_digit(16).unwrap()) as u8);
                i += 3;
            } else {
                let mut k = 1;
                while g(i + k) != 0 && (g(i + k) == 32 || g(i + k) == 9) {
                    k += 1;
                }
                if g(i + k) == 0 {
                    i += k;
                } else if g(i + k) == 13 && g(i + k + 1) == 10 {
                    i += k + 2;
                } else if g(i + k) == 13 || g(i + k) == 10 {
                    i += k + 1;
                } else {
                    out.push(b[i]);
                    i += 1;
                }
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Ok(from_bytes(&out))
}

fn qp_encode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "quoted_printable_encode", &a, 1, 1)?;
    let s = p_str(vm, "quoted_printable_encode", &a, 0, "string")?;
    if s.is_empty() {
        return Ok(vstr(""));
    }
    const MAXL: u64 = 75;
    const HEX: &[u8] = b"0123456789ABCDEF";
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut lp: u64 = 0;
    let mut i = 0;
    let mut length = b.len();
    let nxt = |i: usize| b.get(i).copied().unwrap_or(0);
    while length > 0 {
        length -= 1;
        let c = b[i];
        i += 1;
        if c == 13 && nxt(i) == 10 && length > 0 {
            out.push(13);
            out.push(nxt(i));
            i += 1;
            length -= 1;
            lp = 0;
        } else if c < 32 || c == 0x7f || c & 0x80 != 0 || c == b'=' || (c == b' ' && nxt(i) == 13) {
            lp += 3;
            if (lp > MAXL && c <= 0x7f) || (c > 0x7f && c <= 0xdf && lp + 3 > MAXL) || (c > 0xdf && c <= 0xef && lp + 6 > MAXL) || (c > 0xef && c <= 0xf4 && lp + 9 > MAXL) {
                out.extend_from_slice(b"=\r\n");
                lp = 3;
            }
            out.push(b'=');
            out.push(HEX[(c >> 4) as usize]);
            out.push(HEX[(c & 0xf) as usize]);
        } else {
            lp += 1;
            if lp > MAXL {
                out.extend_from_slice(b"=\r\n");
                lp = 1;
            }
            out.push(c);
        }
    }
    Ok(from_bytes(&out))
}

// ---------- soundex / metaphone / levenshtein ----------
fn soundex(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "soundex", &a, 1, 1)?;
    let s = p_str(vm, "soundex", &a, 0, "string")?;
    const TBL: &[u8; 26] = b"01230120022455012623010202";
    let mut out: Vec<u8> = Vec::new();
    let mut last: i32 = -1;
    for &ch in s.as_bytes() {
        if out.len() >= 4 {
            break;
        }
        let code = ch.to_ascii_uppercase();
        if code.is_ascii_uppercase() {
            if out.is_empty() {
                out.push(code);
                last = TBL[(code - b'A') as usize] as i32;
            } else {
                let c = TBL[(code - b'A') as usize] as i32;
                if c != last {
                    if c != b'0' as i32 {
                        out.push(c as u8);
                    }
                    last = c;
                }
            }
        }
    }
    while out.len() < 4 {
        out.push(b'0');
    }
    Ok(from_bytes(&out))
}

const CODES: [u8; 26] = [1, 16, 4, 16, 9, 2, 4, 16, 9, 2, 0, 2, 2, 2, 1, 4, 0, 2, 4, 4, 1, 0, 0, 0, 8, 0];
fn enc(c: u8) -> u8 {
    if c.is_ascii_alphabetic() {
        CODES[(c.to_ascii_uppercase() - b'A') as usize]
    } else {
        0
    }
}
fn isvowel(c: u8) -> bool {
    enc(c) & 1 != 0
}
fn affecth(c: u8) -> bool {
    enc(c) & 4 != 0
}
fn makesoft(c: u8) -> bool {
    enc(c) & 8 != 0
}
fn noghtof(c: u8) -> bool {
    enc(c) & 16 != 0
}

fn metaphone_impl(word: &[u8], max_phonemes: usize) -> Vec<u8> {
    let w = |i: usize| word.get(i).copied().unwrap_or(0);
    let up = |c: u8| c.to_ascii_uppercase();
    let mut out: Vec<u8> = Vec::new();
    let mut idx = 0usize;
    loop {
        let c = w(idx);
        if c.is_ascii_alphabetic() {
            break;
        }
        if c == 0 {
            return out;
        }
        idx += 1;
    }
    let cur = up(w(idx));
    let next = |i: usize| up(w(i + 1));
    match cur {
        b'A' => {
            if next(idx) == b'E' {
                out.push(b'E');
                idx += 2;
            } else {
                out.push(b'A');
                idx += 1;
            }
        }
        b'G' | b'K' | b'P' => {
            if next(idx) == b'N' {
                out.push(b'N');
                idx += 2;
            }
        }
        b'W' => {
            let n = next(idx);
            if n == b'R' {
                out.push(b'R');
                idx += 2;
            } else if n == b'H' || isvowel(n) {
                out.push(b'W');
                idx += 2;
            }
        }
        b'X' => {
            out.push(b'S');
            idx += 1;
        }
        b'E' | b'I' | b'O' | b'U' => {
            out.push(cur);
            idx += 1;
        }
        _ => {}
    }
    while w(idx) != 0 && (max_phonemes == 0 || out.len() < max_phonemes) {
        let mut skip = 0usize;
        let raw = w(idx);
        if !raw.is_ascii_alphabetic() {
            idx += 1;
            continue;
        }
        let cur = up(raw);
        let back = |n: usize| if idx >= n { up(w(idx - n)) } else { 0 };
        let prev = back(1);
        if cur == prev && cur != b'C' {
            idx += 1;
            continue;
        }
        let after_next = if w(idx + 1) != 0 { up(w(idx + 2)) } else { 0 };
        let look_ahead = |n: usize| {
            let mut k = 0;
            while w(idx + k) != 0 && k < n {
                k += 1;
            }
            up(w(idx + k))
        };
        let nx = next(idx);
        match cur {
            b'B' => {
                if prev != b'M' {
                    out.push(b'B');
                }
            }
            b'C' => {
                if makesoft(nx) {
                    if nx == b'I' && after_next == b'A' {
                        out.push(b'X');
                    } else if prev == b'S' {
                    } else {
                        out.push(b'S');
                    }
                } else if nx == b'H' {
                    out.push(b'X');
                    skip += 1;
                } else {
                    out.push(b'K');
                }
            }
            b'D' => {
                if nx == b'G' && makesoft(after_next) {
                    out.push(b'J');
                    skip += 1;
                } else {
                    out.push(b'T');
                }
            }
            b'G' => {
                if nx == b'H' {
                    if !(noghtof(back(3)) || back(4) == b'H') {
                        out.push(b'F');
                        skip += 1;
                    }
                } else if nx == b'N' {
                    let an = after_next;
                    if !an.is_ascii_alphabetic() || (an == b'E' && look_ahead(3) == b'D') {
                    } else {
                        out.push(b'K');
                    }
                } else if makesoft(nx) && prev != b'G' {
                    out.push(b'J');
                } else {
                    out.push(b'K');
                }
            }
            b'H' => {
                if isvowel(nx) && !affecth(prev) {
                    out.push(b'H');
                }
            }
            b'K' => {
                if prev != b'C' {
                    out.push(b'K');
                }
            }
            b'P' => out.push(if nx == b'H' { b'F' } else { b'P' }),
            b'Q' => out.push(b'K'),
            b'S' => {
                if nx == b'I' && (after_next == b'O' || after_next == b'A') {
                    out.push(b'X');
                } else if nx == b'H' {
                    out.push(b'X');
                    skip += 1;
                } else {
                    out.push(b'S');
                }
            }
            b'T' => {
                if nx == b'I' && (after_next == b'O' || after_next == b'A') {
                    out.push(b'X');
                } else if nx == b'H' {
                    out.push(b'0');
                    skip += 1;
                } else if !(nx == b'C' && after_next == b'H') {
                    out.push(b'T');
                }
            }
            b'V' => out.push(b'F'),
            b'W' => {
                if isvowel(nx) {
                    out.push(b'W');
                }
            }
            b'X' => {
                out.push(b'K');
                out.push(b'S');
            }
            b'Y' => {
                if isvowel(nx) {
                    out.push(b'Y');
                }
            }
            b'Z' => out.push(b'S'),
            b'F' | b'J' | b'L' | b'M' | b'N' | b'R' => out.push(cur),
            _ => {}
        }
        idx += 1 + skip;
    }
    out
}

fn metaphone(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "metaphone", &a, 1, 2)?;
    dep_fn(vm, "metaphone", "8.6", "use a userland phonetic matching library instead");
    let s = p_str(vm, "metaphone", &a, 0, "string")?;
    let ph = p_int_opt(vm, "metaphone", &a, 1, "max_phonemes", 0)?;
    if ph < 0 {
        return ve(vm, "metaphone", 2, "max_phonemes", "must be greater than or equal to 0");
    }
    Ok(from_bytes(&metaphone_impl(s.as_bytes(), ph as usize)))
}

fn levenshtein(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "levenshtein", &a, 2, 5)?;
    let s1 = p_str(vm, "levenshtein", &a, 0, "string1")?;
    let s2 = p_str(vm, "levenshtein", &a, 1, "string2")?;
    let ci = p_int_opt(vm, "levenshtein", &a, 2, "insertion_cost", 1)?;
    let cr = p_int_opt(vm, "levenshtein", &a, 3, "replacement_cost", 1)?;
    let cd = p_int_opt(vm, "levenshtein", &a, 4, "deletion_cost", 1)?;
    let (mut x, mut y) = (s1.as_bytes(), s2.as_bytes());
    if x.is_empty() {
        return Ok(Value::Int(y.len() as i64 * ci));
    }
    if y.is_empty() {
        return Ok(Value::Int(x.len() as i64 * cd));
    }
    if x.len() < y.len() && ci == cr && cr == cd {
        std::mem::swap(&mut x, &mut y);
    }
    let mut p1: Vec<i64> = (0..=y.len() as i64).map(|i| i * ci).collect();
    let mut p2 = vec![0i64; y.len() + 1];
    for i1 in 0..x.len() {
        p2[0] = p1[0] + cd;
        for i2 in 0..y.len() {
            let mut c0 = p1[i2] + if x[i1] == y[i2] { 0 } else { cr };
            let c1 = p1[i2 + 1] + cd;
            if c1 < c0 {
                c0 = c1;
            }
            let c2 = p2[i2] + ci;
            if c2 < c0 {
                c0 = c2;
            }
            p2[i2 + 1] = c0;
        }
        std::mem::swap(&mut p1, &mut p2);
    }
    Ok(Value::Int(p1[y.len()]))
}

fn strcoll(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "strcoll", &a, 2, 2)?;
    dep_fn(vm, "strcoll", "8.6", "use Collator::compare() instead");
    let x = p_str(vm, "strcoll", &a, 0, "string1")?;
    let y = p_str(vm, "strcoll", &a, 1, "string2")?;
    Ok(Value::Int(match x.as_bytes().cmp(y.as_bytes()) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        _ => 1,
    }))
}

fn convert_uuencode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "convert_uuencode", &a, 1, 1)?;
    let s = p_str(vm, "convert_uuencode", &a, 0, "string")?;
    Ok(from_bytes(&php_uuencode(s.as_bytes())))
}
fn convert_uudecode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "convert_uudecode", &a, 1, 1)?;
    let s = p_str(vm, "convert_uudecode", &a, 0, "string")?;
    match php_uudecode(s.as_bytes()) {
        Some(b) => Ok(from_bytes(&b)),
        None => {
            warn(vm, "convert_uudecode(): Argument #1 ($string) is not a valid uuencoded string");
            Ok(Value::Bool(false))
        }
    }
}

pub fn register(m: &mut HashMap<&'static str, VmBuiltin>) {
    m.insert("urlencode", |vm, a| url_fn(vm, "urlencode", a, true, false));
    m.insert("urldecode", |vm, a| url_fn(vm, "urldecode", a, false, false));
    m.insert("rawurlencode", |vm, a| url_fn(vm, "rawurlencode", a, true, true));
    m.insert("rawurldecode", |vm, a| url_fn(vm, "rawurldecode", a, false, true));
    m.insert("parse_url", parse_url);
    m.insert("base64_encode", base64_encode);
    m.insert("base64_decode", base64_decode);
    m.insert("convert_uuencode", convert_uuencode);
    m.insert("convert_uudecode", convert_uudecode);
    m.insert("quoted_printable_decode", qp_decode);
    m.insert("quoted_printable_encode", qp_encode);
    m.insert("soundex", soundex);
    m.insert("metaphone", metaphone);
    m.insert("levenshtein", levenshtein);
    m.insert("strcoll", strcoll);
}
