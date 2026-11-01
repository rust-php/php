//! Port of ext/standard/math.c and the mt_rand/random_int entry points.
use super::args::*;
use super::mt;
use crate::builtins::VmBuiltin;
use crate::value::*;
use crate::vm::Vm;
use std::collections::HashMap;

const HALF_UP: i64 = 1;
const HALF_DOWN: i64 = 2;
const HALF_EVEN: i64 = 3;
const HALF_ODD: i64 = 4;
const CEILING: i64 = 5;
const FLOOR: i64 = 6;
const TOWARD_ZERO: i64 = 7;
const AWAY_FROM_ZERO: i64 = 8;

fn intpow10(power: i32) -> f64 {
    if !(0..=22).contains(&power) {
        return 10f64.powf(power as f64);
    }
    const P: [f64; 23] = [
        1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
    ];
    P[power as usize]
}

fn basic_edge_case(integral: f64, exponent: f64, places: i32) -> f64 {
    if places > 0 {
        ((integral + 0.5f64.copysign(integral)) / exponent).abs()
    } else {
        ((integral + 0.5f64.copysign(integral)) * exponent).abs()
    }
}

fn zero_edge_case(integral: f64, exponent: f64, places: i32) -> f64 {
    if places > 0 {
        (integral / exponent).abs()
    } else {
        (integral * exponent).abs()
    }
}

fn round_helper(integral: f64, value: f64, exponent: f64, places: i32, mode: i64) -> f64 {
    let value_abs = value.abs();
    match mode {
        HALF_UP => {
            if value_abs >= basic_edge_case(integral, exponent, places) {
                return integral + 1f64.copysign(integral);
            }
            integral
        }
        HALF_DOWN => {
            if value_abs > basic_edge_case(integral, exponent, places) {
                return integral + 1f64.copysign(integral);
            }
            integral
        }
        CEILING => {
            if value > 0.0 && value_abs > zero_edge_case(integral, exponent, places) {
                return integral + 1.0;
            }
            integral
        }
        FLOOR => {
            if value < 0.0 && value_abs > zero_edge_case(integral, exponent, places) {
                return integral - 1.0;
            }
            integral
        }
        TOWARD_ZERO => integral,
        AWAY_FROM_ZERO => {
            if value_abs > zero_edge_case(integral, exponent, places) {
                return integral + 1f64.copysign(integral);
            }
            integral
        }
        HALF_EVEN => {
            let edge = basic_edge_case(integral, exponent, places);
            if value_abs > edge {
                return integral + 1f64.copysign(integral);
            } else if value_abs == edge && integral % 2.0 != 0.0 {
                return integral + 1f64.copysign(integral);
            }
            integral
        }
        HALF_ODD => {
            let edge = basic_edge_case(integral, exponent, places);
            if value_abs > edge {
                return integral + 1f64.copysign(integral);
            } else if value_abs == edge && integral % 2.0 == 0.0 {
                return integral + 1f64.copysign(integral);
            }
            integral
        }
        _ => integral,
    }
}

/// _php_math_round
pub fn php_math_round(value: f64, places: i32, mode: i64) -> f64 {
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
    tmp = round_helper(tmp, value, exponent, places, mode);
    if places.abs() < 23 {
        if places > 0 {
            tmp /= exponent;
        } else {
            tmp *= exponent;
        }
    } else {
        let s = format!("{:.6}e{}", tmp, -(places as i64));
        match s.parse::<f64>() {
            Ok(x) if x.is_finite() => tmp = x,
            _ => tmp = value,
        }
    }
    tmp
}

fn num1(vm: &mut Vm<'_>, f: &str, a: &[Value], pname: &str) -> R<Value> {
    check_argc(vm, f, a, 1, 1)?;
    p_number(vm, f, 1, pname, &a[0])
}

fn dbl1(vm: &mut Vm<'_>, f: &str, a: &[Value], op: fn(f64) -> f64) -> R<Value> {
    check_argc(vm, f, a, 1, 1)?;
    Ok(Value::Float(op(p_double(vm, f, 1, "num", &a[0])?)))
}

fn dbl2(vm: &mut Vm<'_>, f: &str, a: &[Value], n1: &str, n2: &str, op: fn(f64, f64) -> f64) -> R<Value> {
    check_argc(vm, f, a, 2, 2)?;
    let x = p_double(vm, f, 1, n1, &a[0])?;
    let y = p_double(vm, f, 2, n2, &a[1])?;
    Ok(Value::Float(op(x, y)))
}

