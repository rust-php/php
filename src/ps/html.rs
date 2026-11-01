//! Port of ext/standard/html.c (UTF-8 semantics; other charsets are recognised but treated as Unicode).
use super::entities::{HTML401, HTML5};
use super::util::*;
use crate::builtins::VmBuiltin;
use crate::value::*;
use crate::vm::Vm;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::OnceLock;

const Q_SINGLE: i64 = 1;
const Q_DOUBLE: i64 = 2;
const IGNORE_ERRORS: i64 = 4;
const SUBSTITUTE_ERRORS: i64 = 8;
const DOC_MASK: i64 = 48;
const DOC_HTML401: i64 = 0;
const DOC_XML1: i64 = 16;
const DOC_XHTML: i64 = 32;
const DOC_HTML5: i64 = 48;
const SUBST_DISALLOWED: i64 = 128;
const ENT_DEFAULT: i64 = 3 | 8;

const LONGEST_ENTITY_LENGTH: usize = 31;

#[derive(PartialEq, Clone, Copy)]
enum Cs {
    Utf8,
    Unicode,
    Partial,
}

const SUPPORTED: &[&str] = &[
    "iso-8859-1", "iso8859-1", "iso-8859-15", "iso8859-15", "cp1252", "windows-1252", "1252", "koi8-r", "koi8-ru", "koi8r", "cp1251", "windows-1251", "win-1251",
    "iso8859-5", "iso-8859-5", "cp866", "866", "ibm866", "macroman",
];
const PARTIAL: &[&str] = &["big5", "950", "gb2312", "936", "big5-hkscs", "shift_jis", "sjis", "932", "sjis-win", "cp932", "eucjp", "euc-jp", "eucjp-win"];

fn determine_charset(vm: &mut Vm<'_>, f: &str, hint: Option<&str>, quiet: bool) -> Cs {
    let hint = match hint {
        Some(h) if !h.is_empty() => h,
        _ => return Cs::Utf8,
    };
    if hint.eq_ignore_ascii_case("utf-8") {
        return Cs::Utf8;
    }
    let lc = hint.to_ascii_lowercase();
    if hint.len() <= 12 {
        if SUPPORTED.contains(&lc.as_str()) {
            return Cs::Unicode;
        }
        if PARTIAL.contains(&lc.as_str()) {
            return Cs::Partial;
        }
    }
    if !quiet {
        warn(vm, &format!("{f}(): Charset \"{hint}\" is not supported, assuming UTF-8"));
    }
    Cs::Utf8
}

struct Tables {
    /// name -> (cp1, cp2)
    dec4: HashMap<&'static str, (u32, u32)>,
    dec5: HashMap<&'static str, (u32, u32)>,
    /// cp -> (default entity, [(second cp, entity)])
    enc4: BTreeMap<u32, (Option<&'static str>, Vec<(u32, &'static str)>)>,
    enc5: BTreeMap<u32, (Option<&'static str>, Vec<(u32, &'static str)>)>,
}

fn build_enc(src: &'static [(&'static str, u32, u32)]) -> BTreeMap<u32, (Option<&'static str>, Vec<(u32, &'static str)>)> {
    let mut m: BTreeMap<u32, (Option<&'static str>, Vec<(u32, &'static str)>)> = BTreeMap::new();
    for &(n, a, b) in src {
        let n = if n == "QUOT" {
            "quot"
        } else if n == "GT" {
            "gt"
        } else {
            n
        };
        let e = m.entry(a).or_insert((None, vec![]));
        if b == 0 {
            e.0 = Some(n);
        } else if let Some(x) = e.1.iter_mut().find(|x| x.0 == b) {
            x.1 = n;
        } else {
            e.1.push((b, n));
        }
    }
    for v in m.values_mut() {
        v.1.sort_by_key(|x| x.0);
    }
    m
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| Tables {
        dec4: HTML401.iter().map(|&(n, a, b)| (n, (a, b))).collect(),
        dec5: HTML5.iter().map(|&(n, a, b)| (n, (a, b))).collect(),
        enc4: build_enc(HTML401),
        enc5: build_enc(HTML5),
    })
}

