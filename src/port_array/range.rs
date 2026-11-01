//! range() ported from ext/standard/array.c.
use super::args::*;
use super::math::php_math_round;
use crate::value::*;
use crate::vm::Vm;
use std::rc::Rc;

const HT_MAX_SIZE: u64 = 0x8000_0000;

#[derive(PartialEq, Clone, Copy)]
enum T {
    Long,
    Double,
    Str,
    Array, // single-byte numeric string: IS_ARRAY in the C code
}

fn process(vm: &mut Vm<'_>, v: &Value, n: usize) -> R<(T, i64, f64)> {
    let v = v.deref();
    match &v {
        Value::Int(i) => Ok((T::Long, *i, *i as f64)),
        Value::Float(d) => check_d(vm, *d, n).map(|d| (T::Double, 0, d)),
        Value::Bool(b) => Ok((T::Long, *b as i64, *b as i64 as f64)),
        Value::Null => Ok((T::Long, 0, 0.0)),
        Value::Str(s) => {
            if s.is_empty() {
                return Ok((T::Long, 0, 0.0));
            }
            match parse_numeric(s) {
                Some((Num::F(d), true)) => check_d(vm, d, n).map(|d| (T::Double, 0, d)),
                Some((Num::I(i), true)) => Ok((if s.len() == 1 { T::Array } else { T::Long }, i, i as f64)),
                _ => Ok((T::Str, 0, 0.0)),
            }
        }
        _ => arg_type_err(vm, "range", n, if n == 1 { "start" } else { "end" }, "string|int|float", &v),
    }
}

fn check_d(vm: &mut Vm<'_>, d: f64, n: usize) -> R<f64> {
    let pname = if n == 1 { "start" } else if n == 2 { "end" } else { "step" };
    if d.is_infinite() {
        return value_err(vm, "range", n, pname, "must be a finite number, INF provided");
    }
    if d.is_nan() {
        return value_err(vm, "range", n, pname, "must be a finite number, NAN provided");
    }
    Ok(d)
}

fn first_byte(v: &Value) -> i64 {
    v.deref().to_str().bytes().next().unwrap_or(0) as i64
}

fn chr(c: i64) -> Value {
    Value::Str(Rc::from(char::from_u32(c as u32).unwrap_or('?').to_string().as_str()))
}

