//! Port of the commonly used parts of ext/mbstring/mbstring.c. UTF-8 only: every encoding name is validated
//! against the libmbfl list, but the string operations always use Unicode code points of the UTF-8 string.
use super::core1::ve;
use super::eaw::EAW;
use super::mbenc::ENCODINGS;
use super::util::*;
use crate::builtins::VmBuiltin;
use crate::value::*;
use crate::vm::Vm;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    static INTERNAL: RefCell<&'static str> = const { RefCell::new("UTF-8") };
    static DETECT_ORDER: RefCell<Vec<&'static str>> = RefCell::new(vec!["ASCII", "UTF-8"]);
    static SUBST: RefCell<Value> = const { RefCell::new(Value::Int(63)) };
}

/// mbfl_name2encoding: case-insensitive match on name or alias.
fn lookup(name: &str) -> Option<&'static str> {
    for (n, _, al) in ENCODINGS {
        if n.eq_ignore_ascii_case(name) || al.iter().any(|a| a.eq_ignore_ascii_case(name)) {
            return Some(n);
        }
    }
    None
}

/// php_mb_get_encoding: resolves optional encoding argument `idx` (0-based position in `a`).
fn get_enc(vm: &mut Vm<'_>, f: &str, a: &[Value], idx: usize) -> R<&'static str> {
    match a.get(idx).map(|v| v.deref()) {
        None | Some(Value::Null) => Ok(INTERNAL.with(|i| *i.borrow())),
        Some(_) => {
            let n = p_str(vm, f, a, idx, "encoding")?;
            match lookup(&n) {
                Some(e) => {
                    if matches!(e, "BASE64" | "UUENCODE" | "HTML-ENTITIES" | "Quoted-Printable") {
                        let msg = match e {
                            "BASE64" => "Handling Base64 via mbstring is deprecated; use base64_encode/base64_decode instead",
                            "Quoted-Printable" => "Handling QPrint via mbstring is deprecated; use quoted_printable_encode/quoted_printable_decode instead",
                            "HTML-ENTITIES" => "Handling HTML entities via mbstring is deprecated; use htmlspecialchars, htmlentities, or mb_encode_numericentity/mb_decode_numericentity instead",
                            _ => "Handling Uuencode via mbstring is deprecated; use convert_uuencode/convert_uudecode instead",
                        };
                        deprecated(vm, &format!("{f}(): {msg}"));
                    }
                    Ok(e)
                }
                None => ve(vm, f, idx + 1, "encoding", &format!("must be a valid encoding, \"{n}\" given")),
            }
        }
    }
}

fn char_width(c: u32) -> usize {
    if c < 0x1100 {
        return 1;
    }
    let (mut lo, mut hi) = (0usize, EAW.len());
    while lo < hi {
        let probe = (lo + hi) / 2;
        if c < EAW[probe].0 {
            hi = probe;
        } else if c > EAW[probe].1 {
            lo = probe + 1;
        } else {
            return 2;
        }
    }
    1
}

fn str_width(s: &str) -> usize {
    s.chars().map(|c| char_width(c as u32)).sum()
}

fn collect(s: &str) -> String {
    s.to_string()
}

fn mb_strlen(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_strlen", &a, 1, 2)?;
    let s = p_str(vm, "mb_strlen", &a, 0, "string")?;
    get_enc(vm, "mb_strlen", &a, 1)?;
    Ok(Value::Int(s.chars().count() as i64))
}

fn sub_chars(s: &str, from: usize, len: usize) -> String {
    s.chars().skip(from).take(len).collect()
}

fn mb_substr(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_substr", &a, 2, 4)?;
    let s = p_str(vm, "mb_substr", &a, 0, "string")?;
    let mut from = p_int(vm, "mb_substr", &a, 1, "start")?;
    let len = p_int_null(vm, "mb_substr", &a, 2, "length")?;
    get_enc(vm, "mb_substr", &a, 3)?;
    let n = s.chars().count() as i64;
    let mut l = len.unwrap_or(n);
    if from < 0 {
        from = from.saturating_add(n);
        if from < 0 {
            from = 0;
        }
    }
    if from > n {
        return Ok(vstr(""));
    }
    if l < 0 {
        l = (n - from).saturating_add(l);
        if l < 0 {
            l = 0;
        }
    }
    if l > n - from {
        l = n - from;
    }
    Ok(vstr(&sub_chars(&s, from as usize, l as usize)))
}

fn mb_strcut(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_strcut", &a, 2, 4)?;
    let s = p_str(vm, "mb_strcut", &a, 0, "string")?;
    let mut from = p_int(vm, "mb_strcut", &a, 1, "start")?;
    let len = p_int_null(vm, "mb_strcut", &a, 2, "length")?;
    get_enc(vm, "mb_strcut", &a, 3)?;
    let n = s.len() as i64;
    let mut l = len.unwrap_or(n);
    if from < 0 {
        from = (n + from).max(0);
    }
    if l < 0 {
        l = ((n - from) + l).max(0);
    }
    if from > n || l == 0 {
        return Ok(vstr(""));
    }
    // round start down to a character boundary, end down to a boundary
    let mut st = from as usize;
    while st > 0 && !s.is_char_boundary(st) {
        st -= 1;
    }
    let mut en = (from.saturating_add(l)).min(n) as usize;
    while en > st && !s.is_char_boundary(en) {
        en -= 1;
    }
    Ok(vstr(&s[st..en]))
}

fn fold1(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

fn fold_str(s: &str) -> Vec<char> {
    s.chars().map(fold1).collect()
}

/// mb_find_strpos on char vectors; returns Err(()) for offset errors.
fn find_pos(h: &[char], n: &[char], offset: i64, reverse: bool) -> Result<Option<usize>, ()> {
    let len = h.len() as i64;
    let start = if offset >= 0 {
        if offset > len {
            return Err(());
        }
        offset
    } else {
        if -offset > len {
            return Err(());
        }
        len + offset
    };
    if h.len() < n.len() {
        return Ok(None);
    }
    let last_start = (h.len() - n.len()) as i64;
    let matches_at = |i: usize| h[i..i + n.len()] == *n;
    if !reverse {
        let mut i = start;
        while i <= last_start {
            if matches_at(i as usize) {
                return Ok(Some(i as usize));
            }
            i += 1;
        }
        Ok(None)
    } else if offset >= 0 {
        let mut i = last_start;
        while i >= start {
            if matches_at(i as usize) {
                return Ok(Some(i as usize));
            }
            i -= 1;
        }
        Ok(None)
    } else {
        let mut i = start.min(last_start);
        while i >= 0 {
            if matches_at(i as usize) {
                return Ok(Some(i as usize));
            }
            i -= 1;
        }
        Ok(None)
    }
}

fn strpos_common(vm: &mut Vm<'_>, f: &str, a: Vec<Value>, reverse: bool, ci: bool) -> R<Value> {
    argc(vm, f, &a, 2, 4)?;
    let h = p_str(vm, f, &a, 0, "haystack")?;
    let n = p_str(vm, f, &a, 1, "needle")?;
    let off = p_int_opt(vm, f, &a, 2, "offset", 0)?;
    get_enc(vm, f, &a, 3)?;
    let (hv, nv): (Vec<char>, Vec<char>) = if ci { (fold_str(&h), fold_str(&n)) } else { (h.chars().collect(), n.chars().collect()) };
    match find_pos(&hv, &nv, off, reverse) {
        Ok(Some(p)) => Ok(Value::Int(p as i64)),
        Ok(None) => Ok(Value::Bool(false)),
        Err(()) => ve(vm, f, 3, "offset", "must be contained in argument #1 ($haystack)"),
    }
}

fn strstr_common(vm: &mut Vm<'_>, f: &str, a: Vec<Value>, reverse: bool, ci: bool) -> R<Value> {
    argc(vm, f, &a, 2, 4)?;
    let h = p_str(vm, f, &a, 0, "haystack")?;
    let n = p_str(vm, f, &a, 1, "needle")?;
    let before = p_bool(&a, 2, false);
    get_enc(vm, f, &a, 3)?;
    let (hv, nv): (Vec<char>, Vec<char>) = if ci { (fold_str(&h), fold_str(&n)) } else { (h.chars().collect(), n.chars().collect()) };
    match find_pos(&hv, &nv, 0, reverse) {
        Ok(Some(p)) => {
            let orig: Vec<char> = h.chars().collect();
            let r: String = if before { orig[..p].iter().collect() } else { orig[p..].iter().collect() };
            Ok(vstr(&r))
        }
        _ => Ok(Value::Bool(false)),
    }
}

fn mb_substr_count(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_substr_count", &a, 2, 3)?;
    let h = p_str(vm, "mb_substr_count", &a, 0, "haystack")?;
    let n = p_str(vm, "mb_substr_count", &a, 1, "needle")?;
    get_enc(vm, "mb_substr_count", &a, 2)?;
    if n.is_empty() {
        return ve(vm, "mb_substr_count", 2, "needle", "must not be empty");
    }
    Ok(Value::Int(h.matches(&*n).count() as i64))
}

fn mb_str_split(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_str_split", &a, 1, 3)?;
    let s = p_str(vm, "mb_str_split", &a, 0, "string")?;
    let n = p_int_opt(vm, "mb_str_split", &a, 1, "length", 1)?;
    get_enc(vm, "mb_str_split", &a, 2)?;
    if n < 1 {
        return ve(vm, "mb_str_split", 2, "length", "must be greater than 0");
    }
    let chars: Vec<char> = s.chars().collect();
    Ok(list(chars.chunks(n.min(i32::MAX as i64) as usize).map(|c| vstr(&c.iter().collect::<String>())).collect()))
}

fn is_cased(c: char) -> bool {
    c.is_lowercase() || c.is_uppercase() || matches!(c, '\u{1C5}' | '\u{1C8}' | '\u{1CB}' | '\u{1F2}')
}
fn is_case_ignorable(c: char) -> bool {
    matches!(c, '\'' | '.' | ':' | '^' | '`' | '\u{a8}' | '\u{ad}' | '\u{af}' | '\u{b4}' | '\u{b7}' | '\u{b8}' | '\u{2018}' | '\u{2019}' | '\u{2024}' | '\u{2027}' | '\u{fe13}' | '\u{fe52}' | '\u{fe55}' | '\u{ff07}' | '\u{ff0e}' | '\u{ff1a}')
        || ('\u{300}'..='\u{36f}').contains(&c)
        || ('\u{2b0}'..='\u{2ff}').contains(&c)
}

fn title_char(c: char) -> String {
    match c {
        '\u{1C4}' | '\u{1C5}' | '\u{1C6}' => "\u{1C5}".into(),
        '\u{1C7}' | '\u{1C8}' | '\u{1C9}' => "\u{1C8}".into(),
        '\u{1CA}' | '\u{1CB}' | '\u{1CC}' => "\u{1CB}".into(),
        '\u{1F1}' | '\u{1F2}' | '\u{1F3}' => "\u{1F2}".into(),
        'ß' => "Ss".into(),
        _ => c.to_uppercase().collect(),
    }
}

fn simple_upper(c: char) -> char {
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

fn convert_case(s: &str, mode: i64) -> String {
    match mode {
        0 => s.to_uppercase(),
        1 => s.to_lowercase(),
        3 => s.chars().flat_map(|c| if c == 'ß' { "ss".chars().collect::<Vec<_>>() } else { c.to_lowercase().collect() }).collect(),
        4 => s.chars().map(simple_upper).collect(),
        5 => s.chars().map(fold1).collect(),
        7 => s.chars().map(fold1).collect(),
        _ => {
            // title (2) and simple title (6)
            let mut out = String::new();
            let mut in_word = false;
            for c in s.chars() {
                if in_word {
                    if is_case_ignorable(c) {
                        out.push(c);
                    } else if is_cased(c) {
                        if mode == 6 {
                            out.push(fold1(c))
                        } else {
                            out.extend(c.to_lowercase())
                        }
                    } else {
                        in_word = false;
                        out.push(c);
                    }
                } else if is_cased(c) {
                    in_word = true;
                    if mode == 6 {
                        out.push(simple_upper(c))
                    } else {
                        out.push_str(&title_char(c))
                    }
                } else {
                    out.push(c);
                }
            }
            out
        }
    }
}

fn mb_case_fn(vm: &mut Vm<'_>, f: &str, a: Vec<Value>, mode: i64) -> R<Value> {
    argc(vm, f, &a, 1, 2)?;
    let s = p_str(vm, f, &a, 0, "string")?;
    get_enc(vm, f, &a, 1)?;
    Ok(vstr(&convert_case(&s, mode)))
}

fn mb_convert_case(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_convert_case", &a, 2, 3)?;
    let s = p_str(vm, "mb_convert_case", &a, 0, "string")?;
    let mode = p_int(vm, "mb_convert_case", &a, 1, "mode")?;
    get_enc(vm, "mb_convert_case", &a, 2)?;
    if !(0..=7).contains(&mode) {
        return ve(vm, "mb_convert_case", 2, "mode", "must be one of the MB_CASE_* constants");
    }
    Ok(vstr(&convert_case(&s, mode)))
}

fn ulcfirst(vm: &mut Vm<'_>, f: &str, a: Vec<Value>, title: bool) -> R<Value> {
    argc(vm, f, &a, 1, 2)?;
    let s = p_str(vm, f, &a, 0, "string")?;
    get_enc(vm, f, &a, 1)?;
    let mut it = s.chars();
    let Some(c) = it.next() else { return Ok(vstr("")) };
    let first: String = if title { title_char(c) } else { c.to_lowercase().collect() };
    Ok(vstr(&format!("{first}{}", it.as_str())))
}

fn mb_strwidth(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_strwidth", &a, 1, 2)?;
    let s = p_str(vm, "mb_strwidth", &a, 0, "string")?;
    get_enc(vm, "mb_strwidth", &a, 1)?;
    Ok(Value::Int(str_width(&s) as i64))
}

fn mb_strimwidth(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_strimwidth", &a, 3, 5)?;
    let s = p_str(vm, "mb_strimwidth", &a, 0, "string")?;
    let mut from = p_int(vm, "mb_strimwidth", &a, 1, "start")?;
    let mut width = p_int(vm, "mb_strimwidth", &a, 2, "width")?;
    let marker = p_str_opt(vm, "mb_strimwidth", &a, 3, "trim_marker", "")?;
    get_enc(vm, "mb_strimwidth", &a, 4)?;
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len() as i64;
    if from != 0 {
        if from < 0 {
            from += n;
        }
        if from < 0 || from > n {
            return ve(vm, "mb_strimwidth", 2, "start", "is out of range");
        }
    }
    if width < 0 {
        deprecated(vm, "mb_strimwidth(): passing a negative integer to argument #3 ($width) is deprecated");
        width += str_width(&s) as i64;
        if from > 0 {
            let pre: String = chars[..from as usize].iter().collect();
            width -= str_width(&pre) as i64;
        }
        if width < 0 {
            return ve(vm, "mb_strimwidth", 3, "width", "is out of range");
        }
    }
    let rest = &chars[from as usize..];
    let total: usize = rest.iter().map(|c| char_width(*c as u32)).sum();
    if total as i64 <= width {
        return Ok(vstr(&rest.iter().collect::<String>()));
    }
    let mw = str_width(&marker) as i64;
    if width <= mw {
        return Ok(Value::Str(marker));
    }
    let mut remaining = width - mw;
    let mut out = String::new();
    for c in rest {
        let w = char_width(*c as u32) as i64;
        if remaining < w {
            break;
        }
        remaining -= w;
        out.push(*c);
    }
    out.push_str(&marker);
    Ok(vstr(&out))
}

fn mb_ord(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_ord", &a, 1, 2)?;
    let s = p_str(vm, "mb_ord", &a, 0, "string")?;
    get_enc(vm, "mb_ord", &a, 1)?;
    if s.is_empty() {
        return ve(vm, "mb_ord", 1, "string", "must not be empty");
    }
    Ok(Value::Int(s.chars().next().unwrap() as i64))
}

fn mb_chr(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_chr", &a, 1, 2)?;
    let cp = p_int(vm, "mb_chr", &a, 0, "codepoint")?;
    get_enc(vm, "mb_chr", &a, 1)?;
    if (0..0x110000).contains(&cp) {
        if let Some(c) = char::from_u32(cp as u32) {
            return Ok(vstr(&c.to_string()));
        }
    }
    Ok(Value::Bool(false))
}

fn mb_scrub(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_scrub", &a, 1, 2)?;
    let s = p_str(vm, "mb_scrub", &a, 0, "string")?;
    get_enc(vm, "mb_scrub", &a, 1)?;
    Ok(Value::Str(s))
}

fn valid_in(enc: &str, s: &str) -> bool {
    match enc {
        "ASCII" | "7bit" => s.is_ascii(),
        _ => true,
    }
}

fn check_value(v: &Value, enc: &str) -> bool {
    match v.deref() {
        Value::Array(x) => x.iter().all(|(k, v)| (match k {
            Key::Str(s) => valid_in(enc, s),
            _ => true,
        }) && check_value(v, enc)),
        Value::Str(s) => valid_in(enc, &s),
        _ => true,
    }
}

fn mb_check_encoding(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_check_encoding", &a, 0, 2)?;
    let enc = get_enc(vm, "mb_check_encoding", &a, 1)?;
    match a.first().map(|v| v.deref()) {
        None | Some(Value::Null) => {
            deprecated(vm, "mb_check_encoding(): Calling mb_check_encoding() without argument is deprecated");
            Ok(Value::Bool(true))
        }
        Some(v @ Value::Array(_)) => Ok(Value::Bool(check_value(&v, enc))),
        Some(_) => {
            let s = p_str(vm, "mb_check_encoding", &a, 0, "value")?;
            Ok(Value::Bool(valid_in(enc, &s)))
        }
    }
}

fn encoding_list(vm: &mut Vm<'_>, f: &str, v: &Value, argn: usize, pname: &str) -> R<Vec<&'static str>> {
    let names: Vec<String> = match v.deref() {
        Value::Array(x) => x.iter().map(|(_, v)| v.deref().to_str().to_string()).collect(),
        o => o.to_str().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty() || false).collect(),
    };
    let mut out = vec![];
    for n in names {
        if n.eq_ignore_ascii_case("auto") {
            out.extend(DETECT_ORDER.with(|d| d.borrow().clone()));
            continue;
        }
        match lookup(&n) {
            Some(e) => out.push(e),
            None => return ve(vm, f, argn, pname, &format!("contains invalid encoding \"{n}\"")),
        }
    }
    Ok(out)
}

fn mb_detect_encoding(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_detect_encoding", &a, 1, 3)?;
    let s = p_str(vm, "mb_detect_encoding", &a, 0, "string")?;
    let list = match a.get(1).map(|v| v.deref()) {
        None | Some(Value::Null) => DETECT_ORDER.with(|d| d.borrow().clone()),
        Some(v) => {
            let l = encoding_list(vm, "mb_detect_encoding", &v, 2, "encodings")?;
            if l.is_empty() {
                return ve(vm, "mb_detect_encoding", 2, "encodings", "must specify at least one encoding");
            }
            l
        }
    };
    for e in list {
        if valid_in(e, &s) {
            return Ok(vstr(e));
        }
    }
    Ok(Value::Bool(false))
}

fn mb_internal_encoding(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_internal_encoding", &a, 0, 1)?;
    match a.first().map(|v| v.deref()) {
        None | Some(Value::Null) => Ok(vstr(INTERNAL.with(|i| *i.borrow()))),
        Some(_) => {
            let n = p_str(vm, "mb_internal_encoding", &a, 0, "encoding")?;
            match lookup(&n) {
                Some(e) => {
                    INTERNAL.with(|i| *i.borrow_mut() = e);
                    Ok(Value::Bool(true))
                }
                None => ve(vm, "mb_internal_encoding", 1, "encoding", &format!("must be a valid encoding, \"{n}\" given")),
            }
        }
    }
}

fn mb_convert_encoding(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_convert_encoding", &a, 2, 3)?;
    let to = p_str(vm, "mb_convert_encoding", &a, 1, "to_encoding")?;
    let Some(to_e) = lookup(&to) else {
        return ve(vm, "mb_convert_encoding", 2, "to_encoding", &format!("must be a valid encoding, \"{to}\" given"));
    };
    if let Some(f) = a.get(2).map(|v| v.deref()) {
        if !matches!(f, Value::Null) {
            let l = encoding_list(vm, "mb_convert_encoding", &f, 3, "from_encoding")?;
            if l.is_empty() {
                return ve(vm, "mb_convert_encoding", 3, "from_encoding", "must specify at least one encoding");
            }
        }
    }
    fn conv(s: &str, to: &str) -> String {
        match to {
            "ASCII" | "7bit" => s.chars().map(|c| if c.is_ascii() { c } else { '?' }).collect(),
            "ISO-8859-1" => s.chars().map(|c| if (c as u32) < 256 { c } else { '?' }).collect(),
            "HTML-ENTITIES" => s.chars().map(|c| if c.is_ascii() { c.to_string() } else { format!("&#{};", c as u32) }).collect(),
            _ => s.to_string(),
        }
    }
    fn walk(v: &Value, to: &str) -> Value {
        match v.deref() {
            Value::Array(x) => {
                let mut r = PhpArray::new();
                for (k, v) in x.iter() {
                    let nk = match k {
                        Key::Str(s) => Key::from_str(&conv(s, to)),
                        o => o.clone(),
                    };
                    r.set(nk, walk(v, to));
                }
                Value::Array(Rc::new(r))
            }
            Value::Str(s) => vstr(&conv(&s, to)),
            o => o,
        }
    }
    match a[0].deref() {
        v @ Value::Array(_) => Ok(walk(&v, to_e)),
        _ => {
            let s = p_str(vm, "mb_convert_encoding", &a, 0, "string")?;
            Ok(vstr(&conv(&s, to_e)))
        }
    }
}

fn mb_list_encodings(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_list_encodings", &a, 0, 0)?;
    Ok(list(ENCODINGS.iter().map(|(n, _, _)| vstr(n)).collect()))
}

fn mb_encoding_aliases(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_encoding_aliases", &a, 1, 1)?;
    let n = p_str(vm, "mb_encoding_aliases", &a, 0, "encoding")?;
    match lookup(&n) {
        Some(e) => {
            let al = ENCODINGS.iter().find(|x| x.0 == e).map(|x| x.2).unwrap_or(&[]);
            Ok(list(al.iter().map(|x| vstr(x)).collect()))
        }
        None => ve(vm, "mb_encoding_aliases", 1, "encoding", &format!("must be a valid encoding, \"{n}\" given")),
    }
}

fn mb_preferred_mime_name(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_preferred_mime_name", &a, 1, 1)?;
    let n = p_str(vm, "mb_preferred_mime_name", &a, 0, "encoding")?;
    match lookup(&n) {
        Some(e) => match ENCODINGS.iter().find(|x| x.0 == e).and_then(|x| x.1) {
            Some(m) => Ok(vstr(m)),
            None => {
                warn(vm, &format!("mb_preferred_mime_name(): No MIME preferred name corresponding to \"{e}\""));
                Ok(Value::Bool(false))
            }
        },
        None => ve(vm, "mb_preferred_mime_name", 1, "encoding", &format!("must be a valid encoding, \"{n}\" given")),
    }
}

fn mb_language(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_language", &a, 0, 1)?;
    match a.first().map(|v| v.deref()) {
        None | Some(Value::Null) => Ok(vstr("neutral")),
        Some(_) => {
            let n = p_str(vm, "mb_language", &a, 0, "language")?;
            const LANGS: &[&str] = &["neutral", "uni", "universal", "Japanese", "ja", "Korean", "ko", "Russian", "ru", "English", "en", "German", "de", "Armenian", "hy", "Turkish", "tr", "Simplified Chinese", "zh-cn", "Traditional Chinese", "zh-tw", "Ukrainian", "ua", "Greek", "el"];
            if LANGS.iter().any(|l| l.eq_ignore_ascii_case(&n)) {
                Ok(Value::Bool(true))
            } else {
                ve(vm, "mb_language", 1, "language", &format!("must be a valid language, \"{n}\" given"))
            }
        }
    }
}

fn mb_detect_order(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_detect_order", &a, 0, 1)?;
    match a.first().map(|v| v.deref()) {
        None | Some(Value::Null) => Ok(list(DETECT_ORDER.with(|d| d.borrow().iter().map(|x| vstr(x)).collect()))),
        Some(v) => {
            let l = encoding_list(vm, "mb_detect_order", &v, 1, "encoding")?;
            if l.is_empty() {
                return ve(vm, "mb_detect_order", 1, "encoding", "must specify at least one encoding");
            }
            DETECT_ORDER.with(|d| *d.borrow_mut() = l);
            Ok(Value::Bool(true))
        }
    }
}

fn mb_substitute_character(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_substitute_character", &a, 0, 1)?;
    match a.first().map(|v| v.deref()) {
        None | Some(Value::Null) => Ok(SUBST.with(|s| s.borrow().clone())),
        Some(Value::Str(s)) if parse_numeric(&s).is_none() => {
            if matches!(&*s.to_ascii_lowercase(), "none" | "long" | "entity") {
                SUBST.with(|x| *x.borrow_mut() = vstr(&s.to_ascii_lowercase()));
                Ok(Value::Bool(true))
            } else {
                ve(vm, "mb_substitute_character", 1, "substitute_character", "must be \"none\", \"long\", \"entity\" or a valid codepoint")
            }
        }
        Some(_) => {
            let n = p_int(vm, "mb_substitute_character", &a, 0, "substitute_character")?;
            if n < 0 || n > 0x10FFFF || (0xD800..=0xDFFF).contains(&n) {
                return ve(vm, "mb_substitute_character", 1, "substitute_character", "is not a valid codepoint");
            }
            SUBST.with(|x| *x.borrow_mut() = Value::Int(n));
            Ok(Value::Bool(true))
        }
    }
}

fn mb_http_output(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_http_output", &a, 0, 1)?;
    match a.first().map(|v| v.deref()) {
        None | Some(Value::Null) => Ok(vstr("UTF-8")),
        Some(_) => {
            let n = p_str(vm, "mb_http_output", &a, 0, "encoding")?;
            match lookup(&n) {
                Some(_) => Ok(Value::Bool(true)),
                None => ve(vm, "mb_http_output", 1, "encoding", &format!("must be a valid encoding, \"{n}\" given")),
            }
        }
    }
}

fn mb_http_input(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_http_input", &a, 0, 1)?;
    Ok(Value::Bool(false))
}

fn is_default_trim(c: char) -> bool {
    matches!(c as u32, 0x20 | 0x0C | 0x0A | 0x0D | 0x09 | 0x0B | 0x00 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0x85 | 0x180E)
}

fn mb_trim_common(vm: &mut Vm<'_>, f: &str, a: Vec<Value>, mode: i32) -> R<Value> {
    argc(vm, f, &a, 1, 3)?;
    let s = p_str(vm, f, &a, 0, "string")?;
    let what: Option<Vec<char>> = match a.get(1).map(|v| v.deref()) {
        None | Some(Value::Null) => None,
        Some(_) => Some(p_str(vm, f, &a, 1, "characters")?.chars().collect()),
    };
    get_enc(vm, f, &a, 2)?;
    if let Some(w) = &what {
        if w.is_empty() {
            return Ok(Value::Str(s));
        }
    }
    let is_trim = |c: char| match &what {
        Some(w) => w.contains(&c),
        None => is_default_trim(c),
    };
    let mut r: &str = &s;
    if mode & 1 != 0 {
        r = r.trim_start_matches(is_trim);
    }
    if mode & 2 != 0 {
        r = r.trim_end_matches(is_trim);
    }
    Ok(vstr(r))
}

fn mb_str_pad(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "mb_str_pad", &a, 2, 5)?;
    let s = p_str(vm, "mb_str_pad", &a, 0, "string")?;
    let len = p_int(vm, "mb_str_pad", &a, 1, "length")?;
    let pad = p_str_opt(vm, "mb_str_pad", &a, 2, "pad_string", " ")?;
    let ty = p_int_opt(vm, "mb_str_pad", &a, 3, "pad_type", 1)?;
    get_enc(vm, "mb_str_pad", &a, 4)?;
    let cur = s.chars().count() as i64;
    if len < 0 || len <= cur {
        return Ok(Value::Str(s));
    }
    if pad.is_empty() {
        return ve(vm, "mb_str_pad", 3, "pad_string", "must not be empty");
    }
    if !(0..=2).contains(&ty) {
        return ve(vm, "mb_str_pad", 4, "pad_type", "must be STR_PAD_LEFT, STR_PAD_RIGHT, or STR_PAD_BOTH");
    }
    let pc: Vec<char> = pad.chars().collect();
    let total = (len - cur) as usize;
    let (l, r) = match ty {
        0 => (total, 0),
        2 => (total / 2, total - total / 2),
        _ => (0, total),
    };
    let fill = |n: usize| -> String { (0..n).map(|i| pc[i % pc.len()]).collect() };
    Ok(vstr(&format!("{}{}{}", fill(l), s, fill(r))))
}

pub fn register(m: &mut HashMap<&'static str, VmBuiltin>) {
    let _ = collect;
    m.insert("mb_strlen", mb_strlen);
    m.insert("mb_substr", mb_substr);
    m.insert("mb_strcut", mb_strcut);
    m.insert("mb_strpos", |vm, a| strpos_common(vm, "mb_strpos", a, false, false));
    m.insert("mb_strrpos", |vm, a| strpos_common(vm, "mb_strrpos", a, true, false));
    m.insert("mb_stripos", |vm, a| strpos_common(vm, "mb_stripos", a, false, true));
    m.insert("mb_strripos", |vm, a| strpos_common(vm, "mb_strripos", a, true, true));
    m.insert("mb_strstr", |vm, a| strstr_common(vm, "mb_strstr", a, false, false));
    m.insert("mb_strrchr", |vm, a| strstr_common(vm, "mb_strrchr", a, true, false));
    m.insert("mb_stristr", |vm, a| strstr_common(vm, "mb_stristr", a, false, true));
    m.insert("mb_strrichr", |vm, a| strstr_common(vm, "mb_strrichr", a, true, true));
    m.insert("mb_substr_count", mb_substr_count);
    m.insert("mb_str_split", mb_str_split);
    m.insert("mb_strtoupper", |vm, a| mb_case_fn(vm, "mb_strtoupper", a, 0));
    m.insert("mb_strtolower", |vm, a| mb_case_fn(vm, "mb_strtolower", a, 1));
    m.insert("mb_convert_case", mb_convert_case);
    m.insert("mb_ucfirst", |vm, a| ulcfirst(vm, "mb_ucfirst", a, true));
    m.insert("mb_lcfirst", |vm, a| ulcfirst(vm, "mb_lcfirst", a, false));
    m.insert("mb_strwidth", mb_strwidth);
    m.insert("mb_strimwidth", mb_strimwidth);
    m.insert("mb_ord", mb_ord);
    m.insert("mb_chr", mb_chr);
    m.insert("mb_scrub", mb_scrub);
    m.insert("mb_check_encoding", mb_check_encoding);
    m.insert("mb_detect_encoding", mb_detect_encoding);
    m.insert("mb_internal_encoding", mb_internal_encoding);
    m.insert("mb_convert_encoding", mb_convert_encoding);
    m.insert("mb_list_encodings", mb_list_encodings);
    m.insert("mb_encoding_aliases", mb_encoding_aliases);
    m.insert("mb_preferred_mime_name", mb_preferred_mime_name);
    m.insert("mb_language", mb_language);
    m.insert("mb_detect_order", mb_detect_order);
    m.insert("mb_substitute_character", mb_substitute_character);
    m.insert("mb_http_output", mb_http_output);
    m.insert("mb_http_input", mb_http_input);
    m.insert("mb_trim", |vm, a| mb_trim_common(vm, "mb_trim", a, 3));
    m.insert("mb_ltrim", |vm, a| mb_trim_common(vm, "mb_ltrim", a, 1));
    m.insert("mb_rtrim", |vm, a| mb_trim_common(vm, "mb_rtrim", a, 2));
    m.insert("mb_str_pad", mb_str_pad);
}