const BASIC: &[(u32, &str)] = &[(0x22, "quot"), (0x26, "amp"), (0x3C, "lt"), (0x3E, "gt")];

fn basic_entity(cp: u32, doctype: i64) -> Option<&'static str> {
    if cp == 0x27 {
        return Some(if doctype == DOC_HTML401 { "#039" } else { "apos" });
    }
    BASIC.iter().find(|x| x.0 == cp).map(|x| x.1)
}

fn unicode_cp_is_allowed(cp: u32, doctype: i64) -> bool {
    match doctype {
        DOC_HTML401 => (0x20..=0x7E).contains(&cp) || cp == 0x0A || cp == 0x09 || cp == 0x0D || (0xA0..=0xD7FF).contains(&cp) || (0xE000..=0x10FFFF).contains(&cp),
        DOC_HTML5 => {
            (0x20..=0x7E).contains(&cp)
                || ((0x09..=0x0D).contains(&cp) && cp != 0x0B)
                || (0xA0..=0xD7FF).contains(&cp)
                || ((0xE000..=0x10FFFF).contains(&cp) && (cp & 0xFFFF) < 0xFFFE && !(0xFDD0..=0xFDEF).contains(&cp))
        }
        DOC_XHTML | DOC_XML1 => (0x20..=0xD7FF).contains(&cp) || cp == 0x0A || cp == 0x09 || cp == 0x0D || ((0xE000..=0x10FFFF).contains(&cp) && cp != 0xFFFE && cp != 0xFFFF),
        _ => true,
    }
}

fn numeric_entity_is_allowed(cp: u32, doctype: i64) -> bool {
    match doctype {
        DOC_HTML401 => cp <= 0x10FFFF,
        DOC_HTML5 => {
            (0x20..=0x7E).contains(&cp)
                || ((0x09..=0x0C).contains(&cp) && cp != 0x0B)
                || ((0xA0..=0x10FFFF).contains(&cp) && (cp & 0xFFFF) < 0xFFFE && !(0xFDD0..=0xFDEF).contains(&cp))
        }
        DOC_XHTML | DOC_XML1 => unicode_cp_is_allowed(cp, doctype),
        _ => true,
    }
}

/// process_numeric_entity: `buf` points after '#'. On success returns (code point, index of ';').
fn process_numeric_entity(b: &[u8], mut i: usize) -> Option<(u32, usize)> {
    let g = |i: usize| b.get(i).copied().unwrap_or(0);
    let hex = g(i) == b'x' || g(i) == b'X';
    if hex {
        i += 1;
    }
    if (hex && !g(i).is_ascii_hexdigit()) || (!hex && !g(i).is_ascii_digit()) {
        return None;
    }
    let start = i;
    while if hex { g(i).is_ascii_hexdigit() } else { g(i).is_ascii_digit() } {
        i += 1;
    }
    let s = std::str::from_utf8(&b[start..i]).ok()?;
    let v = i64::from_str_radix(s, if hex { 16 } else { 10 }).unwrap_or(i64::MAX);
    if g(i) != b';' {
        return None;
    }
    if v > 0x10FFFF {
        return None;
    }
    Some((v as u32, i))
}

/// process_named_entity_html: returns (name end index) if a valid-looking name followed by ';'
fn process_named_entity(b: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i < b.len() && (b[i].is_ascii_alphanumeric()) {
        i += 1;
    }
    if i == start || i >= b.len() || b[i] != b';' {
        return None;
    }
    Some(i)
}

