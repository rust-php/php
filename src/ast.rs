//! Abstract syntax tree.

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    Var(String),
    Array(Vec<(Option<Expr>, Expr)>),
    Index(Box<Expr>, Option<Box<Expr>>),
    Assign(Box<Expr>, Box<Expr>),
    AssignOp(&'static str, Box<Expr>, Box<Expr>),
    IncDec { target: Box<Expr>, inc: bool, prefix: bool },
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Unary(&'static str, Box<Expr>),
    Cast(&'static str, Box<Expr>),
    Ternary(Box<Expr>, Option<Box<Expr>>, Box<Expr>),
    Call(String, Vec<Expr>),
    Isset(Vec<Expr>),
    Empty(Box<Expr>),
    Const(String),
    Print(Box<Expr>),
    Exit(Option<Box<Expr>>),
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct FuncDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Html(String),
    Echo(Vec<Expr>),
    Expr(Expr),
    If(Vec<(Expr, Vec<Stmt>)>, Option<Vec<Stmt>>),
    While(Expr, Vec<Stmt>),
    DoWhile(Vec<Stmt>, Expr),
    For(Vec<Expr>, Vec<Expr>, Vec<Expr>, Vec<Stmt>),
    Foreach { subject: Expr, key: Option<String>, val: String, body: Vec<Stmt> },
    Switch(Expr, Vec<(Option<Expr>, Vec<Stmt>)>),
    Break(u32),
    Continue(u32),
    Return(Option<Expr>),
    Func(FuncDecl),
    Unset(Vec<Expr>),
    Block(Vec<Stmt>),
}