fn f_abs(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    Ok(match num1(vm, "abs", &a, "num")? {
        Value::Int(i) => {
            if i == i64::MIN {
                Value::Float(-(i as f64))
            } else {
                Value::Int(i.abs())
            }
        }
        Value::Float(x) => Value::Float(x.abs()),
        v => v,
    })
}

fn f_ceil(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    Ok(match num1(vm, "ceil", &a, "num")? {
        Value::Int(i) => Value::Float(i as f64),
        Value::Float(x) => Value::Float(x.ceil()),
        v => v,
    })
}

fn f_floor(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    Ok(match num1(vm, "floor", &a, "num")? {
        Value::Int(i) => Value::Float(i as f64),
        Value::Float(x) => Value::Float(x.floor()),
        v => v,
    })
}

fn f_round(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "round", &a, 1, 3)?;
    let value = p_number(vm, "round", 1, "num", &a[0])?;
    let mut places = 0i32;
    let mut mode = HALF_UP;
    if a.len() >= 2 {
        let p = p_long(vm, "round", 2, "precision", &a[1])?;
        places = p.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    }
    if a.len() >= 3 {
        mode = p_long(vm, "round", 3, "mode", &a[2])?;
    }
    if !(HALF_UP..=AWAY_FROM_ZERO).contains(&mode) {
        return value_err(vm, "round", 3, "mode", "must be a valid rounding mode (RoundingMode::*)");
    }
    match value {
        Value::Int(i) if places >= 0 => Ok(Value::Float(i as f64)),
        v => Ok(Value::Float(php_math_round(v.to_float(), places, mode))),
    }
}

fn f_log(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "log", &a, 1, 2)?;
    let num = p_double(vm, "log", 1, "num", &a[0])?;
    if a.len() == 1 {
        return Ok(Value::Float(num.ln()));
    }
    let base = p_double(vm, "log", 2, "base", &a[1])?;
    if base == 2.0 {
        return Ok(Value::Float(num.log2()));
    }
    if base == 10.0 {
        return Ok(Value::Float(num.log10()));
    }
    if base == 1.0 {
        return Ok(Value::Float(f64::NAN));
    }
    if base <= 0.0 {
        return value_err(vm, "log", 2, "base", "must be greater than 0");
    }
    Ok(Value::Float(num.ln() / base.ln()))
}

fn f_intdiv(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "intdiv", &a, 2, 2)?;
    let x = p_long(vm, "intdiv", 1, "num1", &a[0])?;
    let y = p_long(vm, "intdiv", 2, "num2", &a[1])?;
    if y == 0 {
        return vm.throw_error("DivisionByZeroError", "Division by zero");
    }
    if y == -1 && x == i64::MIN {
        return vm.throw_error("ArithmeticError", "Division of PHP_INT_MIN by -1 is not an integer");
    }
    Ok(Value::Int(x / y))
}

/// _php_math_basetozval
fn basetozval(s: &str, base: i64) -> Value {
    let b = s.as_bytes();
    let (mut st, mut e) = (0usize, b.len());
    let ws = |c: u8| c == b' ' || (9..=13).contains(&c);
    while st < e && ws(b[st]) {
        st += 1;
    }
    while st < e && ws(b[e - 1]) {
        e -= 1;
    }
    if e - st >= 2 && b[st] == b'0' {
        let c = b[st + 1];
        if (base == 16 && (c == b'x' || c == b'X')) || (base == 8 && (c == b'o' || c == b'O')) || (base == 2 && (c == b'b' || c == b'B')) {
            st += 2;
        }
    }
    let cutoff = i64::MAX / base;
    let cutlim = i64::MAX % base;
    let (mut num, mut fnum, mut mode) = (0i64, 0f64, 0);
    for &ch in &b[st..e] {
        let c = match ch {
            b'0'..=b'9' => (ch - b'0') as i64,
            b'A'..=b'Z' => (ch - b'A') as i64 + 10,
            b'a'..=b'z' => (ch - b'a') as i64 + 10,
            _ => continue,
        };
        if c >= base {
            continue;
        }
        if mode == 0 {
            if num < cutoff || (num == cutoff && c <= cutlim) {
                num = num * base + c;
                continue;
            }
            fnum = num as f64;
            mode = 1;
        }
        fnum = fnum * base as f64 + c as f64;
    }
    if mode == 1 {
        Value::Float(fnum)
    } else {
        Value::Int(num)
    }
}

