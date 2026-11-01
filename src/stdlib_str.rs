//! String, ctype, mbstring (UTF-8) and hashing builtins.
use crate::builtins::{arg, arr, s, Builtin, Res};
use crate::value::*;
use std::{cmp::Ordering, collections::HashMap, rc::Rc};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sgn(o: Ordering) -> Value {
    Value::Int(o as i64)
}

fn st(a: &[Value], i: usize) -> Rc<str> {
    arg(a, i).to_str()
}

/// Compare strings the way `strnatcmp` does: digit runs compare numerically.
fn natcmp(a: &str, b: &str, ci: bool) -> Ordering {
    let (a, b): (Vec<char>, Vec<char>) = if ci {
        (a.to_ascii_lowercase().chars().collect(), b.to_ascii_lowercase().chars().collect())
    } else {
        (a.chars().collect(), b.chars().collect())
    };
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let x: String = a[si..i].iter().collect::<String>().trim_start_matches('0').to_string();
            let y: String = b[sj..j].iter().collect::<String>().trim_start_matches('0').to_string();
            let o = x.len().cmp(&y.len()).then(x.cmp(&y));
            if o != Ordering::Equal {
                return o;
            }
        } else {
            if a[i] != b[j] {
                return a[i].cmp(&b[j]);
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

fn ctype(a: &[Value], f: fn(char) -> bool) -> Res {
    Ok(Value::Bool(match arg(a, 0) {
        Value::Str(t) => !t.is_empty() && t.chars().all(f),
        _ => false,
    }))
}

fn html_escape(t: &str, quotes: bool) -> String {
    let mut out = String::with_capacity(t.len());
    for c in t.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' if quotes => out.push_str("&#039;"),
            c => out.push(c),
        }
    }
    out
}

