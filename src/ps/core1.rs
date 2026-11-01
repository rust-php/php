//! Port of ext/standard/string.c, part 1: hex, spn, trim, wordwrap, explode/implode, strtok, case, search, substr.
use super::util::*;
use crate::builtins::VmBuiltin;
use crate::value::*;
use crate::vm::Vm;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub fn ve<T>(vm: &mut Vm<'_>, f: &str, n: usize, p: &str, msg: &str) -> R<T> {
    verr(vm, &format!("{f}(): Argument #{n} (${p}) {msg}"))
}

fn must_not_empty<T>(vm: &mut Vm<'_>, f: &str, n: usize, p: &str) -> R<T> {
    ve(vm, f, n, p, "must not be empty")
}

// ---- bin2hex / hex2bin ----
fn bin2hex(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "bin2hex", &a, 1, 1)?;
    let s = p_str(vm, "bin2hex", &a, 0, "string")?;
    Ok(vstr(&s.bytes().map(|b| format!("{b:02x}")).collect::<String>()))
}

fn hex2bin(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "hex2bin", &a, 1, 1)?;
    let s = p_str(vm, "hex2bin", &a, 0, "string")?;
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        warn(vm, "hex2bin(): Hexadecimal input string must have an even length");
        return Ok(Value::Bool(false));
    }
    let mut out = Vec::with_capacity(b.len() / 2);
    for c in b.chunks(2) {
        let h = |x: u8| (x as char).to_digit(16);
        match (h(c[0]), h(c[1])) {
            (Some(x), Some(y)) => out.push((x * 16 + y) as u8),
            _ => {
                warn(vm, "hex2bin(): Input string must be hexadecimal string");
                return Ok(Value::Bool(false));
            }
        }
    }
    Ok(from_bytes(&out))
}

// ---- charmask ----
/// php_charmask. Returns (mask, success)
pub fn charmask(vm: &mut Vm<'_>, f: &str, input: &[u8]) -> ([bool; 256], bool) {
    let mut mask = [false; 256];
    let mut ok = true;
    let len = input.len();
    let mut i = 0;
    while i < len {
        let c = input[i];
        if i + 3 < len && input[i + 1] == b'.' && input[i + 2] == b'.' && input[i + 3] >= c {
            for x in c..=input[i + 3] {
                mask[x as usize] = true;
            }
            i += 4;
            continue;
        } else if i + 1 < len && input[i] == b'.' && input[i + 1] == b'.' {
            if i == 0 {
                warn(vm, &format!("{f}(): Invalid '..'-range, no character to the left of '..'"));
            } else if i + 2 >= len {
                warn(vm, &format!("{f}(): Invalid '..'-range, no character to the right of '..'"));
            } else if input[i - 1] > input[i + 2] {
                warn(vm, &format!("{f}(): Invalid '..'-range, '..'-range needs to be incrementing"));
            } else {
                warn(vm, &format!("{f}(): Invalid '..'-range"));
            }
            ok = false;
        } else {
            mask[c as usize] = true;
        }
        i += 1;
    }
    (mask, ok)
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | 0x0c | b'\n' | b'\r' | b'\t' | 0x0b | 0)
}

// ---- trim ----
fn do_trim(vm: &mut Vm<'_>, f: &str, a: Vec<Value>, mode: i32) -> R<Value> {
    argc(vm, f, &a, 1, 2)?;
    let s = p_str(vm, f, &a, 0, "string")?;
    let b = s.as_bytes();
    let (mut start, mut end) = (0usize, b.len());
    if a.len() > 1 {
        let what = p_str(vm, f, &a, 1, "characters")?;
        if what.is_empty() {
            return Ok(Value::Str(s));
        }
        let w = what.as_bytes();
        // php_trim_int: single char fast path, else charmask
        let mask = if w.len() == 1 {
            let mut m = [false; 256];
            m[w[0] as usize] = true;
            m
        } else {
            charmask(vm, f, w).0
        };
        if mode & 1 != 0 {
            while start < end && mask[b[start] as usize] {
                start += 1;
            }
        }
        if mode & 2 != 0 {
            while end > start && mask[b[end - 1] as usize] {
                end -= 1;
            }
        }
    } else {
        if mode & 1 != 0 {
            while start < end && is_ws(b[start]) {
                start += 1;
            }
        }
        if mode & 2 != 0 {
            while end > start && is_ws(b[end - 1]) {
                end -= 1;
            }
        }
    }
    if start == 0 && end == b.len() {
        return Ok(Value::Str(s));
    }
    Ok(from_bytes(&b[start..end]))
}