const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn longtobase(arg: i64, base: u64) -> String {
    let mut v = arg as u64;
    let mut out = vec![];
    loop {
        out.push(DIGITS[(v % base) as usize]);
        v /= base;
        if v == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn zvaltobase(vm: &mut Vm<'_>, v: &Value, base: i64) -> R<Value> {
    match v {
        Value::Float(x) => {
            let mut fvalue = x.floor();
            if fvalue.is_infinite() {
                return vm.throw_error("ValueError", &format!("An infinite value cannot be converted to base {base}"));
            }
            let mut out = vec![];
            loop {
                out.push(DIGITS[(fvalue % base as f64) as i32 as usize]);
                fvalue /= base as f64;
                if fvalue.abs() < 1.0 || out.len() >= 1000 {
                    break;
                }
            }
            out.reverse();
            Ok(Value::str(&String::from_utf8(out).unwrap_or_default()))
        }
        Value::Int(i) => Ok(Value::str(&longtobase(*i, base as u64))),
        _ => Ok(Value::str("")),
    }
}

fn f_base_convert(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "base_convert", &a, 3, 3)?;
    let number = p_str(vm, "base_convert", 1, "num", &a[0])?;
    let from = p_long(vm, "base_convert", 2, "from_base", &a[1])?;
    let to = p_long(vm, "base_convert", 3, "to_base", &a[2])?;
    if !(2..=36).contains(&from) {
        return value_err(vm, "base_convert", 2, "from_base", "must be between 2 and 36 (inclusive)");
    }
    if !(2..=36).contains(&to) {
        return value_err(vm, "base_convert", 3, "to_base", "must be between 2 and 36 (inclusive)");
    }
    let t = basetozval(&number, from);
    zvaltobase(vm, &t, to)
}

fn dec_to_pwr2(vm: &mut Vm<'_>, f: &str, a: &[Value], base: u64) -> R<Value> {
    check_argc(vm, f, a, 1, 1)?;
    let n = p_long(vm, f, 1, "num", &a[0])?;
    Ok(Value::str(&longtobase(n, base)))
}

fn base_to_dec(vm: &mut Vm<'_>, f: &str, a: &[Value], base: i64) -> R<Value> {
    check_argc(vm, f, a, 1, 1)?;
    let s = p_str(vm, f, 1, if f == "bindec" { "binary_string" } else if f == "hexdec" { "hex_string" } else { "octal_string" }, &a[0])?;
    Ok(basetozval(&s, base))
}

fn group(digits: &str, sep: &str) -> String {
    let n = digits.len();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (n - i) % 3 == 0 {
            out.push_str(sep);
        }
        out.push(ch);
    }
    out
}

/// _php_math_number_format_ex
fn number_format_ex(d: f64, dec: i32, dp: &str, ts: &str) -> String {
    let mut d = d;
    let mut neg = false;
    if d < 0.0 {
        neg = true;
        d = -d;
    }
    d = php_math_round(d, dec, HALF_UP);
    let dec = dec.max(0) as usize;
    if d.is_nan() {
        return "NAN".into();
    }
    if d.is_infinite() {
        return "INF".into();
    }
    let tmp = format!("{:.*}", dec, d);
    if neg && d == 0.0 {
        neg = false;
    }
    let (ip, fp) = match tmp.split_once('.') {
        Some((i, f)) if dec > 0 => (i.to_string(), Some(f.to_string())),
        _ => (tmp.clone(), None),
    };
    let mut res = String::new();
    if neg {
        res.push('-');
    }
    res.push_str(&group(&ip, ts));
    if dec > 0 {
        res.push_str(dp);
        let f = fp.unwrap_or_default();
        res.push_str(&f);
        for _ in f.len()..dec {
            res.push('0');
        }
    }
    res
}

/// _php_math_number_format_long
fn number_format_long(num: i64, dec: i64, dp: &str, ts: &str) -> String {
    let mut neg = num < 0;
    let mut tmp: u64 = if num < 0 { ((-(num + 1)) as u64) + 1 } else { num as u64 };
    if dec < 0 {
        if dec < -19 {
            tmp = 0;
        } else {
            let power = 10u64.pow((-dec) as u32);
            let half = power / 2;
            let rest = tmp % power;
            let q = tmp / power;
            tmp = if rest >= half { q.wrapping_mul(power).wrapping_add(power) } else { q.wrapping_mul(power) };
        }
        if tmp == 0 {
            neg = false;
        }
    }
    let mut res = String::new();
    if neg {
        res.push('-');
    }
    res.push_str(&group(&tmp.to_string(), ts));
    if dec > 0 {
        res.push_str(dp);
        for _ in 0..dec {
            res.push('0');
        }
    }
    res
}

