//! Port of ext/standard/string.c, part 2: slashes, strtr, pad, split, count_chars, misc.
use super::core1::{charmask, ve};
use super::util::*;
use crate::builtins::VmBuiltin;
use crate::value::*;
use crate::vm::Vm;
use std::collections::HashMap;

fn zend_dirname(p: &[u8]) -> Vec<u8> {
    if p.is_empty() {
        return vec![];
    }
    let mut end = p.len() as i64 - 1;
    while end >= 0 && p[end as usize] == b'/' {
        end -= 1;
    }
    if end < 0 {
        return b"/".to_vec();
    }
    while end >= 0 && p[end as usize] != b'/' {
        end -= 1;
    }
    if end < 0 {
        return b".".to_vec();
    }
    while end >= 0 && p[end as usize] == b'/' {
        end -= 1;
    }
    if end < 0 {
        return b"/".to_vec();
    }
    p[..=end as usize].to_vec()
}

fn dirname(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "dirname", &a, 1, 2)?;
    let s = p_str(vm, "dirname", &a, 0, "path")?;
    let mut levels = p_int_opt(vm, "dirname", &a, 1, "levels", 1)?;
    let mut ret = s.as_bytes().to_vec();
    if levels == 1 {
        ret = zend_dirname(&ret);
    } else if levels < 1 {
        return ve(vm, "dirname", 2, "levels", "must be greater than or equal to 1");
    } else {
        loop {
            let before = ret.len();
            ret = zend_dirname(&ret);
            levels -= 1;
            if !(ret.len() < before && levels != 0) {
                break;
            }
        }
    }
    Ok(from_bytes(&ret))
}

fn basename(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "basename", &a, 1, 2)?;
    let s = p_str(vm, "basename", &a, 0, "path")?;
    let suffix = p_str_opt(vm, "basename", &a, 1, "suffix", "")?;
    let b = s.as_bytes();
    let mut end = b.len() as i64 - 1;
    while end >= 0 && b[end as usize] == b'/' {
        end -= 1;
    }
    if end < 0 {
        return Ok(vstr(""));
    }
    let mut start = end;
    let mut end = end + 1;
    while start > 0 && b[start as usize - 1] != b'/' {
        start -= 1;
    }
    let sl = suffix.len() as i64;
    if sl > 0 && sl < end - start && &b[(end - sl) as usize..end as usize] == suffix.as_bytes() {
        end -= sl;
    }
    Ok(from_bytes(&b[start as usize..end as usize]))
}

fn chunk_split(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "chunk_split", &a, 1, 3)?;
    let s = p_str(vm, "chunk_split", &a, 0, "string")?;
    let n = p_int_opt(vm, "chunk_split", &a, 1, "length", 76)?;
    let end = p_str_opt(vm, "chunk_split", &a, 2, "separator", "\r\n")?;
    if n <= 0 {
        return ve(vm, "chunk_split", 2, "length", "must be greater than 0");
    }
    let b = s.as_bytes();
    if n as usize > b.len() {
        return Ok(vstr(&format!("{s}{end}")));
    }
    let mut out = Vec::new();
    for c in b.chunks(n as usize) {
        out.extend_from_slice(c);
        out.extend_from_slice(end.as_bytes());
    }
    Ok(from_bytes(&out))
}

fn norm_from(f: i64, len: i64) -> i64 {
    if f < 0 {
        (len + f).max(0)
    } else {
        f.min(len)
    }
}

