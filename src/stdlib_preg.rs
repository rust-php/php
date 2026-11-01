//! preg_* functions on top of fancy-regex (PCRE-like syntax with backreferences and lookaround).
use crate::value::*;
use crate::vm::{Flow, Vm};
use fancy_regex::{Captures, Regex};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

type R<T> = Result<T, Flow>;

thread_local! {
    static CACHE: RefCell<HashMap<String, Rc<Regex>>> = RefCell::new(HashMap::new());
    static LAST_ERROR: RefCell<i64> = const { RefCell::new(0) };
}

pub fn last_error() -> i64 {
    LAST_ERROR.with(|e| *e.borrow())
}

fn set_error(code: i64) {
    LAST_ERROR.with(|e| *e.borrow_mut() = code);
}

/// Compile a PHP pattern (`/body/flags`) into a regex.
fn compile(pattern: &str) -> Result<Rc<Regex>, String> {
    if let Some(r) = CACHE.with(|c| c.borrow().get(pattern).cloned()) {
        return Ok(r);
    }
    let p = pattern.trim_start();
    let mut chars = p.chars();
    let delim = chars.next().ok_or("preg_match(): Empty regular expression")?;
    if delim.is_alphanumeric() || delim == '\\' {
        return Err("preg_match(): Delimiter must not be alphanumeric, backslash, or NUL".into());
    }
    let close = match delim {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        c => c,
    };
    let rest = &p[delim.len_utf8()..];
    let end = rest.rfind(close).ok_or_else(|| format!("preg_match(): No ending delimiter '{close}' found"))?;
    let (body, flags) = (&rest[..end], &rest[end + close.len_utf8()..]);
    let mut prefix = String::new();
    let mut anchored = false;
    let mut dollar_end = false;
    for f in flags.chars() {
        match f {
            'i' => prefix.push('i'),
            'm' => prefix.push('m'),
            's' => prefix.push('s'),
            'x' => prefix.push('x'),
            'U' => prefix.push('U'),
            'A' => anchored = true,
            'D' => dollar_end = true,
            'u' | 'S' | 'X' | 'J' | '\n' | '\r' | ' ' => {}
            other => return Err(format!("preg_match(): Unknown modifier '{other}'")),
        }
    }
    let _ = dollar_end;
    let mut src = String::new();
    if !prefix.is_empty() {
        src.push_str(&format!("(?{prefix})"));
    }
    if anchored {
        src.push_str("\\A(?:");
    }
    src.push_str(&translate(body));
    if anchored {
        src.push(')');
    }
    let re = Regex::new(&src).map_err(|e| format!("preg_match(): Compilation failed: {e}"))?;
    let re = Rc::new(re);
    CACHE.with(|c| c.borrow_mut().insert(pattern.to_string(), re.clone()));
    Ok(re)
}

/// Rewrite PCRE-only syntax into what fancy-regex accepts.
fn translate(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut it = body.chars().peekable();
    let mut in_class = false;
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('h') => out.push_str(if in_class { " \\t" } else { "[ \\t]" }),
                Some('R') => out.push_str("(?:\\r\\n|\\n|\\r)"),
                Some('A') => out.push_str("\\A"),
                Some('Z') => out.push_str("(?=\\n?\\z)"),
                Some('e') => out.push('\u{1b}'),
                Some('/') => out.push('/'),
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                }
                None => out.push_str("\\\\"),
            },
            '[' if !in_class => {
                in_class = true;
                out.push('[');
                if it.peek() == Some(&'^') {
                    out.push('^');
                    it.next();
                }
                if it.peek() == Some(&']') {
                    out.push_str("\\]");
                    it.next();
                }
            }
            '[' if in_class => {
                // POSIX class like [:alpha:] stays; a bare '[' must be escaped for Rust.
                if it.peek() == Some(&':') {
                    out.push('[');
                } else {
                    out.push_str("\\[");
                }
            }
            ']' if in_class => {
                in_class = false;
                out.push(']');
            }
            c => out.push(c),
        }
    }
    out
}

fn group_names(re: &Regex) -> Vec<Option<String>> {
    re.capture_names().map(|n| n.map(str::to_string)).collect()
}

