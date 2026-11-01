//! Stack-machine interpreter.
use crate::builtins;
use crate::compiler::{Func, Op, Program, NO_SLOT};
use crate::value::*;
use std::rc::Rc;

pub enum Flow {
    Fatal(String),
    Exit(i32),
}

impl From<String> for Flow {
    fn from(s: String) -> Flow {
        Flow::Fatal(s)
    }
}

pub struct Vm<'p> {
    pub prog: &'p Program,
    pub out: String,
    depth: usize,
}

const MAX_DEPTH: usize = 20000;

impl<'p> Vm<'p> {
    pub fn new(prog: &'p Program) -> Self {
        Vm { prog, out: String::new(), depth: 0 }
    }

    pub fn run_main(&mut self) -> Result<(), Flow> {
        let main = &self.prog.main;
        self.exec(main, vec![]).map(|_| ())
    }

    pub fn call_user(&mut self, idx: usize, args: Vec<Value>) -> Result<Value, Flow> {
        if self.depth >= MAX_DEPTH {
            return Err(Flow::Fatal("Maximum call stack size reached. Infinite recursion?".into()));
        }
        self.depth += 1;
        let prog = self.prog;
        let r = self.exec(&prog.funcs[idx], args);
        self.depth -= 1;
        r
    }

    fn exec(&mut self, f: &Func, args: Vec<Value>) -> Result<Value, Flow> {
        let prog = self.prog;
        let argc = args.len();
        let mut locals = vec![Value::Null; f.nlocals];
        for (i, a) in args.into_iter().take(f.nparams).enumerate() {
            locals[i] = a;
        }
        let mut st: Vec<Value> = Vec::with_capacity(16);
        let code = &f.code;
        let mut pc = 0usize;
        macro_rules! pop {
            () => {
                st.pop().unwrap_or(Value::Null)
            };
        }
        loop {
            let op = code[pc];
            pc += 1;
            match op {
                Op::Const(k) => st.push(prog.consts[k as usize].clone()),
                Op::Load(s) => st.push(locals[s as usize].clone()),
                Op::Store(s) => locals[s as usize] = st.last().cloned().unwrap_or(Value::Null),
                Op::Pop => {
                    st.pop();
                }
                Op::Echo => {
                    let v = pop!();
                    self.out.push_str(&v.to_str());
                }
                Op::Bin(o) => {
                    let b = pop!();
                    let a = pop!();
                    st.push(binary(o, &a, &b)?);
                }
                Op::Not => {
                    let v = pop!();
                    st.push(Value::Bool(!v.to_bool()));
                }
                Op::Neg => {
                    let v = pop!();
                    st.push(arith("*", &v, &Value::Int(-1))?);
                }
                Op::Plus => {
                    let v = pop!();
                    st.push(arith("*", &v, &Value::Int(1))?);
                }
                Op::BitNot => {
                    let v = pop!();
                    st.push(Value::Int(!v.to_int()));
                }
                Op::ToBool => {
                    let v = pop!();
                    st.push(Value::Bool(v.to_bool()));
                }
                Op::Cast(k) => {
                    let v = pop!();
                    st.push(cast(k, v));
                }
                Op::IsNotNull => {
                    let v = pop!();
                    st.push(Value::Bool(!matches!(v, Value::Null)));
                }
                Op::Jmp(t) => pc = t as usize,
                Op::JmpZ(t) => {
                    if !pop!().to_bool() {
                        pc = t as usize;
                    }
                }
                Op::JmpNZ(t) => {
                    if pop!().to_bool() {
                        pc = t as usize;
                    }
                }
                Op::JmpZKeep(t) => {
                    if !st.last().is_some_and(Value::to_bool) {
                        pc = t as usize;
                    }
                }
                Op::JmpNZKeep(t) => {
                    if st.last().is_some_and(Value::to_bool) {
                        pc = t as usize;
                    }
                }
                Op::JmpNotNullKeep(t) => {
                    if !matches!(st.last(), Some(Value::Null) | None) {
                        pc = t as usize;
                    }
                }
                Op::NewArray => st.push(Value::Array(Rc::new(PhpArray::new()))),
                Op::ArrPush => {
                    let v = pop!();
                    if let Some(Value::Array(a)) = st.last_mut() {
                        Rc::make_mut(a).push(v);
                    }
                }
                Op::ArrSet => {
                    let v = pop!();
                    let k = Key::from_value(&pop!())?;
                    if let Some(Value::Array(a)) = st.last_mut() {
                        Rc::make_mut(a).set(k, v);
                    }
                }
                Op::Index => {
                    let k = pop!();
                    let base = pop!();
                    st.push(index(&base, &k)?);
                }
                Op::AssignDim(slot, shape) => {
                    let v = pop!();
                    let shape = &prog.shapes[shape as usize];
                    let n = shape.iter().filter(|b| **b).count();
                    let mut vals = st.split_off(st.len() - n).into_iter();
                    let mut keys = Vec::with_capacity(shape.len());
                    for has in shape {
                        keys.push(if *has { Some(Key::from_value(&vals.next().unwrap())?) } else { None });
                    }
                    let mut cur = std::mem::replace(&mut locals[slot as usize], Value::Null);
                    let r = assign_dim(&mut cur, &keys, v.clone());
                    locals[slot as usize] = cur;
                    r?;
                    st.push(v);
                }
                Op::UnsetDim(slot, shape) => {
                    let n = prog.shapes[shape as usize].len();
                    let ks = st.split_off(st.len() - n);
                    let keys: Vec<Key> = ks.iter().map(Key::from_value).collect::<Result<_, _>>()?;
                    unset_dim(&mut locals[slot as usize], &keys);
                    st.push(Value::Null);
                    st.pop();
                }
                Op::IncDec(slot, inc, prefix) => {
                    let old = locals[slot as usize].clone();
                    let new = incdec(&old, inc);
                    locals[slot as usize] = new.clone();
                    st.push(if prefix { new } else { old });
                }
                Op::IterNext { arr, pos, key, val, exit } => {
                    let p = locals[pos as usize].to_int() as usize;
                    let step = match &locals[arr as usize] {
                        Value::Array(a) => a.entry_at(p).map(|(i, k, v)| (i, k.to_value(), v.clone())),
                        _ => None,
                    };
                    match step {
                        Some((i, k, v)) => {
                            locals[pos as usize] = Value::Int(i as i64 + 1);
                            if key != NO_SLOT {
                                locals[key as usize] = k;
                            }
                            locals[val as usize] = v;
                        }
                        None => pc = exit as usize,
                    }
                }
                Op::Call(k, argc) => {
                    let args = st.split_off(st.len() - argc as usize);
                    let Value::Str(name) = &prog.consts[k as usize] else { unreachable!() };
                    if let Some(&idx) = prog.func_index.get(&**name) {
                        let r = self.call_user(idx, args)?;
                        st.push(r);
                    } else if let Some(r) = builtins::call(self, name, args) {
                        st.push(r?);
                    } else {
                        return Err(Flow::Fatal(format!("Call to undefined function {name}()")));
                    }
                }
                Op::Ret => return Ok(pop!()),
                Op::Exit => {
                    let v = pop!();
                    return Err(match v {
                        Value::Int(i) => Flow::Exit(i as i32),
                        Value::Null => Flow::Exit(0),
                        v => {
                            self.out.push_str(&v.to_str());
                            Flow::Exit(0)
                        }
                    });
                }
                Op::Fatal(k) => {
                    let Value::Str(m) = &prog.consts[k as usize] else { unreachable!() };
                    return Err(Flow::Fatal(m.to_string()));
                }
                Op::SkipIfArg(i, t) => {
                    if argc > i as usize {
                        pc = t as usize;
                    }
                }
                Op::Halt => return Ok(Value::Null),
            }
        }
    }
}