fn decode_map(all: bool, flags: i64) -> &'static HashMap<&'static str, (u32, u32)> {
    // basic maps are subsets of the html4 map for the names we care about; build on demand
    static BE_APOS: OnceLock<HashMap<&'static str, (u32, u32)>> = OnceLock::new();
    static BE_NOAPOS: OnceLock<HashMap<&'static str, (u32, u32)>> = OnceLock::new();
    let t = tables();
    let doc = flags & DOC_MASK;
    if all {
        match doc {
            DOC_HTML401 | DOC_XHTML => &t.dec4,
            DOC_HTML5 => &t.dec5,
            _ => BE_APOS.get_or_init(|| [("quot", (0x22, 0)), ("amp", (0x26, 0)), ("apos", (0x27, 0)), ("lt", (0x3C, 0)), ("gt", (0x3E, 0))].into_iter().collect()),
        }
    } else if doc == DOC_HTML401 {
        BE_NOAPOS.get_or_init(|| [("quot", (0x22, 0)), ("amp", (0x26, 0)), ("#039", (0x27, 0)), ("lt", (0x3C, 0)), ("gt", (0x3E, 0))].into_iter().collect())
    } else {
        BE_APOS.get_or_init(|| [("quot", (0x22, 0)), ("amp", (0x26, 0)), ("apos", (0x27, 0)), ("lt", (0x3C, 0)), ("gt", (0x3E, 0))].into_iter().collect())
    }
}

