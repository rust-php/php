//! Stack-machine interpreter with classes, closures and exceptions.
use crate::ast::ClassKind;
use crate::builtins;
use crate::compiler::{Func, Op, Program, NO_SLOT};
use crate::object::*;
use crate::value::*;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub enum Flow {
    Fatal(String),
    Exit(i32),
    Throw(Value),
}

impl From<String> for Flow {
    fn from(s: String) -> Flow {
        Flow::Fatal(s)
    }
}

impl From<&str> for Flow {
    fn from(s: &str) -> Flow {
        Flow::Fatal(s.to_string())
    }
}

type R<T> = Result<T, Flow>;

#[derive(Clone, Default)]
pub struct CallCtx {
    pub this: Option<Value>,
    pub self_class: Option<Rc<Class>>,
    pub static_class: Option<Rc<Class>>,
    pub caps: Vec<Value>,
}

struct Frame {
    locals: Vec<Value>,
    st: Vec<Value>,
    pc: usize,
    argc: usize,
    ctx: CallCtx,
}

pub struct Vm<'p> {
    pub prog: &'p Program,
    pub out: String,
    pub file: String,
    pub classes: HashMap<String, Rc<Class>>,
    pub constants: HashMap<String, Value>,
    depth: usize,
}

const MAX_DEPTH: usize = 20000;

fn lc(s: &str) -> String {
    s.to_ascii_lowercase()
}

impl<'p> Vm<'p> {
    pub fn new(prog: &'p Program) -> Self {
        Vm { prog, out: String::new(), file: String::new(), classes: HashMap::new(), constants: HashMap::new(), depth: 0 }
    }

    pub fn run_main(&mut self) -> R<()> {
        self.declare_classes();
        let prog = self.prog;
        self.exec(&prog.main, vec![], CallCtx::default()).map(|_| ())
    }

    // ---- classes ----

    fn declare_classes(&mut self) {
        let mut pending: Vec<usize> = (0..self.prog.classes.len()).collect();
        loop {
            let before = pending.len();
            pending.retain(|&i| !self.try_declare(i));
            if pending.is_empty() || pending.len() == before {
                break;
            }
        }
    }

    fn try_declare(&mut self, i: usize) -> bool {
        let def = &self.prog.classes[i];
        let lname = lc(&def.name);
        if self.classes.contains_key(&lname) {
            return true;
        }
        let parent = match &def.parent {
            Some(p) => match self.classes.get(&lc(p)) {
                Some(c) => Some(c.clone()),
                None => return false,
            },
            None => None,
        };
        let mut ifaces = vec![];
        for n in &def.interfaces {
            match self.classes.get(&lc(n)) {
                Some(c) => ifaces.push(c.clone()),
                None => return false,
            }
        }
        let mut traits = vec![];
        for n in &def.traits {
            match self.classes.get(&lc(n)) {
                Some(c) => traits.push(c.clone()),
                None => return false,
            }
        }
        let mut interfaces: Vec<String> = parent.as_ref().map(|p| p.interfaces.clone()).unwrap_or_default();
        for ic in &ifaces {
            interfaces.push(lc(&ic.name));
            interfaces.extend(ic.interfaces.iter().cloned());
        }
        interfaces.sort();
        interfaces.dedup();
        let mut methods = parent.as_ref().map(|p| p.methods.clone()).unwrap_or_default();
        let mut props: Vec<PropInfo> = parent.as_ref().map(|p| p.props.clone()).unwrap_or_default();
        let mut const_thunks = HashMap::new();
        let mut static_thunks: Vec<(String, Option<usize>)> = vec![];
        for t in &traits {
            for (k, m) in &t.methods {
                methods.insert(k.clone(), Rc::new(Method { class: def.name.clone(), ..(**m).clone() }));
            }
            for p in &t.props {
                props.retain(|x| x.name != p.name);
                props.push(PropInfo { class: def.name.clone(), ..p.clone() });
            }
            static_thunks.extend(t.static_thunks.iter().cloned());
            const_thunks.extend(t.const_thunks.iter().map(|(k, v)| (k.clone(), *v)));
        }
        for m in &def.methods {
            methods.insert(
                lc(&m.name),
                Rc::new(Method { name: m.name.clone(), func: m.func, class: def.name.clone(), is_static: m.is_static, is_abstract: m.is_abstract, vis: m.vis }),
            );
        }
        for p in &def.props {
            if p.is_static {
                static_thunks.push((p.name.clone(), p.thunk));
            } else {
                let info = PropInfo { name: p.name.clone(), thunk: p.thunk, vis: p.vis, class: def.name.clone() };
                match props.iter().position(|x| x.name == p.name) {
                    Some(at) => props[at] = info,
                    None => props.push(info),
                }
            }
        }
        for (n, t) in &def.consts {
            const_thunks.insert(n.clone(), *t);
        }
        let class = Class {
            name: def.name.clone(),
            kind: def.kind,
            is_abstract: def.is_abstract,
            parent,
            interfaces,
            consts: RefCell::new(HashMap::new()),
            const_thunks,
            props,
            static_thunks,
            statics: RefCell::new(HashMap::new()),
            statics_ready: RefCell::new(false),
            methods,
        };
        self.classes.insert(lname, Rc::new(class));
        true
    }