fn binary(op: &str, a: &Value, b: &Value) -> Result<Value, String> {
    Ok(match op {
        "+" | "-" | "*" | "/" | "%" | "**" => arith(op, a, b)?,
        "." => {
            let (x, y) = (a.to_str(), b.to_str());
            let mut s = String::with_capacity(x.len() + y.len());
            s.push_str(&x);
            s.push_str(&y);
            Value::Str(s.into())
        }
        "==" => Value::Bool(loose_eq(a, b)),
        "!=" => Value::Bool(!loose_eq(a, b)),
        "===" => Value::Bool(strict_eq(a, b)),
        "!==" => Value::Bool(!strict_eq(a, b)),
        "<" => Value::Bool(loose_cmp(a, b).is_lt() && !is_nan_pair(a, b)),
        "<=" => Value::Bool(loose_cmp(a, b).is_le() && !is_nan_pair(a, b)),
        ">" => Value::Bool(loose_cmp(b, a).is_lt() && !is_nan_pair(a, b)),
        ">=" => Value::Bool(loose_cmp(b, a).is_le() && !is_nan_pair(a, b)),
        "<=>" => Value::Int(loose_cmp(a, b) as i64),
        "&" | "|" | "^" | "<<" | ">>" => bitop(op, a, b)?,
        "xor" => Value::Bool(a.to_bool() ^ b.to_bool()),
        _ => return Err(format!("unknown operator {op}")),
    })
}

