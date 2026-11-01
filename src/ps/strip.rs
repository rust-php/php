//! Port of strip_tags (string.c php_strip_tags_ex / php_tag_find).
use super::util::*;
use crate::builtins::VmBuiltin;
use crate::value::*;
use crate::vm::Vm;
use std::collections::HashMap;

fn tag_find(tag: &[u8], set: &[u8]) -> bool {
    if tag.is_empty() {
        return false;
    }
    let g = |i: usize| tag.get(i).copied().unwrap_or(0);
    let mut norm: Vec<u8> = Vec::new();
    let mut t = 0usize;
    let mut state = 0;
    let mut done = false;
    let mut c = g(t).to_ascii_lowercase();
    while !done {
        match c {
            b'<' => norm.push(c),
            b'>' => done = true,
            _ => {
                if !c.is_ascii_whitespace() && c != 0x0b {
                    if state == 0 {
                        state = 1;
                    }
                    if c != b'/' || (g(t.wrapping_sub(1)) != b'<' && g(t + 1) != b'>') {
                        norm.push(c);
                    }
                } else if state == 1 {
                    done = true;
                }
            }
        }
        t += 1;
        c = g(t).to_ascii_lowercase();
    }
    norm.push(b'>');
    set.windows(norm.len()).any(|w| w == &norm[..])
}

fn isspace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