fn urlenc(t: &str, raw: bool) -> String {
    let mut out = String::new();
    for b in t.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(b as char),
            b'~' if raw => out.push('~'),
            b' ' if !raw => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn urldec(t: &str, raw: bool) -> String {
    let b = t.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' if !raw => out.push(b' '),
            b'%' if i + 2 < b.len() + 0 && b.get(i + 1).is_some_and(u8::is_ascii_hexdigit) && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit) => {
                out.push(u8::from_str_radix(&t[i + 1..i + 3], 16).unwrap_or(b'%'));
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64enc(d: &[u8]) -> String {
    let mut out = String::new();
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

fn b64dec(t: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in t.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let v = B64.iter().position(|x| *x == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn hash_hex(algo: &str, data: &[u8]) -> Option<String> {
    Some(match algo {
        "md5" => {
            use md5::Digest;
            hex(&md5::Md5::digest(data))
        }
        "sha1" => {
            use sha1::Digest;
            hex(&sha1::Sha1::digest(data))
        }
        "sha256" => {
            use sha2::Digest;
            hex(&sha2::Sha256::digest(data))
        }
        "sha512" => {
            use sha2::Digest;
            hex(&sha2::Sha512::digest(data))
        }
        "crc32b" => format!("{:08x}", crc32fast::hash(data)),
        _ => return None,
    })
}

fn levenshtein(a: &str, b: &str) -> i64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()] as i64
}

fn similar(a: &[char], b: &[char]) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let (mut best, mut pa, mut pb) = (0, 0, 0);
    for i in 0..a.len() {
        for j in 0..b.len() {
            let mut k = 0;
            while i + k < a.len() && j + k < b.len() && a[i + k] == b[j + k] {
                k += 1;
            }
            if k > best {
                (best, pa, pb) = (k, i, j);
            }
        }
    }
    if best == 0 {
        return 0;
    }
    best + similar(&a[..pa], &b[..pb]) + similar(&a[pa + best..], &b[pb + best..])
}

fn wordwrap(text: &str, width: usize, brk: &str, cut: bool) -> String {
    let mut out = String::new();
    let mut line_len = 0;
    for (wi, word) in text.split(' ').enumerate() {
        let mut word = word.to_string();
        if wi > 0 {
            if line_len + 1 + word.chars().count() <= width {
                out.push(' ');
                line_len += 1;
            } else {
                out.push_str(brk);
                line_len = 0;
            }
        }
        while cut && word.chars().count() > width && width > 0 {
            let head: String = word.chars().take(width).collect();
            out.push_str(&head);
            out.push_str(brk);
            word = word.chars().skip(width).collect();
        }
        out.push_str(&word);
        line_len += word.chars().count();
        if let Some(i) = word.rfind('\n') {
            line_len = word[i + 1..].chars().count();
        }
    }
    out
}

fn mb_pos(h: &str, n: &str, off: i64, last: bool, ci: bool) -> Value {
    let (h, n) = if ci { (h.to_lowercase(), n.to_lowercase()) } else { (h.to_string(), n.to_string()) };
    let hc: Vec<char> = h.chars().collect();
    let start = if off < 0 { (hc.len() as i64 + off).max(0) as usize } else { off as usize };
    if start > hc.len() {
        return Value::Bool(false);
    }
    let tail: String = hc[start..].iter().collect();
    let hit = if last { tail.rfind(&n) } else { tail.find(&n) };
    match hit {
        Some(b) => Value::Int((start + tail[..b].chars().count()) as i64),
        None => Value::Bool(false),
    }
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let list: &[(&str, Builtin)] = &[
        ("strcmp", |_, a| Ok(sgn(st(&a, 0).as_bytes().cmp(st(&a, 1).as_bytes())))),
        ("strcasecmp", |_, a| Ok(sgn(st(&a, 0).to_ascii_lowercase().cmp(&st(&a, 1).to_ascii_lowercase())))),
        ("strncmp", |_, a| {
            let n = arg(&a, 2).to_int().max(0) as usize;
            let (x, y) = (st(&a, 0), st(&a, 1));
            Ok(sgn(x.as_bytes()[..n.min(x.len())].cmp(&y.as_bytes()[..n.min(y.len())])))
        }),
        ("strncasecmp", |_, a| {
            let n = arg(&a, 2).to_int().max(0) as usize;
            let (x, y) = (st(&a, 0).to_ascii_lowercase(), st(&a, 1).to_ascii_lowercase());
            Ok(sgn(x.as_bytes()[..n.min(x.len())].cmp(&y.as_bytes()[..n.min(y.len())])))
        }),
        ("strnatcmp", |_, a| Ok(sgn(natcmp(&st(&a, 0), &st(&a, 1), false)))),
        ("strnatcasecmp", |_, a| Ok(sgn(natcmp(&st(&a, 0), &st(&a, 1), true)))),
        ("strstr", |_, a| {
            let (h, n) = (st(&a, 0), st(&a, 1));
            Ok(match h.find(&*n) {
                Some(i) if arg(&a, 2).to_bool() => s(&h[..i]),
                Some(i) => s(&h[i..]),
                None => Value::Bool(false),
            })
        }),
        ("stristr", |_, a| {
            let (h, n) = (st(&a, 0), st(&a, 1));
            Ok(match h.to_ascii_lowercase().find(&n.to_ascii_lowercase()) {
                Some(i) if arg(&a, 2).to_bool() => s(&h[..i]),
                Some(i) => s(&h[i..]),
                None => Value::Bool(false),
            })
        }),
        ("strrchr", |_, a| {
            let (h, n) = (st(&a, 0), st(&a, 1));
            Ok(match n.chars().next().and_then(|c| h.rfind(c)) {
                Some(i) => s(&h[i..]),
                None => Value::Bool(false),
            })
        }),
        ("substr_count", |_, a| {
            let n = st(&a, 1);
            if n.is_empty() {
                return Err("substr_count(): Argument #2 ($needle) cannot be empty".into());
            }
            Ok(Value::Int(st(&a, 0).matches(&*n).count() as i64))
        }),
        ("substr_replace", |_, a| {
            let t: Vec<char> = st(&a, 0).chars().collect();
            let n = t.len() as i64;
            let mut start = arg(&a, 2).to_int();
            if start < 0 {
                start = (n + start).max(0);
            }
            let start = start.min(n);
            let end = match a.get(3) {
                None | Some(Value::Null) => n,
                Some(l) => {
                    let l = l.to_int();
                    if l < 0 { (n + l).max(start) } else { (start + l).min(n) }
                }
            };
            let out: String = t[..start as usize].iter().collect::<String>() + &st(&a, 1) + &t[end as usize..].iter().collect::<String>();
            Ok(s(&out))
        }),
        ("wordwrap", |_, a| {
            let width = a.get(1).map(Value::to_int).unwrap_or(75).max(0) as usize;
            let brk = a.get(2).map(|v| v.to_str()).unwrap_or_else(|| "\n".into());
            Ok(s(&wordwrap(&st(&a, 0), width, &brk, arg(&a, 3).to_bool())))
        }),
        ("chunk_split", |_, a| {
            let n = a.get(1).map(Value::to_int).unwrap_or(76).max(1) as usize;
            let end = a.get(2).map(|v| v.to_str()).unwrap_or_else(|| "\r\n".into());
            let chars: Vec<char> = st(&a, 0).chars().collect();
            Ok(s(&chars.chunks(n).map(|c| c.iter().collect::<String>() + &end).collect::<String>()))
        }),
        ("str_word_count", |_, a| {
            let t = st(&a, 0);
            let words: Vec<&str> = t.split(|c: char| !(c.is_alphabetic() || c == '\'' || c == '-')).filter(|w| !w.is_empty()).collect();
            Ok(match arg(&a, 1).to_int() {
                0 => Value::Int(words.len() as i64),
                _ => arr(words.into_iter().map(s).collect()),
            })
        }),
        ("strtr", |_, a| {
            let t = st(&a, 0);
            if let Some(Value::Array(pairs)) = a.get(1) {
                let mut subs: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.to_value().to_str().to_string(), v.to_str().to_string())).filter(|(k, _)| !k.is_empty()).collect();
                subs.sort_by(|x, y| y.0.len().cmp(&x.0.len()));
                let mut out = String::new();
                let mut i = 0;
                'outer: while i < t.len() {
                    for (k, v) in &subs {
                        if t[i..].starts_with(k.as_str()) {
                            out.push_str(v);
                            i += k.len();
                            continue 'outer;
                        }
                    }
                    let c = t[i..].chars().next().unwrap();
                    out.push(c);
                    i += c.len_utf8();
                }
                return Ok(s(&out));
            }
            let (from, to): (Vec<char>, Vec<char>) = (st(&a, 1).chars().collect(), st(&a, 2).chars().collect());
            let n = from.len().min(to.len());
            Ok(s(&t.chars().map(|c| from[..n].iter().rposition(|f| *f == c).map(|i| to[i]).unwrap_or(c)).collect::<String>()))
        }),
        ("nl2br", |_, a| Ok(s(&st(&a, 0).replace("\r\n", "<br />\r\n").replace('\n', "<br />\n").replace("<br />\r<br />\n", "<br />\r\n")))),
        ("htmlspecialchars", |_, a| Ok(s(&html_escape(&st(&a, 0), arg(&a, 1).to_int() & 3 != 0 || a.len() < 2)))),
        ("htmlentities", |_, a| Ok(s(&html_escape(&st(&a, 0), true)))),
        ("htmlspecialchars_decode", |_, a| Ok(s(&st(&a, 0).replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#039;", "'").replace("&amp;", "&")))),
        ("html_entity_decode", |_, a| Ok(s(&st(&a, 0).replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#039;", "'").replace("&amp;", "&")))),
        ("addslashes", |_, a| Ok(s(&st(&a, 0).replace('\\', "\\\\").replace('\'', "\\'").replace('"', "\\\"").replace('\0', "\\0")))),
        ("stripslashes", |_, a| {
            let mut out = String::new();
            let text = st(&a, 0);
            let mut it = text.chars();
            while let Some(c) = it.next() {
                if c == '\\' {
                    if let Some(n) = it.next() {
                        out.push(if n == '0' { '\0' } else { n });
                    }
                } else {
                    out.push(c);
                }
            }
            Ok(s(&out))
        }),
        ("md5", |_, a| Ok(s(&hash_hex("md5", st(&a, 0).as_bytes()).unwrap()))),
        ("sha1", |_, a| Ok(s(&hash_hex("sha1", st(&a, 0).as_bytes()).unwrap()))),
        ("crc32", |_, a| Ok(Value::Int(crc32fast::hash(st(&a, 0).as_bytes()) as i64))),
        ("hash", |_, a| match hash_hex(&st(&a, 0).to_ascii_lowercase(), st(&a, 1).as_bytes()) {
            Some(h) => Ok(s(&h)),
            None => Err(format!("hash(): Argument #1 ($algo) must be a valid hashing algorithm")),
        }),
        ("hash_algos", |_, _| Ok(arr(["md5", "sha1", "sha256", "sha512", "crc32b"].into_iter().map(s).collect()))),
        ("base64_encode", |_, a| Ok(s(&b64enc(st(&a, 0).as_bytes())))),
        ("base64_decode", |_, a| Ok(b64dec(&st(&a, 0)).map(|b| s(&String::from_utf8_lossy(&b))).unwrap_or(Value::Bool(false)))),
        ("bin2hex", |_, a| Ok(s(&hex(st(&a, 0).as_bytes())))),
        ("hex2bin", |_, a| {
            let t = st(&a, 0);
            if t.len() % 2 != 0 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Ok(Value::Bool(false));
            }
            let b: Vec<u8> = (0..t.len()).step_by(2).map(|i| u8::from_str_radix(&t[i..i + 2], 16).unwrap()).collect();
            Ok(s(&String::from_utf8_lossy(&b)))
        }),
        ("urlencode", |_, a| Ok(s(&urlenc(&st(&a, 0), false)))),
        ("rawurlencode", |_, a| Ok(s(&urlenc(&st(&a, 0), true)))),
        ("urldecode", |_, a| Ok(s(&urldec(&st(&a, 0), false)))),
        ("rawurldecode", |_, a| Ok(s(&urldec(&st(&a, 0), true)))),
        ("http_build_query", |_, a| {
            let Value::Array(x) = arg(&a, 0) else { return Ok(s("")) };
            Ok(s(&x.iter().map(|(k, v)| format!("{}={}", urlenc(&k.to_value().to_str(), false), urlenc(&v.to_str(), false))).collect::<Vec<_>>().join("&")))
        }),
        ("uniqid", |_, a| {
            let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            Ok(s(&format!("{}{:08x}{:05x}", st(&a, 0), d.as_secs(), d.subsec_micros())))
        }),
        ("levenshtein", |_, a| Ok(Value::Int(levenshtein(&st(&a, 0), &st(&a, 1))))),
        ("similar_text", |_, a| {
            let (x, y): (Vec<char>, Vec<char>) = (st(&a, 0).chars().collect(), st(&a, 1).chars().collect());
            Ok(Value::Int(similar(&x, &y) as i64))
        }),
        ("strpbrk", |_, a| {
            let (t, set) = (st(&a, 0), st(&a, 1));
            Ok(match t.find(|c| set.contains(c)) {
                Some(i) => s(&t[i..]),
                None => Value::Bool(false),
            })
        }),
        ("strspn", |_, a| {
            let (t, set) = (st(&a, 0), st(&a, 1));
            Ok(Value::Int(t.chars().take_while(|c| set.contains(*c)).map(char::len_utf8).sum::<usize>() as i64))
        }),
        ("strcspn", |_, a| {
            let (t, set) = (st(&a, 0), st(&a, 1));
            Ok(Value::Int(t.chars().take_while(|c| !set.contains(*c)).map(char::len_utf8).sum::<usize>() as i64))
        }),
        ("substr_compare", |_, a| {
            let (h, n) = (st(&a, 0), st(&a, 1));
            let off = arg(&a, 2).to_int();
            let start = if off < 0 { (h.len() as i64 + off).max(0) as usize } else { (off as usize).min(h.len()) };
            let mut hs = h[start..].to_string();
            let mut ns = n.to_string();
            if let Some(l) = a.get(3).filter(|v| !matches!(v, Value::Null)) {
                let l = l.to_int().max(0) as usize;
                hs.truncate(l);
                ns.truncate(l);
            }
            if arg(&a, 4).to_bool() {
                hs = hs.to_ascii_lowercase();
                ns = ns.to_ascii_lowercase();
            }
            Ok(sgn(hs.cmp(&ns)))
        }),
        ("ctype_alpha", |_, a| ctype(&a, |c| c.is_ascii_alphabetic())),
        ("ctype_digit", |_, a| ctype(&a, |c| c.is_ascii_digit())),
        ("ctype_alnum", |_, a| ctype(&a, |c| c.is_ascii_alphanumeric())),
        ("ctype_upper", |_, a| ctype(&a, |c| c.is_ascii_uppercase())),
        ("ctype_lower", |_, a| ctype(&a, |c| c.is_ascii_lowercase())),
        ("ctype_space", |_, a| ctype(&a, |c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c'))),
        ("ctype_punct", |_, a| ctype(&a, |c| c.is_ascii_punctuation())),
        ("ctype_xdigit", |_, a| ctype(&a, |c| c.is_ascii_hexdigit())),
        ("ctype_cntrl", |_, a| ctype(&a, |c| c.is_ascii_control())),
        ("ctype_print", |_, a| ctype(&a, |c| c.is_ascii_graphic() || c == ' ')),
        ("ctype_graph", |_, a| ctype(&a, |c| c.is_ascii_graphic())),
        ("mb_strlen", |_, a| Ok(Value::Int(st(&a, 0).chars().count() as i64))),
        ("mb_substr", |_, a| {
            let t: Vec<char> = st(&a, 0).chars().collect();
            let n = t.len() as i64;
            let mut start = arg(&a, 1).to_int();
            if start < 0 {
                start = (n + start).max(0);
            }
            let start = start.min(n);
            let end = match a.get(2) {
                None | Some(Value::Null) => n,
                Some(l) => {
                    let l = l.to_int();
                    if l < 0 { (n + l).max(start) } else { (start + l).min(n) }
                }
            };
            Ok(s(&t[start as usize..end as usize].iter().collect::<String>()))
        }),
        ("mb_strtoupper", |_, a| Ok(s(&st(&a, 0).to_uppercase()))),
        ("mb_strtolower", |_, a| Ok(s(&st(&a, 0).to_lowercase()))),
        ("mb_convert_case", |_, a| {
            let t = st(&a, 0);
            Ok(s(&match arg(&a, 1).to_int() {
                0 => t.to_uppercase(),
                1 => t.to_lowercase(),
                _ => t.split(' ').map(|w| { let mut c = w.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase()).unwrap_or_default() }).collect::<Vec<_>>().join(" "),
            }))
        }),
        ("mb_str_split", |_, a| {
            let n = a.get(1).map(Value::to_int).unwrap_or(1).max(1) as usize;
            let chars: Vec<char> = st(&a, 0).chars().collect();
            Ok(arr(chars.chunks(n).map(|c| s(&c.iter().collect::<String>())).collect()))
        }),
        ("mb_strpos", |_, a| Ok(mb_pos(&st(&a, 0), &st(&a, 1), arg(&a, 2).to_int(), false, false))),
        ("mb_stripos", |_, a| Ok(mb_pos(&st(&a, 0), &st(&a, 1), arg(&a, 2).to_int(), false, true))),
        ("mb_strrpos", |_, a| Ok(mb_pos(&st(&a, 0), &st(&a, 1), arg(&a, 2).to_int(), true, false))),
        ("mb_substr_count", |_, a| Ok(Value::Int(st(&a, 0).matches(&*st(&a, 1)).count() as i64))),
        ("mb_strrev", |_, a| Ok(s(&st(&a, 0).chars().rev().collect::<String>()))),
        ("mb_strwidth", |_, a| Ok(Value::Int(st(&a, 0).chars().map(|c| if (c as u32) >= 0x1100 && !c.is_ascii() { 2 } else { 1 }).sum()))),
        ("mb_internal_encoding", |_, a| Ok(if a.is_empty() { s("UTF-8") } else { Value::Bool(true) })),
        ("mb_check_encoding", |_, _| Ok(Value::Bool(true))),
        ("mb_detect_encoding", |_, _| Ok(s("UTF-8"))),
        ("mb_str_pad", |_, a| {
            let (t, len) = (st(&a, 0), arg(&a, 1).to_int().max(0) as usize);
            let pad: Vec<char> = a.get(2).map(|p| p.to_str().chars().collect()).unwrap_or_else(|| vec![' ']);
            let cur = t.chars().count();
            if len <= cur || pad.is_empty() {
                return Ok(Value::Str(t));
            }
            let fill = |n: usize| -> String { (0..n).map(|i| pad[i % pad.len()]).collect() };
            let total = len - cur;
            Ok(s(&match a.get(3).map(Value::to_int).unwrap_or(1) {
                0 => format!("{}{}", fill(total), t),
                2 => format!("{}{}{}", fill(total / 2), t, fill(total - total / 2)),
                _ => format!("{}{}", t, fill(total)),
            }))
        }),
        ("iconv_strlen", |_, a| Ok(Value::Int(st(&a, 0).chars().count() as i64))),
        ("vsprintf", |_, a| {
            let mut args = vec![arg(&a, 0)];
            if let Value::Array(x) = arg(&a, 1) {
                args.extend(x.iter().map(|(_, v)| v.clone()));
            }
            crate::builtins::sprintf(&args).map(|r| s(&r))
        }),
        ("lcg_value", |_, _| Ok(Value::Float(0.5))),
    ];
    for (k, f) in list {
        m.insert(k, *f);
    }
}