fn f_number_format(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "number_format", &a, 1, 4)?;
    let num = p_number(vm, "number_format", 1, "num", &a[0])?;
    let dec = if a.len() > 1 { p_long(vm, "number_format", 2, "decimals", &a[1])? } else { 0 };
    let dp: String = match a.get(2).map(Value::deref) {
        None | Some(Value::Null) => ".".into(),
        Some(v) => p_str(vm, "number_format", 3, "decimal_separator", &v)?.to_string(),
    };
    let ts: String = match a.get(3).map(Value::deref) {
        None | Some(Value::Null) => ",".into(),
        Some(v) => p_str(vm, "number_format", 4, "thousands_separator", &v)?.to_string(),
    };
    if dec > i32::MAX as i64 || dec < i32::MIN as i64 {
        return vm.throw_error("ValueError", &format!("number_format(): Argument #2 ($decimals) must be between {} and {}", i32::MIN, i32::MAX));
    }
    Ok(Value::str(&match num {
        Value::Int(i) => number_format_long(i, dec, &dp, &ts),
        Value::Float(x) => {
            if (x >= 4503599627370496.0 || x <= -4503599627370496.0) && x >= -9.223372036854775808e18 && x < 9.223372036854775808e18 {
                number_format_long(x as i64, dec, &dp, &ts)
            } else {
                number_format_ex(x, dec as i32, &dp, &ts)
            }
        }
        _ => String::new(),
    }))
}

fn f_mt_srand(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "mt_srand", &a, 0, 2)?;
    let seed = match a.first().map(Value::deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(p_long(vm, "mt_srand", 1, "seed", &v)?),
    };
    let mode = if a.len() > 1 { p_long(vm, "mt_srand", 2, "mode", &a[1])? } else { 0 };
    mt::seed(seed, mode == 1);
    Ok(Value::Null)
}

fn f_mt_rand(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    if a.is_empty() {
        return Ok(Value::Int(mt::mt_rand_31()));
    }
    check_argc(vm, "mt_rand", &a, 2, 2)?;
    let min = p_long(vm, "mt_rand", 1, "min", &a[0])?;
    let max = p_long(vm, "mt_rand", 2, "max", &a[1])?;
    if max < min {
        return value_err(vm, "mt_rand", 2, "max", "must be greater than or equal to argument #1 ($min)");
    }
    Ok(Value::Int(mt::mt_rand_common(min, max)))
}

fn f_rand(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    if a.is_empty() {
        return Ok(Value::Int(mt::mt_rand_31()));
    }
    check_argc(vm, "rand", &a, 2, 2)?;
    let min = p_long(vm, "rand", 1, "min", &a[0])?;
    let max = p_long(vm, "rand", 2, "max", &a[1])?;
    if max < min {
        return Ok(Value::Int(mt::mt_rand_common(max, min)));
    }
    Ok(Value::Int(mt::mt_rand_common(min, max)))
}

fn f_random_int(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "random_int", &a, 2, 2)?;
    let min = p_long(vm, "random_int", 1, "min", &a[0])?;
    let max = p_long(vm, "random_int", 2, "max", &a[1])?;
    if min > max {
        return value_err(vm, "random_int", 1, "min", "must be less than or equal to argument #2 ($max)");
    }
    Ok(Value::Int(mt::secure_range(min, max)))
}

fn f_getrandmax(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "mt_getrandmax", &a, 0, 0)?;
    Ok(Value::Int(2147483647))
}

pub fn constant(name: &str) -> Option<Value> {
    use std::f64::consts as c;
    let f = |x: f64| Some(Value::Float(x));
    let i = |x: i64| Some(Value::Int(x));
    match name {
        "M_E" => f(c::E),
        "M_LOG2E" => f(c::LOG2_E),
        "M_LOG10E" => f(c::LOG10_E),
        "M_LN2" => f(c::LN_2),
        "M_LN10" => f(c::LN_10),
        "M_PI" => f(c::PI),
        "M_PI_2" => f(c::FRAC_PI_2),
        "M_PI_4" => f(c::FRAC_PI_4),
        "M_1_PI" => f(c::FRAC_1_PI),
        "M_2_PI" => f(c::FRAC_2_PI),
        "M_2_SQRTPI" => f(c::FRAC_2_SQRT_PI),
        "M_SQRT2" => f(c::SQRT_2),
        "M_SQRT1_2" => f(c::FRAC_1_SQRT_2),
        "M_LNPI" => f(1.14472988584940017414),
        "M_EULER" => f(0.57721566490153286061),
        "M_SQRTPI" => f(1.77245385090551602729),
        "M_SQRT3" => f(1.73205080756887729352),
        "PHP_ROUND_HALF_UP" => i(HALF_UP),
        "PHP_ROUND_HALF_DOWN" => i(HALF_DOWN),
        "PHP_ROUND_HALF_EVEN" => i(HALF_EVEN),
        "PHP_ROUND_HALF_ODD" => i(HALF_ODD),
        "MT_RAND_MT19937" => i(0),
        "MT_RAND_PHP" => i(1),
        "PHP_FLOAT_DIG" => i(15),
        _ => None,
    }
}