pub fn strip_tags_ex(buf: &[u8], allow: Option<&[u8]>, allow_tag_spaces: bool) -> Vec<u8> {
    let end = buf.len();
    let g = |i: usize| buf.get(i).copied().unwrap_or(0);
    let allow_l: Option<Vec<u8>> = allow.map(|a| a.to_ascii_lowercase());
    let mut tbuf: Vec<u8> = Vec::new();
    let mut rp: Vec<u8> = Vec::new();
    let (mut br, mut depth, mut in_q): (i32, i32, u8) = (0, 0, 0);
    let mut state = 0u8;
    let mut is_xml = false;
    let mut lc = 0u8;
    let mut p = 0usize;
    while p < end {
        let c = buf[p];
        match state {
            0 => {
                match c {
                    0 => {}
                    b'<' => {
                        if in_q != 0 {
                        } else if isspace(g(p + 1)) && !allow_tag_spaces {
                            rp.push(c);
                        } else {
                            lc = b'<';
                            state = 1;
                            if allow_l.is_some() {
                                tbuf.push(b'<');
                            }
                        }
                    }
                    b'>' => {
                        if depth > 0 {
                            depth -= 1;
                        } else if in_q != 0 {
                        } else {
                            rp.push(c);
                        }
                    }
                    _ => rp.push(c),
                }
                p += 1;
            }
            1 => {
                let mut reg = false;
                match c {
                    0 => {}
                    b'<' => {
                        if in_q != 0 {
                        } else if isspace(g(p + 1)) && !allow_tag_spaces {
                            reg = true;
                        } else {
                            depth += 1;
                        }
                    }
                    b'>' => {
                        if depth > 0 {
                            depth -= 1;
                        } else if in_q != 0 {
                        } else {
                            lc = b'>';
                            if is_xml && p >= 1 && buf[p - 1] == b'-' {
                            } else {
                                in_q = 0;
                                state = 0;
                                is_xml = false;
                                if let Some(al) = &allow_l {
                                    tbuf.push(b'>');
                                    if tag_find(&tbuf, al) {
                                        rp.extend_from_slice(&tbuf);
                                    }
                                    tbuf.clear();
                                }
                            }
                        }
                    }
                    b'"' | b'\'' => {
                        if p != 0 && (in_q == 0 || c == in_q) {
                            in_q = if in_q != 0 { 0 } else { c };
                        }
                        reg = true;
                    }
                    b'!' => {
                        if p >= 1 && buf[p - 1] == b'<' {
                            state = 3;
                            lc = c;
                        } else {
                            reg = true;
                        }
                    }
                    b'?' => {
                        if p >= 1 && buf[p - 1] == b'<' {
                            br = 0;
                            state = 2;
                        } else {
                            reg = true;
                        }
                    }
                    _ => reg = true,
                }
                if reg && allow_l.is_some() {
                    tbuf.push(c);
                }
                p += 1;
            }
            2 => {
                match c {
                    b'(' => {
                        if lc != b'"' && lc != b'\'' {
                            lc = b'(';
                            br += 1;
                        }
                    }
                    b')' => {
                        if lc != b'"' && lc != b'\'' {
                            lc = b')';
                            br -= 1;
                        }
                    }
                    b'>' => {
                        if depth > 0 {
                            depth -= 1;
                        } else if in_q != 0 {
                        } else if br == 0 && p >= 1 && lc != b'"' && buf[p - 1] == b'?' {
                            in_q = 0;
                            state = 0;
                            tbuf.clear();
                        }
                    }
                    b'"' | b'\'' => {
                        if p >= 1 && buf[p - 1] != b'\\' {
                            if lc == c {
                                lc = 0;
                            } else if lc != b'\\' {
                                lc = c;
                            }
                            if p != 0 && (in_q == 0 || c == in_q) {
                                in_q = if in_q != 0 { 0 } else { c };
                            }
                        }
                    }
                    b'l' | b'L' => {
                        if state == 2 && p > 4 && matches!(buf[p - 1], b'm' | b'M') && matches!(buf[p - 2], b'x' | b'X') && buf[p - 3] == b'?' && buf[p - 4] == b'<' {
                            state = 1;
                            is_xml = true;
                        }
                    }
                    _ => {}
                }
                p += 1;
            }
            3 => {
                match c {
                    b'>' => {
                        if depth > 0 {
                            depth -= 1;
                        } else if in_q != 0 {
                        } else {
                            in_q = 0;
                            state = 0;
                            tbuf.clear();
                        }
                    }
                    b'"' | b'\'' => {
                        if p != 0 && buf[p - 1] != b'\\' && (in_q == 0 || c == in_q) {
                            in_q = if in_q != 0 { 0 } else { c };
                        }
                    }
                    b'-' => {
                        if p >= 2 && buf[p - 1] == b'-' && buf[p - 2] == b'!' {
                            state = 4;
                        }
                    }
                    b'E' | b'e' => {
                        if p > 6
                            && matches!(buf[p - 1], b'p' | b'P')
                            && matches!(buf[p - 2], b'y' | b'Y')
                            && matches!(buf[p - 3], b't' | b'T')
                            && matches!(buf[p - 4], b'c' | b'C')
                            && matches!(buf[p - 5], b'o' | b'O')
                            && matches!(buf[p - 6], b'd' | b'D')
                        {
                            state = 1;
                        }
                    }
                    _ => {}
                }
                p += 1;
            }
            _ => {
                // state 4: comment
                if c == b'>' && in_q == 0 && p >= 2 && buf[p - 1] == b'-' && buf[p - 2] == b'-' {
                    in_q = 0;
                    state = 0;
                    tbuf.clear();
                }
                p += 1;
            }
        }
    }
    rp
}

fn strip_tags(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "strip_tags", &a, 1, 2)?;
    let s = p_str(vm, "strip_tags", &a, 0, "string")?;
    let allow: Option<String> = match a.get(1).map(|v| v.deref()) {
        None | Some(Value::Null) => None,
        Some(Value::Array(x)) => {
            let mut t = String::new();
            for (_, v) in x.iter() {
                t.push('<');
                t.push_str(&v.deref().to_str());
                t.push('>');
            }
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        }
        Some(_) => Some(p_str(vm, "strip_tags", &a, 1, "allowed_tags")?.to_string()),
    };
    Ok(from_bytes(&strip_tags_ex(s.as_bytes(), allow.as_deref().map(|x| x.as_bytes()), false)))
}

pub fn register(m: &mut HashMap<&'static str, VmBuiltin>) {
    m.insert("strip_tags", strip_tags);
}
