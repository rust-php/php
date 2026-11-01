//! Sort machinery of ext/standard/array.c (compare functions, natural order) and strnatcmp.c.
use crate::value::*;
use std::cmp::Ordering;

pub const SORT_REGULAR: i64 = 0;
pub const SORT_NUMERIC: i64 = 1;
pub const SORT_STRING: i64 = 2;
pub const SORT_DESC: i64 = 3;
pub const SORT_ASC: i64 = 4;
pub const SORT_LOCALE_STRING: i64 = 5;
pub const SORT_NATURAL: i64 = 6;
pub const SORT_FLAG_CASE: i64 = 8;

fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}
fn is_digit(c: u8) -> bool {
    c.is_ascii_digit()
}
fn is_space(c: u8) -> bool {
    c == b' ' || (9..=13).contains(&c)
}

fn compare_right(a: &[u8], ap: &mut usize, b: &[u8], bp: &mut usize) -> i32 {
    let mut bias = 0;
    loop {
        let a_end = *ap >= a.len() || !is_digit(a[*ap]);
        let b_end = *bp >= b.len() || !is_digit(b[*bp]);
        if a_end && b_end {
            return bias;
        } else if a_end {
            return -1;
        } else if b_end {
            return 1;
        } else if a[*ap] < b[*bp] {
            if bias == 0 {
                bias = -1;
            }
        } else if a[*ap] > b[*bp] && bias == 0 {
            bias = 1;
        }
        *ap += 1;
        *bp += 1;
    }
}

fn compare_left(a: &[u8], ap: &mut usize, b: &[u8], bp: &mut usize) -> i32 {
    loop {
        let a_end = *ap >= a.len() || !is_digit(a[*ap]);
        let b_end = *bp >= b.len() || !is_digit(b[*bp]);
        if a_end && b_end {
            return 0;
        } else if a_end {
            return -1;
        } else if b_end {
            return 1;
        } else if a[*ap] < b[*bp] {
            return -1;
        } else if a[*ap] > b[*bp] {
            return 1;
        }
        *ap += 1;
        *bp += 1;
    }
}

/// strnatcmp_ex
pub fn strnatcmp_ex(a: &[u8], b: &[u8], fold_case: bool) -> i32 {
    if a.is_empty() || b.is_empty() {
        return if a.len() == b.len() {
            0
        } else if a.len() > b.len() {
            1
        } else {
            -1
        };
    }
    let (mut ap, mut bp) = (0usize, 0usize);
    let (mut ca, mut cb) = (a[0], b[0]);
    while ca == b'0' && ap + 1 < a.len() && is_digit(a[ap + 1]) {
        ap += 1;
        ca = a[ap];
    }
    while cb == b'0' && bp + 1 < b.len() && is_digit(b[bp + 1]) {
        bp += 1;
        cb = b[bp];
    }
    loop {
        while is_space(ca) {
            ap += 1;
            ca = at(a, ap);
        }
        while is_space(cb) {
            bp += 1;
            cb = at(b, bp);
        }
        if is_digit(ca) && is_digit(cb) {
            let fractional = ca == b'0' || cb == b'0';
            let result = if fractional { compare_left(a, &mut ap, b, &mut bp) } else { compare_right(a, &mut ap, b, &mut bp) };
            if result != 0 {
                return result;
            } else if ap >= a.len() && bp >= b.len() {
                return 0;
            } else if ap >= a.len() {
                return -1;
            } else if bp >= b.len() {
                return 1;
            } else {
                ca = a[ap];
                cb = b[bp];
            }
        }
        if fold_case {
            ca = ca.to_ascii_uppercase();
            cb = cb.to_ascii_uppercase();
        }
        if ca < cb {
            return -1;
        } else if ca > cb {
            return 1;
        }
        ap += 1;
        bp += 1;
        if ap >= a.len() && bp >= b.len() {
            return 0;
        } else if ap >= a.len() {
            return -1;
        } else if bp >= b.len() {
            return 1;
        }
        ca = a[ap];
        cb = b[bp];
    }
}

fn ord(i: i32) -> Ordering {
    i.cmp(&0)
}

fn bin_strcasecmp(a: &str, b: &str) -> Ordering {
    let la = a.bytes().map(|c| c.to_ascii_lowercase());
    let lb = b.bytes().map(|c| c.to_ascii_lowercase());
    la.cmp(lb)
}

fn num_of(v: &Value) -> f64 {
    v.to_float()
}

pub type Cmp = Box<dyn Fn(&Value, &Value) -> Ordering>;

/// php_get_data_compare_func (unstable variant; callers use a stable sort).
pub fn data_cmp(sort_type: i64) -> Cmp {
    match sort_type & !SORT_FLAG_CASE {
        SORT_NUMERIC => Box::new(|a, b| num_of(a).partial_cmp(&num_of(b)).unwrap_or(Ordering::Equal)),
        SORT_STRING | SORT_LOCALE_STRING => {
            if sort_type & SORT_FLAG_CASE != 0 {
                Box::new(|a, b| bin_strcasecmp(&a.to_str(), &b.to_str()))
            } else {
                Box::new(|a, b| a.to_str().as_bytes().cmp(b.to_str().as_bytes()))
            }
        }
        SORT_NATURAL => {
            let fold = sort_type & SORT_FLAG_CASE != 0;
            Box::new(move |a, b| ord(strnatcmp_ex(a.to_str().as_bytes(), b.to_str().as_bytes(), fold)))
        }
        _ => Box::new(loose_cmp),
    }
}

/// php_get_key_compare_func: compares keys (as values).
pub fn key_cmp(sort_type: i64) -> Cmp {
    match sort_type & !SORT_FLAG_CASE {
        SORT_NUMERIC => Box::new(|a, b| match (a, b) {
            (Value::Int(x), Value::Int(y)) => x.cmp(y),
            _ => num_of(a).partial_cmp(&num_of(b)).unwrap_or(Ordering::Equal),
        }),
        _ => data_cmp(sort_type),
    }
}

/// Stable merge sort with a fallible comparator.
pub fn merge_sort<T: Clone, E>(mut v: Vec<T>, cmp: &mut dyn FnMut(&T, &T) -> Result<Ordering, E>) -> Result<Vec<T>, E> {
    let n = v.len();
    if n <= 1 {
        return Ok(v);
    }
    let right = v.split_off(n / 2);
    let l = merge_sort(v, cmp)?;
    let r = merge_sort(right, cmp)?;
    let mut out = Vec::with_capacity(n);
    let (mut i, mut j) = (0, 0);
    while i < l.len() && j < r.len() {
        if cmp(&r[j], &l[i])?.is_lt() {
            out.push(r[j].clone());
            j += 1;
        } else {
            out.push(l[i].clone());
            i += 1;
        }
    }
    out.extend_from_slice(&l[i..]);
    out.extend_from_slice(&r[j..]);
    Ok(out)
}
