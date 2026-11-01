//! AST -> stack-machine bytecode.
use crate::ast::*;
use crate::builtins;
use crate::value::Value;
use std::collections::{HashMap, HashSet};

type R<T> = Result<T, String>;

pub const NO_SLOT: u16 = u16::MAX;

#[derive(Debug, Clone, Copy)]
pub enum Op {
    Const(u32),
    Load(u16),
    Store(u16), // peeks: leaves the value on the stack
    Pop,
    Echo,
    Bin(&'static str),
    Not,
    Neg,
    Plus,
    BitNot,
    ToBool,
    Cast(&'static str),
    IsNotNull,
    Jmp(u32),
    JmpZ(u32),
    JmpNZ(u32),
    JmpZKeep(u32),
    JmpNZKeep(u32),
    JmpNotNullKeep(u32),
    JmpNullKeep(u32),
    NewArray,
    ArrPush,
    ArrSet,
    Index,
    AssignDim(u16, u32),
    UnsetDim(u16, u32),
    IncDec(u16, bool, bool), // slot, inc, prefix
    IterNext { arr: u16, pos: u16, key: u16, val: u16, exit: u32 },
    Call(u32, u8, u32),
    CallRef(u32, u8),
    CallValue(u8, u32),
    CallSpread(u8, u32, bool),
    ArrSpread,
    MakeRef(u16),
    BindRef(u16),
    GlobalBind(u16, u32),
    StaticBind(u16, u32, u32),
    StaticInit(u16, u32),
    IterNextRef { arr: u16, pos: u16, key: u16, val: u16, exit: u32 },
    RefFlush { arr: u16, pos: u16, val: u16 },
    Ret,
    Exit,
    Fatal(u32),
    SkipIfArg(u8, u32),
    Halt,
    // Classes and objects. Class designators are strings or objects on the stack.
    PushSelf,
    PushParent,
    PushStatic,
    New(u8, u32),
    GetProp,
    AssignProp,
    AssignPropDim(u32),
    CallMethod(u8, u32),
    CallStatic(u8, bool, u32),
    ClassConst,
    GetStatic,
    AssignStatic,
    AssignStaticDim(u32),
    LoadVar,
    StoreVar,
    Compact(u8),
    Extract,
    GetDefinedVars,
    UnsetPropDim(u32),
    UnsetStaticDim(u32),
    InstanceOf,
    Clone,
    MakeClosure(u32, u8),
    // Exceptions and constants.
    Throw,
    Yield(bool),
    CoerceParam(u16, u32, u8),
    CheckRet(u32),
    RetNone(u32),
    JmpIfInstance(u32, u32),
    MatchError,
    ConstGet(u32),
    DefConst(u32),
    Eval,
    Include(u8),
}

#[derive(Debug, Clone, Copy)]
pub struct Handler {
    pub start: usize,
    pub end: usize,
    pub target: usize,
}

#[derive(Debug, Default)]
pub struct Func {
    pub name: String,
    pub nparams: usize,
    pub nlocals: usize,
    pub code: Vec<Op>,
    pub handlers: Vec<Handler>,
    pub capture_slots: Vec<u16>,
    pub ref_params: Vec<bool>,
    pub variadic: bool,
    pub is_generator: bool,
    pub param_names: Vec<String>,
    pub local_names: Vec<String>,
}

#[derive(Debug)]
pub struct MethodDef {
    pub name: String,
    pub func: usize,
    pub is_static: bool,
    pub is_abstract: bool,
    pub vis: u8,
}

#[derive(Debug)]
pub struct PropDef {
    pub ty: Option<TypeHint>,
    pub readonly: bool,
    pub name: String,
    pub thunk: Option<usize>,
    pub is_static: bool,
    pub vis: u8,
}

#[derive(Debug)]
pub struct ClassDef {
    pub name: String,
    pub kind: ClassKind,
    pub is_abstract: bool,
    pub parent: Option<String>,
    pub interfaces: Vec<String>,
    pub traits: Vec<String>,
    pub consts: Vec<(String, usize)>,
    pub props: Vec<PropDef>,
    pub methods: Vec<MethodDef>,
    pub cases: Vec<(String, Option<usize>)>,
    pub backed: bool,
}

#[derive(Debug, Default)]
pub struct Program {
    pub consts: Vec<Value>,
    pub shapes: Vec<Vec<bool>>,
    pub funcs: Vec<Box<Func>>,
    pub func_index: HashMap<String, usize>,
    pub classes: Vec<ClassDef>,
    pub main: Func,
    /// Argument-slot info for calls that pass plain variables (candidates for by-reference passing).
    pub sites: Vec<Vec<u16>>,
    pub main_locals: HashMap<String, u16>,
    pub global_names: HashSet<String>,
    pub static_count: u32,
    pub types: Vec<TypeHint>,
}

struct LoopCtx {
    breaks: Vec<usize>,
    continues: Vec<usize>,
    is_switch: bool,
}

struct Fc<'a> {
    prog: &'a mut Program,
    code: Vec<Op>,
    locals: HashMap<String, u16>,
    loops: Vec<LoopCtx>,
    /// Active `finally` blocks with the loop depth at which their `try` began.
    finallys: Vec<(Vec<Stmt>, usize)>,
    handlers: Vec<Handler>,
    has_yield: bool,
    ret_ty: Option<u32>,
}

fn new_fc(prog: &mut Program) -> Fc<'_> {
    let mut fc = Fc { prog, code: vec![], locals: HashMap::new(), loops: vec![], finallys: vec![], handlers: vec![], has_yield: false, ret_ty: None };
    fc.slot("this");
    fc
}

/// Compile top-level statements into a function body (`{main}`-style), returning it with its slot map.
fn build_main(prog: &mut Program, stmts: &[Stmt]) -> R<(Func, HashMap<String, u16>)> {
    let mut fc = new_fc(prog);
    for s in stmts {
        fc.stmt(s)?;
    }
    fc.code.push(Op::Halt);
    let (code, nlocals, handlers) = (std::mem::take(&mut fc.code), fc.locals.len(), std::mem::take(&mut fc.handlers));
    let locals = fc.locals.clone();
    let names = names_by_slot(&locals);
    let func = Func { name: "{main}".into(), nparams: 0, nlocals, code, handlers, capture_slots: vec![], ref_params: vec![], variadic: false, is_generator: false, param_names: vec![], local_names: names };
    Ok((func, locals))
}

