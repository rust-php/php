//! Runtime classes and objects.
use crate::ast::ClassKind;
use crate::value::{PhpArray, Value};
use std::{cell::RefCell, collections::HashMap, fmt, rc::Rc};

thread_local! {
    /// Freed object ids, reused LIFO like PHP's object store.
    static FREE_IDS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: RefCell<u32> = const { RefCell::new(1) };
}

#[derive(Clone)]
pub struct Method {
    pub name: String,
    pub func: usize,
    /// Class that (lexically) owns the code; traits are re-owned by the using class.
    pub class: String,
    pub is_static: bool,
    pub is_abstract: bool,
    pub vis: u8,
}

#[derive(Clone)]
pub struct PropInfo {
    pub ty: Option<crate::ast::TypeHint>,
    pub readonly: bool,
    pub name: String,
    pub thunk: Option<usize>,
    pub vis: u8,
    pub class: String,
}

pub struct Class {
    pub name: String,
    pub kind: ClassKind,
    pub is_abstract: bool,
    pub parent: Option<Rc<Class>>,
    /// Lowercased names of every interface this class implements (transitively).
    pub interfaces: Vec<String>,
    pub consts: RefCell<HashMap<String, Value>>,
    pub const_thunks: HashMap<String, usize>,
    pub props: Vec<PropInfo>,
    pub static_thunks: Vec<(String, Option<usize>)>,
    pub statics: RefCell<HashMap<String, Value>>,
    pub statics_ready: RefCell<bool>,
    pub methods: HashMap<String, Rc<Method>>,
    pub cases: Vec<(String, Option<usize>)>,
    pub backed: bool,
    /// Enum case singletons, created on first use.
    pub enum_vals: RefCell<Option<Vec<(String, Value)>>>,
}

impl Class {
    pub fn is_a(&self, lname: &str) -> bool {
        let mut cur = Some(self);
        while let Some(c) = cur {
            if c.name.eq_ignore_ascii_case(lname) {
                return true;
            }
            cur = c.parent.as_deref();
        }
        self.interfaces.iter().any(|i| i == lname)
    }

    pub fn find_method(&self, lname: &str) -> Option<Rc<Method>> {
        self.methods.get(lname).cloned()
    }

    pub fn prop_info(&self, name: &str) -> Option<&PropInfo> {
        self.props.iter().rev().find(|p| p.name == name)
    }

    pub fn prop_vis(&self, name: &str) -> Option<(u8, &str)> {
        self.props.iter().rev().find(|p| p.name == name).map(|p| (p.vis, p.class.as_str()))
    }
}

pub enum Extra {
    None,
    Closure(ClosureData),
    Generator(Rc<RefCell<crate::vm::GenState>>),
    Fiber(Rc<RefCell<crate::fiber::FiberState>>),
}

#[derive(Clone)]
pub struct ClosureData {
    pub func: usize,
    pub caps: Vec<Value>,
    pub this: Option<Value>,
    pub self_class: Option<Rc<Class>>,
    pub static_class: Option<Rc<Class>>,
}

pub struct Obj {
    pub id: u32,
    pub class: Rc<Class>,
    pub props: RefCell<PhpArray>,
    pub extra: RefCell<Extra>,
}

impl Obj {
    pub fn new(class: Rc<Class>, props: PhpArray, extra: Extra) -> Rc<Obj> {
        let id = FREE_IDS.with(|f| f.borrow_mut().pop()).unwrap_or_else(|| {
            NEXT_ID.with(|n| {
                let mut n = n.borrow_mut();
                let id = *n;
                *n += 1;
                id
            })
        });
        Rc::new(Obj { id, class, props: RefCell::new(props), extra: RefCell::new(extra) })
    }
}

impl Drop for Obj {
    fn drop(&mut self) {
        FREE_IDS.with(|f| f.borrow_mut().push(self.id));
    }
}

impl fmt::Debug for Obj {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.class.name, self.id)
    }
}