// ---- strspn / strcspn ----
fn spn_common(vm: &mut Vm<'_>, f: &str, a: Vec<Value>, is_spn: bool) -> R<Value> {
    argc(vm, f, &a, 2, 4)?;
    let s = p_str(vm, f, &a, 0, "string")?;
    let chars = p_str(vm, f, &a, 1, "characters")?;
    let off = p_int_opt(vm, f, &a, 2, "offset", 0)?;
    let len = p_int_null(vm, f, &a, 3, "length")?;
    let b = s.as_bytes();
    let n = b.len() as i64;
    let mut start = off;
    if start < 0 {
        start += n;
        if start < 0 {
            start = 0;
        }
    } else if start > n {
        start = n;
    }
    let mut l = len.unwrap_or(n);
    let avail = n - start;
    if l < 0 {
        l += avail;
        if l < 0 {
            l = 0;
        }
    } else if l > avail {
        l = avail;
    }
    if l == 0 {
        return Ok(Value::Int(0));
    }
    let hay = &b[start as usize..(start + l) as usize];
    let cs = chars.as_bytes();
    let cnt = if is_spn { hay.iter().take_while(|c| cs.contains(c)).count() } else { hay.iter().take_while(|c| !cs.contains(c)).count() };
    Ok(Value::Int(cnt as i64))
}

// ---- wordwrap ----
fn wordwrap(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "wordwrap", &a, 1, 4)?;
    let text = p_str(vm, "wordwrap", &a, 0, "string")?;
    let linelength = p_int_opt(vm, "wordwrap", &a, 1, "width", 75)?;
    let brk = p_str_opt(vm, "wordwrap", &a, 2, "break", "\n")?;
    let docut = p_bool(&a, 3, false);
    if text.is_empty() {
        return Ok(vstr(""));
    }
    if brk.is_empty() {
        return must_not_empty(vm, "wordwrap", 3, "break");
    }
    if linelength == 0 && docut {
        return ve(vm, "wordwrap", 4, "cut_long_words", "cannot be true when argument #2 ($width) is 0");
    }
    let t = text.as_bytes();
    let br = brk.as_bytes();
    let n = t.len() as i64;
    if br.len() == 1 && !docut {
        let mut nt = t.to_vec();
        let (mut laststart, mut lastspace) = (0i64, 0i64);
        for cur in 0..n {
            let c = t[cur as usize];
            if c == br[0] {
                laststart = cur + 1;
                lastspace = cur + 1;
            } else if c == b' ' {
                if cur - laststart >= linelength {
                    nt[cur as usize] = br[0];
                    laststart = cur + 1;
                }
                lastspace = cur;
            } else if cur - laststart >= linelength && laststart != lastspace {
                nt[lastspace as usize] = br[0];
                laststart = lastspace + 1;
            }
        }
        return Ok(from_bytes(&nt));
    }
    let mut out: Vec<u8> = Vec::new();
    let (mut laststart, mut lastspace) = (0i64, 0i64);
    let mut cur = 0i64;
    while cur < n {
        let c = t[cur as usize];
        if c == br[0] && cur + br.len() as i64 <= n && &t[cur as usize..cur as usize + br.len()] == br {
            out.extend_from_slice(&t[laststart as usize..(cur + br.len() as i64) as usize]);
            cur += br.len() as i64 - 1;
            laststart = cur + 1;
            lastspace = cur + 1;
        } else if c == b' ' {
            if cur - laststart >= linelength {
                out.extend_from_slice(&t[laststart as usize..cur as usize]);
                out.extend_from_slice(br);
                laststart = cur + 1;
            }
            lastspace = cur;
        } else if cur - laststart >= linelength && docut && laststart >= lastspace {
            out.extend_from_slice(&t[laststart as usize..cur as usize]);
            out.extend_from_slice(br);
            laststart = cur;
            lastspace = cur;
        } else if cur - laststart >= linelength && laststart < lastspace {
            out.extend_from_slice(&t[laststart as usize..lastspace as usize]);
            out.extend_from_slice(br);
            lastspace += 1;
            laststart = lastspace;
        }
        cur += 1;
    }
    if laststart != cur {
        out.extend_from_slice(&t[laststart as usize..cur as usize]);
    }
    Ok(from_bytes(&out))
}