pub fn compile(stmts: &[Stmt]) -> R<Program> {
    let mut prog = Program::default();
    let (main, locals) = build_main(&mut prog, stmts)?;
    prog.main = main;
    prog.main_locals = locals;
    Ok(prog)
}

/// Compile code loaded at run time (`eval`, `include`) into an existing program; returns the new function's index.
pub fn compile_snippet(prog: &mut Program, stmts: &[Stmt]) -> R<usize> {
    let (func, _) = build_main(prog, stmts)?;
    prog.funcs.push(Box::new(func));
    Ok(prog.funcs.len() - 1)
}

fn expr_vars(e: &Expr, out: &mut HashSet<String>) {
    let all = |xs: &[Expr], out: &mut HashSet<String>| xs.iter().for_each(|x| expr_vars(x, out));
    match e {
        Expr::Var(n) => {
            out.insert(n.clone());
        }
        Expr::Array(items) => items.iter().for_each(|(k, v)| {
            if let Some(k) = k {
                expr_vars(k, out);
            }
            expr_vars(v, out);
        }),
        Expr::Index(b, k) => {
            expr_vars(b, out);
            if let Some(k) = k {
                expr_vars(k, out);
            }
        }
        Expr::Assign(a, b) | Expr::AssignRef(a, b) | Expr::AssignOp(_, a, b) | Expr::Bin(_, a, b) => {
            expr_vars(a, out);
            expr_vars(b, out);
        }
        Expr::IncDec { target, .. } => expr_vars(target, out),
        Expr::Unary(_, a) | Expr::Cast(_, a) | Expr::Empty(a) | Expr::Print(a) | Expr::Clone(a) | Expr::Throw(a) | Expr::Spread(a) | Expr::YieldFrom(a) | Expr::VarVar(a) | Expr::NamedArg(_, a) => expr_vars(a, out),
        Expr::Ternary(a, b, c) => {
            expr_vars(a, out);
            if let Some(b) = b {
                expr_vars(b, out);
            }
            expr_vars(c, out);
        }
        Expr::Call(_, args) | Expr::Isset(args) => all(args, out),
        Expr::CallExpr(c, args) => {
            expr_vars(c, out);
            all(args, out);
        }
        Expr::New(c, args) | Expr::StaticCall(c, _, args) => {
            if let ClassRef::Dyn(d) = c {
                expr_vars(d, out);
            }
            all(args, out);
        }
        Expr::Prop { obj, name, .. } => {
            expr_vars(obj, out);
            expr_vars(name, out);
        }
        Expr::MethodCall { obj, name, args, .. } => {
            expr_vars(obj, out);
            expr_vars(name, out);
            all(args, out);
        }
        Expr::StaticProp(c, _) | Expr::ClassConst(c, _) => {
            if let ClassRef::Dyn(d) = c {
                expr_vars(d, out);
            }
        }
        Expr::Instanceof(a, c) => {
            expr_vars(a, out);
            if let ClassRef::Dyn(d) = c {
                expr_vars(d, out);
            }
        }
        Expr::Closure(c) => {
            // A nested arrow function sees the enclosing scope through us.
            if c.arrow {
                for s in &c.func.body {
                    if let Stmt::Return(Some(e)) = s {
                        expr_vars(e, out);
                    }
                }
            }
            out.extend(c.uses.iter().map(|(n, _)| n.clone()));
        }
        Expr::Exit(Some(a)) => expr_vars(a, out),
        Expr::Match(s, arms, d) => {
            expr_vars(s, out);
            for (conds, r) in arms {
                all(conds, out);
                expr_vars(r, out);
            }
            if let Some(d) = d {
                expr_vars(d, out);
            }
        }
        _ => {}
    }
}