fn substr_replace(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "substr_replace", &a, 3, 4)?;
    let strv = a[0].deref();
    let repl = a[1].deref();
    let from = a[2].deref();
    let lenv = a.get(3).map(|v| v.deref()).unwrap_or(Value::Null);
    let tstr = |vm: &mut Vm<'_>, i: usize, p: &str, v: &Value| -> R<Option<std::rc::Rc<str>>> {
        match v {
            Value::Array(_) => Ok(None),
            _ => Ok(Some(p_str(vm, "substr_replace", &a, i, p)?)),
        }
    };
    let s_single = tstr(vm, 0, "string", &strv)?;
    let r_single = tstr(vm, 1, "replace", &repl)?;
    let from_arr = if let Value::Array(x) = &from { Some(x.clone()) } else { None };
    let len_arr = if let Value::Array(x) = &lenv { Some(x.clone()) } else { None };
    let from_long = if from_arr.is_none() { p_int(vm, "substr_replace", &a, 2, "offset")? } else { 0 };
    let len_null = matches!(lenv, Value::Null);
    let len_long = if !len_null && len_arr.is_none() { p_int(vm, "substr_replace", &a, 3, "length")? } else { 0 };
    let repl_arr = if let Value::Array(x) = &repl { Some(x.clone()) } else { None };

    if let Some(s) = s_single {
        if from_arr.is_some() {
            return vm.throw_error("TypeError", "substr_replace(): Argument #3 ($offset) cannot be an array when working on a single string");
        }
        if len_arr.is_some() {
            return vm.throw_error("TypeError", "substr_replace(): Argument #4 ($length) cannot be an array when working on a single string");
        }
        let n = s.len() as i64;
        let f = norm_from(from_long, n);
        let mut l = if len_null { n } else { len_long };
        if l < 0 {
            l = ((n - f) + l).max(0);
        }
        if l > n {
            l = n;
        }
        if f + l > n {
            l = n - f;
        }
        let rs: String = match &repl_arr {
            Some(r) => r.iter().next().map(|(_, v)| v.to_str().to_string()).unwrap_or_default(),
            None => r_single.unwrap().to_string(),
        };
        let b = s.as_bytes();
        let mut out = b[..f as usize].to_vec();
        out.extend_from_slice(rs.as_bytes());
        out.extend_from_slice(&b[(f + l) as usize..]);
        return Ok(from_bytes(&out));
    }
    let Value::Array(sarr) = &strv else { unreachable!() };
    let mut res = PhpArray::new();
    let mut from_it = from_arr.as_ref().map(|x| x.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>().into_iter());
    let mut len_it = len_arr.as_ref().map(|x| x.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>().into_iter());
    let mut repl_it = repl_arr.as_ref().map(|x| x.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>().into_iter());
    for (k, v) in sarr.iter() {
        let orig = v.deref().to_str();
        let n = orig.len() as i64;
        let f = match &mut from_it {
            Some(it) => match it.next() {
                Some(x) => norm_from(x.to_int(), n),
                None => 0,
            },
            None => norm_from(from_long, n),
        };
        let mut l = match &mut len_it {
            Some(it) => match it.next() {
                Some(x) => x.to_int(),
                None => n,
            },
            None => {
                if len_null {
                    n
                } else {
                    len_long
                }
            }
        };
        if l < 0 {
            l = ((n - f) + l).max(0);
        }
        if f + l > n {
            l = n - f;
        }
        let rs: String = match &mut repl_it {
            Some(it) => it.next().map(|x| x.to_str().to_string()).unwrap_or_default(),
            None => r_single.clone().unwrap().to_string(),
        };
        let b = orig.as_bytes();
        let mut out = b[..f as usize].to_vec();
        out.extend_from_slice(rs.as_bytes());
        out.extend_from_slice(&b[(f + l) as usize..]);
        res.set(k.clone(), from_bytes(&out));
    }
    Ok(Value::Array(std::rc::Rc::new(res)))
}

fn quotemeta(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "quotemeta", &a, 1, 1)?;
    let s = p_str(vm, "quotemeta", &a, 0, "string")?;
    if s.is_empty() {
        return Ok(vstr(""));
    }
    let mut out = Vec::new();
    for c in s.bytes() {
        if matches!(c, b'.' | b'\\' | b'+' | b'*' | b'?' | b'[' | b'^' | b']' | b'$' | b'(' | b')') {
            out.push(b'\\');
        }
        out.push(c);
    }
    Ok(from_bytes(&out))
}