/// Build the `$matches` array for one match.
fn match_array(re: &Regex, caps: &Captures<'_>, offset_capture: bool, unmatched_null: bool, trim_trailing: bool) -> PhpArray {
    let names = group_names(re);
    let mut last = caps.len();
    if trim_trailing && !unmatched_null {
        while last > 1 && caps.get(last - 1).is_none() {
            last -= 1;
        }
    }
    let mut out = PhpArray::new();
    for i in 0..last {
        let v = match caps.get(i) {
            Some(m) => {
                let s = Value::str(m.as_str());
                if offset_capture {
                    Value::Array(Rc::new(PhpArray::from_list(vec![s, Value::Int(m.start() as i64)])))
                } else {
                    s
                }
            }
            None => {
                let empty = if unmatched_null { Value::Null } else { Value::str("") };
                if offset_capture {
                    Value::Array(Rc::new(PhpArray::from_list(vec![empty, Value::Int(-1)])))
                } else {
                    empty
                }
            }
        };
        if let Some(Some(n)) = names.get(i) {
            out.set(Key::from_str(n), v.clone());
        }
        out.set(Key::Int(i as i64), v);
    }
    out
}

fn expand_replacement(rep: &str, caps: &Captures<'_>) -> String {
    let b: Vec<char> = rep.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if (c == '$' || c == '\\') && i + 1 < b.len() {
            let (mut j, braced) = if c == '$' && b[i + 1] == '{' { (i + 2, true) } else { (i + 1, false) };
            let st = j;
            while j < b.len() && b[j].is_ascii_digit() && j - st < 2 {
                j += 1;
            }
            if j > st && (!braced || b.get(j) == Some(&'}')) {
                let n: usize = b[st..j].iter().collect::<String>().parse().unwrap_or(0);
                if let Some(m) = caps.get(n) {
                    out.push_str(m.as_str());
                }
                i = if braced { j + 1 } else { j };
                continue;
            }
            if c == '\\' && b[i + 1] == '\\' {
                out.push('\\');
                i += 2;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn re_or_warn(vm: &mut Vm<'_>, pattern: &str) -> R<Option<Rc<Regex>>> {
    match compile(pattern) {
        Ok(r) => Ok(Some(r)),
        Err(msg) => {
            set_error(1);
            let _ = vm;
            // PHP emits a warning and returns false/null; keep going quietly.
            let _ = msg;
            Ok(None)
        }
    }
}

fn subject_list(v: &Value) -> Vec<(Key, Value)> {
    match v {
        Value::Array(a) => a.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        other => vec![(Key::Int(0), other.clone())],
    }
}

pub fn call(vm: &mut Vm<'_>, name: &str, args: &mut Vec<Value>) -> Option<R<Value>> {
    let pat = args.first().cloned().unwrap_or(Value::Null);
    match name {
        "preg_match" | "preg_match_all" => {
            let all = name == "preg_match_all";
            let subject = args.get(1).map(|v| v.to_str()).unwrap_or_else(|| "".into());
            let flags = args.get(3).map(Value::to_int).unwrap_or(0);
            let offset = args.get(4).map(Value::to_int).unwrap_or(0).max(0) as usize;
            let offset = offset.min(subject.len());
            let re = match re_or_warn(vm, &pat.to_str()) {
                Ok(Some(r)) => r,
                Ok(None) => return Some(Ok(Value::Bool(false))),
                Err(e) => return Some(Err(e)),
            };
            set_error(0);
            let (offcap, unnull) = (flags & 256 != 0, flags & 512 != 0);
            while args.len() < 3 {
                args.push(Value::Null);
            }
            if !all {
                return Some(Ok(match re.captures_from_pos(&subject, offset) {
                    Ok(Some(c)) => {
                        args[2] = Value::Array(Rc::new(match_array(&re, &c, offcap, unnull, true)));
                        Value::Int(1)
                    }
                    Ok(None) => {
                        args[2] = Value::Array(Rc::new(PhpArray::new()));
                        Value::Int(0)
                    }
                    Err(_) => {
                        set_error(2);
                        Value::Bool(false)
                    }
                }));
            }
            let set_order = flags & 2 != 0;
            let mut rows: Vec<PhpArray> = vec![];
            let mut pos = offset;
            while pos <= subject.len() {
                match re.captures_from_pos(&subject, pos) {
                    Ok(Some(c)) => {
                        let m = c.get(0).unwrap();
                        pos = if m.end() == m.start() { m.end() + subject[m.end()..].chars().next().map(char::len_utf8).unwrap_or(1) } else { m.end() };
                        rows.push(match_array(&re, &c, offcap, unnull, set_order));
                    }
                    _ => break,
                }
            }
            let count = rows.len() as i64;
            args[2] = if set_order {
                Value::Array(Rc::new(PhpArray::from_list(rows.into_iter().map(|r| Value::Array(Rc::new(r))).collect())))
            } else {
                let ngroups = re.captures_len();
                let names = group_names(&re);
                let mut out = PhpArray::new();
                for g in 0..ngroups {
                    let col: Vec<Value> = rows.iter().map(|r| r.get(&Key::Int(g as i64)).cloned().unwrap_or(Value::str(""))).collect();
                    let col = Value::Array(Rc::new(PhpArray::from_list(col)));
                    if let Some(Some(n)) = names.get(g) {
                        out.set(Key::from_str(n), col.clone());
                    }
                    out.set(Key::Int(g as i64), col);
                }
                Value::Array(Rc::new(out))
            };
            Some(Ok(Value::Int(count)))
        }
        "preg_replace" | "preg_replace_callback" => {
            let callback = name == "preg_replace_callback";
            let rep = args.get(1).cloned().unwrap_or(Value::Null);
            let subject = args.get(2).cloned().unwrap_or(Value::Null);
            let limit = args.get(3).map(Value::to_int).filter(|l| *l >= 0).unwrap_or(-1);
            let patterns: Vec<String> = match &pat {
                Value::Array(a) => a.iter().map(|(_, v)| v.to_str().to_string()).collect(),
                p => vec![p.to_str().to_string()],
            };
            let mut total = 0i64;
            let mut results = PhpArray::new();
            for (key, subj) in subject_list(&subject) {
                let mut text = subj.to_str().to_string();
                for (pi, p) in patterns.iter().enumerate() {
                    let re = match re_or_warn(vm, p) {
                        Ok(Some(r)) => r,
                        Ok(None) => return Some(Ok(Value::Null)),
                        Err(e) => return Some(Err(e)),
                    };
                    let rep_str = match &rep {
                        Value::Array(r) if !callback => r.iter().nth(pi).map(|(_, v)| v.to_str().to_string()).unwrap_or_default(),
                        other => other.to_str().to_string(),
                    };
                    let mut out = String::new();
                    let (mut last, mut pos, mut n) = (0usize, 0usize, 0i64);
                    while pos <= text.len() && (limit < 0 || n < limit) {
                        let caps = match re.captures_from_pos(&text, pos) {
                            Ok(Some(c)) => c,
                            _ => break,
                        };
                        let m = caps.get(0).unwrap();
                        out.push_str(&text[last..m.start()]);
                        if callback {
                            let arr = Value::Array(Rc::new(match_array(&re, &caps, false, false, true)));
                            match vm.call_value(&rep, vec![arr]) {
                                Ok(v) => out.push_str(&v.to_str()),
                                Err(e) => return Some(Err(e)),
                            }
                        } else {
                            out.push_str(&expand_replacement(&rep_str, &caps));
                        }
                        n += 1;
                        last = m.end();
                        pos = if m.end() == m.start() {
                            out.push_str(&text[m.end()..].chars().next().map(String::from).unwrap_or_default());
                            last = m.end() + text[m.end()..].chars().next().map(char::len_utf8).unwrap_or(1);
                            last
                        } else {
                            m.end()
                        };
                    }
                    out.push_str(&text[last.min(text.len())..]);
                    total += n;
                    text = out;
                }
                results.set(key, Value::str(&text));
            }
            while args.len() < 5 {
                args.push(Value::Null);
            }
            args[4] = Value::Int(total);
            set_error(0);
            Some(Ok(if matches!(subject, Value::Array(_)) { Value::Array(Rc::new(results)) } else { results.get(&Key::Int(0)).cloned().unwrap_or(Value::Null) }))
        }
        _ => None,
    }
}

/// Simple preg functions that do not write back arguments.
pub fn simple(name: &str, a: &[Value]) -> Option<Result<Value, String>> {
    let arg = |i: usize| a.get(i).cloned().unwrap_or(Value::Null);
    Some(match name {
        "preg_quote" => {
            let delim = a.get(1).map(|d| d.to_str().to_string()).unwrap_or_default();
            let mut out = String::new();
            for c in arg(0).to_str().chars() {
                if ".\\+*?[^]$(){}=!<>|:-#/".contains(c) && (c != '/' || delim.contains('/')) || delim.contains(c) {
                    out.push('\\');
                }
                if c == '\0' {
                    out.push_str("\\000");
                } else {
                    out.push(c);
                }
            }
            Ok(Value::str(&out))
        }
        "preg_split" => {
            let re = match compile(&arg(0).to_str()) {
                Ok(r) => r,
                Err(_) => return Some(Ok(Value::Bool(false))),
            };
            let subject = arg(1).to_str();
            let limit = a.get(2).map(Value::to_int).filter(|l| *l > 0).unwrap_or(-1);
            let flags = arg(3).to_int();
            let (no_empty, delim_capture, offset_capture) = (flags & 1 != 0, flags & 2 != 0, flags & 4 != 0);
            let mut parts: Vec<(String, usize)> = vec![];
            let (mut last, mut pos) = (0usize, 0usize);
            while pos <= subject.len() {
                if limit > 0 && parts.len() as i64 >= limit - 1 {
                    break;
                }
                let caps = match re.captures_from_pos(&subject, pos) {
                    Ok(Some(c)) => c,
                    _ => break,
                };
                let m = caps.get(0).unwrap();
                if m.end() == m.start() {
                    if m.start() >= subject.len() {
                        break;
                    }
                    if m.start() == last {
                        pos = m.start() + subject[m.start()..].chars().next().map(char::len_utf8).unwrap_or(1);
                        if pos > subject.len() {
                            break;
                        }
                        // split between characters
                        parts.push((subject[last..pos].to_string(), last));
                        last = pos;
                        continue;
                    }
                }
                parts.push((subject[last..m.start()].to_string(), last));
                if delim_capture {
                    for g in 1..caps.len() {
                        if let Some(gm) = caps.get(g) {
                            parts.push((gm.as_str().to_string(), gm.start()));
                        }
                    }
                }
                last = m.end();
                pos = if m.end() == m.start() { m.end() + 1 } else { m.end() };
            }
            parts.push((subject[last.min(subject.len())..].to_string(), last));
            let items: Vec<Value> = parts
                .into_iter()
                .filter(|(s, _)| !no_empty || !s.is_empty())
                .map(|(s, off)| if offset_capture { Value::Array(Rc::new(PhpArray::from_list(vec![Value::str(&s), Value::Int(off as i64)]))) } else { Value::str(&s) })
                .collect();
            Ok(Value::Array(Rc::new(PhpArray::from_list(items))))
        }
        "preg_grep" => {
            let re = match compile(&arg(0).to_str()) {
                Ok(r) => r,
                Err(_) => return Some(Ok(Value::Bool(false))),
            };
            let invert = arg(2).to_int() & 1 != 0;
            let mut out = PhpArray::new();
            if let Value::Array(items) = arg(1) {
                for (k, v) in items.iter() {
                    let hit = re.is_match(&v.to_str()).unwrap_or(false);
                    if hit != invert {
                        out.set(k.clone(), v.clone());
                    }
                }
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "preg_last_error" => Ok(Value::Int(last_error())),
        "preg_last_error_msg" => Ok(Value::str(match last_error() {
            0 => "No error",
            1 => "Internal error",
            2 => "Backtrack limit exhausted",
            _ => "Unknown error",
        })),
        _ => return None,
    })
}