    pub fn class_of(&self, v: &Value) -> R<Rc<Class>> {
        match v {
            Value::Object(o) => Ok(o.class.clone()),
            Value::Str(n) => self.find_class(n),
            _ => Err(Flow::Fatal("Cannot use value as class name".into())),
        }
    }

    pub fn find_class(&self, name: &str) -> R<Rc<Class>> {
        self.classes.get(&lc(name)).cloned().ok_or_else(|| Flow::Fatal(format!("Class \"{name}\" not found")))
    }

    fn class_by_name(&self, name: &str) -> Option<Rc<Class>> {
        self.classes.get(&lc(name)).cloned()
    }

    fn run_thunk(&mut self, func: usize, class: &Rc<Class>) -> R<Value> {
        let ctx = CallCtx { self_class: Some(class.clone()), static_class: Some(class.clone()), ..Default::default() };
        self.call_func(func, vec![], ctx)
    }

    fn const_owner(&self, class: &Rc<Class>, name: &str) -> Option<Rc<Class>> {
        let mut cur = Some(class.clone());
        while let Some(c) = cur {
            if c.const_thunks.contains_key(name) {
                return Some(c);
            }
            for i in &c.interfaces {
                if let Some(ic) = self.classes.get(i) {
                    if ic.const_thunks.contains_key(name) {
                        return Some(ic.clone());
                    }
                }
            }
            cur = c.parent.clone();
        }
        None
    }

    pub fn class_const_pub(&mut self, class: &Rc<Class>, name: &str) -> R<Value> {
        self.class_const(class, name)
    }

    fn class_const(&mut self, class: &Rc<Class>, name: &str) -> R<Value> {
        if name == "class" {
            return Ok(Value::str(&class.name));
        }
        let owner = self
            .const_owner(class, name)
            .ok_or_else(|| Flow::Fatal(format!("Undefined constant {}::{}", class.name, name)))?;
        if let Some(v) = owner.consts.borrow().get(name) {
            return Ok(v.clone());
        }
        let v = self.run_thunk(owner.const_thunks[name], &owner)?;
        owner.consts.borrow_mut().insert(name.to_string(), v.clone());
        Ok(v)
    }

    fn static_owner(&mut self, class: &Rc<Class>, name: &str) -> R<Rc<Class>> {
        let mut cur = Some(class.clone());
        while let Some(c) = cur {
            if c.static_thunks.iter().any(|(n, _)| n == name) {
                if !*c.statics_ready.borrow() {
                    *c.statics_ready.borrow_mut() = true;
                    for (n, t) in c.static_thunks.clone() {
                        let v = match t {
                            Some(t) => self.run_thunk(t, &c)?,
                            None => Value::Null,
                        };
                        c.statics.borrow_mut().insert(n, v);
                    }
                }
                return Ok(c);
            }
            cur = c.parent.clone();
        }
        Err(Flow::Fatal(format!("Access to undeclared static property {}::${}", class.name, name)))
    }

    pub fn instantiate(&mut self, class: &Rc<Class>, args: Vec<Value>) -> R<Value> {
        if class.kind != ClassKind::Class || class.is_abstract {
            let what = match class.kind {
                ClassKind::Interface => "interface",
                ClassKind::Trait => "trait",
                ClassKind::Class => "abstract class",
            };
            return Err(Flow::Fatal(format!("Cannot instantiate {} {}", what, class.name)));
        }
        let mut props = PhpArray::new();
        for p in &class.props {
            let v = match p.thunk {
                Some(t) => {
                    let owner = self.class_by_name(&p.class).unwrap_or_else(|| class.clone());
                    self.run_thunk(t, &owner)?
                }
                None => Value::Null,
            };
            props.set(Key::from_str(&p.name), v);
        }
        let obj = Value::Object(Obj::new(class.clone(), props, Extra::None));
        if let Some(m) = class.find_method("__construct") {
            self.call_method(&obj, &m, args)?;
        }
        Ok(obj)
    }

    pub fn make_throwable(&mut self, class: &str, msg: &str) -> R<Value> {
        let c = self.find_class(class)?;
        self.instantiate(&c, vec![Value::str(msg)])
    }

    pub fn throw_error<T>(&mut self, class: &str, msg: &str) -> R<T> {
        Err(match self.make_throwable(class, msg) {
            Ok(v) => Flow::Throw(v),
            Err(_) => Flow::Fatal(msg.to_string()),
        })
    }

    pub fn describe_uncaught(&mut self, v: &Value) -> String {
        let Value::Object(o) = v else { return "Uncaught exception".into() };
        let msg = o.props.borrow().get(&Key::from_str("message")).map(|m| m.to_str().to_string()).unwrap_or_default();
        let file = self.file.clone();
        format!("Uncaught {}: {} in {}:0\nStack trace:\n#0 {{main}}\n  thrown in {} on line 0", o.class.name, msg, file, file)
    }

