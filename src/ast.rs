//! Abstract syntax tree.

#[derive(Debug, Clone)]
pub enum ClassRef {
    /// A class name, or one of `self` / `parent` / `static`.
    Named(String),
    Dyn(Box<Expr>),
}

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
    CallExpr(Box<Expr>, Vec<Expr>),
    Isset(Vec<Expr>),
    Empty(Box<Expr>),
    Const(String),
    Print(Box<Expr>),
    Exit(Option<Box<Expr>>),
    New(ClassRef, Vec<Expr>),
    /// `$obj->name`; `nullsafe` is `?->`.
    Prop { obj: Box<Expr>, name: Box<Expr>, nullsafe: bool },
    MethodCall { obj: Box<Expr>, name: Box<Expr>, args: Vec<Expr>, nullsafe: bool },
    StaticCall(ClassRef, String, Vec<Expr>),
    StaticProp(ClassRef, String),
    ClassConst(ClassRef, String),
    Instanceof(Box<Expr>, ClassRef),
    Clone(Box<Expr>),
    Closure(Box<ClosureDecl>),
    Throw(Box<Expr>),
    /// `$a = &$b`
    AssignRef(Box<Expr>, Box<Expr>),
    /// `...$args` inside a call or array literal
    Spread(Box<Expr>),
    /// Skipped slot in `[, $b] = ...`
    Hole,
    /// Destructuring pattern: `list($a, 'k' => $b)` / `[$a, [$b]]` as an assignment target.
    List(Vec<Option<(Option<Expr>, Expr)>>),
    Yield(Option<Box<Expr>>, Option<Box<Expr>>),
    YieldFrom(Box<Expr>),
    /// `match (subject) { a, b => x, default => y }`
    Match(Box<Expr>, Vec<(Vec<Expr>, Expr)>, Option<Box<Expr>>),
}

/// A declared type such as `?int`, `int|string` or `Foo`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeHint {
    /// Lowercased member names (`int`, `string`, `foo`, ...).
    pub names: Vec<String>,
    pub nullable: bool,
    /// Source spelling for messages.
    pub display: String,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub ty: Option<TypeHint>,
    pub name: String,
    pub default: Option<Expr>,
    /// Constructor property promotion (`public function __construct(private $x)`).
    pub promote: bool,
    pub by_ref: bool,
    pub variadic: bool,
}

#[derive(Debug, Clone)]
pub struct FuncDecl {
    pub ret: Option<TypeHint>,
    pub name: String,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct ClosureDecl {
    pub func: FuncDecl,
    /// (variable name, captured by reference)
    pub uses: Vec<(String, bool)>,
    pub arrow: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClassKind {
    Class,
    Interface,
    Trait,
    Enum,
}

#[derive(Debug, Clone)]
pub struct PropDecl {
    pub ty: Option<TypeHint>,
    pub readonly: bool,
    pub name: String,
    pub default: Option<Expr>,
    pub is_static: bool,
    /// 0 = public, 1 = protected, 2 = private
    pub vis: u8,
}

#[derive(Debug, Clone)]
pub struct MethodDecl {
    pub func: FuncDecl,
    pub is_static: bool,
    pub is_abstract: bool,
    pub vis: u8,
}

#[derive(Debug, Clone)]
pub struct ClassDecl {
    pub name: String,
    pub kind: ClassKind,
    pub is_abstract: bool,
    pub parent: Option<String>,
    pub interfaces: Vec<String>,
    pub traits: Vec<String>,
    pub consts: Vec<(String, Expr)>,
    pub props: Vec<PropDecl>,
    pub methods: Vec<MethodDecl>,
    /// Enum cases with optional backing value expressions.
    pub cases: Vec<(String, Option<Expr>)>,
    pub backed: bool,
}

#[derive(Debug, Clone)]
pub struct CatchClause {
    pub classes: Vec<String>,
    pub var: Option<String>,
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
    Foreach { subject: Expr, key: Option<String>, val: String, pattern: Option<Expr>, by_ref: bool, body: Vec<Stmt> },
    Global(Vec<String>),
    StaticVar(Vec<(String, Option<Expr>)>),
    Switch(Expr, Vec<(Option<Expr>, Vec<Stmt>)>),
    Break(u32),
    Continue(u32),
    Return(Option<Expr>),
    Func(FuncDecl),
    Class(ClassDecl),
    Try { body: Vec<Stmt>, catches: Vec<CatchClause>, finally: Option<Vec<Stmt>> },
    DefConst(String, Expr),
    Unset(Vec<Expr>),
    Block(Vec<Stmt>),
}