fn ord(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "ord", &a, 1, 1)?;
    let s = p_str(vm, "ord", &a, 0, "character")?;
    if s.len() != 1 {
        if s.is_empty() {
            deprecated(vm, "ord(): Providing an empty string is deprecated");
        } else {
            deprecated(vm, "ord(): Providing a string that is not one byte long is deprecated. Use ord($str[0]) instead");
        }
    }
    Ok(Value::Int(s.as_bytes().first().copied().unwrap_or(0) as i64))
}

fn chr(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "chr", &a, 1, 1)?;
    let c = p_int(vm, "chr", &a, 0, "codepoint")?;
    if !(0..=255).contains(&c) {
        deprecated(vm, "chr(): Providing a value not in-between 0 and 255 is deprecated, this is because a byte value must be in the [0, 255] interval. The value used will be constrained using % 256");
    }
    let b = (c & 0xff) as u8;
    // The engine has no binary strings: bytes >= 0x80 map to the Latin-1 code point.
    Ok(if b < 0x80 { from_bytes(&[b]) } else { vstr(&(b as char).to_string()) })
}

fn strrev(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "strrev", &a, 1, 1)?;
    let s = p_str(vm, "strrev", &a, 0, "string")?;
    let mut b = s.as_bytes().to_vec();
    b.reverse();
    Ok(from_bytes(&b))
}

fn addcslashes(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "addcslashes", &a, 2, 2)?;
    let s = p_str(vm, "addcslashes", &a, 0, "string")?;
    let what = p_str(vm, "addcslashes", &a, 1, "characters")?;
    if s.is_empty() || what.is_empty() {
        return Ok(Value::Str(s));
    }
    let (flags, _) = charmask(vm, "addcslashes", what.as_bytes());
    let mut out = Vec::new();
    for c in s.bytes() {
        if flags[c as usize] {
            if !(32..=126).contains(&c) {
                out.push(b'\\');
                match c {
                    b'\n' => out.push(b'n'),
                    b'\t' => out.push(b't'),
                    b'\r' => out.push(b'r'),
                    7 => out.push(b'a'),
                    0x0b => out.push(b'v'),
                    8 => out.push(b'b'),
                    0x0c => out.push(b'f'),
                    _ => {
                        out.push((c >> 6) + b'0');
                        out.push(((c >> 3) & 7) + b'0');
                        out.push((c & 7) + b'0');
                    }
                }
                continue;
            }
            out.push(b'\\');
        }
        out.push(c);
    }
    Ok(from_bytes(&out))
}

fn addslashes(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "addslashes", &a, 1, 1)?;
    let s = p_str(vm, "addslashes", &a, 0, "string")?;
    let mut out = Vec::new();
    for c in s.bytes() {
        match c {
            0 => out.extend_from_slice(b"\\0"),
            b'\'' | b'"' | b'\\' => {
                out.push(b'\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    Ok(from_bytes(&out))
}

fn stripcslashes(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "stripcslashes", &a, 1, 1)?;
    let s = p_str(vm, "stripcslashes", &a, 0, "string")?;
    let b = s.as_bytes();
    let end = b.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < end {
        if b[i] == b'\\' && i + 1 < end {
            i += 1;
            match b[i] {
                b'n' => out.push(b'\n'),
                b'r' => out.push(b'\r'),
                b'a' => out.push(7),
                b't' => out.push(b'\t'),
                b'v' => out.push(0x0b),
                b'b' => out.push(8),
                b'f' => out.push(0x0c),
                b'\\' => out.push(b'\\'),
                b'x' if i + 1 < end && b[i + 1].is_ascii_hexdigit() => {
                    i += 1;
                    let mut v = (b[i] as char).to_digit(16).unwrap();
                    if i + 1 < end && b[i + 1].is_ascii_hexdigit() {
                        i += 1;
                        v = v * 16 + (b[i] as char).to_digit(16).unwrap();
                    }
                    out.push(v as u8);
                }
                _ => {
                    let mut cnt = 0;
                    let mut v: u32 = 0;
                    while i < end && (b'0'..=b'7').contains(&b[i]) && cnt < 3 {
                        v = v * 8 + (b[i] - b'0') as u32;
                        i += 1;
                        cnt += 1;
                    }
                    if cnt > 0 {
                        out.push(v as u8);
                        i -= 1;
                    } else {
                        out.push(b[i]);
                    }
                }
            }
        } else {
            out.push(b[i]);
        }
        i += 1;
    }
    Ok(from_bytes(&out))
}

fn stripslashes(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "stripslashes", &a, 1, 1)?;
    let s = p_str(vm, "stripslashes", &a, 0, "string")?;
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            i += 1;
            if i < b.len() {
                out.push(if b[i] == b'0' { 0 } else { b[i] });
            }
        } else {
            out.push(b[i]);
        }
        i += 1;
    }
    Ok(from_bytes(&out))
}