fn push_cp(out: &mut Vec<u8>, cp: u32) {
    match char::from_u32(cp) {
        Some(c) => {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
        None => {
            // surrogate code point: encode as 3-byte sequence like php_utf32_utf8
            out.push(0xE0 | (cp >> 12) as u8);
            out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
            out.push(0x80 | (cp & 0x3F) as u8);
        }
    }
}

fn unescape(input: &[u8], all: bool, flags: i64) -> Vec<u8> {
    let inv = decode_map(all, flags);
    let doctype = flags & DOC_MASK;
    let mut out = Vec::with_capacity(input.len());
    let mut p = 0usize;
    let end = input.len();
    while p < end {
        let Some(off) = input[p..].iter().position(|&c| c == b'&') else {
            out.extend_from_slice(&input[p..]);
            break;
        };
        out.extend_from_slice(&input[p..p + off]);
        p += off;
        if end - p < 4 {
            out.extend_from_slice(&input[p..]);
            break;
        }
        let (mut code, mut code2) = (0u32, 0u32);
        let entity_end;
        let mut invalid_incomplete = false;
        if input[p + 1] == b'#' {
            match process_numeric_entity(input, p + 2) {
                None => {
                    invalid_incomplete = true;
                    entity_end = 0;
                }
                Some((c, e)) => {
                    code = c;
                    entity_end = e;
                    if !all && !matches!(code, 0x22 | 0x26 | 0x27 | 0x3C | 0x3E) {
                        invalid_incomplete = true;
                    } else if !unicode_cp_is_allowed(code, doctype) || (doctype == DOC_HTML5 && code == 0x0D) {
                        invalid_incomplete = true;
                    }
                }
            }
        } else {
            let name_start = p + 1;
            let limit = name_start + LONGEST_ENTITY_LENGTH.min(end - name_start - 1);
            let mut semi = name_start;
            while semi < limit && (input[semi].is_ascii_alphanumeric()) {
                semi += 1;
            }
            if input.get(semi) != Some(&b';') || semi == name_start {
                invalid_incomplete = true;
                entity_end = 0;
            } else {
                let name = std::str::from_utf8(&input[name_start..semi]).unwrap_or("");
                match inv.get(name) {
                    Some(&(a, b)) => {
                        code = a;
                        code2 = b;
                    }
                    None => {
                        if doctype == DOC_XHTML && name == "apos" {
                            code = b'\'' as u32;
                        } else {
                            invalid_incomplete = true;
                        }
                    }
                }
                entity_end = semi;
            }
        }
        if invalid_incomplete {
            out.push(input[p]);
            p += 1;
            continue;
        }
        if (code == b'\'' as u32 && flags & Q_SINGLE == 0) || (code == b'"' as u32 && flags & Q_DOUBLE == 0) {
            out.extend_from_slice(&input[p..entity_end]);
            p = entity_end;
            continue;
        }
        push_cp(&mut out, code);
        if code2 != 0 {
            push_cp(&mut out, code2);
        }
        p = entity_end + 1;
    }
    out
}

fn find_entity(all: bool, doctype: i64, cp: u32, next: Option<u32>) -> (Option<&'static str>, bool) {
    // returns (entity, consumed next char)
    if !all {
        return (basic_entity(cp, doctype), false);
    }
    let t = tables();
    let m = if doctype == DOC_HTML5 { &t.enc5 } else { &t.enc4 };
    match m.get(&cp) {
        None => (None, false),
        Some((def, multi)) => {
            if multi.is_empty() {
                return (*def, false);
            }
            if let Some(n) = next {
                if let Some(x) = multi.iter().find(|x| x.0 == n) {
                    return (Some(x.1), true);
                }
            }
            (*def, false)
        }
    }
}

fn escape(input: &str, all: bool, flags: i64, double_encode: bool) -> Vec<u8> {
    let doctype = flags & DOC_MASK;
    let inv = if double_encode { None } else { Some(decode_map(true, flags)) };
    let b = input.as_bytes();
    let chars: Vec<char> = input.chars().collect();
    let mut out: Vec<u8> = Vec::with_capacity(b.len() + 16);
    let mut bpos = 0usize; // byte position of chars[i]
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let cp = c as u32;
        let clen = c.len_utf8();
        i += 1;
        let mbseq = &b[bpos..bpos + clen];
        bpos += clen;
        if cp != b'&' as u32 {
            if (cp == b'\'' as u32 && flags & Q_SINGLE == 0) || (cp == b'"' as u32 && flags & Q_DOUBLE == 0) {
                out.extend_from_slice(mbseq);
                continue;
            }
            let next = chars.get(i).map(|c| *c as u32);
            let (rep, consumed) = find_entity(all, doctype, cp, next);
            if let Some(r) = rep {
                out.push(b'&');
                out.extend_from_slice(r.as_bytes());
                out.push(b';');
                if consumed {
                    bpos += chars[i].len_utf8();
                    i += 1;
                }
            } else if flags & SUBST_DISALLOWED != 0 && !unicode_cp_is_allowed(cp, doctype) {
                out.extend_from_slice("\u{FFFD}".as_bytes());
            } else {
                out.extend_from_slice(mbseq);
            }
        } else if double_encode {
            out.extend_from_slice(b"&amp;");
        } else {
            let enc_amp = |out: &mut Vec<u8>| out.extend_from_slice(b"&amp;");
            let nb = b.get(bpos).copied().unwrap_or(0);
            let ent_len;
            if nb == b'#' {
                match process_numeric_entity(b, bpos + 1) {
                    None => {
                        enc_amp(&mut out);
                        continue;
                    }
                    Some((code, e)) => {
                        if flags & SUBST_DISALLOWED != 0 && !numeric_entity_is_allowed(code, doctype) {
                            enc_amp(&mut out);
                            continue;
                        }
                        ent_len = e - bpos;
                    }
                }
            } else {
                match process_named_entity(b, bpos) {
                    None => {
                        enc_amp(&mut out);
                        continue;
                    }
                    Some(e) => {
                        let name = std::str::from_utf8(&b[bpos..e]).unwrap_or("");
                        if !inv.unwrap().contains_key(name) && !(doctype == DOC_XHTML && name == "apos") {
                            enc_amp(&mut out);
                            continue;
                        }
                        ent_len = e - bpos;
                    }
                }
            }
            out.push(b'&');
            out.extend_from_slice(&b[bpos..bpos + ent_len]);
            out.push(b';');
            // advance by ent_len + 1 bytes (entity chars are ASCII)
            bpos += ent_len + 1;
            i += ent_len + 1;
        }
    }
    out
}

fn html_entities(vm: &mut Vm<'_>, f: &str, a: Vec<Value>, all: bool) -> R<Value> {
    argc(vm, f, &a, 1, 4)?;
    let s = p_str(vm, f, &a, 0, "string")?;
    let flags = p_int_opt(vm, f, &a, 1, "flags", ENT_DEFAULT)?;
    let hint = match a.get(2).map(|v| v.deref()) {
        None | Some(Value::Null) => None,
        _ => Some(p_str(vm, f, &a, 2, "encoding")?),
    };
    let double_encode = p_bool(&a, 3, true);
    if s.is_empty() {
        return Ok(vstr(""));
    }
    let cs = determine_charset(vm, f, hint.as_deref(), false);
    let mut all = all;
    if all && cs == Cs::Partial {
        notice(vm, &format!("{f}(): Only basic entities substitution is supported for multi-byte encodings other than UTF-8; functionality is equivalent to htmlspecialchars"));
    }
    all = all && cs != Cs::Partial && (flags & DOC_MASK) != DOC_XML1;
    Ok(from_bytes(&escape(&s, all, flags, double_encode)))
}