// ---- explode / implode ----
fn explode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "explode", &a, 2, 3)?;
    let delim = p_str(vm, "explode", &a, 0, "separator")?;
    let s = p_str(vm, "explode", &a, 1, "string")?;
    let limit = p_int_opt(vm, "explode", &a, 2, "limit", i64::MAX)?;
    if delim.is_empty() {
        return ve(vm, "explode", 1, "separator", "must not be empty, use str_split() to split a string into characters");
    }
    if s.is_empty() {
        return Ok(if limit >= 0 { list(vec![vstr("")]) } else { list(vec![]) });
    }
    let limit = if limit == 0 { 1 } else { limit };
    let mut parts: Vec<Value> = if limit > 1 {
        s.splitn(limit.min(i32::MAX as i64) as usize, &*delim).map(vstr).collect()
    } else if limit < 0 {
        s.split(&*delim).map(vstr).collect()
    } else {
        vec![vstr(&s)]
    };
    if limit < 0 {
        let drop = limit.unsigned_abs() as usize;
        let keep = parts.len().saturating_sub(drop);
        parts.truncate(keep);
    }
    Ok(list(parts))
}

fn implode_impl(vm: &mut Vm<'_>, glue: &str, arr: &PhpArray) -> R<String> {
    let mut out = String::new();
    let mut first = true;
    for (_, v) in arr.iter() {
        if !first {
            out.push_str(glue);
        }
        first = false;
        match v.deref() {
            Value::Array(_) => {
                warn(vm, "Array to string conversion");
                out.push_str("Array");
            }
            x => {
                let s = vm.to_string_val(&x)?;
                out.push_str(&s)
            }
        }
    }
    Ok(out)
}

fn implode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    implode_named(vm, "implode", a)
}

fn implode_named(vm: &mut Vm<'_>, f: &str, a: Vec<Value>) -> R<Value> {
    argc(vm, f, &a, 1, 2)?;
    let a1 = a[0].deref();
    let a2 = a.get(1).map(|v| v.deref()).unwrap_or(Value::Null);
    let pieces = match &a2 {
        Value::Array(x) => Some(x.clone()),
        Value::Null => None,
        _ => return vm.throw_error("TypeError", &format!("{f}(): Argument #2 ($array) must be of type ?array, {} given", zname(&a2))),
    };
    let (glue, arr): (Rc<str>, Rc<PhpArray>) = match pieces {
        None => match &a1 {
            Value::Array(x) => ("".into(), x.clone()),
            Value::Str(_) => {
                return vm.throw_error("TypeError", &format!("{f}(): If argument #1 ($separator) is of type string, argument #2 ($array) must be of type array, null given"));
            }
            _ => return vm.throw_error("TypeError", &format!("{f}(): Argument #1 ($array) must be of type array|string, {} given", zname(&a1))),
        },
        Some(x) => {
            if matches!(a1, Value::Array(_)) {
                return vm.throw_error("TypeError", &format!("{f}(): Argument #1 ($separator) must be of type string, array given"));
            }
            (p_str(vm, f, &a, 0, "separator")?, x)
        }
    };
    Ok(vstr(&implode_impl(vm, &glue, &arr)?))
}

// ---- strtok ----
thread_local! { static TOK: RefCell<(Option<Vec<u8>>, usize)> = const { RefCell::new((None, 0)) }; }

fn strtok(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "strtok", &a, 1, 2)?;
    let str_ = p_str(vm, "strtok", &a, 0, "string")?;
    let tok: Rc<str> = match a.get(1).map(|v| v.deref()) {
        None | Some(Value::Null) => str_.clone(),
        Some(_) => {
            let t = p_str(vm, "strtok", &a, 1, "token")?;
            TOK.with(|s| *s.borrow_mut() = (Some(str_.as_bytes().to_vec()), 0));
            t
        }
    };
    let (buf, last) = TOK.with(|s| {
        let s = s.borrow();
        (s.0.clone(), s.1)
    });
    let Some(buf) = buf else {
        warn(vm, "strtok(): Both arguments must be provided when starting tokenization");
        return Ok(Value::Bool(false));
    };
    let pe = buf.len();
    let mut p = last;
    if p >= pe {
        return Ok(Value::Bool(false));
    }
    let mut table = [false; 256];
    for &c in tok.as_bytes() {
        table[c as usize] = true;
    }
    let mut skipped = 0usize;
    let mut fail = false;
    while table[buf[p] as usize] {
        p += 1;
        if p >= pe {
            fail = true;
            break;
        }
        skipped += 1;
    }
    let mut ret = Value::Bool(false);
    if !fail {
        let mut found = false;
        loop {
            p += 1;
            if p >= pe {
                break;
            }
            if table[buf[p] as usize] {
                found = true;
                break;
            }
        }
        if found || p - last > 0 {
            ret = from_bytes(&buf[last + skipped..p]);
            TOK.with(|s| s.borrow_mut().1 = p + 1);
        } else {
            fail = true;
        }
    }
    if fail {
        TOK.with(|s| s.borrow_mut().0 = None);
    }
    Ok(ret)
}

