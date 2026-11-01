//! Port of str_getcsv (string.c) and php_fgetcsv (file.c, stream == NULL case).
use super::core1::ve;
use super::util::*;
use crate::builtins::VmBuiltin;
use crate::value::*;
use crate::vm::Vm;
use std::collections::HashMap;

/// php_fgetcsv_lookup_trailing_spaces: strips one trailing line ending.
fn lookup_trailing(b: &[u8]) -> usize {
    let n = b.len();
    let l1 = b.last().copied().unwrap_or(0);
    let l0 = if n >= 2 { b[n - 2] } else { 0 };
    match l1 {
        b'\n' => {
            if l0 == b'\r' {
                n - 2
            } else {
                n - 1
            }
        }
        b'\r' => n - 1,
        _ => n,
    }
}

fn isspace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// Returns None for a blank line. `escape` None means no escape character.
pub fn fgetcsv_str(buf: &[u8], delimiter: u8, enclosure: u8, escape: Option<u8>) -> Option<Vec<Vec<u8>>> {
    let g = |i: usize| buf.get(i).copied().unwrap_or(0);
    let limit = lookup_trailing(buf);
    let line_end = limit;
    let line_end_len = buf.len() - limit;
    let mut bptr = 0usize;
    let mut first_field = true;
    let mut values: Vec<Vec<u8>> = Vec::new();
    let sl = |a: usize, b: usize| -> &[u8] {
        let b = b.min(buf.len());
        let a = a.min(b);
        &buf[a..b]
    };
    loop {
        let mut temp: Vec<u8> = Vec::new();
        let mut inc_len: usize = if bptr < limit { 1 } else { 0 };
        if inc_len == 1 {
            let mut tmp = bptr;
            while g(tmp) != delimiter && isspace(g(tmp)) {
                tmp += 1;
            }
            if g(tmp) == enclosure && tmp < limit {
                bptr = tmp;
            }
        }
        if first_field && bptr == line_end {
            return None;
        }
        first_field = false;
        let comp_end;
        if inc_len != 0 && g(bptr) == enclosure {
            let mut state = 0;
            bptr += 1;
            let mut hunk_begin = bptr;
            'q2: loop {
                if inc_len == 0 {
                    match state {
                        2 => {
                            temp.extend_from_slice(sl(hunk_begin, bptr - 1));
                            hunk_begin = bptr;
                            break 'q2;
                        }
                        _ => {
                            if state == 1 {
                                temp.extend_from_slice(sl(hunk_begin, bptr));
                                hunk_begin = bptr;
                            }
                            if hunk_begin != line_end {
                                temp.extend_from_slice(sl(hunk_begin, bptr));
                                hunk_begin = bptr;
                            }
                            temp.extend_from_slice(sl(line_end, line_end + line_end_len));
                            if bptr > limit {
                                if hunk_begin == bptr {
                                    hunk_begin -= 1;
                                }
                                bptr -= 1;
                            }
                            break 'q2;
                        }
                    }
                } else {
                    match state {
                        1 => {
                            bptr += 1;
                            state = 0;
                        }
                        2 => {
                            if g(bptr) != enclosure {
                                temp.extend_from_slice(sl(hunk_begin, bptr - 1));
                                hunk_begin = bptr;
                                break 'q2;
                            }
                            temp.extend_from_slice(sl(hunk_begin, bptr));
                            bptr += 1;
                            hunk_begin = bptr;
                            state = 0;
                        }
                        _ => {
                            if g(bptr) == enclosure {
                                state = 2;
                            } else if escape.is_some_and(|e| e == g(bptr)) {
                                state = 1;
                            }
                            bptr += 1;
                        }
                    }
                }
                inc_len = if bptr < limit { 1 } else { 0 };
            }
            // look up for a delimiter
            loop {
                if inc_len == 0 || g(bptr) == delimiter {
                    break;
                }
                bptr += inc_len;
                inc_len = if bptr < limit { 1 } else { 0 };
            }
            temp.extend_from_slice(sl(hunk_begin, bptr));
            bptr += inc_len;
            comp_end = temp.len();
        } else {
            let hunk_begin = bptr;
            loop {
                if inc_len == 0 || g(bptr) == delimiter {
                    break;
                }
                bptr += inc_len;
                inc_len = if bptr < limit { 1 } else { 0 };
            }
            temp.extend_from_slice(sl(hunk_begin, bptr));
            comp_end = lookup_trailing(&temp);
            if g(bptr) == delimiter {
                bptr += 1;
            }
        }
        temp.truncate(comp_end);
        values.push(temp);
        if inc_len == 0 {
            break;
        }
    }
    Some(values)
}

fn str_getcsv(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "str_getcsv", &a, 1, 4)?;
    let s = p_str(vm, "str_getcsv", &a, 0, "string")?;
    let mut delimiter = b',';
    let mut enclosure = b'"';
    if a.len() > 1 {
        let d = p_str(vm, "str_getcsv", &a, 1, "separator")?;
        if d.len() != 1 {
            return ve(vm, "str_getcsv", 2, "separator", "must be a single character");
        }
        delimiter = d.as_bytes()[0];
    }
    if a.len() > 2 {
        let e = p_str(vm, "str_getcsv", &a, 2, "enclosure")?;
        if e.len() != 1 {
            return ve(vm, "str_getcsv", 3, "enclosure", "must be a single character");
        }
        enclosure = e.as_bytes()[0];
    }
    let escape: Option<u8> = if a.len() > 3 {
        let e = p_str(vm, "str_getcsv", &a, 3, "escape")?;
        if e.len() > 1 {
            return ve(vm, "str_getcsv", 4, "escape", "must be empty or a single character");
        }
        e.as_bytes().first().copied()
    } else {
        deprecated(vm, "str_getcsv(): the $escape parameter must be provided as its default value will change");
        Some(b'\\')
    };
    Ok(match fgetcsv_str(s.as_bytes(), delimiter, enclosure, escape) {
        Some(v) => list(v.iter().map(|b| from_bytes(b)).collect()),
        None => list(vec![Value::Null]),
    })
}

pub fn register(m: &mut HashMap<&'static str, VmBuiltin>) {
    m.insert("str_getcsv", str_getcsv);
}