fn nl2br(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "nl2br", &a, 1, 2)?;
    let s = p_str(vm, "nl2br", &a, 0, "string")?;
    let xhtml = p_bool(&a, 1, true);
    if s.is_empty() {
        return Ok(vstr(""));
    }
    let tag: &[u8] = if xhtml { b"<br />" } else { b"<br>" };
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' || b[i] == b'\n' {
            out.extend_from_slice(tag);
            out.push(b[i]);
            if i + 1 < b.len() && (b[i + 1] == b'\r' || b[i + 1] == b'\n') && b[i + 1] != b[i] {
                i += 1;
                out.push(b[i]);
            }
        } else {
            out.push(b[i]);
        }
        i += 1;
    }
    Ok(from_bytes(&out))
}

fn str_repeat(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "str_repeat", &a, 2, 2)?;
    let s = p_str(vm, "str_repeat", &a, 0, "string")?;
    let n = p_int(vm, "str_repeat", &a, 1, "times")?;
    if n < 0 {
        return ve(vm, "str_repeat", 2, "times", "must be greater than or equal to 0");
    }
    Ok(vstr(&s.repeat(n as usize)))
}

fn count_chars(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "count_chars", &a, 1, 2)?;
    let s = p_str(vm, "count_chars", &a, 0, "string")?;
    let mode = p_int_opt(vm, "count_chars", &a, 1, "mode", 0)?;
    if !(0..=4).contains(&mode) {
        return ve(vm, "count_chars", 2, "mode", "must be between 0 and 4 (inclusive)");
    }
    let mut chars = [0i64; 256];
    for c in s.bytes() {
        chars[c as usize] += 1;
    }
    if mode < 3 {
        let mut r = PhpArray::new();
        for (i, c) in chars.iter().enumerate() {
            if mode == 0 || (mode == 1 && *c != 0) || (mode == 2 && *c == 0) {
                r.set(Key::Int(i as i64), Value::Int(*c));
            }
        }
        return Ok(Value::Array(std::rc::Rc::new(r)));
    }
    let b: Vec<u8> = (0..256).filter(|i| (chars[*i] != 0) == (mode == 3)).map(|i| i as u8).collect();
    Ok(from_bytes(&b))
}

fn str_pad(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "str_pad", &a, 2, 4)?;
    let s = p_str(vm, "str_pad", &a, 0, "string")?;
    let len = p_int(vm, "str_pad", &a, 1, "length")?;
    let pad = p_str_opt(vm, "str_pad", &a, 2, "pad_string", " ")?;
    let ty = p_int_opt(vm, "str_pad", &a, 3, "pad_type", 1)?;
    if len < 0 || len as usize <= s.len() {
        return Ok(Value::Str(s));
    }
    if pad.is_empty() {
        return ve(vm, "str_pad", 3, "pad_string", "must be a non-empty string");
    }
    if !(0..=2).contains(&ty) {
        return ve(vm, "str_pad", 4, "pad_type", "must be STR_PAD_LEFT, STR_PAD_RIGHT, or STR_PAD_BOTH");
    }
    let total = len as usize - s.len();
    let (left, right) = match ty {
        0 => (total, 0),
        2 => (total / 2, total - total / 2),
        _ => (0, total),
    };
    let fill = |n: usize| -> Vec<u8> { (0..n).map(|i| pad.as_bytes()[i % pad.len()]).collect() };
    let mut out = fill(left);
    out.extend_from_slice(s.as_bytes());
    out.extend(fill(right));
    Ok(from_bytes(&out))
}