    // ---- calls ----

    pub fn call_func(&mut self, idx: usize, args: Vec<Value>, ctx: CallCtx) -> R<Value> {
        if self.depth >= MAX_DEPTH {
            return Err(Flow::Fatal("Maximum call stack size reached. Infinite recursion?".into()));
        }
        self.depth += 1;
        let prog = self.prog;
        let r = self.exec(&prog.funcs[idx], args, ctx);
        self.depth -= 1;
        r
    }

    pub fn call_method(&mut self, this: &Value, m: &Rc<Method>, args: Vec<Value>) -> R<Value> {
        let Value::Object(o) = this else { return Err(Flow::Fatal("method call on non-object".into())) };
        let ctx = CallCtx { this: Some(this.clone()), self_class: self.class_by_name(&m.class), static_class: Some(o.class.clone()), caps: vec![] };
        self.call_func(m.func, args, ctx)
    }

    /// Invoke any PHP callable: closure, `__invoke` object, function name, "A::m" or [obj|class, "m"].
    pub fn call_value(&mut self, callee: &Value, args: Vec<Value>) -> R<Value> {
        match callee {
            Value::Object(o) => {
                let cd = match &*o.extra.borrow() {
                    Extra::Closure(c) => Some(c.clone()),
                    Extra::None => None,
                };
                if let Some(cd) = cd {
                    return self.call_closure(&cd, args, None);
                }
                match o.class.find_method("__invoke") {
                    Some(m) => self.call_method(callee, &m, args),
                    None => self.throw_error("Error", "Object of type is not callable"),
                }
            }
            Value::Str(s) => {
                if let Some((c, m)) = s.split_once("::") {
                    let class = self.find_class(c)?;
                    return self.call_static_named(&class, m, args);
                }
                self.call_function_named(&lc(s), args)
            }
            Value::Array(a) if a.len() == 2 => {
                let first = a.get(&Key::Int(0)).cloned().unwrap_or(Value::Null);
                let name = a.get(&Key::Int(1)).map(|v| v.to_str()).unwrap_or_else(|| "".into());
                match &first {
                    Value::Object(o) => match o.class.find_method(&lc(&name)) {
                        Some(m) => self.call_method(&first, &m, args),
                        None => self.throw_error("Error", &format!("Call to undefined method {}::{}()", o.class.name, name)),
                    },
                    Value::Str(c) => {
                        let class = self.find_class(c)?;
                        self.call_static_named(&class, &name, args)
                    }
                    _ => self.throw_error("Error", "Array callback must have exactly two elements"),
                }
            }
            _ => self.throw_error("Error", "Value not callable"),
        }
    }

    fn call_static_named(&mut self, class: &Rc<Class>, name: &str, args: Vec<Value>) -> R<Value> {
        match class.find_method(&lc(name)) {
            Some(m) => {
                let ctx = CallCtx { this: None, self_class: self.class_by_name(&m.class), static_class: Some(class.clone()), caps: vec![] };
                self.call_func(m.func, args, ctx)
            }
            None => self.throw_error("Error", &format!("Call to undefined method {}::{}()", class.name, name)),
        }
    }

    pub fn call_function_named(&mut self, lname: &str, args: Vec<Value>) -> R<Value> {
        if let Some(&idx) = self.prog.func_index.get(lname) {
            return self.call_func(idx, args, CallCtx::default());
        }
        match builtins::call(self, lname, args) {
            Some(r) => r,
            None => self.throw_error("Error", &format!("Call to undefined function {lname}()")),
        }
    }

    fn call_closure(&mut self, cd: &ClosureData, args: Vec<Value>, new_this: Option<Value>) -> R<Value> {
        let this = new_this.or_else(|| cd.this.clone());
        let ctx = CallCtx { this, self_class: cd.self_class.clone(), static_class: cd.static_class.clone(), caps: cd.caps.clone() };
        self.call_func(cd.func, args, ctx)
    }

    pub fn to_string_val(&mut self, v: &Value) -> R<Rc<str>> {
        if let Value::Object(o) = v {
            return match o.class.find_method("__tostring") {
                Some(m) => Ok(self.call_method(v, &m, vec![])?.to_str()),
                None => self.throw_error("Error", &format!("Object of class {} could not be converted to string", o.class.name)),
            };
        }
        Ok(v.to_str())
    }

    fn to_throw(&mut self, e: Flow) -> Flow {
        let Flow::Fatal(msg) = e else { return e };
        if msg.starts_with("Maximum call stack") {
            return Flow::Fatal(msg);
        }
        let class = if msg.starts_with("Division by zero") || msg.starts_with("Modulo by zero") {
            "DivisionByZeroError"
        } else if msg.contains("Unsupported operand types") || msg.contains("must be of type") {
            "TypeError"
        } else if msg.contains("cannot be empty") || msg.contains("must be greater") || msg.contains("must have the same number") {
            "ValueError"
        } else {
            "Error"
        };
        match self.make_throwable(class, &msg) {
            Ok(v) => Flow::Throw(v),
            Err(_) => Flow::Fatal(msg),
        }
    }

