//! Fibers on top of stackful coroutines: `Fiber::suspend()` can unwind from any call depth.
use crate::object::{Extra, Obj};
use crate::value::{PhpArray, Value};
use crate::vm::{Flow, Vm};
use corosensei::{stack::DefaultStack, Coroutine, CoroutineResult, Yielder};
use std::{cell::RefCell, rc::Rc};

type R<T> = Result<T, Flow>;

pub enum Resume {
    Start,
    Value(Value),
    Throw(Value),
}

#[derive(Clone, Copy, PartialEq)]
pub enum Status {
    Init,
    Running,
    Suspended,
    Terminated,
}

type Co = Coroutine<Resume, Value, Result<Value, Flow>, DefaultStack>;

pub struct FiberState {
    co: Option<Co>,
    pub status: Status,
    callable: Value,
    ret: Value,
    yielder: *const Yielder<Resume, Value>,
}

impl FiberState {
    pub fn new(callable: Value) -> Self {
        FiberState { co: None, status: Status::Init, callable, ret: Value::Null, yielder: std::ptr::null() }
    }
}

impl Drop for FiberState {
    fn drop(&mut self) {
        // Unwinding a suspended coroutine would run PHP frames out of order; leak its stack instead.
        if let Some(co) = self.co.take() {
            std::mem::forget(co);
        }
    }
}

fn fiber_error<T>(vm: &mut Vm<'_>, msg: &str) -> R<T> {
    vm.throw_error("FiberError", msg)
}

pub fn method(vm: &mut Vm<'_>, obj: &Value, st: &Rc<RefCell<FiberState>>, lname: &str, args: Vec<Value>) -> R<Value> {
    let status = st.borrow().status;
    match lname {
        "start" => {
            if status != Status::Init {
                return fiber_error(vm, "Cannot start a fiber that has already been started");
            }
            let callable = st.borrow().callable.clone();
            let vm_ptr = vm as *mut Vm<'_> as *mut Vm<'static>;
            let state = st.clone();
            let stack = DefaultStack::new(32 << 20).map_err(|e| Flow::Fatal(format!("cannot allocate fiber stack: {e}")))?;
            let co: Co = Coroutine::with_stack(stack, move |yielder: &Yielder<Resume, Value>, _first: Resume| {
                state.borrow_mut().yielder = yielder as *const _;
                // SAFETY: the VM outlives every fiber and is only used by whichever side is currently running.
                let vm: &mut Vm<'static> = unsafe { &mut *vm_ptr };
                vm.call_value(&callable, args)
            });
            st.borrow_mut().co = Some(co);
            run(vm, obj, st, Resume::Start)
        }
        "resume" | "throw" => {
            if status != Status::Suspended {
                return fiber_error(vm, "Cannot resume a fiber that is not suspended");
            }
            let v = args.into_iter().next().unwrap_or(Value::Null);
            run(vm, obj, st, if lname == "throw" { Resume::Throw(v) } else { Resume::Value(v) })
        }
        "getreturn" => match status {
            Status::Terminated => Ok(st.borrow().ret.clone()),
            Status::Init => fiber_error(vm, "Cannot get fiber return value: The fiber has not been started"),
            _ => fiber_error(vm, "Cannot get fiber return value: The fiber has not returned"),
        },
        "isstarted" => Ok(Value::Bool(status != Status::Init)),
        "issuspended" => Ok(Value::Bool(status == Status::Suspended)),
        "isrunning" => Ok(Value::Bool(status == Status::Running)),
        "isterminated" => Ok(Value::Bool(status == Status::Terminated)),
        other => vm.throw_error("Error", &format!("Call to undefined method Fiber::{other}()")),
    }
}

fn run(vm: &mut Vm<'_>, obj: &Value, st: &Rc<RefCell<FiberState>>, input: Resume) -> R<Value> {
    let mut co = st.borrow_mut().co.take().expect("fiber coroutine");
    st.borrow_mut().status = Status::Running;
    vm.fiber_stack.push(obj.clone());
    let result = co.resume(input);
    vm.fiber_stack.pop();
    match result {
        CoroutineResult::Yield(v) => {
            let mut s = st.borrow_mut();
            s.status = Status::Suspended;
            s.co = Some(co);
            Ok(v)
        }
        CoroutineResult::Return(r) => {
            let mut s = st.borrow_mut();
            s.status = Status::Terminated;
            match r {
                Ok(v) => {
                    s.ret = v;
                    Ok(Value::Null)
                }
                Err(e) => Err(e),
            }
        }
    }
}

/// `Fiber::suspend()` / `Fiber::getCurrent()`.
pub fn static_call(vm: &mut Vm<'_>, lname: &str, args: Vec<Value>) -> Option<R<Value>> {
    match lname {
        "getcurrent" => Some(Ok(vm.fiber_stack.last().cloned().unwrap_or(Value::Null))),
        "suspend" => Some((|| {
            let Some(Value::Object(cur)) = vm.fiber_stack.last().cloned() else {
                return fiber_error(vm, "Cannot suspend outside of fiber");
            };
            let st = match &*cur.extra.borrow() {
                Extra::Fiber(s) => s.clone(),
                _ => return fiber_error(vm, "Cannot suspend outside of fiber"),
            };
            let yielder = st.borrow().yielder;
            // SAFETY: the yielder lives on the coroutine stack, which is alive while this fiber runs.
            let yielder = unsafe { &*yielder };
            match yielder.suspend(args.into_iter().next().unwrap_or(Value::Null)) {
                Resume::Value(v) => Ok(v),
                Resume::Throw(e) => Err(Flow::Throw(e)),
                Resume::Start => Ok(Value::Null),
            }
        })()),
        _ => None,
    }
}

pub fn create(class: Rc<crate::object::Class>, callable: Value) -> Value {
    Value::Object(Obj::new(class, PhpArray::new(), Extra::Fiber(Rc::new(RefCell::new(FiberState::new(callable))))))
}