fn htmlspecialchars_decode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "htmlspecialchars_decode", &a, 1, 2)?;
    let s = p_str(vm, "htmlspecialchars_decode", &a, 0, "string")?;
    let qs = p_int_opt(vm, "htmlspecialchars_decode", &a, 1, "flags", ENT_DEFAULT)?;
    if !s.contains('&') {
        return Ok(Value::Str(s));
    }
    Ok(from_bytes(&unescape(s.as_bytes(), false, qs)))
}

fn html_entity_decode(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "html_entity_decode", &a, 1, 3)?;
    let s = p_str(vm, "html_entity_decode", &a, 0, "string")?;
    let qs = p_int_opt(vm, "html_entity_decode", &a, 1, "flags", ENT_DEFAULT)?;
    let hint = match a.get(2).map(|v| v.deref()) {
        None | Some(Value::Null) => None,
        _ => Some(p_str(vm, "html_entity_decode", &a, 2, "encoding")?),
    };
    if !s.contains('&') {
        return Ok(Value::Str(s));
    }
    determine_charset(vm, "html_entity_decode", hint.as_deref(), false);
    Ok(from_bytes(&unescape(s.as_bytes(), true, qs)))
}

fn get_html_translation_table(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "get_html_translation_table", &a, 0, 3)?;
    let mut all = p_int_opt(vm, "get_html_translation_table", &a, 0, "table", 0)? != 0;
    let flags = p_int_opt(vm, "get_html_translation_table", &a, 1, "flags", ENT_DEFAULT)?;
    let hint = if a.len() > 2 { Some(p_str(vm, "get_html_translation_table", &a, 2, "encoding")?) } else { None };
    let cs = determine_charset(vm, "get_html_translation_table", hint.as_deref(), false);
    let doctype = flags & DOC_MASK;
    all = all && cs != Cs::Partial && doctype != DOC_XML1;
    let mut r = PhpArray::new();
    let skip = |cp: u32| (cp == b'\'' as u32 && flags & Q_SINGLE == 0) || (cp == b'"' as u32 && flags & Q_DOUBLE == 0);
    let key = |cp: u32, cp2: u32| {
        let mut v = Vec::new();
        push_cp(&mut v, cp);
        if cp2 != 0 {
            push_cp(&mut v, cp2);
        }
        String::from_utf8_lossy(&v).to_string()
    };
    if all {
        let t = tables();
        let m = if doctype == DOC_HTML5 { &t.enc5 } else { &t.enc4 };
        for (&cp, (def, multi)) in m {
            if skip(cp) {
                continue;
            }
            if let Some(d) = def {
                r.set(Key::from_str(&key(cp, 0)), vstr(&format!("&{d};")));
            }
            for (cp2, n) in multi {
                r.set(Key::from_str(&key(cp, *cp2)), vstr(&format!("&{n};")));
            }
        }
    } else {
        for cp in 0u32..64 {
            if skip(cp) {
                continue;
            }
            if let Some(n) = basic_entity(cp, doctype) {
                r.set(Key::from_str(&key(cp, 0)), vstr(&format!("&{n};")));
            }
        }
    }
    Ok(Value::Array(Rc::new(r)))
}

pub fn register(m: &mut HashMap<&'static str, VmBuiltin>) {
    m.insert("htmlspecialchars", |vm, a| html_entities(vm, "htmlspecialchars", a, false));
    m.insert("htmlentities", |vm, a| html_entities(vm, "htmlentities", a, true));
    m.insert("htmlspecialchars_decode", htmlspecialchars_decode);
    m.insert("html_entity_decode", html_entity_decode);
    m.insert("get_html_translation_table", get_html_translation_table);
}