    fn exec(&mut self, f: &'p Func, args: Vec<Value>, ctx: CallCtx) -> R<Value> {
        let mut locals = vec![Value::Null; f.nlocals];
        if let Some(t) = &ctx.this {
            locals[0] = t.clone();
        }
        let argc = args.len();
        for (i, a) in args.into_iter().take(f.nparams).enumerate() {
            locals[1 + i] = a;
        }
        for (slot, v) in f.capture_slots.iter().zip(ctx.caps.iter()) {
            locals[*slot as usize] = v.clone();
        }
        let mut fr = Frame { locals, st: Vec::with_capacity(16), pc: 0, argc, ctx };
        loop {
            let err = match self.run(f, &mut fr) {
                Ok(v) => return Ok(v),
                Err(e) => e,
            };
            let err = self.to_throw(err);
            let Flow::Throw(v) = err else { return Err(err) };
            let at = fr.pc.saturating_sub(1);
            match f.handlers.iter().find(|h| h.start <= at && at < h.end) {
                Some(h) => {
                    fr.st.clear();
                    fr.st.push(v);
                    fr.pc = h.target;
                }
                None => return Err(Flow::Throw(v)),
            }
        }
    }

    fn run(&mut self, f: &'p Func, fr: &mut Frame) -> R<Value> {
        let prog = self.prog;
        let code = &f.code;
        macro_rules! pop {
            () => {
                fr.st.pop().unwrap_or(Value::Null)
            };
        }
        macro_rules! split_args {
            ($n:expr) => {{
                let at = fr.st.len().saturating_sub($n as usize);
                fr.st.split_off(at)
            }};
        }
        loop {
            let op = code[fr.pc];
            fr.pc += 1;
            match op {
                Op::Const(k) => fr.st.push(prog.consts[k as usize].clone()),
                Op::Load(s) => fr.st.push(fr.locals[s as usize].clone()),
                Op::Store(s) => fr.locals[s as usize] = fr.st.last().cloned().unwrap_or(Value::Null),
                Op::Pop => {
                    fr.st.pop();
                }
                Op::Echo => {
                    let v = pop!();
                    let s = self.to_string_val(&v)?;
                    self.out.push_str(&s);
                }
                Op::Bin(o) => {
                    let b = pop!();
                    let a = pop!();
                    if o == "." && (matches!(a, Value::Object(_)) || matches!(b, Value::Object(_))) {
                        let (x, y) = (self.to_string_val(&a)?, self.to_string_val(&b)?);
                        fr.st.push(Value::Str(format!("{x}{y}").into()));
                    } else {
                        fr.st.push(binary(o, &a, &b)?);
                    }
                }
                Op::Not => {
                    let v = pop!();
                    fr.st.push(Value::Bool(!v.to_bool()));
                }
                Op::Neg => {
                    let v = pop!();
                    fr.st.push(arith("*", &v, &Value::Int(-1))?);
                }
                Op::Plus => {
                    let v = pop!();
                    fr.st.push(arith("*", &v, &Value::Int(1))?);
                }
                Op::BitNot => {
                    let v = pop!();
                    fr.st.push(Value::Int(!v.to_int()));
                }
                Op::ToBool => {
                    let v = pop!();
                    fr.st.push(Value::Bool(v.to_bool()));
                }
                Op::Cast(k) => {
                    let v = pop!();
                    let r = if k == "string" { Value::Str(self.to_string_val(&v)?) } else { cast(k, v) };
                    fr.st.push(r);
                }
                Op::IsNotNull => {
                    let v = pop!();
                    fr.st.push(Value::Bool(!matches!(v, Value::Null)));
                }
                Op::Jmp(t) => fr.pc = t as usize,
                Op::JmpZ(t) => {
                    if !pop!().to_bool() {
                        fr.pc = t as usize;
                    }
                }
                Op::JmpNZ(t) => {
                    if pop!().to_bool() {
                        fr.pc = t as usize;
                    }
                }
                Op::JmpZKeep(t) => {
                    if !fr.st.last().is_some_and(Value::to_bool) {
                        fr.pc = t as usize;
                    }
                }
                Op::JmpNZKeep(t) => {
                    if fr.st.last().is_some_and(Value::to_bool) {
                        fr.pc = t as usize;
                    }
                }
                Op::JmpNotNullKeep(t) => {
                    if !matches!(fr.st.last(), Some(Value::Null) | None) {
                        fr.pc = t as usize;
                    }
                }
                Op::JmpNullKeep(t) => {
                    if matches!(fr.st.last(), Some(Value::Null) | None) {
                        fr.pc = t as usize;
                    }
                }
                Op::NewArray => fr.st.push(Value::Array(Rc::new(PhpArray::new()))),
                Op::ArrPush => {
                    let v = pop!();
                    if let Some(Value::Array(a)) = fr.st.last_mut() {
                        Rc::make_mut(a).push(v);
                    }
                }
                Op::ArrSet => {
                    let v = pop!();
                    let k = Key::from_value(&pop!())?;
                    if let Some(Value::Array(a)) = fr.st.last_mut() {
                        Rc::make_mut(a).set(k, v);
                    }
                }
                Op::Index => {
                    let k = pop!();
                    let base = pop!();
                    let r = match &base {
                        Value::Object(o) => match o.class.find_method("offsetget") {
                            Some(m) => self.call_method(&base, &m, vec![k])?,
                            None => return self.throw_error("Error", &format!("Cannot use object of type {} as array", o.class.name)),
                        },
                        _ => index(&base, &k)?,
                    };
                    fr.st.push(r);
                }
                Op::AssignDim(slot, shape) => {
                    let v = pop!();
                    let shape = &prog.shapes[shape as usize];
                    let n = shape.iter().filter(|b| **b).count();
                    let mut vals = split_args!(n).into_iter();
                    if let Value::Object(o) = fr.locals[slot as usize].clone() {
                        let key = vals.next().unwrap_or(Value::Null);
                        match o.class.find_method("offsetset") {
                            Some(m) => self.call_method(&Value::Object(o), &m, vec![key, v.clone()])?,
                            None => return self.throw_error("Error", &format!("Cannot use object of type {} as array", o.class.name)),
                        };
                        fr.st.push(v);
                        continue;
                    }
                    let mut keys = Vec::with_capacity(shape.len());
                    for has in shape {
                        keys.push(if *has { Some(Key::from_value(&vals.next().unwrap())?) } else { None });
                    }
                    let mut cur = std::mem::replace(&mut fr.locals[slot as usize], Value::Null);
                    let r = assign_dim(&mut cur, &keys, v.clone());
                    fr.locals[slot as usize] = cur;
                    r?;
                    fr.st.push(v);
                }
                Op::UnsetDim(slot, shape) => {
                    let n = prog.shapes[shape as usize].len();
                    let ks = split_args!(n);
                    let keys: Vec<Key> = ks.iter().map(Key::from_value).collect::<Result<_, _>>()?;
                    unset_dim(&mut fr.locals[slot as usize], &keys);
                }
                Op::IncDec(slot, inc, prefix) => {
                    let old = fr.locals[slot as usize].clone();
                    let new = incdec(&old, inc);
                    fr.locals[slot as usize] = new.clone();
                    fr.st.push(if prefix { new } else { old });
                }
                Op::IterNext { arr, pos, key, val, exit } => {
                    if let Value::Object(o) = fr.locals[arr as usize].clone() {
                        if self.iter_object(fr, &o, arr, pos, key, val)? {
                            fr.pc = exit as usize;
                        }
                        continue;
                    }
                    let p = fr.locals[pos as usize].to_int() as usize;
                    let step = match &fr.locals[arr as usize] {
                        Value::Array(a) => a.entry_at(p).map(|(i, k, v)| (i, k.to_value(), v.clone())),
                        _ => None,
                    };
                    match step {
                        Some((i, k, v)) => {
                            fr.locals[pos as usize] = Value::Int(i as i64 + 1);
                            if key != NO_SLOT {
                                fr.locals[key as usize] = k;
                            }
                            fr.locals[val as usize] = v;
                        }
                        None => fr.pc = exit as usize,
                    }
                }
                Op::Call(k, argc) => {
                    let args = split_args!(argc);
                    let Value::Str(name) = &prog.consts[k as usize] else { unreachable!() };
                    let r = if let Some(&idx) = prog.func_index.get(&**name) {
                        self.call_func(idx, args, CallCtx::default())?
                    } else if let Some(r) = builtins::call(self, name, args) {
                        r?
                    } else {
                        return self.throw_error("Error", &format!("Call to undefined function {name}()"));
                    };
                    fr.st.push(r);
                }
                Op::CallValue(argc) => {
                    let args = split_args!(argc);
                    let callee = pop!();
                    let r = self.call_value(&callee, args)?;
                    fr.st.push(r);
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
                    if fr.argc > i as usize {
                        fr.pc = t as usize;
                    }
                }
                Op::Halt => return Ok(Value::Null),
                Op::PushSelf => match &fr.ctx.self_class {
                    Some(c) => fr.st.push(Value::str(&c.name)),
                    None => return self.throw_error("Error", "Cannot use \"self\" when no class scope is active"),
                },
                Op::PushParent => match fr.ctx.self_class.as_ref().and_then(|c| c.parent.clone()) {
                    Some(p) => fr.st.push(Value::str(&p.name)),
                    None => return self.throw_error("Error", "Cannot use \"parent\" when current class scope has no parent"),
                },
                Op::PushStatic => match fr.ctx.static_class.as_ref().or(fr.ctx.self_class.as_ref()) {
                    Some(c) => fr.st.push(Value::str(&c.name)),
                    None => return self.throw_error("Error", "Cannot use \"static\" when no class scope is active"),
                },
                Op::New(argc) => {
                    let args = split_args!(argc);
                    let cv = pop!();
                    let class = self.class_of(&cv)?;
                    let obj = self.instantiate(&class, args)?;
                    fr.st.push(obj);
                }
                Op::GetProp => {
                    let name = pop!().to_str();
                    let obj = pop!();
                    let r = match &obj {
                        Value::Object(o) => {
                            let v = o.props.borrow().get(&Key::from_str(&name)).cloned();
                            match v {
                                Some(v) => v,
                                None => match o.class.find_method("__get") {
                                    Some(m) => self.call_method(&obj, &m, vec![Value::Str(name)])?,
                                    None => Value::Null,
                                },
                            }
                        }
                        _ => Value::Null,
                    };
                    fr.st.push(r);
                }
                Op::AssignProp => {
                    let v = pop!();
                    let name = pop!().to_str();
                    let obj = pop!();
                    let Value::Object(o) = &obj else {
                        return self.throw_error("Error", &format!("Attempt to assign property \"{}\" on {}", name, obj.type_name()));
                    };
                    let key = Key::from_str(&name);
                    let exists = o.props.borrow().get(&key).is_some();
                    match o.class.find_method("__set") {
                        Some(m) if !exists => {
                            self.call_method(&obj, &m, vec![Value::Str(name), v.clone()])?;
                        }
                        _ => o.props.borrow_mut().set(key, v.clone()),
                    }
                    fr.st.push(v);
                }
                Op::AssignPropDim(shape) => {
                    let v = pop!();
                    let shape = &prog.shapes[shape as usize];
                    let n = shape.iter().filter(|b| **b).count();
                    let mut vals = split_args!(n).into_iter();
                    let name = pop!().to_str();
                    let obj = pop!();
                    let Value::Object(o) = &obj else {
                        return self.throw_error("Error", &format!("Attempt to assign property \"{}\" on {}", name, obj.type_name()));
                    };
                    let mut keys = Vec::with_capacity(shape.len());
                    for has in shape {
                        keys.push(if *has { Some(Key::from_value(&vals.next().unwrap())?) } else { None });
                    }
                    let pk = Key::from_str(&name);
                    let mut cur = o.props.borrow_mut().get_mut(&pk).map(|x| std::mem::replace(x, Value::Null)).unwrap_or(Value::Null);
                    let r = assign_dim(&mut cur, &keys, v.clone());
                    o.props.borrow_mut().set(pk, cur);
                    r?;
                    fr.st.push(v);
                }
                Op::CallMethod(argc) => {
                    let args = split_args!(argc);
                    let name = pop!().to_str();
                    let obj = pop!();
                    let r = self.call_method_named(&obj, &name, args)?;
                    fr.st.push(r);
                }
                Op::CallStatic(argc, forward) => {
                    let args = split_args!(argc);
                    let name = pop!().to_str();
                    let cv = pop!();
                    let r = self.call_static_op(fr, &cv, &name, args, forward)?;
                    fr.st.push(r);
                }
                Op::ClassConst => {
                    let name = pop!().to_str();
                    let cv = pop!();
                    let r = match (&cv, &*name) {
                        (Value::Str(_), "class") => cv.clone(),
                        _ => {
                            let class = self.class_of(&cv)?;
                            self.class_const(&class, &name)?
                        }
                    };
                    fr.st.push(r);
                }
                Op::GetStatic => {
                    let name = pop!().to_str();
                    let cv = pop!();
                    let class = self.class_of(&cv)?;
                    let owner = self.static_owner(&class, &name)?;
                    let v = owner.statics.borrow().get(&*name).cloned().unwrap_or(Value::Null);
                    fr.st.push(v);
                }
                Op::AssignStatic => {
                    let v = pop!();
                    let name = pop!().to_str();
                    let cv = pop!();
                    let class = self.class_of(&cv)?;
                    let owner = self.static_owner(&class, &name)?;
                    owner.statics.borrow_mut().insert(name.to_string(), v.clone());
                    fr.st.push(v);
                }
                Op::AssignStaticDim(shape) => {
                    let v = pop!();
                    let shape = &prog.shapes[shape as usize];
                    let n = shape.iter().filter(|b| **b).count();
                    let mut vals = split_args!(n).into_iter();
                    let name = pop!().to_str();
                    let cv = pop!();
                    let class = self.class_of(&cv)?;
                    let owner = self.static_owner(&class, &name)?;
                    let mut keys = Vec::with_capacity(shape.len());
                    for has in shape {
                        keys.push(if *has { Some(Key::from_value(&vals.next().unwrap())?) } else { None });
                    }
                    let mut cur = owner.statics.borrow_mut().remove(&*name).unwrap_or(Value::Null);
                    let r = assign_dim(&mut cur, &keys, v.clone());
                    owner.statics.borrow_mut().insert(name.to_string(), cur);
                    r?;
                    fr.st.push(v);
                }
                Op::InstanceOf => {
                    let cv = pop!();
                    let v = pop!();
                    let target = match &cv {
                        Value::Object(o) => lc(&o.class.name),
                        other => lc(&other.to_str()),
                    };
                    fr.st.push(Value::Bool(matches!(&v, Value::Object(o) if o.class.is_a(&target))));
                }
                Op::Clone => {
                    let v = pop!();
                    let Value::Object(o) = &v else { return self.throw_error("Error", "__clone method called on non-object") };
                    let extra = match &*o.extra.borrow() {
                        Extra::None => Extra::None,
                        Extra::Closure(c) => Extra::Closure(c.clone()),
                    };
                    let copy = Value::Object(Obj::new(o.class.clone(), o.props.borrow().clone(), extra));
                    if let Some(m) = o.class.find_method("__clone") {
                        self.call_method(&copy, &m, vec![])?;
                    }
                    fr.st.push(copy);
                }
                Op::MakeClosure(func, ncaps) => {
                    let caps = split_args!(ncaps);
                    let class = self.find_class("Closure")?;
                    let cd = ClosureData {
                        func: func as usize,
                        caps,
                        this: fr.ctx.this.clone(),
                        self_class: fr.ctx.self_class.clone(),
                        static_class: fr.ctx.static_class.clone(),
                    };
                    fr.st.push(Value::Object(Obj::new(class, PhpArray::new(), Extra::Closure(cd))));
                }
                Op::Throw => {
                    let v = pop!();
                    return match &v {
                        Value::Object(o) if o.class.is_a("throwable") => Err(Flow::Throw(v)),
                        _ => self.throw_error("Error", "Can only throw objects"),
                    };
                }
                Op::JmpIfInstance(k, t) => {
                    let Value::Str(name) = &prog.consts[k as usize] else { unreachable!() };
                    if matches!(fr.st.last(), Some(Value::Object(o)) if o.class.is_a(name)) {
                        fr.pc = t as usize;
                    }
                }
                Op::MatchError => {
                    let v = pop!();
                    let repr = match &v {
                        Value::Str(s) => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
                        Value::Object(o) => format!("of type {}", o.class.name),
                        Value::Array(_) => "of type array".to_string(),
                        Value::Bool(b) => (if *b { "true" } else { "false" }).to_string(),
                        Value::Null => "NULL".to_string(),
                        other => other.to_str().to_string(),
                    };
                    return self.throw_error("UnhandledMatchError", &format!("Unhandled match case {repr}"));
                }
                Op::ConstGet(k) => {
                    let Value::Str(name) = &prog.consts[k as usize] else { unreachable!() };
                    let v = match self.constants.get(&**name) {
                        Some(v) => v.clone(),
                        None => match &**name {
                            "__FILE__" => Value::str(&self.file),
                            "__DIR__" => Value::str(std::path::Path::new(&self.file).parent().and_then(|p| p.to_str()).unwrap_or("")),
                            "__LINE__" => Value::Int(0),
                            "__CLASS__" => fr.ctx.self_class.as_ref().map(|c| Value::str(&c.name)).unwrap_or_else(|| Value::str("")),
                            "__FUNCTION__" | "__METHOD__" | "__NAMESPACE__" => Value::str(""),
                            _ => return self.throw_error("Error", &format!("Undefined constant \"{name}\"")),
                        },
                    };
                    fr.st.push(v);
                }
                Op::DefConst(k) => {
                    let Value::Str(name) = &prog.consts[k as usize] else { unreachable!() };
                    let v = pop!();
                    self.constants.insert(name.to_string(), v);
                }
            }
        }
    }

