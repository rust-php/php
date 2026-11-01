//! Port of number_format (ext/standard/math.c) and the rounding helper it needs.
use super::util::*;
use crate::builtins::VmBuiltin;
use crate::value::*;
use crate::vm::Vm;
use std::collections::HashMap;

fn intpow10(power: i32) -> f64 {
    if !(0..=22).contains(&power) {
        return 10f64.powf(power as f64);
    }
    const P: [f64; 23] = [1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22];
    P[power as usize]
}

/// _php_math_round with PHP_ROUND_HALF_UP
pub fn math_round_half_up(value: f64, places: i32) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    if places == 0 && value == value.trunc() {
        return value;
    }
    let places = places.max(i32::MIN + 1);
    let exponent = intpow10(places.abs());
    let (mut tmp, tmp2);
    if value >= 0.0 {
        tmp = (if places > 0 { value * exponent } else { value / exponent }).floor();
        tmp2 = tmp + 1.0;
    } else {
        tmp = (if places > 0 { value * exponent } else { value / exponent }).ceil();
        tmp2 = tmp - 1.0;
    }
    if (if places > 0 { tmp2 / exponent } else { tmp2 * exponent }) == value {
        tmp = tmp2;
    }
    if tmp.abs() >= 1e16 {
        return value;
    }
    // php_round_helper, HALF_UP
    let value_abs = value.abs();
    let edge = if places > 0 { ((tmp + 0.5f64.copysign(tmp)) / exponent).abs() } else { ((tmp + 0.5f64.copysign(tmp)) * exponent).abs() };
    if value_abs >= edge {
        tmp += 1.0f64.copysign(tmp);
    }
    if places.abs() < 23 {
        if places > 0 {
            tmp /= exponent;
        } else {
            tmp *= exponent;
        }
    } else {
        let s = format!("{:15.6}e{}", tmp, -places);
        tmp = s.trim().parse().unwrap_or(f64::NAN);
        if !tmp.is_finite() {
            tmp = value;
        }
    }
    tmp
}

fn group_digits(int_part: &str, sep: &str) -> String {
    let b = int_part.as_bytes();
    let mut out = String::new();
    for (i, c) in b.iter().enumerate() {
        if i > 0 && (b.len() - i) % 3 == 0 {
            out.push_str(sep);
        }
        out.push(*c as char);
    }
    out
}

pub fn number_format_ex(d: f64, dec: i32, dec_point: &str, sep: &str) -> String {
    let mut is_negative = false;
    let mut d = d;
    if d < 0.0 {
        is_negative = true;
        d = -d;
    }
    d = math_round_half_up(d, dec);
    let dec = dec.max(0);
    if d.is_nan() {
        return "nan".into();
    }
    if d.is_infinite() {
        return "inf".into();
    }
    let tmp = format!("{:.*}", dec.min(318) as usize, d);
    if is_negative && d == 0.0 {
        is_negative = false;
    }
    let (ip, fp) = match tmp.split_once('.') {
        Some((i, f)) if dec > 0 => (i.to_string(), f.to_string()),
        _ => (tmp.clone(), String::new()),
    };
    let mut res = String::new();
    if is_negative {
        res.push('-');
    }
    res.push_str(&group_digits(&ip, sep));
    if dec > 0 {
        res.push_str(dec_point);
        let mut f = fp;
        while (f.len() as i32) < dec {
            f.push('0');
        }
        res.push_str(&f);
    }
    res
}

fn number_format_long(num: i64, dec: i64, dec_point: &str, sep: &str) -> String {
    const POWERS: [u64; 20] = [
        1, 10, 100, 1000, 10000, 100000, 1000000, 10000000, 100000000, 1000000000, 10000000000, 100000000000, 1000000000000, 10000000000000, 100000000000000, 1000000000000000, 10000000000000000,
        100000000000000000, 1000000000000000000, 10000000000000000000,
    ];
    let mut is_negative = num < 0;
    let mut tmpnum: u64 = num.unsigned_abs();
    if dec < 0 {
        if dec < -(POWERS.len() as i64 - 1) {
            tmpnum = 0;
        } else {
            let power = POWERS[(-dec) as usize];
            let half = power / 2;
            let rest = tmpnum % power;
            tmpnum /= power;
            tmpnum = if rest >= half { tmpnum.wrapping_mul(power).wrapping_add(power) } else { tmpnum.wrapping_mul(power) };
        }
        if tmpnum == 0 {
            is_negative = false;
        }
    }
    let mut res = String::new();
    if is_negative {
        res.push('-');
    }
    res.push_str(&group_digits(&tmpnum.to_string(), sep));
    if dec > 0 {
        res.push_str(dec_point);
        res.push_str(&"0".repeat(dec as usize));
    }
    res
}

fn number_format(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    argc(vm, "number_format", &a, 1, 4)?;
    let num = match a[0].deref() {
        v @ (Value::Int(_) | Value::Float(_)) => v,
        Value::Bool(b) => Value::Int(b as i64),
        Value::Null => {
            deprecated(vm, "number_format(): Passing null to parameter #1 ($num) of type int|float is deprecated");
            Value::Int(0)
        }
        Value::Str(s) => match parse_numeric(&s) {
            Some((Num::I(i), _)) => Value::Int(i),
            Some((Num::F(f), _)) => Value::Float(f),
            None => return vm.throw_error("TypeError", "number_format(): Argument #1 ($num) must be of type int|float, string given"),
        },
        o => return vm.throw_error("TypeError", &format!("number_format(): Argument #1 ($num) must be of type int|float, {} given", zname(&o))),
    };
    let dec = p_int_opt(vm, "number_format", &a, 1, "decimals", 0)?;
    let dp: std::rc::Rc<str> = match a.get(2).map(|v| v.deref()) {
        None | Some(Value::Null) => ".".into(),
        _ => p_str(vm, "number_format", &a, 2, "decimal_separator")?,
    };
    let ts: std::rc::Rc<str> = match a.get(3).map(|v| v.deref()) {
        None | Some(Value::Null) => ",".into(),
        _ => p_str(vm, "number_format", &a, 3, "thousands_separator")?,
    };
    if dec > i32::MAX as i64 || dec < i32::MIN as i64 {
        return verr(vm, &format!("number_format(): Argument #2 ($decimals) must be between {} and {}", i32::MIN, i32::MAX));
    }
    Ok(vstr(&match num {
        Value::Int(n) => number_format_long(n, dec, &dp, &ts),
        Value::Float(f) => {
            if (f >= 4503599627370496.0 || f <= -4503599627370496.0) && f >= -9.2233720368547758e18 && f < 9.2233720368547758e18 {
                number_format_long(f as i64, dec, &dp, &ts)
            } else {
                number_format_ex(f, dec as i32, &dp, &ts)
            }
        }
        _ => unreachable!(),
    }))
}

pub fn register(m: &mut HashMap<&'static str, VmBuiltin>) {
    m.insert("number_format", number_format);
}
