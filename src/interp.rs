//! Tree-walking evaluator. Replaced by the opcode VM in Phase 3.
use crate::parser::{Expr, Stmt};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
}

impl Value {
    fn to_f64(&self) -> f64 {
        match self {
            Value::Int(n) => *n as f64,
            Value::Float(f) => *f,
            Value::Str(s) => numeric_prefix(s),
        }
    }

    pub fn to_php_string(&self) -> String {
        match self {
            Value::Int(n) => n.to_string(),
            Value::Float(f) => fmt_float(*f),
            Value::Str(s) => s.clone(),
        }
    }
}

fn numeric_prefix(s: &str) -> f64 {
    let t = s.trim_start();
    let end = t
        .char_indices()
        .take_while(|(i, c)| c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '-' || *c == '+')))
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    t[..end].parse().unwrap_or(0.0)
}

/// PHP prints floats with up to 14 significant digits and no trailing zeros.
fn fmt_float(f: f64) -> String {
    if f.is_nan() {
        return "NAN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "INF".into() } else { "-INF".into() };
    }
    if f == f.trunc() && f.abs() < 1e15 {
        return format!("{}", f as i64);
    }
    let s = format!("{:.13e}", f);
    let (mant, exp) = s.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    if (-5..15).contains(&exp) {
        let decimals = (13 - exp).max(0) as usize;
        let t = format!("{:.*}", decimals, f);
        let t = t.trim_end_matches('0').trim_end_matches('.');
        t.to_string()
    } else {
        let m = mant.trim_end_matches('0').trim_end_matches('.');
        format!("{m}E{}{}", if exp < 0 { "-" } else { "+" }, exp.abs())
    }
}

pub fn eval(e: &Expr) -> Result<Value, String> {
    match e {
        Expr::Int(n) => Ok(Value::Int(*n)),
        Expr::Float(f) => Ok(Value::Float(*f)),
        Expr::Str(s) => Ok(Value::Str(s.clone())),
        Expr::Neg(x) => match eval(x)? {
            Value::Int(n) => Ok(n.checked_neg().map(Value::Int).unwrap_or(Value::Float(-(n as f64)))),
            v => Ok(Value::Float(-v.to_f64())),
        },
        Expr::Bin(op, a, b) => {
            let (a, b) = (eval(a)?, eval(b)?);
            if *op == '.' {
                return Ok(Value::Str(a.to_php_string() + &b.to_php_string()));
            }
            if let (Value::Int(x), Value::Int(y)) = (&a, &b) {
                let r = match op {
                    '+' => x.checked_add(*y),
                    '-' => x.checked_sub(*y),
                    '*' => x.checked_mul(*y),
                    '/' => {
                        if *y == 0 {
                            return Err("Division by zero".into());
                        }
                        if x % y == 0 { x.checked_div(*y) } else { None }
                    }
                    _ => unreachable!(),
                };
                if let Some(r) = r {
                    return Ok(Value::Int(r));
                }
            }
            let (x, y) = (a.to_f64(), b.to_f64());
            Ok(Value::Float(match op {
                '+' => x + y,
                '-' => x - y,
                '*' => x * y,
                '/' => {
                    if y == 0.0 {
                        return Err("Division by zero".into());
                    }
                    x / y
                }
                _ => unreachable!(),
            }))
        }
    }
}

pub fn run(stmts: &[Stmt]) -> Result<String, String> {
    let mut out = String::new();
    for s in stmts {
        match s {
            Stmt::Html(h) => out.push_str(h),
            Stmt::Echo(args) => {
                for a in args {
                    out.push_str(&eval(a)?.to_php_string());
                }
            }
        }
    }
    Ok(out)
}