    pub fn call_method_named(&mut self, obj: &Value, name: &str, args: Vec<Value>) -> R<Value> {
        let Value::Object(o) = obj else {
            return self.throw_error("Error", &format!("Call to a member function {}() on {}", name, obj.type_name()));
        };
        let lname = lc(name);
        let closure = match &*o.extra.borrow() {
            Extra::Closure(c) => Some(c.clone()),
            Extra::None => None,
        };
        if let Some(cd) = closure {
            match lname.as_str() {
                "__invoke" => return self.call_closure(&cd, args, None),
                "call" => {
                    let mut it = args.into_iter();
                    let new_this = it.next().unwrap_or(Value::Null);
                    let mut cd2 = cd.clone();
                    if let Value::Object(t) = &new_this {
                        cd2.self_class = Some(t.class.clone());
                        cd2.static_class = Some(t.class.clone());
                    }
                    return self.call_closure(&cd2, it.collect(), Some(new_this));
                }
                "bindto" | "bind" => {
                    let mut cd2 = cd.clone();
                    let this = args.first().cloned().unwrap_or(Value::Null);
                    cd2.this = if matches!(this, Value::Null) { None } else { Some(this.clone()) };
                    if let Value::Object(t) = &this {
                        cd2.self_class = Some(t.class.clone());
                        cd2.static_class = Some(t.class.clone());
                    }
                    if let Some(scope) = args.get(1) {
                        if let Ok(c) = self.class_of(scope) {
                            cd2.self_class = Some(c);
                        }
                    }
                    let class = o.class.clone();
                    return Ok(Value::Object(Obj::new(class, PhpArray::new(), Extra::Closure(cd2))));
                }
                _ => {}
            }
        }
        if let Some(m) = o.class.find_method(&lname) {
            return self.call_method(obj, &m, args);
        }
        if let Some(m) = o.class.find_method("__call") {
            let list = Value::Array(Rc::new(PhpArray::from_list(args)));
            return self.call_method(obj, &m, vec![Value::str(name), list]);
        }
        self.throw_error("Error", &format!("Call to undefined method {}::{}()", o.class.name, name))
    }