fn str_rot13(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "str_rot13", &a, 1, 1)?;
    let s = p_str(vm, "str_rot13", &a, 0, "string")?;
    let b: Vec<u8> = s
        .bytes()
        .map(|c| match c {
            b'a'..=b'z' => (c - b'a' + 13) % 26 + b'a',
            b'A'..=b'Z' => (c - b'A' + 13) % 26 + b'A',
            _ => c,
        })
        .collect();
    Ok(from_bytes(&b))
}

thread_local! { static RNG: std::cell::Cell<u64> = const { std::cell::Cell::new(0x9E3779B97F4A7C15) }; }
fn rnd(max: usize) -> usize {
    RNG.with(|r| {
        let mut x = r.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        r.set(x);
        (x % (max as u64 + 1)) as usize
    })
}

fn str_shuffle(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "str_shuffle", &a, 1, 1)?;
    let s = p_str(vm, "str_shuffle", &a, 0, "string")?;
    let mut b = s.as_bytes().to_vec();
    let mut n_left = b.len();
    while n_left > 1 {
        n_left -= 1;
        let j = rnd(n_left);
        b.swap(n_left, j);
    }
    Ok(from_bytes(&b))
}

fn str_word_count(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "str_word_count", &a, 1, 3)?;
    let s = p_str(vm, "str_word_count", &a, 0, "string")?;
    let ty = p_int_opt(vm, "str_word_count", &a, 1, "format", 0)?;
    let cl = match a.get(2).map(|v| v.deref()) {
        None | Some(Value::Null) => None,
        _ => Some(p_str(vm, "str_word_count", &a, 2, "characters")?),
    };
    let mut res = PhpArray::new();
    match ty {
        1 | 2 => {
            if s.is_empty() {
                return Ok(Value::Array(std::rc::Rc::new(res)));
            }
        }
        0 => {
            if s.is_empty() {
                return Ok(Value::Int(0));
            }
        }
        _ => return ve(vm, "str_word_count", 2, "format", "must be a valid format value"),
    }
    let ch = cl.as_ref().map(|c| charmask(vm, "str_word_count", c.as_bytes()).0);
    let b = s.as_bytes();
    let mut p = 0usize;
    let mut e = b.len();
    let allowed = |c: u8| ch.as_ref().is_some_and(|m| m[c as usize]);
    if (b[0] == b'\'' && !allowed(b'\'')) || (b[0] == b'-' && !allowed(b'-')) {
        p += 1;
    }
    if b[e - 1] == b'-' && !allowed(b'-') {
        e -= 1;
    }
    let mut count = 0;
    while p < e {
        let st = p;
        while p < e && (b[p].is_ascii_alphabetic() || allowed(b[p]) || b[p] == b'\'' || b[p] == b'-') {
            p += 1;
        }
        if p > st {
            match ty {
                1 => res.push(from_bytes(&b[st..p])),
                2 => res.set(Key::Int(st as i64), from_bytes(&b[st..p])),
                _ => count += 1,
            }
        }
        p += 1;
    }
    Ok(if ty == 0 { Value::Int(count) } else { Value::Array(std::rc::Rc::new(res)) })
}

fn str_split(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "str_split", &a, 1, 2)?;
    let s = p_str(vm, "str_split", &a, 0, "string")?;
    let n = p_int_opt(vm, "str_split", &a, 1, "length", 1)?;
    if n <= 0 {
        return ve(vm, "str_split", 2, "length", "must be greater than 0");
    }
    if s.is_empty() {
        return Ok(list(vec![]));
    }
    if n as usize >= s.len() {
        return Ok(list(vec![Value::Str(s)]));
    }
    Ok(list(s.as_bytes().chunks(n as usize).map(from_bytes).collect()))
}