impl Fc<'_> {
    fn emit(&mut self, op: Op) -> usize {
        self.code.push(op);
        self.code.len() - 1
    }

    fn here(&self) -> u32 {
        self.code.len() as u32
    }

    fn patch(&mut self, at: usize, target: u32) {
        self.code[at] = match self.code[at] {
            Op::Jmp(_) => Op::Jmp(target),
            Op::JmpZ(_) => Op::JmpZ(target),
            Op::JmpNZ(_) => Op::JmpNZ(target),
            Op::JmpZKeep(_) => Op::JmpZKeep(target),
            Op::JmpNZKeep(_) => Op::JmpNZKeep(target),
            Op::JmpNotNullKeep(_) => Op::JmpNotNullKeep(target),
            Op::JmpNullKeep(_) => Op::JmpNullKeep(target),
            Op::IterNext { arr, pos, key, val, .. } => Op::IterNext { arr, pos, key, val, exit: target },
            Op::SkipIfArg(i, _) => Op::SkipIfArg(i, target),
            Op::StaticBind(a, b, _) => Op::StaticBind(a, b, target),
            Op::IterNextRef { arr, pos, key, val, .. } => Op::IterNextRef { arr, pos, key, val, exit: target },
            Op::JmpIfInstance(k, _) => Op::JmpIfInstance(k, target),
            op => op,
        };
    }

    fn konst(&mut self, v: Value) -> u32 {
        self.prog.consts.push(v);
        (self.prog.consts.len() - 1) as u32
    }

    fn push_const(&mut self, v: Value) {
        let k = self.konst(v);
        self.emit(Op::Const(k));
    }

    fn slot(&mut self, name: &str) -> u16 {
        let n = self.locals.len() as u16;
        *self.locals.entry(name.to_string()).or_insert(n)
    }

    fn hidden(&mut self) -> u16 {
        let name = format!("#h{}", self.locals.len());
        self.slot(&name)
    }

    fn block(&mut self, stmts: &[Stmt]) -> R<()> {
        stmts.iter().try_for_each(|s| self.stmt(s))
    }

    fn new_loop(&mut self, is_switch: bool) {
        self.loops.push(LoopCtx { breaks: vec![], continues: vec![], is_switch });
    }

    fn stmt(&mut self, s: &Stmt) -> R<()> {
        match s {
            Stmt::Html(h) => {
                self.push_const(Value::str(h));
                self.emit(Op::Echo);
            }
            Stmt::Echo(args) => {
                for a in args {
                    self.expr(a)?;
                    self.emit(Op::Echo);
                }
            }
            Stmt::Expr(e) => {
                self.expr(e)?;
                self.emit(Op::Pop);
            }
            Stmt::Block(b) => self.block(b)?,
            Stmt::If(arms, els) => {
                let mut ends = vec![];
                for (cond, body) in arms {
                    self.expr(cond)?;
                    let skip = self.emit(Op::JmpZ(0));
                    self.block(body)?;
                    ends.push(self.emit(Op::Jmp(0)));
                    let h = self.here();
                    self.patch(skip, h);
                }
                if let Some(b) = els {
                    self.block(b)?;
                }
                let h = self.here();
                ends.into_iter().for_each(|e| self.patch(e, h));
            }
            Stmt::While(cond, body) => {
                let top = self.here();
                self.expr(cond)?;
                let exit = self.emit(Op::JmpZ(0));
                self.new_loop(false);
                self.block(body)?;
                self.emit(Op::Jmp(top));
                let end = self.here();
                self.patch(exit, end);
                self.finish_loop(top, end);
            }
            Stmt::DoWhile(body, cond) => {
                let top = self.here();
                self.new_loop(false);
                self.block(body)?;
                let test = self.here();
                self.expr(cond)?;
                self.emit(Op::JmpNZ(top));
                let end = self.here();
                self.finish_loop(test, end);
            }
            Stmt::For(init, cond, step, body) => {
                for e in init {
                    self.expr(e)?;
                    self.emit(Op::Pop);
                }
                let top = self.here();
                let mut exits = vec![];
                for (i, c) in cond.iter().enumerate() {
                    self.expr(c)?;
                    if i + 1 < cond.len() {
                        self.emit(Op::Pop);
                    }
                }
                if !cond.is_empty() {
                    exits.push(self.emit(Op::JmpZ(0)));
                }
                self.new_loop(false);
                self.block(body)?;
                let cont = self.here();
                for e in step {
                    self.expr(e)?;
                    self.emit(Op::Pop);
                }
                self.emit(Op::Jmp(top));
                let end = self.here();
                exits.into_iter().for_each(|e| self.patch(e, end));
                self.finish_loop(cont, end);
            }
            Stmt::Foreach { subject, key, val, pattern, by_ref, body } => {
                let arr = self.hidden();
                let pos = self.hidden();
                let ref_subject = match (by_ref, subject) {
                    (true, Expr::Var(n)) => Some(self.slot(n)),
                    _ => None,
                };
                if let Some(src) = ref_subject {
                    self.emit(Op::MakeRef(src));
                    self.emit(Op::BindRef(arr));
                } else {
                    self.expr(subject)?;
                    self.emit(Op::Store(arr));
                    self.emit(Op::Pop);
                }
                self.push_const(Value::Int(0));
                self.emit(Op::Store(pos));
                self.emit(Op::Pop);
                let kslot = key.as_ref().map(|k| self.slot(k)).unwrap_or(NO_SLOT);
                let vslot = self.slot(val);
                let top = if ref_subject.is_some() {
                    self.emit(Op::IterNextRef { arr, pos, key: kslot, val: vslot, exit: 0 })
                } else {
                    self.emit(Op::IterNext { arr, pos, key: kslot, val: vslot, exit: 0 })
                };
                self.new_loop(false);
                if let Some(p) = pattern {
                    let bind = Stmt::Expr(Expr::Assign(Box::new(p.clone()), Box::new(Expr::Var(val.clone()))));
                    self.stmt(&bind)?;
                }
                self.block(body)?;
                self.emit(Op::Jmp(top as u32));
                let end = self.here();
                self.patch(top, end);
                if ref_subject.is_some() {
                    self.emit(Op::RefFlush { arr, pos, val: vslot });
                }
                self.finish_loop(top as u32, end);
            }
            Stmt::Switch(subject, cases) => {
                let tmp = self.hidden();
                self.expr(subject)?;
                self.emit(Op::Store(tmp));
                self.emit(Op::Pop);
                let mut jumps = vec![];
                for (label, _) in cases {
                    if let Some(l) = label {
                        self.emit(Op::Load(tmp));
                        self.expr(l)?;
                        self.emit(Op::Bin("=="));
                        jumps.push(Some(self.emit(Op::JmpNZ(0))));
                    } else {
                        jumps.push(None);
                    }
                }
                let to_default = self.emit(Op::Jmp(0));
                self.new_loop(true);
                let mut default_at = None;
                for (i, (label, body)) in cases.iter().enumerate() {
                    let h = self.here();
                    if let Some(j) = jumps[i] {
                        self.patch(j, h);
                    }
                    if label.is_none() {
                        default_at = Some(h);
                    }
                    self.block(body)?;
                }
                let end = self.here();
                self.patch(to_default, default_at.unwrap_or(end));
                self.finish_loop(end, end);
            }
            Stmt::Break(n) | Stmt::Continue(n) => {
                let is_break = matches!(s, Stmt::Break(_));
                let n = (*n).max(1) as usize;
                if n > self.loops.len() {
                    let m = format!("Cannot '{}' {} level{}", if is_break { "break" } else { "continue" }, n, if n > 1 { "s" } else { "" });
                    let k = self.konst(Value::str(&m));
                    self.emit(Op::Fatal(k));
                } else {
                    let idx = self.loops.len() - n;
                    self.run_finallys(|depth| depth > idx)?;
                    let at = self.emit(Op::Jmp(0));
                    let l = &mut self.loops[idx];
                    if is_break || l.is_switch {
                        l.breaks.push(at);
                    } else {
                        l.continues.push(at);
                    }
                }
            }
            Stmt::Return(e) => {
                match e {
                    Some(e) => self.expr(e)?,
                    None => self.push_const(Value::Null),
                }
                if let (Some(ti), Some(_)) = (self.ret_ty, e) {
                    self.emit(Op::CheckRet(ti));
                }
                if !self.finallys.is_empty() {
                    let tmp = self.hidden();
                    self.emit(Op::Store(tmp));
                    self.emit(Op::Pop);
                    self.run_finallys(|_| true)?;
                    self.emit(Op::Load(tmp));
                }
                self.emit(Op::Ret);
            }
            Stmt::Unset(targets) => {
                for t in targets {
                    match t {
                        Expr::Var(n) => {
                            let s = self.slot(n);
                            self.push_const(Value::Null);
                            self.emit(Op::Store(s));
                            self.emit(Op::Pop);
                        }
                        Expr::Index(..) => {
                            let (base, dims) = flatten(t)?;
                            let (slot, extra) = match base {
                                Base::Var(name) => (self.slot(&name), 0u8),
                                Base::Prop(obj, name) => {
                                    self.expr(obj)?;
                                    self.expr(name)?;
                                    (0, 1)
                                }
                                Base::Static(c, name) => {
                                    self.class_ref(c)?;
                                    self.push_const(Value::str(name));
                                    (0, 2)
                                }
                            };
                            let mut shape = vec![];
                            for d in dims {
                                match d {
                                    Some(k) => {
                                        self.expr(k)?;
                                        shape.push(true);
                                    }
                                    None => return Err("Cannot use [] for unsetting".into()),
                                }
                            }
                            let si = self.shape(shape);
                            self.emit(match extra {
                                0 => Op::UnsetDim(slot, si),
                                1 => Op::UnsetPropDim(si),
                                _ => Op::UnsetStaticDim(si),
                            });
                        }
                        Expr::Prop { .. } => {
                            self.assign(t, |c| {
                                c.push_const(Value::Null);
                                Ok(())
                            })?;
                            self.emit(Op::Pop);
                        }
                        _ => return Err("Cannot unset expression".into()),
                    }
                }
            }
            Stmt::Func(f) => {
                let lname = f.name.to_ascii_lowercase();
                if self.prog.func_index.contains_key(&lname) || builtins::exists(&lname) {
                    return Err(format!("Cannot redeclare {}()", f.name));
                }
                let idx = self.compile_function(f, &[])?;
                self.prog.func_index.insert(lname, idx);
            }
            Stmt::Class(c) => self.class(c)?,
            Stmt::DefConst(name, e) => {
                self.expr(e)?;
                let k = self.konst(Value::str(name));
                self.emit(Op::DefConst(k));
            }
            Stmt::Global(names) => {
                for n in names {
                    let slot = self.slot(n);
                    self.prog.global_names.insert(n.clone());
                    let k = self.konst(Value::str(n));
                    self.emit(Op::GlobalBind(slot, k));
                }
            }
            Stmt::StaticVar(vars) => {
                for (n, init) in vars {
                    let slot = self.slot(n);
                    let key = self.prog.static_count;
                    self.prog.static_count += 1;
                    let skip = self.emit(Op::StaticBind(slot, key, 0));
                    match init {
                        Some(e) => self.expr(e)?,
                        None => self.push_const(Value::Null),
                    }
                    self.emit(Op::StaticInit(slot, key));
                    let h = self.here();
                    self.patch(skip, h);
                }
            }
            Stmt::Try { body, catches, finally } => self.try_stmt(body, catches, finally.as_deref())?,
        }
        Ok(())
    }

    /// Emit inline copies of pending `finally` blocks whose try began at a loop depth matching `pred`.
    fn run_finallys(&mut self, pred: impl Fn(usize) -> bool) -> R<()> {
        let saved = self.finallys.clone();
        for i in (0..saved.len()).rev() {
            if pred(saved[i].1) {
                self.finallys.truncate(i);
                self.block(&saved[i].0)?;
            }
        }
        self.finallys = saved;
        Ok(())
    }

    fn inline_finally(&mut self, finally: Option<&[Stmt]>) -> R<()> {
        if let Some(f) = finally {
            let saved = self.finallys.pop();
            let r = self.block(f);
            self.finallys.extend(saved);
            r?;
        }
        Ok(())
    }

    fn try_stmt(&mut self, body: &[Stmt], catches: &[CatchClause], finally: Option<&[Stmt]>) -> R<()> {
        if let Some(f) = finally {
            self.finallys.push((f.to_vec(), self.loops.len()));
        }
        let start = self.code.len();
        self.block(body)?;
        let end_body = self.code.len();
        let mut segments = vec![(start, end_body)];
        let mut ends = vec![];
        self.inline_finally(finally)?;
        ends.push(self.emit(Op::Jmp(0)));
        let mut catch_handler = None;
        if !catches.is_empty() {
            let target = self.code.len();
            catch_handler = Some(Handler { start, end: end_body, target });
            let mut seg_start = self.code.len();
            for c in catches {
                let mut body_jumps = vec![];
                for cls in &c.classes {
                    let k = self.konst(Value::str(&cls.to_ascii_lowercase()));
                    body_jumps.push(self.emit(Op::JmpIfInstance(k, 0)));
                }
                let next = self.emit(Op::Jmp(0));
                let h = self.here();
                body_jumps.into_iter().for_each(|j| self.patch(j, h));
                match &c.var {
                    Some(v) => {
                        let s = self.slot(v);
                        self.emit(Op::Store(s));
                        self.emit(Op::Pop);
                    }
                    None => {
                        self.emit(Op::Pop);
                    }
                }
                self.block(&c.body)?;
                segments.push((seg_start, self.code.len()));
                self.inline_finally(finally)?;
                ends.push(self.emit(Op::Jmp(0)));
                let h = self.here();
                self.patch(next, h);
                seg_start = self.code.len();
            }
            // No clause matched: rethrow (still protected by the finally handler).
            self.emit(Op::Throw);
            segments.push((seg_start, self.code.len()));
        }
        if let Some(h) = catch_handler {
            self.handlers.push(h);
        }
        if let Some(f) = finally {
            let target = self.code.len();
            for (a, b) in segments {
                self.handlers.push(Handler { start: a, end: b, target });
            }
            self.finallys.pop();
            let tmp = self.hidden();
            self.emit(Op::Store(tmp));
            self.emit(Op::Pop);
            self.block(f)?;
            self.emit(Op::Load(tmp));
            self.emit(Op::Throw);
        }
        let h = self.here();
        ends.into_iter().for_each(|e| self.patch(e, h));
        Ok(())
    }

    fn shape(&mut self, s: Vec<bool>) -> u32 {
        self.prog.shapes.push(s);
        (self.prog.shapes.len() - 1) as u32
    }

    fn finish_loop(&mut self, cont_target: u32, break_target: u32) {
        let l = self.loops.pop().expect("loop ctx");
        l.breaks.into_iter().for_each(|b| self.patch(b, break_target));
        l.continues.into_iter().for_each(|c| self.patch(c, cont_target));
    }

    /// Compile a function body into `prog.funcs`; slot 0 is `$this`, parameters follow, then captures.
    fn compile_function(&mut self, f: &FuncDecl, uses: &[String]) -> R<usize> {
        let mut sub = new_fc(&mut *self.prog);
        for p in &f.params {
            sub.slot(&p.name);
        }
        let capture_slots: Vec<u16> = uses.iter().map(|u| sub.slot(u)).collect();
        for (i, p) in f.params.iter().enumerate() {
            if let (Some(t), false, false) = (&p.ty, p.variadic, p.by_ref) {
                let ti = sub.type_idx(t);
                let slot = sub.slot(&p.name);
                sub.emit(Op::CoerceParam(slot, ti, i as u8));
            }
        }
        sub.ret_ty = f.ret.as_ref().filter(|t| !matches!(t.names[0].as_str(), "void" | "never" | "mixed") || t.names.len() > 1).map(|t| t.clone()).map(|t| sub.type_idx(&t));
        let ret_none_needs_error = f.ret.as_ref().is_some_and(|t| !t.nullable && !t.names.iter().any(|n| matches!(n.as_str(), "void" | "never" | "mixed" | "null")));
        let ret_idx = f.ret.as_ref().map(|t| sub.type_idx(t));
        for (i, p) in f.params.iter().enumerate() {
            if let Some(d) = &p.default {
                let skip = sub.emit(Op::SkipIfArg(i as u8, 0));
                sub.expr(d)?;
                let s = sub.slot(&p.name);
                sub.emit(Op::Store(s));
                sub.emit(Op::Pop);
                let h = sub.here();
                sub.patch(skip, h);
            }
        }
        sub.block(&f.body)?;
        if let (true, Some(ti)) = (ret_none_needs_error, ret_idx) {
            sub.emit(Op::RetNone(ti));
        }
        sub.push_const(Value::Null);
        sub.emit(Op::Ret);
        let (code, nlocals, handlers, is_generator) = (std::mem::take(&mut sub.code), sub.locals.len(), std::mem::take(&mut sub.handlers), sub.has_yield);
        let local_names = names_by_slot(&sub.locals);
        let idx = self.prog.funcs.len();
        self.prog.funcs.push(Box::new(Func {
            name: f.name.clone(),
            nparams: f.params.len(),
            nlocals,
            code,
            handlers,
            capture_slots,
            ref_params: f.params.iter().map(|p| p.by_ref).collect(),
            variadic: f.params.last().is_some_and(|p| p.variadic),
            is_generator,
            param_names: f.params.iter().map(|p| p.name.clone()).collect(),
            local_names,
        }));
        Ok(idx)
    }

    fn type_idx(&mut self, t: &TypeHint) -> u32 {
        if let Some(i) = self.prog.types.iter().position(|x| x == t) {
            return i as u32;
        }
        self.prog.types.push(t.clone());
        (self.prog.types.len() - 1) as u32
    }

    fn thunk(&mut self, e: &Expr) -> R<usize> {
        let f = FuncDecl { ret: None, name: "{thunk}".into(), params: vec![], body: vec![Stmt::Return(Some(e.clone()))] };
        self.compile_function(&f, &[])
    }

    fn class(&mut self, c: &ClassDecl) -> R<()> {
        let mut def = ClassDef {
            name: c.name.clone(),
            kind: c.kind,
            is_abstract: c.is_abstract,
            parent: c.parent.clone(),
            interfaces: c.interfaces.clone(),
            traits: c.traits.clone(),
            consts: vec![],
            props: vec![],
            methods: vec![],
            cases: vec![],
            backed: c.backed,
        };
        for (n, e) in &c.cases {
            let t = match e {
                Some(e) => Some(self.thunk(e)?),
                None => None,
            };
            def.cases.push((n.clone(), t));
        }
        for (n, e) in &c.consts {
            let t = self.thunk(e)?;
            def.consts.push((n.clone(), t));
        }
        for p in &c.props {
            let thunk = match &p.default {
                Some(e) => Some(self.thunk(e)?),
                None => None,
            };
            def.props.push(PropDef { ty: p.ty.clone(), readonly: p.readonly, name: p.name.clone(), thunk, is_static: p.is_static, vis: p.vis });
        }
        for m in &c.methods {
            let mut func = m.func.clone();
            if func.name.eq_ignore_ascii_case("__construct") {
                let mut prefix = vec![];
                for p in func.params.iter().filter(|p| p.promote) {
                    def.props.push(PropDef { ty: p.ty.clone(), readonly: false, name: p.name.clone(), thunk: None, is_static: false, vis: 0 });
                    prefix.push(Stmt::Expr(Expr::Assign(
                        Box::new(Expr::Prop { obj: Box::new(Expr::Var("this".into())), name: Box::new(Expr::Str(p.name.clone())), nullsafe: false }),
                        Box::new(Expr::Var(p.name.clone())),
                    )));
                }
                prefix.extend(std::mem::take(&mut func.body));
                func.body = prefix;
            }
            func.name = format!("{}::{}", c.name, m.func.name);
            let idx = self.compile_function(&func, &[])?;
            def.methods.push(MethodDef { name: m.func.name.clone(), func: idx, is_static: m.is_static, is_abstract: m.is_abstract, vis: m.vis });
        }
        self.prog.classes.push(def);
        Ok(())
    }

    /// Push a class designator; returns true for `self` / `parent` / `static` (forwarding calls).
    fn class_ref(&mut self, c: &ClassRef) -> R<bool> {
        Ok(match c {
            ClassRef::Named(n) => match n.to_ascii_lowercase().as_str() {
                "self" => {
                    self.emit(Op::PushSelf);
                    true
                }
                "parent" => {
                    self.emit(Op::PushParent);
                    true
                }
                "static" => {
                    self.emit(Op::PushStatic);
                    true
                }
                _ => {
                    self.push_const(Value::str(n));
                    false
                }
            },
            ClassRef::Dyn(e) => {
                self.expr(e)?;
                false
            }
        })
    }

    /// Record which call arguments are plain variables so by-reference parameters can bind to them.
    fn site(&mut self, args: &[Expr]) -> u32 {
        if !args.iter().any(|a| matches!(a, Expr::Var(_))) {
            return u32::MAX;
        }
        let slots: Vec<u16> = args
            .iter()
            .map(|a| match a {
                Expr::Var(n) => self.slot(n),
                _ => NO_SLOT,
            })
            .collect();
        self.prog.sites.push(slots);
        (self.prog.sites.len() - 1) as u32
    }

    /// Push the arguments as one array (used when any argument is `...$spread`).
    fn spread_args(&mut self, args: &[Expr]) -> R<()> {
        self.emit(Op::NewArray);
        for a in args {
            match a {
                Expr::Spread(inner) => {
                    self.expr(inner)?;
                    self.emit(Op::ArrSpread);
                }
                Expr::NamedArg(name, value) => {
                    self.push_const(Value::str(name));
                    self.expr(value)?;
                    self.emit(Op::ArrSet);
                }
                other => {
                    self.expr(other)?;
                    self.emit(Op::ArrPush);
                }
            }
        }
        Ok(())
    }

    /// Call a builtin that writes to some of its arguments (`sort`, `preg_match`, ...), then store the new values back.
    fn call_ref_builtin(&mut self, k: u32, args: &[Expr], refs: &[usize]) -> R<()> {
        let n = self.args(args)?;
        self.emit(Op::CallRef(k, n));
        let positions: Vec<usize> = refs.iter().copied().filter(|p| *p < args.len()).collect();
        for p in positions.into_iter().rev() {
            let target = &args[p];
            if matches!(target, Expr::Var(_) | Expr::Index(..) | Expr::Prop { .. } | Expr::StaticProp(..)) {
                let tmp = self.hidden();
                self.emit(Op::Store(tmp));
                self.emit(Op::Pop);
                self.assign(target, |c| {
                    c.emit(Op::Load(tmp));
                    Ok(())
                })?;
            }
            self.emit(Op::Pop);
        }
        Ok(())
    }

    fn args(&mut self, args: &[Expr]) -> R<u8> {
        for a in args {
            self.expr(a)?;
        }
        Ok(args.len() as u8)
    }

    fn expr(&mut self, e: &Expr) -> R<()> {
        match e {
            Expr::Int(i) => self.push_const(Value::Int(*i)),
            Expr::Float(f) => self.push_const(Value::Float(*f)),
            Expr::Str(s) => self.push_const(Value::str(s)),
            Expr::Var(n) => {
                let s = self.slot(n);
                self.emit(Op::Load(s));
            }
            Expr::Array(items) => {
                self.emit(Op::NewArray);
                for (k, v) in items {
                    match (k, v) {
                        (None, Expr::Spread(inner)) => {
                            self.expr(inner)?;
                            self.emit(Op::ArrSpread);
                        }
                        (Some(k), _) => {
                            self.expr(k)?;
                            self.expr(v)?;
                            self.emit(Op::ArrSet);
                        }
                        (None, _) => {
                            self.expr(v)?;
                            self.emit(Op::ArrPush);
                        }
                    }
                }
            }
            Expr::Index(base, Some(k)) => {
                self.expr(base)?;
                self.expr(k)?;
                self.emit(Op::Index);
            }
            Expr::Index(_, None) => return Err("Cannot use [] for reading".into()),
            Expr::Assign(target, rhs) => self.assign(target, |c| c.expr(rhs))?,
            Expr::AssignOp(op, target, rhs) => {
                let combined = Expr::Bin(op, target.clone(), rhs.clone());
                self.assign(target, |c| c.expr(&combined))?;
            }
            Expr::IncDec { target, inc, prefix } => match &**target {
                Expr::Var(n) => {
                    let s = self.slot(n);
                    self.emit(Op::IncDec(s, *inc, *prefix));
                }
                t => {
                    let one = Expr::Int(1);
                    let combined = Expr::Bin(if *inc { "+" } else { "-" }, Box::new(t.clone()), Box::new(one));
                    self.assign(t, |c| c.expr(&combined))?;
                }
            },
            Expr::Bin(op, a, b) => match *op {
                "&&" | "||" => {
                    self.expr(a)?;
                    self.emit(Op::ToBool);
                    let j = self.emit(if *op == "&&" { Op::JmpZKeep(0) } else { Op::JmpNZKeep(0) });
                    self.emit(Op::Pop);
                    self.expr(b)?;
                    self.emit(Op::ToBool);
                    let h = self.here();
                    self.patch(j, h);
                }
                "??" => {
                    self.expr(a)?;
                    let j = self.emit(Op::JmpNotNullKeep(0));
                    self.emit(Op::Pop);
                    self.expr(b)?;
                    let h = self.here();
                    self.patch(j, h);
                }
                _ => {
                    self.expr(a)?;
                    self.expr(b)?;
                    self.emit(Op::Bin(op));
                }
            },
            Expr::Unary(op, x) => {
                self.expr(x)?;
                self.emit(match *op {
                    "!" => Op::Not,
                    "-" => Op::Neg,
                    "+" => Op::Plus,
                    _ => Op::BitNot,
                });
            }
            Expr::Cast(kind, x) => {
                self.expr(x)?;
                self.emit(Op::Cast(kind));
            }
            Expr::Ternary(c, m, r) => {
                self.expr(c)?;
                match m {
                    Some(m) => {
                        let jf = self.emit(Op::JmpZ(0));
                        self.expr(m)?;
                        let je = self.emit(Op::Jmp(0));
                        let h = self.here();
                        self.patch(jf, h);
                        self.expr(r)?;
                        let h = self.here();
                        self.patch(je, h);
                    }
                    None => {
                        let j = self.emit(Op::JmpNZKeep(0));
                        self.emit(Op::Pop);
                        self.expr(r)?;
                        let h = self.here();
                        self.patch(j, h);
                    }
                }
            }
            Expr::Call(name, args) => {
                let lname = name.to_ascii_lowercase();
                let k = self.konst(Value::str(&lname));
                if has_spread(args) {
                    self.spread_args(args)?;
                    self.emit(Op::CallSpread(0, k, false));
                    return Ok(());
                }
                match lname.as_str() {
                    "eval" => {
                        self.args(args)?;
                        self.emit(Op::Eval);
                        return Ok(());
                    }
                    "compact" => {
                        let n = self.args(args)?;
                        self.emit(Op::Compact(n));
                        return Ok(());
                    }
                    "extract" => {
                        self.args(args)?;
                        self.emit(Op::Extract);
                        return Ok(());
                    }
                    "get_defined_vars" => {
                        self.emit(Op::GetDefinedVars);
                        return Ok(());
                    }
                    _ => {}
                }
                let refs = builtins::ref_positions(&lname);
                if !refs.is_empty() {
                    return self.call_ref_builtin(k, args, refs);
                }
                let n = self.args(args)?;
                let site = self.site(args);
                self.emit(Op::Call(k, n, site));
            }
            Expr::CallExpr(callee, args) => {
                self.expr(callee)?;
                if has_spread(args) {
                    self.spread_args(args)?;
                    self.emit(Op::CallSpread(1, 0, false));
                    return Ok(());
                }
                let n = self.args(args)?;
                let site = self.site(args);
                self.emit(Op::CallValue(n, site));
            }
            Expr::Include(kind, inner) => {
                self.expr(inner)?;
                self.emit(Op::Include(*kind));
            }
            Expr::Spread(_) | Expr::NamedArg(..) => return Err("Cannot use spread operator here".into()),
            Expr::VarVar(inner) => {
                self.expr(inner)?;
                self.emit(Op::LoadVar);
            }
            Expr::Hole | Expr::List(_) => return Err("Cannot use list() outside of an assignment".into()),
            Expr::Yield(k, v) => {
                self.has_yield = true;
                if let Some(k) = k {
                    self.expr(k)?;
                }
                match v {
                    Some(v) => self.expr(v)?,
                    None => self.push_const(Value::Null),
                }
                self.emit(Op::Yield(k.is_some()));
            }
            Expr::YieldFrom(inner) => {
                self.has_yield = true;
                let n = self.locals.len();
                let (src, k, v) = (format!("#ys{n}"), format!("#yk{n}"), format!("#yv{n}"));
                self.expr(&Expr::Assign(Box::new(Expr::Var(src.clone())), inner.clone()))?;
                self.emit(Op::Pop);
                let looped = Stmt::Foreach {
                    subject: Expr::Var(src.clone()),
                    key: Some(k.clone()),
                    val: v.clone(),
                    pattern: None,
                    by_ref: false,
                    body: vec![Stmt::Expr(Expr::Yield(Some(Box::new(Expr::Var(k))), Some(Box::new(Expr::Var(v)))))],
                };
                self.stmt(&looped)?;
                let ret = Expr::Ternary(
                    Box::new(Expr::Instanceof(Box::new(Expr::Var(src.clone())), ClassRef::Named("Generator".into()))),
                    Some(Box::new(Expr::MethodCall { obj: Box::new(Expr::Var(src)), name: Box::new(Expr::Str("getReturn".into())), args: vec![], nullsafe: false })),
                    Box::new(Expr::Const("null".into())),
                );
                self.expr(&ret)?;
            }
            Expr::AssignRef(target, src) => match (&**target, &**src) {
                (Expr::Var(a), Expr::Var(b)) => {
                    let (sa, sb) = (self.slot(a), self.slot(b));
                    self.emit(Op::MakeRef(sb));
                    self.emit(Op::BindRef(sa));
                    self.emit(Op::Load(sa));
                }
                _ => self.assign(target, |c| c.expr(src))?,
            },
            Expr::Isset(items) => {
                let mut jumps = vec![];
                for (i, it) in items.iter().enumerate() {
                    self.expr(it)?;
                    self.emit(Op::IsNotNull);
                    if i + 1 < items.len() {
                        jumps.push(self.emit(Op::JmpZKeep(0)));
                        self.emit(Op::Pop);
                    }
                }
                let h = self.here();
                jumps.into_iter().for_each(|j| self.patch(j, h));
            }
            Expr::Empty(x) => {
                self.expr(x)?;
                self.emit(Op::Not);
            }
            Expr::Const(name) => match name.to_ascii_lowercase().as_str() {
                "true" => self.push_const(Value::Bool(true)),
                "false" => self.push_const(Value::Bool(false)),
                "null" => self.push_const(Value::Null),
                _ => match builtins::constant(name) {
                    Some(v) => self.push_const(v),
                    None => {
                        let k = self.konst(Value::str(name));
                        self.emit(Op::ConstGet(k));
                    }
                },
            },
            Expr::Print(x) => {
                self.expr(x)?;
                self.emit(Op::Echo);
                self.push_const(Value::Int(1));
            }
            Expr::Exit(x) => {
                match x {
                    Some(x) => self.expr(x)?,
                    None => self.push_const(Value::Null),
                }
                self.emit(Op::Exit);
                self.push_const(Value::Null);
            }
            Expr::New(c, args) => {
                self.class_ref(c)?;
                if has_spread(args) {
                    self.spread_args(args)?;
                    self.emit(Op::CallSpread(4, 0, false));
                    return Ok(());
                }
                let n = self.args(args)?;
                let site = self.site(args);
                self.emit(Op::New(n, site));
            }
            Expr::Prop { obj, name, nullsafe } => {
                self.expr(obj)?;
                let j = nullsafe.then(|| self.emit(Op::JmpNullKeep(0)));
                self.expr(name)?;
                self.emit(Op::GetProp);
                if let Some(j) = j {
                    let h = self.here();
                    self.patch(j, h);
                }
            }
            Expr::MethodCall { obj, name, args, nullsafe } => {
                self.expr(obj)?;
                let j = nullsafe.then(|| self.emit(Op::JmpNullKeep(0)));
                self.expr(name)?;
                if has_spread(args) {
                    self.spread_args(args)?;
                    self.emit(Op::CallSpread(2, 0, false));
                } else {
                    let n = self.args(args)?;
                    let site = self.site(args);
                    self.emit(Op::CallMethod(n, site));
                }
                if let Some(j) = j {
                    let h = self.here();
                    self.patch(j, h);
                }
            }
            Expr::StaticCall(c, name, args) => {
                let forward = self.class_ref(c)?;
                self.push_const(Value::str(name));
                if has_spread(args) {
                    self.spread_args(args)?;
                    self.emit(Op::CallSpread(3, 0, forward));
                    return Ok(());
                }
                let n = self.args(args)?;
                let site = self.site(args);
                self.emit(Op::CallStatic(n, forward, site));
            }
            Expr::StaticProp(c, name) => {
                self.class_ref(c)?;
                self.push_const(Value::str(name));
                self.emit(Op::GetStatic);
            }
            Expr::ClassConst(c, name) => {
                self.class_ref(c)?;
                self.push_const(Value::str(name));
                self.emit(Op::ClassConst);
            }
            Expr::Instanceof(v, c) => {
                self.expr(v)?;
                self.class_ref(c)?;
                self.emit(Op::InstanceOf);
            }
            Expr::Clone(x) => {
                self.expr(x)?;
                self.emit(Op::Clone);
            }
            Expr::Throw(x) => {
                self.expr(x)?;
                self.emit(Op::Throw);
                self.push_const(Value::Null);
            }
            Expr::Closure(c) => {
                let uses: Vec<(String, bool)> = if c.arrow {
                    let mut vars = HashSet::new();
                    for s in &c.func.body {
                        if let Stmt::Return(Some(e)) = s {
                            expr_vars(e, &mut vars);
                        }
                    }
                    let params: HashSet<&str> = c.func.params.iter().map(|p| p.name.as_str()).collect();
                    let mut v: Vec<String> = vars.into_iter().filter(|n| n != "this" && !params.contains(n.as_str()) && self.locals.contains_key(n)).collect();
                    v.sort();
                    v.into_iter().map(|n| (n, false)).collect()
                } else {
                    c.uses.clone()
                };
                let names: Vec<String> = uses.iter().map(|(n, _)| n.clone()).collect();
                let idx = self.compile_function(&c.func, &names)?;
                for (u, by_ref) in &uses {
                    let s = self.slot(u);
                    self.emit(if *by_ref { Op::MakeRef(s) } else { Op::Load(s) });
                }
                self.emit(Op::MakeClosure(idx as u32, uses.len() as u8));
            }
            Expr::Match(subject, arms, default) => {
                let tmp = self.hidden();
                self.expr(subject)?;
                self.emit(Op::Store(tmp));
                self.emit(Op::Pop);
                let mut arm_jumps: Vec<Vec<usize>> = vec![];
                for (conds, _) in arms {
                    let mut js = vec![];
                    for c in conds {
                        self.emit(Op::Load(tmp));
                        self.expr(c)?;
                        self.emit(Op::Bin("==="));
                        js.push(self.emit(Op::JmpNZ(0)));
                    }
                    arm_jumps.push(js);
                }
                match default {
                    Some(d) => self.expr(d)?,
                    None => {
                        self.emit(Op::Load(tmp));
                        self.emit(Op::MatchError);
                        self.push_const(Value::Null);
                    }
                }
                let mut ends = vec![self.emit(Op::Jmp(0))];
                for ((_, result), js) in arms.iter().zip(arm_jumps) {
                    let h = self.here();
                    js.into_iter().for_each(|j| self.patch(j, h));
                    self.expr(result)?;
                    ends.push(self.emit(Op::Jmp(0)));
                }
                let h = self.here();
                ends.into_iter().for_each(|e| self.patch(e, h));
            }
        }
        Ok(())
    }

    /// Compile an assignment whose value is produced by `rhs`; leaves the assigned value on the stack.
    fn assign(&mut self, target: &Expr, rhs: impl FnOnce(&mut Self) -> R<()>) -> R<()> {
        if let Expr::VarVar(inner) = target {
            self.expr(inner)?;
            rhs(self)?;
            self.emit(Op::StoreVar);
            return Ok(());
        }
        if let Expr::List(items) = target {
            rhs(self)?;
            let name = format!("#l{}", self.locals.len());
            let tmp = self.slot(&name);
            self.emit(Op::Store(tmp));
            let mut idx = 0i64;
            for item in items {
                let Some((k, t)) = item else {
                    idx += 1;
                    continue;
                };
                let key = match k {
                    Some(k) => k.clone(),
                    None => {
                        idx += 1;
                        Expr::Int(idx - 1)
                    }
                };
                let src = Expr::Index(Box::new(Expr::Var(name.clone())), Some(Box::new(key)));
                self.expr(&Expr::Assign(Box::new(t.clone()), Box::new(src)))?;
                self.emit(Op::Pop);
            }
            return Ok(());
        }
        let (base, dims) = flatten(target)?;
        let has_dims = !dims.is_empty();
        let shape_of = |dims: &[Option<&Expr>]| dims.iter().map(|d| d.is_some()).collect::<Vec<bool>>();
        match base {
            Base::Var(n) => {
                let slot = self.slot(&n);
                if !has_dims {
                    rhs(self)?;
                    self.emit(Op::Store(slot));
                    return Ok(());
                }
                self.keys(&dims)?;
                rhs(self)?;
                let si = self.shape(shape_of(&dims));
                self.emit(Op::AssignDim(slot, si));
            }
            Base::Prop(obj, name) => {
                self.expr(obj)?;
                self.expr(name)?;
                if !has_dims {
                    rhs(self)?;
                    self.emit(Op::AssignProp);
                    return Ok(());
                }
                self.keys(&dims)?;
                rhs(self)?;
                let si = self.shape(shape_of(&dims));
                self.emit(Op::AssignPropDim(si));
            }
            Base::Static(c, name) => {
                self.class_ref(c)?;
                self.push_const(Value::str(name));
                if !has_dims {
                    rhs(self)?;
                    self.emit(Op::AssignStatic);
                    return Ok(());
                }
                self.keys(&dims)?;
                rhs(self)?;
                let si = self.shape(shape_of(&dims));
                self.emit(Op::AssignStaticDim(si));
            }
        }
        Ok(())
    }

    fn keys(&mut self, dims: &[Option<&Expr>]) -> R<()> {
        for k in dims.iter().flatten() {
            self.expr(k)?;
        }
        Ok(())
    }
}