    fn call_static_op(&mut self, fr: &Frame, cv: &Value, name: &str, args: Vec<Value>, forward: bool) -> R<Value> {
        if let Value::Str(c) = cv {
            if c.eq_ignore_ascii_case("closure") {
                match lc(name).as_str() {
                    "fromcallable" => return Ok(args.into_iter().next().unwrap_or(Value::Null)),
                    "bind" => {
                        let mut it = args.into_iter();
                        let closure = it.next().unwrap_or(Value::Null);
                        return self.call_method_named(&closure, "bindTo", it.collect());
                    }
                    _ => {}
                }
            }
        }
        let class = self.class_of(cv)?;
        let lname = lc(name);
        match class.find_method(&lname) {
            Some(m) => {
                let this = if m.is_static {
                    None
                } else {
                    fr.ctx.this.clone().filter(|t| matches!(t, Value::Object(o) if o.class.is_a(&lc(&class.name))))
                };
                if !m.is_static && this.is_none() {
                    return self.throw_error("Error", &format!("Non-static method {}::{}() cannot be called statically", class.name, m.name));
                }
                let static_class = match &this {
                    Some(Value::Object(o)) => o.class.clone(),
                    _ if forward => fr.ctx.static_class.clone().unwrap_or_else(|| class.clone()),
                    _ => class.clone(),
                };
                let ctx = CallCtx { this, self_class: self.class_by_name(&m.class), static_class: Some(static_class), caps: vec![] };
                self.call_func(m.func, args, ctx)
            }
            None => match class.find_method("__callstatic") {
                Some(m) => {
                    let list = Value::Array(Rc::new(PhpArray::from_list(args)));
                    let ctx = CallCtx { this: None, self_class: self.class_by_name(&m.class), static_class: Some(class.clone()), caps: vec![] };
                    self.call_func(m.func, vec![Value::str(name), list], ctx)
                }
                None => self.throw_error("Error", &format!("Call to undefined method {}::{}()", class.name, name)),
            },
        }
    }