// ---- case ----
fn strtoupper(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "strtoupper", &a, 1, 1)?;
    let s = p_str(vm, "strtoupper", &a, 0, "string")?;
    Ok(vstr(&s.to_ascii_uppercase()))
}
fn strtolower(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "strtolower", &a, 1, 1)?;
    let s = p_str(vm, "strtolower", &a, 0, "string")?;
    Ok(vstr(&s.to_ascii_lowercase()))
}
fn ucfirst(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "ucfirst", &a, 1, 1)?;
    let s = p_str(vm, "ucfirst", &a, 0, "string")?;
    let mut b = s.as_bytes().to_vec();
    if let Some(c) = b.first_mut() {
        *c = c.to_ascii_uppercase();
    }
    Ok(from_bytes(&b))
}
fn lcfirst(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "lcfirst", &a, 1, 1)?;
    let s = p_str(vm, "lcfirst", &a, 0, "string")?;
    let mut b = s.as_bytes().to_vec();
    if let Some(c) = b.first_mut() {
        *c = c.to_ascii_lowercase();
    }
    Ok(from_bytes(&b))
}
fn ucwords(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "ucwords", &a, 1, 2)?;
    let s = p_str(vm, "ucwords", &a, 0, "string")?;
    let delims = p_str_opt(vm, "ucwords", &a, 1, "separators", " \t\r\n\x0c\x0b")?;
    if s.is_empty() {
        return Ok(vstr(""));
    }
    let mut mask = [false; 256];
    for c in delims.bytes() {
        mask[c as usize] = true;
    }
    let mut b = s.as_bytes().to_vec();
    b[0] = b[0].to_ascii_uppercase();
    for i in 1..b.len() {
        if mask[b[i - 1] as usize] {
            b[i] = b[i].to_ascii_uppercase();
        }
    }
    Ok(from_bytes(&b))
}

// ---- str_increment / str_decrement ----
fn str_increment(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "str_increment", &a, 1, 1)?;
    let s = p_str(vm, "str_increment", &a, 0, "string")?;
    if s.is_empty() {
        return ve(vm, "str_increment", 1, "string", "must not be empty");
    }
    if !s.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return ve(vm, "str_increment", 1, "string", "must be composed only of alphanumeric ASCII characters");
    }
    let mut b = s.as_bytes().to_vec();
    let mut pos = b.len();
    let mut carry;
    loop {
        pos -= 1;
        carry = false;
        match b[pos] {
            b'z' => {
                b[pos] = b'a';
                carry = true;
            }
            b'Z' => {
                b[pos] = b'A';
                carry = true;
            }
            b'9' => {
                b[pos] = b'0';
                carry = true;
            }
            c => b[pos] = c + 1,
        }
        if !carry || pos == 0 {
            break;
        }
    }
    if carry {
        let first = match s.as_bytes()[0] {
            b'0'..=b'9' => b'1',
            b'a'..=b'z' => b'a',
            _ => b'A',
        };
        b.insert(0, first);
    }
    Ok(from_bytes(&b))
}