pub fn f_range(vm: &mut Vm<'_>, a: Vec<Value>) -> R<Value> {
    check_argc(vm, "range", &a, 2, 3)?;
    let mut is_step_double = false;
    let mut is_step_negative = false;
    let mut step_double = 1.0f64;
    let mut step = 1i64;
    if let Some(us) = a.get(2) {
        let us = p_number(vm, "range", 3, "step", us)?;
        if let Value::Float(d) = us {
            step_double = check_d(vm, d, 3)?;
            if step_double < 0.0 {
                is_step_negative = true;
                step_double *= -1.0;
            }
            step = if step_double >= -9.2e18 && step_double < 9.2e18 { step_double as i64 } else { 0 };
            if step as f64 != step_double {
                is_step_double = true;
            }
        } else {
            step = us.to_int();
            if step < 0 {
                if step == i64::MIN {
                    return vm.throw_error("ValueError", &format!("range(): Argument #3 ($step) must be greater than {step}"));
                }
                is_step_negative = true;
                step *= -1;
            }
            step_double = step as f64;
        }
        if step_double == 0.0 {
            return value_err(vm, "range", 3, "step", "cannot be 0");
        }
    }
    let (mut st, sl, sd) = process(vm, &a[0], 1)?;
    let (mut et, el, ed) = process(vm, &a[1], 2)?;
    let (mut start_long, mut end_long, mut start_double, mut end_double) = (sl, el, sd, ed);
    let _ = (&mut start_long, &mut end_long, &mut start_double, &mut end_double);
    let rank = |t: T| match t {
        T::Long => 0,
        T::Double => 1,
        T::Str => 2,
        T::Array => 3,
    };
    let mut goto_numeric = false;
    if rank(st) >= 2 || rank(et) >= 2 {
        if rank(st) < 2 || rank(et) < 2 {
            if rank(st) < 2 {
                et = T::Long;
            } else {
                st = T::Long;
            }
            goto_numeric = true;
        } else if is_step_double {
            st = T::Long;
            et = T::Long;
            goto_numeric = true;
        }
        if !goto_numeric {
            let mut low = first_byte(&a[0]);
            let high = first_byte(&a[1]);
            let mut out = PhpArray::new();
            if low > high {
                if low - high < step {
                    return boundary(vm);
                }
                while low >= high {
                    out.push(chr(low));
                    low -= step;
                }
            } else if high > low {
                if is_step_negative {
                    return negstep(vm);
                }
                if high - low < step {
                    return boundary(vm);
                }
                while low <= high {
                    out.push(chr(low));
                    low += step;
                }
            } else {
                out.push(chr(low));
            }
            return Ok(Value::Array(Rc::new(out)));
        }
    }
    // numeric inputs: a single-byte numeric string counts as its integer value
    let _ = (st == T::Array, et == T::Array);
    let mut out = PhpArray::new();
    if st == T::Double || et == T::Double || is_step_double {
        if start_double > end_double {
            if start_double - end_double < step_double {
                return boundary(vm);
            }
            let size = size_d(vm, start_double, end_double, step_double)?;
            let mut i = 0u64;
            let mut el = start_double;
            while i < size && el >= end_double {
                out.push(Value::Float(el));
                i += 1;
                el = start_double - (i as f64 * step_double);
            }
        } else if end_double > start_double {
            if is_step_negative {
                return negstep(vm);
            }
            if end_double - start_double < step_double {
                return boundary(vm);
            }
            let size = size_d(vm, end_double, start_double, step_double)?;
            let mut i = 0u64;
            let mut el = start_double;
            while i < size && el <= end_double {
                out.push(Value::Float(el));
                i += 1;
                el = start_double + (i as f64 * step_double);
            }
        } else {
            out.push(Value::Float(start_double));
        }
    } else {
        let ustep = step as u64;
        if start_long > end_long {
            if (start_long as u64).wrapping_sub(end_long as u64) < ustep {
                return boundary(vm);
            }
            let size = size_l(vm, start_long, end_long, ustep)?;
            for i in 0..size {
                out.push(Value::Int(start_long.wrapping_sub((i * ustep) as i64)));
            }
        } else if end_long > start_long {
            if is_step_negative {
                return negstep(vm);
            }
            if (end_long as u64).wrapping_sub(start_long as u64) < ustep {
                return boundary(vm);
            }
            let size = size_l(vm, end_long, start_long, ustep)?;
            for i in 0..size {
                out.push(Value::Int(start_long.wrapping_add((i * ustep) as i64)));
            }
        } else {
            out.push(Value::Int(start_long));
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

fn size_d(vm: &mut Vm<'_>, start: f64, end: f64, step: f64) -> R<u64> {
    let calc = ((start - end) / step) + 1.0;
    if calc >= HT_MAX_SIZE as f64 {
        let exceed = calc - HT_MAX_SIZE as f64;
        return vm.throw_error(
            "ValueError",
            &format!(
                "The supplied range exceeds the maximum array size by {:.1} elements: start={:.1}, end={:.1}, step={:.1}. Max size: {:.0}",
                exceed, end, start, step, HT_MAX_SIZE as f64
            ),
        );
    }
    Ok(php_math_round(calc, 0, 1) as u64)
}

fn size_l(vm: &mut Vm<'_>, start: i64, end: i64, step: u64) -> R<u64> {
    let calc = (start as u64).wrapping_sub(end as u64) / step;
    if calc >= HT_MAX_SIZE - 1 {
        let excess = calc - (HT_MAX_SIZE - 1);
        return vm.throw_error(
            "ValueError",
            &format!(
                "The supplied range exceeds the maximum array size by {excess} elements: start={end}, end={start}, step={step}. Calculated size: {calc}. Maximum size: {HT_MAX_SIZE}."
            ),
        );
    }
    Ok(calc + 1)
}

fn boundary(vm: &mut Vm<'_>) -> R<Value> {
    value_err(vm, "range", 3, "step", "must be less than the range spanned by argument #1 ($start) and argument #2 ($end)")
}

fn negstep(vm: &mut Vm<'_>) -> R<Value> {
    value_err(vm, "range", 3, "step", "must be greater than 0 for increasing ranges")
}
