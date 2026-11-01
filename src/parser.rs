//! Recursive-descent / precedence-climbing parser.
use crate::ast::*;
use crate::lexer::{lex, Part, Tok};

type R<T> = Result<T, String>;

pub fn parse(src: &str) -> R<Vec<Stmt>> {
    let mut p = Parser { toks: lex(src)?, pos: 0 };
    let mut out = Vec::new();
    while *p.peek() != Tok::Eof {
        out.push(p.stmt()?);
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

fn boxed(e: Expr) -> Box<Expr> {
    Box::new(e)
}

fn is_lvalue(e: &Expr) -> bool {
    matches!(e, Expr::Var(_) | Expr::Index(..))
}

fn assign_op(o: &str) -> Option<&'static str> {
    Some(match o {
        "+=" => "+",
        "-=" => "-",
        "*=" => "*",
        "/=" => "/",
        ".=" => ".",
        "%=" => "%",
        "**=" => "**",
        "??=" => "??",
        "&=" => "&",
        "|=" => "|",
        "^=" => "^",
        "<<=" => "<<",
        ">>=" => ">>",
        _ => return None,
    })
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)]
    }

    fn next(&mut self) -> Tok {
        let t = self.peek().clone();
        self.pos += 1;
        t
    }

    fn is_op(&self, o: &str) -> bool {
        matches!(self.peek(), Tok::Op(x) if *x == o)
    }

    fn eat_op(&mut self, o: &str) -> bool {
        let hit = self.is_op(o);
        if hit {
            self.pos += 1;
        }
        hit
    }

    fn expect_op(&mut self, o: &str) -> R<()> {
        if self.eat_op(o) {
            Ok(())
        } else {
            Err(format!("syntax error, unexpected {:?}, expecting \"{o}\"", self.peek()))
        }
    }

    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case(k))
    }

    fn eat_kw(&mut self, k: &str) -> bool {
        let hit = self.is_kw(k);
        if hit {
            self.pos += 1;
        }
        hit
    }

    fn end_stmt(&mut self) -> R<()> {
        if self.eat_op(";") || *self.peek() == Tok::Eof {
            Ok(())
        } else {
            Err(format!("syntax error, unexpected {:?}, expecting \";\"", self.peek()))
        }
    }

    fn body(&mut self) -> R<Vec<Stmt>> {
        if self.eat_op("{") {
            let mut v = Vec::new();
            while !self.is_op("}") {
                if *self.peek() == Tok::Eof {
                    return Err("syntax error, unexpected end of file".into());
                }
                v.push(self.stmt()?);
            }
            self.pos += 1;
            Ok(v)
        } else {
            Ok(vec![self.stmt()?])
        }
    }

    fn paren_expr(&mut self) -> R<Expr> {
        self.expect_op("(")?;
        let e = self.expr(0)?;
        self.expect_op(")")?;
        Ok(e)
    }

    fn expr_list(&mut self, end: &str) -> R<Vec<Expr>> {
        let mut v = Vec::new();
        while !self.is_op(end) {
            v.push(self.expr(0)?);
            if !self.eat_op(",") {
                break;
            }
        }
        Ok(v)
    }

    fn var_name(&mut self) -> R<String> {
        match self.next() {
            Tok::Var(n) => Ok(n),
            t => Err(format!("syntax error, unexpected {t:?}, expecting variable")),
        }
    }

    fn stmt(&mut self) -> R<Stmt> {
        let tok = self.peek().clone();
        match tok {
            Tok::Html(h) => {
                self.pos += 1;
                Ok(Stmt::Html(h))
            }
            Tok::Op("{") => Ok(Stmt::Block(self.body()?)),
            Tok::Op(";") => {
                self.pos += 1;
                Ok(Stmt::Block(vec![]))
            }
            Tok::Ident(w) => self.keyword_stmt(&w.to_ascii_lowercase()),
            _ => self.expr_stmt(),
        }
    }

    fn expr_stmt(&mut self) -> R<Stmt> {
        let e = self.expr(0)?;
        self.end_stmt()?;
        Ok(Stmt::Expr(e))
    }

    fn keyword_stmt(&mut self, kw: &str) -> R<Stmt> {
        match kw {
            "echo" => {
                self.pos += 1;
                let mut args = vec![self.expr(0)?];
                while self.eat_op(",") {
                    args.push(self.expr(0)?);
                }
                self.end_stmt()?;
                Ok(Stmt::Echo(args))
            }
            "if" => {
                self.pos += 1;
                let mut arms = vec![(self.paren_expr()?, self.body()?)];
                let mut els = None;
                loop {
                    if self.eat_kw("elseif") {
                        arms.push((self.paren_expr()?, self.body()?));
                    } else if self.is_kw("else") && matches!(self.peek_at(1), Tok::Ident(w) if w.eq_ignore_ascii_case("if")) {
                        self.pos += 2;
                        arms.push((self.paren_expr()?, self.body()?));
                    } else if self.eat_kw("else") {
                        els = Some(self.body()?);
                        break;
                    } else {
                        break;
                    }
                }
                Ok(Stmt::If(arms, els))
            }
            "while" => {
                self.pos += 1;
                let c = self.paren_expr()?;
                Ok(Stmt::While(c, self.body()?))
            }
            "do" => {
                self.pos += 1;
                let b = self.body()?;
                if !self.eat_kw("while") {
                    return Err("syntax error, expecting \"while\"".into());
                }
                let c = self.paren_expr()?;
                self.end_stmt()?;
                Ok(Stmt::DoWhile(b, c))
            }
            "for" => {
                self.pos += 1;
                self.expect_op("(")?;
                let init = self.expr_list(";")?;
                self.expect_op(";")?;
                let cond = self.expr_list(";")?;
                self.expect_op(";")?;
                let step = self.expr_list(")")?;
                self.expect_op(")")?;
                Ok(Stmt::For(init, cond, step, self.body()?))
            }
            "foreach" => {
                self.pos += 1;
                self.expect_op("(")?;
                let subject = self.expr(0)?;
                if !self.eat_kw("as") {
                    return Err("syntax error, expecting \"as\"".into());
                }
                let mut key = None;
                let mut val = self.var_name()?;
                if self.eat_op("=>") {
                    key = Some(val);
                    val = self.var_name()?;
                }
                self.expect_op(")")?;
                Ok(Stmt::Foreach { subject, key, val, body: self.body()? })
            }
            "switch" => {
                self.pos += 1;
                let subject = self.paren_expr()?;
                self.expect_op("{")?;
                let mut cases = Vec::new();
                while !self.eat_op("}") {
                    let label = if self.eat_kw("case") {
                        Some(self.expr(0)?)
                    } else if self.eat_kw("default") {
                        None
                    } else {
                        return Err(format!("syntax error, unexpected {:?} in switch", self.peek()));
                    };
                    if !self.eat_op(":") {
                        self.expect_op(";")?;
                    }
                    let mut body = Vec::new();
                    while !(self.is_kw("case") || self.is_kw("default") || self.is_op("}")) {
                        body.push(self.stmt()?);
                    }
                    cases.push((label, body));
                }
                Ok(Stmt::Switch(subject, cases))
            }
            "break" | "continue" => {
                self.pos += 1;
                let n = if let Tok::Int(n) = self.peek() { let n = *n as u32; self.pos += 1; n } else { 1 };
                self.end_stmt()?;
                Ok(if kw == "break" { Stmt::Break(n) } else { Stmt::Continue(n) })
            }
            "return" => {
                self.pos += 1;
                let e = if self.is_op(";") || *self.peek() == Tok::Eof { None } else { Some(self.expr(0)?) };
                self.end_stmt()?;
                Ok(Stmt::Return(e))
            }
            "unset" if matches!(self.peek_at(1), Tok::Op("(")) => {
                self.pos += 2;
                let v = self.expr_list(")")?;
                self.expect_op(")")?;
                self.end_stmt()?;
                Ok(Stmt::Unset(v))
            }
            "function" if matches!(self.peek_at(1), Tok::Ident(_)) => {
                self.pos += 1;
                let Tok::Ident(name) = self.next() else { unreachable!() };
                self.expect_op("(")?;
                let params = self.params()?;
                if self.eat_op(":") {
                    while !self.is_op("{") && *self.peek() != Tok::Eof {
                        self.pos += 1;
                    }
                }
                let body = self.body()?;
                Ok(Stmt::Func(FuncDecl { name, params, body }))
            }
            _ => self.expr_stmt(),
        }
    }

    fn params(&mut self) -> R<Vec<Param>> {
        let mut params = Vec::new();
        while !self.eat_op(")") {
            while matches!(self.peek(), Tok::Ident(_) | Tok::Op("?") | Tok::Op("|")) {
                self.pos += 1;
            }
            if self.is_op("&") || self.is_op("...") {
                return Err("by-reference and variadic parameters are not supported yet".into());
            }
            let name = self.var_name()?;
            let default = if self.eat_op("=") { Some(self.expr(4)?) } else { None };
            params.push(Param { name, default });
            if !self.eat_op(",") {
                self.expect_op(")")?;
                break;
            }
        }
        Ok(params)
    }

    pub fn expr(&mut self, min: u8) -> R<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let (op, prec, right): (&'static str, u8, bool) = match self.peek() {
                Tok::Op(o) => match *o {
                    "?" => ("?", 5, false),
                    "??" => ("??", 6, true),
                    "||" => ("||", 7, false),
                    "&&" => ("&&", 8, false),
                    "|" => ("|", 9, false),
                    "^" => ("^", 10, false),
                    "&" => ("&", 11, false),
                    "==" | "!=" | "===" | "!==" => (o, 12, false),
                    "<>" => ("!=", 12, false),
                    "<" | "<=" | ">" | ">=" | "<=>" => (o, 13, false),
                    "." => (".", 14, false),
                    "<<" | ">>" => (o, 15, false),
                    "+" | "-" => (o, 16, false),
                    "*" | "/" | "%" => (o, 17, false),
                    "**" => ("**", 19, true),
                    _ => break,
                },
                Tok::Ident(w) => match w.to_ascii_lowercase().as_str() {
                    "or" => ("||", 1, false),
                    "xor" => ("xor", 2, false),
                    "and" => ("&&", 3, false),
                    _ => break,
                },
                _ => break,
            };
            if prec < min {
                break;
            }
            self.pos += 1;
            if op == "?" {
                if self.eat_op(":") {
                    let r = self.expr(6)?;
                    lhs = Expr::Ternary(boxed(lhs), None, boxed(r));
                } else {
                    let m = self.expr(0)?;
                    self.expect_op(":")?;
                    let r = self.expr(6)?;
                    lhs = Expr::Ternary(boxed(lhs), Some(boxed(m)), boxed(r));
                }
                continue;
            }
            let r = self.expr(if right { prec } else { prec + 1 })?;
            lhs = Expr::Bin(op, boxed(lhs), boxed(r));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> R<Expr> {
        let tok = self.peek().clone();
        match tok {
            Tok::Op("!") => {
                self.pos += 1;
                Ok(Expr::Unary("!", boxed(self.expr(18)?)))
            }
            Tok::Op(o @ ("-" | "+" | "~")) => {
                self.pos += 1;
                Ok(Expr::Unary(o, boxed(self.expr(18)?)))
            }
            Tok::Op("@") => {
                self.pos += 1;
                self.expr(18)
            }
            Tok::Op(o @ ("++" | "--")) => {
                self.pos += 1;
                let target = self.postfix_no_assign()?;
                if !is_lvalue(&target) {
                    return Err("syntax error, increment target must be a variable".into());
                }
                Ok(Expr::IncDec { target: boxed(target), inc: o == "++", prefix: true })
            }
            Tok::Op("(") => {
                if let (Tok::Ident(w), Tok::Op(")")) = (self.peek_at(1).clone(), self.peek_at(2).clone()) {
                    let kind = match w.to_ascii_lowercase().as_str() {
                        "int" | "integer" => Some("int"),
                        "float" | "double" | "real" => Some("float"),
                        "string" | "binary" => Some("string"),
                        "bool" | "boolean" => Some("bool"),
                        "array" => Some("array"),
                        _ => None,
                    };
                    if let Some(k) = kind {
                        self.pos += 3;
                        return Ok(Expr::Cast(k, boxed(self.expr(18)?)));
                    }
                }
                self.postfix()
            }
            Tok::Ident(w) => match w.to_ascii_lowercase().as_str() {
                "print" => {
                    self.pos += 1;
                    Ok(Expr::Print(boxed(self.expr(4)?)))
                }
                "exit" | "die" => {
                    self.pos += 1;
                    let mut arg = None;
                    if self.eat_op("(") {
                        if !self.is_op(")") {
                            arg = Some(boxed(self.expr(0)?));
                        }
                        self.expect_op(")")?;
                    }
                    Ok(Expr::Exit(arg))
                }
                "isset" if matches!(self.peek_at(1), Tok::Op("(")) => {
                    self.pos += 2;
                    let v = self.expr_list(")")?;
                    self.expect_op(")")?;
                    Ok(Expr::Isset(v))
                }
                "empty" if matches!(self.peek_at(1), Tok::Op("(")) => {
                    self.pos += 1;
                    Ok(Expr::Empty(boxed(self.paren_expr()?)))
                }
                _ => self.postfix(),
            },
            _ => self.postfix(),
        }
    }

    fn postfix_no_assign(&mut self) -> R<Expr> {
        let mut e = self.primary()?;
        while self.is_op("[") {
            self.pos += 1;
            if self.eat_op("]") {
                e = Expr::Index(boxed(e), None);
            } else {
                let k = self.expr(0)?;
                self.expect_op("]")?;
                e = Expr::Index(boxed(e), Some(boxed(k)));
            }
        }
        Ok(e)
    }

    fn postfix(&mut self) -> R<Expr> {
        let mut e = self.postfix_no_assign()?;
        if let Tok::Op(o @ ("++" | "--")) = self.peek().clone() {
            if is_lvalue(&e) {
                self.pos += 1;
                e = Expr::IncDec { target: boxed(e), inc: o == "++", prefix: false };
            }
        }
        if is_lvalue(&e) {
            if let Tok::Op(o) = self.peek().clone() {
                if o == "=" {
                    self.pos += 1;
                    if self.is_op("&") {
                        return Err("reference assignment is not supported yet".into());
                    }
                    let rhs = self.expr(4)?;
                    return Ok(Expr::Assign(boxed(e), boxed(rhs)));
                }
                if let Some(base) = assign_op(o) {
                    self.pos += 1;
                    let rhs = self.expr(4)?;
                    return Ok(Expr::AssignOp(base, boxed(e), boxed(rhs)));
                }
            }
        }
        Ok(e)
    }

    fn primary(&mut self) -> R<Expr> {
        match self.next() {
            Tok::Var(n) => Ok(Expr::Var(n)),
            Tok::Int(i) => Ok(Expr::Int(i)),
            Tok::Float(f) => Ok(Expr::Float(f)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::Template(parts) => template(&parts),
            Tok::Op("(") => {
                let e = self.expr(0)?;
                self.expect_op(")")?;
                Ok(e)
            }
            Tok::Op("[") => self.array_lit("]"),
            Tok::Ident(w) => {
                if w.eq_ignore_ascii_case("array") && self.is_op("(") {
                    self.pos += 1;
                    return self.array_lit(")");
                }
                if self.eat_op("(") {
                    let args = self.expr_list(")")?;
                    self.expect_op(")")?;
                    return Ok(Expr::Call(w, args));
                }
                Ok(Expr::Const(w))
            }
            t => Err(format!("syntax error, unexpected {t:?}")),
        }
    }

    fn array_lit(&mut self, end: &str) -> R<Expr> {
        let mut items = Vec::new();
        while !self.is_op(end) {
            let first = self.expr(0)?;
            if self.eat_op("=>") {
                items.push((Some(first), self.expr(0)?));
            } else {
                items.push((None, first));
            }
            if !self.eat_op(",") {
                break;
            }
        }
        self.expect_op(end)?;
        Ok(Expr::Array(items))
    }
}

fn template(parts: &[Part]) -> R<Expr> {
    let mut acc = Expr::Str(String::new());
    for p in parts {
        let e = match p {
            Part::Lit(s) => Expr::Str(s.clone()),
            Part::Code(code) => {
                let mut sub = Parser { toks: lex(&format!("<?php {code};"))?, pos: 0 };
                sub.expr(0)?
            }
        };
        acc = Expr::Bin(".", boxed(acc), boxed(e));
    }
    Ok(acc)
}
