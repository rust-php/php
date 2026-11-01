//! Precedence-climbing parser producing a small AST.
use crate::lexer::Token;

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    Neg(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Html(String),
    Echo(Vec<Expr>),
}

pub fn parse(tokens: &[Token]) -> Result<Vec<Stmt>, String> {
    let mut p = Parser { t: tokens, i: 0 };
    let mut out = Vec::new();
    while p.i < p.t.len() {
        match &p.t[p.i] {
            Token::Html(h) => {
                out.push(Stmt::Html(h.clone()));
                p.i += 1;
            }
            Token::Echo => {
                p.i += 1;
                let mut args = vec![p.expr(0)?];
                while matches!(p.t.get(p.i), Some(Token::Op(','))) {
                    p.i += 1;
                    args.push(p.expr(0)?);
                }
                p.expect_semi()?;
                out.push(Stmt::Echo(args));
            }
            Token::Semi => p.i += 1,
            t => return Err(format!("unexpected token {t:?}")),
        }
    }
    Ok(out)
}

struct Parser<'a> {
    t: &'a [Token],
    i: usize,
}

fn prec(op: char) -> Option<u8> {
    match op {
        '.' => Some(1),
        '+' | '-' => Some(2),
        '*' | '/' => Some(3),
        _ => None,
    }
}

impl Parser<'_> {
    fn expect_semi(&mut self) -> Result<(), String> {
        match self.t.get(self.i) {
            Some(Token::Semi) => {
                self.i += 1;
                Ok(())
            }
            // `?>` closes a statement implicitly; end of input does too.
            None | Some(Token::Html(_)) => Ok(()),
            Some(t) => Err(format!("expected ';', got {t:?}")),
        }
    }

    fn expr(&mut self, min: u8) -> Result<Expr, String> {
        let mut lhs = self.atom()?;
        while let Some(Token::Op(op)) = self.t.get(self.i) {
            let Some(p) = prec(*op) else { break };
            if p < min {
                break;
            }
            let op = *op;
            self.i += 1;
            let rhs = self.expr(p + 1)?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn atom(&mut self) -> Result<Expr, String> {
        let tok = self.t.get(self.i).cloned().ok_or("unexpected end of input")?;
        self.i += 1;
        match tok {
            Token::Int(n) => Ok(Expr::Int(n)),
            Token::Float(f) => Ok(Expr::Float(f)),
            Token::Str(s) => Ok(Expr::Str(s)),
            Token::Op('-') => Ok(Expr::Neg(Box::new(self.atom()?))),
            Token::Op('+') => self.atom(),
            Token::LParen => {
                let e = self.expr(0)?;
                match self.t.get(self.i) {
                    Some(Token::RParen) => {
                        self.i += 1;
                        Ok(e)
                    }
                    _ => Err("expected ')'".into()),
                }
            }
            t => Err(format!("unexpected token {t:?}")),
        }
    }
}