fn has_spread(args: &[Expr]) -> bool {
    args.iter().any(|a| matches!(a, Expr::Spread(_) | Expr::NamedArg(..)))
}

fn names_by_slot(locals: &HashMap<String, u16>) -> Vec<String> {
    let mut v = vec![String::new(); locals.len()];
    for (n, s) in locals {
        v[*s as usize] = n.clone();
    }
    v
}

enum Base<'a> {
    Var(String),
    Prop(&'a Expr, &'a Expr),
    Static(&'a ClassRef, &'a str),
}

/// `$a[1][] ...` -> (Var a, [Some(1), None]); `$o->p[1]` -> (Prop o p, [Some(1)])
fn flatten(e: &Expr) -> R<(Base<'_>, Vec<Option<&Expr>>)> {
    let mut dims = vec![];
    let mut cur = e;
    loop {
        match cur {
            Expr::Index(b, k) => {
                dims.push(k.as_deref());
                cur = b;
            }
            Expr::Var(n) => {
                dims.reverse();
                return Ok((Base::Var(n.clone()), dims));
            }
            Expr::Prop { obj, name, .. } => {
                dims.reverse();
                return Ok((Base::Prop(obj, name), dims));
            }
            Expr::StaticProp(c, n) => {
                dims.reverse();
                return Ok((Base::Static(c, n), dims));
            }
            _ => return Err("Cannot write to this expression".into()),
        }
    }
}
