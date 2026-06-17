//! Lexical environment records (scope chain).
//!
//! Each environment holds a set of mutable bindings plus an optional parent.
//! `this` binding and `var`-hoisting semantics are tracked via `kind`.

use crate::value::Value;
use std::cell::RefCell;
use indexmap::IndexMap;
use std::rc::Rc;

/// A binding cell. `mutable=false` for `const` and for some class-private
/// semantics; `initialized=false` models the temporal dead zone (TDZ).
#[derive(Debug)]
pub struct Binding {
    pub value: Value,
    pub mutable: bool,
    pub initialized: bool,
}

impl Default for Binding {
    fn default() -> Self {
        Binding {
            value: Value::Undefined,
            mutable: true,
            initialized: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum EnvKind {
    Global,
    Function,
    Block,
    Module,
    Class,
    With,
}

#[derive(Debug)]
pub struct EnvInner {
    pub bindings: IndexMap<Rc<str>, Binding>,
    pub parent: Option<Env>,
    pub kind: EnvKind,
    /// `this` value for this environment (function/global environments).
    pub this_val: Value,
    /// The `new.target` for this environment (None for non-function).
    pub new_target: Option<Value>,
    /// Home object for `super` lookups (the prototype where `super` methods live).
    pub home_object: Option<Value>,
    /// The constructor that `super()` should invoke (parent class).
    pub parent_constructor: Option<Value>,
    /// For `with` environments: the binding object.
    pub with_object: Option<Value>,
    /// Hoisted function declarations captured for re-declaration semantics.
    pub func_decls: Vec<Rc<str>>,
}

#[derive(Clone, Debug)]
pub struct Env(pub Rc<RefCell<EnvInner>>);

impl Env {
    pub fn new(parent: Option<Env>, kind: EnvKind) -> Env {
        Env(Rc::new(RefCell::new(EnvInner {
            bindings: IndexMap::new(),
            parent,
            kind,
            this_val: Value::Undefined,
            new_target: None,
            home_object: None,
            parent_constructor: None,
            with_object: None,
            func_decls: Vec::new(),
        })))
    }

    pub fn global() -> Env {
        Env::new(None, EnvKind::Global)
    }

    /// Create a binding (declaration). `mutable=false` for const.
    pub fn create(&self, name: &Rc<str>, value: Value, mutable: bool) {
        self.0.borrow_mut().bindings.insert(
            name.clone(),
            Binding {
                value,
                mutable,
                initialized: true,
            },
        );
    }

    /// Create an uninitialized binding (TDZ for let/const/class).
    pub fn create_uninit(&self, name: &Rc<str>, mutable: bool) {
        self.0.borrow_mut().bindings.insert(
            name.clone(),
            Binding {
                value: Value::Undefined,
                mutable,
                initialized: false,
            },
        );
    }

    pub fn has_own(&self, name: &str) -> bool {
        self.0.borrow().bindings.contains_key(name)
    }

    /// Resolve a binding walking up the scope chain.
    pub fn resolve(&self, name: &str) -> Option<Env> {
        let mut cur = Some(self.clone());
        while let Some(e) = cur {
            let found = {
                let inner = e.0.borrow();
                if inner.bindings.contains_key(name) {
                    true
                } else if let Some(with_obj) = &inner.with_object {
                    // `with` environment: check the object's properties.
                    if let Value::Object(o) = with_obj {
                        o.borrow().props.contains_key(&crate::value::PropKey::from_str(name))
                    } else {
                        false
                    }
                } else {
                    false
                }
            };
            if found {
                return Some(e);
            }
            cur = e.0.borrow().parent.clone();
        }
        None
    }

    /// Get a variable value, honoring TDZ.
    pub fn get(&self, name: &str) -> Result<Value, Value> {
        if let Some(e) = self.resolve(name) {
            let inner = e.0.borrow();
            let b = inner.bindings.get(name).expect("resolved binding must exist");
            if !b.initialized {
                return Err(crate::error::throw_reference(&format!(
                    "Cannot access '{}' before initialization",
                    name
                )));
            }
            Ok(b.value.clone())
        } else {
            Err(crate::error::throw_reference(&format!(
                "{} is not defined",
                name
            )))
        }
    }

    /// Set an existing binding; returns Err if not found or immutable.
    pub fn set(&self, name: &str, value: Value) -> Result<(), Value> {
        if let Some(e) = self.resolve(name) {
            let mut inner = e.0.borrow_mut();
            let b = inner.bindings.get_mut(name).expect("resolved binding must exist");
            if !b.initialized {
                return Err(crate::error::throw_reference(&format!(
                    "Cannot access '{}' before initialization",
                    name
                )));
            }
            if !b.mutable {
                return Err(crate::error::throw_type(&format!(
                    "Assignment to constant variable '{}'",
                    name
                )));
            }
            b.value = value;
            Ok(())
        } else {
            // Implicit global (sloppy mode).
            Err(crate::error::throw_reference(&format!("{} is not defined", name)))
        }
    }

    /// Walk up to find the nearest `this` value.
    pub fn this(&self) -> Value {
        let mut cur = Some(self.clone());
        while let Some(e) = cur {
            let inner = e.0.borrow();
            match inner.kind {
                EnvKind::Function | EnvKind::Global | EnvKind::Module => {
                    return inner.this_val.clone();
                }
                _ => {}
            }
            cur = inner.parent.clone();
        }
        Value::Undefined
    }

    pub fn new_target(&self) -> Option<Value> {
        let mut cur = Some(self.clone());
        while let Some(e) = cur {
            let inner = e.0.borrow();
            if inner.new_target.is_some() {
                return inner.new_target.clone();
            }
            cur = inner.parent.clone();
        }
        None
    }

    /// Find home object for `super` property access.
    pub fn home_object(&self) -> Option<Value> {
        let mut cur = Some(self.clone());
        while let Some(e) = cur {
            let inner = e.0.borrow();
            if inner.home_object.is_some() {
                return inner.home_object.clone();
            }
            cur = inner.parent.clone();
        }
        None
    }

    pub fn parent_constructor(&self) -> Option<Value> {
        let mut cur = Some(self.clone());
        while let Some(e) = cur {
            let inner = e.0.borrow();
            if inner.parent_constructor.is_some() {
                return inner.parent_constructor.clone();
            }
            cur = inner.parent.clone();
        }
        None
    }
}