fn str_decrement(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "str_decrement", &a, 1, 1)?;
    let s = p_str(vm, "str_decrement", &a, 0, "string")?;
    if s.is_empty() {
        return ve(vm, "str_decrement", 1, "string", "must not be empty");
    }
    if !s.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return ve(vm, "str_decrement", 1, "string", "must be composed only of alphanumeric ASCII characters");
    }
    let b0 = s.as_bytes();
    let first = b0[0];
    let out_of_range = |vm: &mut Vm<'_>| -> R<Value> { ve(vm, "str_decrement", 1, "string", &format!("\"{s}\" is out of decrement range")) };
    if b0 == b"a" || b0 == b"A" || b0 == b"0" {
        return out_of_range(vm);
    }
    let mut b = b0.to_vec();
    let mut pos = b.len();
    let mut carry;
    loop {
        pos -= 1;
        carry = false;
        match b[pos] {
            b'a' => {
                b[pos] = b'z';
                carry = true;
            }
            b'A' => {
                b[pos] = b'Z';
                carry = true;
            }
            b'0' => {
                b[pos] = b'9';
                carry = true;
            }
            c => b[pos] = c - 1,
        }
        if !carry || pos == 0 {
            break;
        }
    }
    if carry {
        if b.len() == 1 {
            return out_of_range(vm);
        }
        // leading char underflowed: allowed only when it was 'a'/'A'/'1'... length shrinks by one
        if matches!(first, b'a' | b'A') || first == b'0' {
            if first == b'0' {
                return out_of_range(vm);
            }
            b.remove(0);
        } else {
            return out_of_range(vm);
        }
    } else if first == b'1' && b[0] == b'0' && b.len() > 1 {
        b.remove(0);
    }
    Ok(from_bytes(&b))
}

// ---- strstr family ----
fn find(h: &[u8], n: &[u8], from: usize) -> Option<usize> {
    if n.is_empty() {
        return if from <= h.len() { Some(from) } else { None };
    }
    if from > h.len() || h.len() - from < n.len() {
        return None;
    }
    h[from..].windows(n.len()).position(|w| w == n).map(|p| p + from)
}
fn rfind(h: &[u8], n: &[u8], from: usize, to: usize) -> Option<usize> {
    // last occurrence starting in [from, to]
    if h.len() < n.len() {
        return None;
    }
    let max_start = (h.len() - n.len()).min(to);
    if from > max_start {
        return None;
    }
    (from..=max_start).rev().find(|&i| &h[i..i + n.len()] == n)
}

fn strstr_common(vm: &mut Vm<'_>, f: &str, a: Vec<Value>, ci: bool) -> R<Value> {
    argc(vm, f, &a, 2, 3)?;
    let h = p_str(vm, f, &a, 0, "haystack")?;
    let n = p_str(vm, f, &a, 1, "needle")?;
    let before = p_bool(&a, 2, false);
    let (hb, nb) = if ci { (h.to_ascii_lowercase().into_bytes(), n.to_ascii_lowercase().into_bytes()) } else { (h.as_bytes().to_vec(), n.as_bytes().to_vec()) };
    match find(&hb, &nb, 0) {
        Some(p) => {
            let orig = h.as_bytes();
            Ok(from_bytes(if before { &orig[..p] } else { &orig[p..] }))
        }
        None => Ok(Value::Bool(false)),
    }
}

fn str_contains(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "str_contains", &a, 2, 2)?;
    let h = p_str(vm, "str_contains", &a, 0, "haystack")?;
    let n = p_str(vm, "str_contains", &a, 1, "needle")?;
    Ok(Value::Bool(find(h.as_bytes(), n.as_bytes(), 0).is_some()))
}
fn str_starts_with(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "str_starts_with", &a, 2, 2)?;
    let h = p_str(vm, "str_starts_with", &a, 0, "haystack")?;
    let n = p_str(vm, "str_starts_with", &a, 1, "needle")?;
    Ok(Value::Bool(h.as_bytes().starts_with(n.as_bytes())))
}
fn str_ends_with(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "str_ends_with", &a, 2, 2)?;
    let h = p_str(vm, "str_ends_with", &a, 0, "haystack")?;
    let n = p_str(vm, "str_ends_with", &a, 1, "needle")?;
    Ok(Value::Bool(h.as_bytes().ends_with(n.as_bytes())))
}

fn strpos_common(vm: &mut Vm<'_>, f: &str, a: Vec<Value>, ci: bool) -> R<Value> {
    argc(vm, f, &a, 2, 3)?;
    let h = p_str(vm, f, &a, 0, "haystack")?;
    let n = p_str(vm, f, &a, 1, "needle")?;
    let mut off = p_int_opt(vm, f, &a, 2, "offset", 0)?;
    let hl = h.len() as i64;
    if off < 0 {
        off += hl;
    }
    if off < 0 || off > hl {
        return ve(vm, f, 3, "offset", "must be contained in argument #1 ($haystack)");
    }
    let (hb, nb) = if ci { (h.to_ascii_lowercase().into_bytes(), n.to_ascii_lowercase().into_bytes()) } else { (h.as_bytes().to_vec(), n.as_bytes().to_vec()) };
    Ok(match find(&hb, &nb, off as usize) {
        Some(p) => Value::Int(p as i64),
        None => Value::Bool(false),
    })
}

