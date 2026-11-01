//! Stack-machine interpreter with classes, closures and exceptions.
use crate::ast::{ClassKind, TypeHint};
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

pub(crate) struct Frame {
    /// All actual arguments as passed (for func_get_args).
    all_args: Vec<Value>,
    dyn_vars: HashMap<String, Value>,
    undef_mask: u64,
    locals: Vec<Value>,
    st: Vec<Value>,
    pc: usize,
    argc: usize,
    ctx: CallCtx,
}

/// State of a suspended generator function.
pub struct GenState {
    frame: Option<Frame>,
    func: usize,
    started: bool,
    done: bool,
    running: bool,
    key: Value,
    val: Value,
    auto_key: i64,
    ret: Value,
}

pub enum Step {
    Return(Value),
    Yield(Option<Value>, Value),
}

pub struct Vm<'p> {
    pub prog: &'p Program,
    prog_ptr: *mut Program,
    pub included: std::collections::HashSet<String>,
    pub out: String,
    pub file: String,
    pub classes: HashMap<String, Rc<Class>>,
    pub constants: HashMap<String, Value>,
    pub globals: HashMap<String, Rc<RefCell<Value>>>,
    pub exception_handler: Option<Value>,
    pub error_handler: Option<Value>,
    pub shutdown: Vec<(Value, Vec<Value>)>,
    pub fiber_stack: Vec<Value>,
    pub json_error: (i64, &'static str),
    /// stream_wrapper_register(): scheme -> class name.
    pub stream_wrappers: HashMap<String, String>,
    /// Output-buffer stack (`ob_start`); the last entry receives output.
    pub ob: Vec<String>,
    statics: HashMap<u32, Rc<RefCell<Value>>>,
    depth: usize,
}

const MAX_DEPTH: usize = 20000;

fn lc(s: &str) -> String {
    s.to_ascii_lowercase()
}

impl<'p> Vm<'p> {
    pub fn new(prog: &'p mut Program) -> Self {
        let prog_ptr: *mut Program = prog;
        // SAFETY: the program is only extended (never shrunk or reordered) while running; funcs are boxed so
        // references to existing functions stay valid.
        let prog: &'p Program = unsafe { &*prog_ptr };
        Vm { prog, prog_ptr, included: Default::default(), out: String::new(), file: String::new(), classes: HashMap::new(), constants: HashMap::new(), globals: HashMap::new(), exception_handler: None, error_handler: None, shutdown: Vec::new(), fiber_stack: Vec::new(), json_error: (0, "No error"), stream_wrappers: HashMap::new(), ob: Vec::new(), statics: HashMap::new(), depth: 0 }
    }

    /// Append to the active output buffer, or to the real output.
    pub fn write(&mut self, s: &str) {
        match self.ob.last_mut() {
            Some(b) => b.push_str(s),
            None => self.out.push_str(s),
        }
    }

    /// Flush every open output buffer into the real output (end of script).
    pub fn flush_buffers(&mut self) {
        while let Some(b) = self.ob.pop() {
            self.write(&b);
        }
    }