    /// One foreach step over an object. Returns true when iteration is finished.
    fn iter_object(&mut self, fr: &mut Frame, o: &Rc<Obj>, arr: u16, pos: u16, key: u16, val: u16) -> R<bool> {
        let obj = Value::Object(o.clone());
        if o.class.is_a("iteratoraggregate") {
            let mut inner = self.call_method_named(&obj, "getIterator", vec![])?;
            while let Value::Object(i) = &inner {
                if i.class.is_a("iteratoraggregate") {
                    inner = self.call_method_named(&inner, "getIterator", vec![])?;
                } else {
                    break;
                }
            }
            fr.locals[arr as usize] = inner;
            return self.step_iter(fr, arr, pos, key, val);
        }
        if o.class.is_a("iterator") {
            return self.step_iter(fr, arr, pos, key, val);
        }
        // Plain object: iterate its properties.
        fr.locals[arr as usize] = Value::Array(Rc::new(o.props.borrow().clone()));
        let p = fr.locals[pos as usize].to_int() as usize;
        let step = match &fr.locals[arr as usize] {
            Value::Array(a) => a.entry_at(p).map(|(i, k, v)| (i, k.to_value(), v.clone())),
            _ => None,
        };
        match step {
            Some((i, k, v)) => {
                fr.locals[pos as usize] = Value::Int(i as i64 + 1);
                if key != NO_SLOT {
                    fr.locals[key as usize] = k;
                }
                fr.locals[val as usize] = v;
                Ok(false)
            }
            None => Ok(true),
        }
    }

    fn step_iter(&mut self, fr: &mut Frame, arr: u16, pos: u16, key: u16, val: u16) -> R<bool> {
        let it = fr.locals[arr as usize].clone();
        let Value::Object(io) = &it else { return Ok(true) };
        if !io.class.is_a("iterator") {
            return Ok(true);
        }
        let started = fr.locals[pos as usize].to_int() > 0;
        self.call_method_named(&it, if started { "next" } else { "rewind" }, vec![])?;
        fr.locals[pos as usize] = Value::Int(1);
        if !self.call_method_named(&it, "valid", vec![])?.to_bool() {
            return Ok(true);
        }
        fr.locals[val as usize] = self.call_method_named(&it, "current", vec![])?;
        if key != NO_SLOT {
            fr.locals[key as usize] = self.call_method_named(&it, "key", vec![])?;
        }
        Ok(false)
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
            Value::Object(o) => Value::Array(Rc::new(o.props.borrow().clone())),
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