fn strrpos_common(vm: &mut Vm<'_>, f: &str, a: Vec<Value>, ci: bool) -> R<Value> {
    argc(vm, f, &a, 2, 3)?;
    let h = p_str(vm, f, &a, 0, "haystack")?;
    let n = p_str(vm, f, &a, 1, "needle")?;
    let off = p_int_opt(vm, f, &a, 2, "offset", 0)?;
    let (hb, nb) = if ci { (h.to_ascii_lowercase().into_bytes(), n.to_ascii_lowercase().into_bytes()) } else { (h.as_bytes().to_vec(), n.as_bytes().to_vec()) };
    let hl = hb.len() as i64;
    let nl = nb.len() as i64;
    let (from, to);
    if off >= 0 {
        if off > hl {
            return ve(vm, f, 3, "offset", "must be contained in argument #1 ($haystack)");
        }
        from = off as usize;
        to = usize::MAX;
    } else {
        if off < -hl {
            return ve(vm, f, 3, "offset", "must be contained in argument #1 ($haystack)");
        }
        from = 0;
        // e = haystack + len - needle_len (when -offset >= needle_len) else limited
        let lim = if -off < nl { hl - nl } else { hl + off };
        if lim < 0 {
            return Ok(Value::Bool(false));
        }
        to = lim as usize;
    }
    Ok(match rfind(&hb, &nb, from, to) {
        Some(p) => Value::Int(p as i64),
        None => Value::Bool(false),
    })
}

fn strrchr(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "strrchr", &a, 2, 3)?;
    let h = p_str(vm, "strrchr", &a, 0, "haystack")?;
    let n = p_str(vm, "strrchr", &a, 1, "needle")?;
    let before = p_bool(&a, 2, false);
    let c = n.as_bytes().first().copied().unwrap_or(0);
    let b = h.as_bytes();
    match b.iter().rposition(|x| *x == c) {
        Some(p) => Ok(from_bytes(if before { &b[..p] } else { &b[p..] })),
        None => Ok(Value::Bool(false)),
    }
}

fn strpbrk(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "strpbrk", &a, 2, 2)?;
    let s = p_str(vm, "strpbrk", &a, 0, "string")?;
    let cl = p_str(vm, "strpbrk", &a, 1, "characters")?;
    if cl.is_empty() {
        return ve(vm, "strpbrk", 2, "characters", "must be a non-empty string");
    }
    let b = s.as_bytes();
    Ok(match b.iter().position(|c| cl.as_bytes().contains(c)) {
        Some(p) => from_bytes(&b[p..]),
        None => Value::Bool(false),
    })
}

// ---- substr ----
fn substr(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "substr", &a, 2, 3)?;
    let s = p_str(vm, "substr", &a, 0, "string")?;
    let mut f = p_int(vm, "substr", &a, 1, "offset")?;
    let l = p_int_null(vm, "substr", &a, 2, "length")?;
    let n = s.len() as i64;
    let mut len = l.unwrap_or(n);
    if f < 0 {
        if f.checked_neg().is_none_or(|x| x > n) {
            f = 0;
        } else {
            f += n;
        }
    } else if f > n {
        return Ok(vstr(""));
    }
    if len < 0 {
        len = (n - f) + len;
        if len < 0 {
            len = 0;
        }
    }
    if len > n - f {
        len = n - f;
    }
    Ok(from_bytes(&s.as_bytes()[f as usize..(f + len) as usize]))
}