    pub fn run_main(&mut self) -> R<()> {
        self.declare_classes();
        let prog = self.prog;
        let mut result = self.exec(&prog.main, vec![], CallCtx::default()).map(|_| ());
        if let (Err(Flow::Throw(ex)), Some(h)) = (&result, self.exception_handler.take()) {
            let ex = ex.clone();
            result = self.call_value(&h, vec![ex]).map(|_| ());
        }
        // Shutdown functions run even after exit() or an uncaught exception.
        let mut i = 0;
        while i < self.shutdown.len() {
            let (f, args) = self.shutdown[i].clone();
            i += 1;
            match self.call_value(&f, args) {
                Err(Flow::Exit(_)) => break,
                Err(e) if result.is_ok() => result = Err(e),
                _ => {}
            }
        }
        result
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
        if def.kind == ClassKind::Enum {
            interfaces.push("unitenum".into());
            if def.backed {
                interfaces.push("backedenum".into());
            }
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
                let info = PropInfo { ty: p.ty.clone(), readonly: p.readonly, name: p.name.clone(), thunk: p.thunk, vis: p.vis, class: def.name.clone() };
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
            cases: def.cases.clone(),
            backed: def.backed,
            enum_vals: RefCell::new(None),
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

    pub fn call_thunk(&mut self, func: usize, class: &Rc<Class>) -> R<Value> {
        self.run_thunk(func, class)
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
        if class.kind == ClassKind::Enum && class.cases.iter().any(|(n, _)| n == name) {
            let vals = self.enum_values(class)?;
            return Ok(vals.into_iter().find(|(n, _)| n == name).map(|(_, v)| v).unwrap_or(Value::Null));
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
        if class.name == "Fiber" {
            return Ok(crate::fiber::create(class.clone(), args.into_iter().next().unwrap_or(Value::Null)));
        }
        if class.kind != ClassKind::Class || class.is_abstract {
            let what = match class.kind {
                ClassKind::Interface => "interface",
                ClassKind::Trait => "trait",
                ClassKind::Enum => "enum",
                ClassKind::Class => "abstract class",
            };
            return Err(Flow::Fatal(format!("Cannot instantiate {} {}", what, class.name)));
        }
        let mut props = PhpArray::new();
        for p in &class.props {
            if p.thunk.is_none() && p.ty.is_some() {
                continue; // typed property without default stays uninitialized
            }
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
        let f = &prog.funcs[idx];
        let r = if f.is_generator {
            let fr = self.build_frame(f, args, ctx);
            self.make_generator(idx, fr)
        } else {
            self.exec(f, args, ctx)
        };
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
                    _ => None,
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

    fn build_frame(&mut self, f: &'p Func, args: Vec<Value>, ctx: CallCtx) -> Frame {
        let mut locals = vec![Value::Null; f.nlocals];
        if let Some(t) = &ctx.this {
            locals[0] = t.clone();
        }
        let argc = args.len();
        let mut undef_mask = 0u64;
        let all_args = args.clone();
        let mut it = args.into_iter();
        for i in 0..f.nparams {
            let by_ref = f.ref_params.get(i).copied().unwrap_or(false);
            if f.variadic && i + 1 == f.nparams {
                let rest: Vec<Value> = it.by_ref().map(|a| if by_ref { a } else { unref(a) }).collect();
                locals[1 + i] = Value::Array(Rc::new(PhpArray::from_list(rest)));
                break;
            }
            match it.next() {
                Some(a) if is_undef(&a) => {
                    if i < 64 {
                        undef_mask |= 1 << i;
                    }
                }
                Some(a) => locals[1 + i] = if by_ref { a } else { unref(a) },
                None => break,
            }
        }
        for (slot, v) in f.capture_slots.iter().zip(ctx.caps.iter()) {
            locals[*slot as usize] = v.clone();
        }
        if std::ptr::eq(f, &self.prog.main) {
            for name in &self.prog.global_names {
                if let Some(&slot) = self.prog.main_locals.get(name) {
                    let cell = self.globals.entry(name.clone()).or_insert_with(|| Rc::new(RefCell::new(Value::Null))).clone();
                    locals[slot as usize] = Value::Ref(cell);
                }
            }
        }
        Frame { all_args, dyn_vars: HashMap::new(), undef_mask, locals, st: Vec::with_capacity(16), pc: 0, argc, ctx }
    }

    fn exec(&mut self, f: &'p Func, args: Vec<Value>, ctx: CallCtx) -> R<Value> {
        let mut fr = self.build_frame(f, args, ctx);
        match self.run_frame(f, &mut fr, None)? {
            Step::Return(v) => Ok(v),
            Step::Yield(..) => Err(Flow::Fatal("Cannot yield outside a generator".into())),
        }
    }

    /// Run a frame to completion or the next `yield`, routing exceptions to its handlers.
    fn run_frame(&mut self, f: &'p Func, fr: &mut Frame, mut pending: Option<Flow>) -> R<Step> {
        loop {
            let err = match pending.take() {
                Some(e) => e,
                None => match self.run(f, fr) {
                    Ok(step) => return Ok(step),
                    Err(e) => e,
                },
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

    fn run(&mut self, f: &'p Func, fr: &mut Frame) -> R<Step> {
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
                Op::Load(s) => {
                    let v = get_local(&fr.locals, s);
                    fr.st.push(v);
                }
                Op::Store(s) => {
                    let v = fr.st.last().cloned().unwrap_or(Value::Null);
                    set_local(&mut fr.locals, s, v);
                }
                Op::Pop => {
                    fr.st.pop();
                }
                Op::Echo => {
                    let v = pop!();
                    let s = self.to_string_val(&v)?;
                    self.write(&s);
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
                    let r = if k == "string" {
                        Value::Str(self.to_string_val(&v)?)
                    } else if k == "object" {
                        let std = self.find_class("stdClass")?;
                        match v {
                            Value::Object(_) => v,
                            Value::Array(a) => Value::Object(Obj::new(std, (*a).clone(), Extra::None)),
                            Value::Null => Value::Object(Obj::new(std, PhpArray::new(), Extra::None)),
                            other => {
                                let mut p = PhpArray::new();
                                p.set(Key::from_str("scalar"), other);
                                Value::Object(Obj::new(std, p, Extra::None))
                            }
                        }
                    } else {
                        cast(k, v)
                    };
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
                    if let Value::Object(o) = get_local(&fr.locals, slot) {
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
                    let r = with_local_mut(&mut fr.locals, slot, |cur| assign_dim(cur, &keys, v.clone()));
                    r?;
                    fr.st.push(v);
                }
                Op::UnsetDim(slot, shape) => {
                    let n = prog.shapes[shape as usize].len();
                    let ks = split_args!(n);
                    let keys: Vec<Key> = ks.iter().map(Key::from_value).collect::<Result<_, _>>()?;
                    with_local_mut(&mut fr.locals, slot, |cur| unset_dim(cur, &keys));
                }
                Op::Eval => {
                    let code = pop!().to_str();
                    let r = self.run_snippet(fr, f, format!("<?php {code}"), "eval()'d code", false)?;
                    fr.st.push(r);
                }
                Op::Include(kind) => {
                    let path = pop!().to_str();
                    let r = self.include_file(fr, f, &path, kind)?;
                    fr.st.push(r);
                }
                Op::FuncArgs(kind) => {
                    let n = fr.all_args.len();
                    match kind {
                        0 => {
                            let items: Vec<Value> = fr.all_args.iter().map(|v| v.deref()).collect();
                            fr.st.push(Value::Array(Rc::new(PhpArray::from_list(items))));
                        }
                        1 => fr.st.push(Value::Int(n as i64)),
                        _ => {
                            let idx = pop!().to_int();
                            if idx < 0 || idx as usize >= n {
                                return self.throw_error("ValueError", "func_get_arg(): Argument #1 ($position) must be less than the number of the arguments passed to the currently executed function");
                            }
                            fr.st.push(fr.all_args[idx as usize].deref());
                        }
                    }
                }
                Op::LoadVar => {
                    let name = pop!().to_str();
                    let v = match f.local_names.iter().position(|n| **n == *name) {
                        Some(slot) => get_local(&fr.locals, slot as u16),
                        None => fr.dyn_vars.get(&*name).cloned().unwrap_or(Value::Null),
                    };
                    fr.st.push(v);
                }
                Op::StoreVar => {
                    let v = pop!();
                    let name = pop!().to_str();
                    match f.local_names.iter().position(|n| **n == *name) {
                        Some(slot) => set_local(&mut fr.locals, slot as u16, v.clone()),
                        None => {
                            fr.dyn_vars.insert(name.to_string(), v.clone());
                        }
                    }
                    fr.st.push(v);
                }
                Op::Compact(argc) => {
                    let names = split_args!(argc);
                    let mut out = PhpArray::new();
                    let mut add = |n: &str, fr: &Frame| {
                        let found = match f.local_names.iter().position(|x| x == n) {
                            Some(slot) => Some(get_local(&fr.locals, slot as u16)),
                            None => fr.dyn_vars.get(n).cloned(),
                        };
                        if let Some(v) = found.filter(|v| !matches!(v, Value::Null)) {
                            out.set(Key::from_str(n), v);
                        }
                    };
                    for n in &names {
                        match n {
                            Value::Array(a) => a.iter().for_each(|(_, x)| add(&x.to_str(), fr)),
                            other => add(&other.to_str(), fr),
                        }
                    }
                    fr.st.push(Value::Array(Rc::new(out)));
                }
                Op::Extract => {
                    let src = pop!();
                    let mut n = 0;
                    if let Value::Array(a) = &src {
                        for (k, v) in a.iter() {
                            if let Key::Str(name) = k {
                                match f.local_names.iter().position(|x| **x == **name) {
                                    Some(slot) => set_local(&mut fr.locals, slot as u16, v.clone()),
                                    None => {
                                        fr.dyn_vars.insert(name.to_string(), v.clone());
                                    }
                                }
                                n += 1;
                            }
                        }
                    }
                    fr.st.push(Value::Int(n));
                }
                Op::GetDefinedVars => {
                    let mut out = PhpArray::new();
                    for (slot, name) in f.local_names.iter().enumerate() {
                        if name.starts_with('#') || name == "this" {
                            continue;
                        }
                        let v = get_local(&fr.locals, slot as u16);
                        if !matches!(v, Value::Null) {
                            out.set(Key::from_str(name), v);
                        }
                    }
                    fr.st.push(Value::Array(Rc::new(out)));
                }
                Op::UnsetPropDim(shape) => {
                    let n = prog.shapes[shape as usize].len();
                    let ks = split_args!(n);
                    let keys: Vec<Key> = ks.iter().map(Key::from_value).collect::<Result<_, _>>()?;
                    let name = pop!().to_str();
                    let obj = pop!();
                    if let Value::Object(o) = &obj {
                        if keys.len() == 1 && o.class.find_method("offsetunset").is_some() && o.props.borrow().get(&Key::from_str(&name)).is_none() {
                            let m = o.class.find_method("offsetunset").unwrap();
                            self.call_method(&obj, &m, vec![keys[0].to_value()])?;
                        } else {
                            let pk = Key::from_str(&name);
                            let mut cur = o.props.borrow_mut().get_mut(&pk).map(|x| std::mem::replace(x, Value::Null));
                            if let Some(c) = cur.as_mut() {
                                unset_dim(c, &keys);
                            }
                            if let Some(c) = cur {
                                o.props.borrow_mut().set(pk, c);
                            }
                        }
                    }
                }
                Op::UnsetStaticDim(shape) => {
                    let n = prog.shapes[shape as usize].len();
                    let ks = split_args!(n);
                    let keys: Vec<Key> = ks.iter().map(Key::from_value).collect::<Result<_, _>>()?;
                    let name = pop!().to_str();
                    let cv = pop!();
                    let class = self.class_of(&cv)?;
                    let owner = self.static_owner(&class, &name)?;
                    let mut cur = owner.statics.borrow_mut().remove(&*name).unwrap_or(Value::Null);
                    unset_dim(&mut cur, &keys);
                    owner.statics.borrow_mut().insert(name.to_string(), cur);
                }
                Op::IncDec(slot, inc, prefix) => {
                    let old = get_local(&fr.locals, slot);
                    let new = incdec(&old, inc);
                    set_local(&mut fr.locals, slot, new.clone());
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
                                set_local(&mut fr.locals, key, k);
                            }
                            set_local(&mut fr.locals, val, v);
                        }
                        None => fr.pc = exit as usize,
                    }
                }
                Op::IterNextRef { arr, pos, key, val, exit } => {
                    let cell = match &fr.locals[arr as usize] {
                        Value::Ref(r) => r.clone(),
                        _ => {
                            fr.pc = exit as usize;
                            continue;
                        }
                    };
                    let p = fr.locals[pos as usize].to_int() as usize;
                    write_back_elem(&cell, &fr.locals[val as usize], p);
                    let step = match &*cell.borrow() {
                        Value::Array(a) => a.entry_at(p).map(|(i, k, v)| (i, k.to_value(), v.clone())),
                        _ => None,
                    };
                    match step {
                        Some((i, k, v)) => {
                            fr.locals[pos as usize] = Value::Int(i as i64 + 1);
                            if key != NO_SLOT {
                                set_local(&mut fr.locals, key, k);
                            }
                            fr.locals[val as usize] = Value::Ref(Rc::new(RefCell::new(v)));
                        }
                        None => fr.pc = exit as usize,
                    }
                }
                Op::RefFlush { arr, pos, val } => {
                    if let Value::Ref(cell) = fr.locals[arr as usize].clone() {
                        let p = fr.locals[pos as usize].to_int() as usize;
                        write_back_elem(&cell, &fr.locals[val as usize], p);
                    }
                }
                Op::Call(k, argc, site) => {
                    let mut args = split_args!(argc);
                    let Value::Str(name) = &prog.consts[k as usize] else { unreachable!() };
                    let r = if let Some(&idx) = prog.func_index.get(&**name) {
                        self.bind_ref_args(fr, site, Some(idx), &mut args);
                        self.call_func(idx, args, CallCtx::default())?
                    } else if let Some(r) = builtins::call(self, name, args) {
                        r?
                    } else {
                        return self.throw_error("Error", &format!("Call to undefined function {name}()"));
                    };
                    fr.st.push(r);
                }
                Op::CallValue(argc, site) => {
                    let mut args = split_args!(argc);
                    let callee = pop!();
                    if let Value::Object(o) = &callee {
                        if let Extra::Closure(cd) = &*o.extra.borrow() {
                            self.bind_ref_args(fr, site, Some(cd.func), &mut args);
                        }
                    }
                    let r = self.call_value(&callee, args)?;
                    fr.st.push(r);
                }
                Op::Ret => return Ok(Step::Return(pop!())),
                Op::Yield(has_key) => {
                    let v = pop!();
                    let k = if has_key { Some(pop!()) } else { None };
                    return Ok(Step::Yield(k, v));
                }
                Op::Exit => {
                    let v = pop!();
                    return Err(match v {
                        Value::Int(i) => Flow::Exit(i as i32),
                        Value::Null => Flow::Exit(0),
                        v => {
                            self.write(&v.to_str());
                            Flow::Exit(0)
                        }
                    });
                }
                Op::Fatal(k) => {
                    let Value::Str(m) = &prog.consts[k as usize] else { unreachable!() };
                    return Err(Flow::Fatal(m.to_string()));
                }
                Op::SkipIfArg(i, t) => {
                    if fr.argc > i as usize && (i >= 64 || fr.undef_mask & (1 << i) == 0) {
                        fr.pc = t as usize;
                    }
                }
                Op::Halt => return Ok(Step::Return(Value::Null)),
                Op::CoerceParam(slot, ty, i) => {
                    if (i as usize) < fr.argc {
                        let v = get_local(&fr.locals, slot);
                        let ty = &prog.types[ty as usize];
                        match self.coerce_to(v.clone(), ty, &fr.ctx)? {
                            Some(nv) => set_local(&mut fr.locals, slot, nv),
                            None => {
                                let pname = f.param_names.get(i as usize).cloned().unwrap_or_default();
                                let msg = format!(
                                    "{}(): Argument #{} (${}) must be of type {}, {} given, called in {} on line 0",
                                    f.name, i + 1, pname, ty.display, type_name_of(&v), self.file
                                );
                                return self.throw_error("TypeError", &msg);
                            }
                        }
                    }
                }
                Op::CheckRet(ty) => {
                    if !f.is_generator {
                        let v = pop!();
                        let ty = &prog.types[ty as usize];
                        match self.coerce_to(v.clone(), ty, &fr.ctx)? {
                            Some(nv) => fr.st.push(nv),
                            None => {
                                let msg = format!("{}(): Return value must be of type {}, {} returned", f.name, ty.display, type_name_of(&v));
                                return self.throw_error("TypeError", &msg);
                            }
                        }
                    }
                }
                Op::RetNone(ty) => {
                    if !f.is_generator {
                        let ty = &prog.types[ty as usize];
                        let msg = format!("{}(): Return value must be of type {}, none returned", f.name, ty.display);
                        return self.throw_error("TypeError", &msg);
                    }
                }
                Op::MakeRef(slot) => {
                    let cell = make_ref_cell(&mut fr.locals[slot as usize]);
                    fr.st.push(Value::Ref(cell));
                }
                Op::BindRef(slot) => {
                    let v = pop!();
                    fr.locals[slot as usize] = v;
                }
                Op::GlobalBind(slot, k) => {
                    let Value::Str(name) = &prog.consts[k as usize] else { unreachable!() };
                    let cell = self.globals.entry(name.to_string()).or_insert_with(|| Rc::new(RefCell::new(Value::Null))).clone();
                    fr.locals[slot as usize] = Value::Ref(cell);
                }
                Op::StaticBind(slot, key, skip) => {
                    if let Some(cell) = self.statics.get(&key) {
                        fr.locals[slot as usize] = Value::Ref(cell.clone());
                        fr.pc = skip as usize;
                    }
                }
                Op::StaticInit(slot, key) => {
                    let v = pop!();
                    let cell = Rc::new(RefCell::new(v));
                    self.statics.insert(key, cell.clone());
                    fr.locals[slot as usize] = Value::Ref(cell);
                }
                Op::CallRef(k, argc) => {
                    let mut args = split_args!(argc);
                    let Value::Str(name) = &prog.consts[k as usize] else { unreachable!() };
                    let r = match builtins::call_ref(self, name, &mut args) {
                        Some(r) => r?,
                        None => return self.throw_error("Error", &format!("Call to undefined function {name}()")),
                    };
                    fr.st.push(r);
                    for p in builtins::ref_positions(name) {
                        if *p < argc as usize {
                            fr.st.push(args.get(*p).cloned().unwrap_or(Value::Null));
                        }
                    }
                }
                Op::ArrSpread => {
                    let src = pop!();
                    let items = self.spread_items(&src)?;
                    if let Some(Value::Array(a)) = fr.st.last_mut() {
                        let arr = Rc::make_mut(a);
                        for (k, v) in items {
                            match k {
                                Key::Int(_) => arr.push(v),
                                k => arr.set(k, v),
                            }
                        }
                    }
                }
                Op::CallSpread(kind, k, forward) => {
                    let list = pop!();
                    let target = self.spread_target(fr, kind, k);
                    let args: Vec<Value> = match &list {
                        Value::Array(a) => self.arrange_args(target, a)?,
                        _ => vec![],
                    };
                    let r = match kind {
                        0 => {
                            let Value::Str(name) = &prog.consts[k as usize] else { unreachable!() };
                            self.call_function_named(name, args)?
                        }
                        1 => {
                            let callee = pop!();
                            self.call_value(&callee, args)?
                        }
                        2 => {
                            let name = pop!().to_str();
                            let obj = pop!();
                            self.call_method_named(&obj, &name, args)?
                        }
                        3 => {
                            let name = pop!().to_str();
                            let cv = pop!();
                            self.call_static_op(fr, &cv, &name, args, forward)?
                        }
                        _ => {
                            let cv = pop!();
                            let class = self.class_of(&cv)?;
                            self.instantiate(&class, args)?
                        }
                    };
                    fr.st.push(r);
                }
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
                Op::New(argc, site) => {
                    let mut args = split_args!(argc);
                    let cv = pop!();
                    let class = self.class_of(&cv)?;
                    let ctor = class.find_method("__construct").map(|m| m.func);
                    self.bind_ref_args(fr, site, ctor, &mut args);
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
                                None => {
                                    let typed = o.class.prop_info(&name).filter(|pi| pi.ty.is_some()).map(|pi| pi.class.clone());
                                    if let Some(c) = typed {
                                        return self.throw_error("Error", &format!("Typed property {c}::${name} must not be accessed before initialization"));
                                    }
                                    match o.class.find_method("__get") {
                                        Some(m) => self.call_method(&obj, &m, vec![Value::Str(name)])?,
                                        None => Value::Null,
                                    }
                                }
                            }
                        }
                        _ => Value::Null,
                    };
                    fr.st.push(r);
                }
                Op::AssignProp => {
                    let mut v = pop!();
                    let name = pop!().to_str();
                    let obj = pop!();
                    let Value::Object(o) = &obj else {
                        return self.throw_error("Error", &format!("Attempt to assign property \"{}\" on {}", name, obj.type_name()));
                    };
                    let key = Key::from_str(&name);
                    let exists = o.props.borrow().get(&key).is_some();
                    if let Some(pi) = o.class.prop_info(&name).cloned() {
                        if pi.readonly {
                            if exists {
                                return self.throw_error("Error", &format!("Cannot modify readonly property {}::${}", pi.class, name));
                            }
                            let in_scope = fr.ctx.self_class.as_ref().is_some_and(|c| c.name.eq_ignore_ascii_case(&pi.class));
                            if !in_scope {
                                let scope = fr.ctx.self_class.as_ref().map(|c| format!("scope {}", c.name)).unwrap_or_else(|| "global scope".into());
                                return self.throw_error("Error", &format!("Cannot initialize readonly property {}::${} from {}", pi.class, name, scope));
                            }
                        }
                        if let Some(ty) = &pi.ty {
                            match self.coerce_to(v.clone(), ty, &fr.ctx)? {
                                Some(nv) => v = nv,
                                None => {
                                    return self.throw_error("TypeError", &format!("Cannot assign {} to property {}::${} of type {}", type_name_of(&v), pi.class, name, ty.display));
                                }
                            }
                        }
                    }
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
                Op::CallMethod(argc, site) => {
                    let mut args = split_args!(argc);
                    let name = pop!().to_str();
                    let obj = pop!();
                    if let Value::Object(o) = &obj {
                        let target = o.class.find_method(&lc(&name)).map(|m| m.func);
                        self.bind_ref_args(fr, site, target, &mut args);
                    }
                    let r = self.call_method_named(&obj, &name, args)?;
                    fr.st.push(r);
                }
                Op::CallStatic(argc, forward, site) => {
                    let mut args = split_args!(argc);
                    let name = pop!().to_str();
                    let cv = pop!();
                    if let Ok(c) = self.class_of(&cv) {
                        let target = c.find_method(&lc(&name)).map(|m| m.func);
                        self.bind_ref_args(fr, site, target, &mut args);
                    }
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
                        Extra::Generator(_) | Extra::Fiber(_) => return self.throw_error("Error", "Trying to clone an uncloneable object"),
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

    /// Turn plain-variable arguments into shared reference cells where the callee takes them by reference.
    fn bind_ref_args(&self, fr: &mut Frame, site: u32, func: Option<usize>, args: &mut [Value]) {
        let Some(fi) = func else { return };
        if site == u32::MAX {
            return;
        }
        let f = &self.prog.funcs[fi];
        if !f.ref_params.iter().any(|b| *b) {
            return;
        }
        for (i, slot) in self.prog.sites[site as usize].iter().enumerate() {
            if *slot == NO_SLOT || i >= args.len() {
                continue;
            }
            let by_ref = f.ref_params.get(i).copied().unwrap_or(false) || (f.variadic && i + 1 >= f.nparams && f.ref_params.last().copied().unwrap_or(false));
            if by_ref {
                args[i] = Value::Ref(make_ref_cell(&mut fr.locals[*slot as usize]));
            }
        }
    }

    pub fn enum_values(&mut self, class: &Rc<Class>) -> R<Vec<(String, Value)>> {
        if let Some(v) = &*class.enum_vals.borrow() {
            return Ok(v.clone());
        }
        let mut out = Vec::new();
        for (n, thunk) in &class.cases {
            let mut props = PhpArray::new();
            props.set(Key::from_str("name"), Value::str(n));
            if let Some(t) = thunk {
                let v = self.run_thunk(*t, class)?;
                props.set(Key::from_str("value"), v);
            }
            out.push((n.clone(), Value::Object(Obj::new(class.clone(), props, Extra::None))));
        }
        *class.enum_vals.borrow_mut() = Some(out.clone());
        Ok(out)
    }

    fn enum_static(&mut self, class: &Rc<Class>, lname: &str, args: &[Value]) -> R<Option<Value>> {
        match lname {
            "cases" => {
                let vals = self.enum_values(class)?;
                Ok(Some(Value::Array(Rc::new(PhpArray::from_list(vals.into_iter().map(|(_, v)| v).collect())))))
            }
            "from" | "tryfrom" if class.backed => {
                let needle = args.first().cloned().unwrap_or(Value::Null);
                let vals = self.enum_values(class)?;
                for (_, v) in &vals {
                    if let Value::Object(o) = v {
                        if o.props.borrow().get(&Key::from_str("value")).is_some_and(|x| loose_eq(x, &needle) && matches!((x, &needle), (Value::Int(_), Value::Int(_) | Value::Str(_) | Value::Float(_)) | (Value::Str(_), Value::Str(_) | Value::Int(_)))) {
                            return Ok(Some(v.clone()));
                        }
                    }
                }
                if lname == "tryfrom" {
                    return Ok(Some(Value::Null));
                }
                let repr = match &needle {
                    Value::Str(s) => format!("\"{s}\""),
                    other => other.to_str().to_string(),
                };
                self.throw_error("ValueError", &format!("{repr} is not a valid backing value for enum {}", class.name))
            }
            _ => Ok(None),
        }
    }

    fn type_matches(&self, v: &Value, n: &str, ctx: &CallCtx) -> bool {
        match (n, v) {
            ("mixed", _) => true,
            ("int", Value::Int(_)) | ("float", Value::Float(_)) | ("string", Value::Str(_)) | ("bool", Value::Bool(_)) => true,
            ("false", Value::Bool(false)) | ("true", Value::Bool(true)) | ("array", Value::Array(_)) | ("null", Value::Null) => true,
            ("object", Value::Object(_)) => true,
            ("iterable", Value::Array(_)) => true,
            ("iterable", Value::Object(o)) => o.class.is_a("traversable"),
            ("callable", Value::Object(o)) => matches!(&*o.extra.borrow(), Extra::Closure(_)) || o.class.find_method("__invoke").is_some(),
            ("callable", Value::Str(s)) => self.prog.func_index.contains_key(&lc(s)) || builtins::exists(&lc(s)) || s.contains("::"),
            ("callable", Value::Array(a)) => a.len() == 2,
            ("self", Value::Object(o)) => ctx.self_class.as_ref().is_some_and(|c| o.class.is_a(&lc(&c.name))),
            ("static", Value::Object(o)) => ctx.static_class.as_ref().or(ctx.self_class.as_ref()).is_some_and(|c| o.class.is_a(&lc(&c.name))),
            (_, Value::Object(o)) => o.class.is_a(n),
            _ => false,
        }
    }

    /// Coerce `v` to type `ty` using PHP's weak-mode rules. `None` means a type mismatch.
    fn coerce_to(&mut self, v: Value, ty: &TypeHint, ctx: &CallCtx) -> R<Option<Value>> {
        if matches!(v, Value::Null) {
            return Ok(if ty.nullable || ty.names.iter().any(|n| n == "mixed" || n == "null") { Some(v) } else { None });
        }
        if ty.display.contains('&') {
            return Ok(ty.names.iter().all(|n| self.type_matches(&v, n, ctx)).then_some(v));
        }
        if ty.names.iter().any(|n| self.type_matches(&v, n, ctx)) {
            return Ok(Some(v));
        }
        let has = |t: &str| ty.names.iter().any(|n| n == t);
        if has("int") {
            match &v {
                Value::Float(f) if f.is_finite() && (f.fract() == 0.0 || !has("string")) => return Ok(Some(Value::Int(*f as i64))),
                Value::Bool(b) => return Ok(Some(Value::Int(*b as i64))),
                Value::Str(s) => match parse_numeric(s) {
                    Some((Num::I(i), true)) => return Ok(Some(Value::Int(i))),
                    Some((Num::F(f), true)) if f.is_finite() && !has("float") => return Ok(Some(Value::Int(f as i64))),
                    _ => {}
                },
                _ => {}
            }
        }
        if has("float") {
            match &v {
                Value::Int(i) => return Ok(Some(Value::Float(*i as f64))),
                Value::Bool(b) => return Ok(Some(Value::Float(*b as i64 as f64))),
                Value::Str(s) => {
                    if let Some((n, true)) = parse_numeric(s) {
                        return Ok(Some(Value::Float(n.f())));
                    }
                }
                _ => {}
            }
        }
        if has("string") {
            match &v {
                Value::Int(_) | Value::Float(_) | Value::Bool(_) => return Ok(Some(Value::Str(v.to_str()))),
                Value::Object(o) if o.class.find_method("__tostring").is_some() => return Ok(Some(Value::Str(self.to_string_val(&v)?))),
                _ => {}
            }
        }
        if has("bool") {
            if let Value::Int(_) | Value::Float(_) | Value::Str(_) = &v {
                return Ok(Some(Value::Bool(v.to_bool())));
            }
        }
        Ok(None)
    }

    fn make_generator(&mut self, func: usize, frame: Frame) -> R<Value> {
        let class = self.find_class("Generator")?;
        let st = GenState { frame: Some(frame), func, started: false, done: false, running: false, key: Value::Null, val: Value::Null, auto_key: 0, ret: Value::Null };
        Ok(Value::Object(Obj::new(class, PhpArray::new(), Extra::Generator(Rc::new(RefCell::new(st))))))
    }

    fn gen_advance(&mut self, g: &Rc<RefCell<GenState>>, send: Option<Value>, throw: Option<Value>) -> R<()> {
        let (mut fr, func, was_started) = {
            let mut st = g.borrow_mut();
            if st.done {
                return Ok(());
            }
            if st.running {
                drop(st);
                return self.throw_error("Error", "Cannot resume an already running generator");
            }
            let Some(fr) = st.frame.take() else { return Ok(()) };
            st.running = true;
            let was = st.started;
            st.started = true;
            (fr, st.func, was)
        };
        let prog = self.prog;
        let f = &prog.funcs[func];
        let mut pending = None;
        if was_started {
            match throw {
                Some(ex) => pending = Some(Flow::Throw(ex)),
                None => fr.st.push(send.unwrap_or(Value::Null)),
            }
        }
        let res = self.run_frame(f, &mut fr, pending);
        let mut st = g.borrow_mut();
        st.running = false;
        match res {
            Ok(Step::Yield(k, v)) => {
                st.val = v;
                st.key = match k {
                    Some(k) => {
                        if let Value::Int(i) = &k {
                            if *i >= st.auto_key {
                                st.auto_key = *i + 1;
                            }
                        }
                        k
                    }
                    None => {
                        let k = Value::Int(st.auto_key);
                        st.auto_key += 1;
                        k
                    }
                };
                st.frame = Some(fr);
                Ok(())
            }
            Ok(Step::Return(v)) => {
                st.done = true;
                st.ret = v;
                st.val = Value::Null;
                st.key = Value::Null;
                Ok(())
            }
            Err(e) => {
                st.done = true;
                st.val = Value::Null;
                st.key = Value::Null;
                Err(e)
            }
        }
    }

    fn generator_method(&mut self, g: &Rc<RefCell<GenState>>, lname: &str, args: Vec<Value>) -> R<Value> {
        if !g.borrow().started && lname != "getreturn" {
            self.gen_advance(g, None, None)?;
            if matches!(lname, "rewind" | "current" | "key" | "valid") {
                // fallthrough to the accessors below
            }
        }
        match lname {
            "current" => Ok(g.borrow().val.clone()),
            "key" => Ok(g.borrow().key.clone()),
            "valid" => Ok(Value::Bool(!g.borrow().done)),
            "rewind" => Ok(Value::Null),
            "next" => {
                self.gen_advance(g, None, None)?;
                Ok(Value::Null)
            }
            "send" => {
                self.gen_advance(g, Some(args.into_iter().next().unwrap_or(Value::Null)), None)?;
                Ok(g.borrow().val.clone())
            }
            "throw" => {
                self.gen_advance(g, None, Some(args.into_iter().next().unwrap_or(Value::Null)))?;
                Ok(g.borrow().val.clone())
            }
            "getreturn" => Ok(g.borrow().ret.clone()),
            other => self.throw_error("Error", &format!("Call to undefined method Generator::{other}()")),
        }
    }

    /// The user function a spread/named call will invoke, if it can be determined from the stack.
    fn spread_target(&self, fr: &Frame, kind: u8, k: u32) -> Option<usize> {
        let n = fr.st.len();
        match kind {
            0 => {
                let Value::Str(name) = &self.prog.consts[k as usize] else { return None };
                self.prog.func_index.get(&**name).copied()
            }
            1 => match fr.st.last()? {
                Value::Object(o) => match &*o.extra.borrow() {
                    Extra::Closure(c) => Some(c.func),
                    _ => o.class.find_method("__invoke").map(|m| m.func),
                },
                Value::Str(s) => self.prog.func_index.get(&lc(s)).copied(),
                _ => None,
            },
            2 => match fr.st.get(n.checked_sub(2)?)? {
                Value::Object(o) => o.class.find_method(&lc(&fr.st.last()?.to_str())).map(|m| m.func),
                _ => None,
            },
            3 => {
                let class = self.class_of(fr.st.get(n.checked_sub(2)?)?).ok()?;
                class.find_method(&lc(&fr.st.last()?.to_str())).map(|m| m.func)
            }
            _ => {
                let class = self.class_of(fr.st.last()?).ok()?;
                class.find_method("__construct").map(|m| m.func)
            }
        }
    }

    /// Turn an argument array (positional and `name => value` entries) into a positional argument list.
    fn arrange_args(&mut self, target: Option<usize>, list: &PhpArray) -> R<Vec<Value>> {
        let mut args: Vec<Value> = Vec::new();
        let mut named: Vec<(String, Value)> = Vec::new();
        for (k, v) in list.iter() {
            match k {
                Key::Int(_) if named.is_empty() => args.push(v.clone()),
                Key::Int(_) => return self.throw_error("Error", "Cannot use positional argument after named argument"),
                Key::Str(n) => named.push((n.to_string(), v.clone())),
            }
        }
        for (name, v) in named {
            let Some(fi) = target else {
                args.push(v);
                continue;
            };
            let f = &self.prog.funcs[fi];
            match f.param_names.iter().position(|p| *p == name) {
                Some(i) => {
                    while args.len() <= i {
                        args.push(undef());
                    }
                    if !is_undef(&args[i]) {
                        return self.throw_error("Error", &format!("Named parameter ${name} overwrites previous argument"));
                    }
                    args[i] = v;
                }
                None => return self.throw_error("Error", &format!("Unknown named parameter ${name}")),
            }
        }
        Ok(args)
    }

    /// Compile and run source text in the caller's variable scope (`eval`, `include`).
    fn run_snippet(&mut self, fr: &mut Frame, f: &'p Func, src: String, file: &str, is_include: bool) -> R<Value> {
        let stmts = match crate::parser::parse(&src) {
            Ok(s) => s,
            Err(e) => return self.throw_error("ParseError", e.trim_start_matches("PHP Parse error: ")),
        };
        // SAFETY: see Vm::new; no references into the program's vectors are held across this call.
        let prog = unsafe { &mut *self.prog_ptr };
        let first_class = prog.classes.len();
        let idx = match crate::compiler::compile_snippet(prog, &stmts) {
            Ok(i) => i,
            Err(e) => return self.throw_error("Error", &e),
        };
        let mut pending: Vec<usize> = (first_class..prog.classes.len()).collect();
        loop {
            let before = pending.len();
            pending.retain(|&i| !self.try_declare(i));
            if pending.is_empty() || pending.len() == before {
                break;
            }
        }
        let sf: &'p Func = &self.prog.funcs[idx];
        let mut sframe = self.build_frame(sf, vec![], fr.ctx.clone());
        for (slot, name) in sf.local_names.iter().enumerate() {
            if name.is_empty() || name == "this" {
                continue;
            }
            if let Some(cs) = f.local_names.iter().position(|n| n == name) {
                sframe.locals[slot] = Value::Ref(make_ref_cell(&mut fr.locals[cs]));
            } else if let Some(v) = fr.dyn_vars.get(name) {
                sframe.locals[slot] = v.clone();
            }
        }
        let saved_file = std::mem::replace(&mut self.file, file.to_string());
        let result = self.run_frame(sf, &mut sframe, None);
        self.file = saved_file;
        for (slot, name) in sf.local_names.iter().enumerate() {
            if name.is_empty() || name == "this" || name.starts_with('#') || f.local_names.contains(name) {
                continue;
            }
            fr.dyn_vars.insert(name.clone(), get_local(&sframe.locals, slot as u16));
        }
        match result? {
            Step::Return(v) => Ok(if is_include && matches!(v, Value::Null) { Value::Int(1) } else { v }),
            Step::Yield(..) => Ok(Value::Null),
        }
    }

    fn include_file(&mut self, fr: &mut Frame, f: &'p Func, path: &str, kind: u8) -> R<Value> {
        let require = kind >= 2;
        let once = kind % 2 == 1;
        let mut candidates = vec![std::path::PathBuf::from(path)];
        if !path.starts_with('/') {
            if let Some(dir) = std::path::Path::new(&self.file).parent() {
                candidates.push(dir.join(path));
            }
        }
        let found = candidates.into_iter().find(|p| p.is_file());
        let Some(found) = found else {
            let verb = ["include", "include_once", "require", "require_once"][kind as usize];
            if require {
                return self.throw_error("Error", &format!("Failed opening required '{path}' (include_path='.:/usr/share/php')"));
            }
            let _ = verb;
            return Ok(Value::Bool(false));
        };
        let canon = std::fs::canonicalize(&found).map(|p| p.display().to_string()).unwrap_or_else(|_| found.display().to_string());
        if once && !self.included.insert(canon.clone()) {
            return Ok(Value::Bool(true));
        }
        if !once {
            self.included.insert(canon.clone());
        }
        let src = std::fs::read_to_string(&found).unwrap_or_default();
        self.run_snippet(fr, f, src, &canon, true)
    }

    /// Elements of an array or Traversable, for `...$spread`.
    fn spread_items(&mut self, src: &Value) -> R<Vec<(Key, Value)>> {
        match src {
            Value::Array(a) => Ok(a.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            Value::Object(_) => {
                let mut it = src.clone();
                while matches!(&it, Value::Object(o) if o.class.is_a("iteratoraggregate")) {
                    it = self.call_method_named(&it, "getIterator", vec![])?;
                }
                let mut out = vec![];
                self.call_method_named(&it, "rewind", vec![])?;
                while self.call_method_named(&it, "valid", vec![])?.to_bool() {
                    let v = self.call_method_named(&it, "current", vec![])?;
                    let k = self.call_method_named(&it, "key", vec![])?;
                    out.push((Key::from_value(&k)?, v));
                    self.call_method_named(&it, "next", vec![])?;
                }
                Ok(out)
            }
            _ => self.throw_error("Error", "Only arrays and Traversables can be unpacked"),
        }
    }

    pub fn call_method_named(&mut self, obj: &Value, name: &str, args: Vec<Value>) -> R<Value> {
        let Value::Object(o) = obj else {
            return self.throw_error("Error", &format!("Call to a member function {}() on {}", name, obj.type_name()));
        };
        let lname = lc(name);
        let fiber = match &*o.extra.borrow() {
            Extra::Fiber(f) => Some(f.clone()),
            _ => None,
        };
        if let Some(f) = fiber {
            return crate::fiber::method(self, obj, &f, &lname, args);
        }
        let gen = match &*o.extra.borrow() {
            Extra::Generator(g) => Some(g.clone()),
            _ => None,
        };
        if let Some(g) = gen {
            return self.generator_method(&g, &lname, args);
        }
        let closure = match &*o.extra.borrow() {
            Extra::Closure(c) => Some(c.clone()),
            _ => None,
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
            if c.eq_ignore_ascii_case("fiber") {
                if let Some(r) = crate::fiber::static_call(self, &lc(name), args.clone()) {
                    return r;
                }
            }
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
        if class.kind == ClassKind::Enum {
            if let Some(v) = self.enum_static(&class, &lname, &args)? {
                return Ok(v);
            }
        }
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

fn unref(v: Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().deref(),
        other => other,
    }
}

fn get_local(locals: &[Value], s: u16) -> Value {
    match &locals[s as usize] {
        Value::Ref(r) => r.borrow().clone(),
        v => v.clone(),
    }
}

fn set_local(locals: &mut [Value], s: u16, v: Value) {
    if let Value::Ref(r) = &locals[s as usize] {
        *r.borrow_mut() = v;
        return;
    }
    locals[s as usize] = v;
}

/// Run `f` on the variable's value in place (through a reference cell if there is one).
fn with_local_mut<T>(locals: &mut [Value], s: u16, f: impl FnOnce(&mut Value) -> T) -> T {
    if let Value::Ref(r) = &locals[s as usize] {
        let cell = r.clone();
        let mut cur = std::mem::replace(&mut *cell.borrow_mut(), Value::Null);
        let out = f(&mut cur);
        *cell.borrow_mut() = cur;
        return out;
    }
    f(&mut locals[s as usize])
}

fn make_ref_cell(slot: &mut Value) -> Rc<RefCell<Value>> {
    match slot {
        Value::Ref(r) => r.clone(),
        v => {
            let cell = Rc::new(RefCell::new(std::mem::replace(v, Value::Null)));
            *v = Value::Ref(cell.clone());
            cell
        }
    }
}

/// foreach-by-reference: copy the loop variable's cell back into array element `p - 1`.
fn write_back_elem(arr_cell: &Rc<RefCell<Value>>, var: &Value, p: usize) {
    let Value::Ref(vc) = var else { return };
    if p == 0 {
        return;
    }
    let newv = vc.borrow().clone();
    if let Value::Array(rc) = &mut *arr_cell.borrow_mut() {
        let arr = Rc::make_mut(rc);
        if let Some(k) = arr.key_at(p - 1) {
            arr.set(k, newv);
        }
    }
}

/// Type name as shown in TypeError messages.
fn type_name_of(v: &Value) -> String {
    match v {
        Value::Object(o) => o.class.name.clone(),
        other => other.type_name().to_string(),
    }
}

/// Marker for a skipped argument in a named-argument call (so the parameter default applies).
fn undef() -> Value {
    Value::Str("\u{0}undef".into())
}

fn is_undef(v: &Value) -> bool {
    matches!(v, Value::Str(s) if &**s == "\u{0}undef")
}
