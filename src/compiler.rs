//! AST -> stack-machine bytecode.
use crate::ast::*;
use crate::builtins;
use crate::value::Value;
use std::collections::HashMap;

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
    NewArray,
    ArrPush,
    ArrSet,
    Index,
    AssignDim(u16, u32),
    UnsetDim(u16, u32),
    IncDec(u16, bool, bool), // slot, inc, prefix
    IterNext { arr: u16, pos: u16, key: u16, val: u16, exit: u32 },
    Call(u32, u8),
    Ret,
    Exit,
    Fatal(u32),
    SkipIfArg(u8, u32),
    Halt,
}

#[derive(Debug, Default)]
pub struct Func {
    pub name: String,
    pub nparams: usize,
    pub nlocals: usize,
    pub code: Vec<Op>,
}

#[derive(Debug, Default)]
pub struct Program {
    pub consts: Vec<Value>,
    pub shapes: Vec<Vec<bool>>,
    pub funcs: Vec<Func>,
    pub func_index: HashMap<String, usize>,
    pub main: Func,
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
}

pub fn compile(stmts: &[Stmt]) -> R<Program> {
    let mut prog = Program::default();
    let mut fc = Fc { prog: &mut prog, code: vec![], locals: HashMap::new(), loops: vec![] };
    for s in stmts {
        fc.stmt(s)?;
    }
    fc.code.push(Op::Halt);
    let (code, nlocals) = (std::mem::take(&mut fc.code), fc.locals.len());
    prog.main = Func { name: "{main}".into(), nparams: 0, nlocals, code };
    Ok(prog)
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
            Op::IterNext { arr, pos, key, val, .. } => Op::IterNext { arr, pos, key, val, exit: target },
            Op::SkipIfArg(i, _) => Op::SkipIfArg(i, target),
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
                self.loops.push(LoopCtx { breaks: vec![], continues: vec![], is_switch: false });
                self.block(body)?;
                self.emit(Op::Jmp(top));
                let end = self.here();
                self.patch(exit, end);
                self.finish_loop(top, end);
            }
            Stmt::DoWhile(body, cond) => {
                let top = self.here();
                self.loops.push(LoopCtx { breaks: vec![], continues: vec![], is_switch: false });
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
                self.loops.push(LoopCtx { breaks: vec![], continues: vec![], is_switch: false });
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
            Stmt::Foreach { subject, key, val, body } => {
                let arr = self.hidden();
                let pos = self.hidden();
                self.expr(subject)?;
                self.emit(Op::Store(arr));
                self.emit(Op::Pop);
                self.push_const(Value::Int(0));
                self.emit(Op::Store(pos));
                self.emit(Op::Pop);
                let kslot = key.as_ref().map(|k| self.slot(k)).unwrap_or(NO_SLOT);
                let vslot = self.slot(val);
                let top = self.emit(Op::IterNext { arr, pos, key: kslot, val: vslot, exit: 0 });
                self.loops.push(LoopCtx { breaks: vec![], continues: vec![], is_switch: false });
                self.block(body)?;
                self.emit(Op::Jmp(top as u32));
                let end = self.here();
                self.patch(top, end);
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
                self.loops.push(LoopCtx { breaks: vec![], continues: vec![], is_switch: true });
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
                    let at = self.emit(Op::Jmp(0));
                    let idx = self.loops.len() - n;
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
                            let (name, dims) = flatten(t)?;
                            let slot = self.slot(&name);
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
                            self.emit(Op::UnsetDim(slot, si));
                        }
                        _ => return Err("Cannot unset expression".into()),
                    }
                }
            }
            Stmt::Func(f) => self.func(f)?,
        }
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

    fn func(&mut self, f: &FuncDecl) -> R<()> {
        let lname = f.name.to_ascii_lowercase();
        if self.prog.func_index.contains_key(&lname) || builtins::exists(&lname) {
            return Err(format!("Cannot redeclare {}()", f.name));
        }
        let mut sub = Fc { prog: &mut *self.prog, code: vec![], locals: HashMap::new(), loops: vec![] };
        for p in &f.params {
            sub.slot(&p.name);
        }
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
        sub.push_const(Value::Null);
        sub.emit(Op::Ret);
        let (code, nlocals) = (std::mem::take(&mut sub.code), sub.locals.len());
        let idx = self.prog.funcs.len();
        self.prog.funcs.push(Func { name: f.name.clone(), nparams: f.params.len(), nlocals, code });
        self.prog.func_index.insert(lname, idx);
        Ok(())
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
                    match k {
                        Some(k) => {
                            self.expr(k)?;
                            self.expr(v)?;
                            self.emit(Op::ArrSet);
                        }
                        None => {
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
                if *op == "??" {
                    let combined = Expr::Bin("??", target.clone(), rhs.clone());
                    self.assign(target, |c| c.expr(&combined))?;
                } else {
                    let combined = Expr::Bin(op, target.clone(), rhs.clone());
                    self.assign(target, |c| c.expr(&combined))?;
                }
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
                for a in args {
                    self.expr(a)?;
                }
                let k = self.konst(Value::str(&name.to_ascii_lowercase()));
                self.emit(Op::Call(k, args.len() as u8));
            }
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
                        let k = self.konst(Value::str(&format!("Undefined constant \"{name}\"")));
                        self.emit(Op::Fatal(k));
                        self.push_const(Value::Null);
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
        }
        Ok(())
    }

    /// Compile an assignment whose value is produced by `rhs`; leaves the assigned value on the stack.
    fn assign(&mut self, target: &Expr, rhs: impl FnOnce(&mut Self) -> R<()>) -> R<()> {
        match target {
            Expr::Var(n) => {
                rhs(self)?;
                let s = self.slot(n);
                self.emit(Op::Store(s));
            }
            Expr::Index(..) => {
                let (name, dims) = flatten(target)?;
                let slot = self.slot(&name);
                let mut shape = vec![];
                for d in dims {
                    match d {
                        Some(k) => {
                            self.expr(k)?;
                            shape.push(true);
                        }
                        None => shape.push(false),
                    }
                }
                rhs(self)?;
                let si = self.shape(shape);
                self.emit(Op::AssignDim(slot, si));
            }
            _ => return Err("Cannot assign to this expression".into()),
        }
        Ok(())
    }
}

/// `$a[1][] ...` -> ("a", [Some(1), None])
fn flatten(e: &Expr) -> R<(String, Vec<Option<&Expr>>)> {
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
                return Ok((n.clone(), dims));
            }
            _ => return Err("Cannot write to this array expression".into()),
        }
    }
}