fn is_nan_pair(a: &Value, b: &Value) -> bool {
    matches!(a, Value::Float(f) if f.is_nan()) || matches!(b, Value::Float(f) if f.is_nan())
}

fn cast(kind: &str, v: Value) -> Value {
    match kind {
        "int" => Value::Int(v.to_int()),
        "float" => Value::Float(v.to_float()),
        "string" => Value::Str(v.to_str()),
        "bool" => Value::Bool(v.to_bool()),
        _ => match v {
            Value::Array(_) => v,
            Value::Null => Value::Array(Rc::new(PhpArray::new())),
            other => Value::Array(Rc::new(PhpArray::from_list(vec![other]))),
        },
    }
}

fn index(base: &Value, k: &Value) -> Result<Value, String> {
    Ok(match base {
        Value::Array(a) => a.get(&Key::from_value(k)?).cloned().unwrap_or(Value::Null),
        Value::Str(s) => {
            let i = k.to_int();
            let n = s.len() as i64;
            let i = if i < 0 { i + n } else { i };
            match s.as_bytes().get(i as usize).filter(|_| i >= 0) {
                Some(b) => Value::Str((*b as char).to_string().into()),
                None => Value::Null,
            }
        }
        _ => Value::Null,
    })
}

fn assign_dim(target: &mut Value, keys: &[Option<Key>], v: Value) -> Result<(), String> {
    if matches!(target, Value::Null) {
        *target = Value::Array(Rc::new(PhpArray::new()));
    }
    let Value::Array(rc) = target else {
        return Err("Cannot use a scalar value as an array".into());
    };
    let arr = Rc::make_mut(rc);
    match keys {
        [] => unreachable!(),
        [last] => match last {
            Some(k) => arr.set(k.clone(), v),
            None => arr.push(v),
        },
        [first, rest @ ..] => match first {
            Some(k) => {
                if arr.get(k).is_none() {
                    arr.set(k.clone(), Value::Null);
                }
                assign_dim(arr.get_mut(k).unwrap(), rest, v)?;
            }
            None => {
                arr.push(Value::Null);
                let last_key = arr.iter().last().map(|(k, _)| k.clone()).unwrap();
                assign_dim(arr.get_mut(&last_key).unwrap(), rest, v)?;
            }
        },
    }
    Ok(())
}

fn unset_dim(target: &mut Value, keys: &[Key]) {
    let Value::Array(rc) = target else { return };
    let arr = Rc::make_mut(rc);
    match keys {
        [] => {}
        [last] => arr.remove(last),
        [first, rest @ ..] => {
            if let Some(inner) = arr.get_mut(first) {
                unset_dim(inner, rest);
            }
        }
    }
}

fn incdec(v: &Value, inc: bool) -> Value {
    match v {
        Value::Null => if inc { Value::Int(1) } else { Value::Null },
        Value::Int(i) => match if inc { i.checked_add(1) } else { i.checked_sub(1) } {
            Some(n) => Value::Int(n),
            None => Value::Float(*i as f64 + if inc { 1.0 } else { -1.0 }),
        },
        Value::Float(f) => Value::Float(f + if inc { 1.0 } else { -1.0 }),
        Value::Str(s) => match parse_numeric(s) {
            Some((Num::I(i), true)) => incdec(&Value::Int(i), inc),
            Some((Num::F(f), true)) => Value::Float(f + if inc { 1.0 } else { -1.0 }),
            _ => v.clone(),
        },
        other => other.clone(),
    }
}