pub fn register_vm(m: &mut HashMap<&'static str, VmBuiltin>) {
    let list: &[(&str, VmBuiltin)] = &[
        ("abs", f_abs),
        ("ceil", f_ceil),
        ("floor", f_floor),
        ("round", f_round),
        ("log", f_log),
        ("intdiv", f_intdiv),
        ("base_convert", f_base_convert),
        ("number_format", f_number_format),
        ("mt_srand", f_mt_srand),
        ("srand", f_mt_srand),
        ("mt_rand", f_mt_rand),
        ("rand", f_rand),
        ("random_int", f_random_int),
        ("mt_getrandmax", f_getrandmax),
        ("getrandmax", f_getrandmax),
        ("sin", |vm, a| dbl1(vm, "sin", &a, f64::sin)),
        ("cos", |vm, a| dbl1(vm, "cos", &a, f64::cos)),
        ("tan", |vm, a| dbl1(vm, "tan", &a, f64::tan)),
        ("asin", |vm, a| dbl1(vm, "asin", &a, f64::asin)),
        ("acos", |vm, a| dbl1(vm, "acos", &a, f64::acos)),
        ("atan", |vm, a| dbl1(vm, "atan", &a, f64::atan)),
        ("sinh", |vm, a| dbl1(vm, "sinh", &a, f64::sinh)),
        ("cosh", |vm, a| dbl1(vm, "cosh", &a, f64::cosh)),
        ("tanh", |vm, a| dbl1(vm, "tanh", &a, f64::tanh)),
        ("asinh", |vm, a| dbl1(vm, "asinh", &a, f64::asinh)),
        ("acosh", |vm, a| dbl1(vm, "acosh", &a, f64::acosh)),
        ("atanh", |vm, a| dbl1(vm, "atanh", &a, f64::atanh)),
        ("expm1", |vm, a| dbl1(vm, "expm1", &a, f64::exp_m1)),
        ("log1p", |vm, a| dbl1(vm, "log1p", &a, f64::ln_1p)),
        ("exp", |vm, a| dbl1(vm, "exp", &a, f64::exp)),
        ("log10", |vm, a| dbl1(vm, "log10", &a, f64::log10)),
        ("log2", |vm, a| dbl1(vm, "log2", &a, f64::log2)),
        ("sqrt", |vm, a| dbl1(vm, "sqrt", &a, f64::sqrt)),
        ("deg2rad", |vm, a| dbl1(vm, "deg2rad", &a, |x| (x / 180.0) * std::f64::consts::PI)),
        ("rad2deg", |vm, a| dbl1(vm, "rad2deg", &a, |x| (x / std::f64::consts::PI) * 180.0)),
        ("atan2", |vm, a| dbl2(vm, "atan2", &a, "y", "x", f64::atan2)),
        ("hypot", |vm, a| dbl2(vm, "hypot", &a, "x", "y", f64::hypot)),
        ("fmod", |vm, a| dbl2(vm, "fmod", &a, "num1", "num2", |x, y| x % y)),
        ("fdiv", |vm, a| dbl2(vm, "fdiv", &a, "num1", "num2", |x, y| x / y)),
        ("fpow", |vm, a| dbl2(vm, "fpow", &a, "num", "exponent", f64::powf)),
        ("is_nan", |vm, a| dbl1(vm, "is_nan", &a, |x| x).map(|v| Value::Bool(v.to_float().is_nan()))),
        ("is_finite", |vm, a| dbl1(vm, "is_finite", &a, |x| x).map(|v| Value::Bool(v.to_float().is_finite()))),
        ("is_infinite", |vm, a| dbl1(vm, "is_infinite", &a, |x| x).map(|v| Value::Bool(v.to_float().is_infinite()))),
        ("bindec", |vm, a| base_to_dec(vm, "bindec", &a, 2)),
        ("hexdec", |vm, a| base_to_dec(vm, "hexdec", &a, 16)),
        ("octdec", |vm, a| base_to_dec(vm, "octdec", &a, 8)),
        ("decbin", |vm, a| dec_to_pwr2(vm, "decbin", &a, 2)),
        ("decoct", |vm, a| dec_to_pwr2(vm, "decoct", &a, 8)),
        ("dechex", |vm, a| dec_to_pwr2(vm, "dechex", &a, 16)),
    ];
    for (k, f) in list {
        m.insert(k, *f);
    }
}
