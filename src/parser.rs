//! Recursive-descent / precedence-climbing parser.
use crate::ast::*;
use crate::lexer::{lex, Part, Tok};

type R<T> = Result<T, String>;

pub fn parse(src: &str) -> R<Vec<Stmt>> {
    let mut p = Parser { toks: lex(src)?, pos: 0, anon: vec![] };
    let mut out = Vec::new();
    while *p.peek() != Tok::Eof {
        out.push(p.stmt()?);
    }
    let mut all: Vec<Stmt> = p.anon.drain(..).map(Stmt::Class).collect();
    all.extend(out);
    Ok(all)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    /// Anonymous classes found while parsing; hoisted ahead of the program.
    anon: Vec<ClassDecl>,
}

fn boxed(e: Expr) -> Box<Expr> {
    Box::new(e)
}

fn is_lvalue(e: &Expr) -> bool {
    matches!(e, Expr::Var(_) | Expr::Index(..) | Expr::Prop { .. } | Expr::StaticProp(..) | Expr::VarVar(_))
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
            if self.eat_op("...") {
                v.push(Expr::Spread(boxed(self.expr(0)?)));
            } else if matches!((self.peek(), self.peek_at(1)), (Tok::Ident(_), Tok::Op(":"))) {
                let Tok::Ident(n) = self.next() else { unreachable!() };
                self.pos += 1;
                v.push(Expr::NamedArg(n, boxed(self.expr(0)?)));
            } else {
                v.push(self.expr(0)?);
            }
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
                let mut by_ref = self.eat_op("&");
                let mut pattern = None;
                let mut val = self.foreach_target(&mut pattern)?;
                if self.eat_op("=>") {
                    key = Some(val);
                    by_ref = self.eat_op("&");
                    pattern = None;
                    val = self.foreach_target(&mut pattern)?;
                }
                self.expect_op(")")?;
                Ok(Stmt::Foreach { subject, key, val, pattern, by_ref, body: self.body()? })
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
            "function" if matches!(self.peek_at(1), Tok::Ident(_)) || (self.peek_at(1) == &Tok::Op("&") && matches!(self.peek_at(2), Tok::Ident(_))) => {
                self.pos += 1;
                self.eat_op("&");
                let Tok::Ident(name) = self.next() else { unreachable!() };
                self.expect_op("(")?;
                let params = self.params()?;
                let ret = self.parse_return_type();
                let body = self.body()?;
                Ok(Stmt::Func(FuncDecl { ret, name, params, body }))
            }
            "abstract" | "final" | "readonly" | "class" | "interface" | "trait" | "enum"
                if (kw != "class" && kw != "interface" && kw != "trait" && kw != "enum") || matches!(self.peek_at(1), Tok::Ident(_)) =>
            {
                self.class_decl()
            }
            "try" => {
                self.pos += 1;
                let body = self.body()?;
                let mut catches = Vec::new();
                let mut finally = None;
                loop {
                    if self.eat_kw("catch") {
                        self.expect_op("(")?;
                        let mut classes = Vec::new();
                        loop {
                            match self.next() {
                                Tok::Ident(n) => classes.push(n),
                                t => return Err(format!("syntax error, unexpected {t:?} in catch")),
                            }
                            if !self.eat_op("|") {
                                break;
                            }
                        }
                        let var = if let Tok::Var(_) = self.peek() { Some(self.var_name()?) } else { None };
                        self.expect_op(")")?;
                        catches.push(CatchClause { classes, var, body: self.body()? });
                    } else if self.eat_kw("finally") {
                        finally = Some(self.body()?);
                    } else {
                        break;
                    }
                }
                Ok(Stmt::Try { body, catches, finally })
            }
            "global" if matches!(self.peek_at(1), Tok::Var(_)) => {
                self.pos += 1;
                let mut names = vec![self.var_name()?];
                while self.eat_op(",") {
                    names.push(self.var_name()?);
                }
                self.end_stmt()?;
                Ok(Stmt::Global(names))
            }
            "static" if matches!(self.peek_at(1), Tok::Var(_)) => {
                self.pos += 1;
                let mut vars = Vec::new();
                loop {
                    let name = self.var_name()?;
                    let init = if self.eat_op("=") { Some(self.expr(4)?) } else { None };
                    vars.push((name, init));
                    if !self.eat_op(",") {
                        break;
                    }
                }
                self.end_stmt()?;
                Ok(Stmt::StaticVar(vars))
            }
            "namespace" if !matches!(self.peek_at(1), Tok::Op("\\")) => {
                self.pos += 1;
                while matches!(self.peek(), Tok::Ident(_)) {
                    self.pos += 1;
                }
                if self.is_op("{") {
                    Ok(Stmt::Block(self.body()?))
                } else {
                    self.end_stmt()?;
                    Ok(Stmt::Block(vec![]))
                }
            }
            "use" | "declare" | "goto" => {
                // Imports and declare() have no effect on flattened namespaces.
                let mut depth = 0;
                while *self.peek() != Tok::Eof {
                    match self.next() {
                        Tok::Op("{") => depth += 1,
                        Tok::Op("}") => depth -= 1,
                        Tok::Op(";") if depth == 0 => break,
                        _ => {}
                    }
                }
                Ok(Stmt::Block(vec![]))
            }
            "const" => {
                self.pos += 1;
                let mut stmts = Vec::new();
                loop {
                    let Tok::Ident(name) = self.next() else { return Err("syntax error in const".into()) };
                    self.expect_op("=")?;
                    stmts.push(Stmt::DefConst(name, self.expr(0)?));
                    if !self.eat_op(",") {
                        break;
                    }
                }
                self.end_stmt()?;
                Ok(Stmt::Block(stmts))
            }
            _ => self.expr_stmt(),
        }
    }

    /// Loop variable of a foreach: a name, or a destructuring pattern (returned via a hidden name).
    fn foreach_target(&mut self, pattern: &mut Option<Expr>) -> R<String> {
        if self.is_op("[") || (self.is_kw("list") && matches!(self.peek_at(1), Tok::Op("("))) {
            let e = if self.eat_op("[") {
                self.array_lit("]")?
            } else {
                self.pos += 2;
                self.array_lit(")")?
            };
            let Expr::Array(items) = e else { unreachable!() };
            *pattern = Some(self.finish_pattern(items)?);
            return Ok(format!("#fe{}", self.pos));
        }
        self.var_name()
    }

    /// Parse `T`, `?T`, `A|B`, `A&B` (stops before a by-reference `&$x`).
    fn parse_type(&mut self) -> Option<TypeHint> {
        let mut names = Vec::new();
        let mut nullable = false;
        let mut display = String::new();
        loop {
            match self.peek().clone() {
                Tok::Op("?") => {
                    nullable = true;
                    display.push('?');
                }
                Tok::Op("|") => display.push('|'),
                Tok::Op("&") if !matches!(self.peek_at(1), Tok::Var(_) | Tok::Op("...")) => display.push('&'),
                Tok::Op("(") | Tok::Op(")") if !names.is_empty() || matches!(self.peek(), Tok::Op("(")) => {
                    if matches!(self.peek(), Tok::Op(")")) && !self.type_continues() {
                        break;
                    }
                }
                Tok::Ident(n) => {
                    let l = n.to_ascii_lowercase();
                    if l == "null" {
                        nullable = true;
                    }
                    display.push_str(&n);
                    names.push(l);
                }
                _ => break,
            }
            self.pos += 1;
        }
        if names.is_empty() { None } else { Some(TypeHint { names, nullable, display }) }
    }

    fn type_continues(&self) -> bool {
        matches!(self.peek_at(1), Tok::Op("|") | Tok::Op("&"))
    }

    fn parse_return_type(&mut self) -> Option<TypeHint> {
        if self.eat_op(":") { self.parse_type() } else { None }
    }

    fn ident(&mut self) -> R<String> {
        match self.next() {
            Tok::Ident(n) => Ok(n),
            t => Err(format!("syntax error, unexpected {t:?}, expecting identifier")),
        }
    }

    fn name_list(&mut self) -> R<Vec<String>> {
        let mut v = vec![self.ident()?];
        while self.eat_op(",") {
            v.push(self.ident()?);
        }
        Ok(v)
    }

    fn class_decl(&mut self) -> R<Stmt> {
        let mut is_abstract = false;
        loop {
            if self.eat_kw("abstract") {
                is_abstract = true;
            } else if !(self.eat_kw("final") || self.eat_kw("readonly")) {
                break;
            }
        }
        let kind = if self.eat_kw("class") {
            ClassKind::Class
        } else if self.eat_kw("enum") {
            ClassKind::Enum
        } else if self.eat_kw("interface") {
            ClassKind::Interface
        } else if self.eat_kw("trait") {
            ClassKind::Trait
        } else {
            return Err(format!("syntax error, unexpected {:?}, expecting class", self.peek()));
        };
        let name = self.ident()?;
        let decl = self.class_rest(name, kind, is_abstract)?;
        Ok(Stmt::Class(decl))
    }

    fn class_rest(&mut self, name: String, kind: ClassKind, is_abstract: bool) -> R<ClassDecl> {
        let (mut parent, mut interfaces) = (None, vec![]);
        let mut backed = false;
        if kind == ClassKind::Enum && self.eat_op(":") {
            backed = true;
            while matches!(self.peek(), Tok::Ident(_)) {
                self.pos += 1;
            }
        }
        if self.eat_kw("extends") {
            if kind == ClassKind::Interface {
                interfaces = self.name_list()?;
            } else {
                parent = Some(self.ident()?);
            }
        }
        if self.eat_kw("implements") {
            interfaces = self.name_list()?;
        }
        self.expect_op("{")?;
        let mut decl = ClassDecl { name, kind, is_abstract, parent, interfaces, traits: vec![], consts: vec![], props: vec![], methods: vec![], cases: vec![], backed };
        while !self.eat_op("}") {
            if self.eat_kw("use") {
                decl.traits.extend(self.name_list()?);
                if self.is_op("{") {
                    let mut depth = 0;
                    loop {
                        match self.next() {
                            Tok::Op("{") => depth += 1,
                            Tok::Op("}") => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Tok::Eof => return Err("syntax error, unexpected end of file".into()),
                            _ => {}
                        }
                    }
                } else {
                    self.end_stmt()?;
                }
                continue;
            }
            if kind == ClassKind::Enum && self.eat_kw("case") {
                let n = self.ident()?;
                let v = if self.eat_op("=") { Some(self.expr(0)?) } else { None };
                self.end_stmt()?;
                decl.cases.push((n, v));
                continue;
            }
            let (mut vis, mut is_static, mut m_abstract, mut readonly) = (0u8, false, false, false);
            loop {
                let Tok::Ident(w) = self.peek().clone() else { break };
                match w.to_ascii_lowercase().as_str() {
                    "public" | "var" => vis = 0,
                    "protected" => vis = 1,
                    "private" => vis = 2,
                    "static" => is_static = true,
                    "abstract" => m_abstract = true,
                    "readonly" => readonly = true,
                    "final" => {}
                    _ => break,
                }
                self.pos += 1;
            }
            if self.eat_kw("const") {
                // Optional type before the name: `const int X = 1;`
                loop {
                    if matches!(self.peek_at(1), Tok::Op("=")) {
                        let name = self.ident()?;
                        self.pos += 1;
                        decl.consts.push((name, self.expr(0)?));
                    } else {
                        self.pos += 1;
                        continue;
                    }
                    if !self.eat_op(",") {
                        break;
                    }
                }
                self.end_stmt()?;
            } else if self.eat_kw("function") {
                self.eat_op("&");
                let name = self.ident()?;
                self.expect_op("(")?;
                let params = self.params()?;
                let ret = self.parse_return_type();
                let (body, abstract_now) = if self.is_op("{") { (self.body()?, false) } else { self.end_stmt()?; (vec![], true) };
                decl.methods.push(MethodDecl {
                    func: FuncDecl { ret, name, params, body },
                    is_static,
                    is_abstract: m_abstract || abstract_now,
                    vis,
                });
            } else {
                let ty = self.parse_type();
                loop {
                    let name = self.var_name()?;
                    let default = if self.eat_op("=") { Some(self.expr(0)?) } else { None };
                    decl.props.push(PropDecl { ty: ty.clone(), readonly, name, default, is_static, vis });
                    if !self.eat_op(",") {
                        break;
                    }
                }
                self.end_stmt()?;
            }
        }
        Ok(decl)
    }

    fn params(&mut self) -> R<Vec<Param>> {
        let mut params = Vec::new();
        while !self.eat_op(")") {
            let mut promote = false;
            while let Tok::Ident(w) = self.peek() {
                if matches!(w.to_ascii_lowercase().as_str(), "public" | "protected" | "private" | "readonly") {
                    promote = true;
                    self.pos += 1;
                } else {
                    break;
                }
            }
            let ty = self.parse_type();
            let by_ref = self.eat_op("&");
            let variadic = self.eat_op("...");
            let name = self.var_name()?;
            let default = if self.eat_op("=") { Some(self.expr(4)?) } else { None };
            params.push(Param { ty, name, default, promote, by_ref, variadic });
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
                    "instanceof" => ("instanceof", 20, false),
                    _ => break,
                },
                _ => break,
            };
            if prec < min {
                break;
            }
            self.pos += 1;
            if op == "instanceof" {
                let cref = match self.peek().clone() {
                    Tok::Ident(n) => {
                        self.pos += 1;
                        ClassRef::Named(n)
                    }
                    _ => ClassRef::Dyn(boxed(self.postfix_no_assign()?)),
                };
                lhs = Expr::Instanceof(boxed(lhs), cref);
                continue;
            }
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
                        "object" => Some("object"),
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
                "clone" => {
                    self.pos += 1;
                    Ok(Expr::Clone(boxed(self.postfix()?)))
                }
                "yield" => {
                    self.pos += 1;
                    if self.eat_kw("from") {
                        return Ok(Expr::YieldFrom(boxed(self.expr(4)?)));
                    }
                    if matches!(self.peek(), Tok::Op(";" | ")" | "," | "]")) || *self.peek() == Tok::Eof {
                        return Ok(Expr::Yield(None, None));
                    }
                    let first = self.expr(5)?;
                    if self.eat_op("=>") {
                        let v = self.expr(5)?;
                        return Ok(Expr::Yield(Some(boxed(first)), Some(boxed(v))));
                    }
                    Ok(Expr::Yield(None, Some(boxed(first))))
                }
                "list" if matches!(self.peek_at(1), Tok::Op("(")) => {
                    self.pos += 2;
                    let items = self.array_lit(")")?;
                    let Expr::Array(items) = items else { unreachable!() };
                    let pattern = self.finish_pattern(items)?;
                    if self.eat_op("=") {
                        let rhs = self.expr(4)?;
                        return Ok(Expr::Assign(boxed(pattern), boxed(rhs)));
                    }
                    Ok(pattern)
                }
                "throw" => {
                    self.pos += 1;
                    Ok(Expr::Throw(boxed(self.expr(0)?)))
                }
                "new" => {
                    self.pos += 1;
                    let cref = match self.peek().clone() {
                        Tok::Ident(n) if n.eq_ignore_ascii_case("class") => {
                            self.pos += 1;
                            let args = if self.eat_op("(") {
                                let a = self.expr_list(")")?;
                                self.expect_op(")")?;
                                a
                            } else {
                                vec![]
                            };
                            let name = format!("class@anonymous\0{}", self.anon.len());
                            let decl = self.class_rest(name.clone(), ClassKind::Class, false)?;
                            self.anon.push(decl);
                            return self.postfix_ops(Expr::New(ClassRef::Named(name), args));
                        }
                        Tok::Ident(n) => {
                            self.pos += 1;
                            ClassRef::Named(n)
                        }
                        Tok::Op("(") => {
                            self.pos += 1;
                            let e = self.expr(0)?;
                            self.expect_op(")")?;
                            ClassRef::Dyn(boxed(e))
                        }
                        _ => ClassRef::Dyn(boxed(self.new_target()?)),
                    };
                    let args = if self.eat_op("(") {
                        let a = self.expr_list(")")?;
                        self.expect_op(")")?;
                        a
                    } else {
                        vec![]
                    };
                    self.postfix_ops(Expr::New(cref, args))
                }
                "match" if matches!(self.peek_at(1), Tok::Op("(")) => self.match_expr(),
                "function" | "fn" => self.closure(),
                "static" if matches!(self.peek_at(1), Tok::Ident(w) if w.eq_ignore_ascii_case("function") || w.eq_ignore_ascii_case("fn")) => {
                    self.pos += 1;
                    self.closure()
                }
                _ => self.postfix(),
            },
            _ => self.postfix(),
        }
    }

    fn postfix_no_assign(&mut self) -> R<Expr> {
        let e = self.primary()?;
        self.postfix_ops(e)
    }

    /// Target of `new $x->y`: a variable with property/index accesses, but no calls.
    fn new_target(&mut self) -> R<Expr> {
        let mut e = match self.next() {
            Tok::Var(n) => Expr::Var(n),
            t => return Err(format!("syntax error, unexpected {t:?} after new")),
        };
        loop {
            if self.eat_op("->") {
                let name = self.ident()?;
                e = Expr::Prop { obj: boxed(e), name: boxed(Expr::Str(name)), nullsafe: false };
            } else if self.eat_op("::") {
                let name = self.var_name()?;
                e = Expr::StaticProp(ClassRef::Dyn(boxed(e)), name);
            } else if self.is_op("[") {
                self.pos += 1;
                let k = self.expr(0)?;
                self.expect_op("]")?;
                e = Expr::Index(boxed(e), Some(boxed(k)));
            } else {
                return Ok(e);
            }
        }
    }

    fn member_name(&mut self) -> R<Expr> {
        match self.next() {
            Tok::Ident(n) => Ok(Expr::Str(n)),
            Tok::Var(v) => Ok(Expr::Var(v)),
            Tok::Op("{") => {
                let e = self.expr(0)?;
                self.expect_op("}")?;
                Ok(e)
            }
            t => Err(format!("syntax error, unexpected {t:?} after ->")),
        }
    }

    fn static_member(&mut self, cref: ClassRef) -> R<Expr> {
        match self.next() {
            Tok::Var(n) => Ok(Expr::StaticProp(cref, n)),
            Tok::Ident(n) => {
                if self.eat_op("(") {
                    let args = self.expr_list(")")?;
                    self.expect_op(")")?;
                    Ok(Expr::StaticCall(cref, n, args))
                } else {
                    Ok(Expr::ClassConst(cref, n))
                }
            }
            t => Err(format!("syntax error, unexpected {t:?} after ::")),
        }
    }

    fn postfix_ops(&mut self, mut e: Expr) -> R<Expr> {
        loop {
            match self.peek().clone() {
                Tok::Op("[") => {
                    self.pos += 1;
                    if self.eat_op("]") {
                        e = Expr::Index(boxed(e), None);
                    } else {
                        let k = self.expr(0)?;
                        self.expect_op("]")?;
                        e = Expr::Index(boxed(e), Some(boxed(k)));
                    }
                }
                Tok::Op(o @ ("->" | "?->")) => {
                    self.pos += 1;
                    let name = self.member_name()?;
                    let nullsafe = o == "?->";
                    if self.eat_op("(") {
                        let args = self.expr_list(")")?;
                        self.expect_op(")")?;
                        e = Expr::MethodCall { obj: boxed(e), name: boxed(name), args, nullsafe };
                    } else {
                        e = Expr::Prop { obj: boxed(e), name: boxed(name), nullsafe };
                    }
                }
                Tok::Op("::") => {
                    self.pos += 1;
                    e = self.static_member(ClassRef::Dyn(boxed(e)))?;
                }
                Tok::Op("(") if !matches!(e, Expr::Int(_) | Expr::Float(_) | Expr::Str(_)) => {
                    self.pos += 1;
                    let args = self.expr_list(")")?;
                    self.expect_op(")")?;
                    e = Expr::CallExpr(boxed(e), args);
                }
                _ => return Ok(e),
            }
        }
    }

    /// Turn the items of an array literal into a destructuring pattern.
    fn finish_pattern(&mut self, items: Vec<(Option<Expr>, Expr)>) -> R<Expr> {
        let mut out = Vec::new();
        for (k, v) in items {
            out.push(match v {
                Expr::Hole => None,
                Expr::Array(inner) => Some((k, self.finish_pattern(inner)?)),
                other => Some((k, other)),
            });
        }
        Ok(Expr::List(out))
    }

    fn closure(&mut self) -> R<Expr> {
        let arrow = self.is_kw("fn");
        self.pos += 1;
        self.eat_op("&");
        self.expect_op("(")?;
        let params = self.params()?;
        let mut uses = Vec::new();
        if !arrow && self.eat_kw("use") {
            self.expect_op("(")?;
            while !self.eat_op(")") {
                let by_ref = self.eat_op("&");
                uses.push((self.var_name()?, by_ref));
                self.eat_op(",");
            }
        }
        let ret = self.parse_return_type();
        let body = if arrow {
            self.expect_op("=>")?;
            vec![Stmt::Return(Some(self.expr(4)?))]
        } else {
            self.body()?
        };
        Ok(Expr::Closure(Box::new(ClosureDecl { func: FuncDecl { ret, name: "{closure}".into(), params, body }, uses, arrow })))
    }

    fn match_expr(&mut self) -> R<Expr> {
        self.pos += 1;
        let subject = self.paren_expr()?;
        self.expect_op("{")?;
        let (mut arms, mut default) = (Vec::new(), None);
        while !self.eat_op("}") {
            if self.is_kw("default") && matches!(self.peek_at(1), Tok::Op("=>") | Tok::Op(",")) {
                self.pos += 1;
                self.eat_op(",");
                self.expect_op("=>")?;
                default = Some(boxed(self.expr(0)?));
            } else {
                let mut conds = vec![self.expr(0)?];
                while self.eat_op(",") {
                    if self.is_op("=>") {
                        break;
                    }
                    conds.push(self.expr(0)?);
                }
                self.expect_op("=>")?;
                arms.push((conds, self.expr(0)?));
            }
            if !self.eat_op(",") {
                self.expect_op("}")?;
                break;
            }
        }
        Ok(Expr::Match(boxed(subject), arms, default))
    }

    fn postfix(&mut self) -> R<Expr> {
        let mut e = self.postfix_no_assign()?;
        if let Tok::Op(o @ ("++" | "--")) = self.peek().clone() {
            if is_lvalue(&e) {
                self.pos += 1;
                e = Expr::IncDec { target: boxed(e), inc: o == "++", prefix: false };
            }
        }
        if matches!(e, Expr::Array(_) | Expr::List(_)) && self.is_op("=") {
            self.pos += 1;
            let rhs = self.expr(4)?;
            let pattern = match e {
                Expr::Array(items) => self.finish_pattern(items)?,
                other => other,
            };
            return Ok(Expr::Assign(boxed(pattern), boxed(rhs)));
        }
        if is_lvalue(&e) {
            if let Tok::Op(o) = self.peek().clone() {
                if o == "=" {
                    self.pos += 1;
                    if self.eat_op("&") {
                        let rhs = self.postfix_no_assign()?;
                        return Ok(Expr::AssignRef(boxed(e), boxed(rhs)));
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
            Tok::Op("$") => {
                let inner = match self.peek().clone() {
                    Tok::Var(n) => {
                        self.pos += 1;
                        Expr::Var(n)
                    }
                    Tok::Op("$") => self.primary()?,
                    Tok::Op("{") => {
                        self.pos += 1;
                        let e = self.expr(0)?;
                        self.expect_op("}")?;
                        e
                    }
                    t => return Err(format!("syntax error, unexpected {t:?} after $")),
                };
                Ok(Expr::VarVar(boxed(inner)))
            }
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
                if self.eat_op("::") {
                    return self.static_member(ClassRef::Named(w));
                }
                Ok(Expr::Const(w))
            }
            t => Err(format!("syntax error, unexpected {t:?}")),
        }
    }

    fn array_lit(&mut self, end: &str) -> R<Expr> {
        let mut items = Vec::new();
        while !self.is_op(end) {
            if self.is_op(",") {
                self.pos += 1;
                items.push((None, Expr::Hole));
                continue;
            }
            if self.eat_op("...") {
                items.push((None, Expr::Spread(boxed(self.expr(0)?))));
                if !self.eat_op(",") {
                    break;
                }
                continue;
            }
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
                let mut sub = Parser { toks: lex(&format!("<?php {code};"))?, pos: 0, anon: vec![] };
                sub.expr(0)?
            }
        };
        acc = Expr::Bin(".", boxed(acc), boxed(e));
    }
    Ok(acc)
}