// ---- strtr ----
fn strtr(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "strtr", &a, 2, 3)?;
    let s = p_str(vm, "strtr", &a, 0, "string")?;
    if a.len() == 2 {
        let Value::Array(pats) = a[1].deref() else {
            return vm.throw_error("TypeError", "strtr(): Argument #2 ($from) must be of type array, string given");
        };
        if pats.is_empty() || s.is_empty() {
            return Ok(Value::Str(s));
        }
        let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for (k, v) in pats.iter() {
            let key = match k {
                Key::Int(i) => i.to_string().into_bytes(),
                Key::Str(x) => x.as_bytes().to_vec(),
            };
            if key.is_empty() {
                warn(vm, "strtr(): Ignoring replacement of empty string");
                continue;
            }
            pairs.push((key, v.deref().to_str().as_bytes().to_vec()));
        }
        if pairs.is_empty() {
            return Ok(Value::Str(s));
        }
        let maxlen = pairs.iter().map(|p| p.0.len()).max().unwrap();
        let map: HashMap<&[u8], &[u8]> = pairs.iter().map(|(k, v)| (&k[..], &v[..])).collect();
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        'outer: while i < b.len() {
            let mut l = maxlen.min(b.len() - i);
            while l > 0 {
                if let Some(r) = map.get(&b[i..i + l]) {
                    out.extend_from_slice(r);
                    i += l;
                    continue 'outer;
                }
                l -= 1;
            }
            out.push(b[i]);
            i += 1;
        }
        return Ok(from_bytes(&out));
    }
    let from = p_str(vm, "strtr", &a, 1, "from")?;
    let to = p_str(vm, "strtr", &a, 2, "to")?;
    let n = from.len().min(to.len());
    if s.is_empty() || n == 0 {
        return Ok(Value::Str(s));
    }
    let mut table: Vec<u8> = (0..=255).collect();
    for i in 0..n {
        table[from.as_bytes()[i] as usize] = to.as_bytes()[i];
    }
    let b: Vec<u8> = s.bytes().map(|c| table[c as usize]).collect();
    Ok(from_bytes(&b))
}

// ---- comparison ----
fn cmpr(o: std::cmp::Ordering) -> Value {
    Value::Int(match o {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        _ => 1,
    })
}

fn strcmp_family(vm: &mut Vm<'_>, f: &str, a: Vec<Value>, n: bool, ci: bool) -> R<Value> {
    argc(vm, f, &a, if n { 3 } else { 2 }, if n { 3 } else { 2 })?;
    let x = p_str(vm, f, &a, 0, "string1")?;
    let y = p_str(vm, f, &a, 1, "string2")?;
    let (mut x, mut y) = (x.as_bytes().to_vec(), y.as_bytes().to_vec());
    if n {
        let l = p_int(vm, f, &a, 2, "length")?;
        if l < 0 {
            return ve(vm, f, 3, "length", "must be greater than or equal to 0");
        }
        x.truncate(l as usize);
        y.truncate(l as usize);
    }
    if ci {
        x.make_ascii_lowercase();
        y.make_ascii_lowercase();
    }
    // PHP 8.2+: strcmp returns -1/0/1
    Ok(cmpr(x.cmp(&y)))
}

// ---- utf8_encode / decode ----
fn utf8_encode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "utf8_encode", &a, 1, 1)?;
    deprecated(vm, "Function utf8_encode() is deprecated since 8.2, visit the php.net documentation for various alternatives");
    let s = p_str(vm, "utf8_encode", &a, 0, "string")?;
    // The input is a byte string; bytes are treated as Latin-1 code points.
    Ok(vstr(&s.bytes().map(|b| b as char).collect::<String>()))
}
fn utf8_decode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "utf8_decode", &a, 1, 1)?;
    deprecated(vm, "Function utf8_decode() is deprecated since 8.2, visit the php.net documentation for various alternatives");
    let s = p_str(vm, "utf8_decode", &a, 0, "string")?;
    let b: String = s.chars().map(|c| if (c as u32) < 256 { (c as u32 as u8) as char } else { '?' }).collect();
    Ok(vstr(&b))
}