fn substr_count(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "substr_count", &a, 2, 4)?;
    let h = p_str(vm, "substr_count", &a, 0, "haystack")?;
    let n = p_str(vm, "substr_count", &a, 1, "needle")?;
    let mut off = p_int_opt(vm, "substr_count", &a, 2, "offset", 0)?;
    let len = p_int_null(vm, "substr_count", &a, 3, "length")?;
    if n.is_empty() {
        return ve(vm, "substr_count", 2, "needle", "must not be empty");
    }
    let hl = h.len() as i64;
    if off < 0 {
        off += hl;
    }
    if off < 0 || off > hl {
        return ve(vm, "substr_count", 3, "offset", "must be contained in argument #1 ($haystack)");
    }
    let mut end = hl;
    if let Some(mut l) = len {
        if l < 0 {
            l += hl - off;
        }
        if l < 0 || l > hl - off {
            return ve(vm, "substr_count", 4, "length", "must be contained in argument #1 ($haystack)");
        }
        end = off + l;
    }
    let hay = &h.as_bytes()[off as usize..end as usize];
    let nb = n.as_bytes();
    let mut cnt = 0;
    let mut p = 0;
    while let Some(i) = find(hay, nb, p) {
        cnt += 1;
        p = i + nb.len();
    }
    Ok(Value::Int(cnt))
}

fn substr_compare(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "substr_compare", &a, 3, 5)?;
    let m1 = p_str(vm, "substr_compare", &a, 0, "haystack")?;
    let s2 = p_str(vm, "substr_compare", &a, 1, "needle")?;
    let mut offset = p_int(vm, "substr_compare", &a, 2, "offset")?;
    let len = p_int_null(vm, "substr_compare", &a, 3, "length")?;
    let ci = p_bool(&a, 4, false);
    if let Some(l) = len {
        if l < 0 {
            return ve(vm, "substr_compare", 4, "length", "must be greater than or equal to 0");
        }
    }
    let n1 = m1.len() as i64;
    if offset < 0 {
        offset += n1;
        if offset < 0 {
            offset = 0;
        }
    }
    if offset > n1 {
        return ve(vm, "substr_compare", 3, "offset", "must be contained in argument #1 ($main_str)");
    }
    let cmp_len = match len {
        Some(l) if l > 0 => l as usize,
        Some(_) => return Ok(Value::Int(0)),
        None => (n1 - offset).max(s2.len() as i64) as usize,
    };
    let x = &m1.as_bytes()[offset as usize..];
    let y = s2.as_bytes();
    let (x, y) = (&x[..x.len().min(cmp_len)], &y[..y.len().min(cmp_len)]);
    let o = if ci { x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase()) } else { x.cmp(y) };
    Ok(Value::Int(match o {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        _ => 1,
    }))
}

pub fn register(m: &mut HashMap<&'static str, VmBuiltin>) {
    m.insert("bin2hex", bin2hex);
    m.insert("hex2bin", hex2bin);
    m.insert("strspn", |vm, a| spn_common(vm, "strspn", a, true));
    m.insert("strcspn", |vm, a| spn_common(vm, "strcspn", a, false));
    m.insert("trim", |vm, a| do_trim(vm, "trim", a, 3));
    m.insert("rtrim", |vm, a| do_trim(vm, "rtrim", a, 2));
    m.insert("chop", |vm, a| do_trim(vm, "chop", a, 2));
    m.insert("ltrim", |vm, a| do_trim(vm, "ltrim", a, 1));
    m.insert("wordwrap", wordwrap);
    m.insert("explode", explode);
    m.insert("implode", implode);
    m.insert("join", |vm, a| implode_named(vm, "join", a));
    m.insert("strtok", strtok);
    m.insert("strtoupper", strtoupper);
    m.insert("strtolower", strtolower);
    m.insert("ucfirst", ucfirst);
    m.insert("lcfirst", lcfirst);
    m.insert("ucwords", ucwords);
    m.insert("str_increment", str_increment);
    m.insert("str_decrement", str_decrement);
    m.insert("stristr", |vm, a| strstr_common(vm, "stristr", a, true));
    m.insert("strstr", |vm, a| strstr_common(vm, "strstr", a, false));
    m.insert("strchr", |vm, a| strstr_common(vm, "strchr", a, false));
    m.insert("str_contains", str_contains);
    m.insert("str_starts_with", str_starts_with);
    m.insert("str_ends_with", str_ends_with);
    m.insert("strpos", |vm, a| strpos_common(vm, "strpos", a, false));
    m.insert("stripos", |vm, a| strpos_common(vm, "stripos", a, true));
    m.insert("strrpos", |vm, a| strrpos_common(vm, "strrpos", a, false));
    m.insert("strripos", |vm, a| strrpos_common(vm, "strripos", a, true));
    m.insert("strrchr", strrchr);
    m.insert("strpbrk", strpbrk);
    m.insert("substr", substr);
    m.insert("substr_count", substr_count);
    m.insert("substr_compare", substr_compare);
}