// ---- setlocale / localeconv ----
fn setlocale(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    if a.len() < 2 {
        return vm.throw_error("ArgumentCountError", &format!("setlocale() expects at least 2 arguments, {} given", a.len()));
    }
    let cat = p_int(vm, "setlocale", &a, 0, "category")?;
    let mut names: Vec<Value> = Vec::new();
    for v in &a[1..] {
        match v.deref() {
            Value::Array(x) => names.extend(x.iter().map(|(_, v)| v.deref())),
            o => names.push(o),
        }
    }
    let _ = cat;
    for n in names {
        let name = n.to_str();
        if name.len() >= 255 {
            warn(vm, "setlocale(): Specified locale name is too long");
            continue;
        }
        match &*name {
            "0" | "" | "C" | "POSIX" => return Ok(vstr("C")),
            _ => {}
        }
        if matches!(n, Value::Null) {
            return Ok(vstr("C"));
        }
    }
    Ok(Value::Bool(false))
}

fn localeconv(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "localeconv", &a, 0, 0)?;
    let mut r = PhpArray::new();
    let sv = |s: &str| vstr(s);
    r.set(Key::from_str("decimal_point"), sv("."));
    r.set(Key::from_str("thousands_sep"), sv(""));
    r.set(Key::from_str("int_curr_symbol"), sv(""));
    r.set(Key::from_str("currency_symbol"), sv(""));
    r.set(Key::from_str("mon_decimal_point"), sv(""));
    r.set(Key::from_str("mon_thousands_sep"), sv(""));
    r.set(Key::from_str("positive_sign"), sv(""));
    r.set(Key::from_str("negative_sign"), sv(""));
    for k in ["int_frac_digits", "frac_digits", "p_cs_precedes", "p_sep_by_space", "n_cs_precedes", "n_sep_by_space", "p_sign_posn", "n_sign_posn"] {
        r.set(Key::from_str(k), Value::Int(127));
    }
    r.set(Key::from_str("grouping"), list(vec![]));
    r.set(Key::from_str("mon_grouping"), list(vec![]));
    Ok(Value::Array(std::rc::Rc::new(r)))
}

pub fn register(m: &mut HashMap<&'static str, VmBuiltin>) {
    m.insert("dirname", dirname);
    m.insert("basename", basename);
    m.insert("chunk_split", chunk_split);
    m.insert("substr_replace", substr_replace);
    m.insert("quotemeta", quotemeta);
    m.insert("ord", ord);
    m.insert("chr", chr);
    m.insert("strrev", strrev);
    m.insert("addcslashes", addcslashes);
    m.insert("addslashes", addslashes);
    m.insert("stripcslashes", stripcslashes);
    m.insert("stripslashes", stripslashes);
    m.insert("nl2br", nl2br);
    m.insert("str_repeat", str_repeat);
    m.insert("count_chars", count_chars);
    m.insert("str_pad", str_pad);
    m.insert("str_rot13", str_rot13);
    m.insert("str_shuffle", str_shuffle);
    m.insert("str_word_count", str_word_count);
    m.insert("str_split", str_split);
    m.insert("strtr", strtr);
    m.insert("strcmp", |vm, a| strcmp_family(vm, "strcmp", a, false, false));
    m.insert("strcasecmp", |vm, a| strcmp_family(vm, "strcasecmp", a, false, true));
    m.insert("strncmp", |vm, a| strcmp_family(vm, "strncmp", a, true, false));
    m.insert("strncasecmp", |vm, a| strcmp_family(vm, "strncasecmp", a, true, true));
    m.insert("utf8_encode", utf8_encode);
    m.insert("utf8_decode", utf8_decode);
    m.insert("setlocale", setlocale);
    m.insert("localeconv", localeconv);
}
