//! AST-walking interpreter.
//!
//! Evaluates a parsed `Program`. Supports the full statement/expression
//! grammar, lexical scoping, closures, classes, generators and async/await
//! (via stackful coroutines from `corosensei`), and Promises driven by the
//! Tokio-based microtask queue.

use crate::ast::*;
use crate::error;
use crate::realm::Realm;
use crate::scope::{Env, EnvKind};
use crate::value::*;
use crate::asyncrt::{self, AsyncRt};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Yielder type used by generator/async coroutines.
pub type Yielder = corosensei::Yielder<Result<Value, Value>, GeneratorYield>;

/// Shared, cheaply-clonable interpreter context.
pub struct Shared {
    pub realm: Rc<Realm>,
    pub async_rt: Rc<RefCell<AsyncRt>>,
    pub yielder: Cell<*const ()>,
    pub depth: Cell<usize>,
    pub max_depth: usize,
    pub stack: RefCell<Vec<String>>,
}

/// The interpreter. Cloning shares the realm/async state but gives the clone
/// its own current scope (used for coroutines).
#[derive(Clone)]
pub struct Interpreter {
    pub shared: Rc<Shared>,
    pub scope: Env,
}

/// Completion of a statement.
#[derive(Clone)]
pub enum Completion {
    Normal(Value),
    Return(Value),
    Break(Option<Rc<str>>),
    Continue(Option<Rc<str>>),
}

impl Completion {
    pub fn unwrap_value(self) -> Value {
        match self {
            Completion::Normal(v) | Completion::Return(v) => v,
            _ => Value::Undefined,
        }
    }
}

const MAX_DEPTH: usize = 1200;

impl Interpreter {
    pub fn new(realm: Rc<Realm>) -> Self {
        let shared = Rc::new(Shared {
            realm,
            async_rt: AsyncRt::new(),
            yielder: Cell::new(std::ptr::null()),
            depth: Cell::new(0),
            max_depth: MAX_DEPTH,
            stack: RefCell::new(Vec::new()),
        });
        Interpreter {
            shared: shared.clone(),
            scope: shared.realm.global_env.clone(),
        }
    }

    pub fn realm(&self) -> &Rc<Realm> {
        &self.shared.realm
    }

    // -----------------------------------------------------------------
    // Entry points
    // -----------------------------------------------------------------

    /// Parse and evaluate a script, returning the last expression value.
    /// If the source contains `export` or `import` statements, it is evaluated
    /// as a module (exports collected into a namespace object).
    pub fn run(&mut self, src: &str) -> Result<Value, Value> {
        // Detect module syntax: export/import at the top level.
        let is_module = src.lines().any(|line| {
            let t = line.trim();
            t.starts_with("export ") || t.starts_with("export{") || t.starts_with("export default")
                || t.starts_with("import ") || t.starts_with("import{") || t.starts_with("import *")
        });
        if is_module {
            let prog = crate::parser::parse_module(src).map_err(|e| {
                error::throw_syntax(&e.message)
            })?;
            return self.eval_module(&prog);
        }
        let prog = crate::parser::parse(src).map_err(|e| {
            error::throw_syntax(&e.message)
        })?;
        self.eval_program(&prog)
    }

    pub fn eval_program(&mut self, prog: &Program) -> Result<Value, Value> {
        // Hoist top-level function declarations and var declarations.
        self.hoist(&prog.body, &self.scope.clone(), true)?;
        let mut last = Value::Undefined;
        for s in &prog.body {
            match self.exec_stmt(s)? {
                Completion::Normal(v) => last = v,
                Completion::Return(v) => return Ok(v),
                _ => {}
            }
        }
        Ok(last)
    }

    /// Evaluate a program as an ES module: handle import/export statements.
    /// Returns the module namespace object (with all exports).
    pub fn eval_module(&mut self, prog: &Program) -> Result<Value, Value> {
        // Use a module-scoped environment.
        let mod_env = Env::new(Some(self.scope.clone()), EnvKind::Module);
        let saved = self.scope.clone();
        self.scope = mod_env;
        // Hoist top-level declarations.
        self.hoist(&prog.body, &self.scope.clone(), true)?;
        // Execute all statements, collecting exports.
        let mut exports: Vec<(String, Value)> = Vec::new();
        let mut default_export: Value = Value::Undefined;
        for s in &prog.body {
            match s {
                Stmt::Import(imp) => {
                    // Try to load the module file. If it fails, create undefined bindings.
                    let mod_ns = if imp.source.starts_with('.') || imp.source.starts_with('/') {
                        self.load_module(&imp.source).unwrap_or(Value::Undefined)
                    } else {
                        Value::Undefined
                    };
                    for spec in &imp.specifiers {
                        match spec {
                            crate::ast::ImportSpecifier::Default(local) => {
                                let val = if !mod_ns.is_undefined() {
                                    self.get_property(&mod_ns, &PropKey::from_str("default")).unwrap_or(Value::Undefined)
                                } else { Value::Undefined };
                                self.scope.create(local, val, true);
                            }
                            crate::ast::ImportSpecifier::Namespace(local) => {
                                self.scope.create(local, mod_ns.clone(), true);
                            }
                            crate::ast::ImportSpecifier::Named { imported, local } => {
                                let val = if !mod_ns.is_undefined() {
                                    self.get_property(&mod_ns, &PropKey::from_str(imported)).unwrap_or(Value::Undefined)
                                } else { Value::Undefined };
                                self.scope.create(local, val, true);
                            }
                        }
                    }
                }
                Stmt::ExportNamed(en) => {
                    // Execute the declaration (if any) and collect exports.
                    if let Some(decl) = &en.declaration {
                        self.exec_stmt(decl)?;
                        // Collect exported bindings.
                        if let Stmt::Var(v) = decl.as_ref() {
                            for d in &v.decls {
                                if let Pattern::Ident(n) = &d.pattern {
                                    let val = self.scope.get(n)?;
                                    exports.push((n.to_string(), val));
                                }
                            }
                        }
                        if let Stmt::Function(fd) = decl.as_ref() {
                            if let Some(name) = &fd.name {
                                let val = self.scope.get(name)?;
                                exports.push((name.to_string(), val));
                            }
                        }
                        if let Stmt::Class(c) = decl.as_ref() {
                            if let Some(name) = &c.name {
                                let val = self.scope.get(name)?;
                                exports.push((name.to_string(), val));
                            }
                        }
                    }
                    // Re-export specifiers (from another module) — best-effort: skip.
                    for (local, exported) in &en.specifiers {
                        let val = self.scope.get(local).unwrap_or(Value::Undefined);
                        exports.push((exported.to_string(), val));
                    }
                }
                Stmt::ExportDefault(ed) => {
                    let v = self.eval_expr(&ed.expr)?;
                    default_export = v;
                    exports.push(("default".to_string(), default_export.clone()));
                }
                Stmt::ExportAll(_) => {
                    // Re-export all from another module — can't load, skip.
                }
                _ => {
                    self.exec_stmt(s)?;
                }
            }
        }
        self.scope = saved;
        // Build the module namespace object.
        let ns = ObjectInner::new_object();
        ns.borrow_mut().proto = Some(Value::Object(self.realm().object_proto.clone()));
        ns.borrow_mut().class = "Module";
        for (name, val) in &exports {
            ns.borrow_mut().props.insert(
                PropKey::from_str(name),
                Property::data(val.clone()),
            );
        }
        Ok(Value::Object(ns))
    }

    // -----------------------------------------------------------------
    // Hoisting
    // -----------------------------------------------------------------

    fn hoist(&mut self, stmts: &[Stmt], env: &Env, top: bool) -> Result<(), Value> {
        for s in stmts {
            match s {
                Stmt::Function(fd) => {
                    if let Some(name) = &fd.name {
                        let func = self.make_function(
                            &fd.func,
                            fd.is_async,
                            fd.is_generator,
                            env.clone(),
                        );
                        if top && env.0.borrow().kind == EnvKind::Global {
                            // also define on global object
                            self.define_global(name, func.clone());
                        }
                        if env.has_own(name) {
                            // reassign for var-style
                            let _ = env.set(name, func);
                        } else {
                            env.create(name, func, true);
                        }
                    }
                }
                Stmt::Var(v) if v.kind == VarKind::Var => {
                    for d in &v.decls {
                        self.hoist_pattern(&d.pattern, env);
                    }
                }
                Stmt::Block(b) => {
                    // var hoists out of blocks; function declarations are block-scoped
                    // (we treat them as block-scoped here for simplicity)
                    self.hoist(&b.stmts, env, false)?;
                }
                Stmt::If { cons, alt, .. } => {
                    self.hoist(std::slice::from_ref(cons), env, false)?;
                    if let Some(a) = alt {
                        self.hoist(std::slice::from_ref(a), env, false)?;
                    }
                }
                Stmt::For { body, init, .. } => {
                    if let Some(ForInit::Var(v)) = init {
                        if v.kind == VarKind::Var {
                            for d in &v.decls {
                                self.hoist_pattern(&d.pattern, env);
                            }
                        }
                    }
                    self.hoist(std::slice::from_ref(body), env, false)?;
                }
                Stmt::While { body, .. }
                | Stmt::DoWhile { body, .. }
                | Stmt::Labeled { body, .. } => {
                    self.hoist(std::slice::from_ref(body), env, false)?;
                }
                Stmt::Try { block, handler, finalizer, .. } => {
                    self.hoist(&block.stmts, env, false)?;
                    if let Some(h) = handler {
                        self.hoist(&h.body.stmts, env, false)?;
                    }
                    if let Some(f) = finalizer {
                        self.hoist(&f.stmts, env, false)?;
                    }
                }
                Stmt::Switch { cases, .. } => {
                    for c in cases {
                        self.hoist(&c.cons, env, false)?;
                    }
                }
                Stmt::ExportNamed(en) => {
                    if let Some(decl) = &en.declaration {
                        self.hoist(std::slice::from_ref(decl), env, top)?;
                    }
                }
                Stmt::ExportDefault(_) => {
                    // default export of a function/class is hoisted as `default`
                    // (simplified: not hoisted, evaluated in order)
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn hoist_pattern(&self, pat: &Pattern, env: &Env) {
        match pat {
            Pattern::Ident(n) => {
                if !env.has_own(n) {
                    env.create(n, Value::Undefined, true);
                }
            }
            Pattern::Array { elements, rest } => {
                for e in elements {
                    if let Some(pe) = e {
                        self.hoist_pattern(&pe.pattern, env);
                    }
                }
                if let Some(r) = rest {
                    self.hoist_pattern(r, env);
                }
            }
            Pattern::Object { properties, rest } => {
                for p in properties {
                    self.hoist_pattern(&p.value, env);
                }
                if let Some(r) = rest {
                    if !env.has_own(r) {
                        env.create(r, Value::Undefined, true);
                    }
                }
            }
            Pattern::Rest(p) => self.hoist_pattern(p, env),
            Pattern::Assignment { pattern, .. } => self.hoist_pattern(pattern, env),
            Pattern::ArrayHole => {}
        }
    }

    // -----------------------------------------------------------------
    // Statements
    // -----------------------------------------------------------------

    pub fn exec_stmt(&mut self, s: &Stmt) -> Result<Completion, Value> {
        self.enter()?;
        let r = self.exec_stmt_inner(s);
        self.leave();
        r
    }

    fn enter(&self) -> Result<(), Value> {
        let d = self.shared.depth.get();
        if d >= self.shared.max_depth {
            return Err(error::throw_range("Maximum call stack size exceeded"));
        }
        self.shared.depth.set(d + 1);
        Ok(())
    }
    fn leave(&self) {
        self.shared.depth.set(self.shared.depth.get() - 1);
    }

    fn exec_stmt_inner(&mut self, s: &Stmt) -> Result<Completion, Value> {
        match s {
            Stmt::Empty => Ok(Completion::Normal(Value::Undefined)),
            Stmt::Block(b) => {
                let env = Env::new(Some(self.scope.clone()), EnvKind::Block);
                let saved = self.scope.clone();
                self.scope = env;
                // hoist block-scoped function declarations
                self.hoist(&b.stmts, &self.scope.clone(), false)?;
                let mut last = Value::Undefined;
                for st in &b.stmts {
                    let c = self.exec_stmt(st)?;
                    match c {
                        Completion::Normal(v) => last = v,
                        other => {
                            self.scope = saved;
                            return Ok(other);
                        }
                    }
                }
                self.scope = saved;
                Ok(Completion::Normal(last))
            }
            Stmt::Expr(e) => {
                let v = self.eval_expr(e)?;
                Ok(Completion::Normal(v))
            }
            Stmt::Var(v) => {
                for d in &v.decls {
                    let mutable = v.kind != VarKind::Const;
                    // For `var` without an initializer, the binding was already
                    // created by hoisting with `undefined`. Skip re-binding so we
                    // don't clobber a value assigned before the var statement.
                    if v.kind == VarKind::Var && d.init.is_none() {
                        continue;
                    }
                    let val = match &d.init {
                        Some(e) => self.eval_expr(e)?,
                        None => Value::Undefined,
                    };
                    self.bind_pattern(&d.pattern, val, &self.scope.clone(), v.kind, mutable)?;
                }
                Ok(Completion::Normal(Value::Undefined))
            }
            Stmt::Function(_) => {
                // already hoisted
                Ok(Completion::Normal(Value::Undefined))
            }
            Stmt::Class(c) => {
                let cls = self.make_class(c, &self.scope.clone())?;
                if let Some(name) = &c.name {
                    self.scope.create(name, cls, false);
                }
                Ok(Completion::Normal(Value::Undefined))
            }
            Stmt::Return(arg) => {
                let v = match arg {
                    Some(e) => self.eval_expr(e)?,
                    None => Value::Undefined,
                };
                Ok(Completion::Return(v))
            }
            Stmt::If { test, cons, alt } => {
                let t = self.eval_expr(test)?;
                if to_boolean(&t) {
                    self.exec_stmt(cons)
                } else if let Some(a) = alt {
                    self.exec_stmt(a)
                } else {
                    Ok(Completion::Normal(Value::Undefined))
                }
            }
            Stmt::While { test, body } => {
                let mut last = Value::Undefined;
                loop {
                    let t = self.eval_expr(test)?;
                    if !to_boolean(&t) {
                        break;
                    }
                    match self.exec_stmt(body)? {
                        Completion::Normal(v) => last = v,
                        Completion::Return(v) => return Ok(Completion::Return(v)),
                        Completion::Break(None) => break,
                        Completion::Break(Some(_)) => break,
                        Completion::Continue(None) => continue,
                        Completion::Continue(Some(_)) => continue,
                    }
                }
                Ok(Completion::Normal(last))
            }
            Stmt::DoWhile { test, body } => {
                let mut last = Value::Undefined;
                loop {
                    match self.exec_stmt(body)? {
                        Completion::Normal(v) => last = v,
                        Completion::Return(v) => return Ok(Completion::Return(v)),
                        Completion::Break(None) => break,
                        Completion::Break(Some(_)) => break,
                        Completion::Continue(None) => {}
                        Completion::Continue(Some(_)) => {}
                    }
                    let t = self.eval_expr(test)?;
                    if !to_boolean(&t) {
                        break;
                    }
                }
                Ok(Completion::Normal(last))
            }
            Stmt::For { init, test, update, body } => {
                let for_env = Env::new(Some(self.scope.clone()), EnvKind::Block);
                let saved = self.scope.clone();
                self.scope = for_env;
                if let Some(init) = init {
                    match init {
                        ForInit::Var(v) => {
                            for d in &v.decls {
                                let val = match &d.init {
                                    Some(e) => self.eval_expr(e)?,
                                    None => Value::Undefined,
                                };
                                self.bind_pattern(
                                    &d.pattern,
                                    val,
                                    &self.scope.clone(),
                                    v.kind,
                                    v.kind != VarKind::Const,
                                )?;
                            }
                        }
                        ForInit::Expr(e) => {
                            self.eval_expr(e)?;
                        }
                    }
                }
                let mut last = Value::Undefined;
                loop {
                    if let Some(t) = test {
                        let tv = self.eval_expr(t)?;
                        if !to_boolean(&tv) {
                            break;
                        }
                    }
                    match self.exec_stmt(body)? {
                        Completion::Normal(v) => last = v,
                        Completion::Return(v) => {
                            self.scope = saved;
                            return Ok(Completion::Return(v));
                        }
                        Completion::Break(None) => break,
                        Completion::Break(Some(_)) => break,
                        Completion::Continue(None) => {}
                        Completion::Continue(Some(_)) => {}
                    }
                    if let Some(u) = update {
                        self.eval_expr(u)?;
                    }
                }
                self.scope = saved;
                Ok(Completion::Normal(last))
            }
            Stmt::ForIn { left, right, body } => {
                let obj = self.eval_expr(right)?;
                let keys = self.enumerate_for_in(&obj)?;
                let mut last = Value::Undefined;
                for k in keys {
                    let kv = Value::from_string(k.to_string());
                    self.assign_for_target(left, kv.clone())?;
                    match self.exec_stmt(body)? {
                        Completion::Normal(v) => last = v,
                        Completion::Return(v) => return Ok(Completion::Return(v)),
                        Completion::Break(None) => break,
                        Completion::Break(Some(_)) => break,
                        Completion::Continue(None) => continue,
                        Completion::Continue(Some(_)) => continue,
                    }
                }
                Ok(Completion::Normal(last))
            }
            Stmt::ForOf { left, right, body, .. } => {
                let iter_val = self.eval_expr(right)?;
                let iter = self.get_iterator(&iter_val)?;
                let mut last = Value::Undefined;
                loop {
                    let next = self.iterator_step(&iter)?;
                    match next {
                        None => break,
                        Some(v) => {
                            self.assign_for_target(left, v)?;
                            match self.exec_stmt(body)? {
                                Completion::Normal(x) => last = x,
                                Completion::Return(v) => {
                                    return Ok(Completion::Return(v));
                                }
                                Completion::Break(None) => break,
                                Completion::Break(Some(_)) => break,
                                Completion::Continue(None) => continue,
                                Completion::Continue(Some(_)) => continue,
                            }
                        }
                    }
                }
                Ok(Completion::Normal(last))
            }
            Stmt::Switch { disc, cases } => {
                let d = self.eval_expr(disc)?;
                let mut matched = false;
                let mut last = Value::Undefined;
                let env = Env::new(Some(self.scope.clone()), EnvKind::Block);
                let saved = self.scope.clone();
                self.scope = env;
                self.hoist(
                    &cases.iter().flat_map(|c| c.cons.iter()).cloned().collect::<Vec<_>>(),
                    &self.scope.clone(),
                    false,
                )?;
                for (i, c) in cases.iter().enumerate() {
                    if !matched {
                        if let Some(t) = &c.test {
                            let tv = self.eval_expr(t)?;
                            if strict_equals(&d, &tv) {
                                matched = true;
                            }
                        }
                    }
                    if matched {
                        for st in &c.cons {
                            match self.exec_stmt(st)? {
                                Completion::Normal(v) => last = v,
                                Completion::Return(v) => {
                                    self.scope = saved;
                                    return Ok(Completion::Return(v));
                                }
                                Completion::Break(None) => {
                                    self.scope = saved;
                                    return Ok(Completion::Normal(last));
                                }
                                Completion::Break(Some(_)) => {
                                    self.scope = saved;
                                    return Ok(Completion::Normal(last));
                                }
                                Completion::Continue(_) => {
                                    self.scope = saved;
                                    return Ok(Completion::Continue(None));
                                }
                            }
                        }
                    }
                    let _ = i;
                }
                // default
                if !matched {
                    if let Some(di) = cases.iter().position(|c| c.test.is_none()) {
                        for c in &cases[di..] {
                            for st in &c.cons {
                                match self.exec_stmt(st)? {
                                    Completion::Normal(v) => last = v,
                                    Completion::Return(v) => {
                                        self.scope = saved;
                                        return Ok(Completion::Return(v));
                                    }
                                    Completion::Break(None) => {
                                        self.scope = saved;
                                        return Ok(Completion::Normal(last));
                                    }
                                    Completion::Break(Some(_)) => {
                                        self.scope = saved;
                                        return Ok(Completion::Normal(last));
                                    }
                                    Completion::Continue(_) => {
                                        self.scope = saved;
                                        return Ok(Completion::Continue(None));
                                    }
                                }
                            }
                        }
                    }
                }
                let _ = matched;
                self.scope = saved;
                Ok(Completion::Normal(last))
            }
            Stmt::Break(label) => Ok(Completion::Break(label.clone())),
            Stmt::Continue(label) => Ok(Completion::Continue(label.clone())),
            Stmt::Throw(e) => {
                let v = self.eval_expr(e)?;
                Err(v)
            }
            Stmt::Try { block, handler, finalizer } => {
                let env = Env::new(Some(self.scope.clone()), EnvKind::Block);
                let saved = self.scope.clone();
                self.scope = env;
                self.hoist(&block.stmts, &self.scope.clone(), false)?;
                let result: Result<Completion, Value> = (|| {
                    let mut last = Value::Undefined;
                    for st in &block.stmts {
                        match self.exec_stmt(st)? {
                            Completion::Normal(v) => last = v,
                            other => return Ok(other),
                        }
                    }
                    Ok(Completion::Normal(last))
                })();
                let result = match result {
                    Ok(c) => Ok(c),
                    Err(e) => {
                        if let Some(h) = handler {
                            let catch_env = Env::new(Some(self.scope.clone()), EnvKind::Block);
                            if let Some(p) = &h.param {
                                self.bind_pattern(p, e, &catch_env.clone(), VarKind::Let, true)?;
                            }
                            let saved2 = self.scope.clone();
                            self.scope = catch_env;
                            self.hoist(&h.body.stmts, &self.scope.clone(), false)?;
                            let r = (|| {
                                let mut last = Value::Undefined;
                                for st in &h.body.stmts {
                                    match self.exec_stmt(st)? {
                                        Completion::Normal(v) => last = v,
                                        other => return Ok(other),
                                    }
                                }
                                Ok(Completion::Normal(last))
                            })();
                            self.scope = saved2;
                            r
                        } else {
                            Err(e)
                        }
                    }
                };
                if let Some(f) = finalizer {
                    let saved3 = self.scope.clone();
                    let fenv = Env::new(Some(self.scope.clone()), EnvKind::Block);
                    self.scope = fenv;
                    self.hoist(&f.stmts, &self.scope.clone(), false)?;
                    let fres = (|| {
                        for st in &f.stmts {
                            self.exec_stmt(st)?;
                        }
                        Ok::<(), Value>(())
                    })();
                    self.scope = saved3;
                    fres?;
                }
                self.scope = saved;
                result
            }
            Stmt::Labeled { label, body } => {
                match self.exec_stmt(body)? {
                    Completion::Break(Some(l)) if l == *label => Ok(Completion::Normal(Value::Undefined)),
                    Completion::Continue(Some(l)) if l == *label => Ok(Completion::Normal(Value::Undefined)),
                    other => Ok(other),
                }
            }
            Stmt::Debugger => Ok(Completion::Normal(Value::Undefined)),
            Stmt::With { object, body } => {
                let obj = self.eval_expr(object)?;
                let wenv = Env::new(Some(self.scope.clone()), EnvKind::With);
                wenv.0.borrow_mut().with_object = Some(obj);
                let saved = self.scope.clone();
                self.scope = wenv;
                let r = self.exec_stmt(body);
                self.scope = saved;
                r
            }
            Stmt::Import(_) | Stmt::ExportNamed(_) | Stmt::ExportDefault(_) | Stmt::ExportAll(_) => {
                // Module statements handled in module eval; no-op in script mode.
                Ok(Completion::Normal(Value::Undefined))
            }
        }
    }

    fn assign_for_target(&mut self, left: &ForTarget, value: Value) -> Result<(), Value> {
        match left {
            ForTarget::Var(kind, pat) => {
                let mutable = *kind != VarKind::Const;
                self.bind_pattern(pat, value, &self.scope.clone(), *kind, mutable)
            }
            ForTarget::Pattern(target) => self.assign_target(target, value),
        }
    }

    fn enumerate_for_in(&mut self, obj: &Value) -> Result<Vec<Rc<str>>, Value> {
        let mut keys: Vec<Rc<str>> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        // If the target is a Proxy, use its ownKeys trap for the first level.
        if let Value::Object(o) = obj {
            if matches!(o.borrow().kind, ObjectKind::Proxy(_)) {
                let own = self.own_property_keys(obj)?;
                for k in own {
                    if let PropKey::Str(s) = k {
                        if seen.insert(s.to_string()) {
                            keys.push(s);
                        }
                    }
                }
                // Walk prototype chain (target's proto)
                let proto = self.get_prototype_of(obj)?;
                if proto.is_object() {
                    let more = self.enumerate_for_in(&proto)?;
                    for k in more {
                        if seen.insert(k.to_string()) {
                            keys.push(k);
                        }
                    }
                }
                return Ok(keys);
            }
        }
        let mut cur = Some(obj.clone());
        while let Some(v) = cur {
            match v {
                Value::Object(o) => {
                    let b = o.borrow();
                    if let ObjectKind::Array(items) = &b.kind {
                        for i in 0..items.len() {
                            let k = i.to_string();
                            if seen.insert(k.clone()) {
                                keys.push(Rc::from(k.as_str()));
                            }
                        }
                    }
                    for (k, p) in b.props.iter() {
                        if p.enumerable {
                            let ks = match k {
                                PropKey::Str(s) => s.to_string(),
                                PropKey::Sym(_) => continue,
                            };
                            if seen.insert(ks.clone()) {
                                keys.push(Rc::from(ks.as_str()));
                            }
                        }
                    }
                    cur = b.proto.clone();
                }
                Value::String(s) => {
                    for (i, _) in s.char_indices() {
                        let k = i.to_string();
                        if seen.insert(k.clone()) {
                            keys.push(Rc::from(k.as_str()));
                        }
                    }
                    cur = None;
                }
                _ => cur = None,
            }
        }
        Ok(keys)
    }

    // -----------------------------------------------------------------
    // Expressions
    // -----------------------------------------------------------------

    pub fn eval_expr(&mut self, e: &Expr) -> Result<Value, Value> {
        self.enter()?;
        let r = self.eval_expr_inner(e);
        self.leave();
        r
    }

    fn eval_expr_inner(&mut self, e: &Expr) -> Result<Value, Value> {
        match e {
            Expr::Number(n) => Ok(Value::Number(*n)),
            Expr::BigInt(s) => Ok(Value::BigInt(Rc::new(parse_bigint(s)))),
            Expr::String(s) => Ok(Value::String(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Null => Ok(Value::Null),
            Expr::Undefined => Ok(Value::Undefined),
            Expr::Empty => Ok(Value::Undefined),
            Expr::Spread(e) => self.eval_expr(e),
            Expr::This => Ok(self.scope.this()),
            Expr::Ident(n) => self.scope.get(n),
            Expr::Paren(e) => self.eval_expr(e),
            Expr::NewTarget => Ok(self.scope.new_target().unwrap_or(Value::Undefined)),
            Expr::ImportMeta => Ok(Value::Object(self.make_import_meta())),
            Expr::ImportCall(arg) => self.eval_import_call(arg),
            Expr::Sequence(exprs) => {
                let mut last = Value::Undefined;
                for e in exprs {
                    last = self.eval_expr(e)?;
                }
                Ok(last)
            }
            Expr::TemplateLit { quasis, exprs, tag } => {
                if let Some(tag) = tag {
                    return self.eval_tagged_template(tag, quasis, exprs);
                }
                let mut s = String::new();
                for (i, q) in quasis.iter().enumerate() {
                    s.push_str(q);
                    if i < exprs.len() {
                        let v = self.eval_expr(&exprs[i])?;
                        s.push_str(&to_string(&v));
                    }
                }
                Ok(Value::from_string(s))
            }
            Expr::TaggedTemplate { tag, quasis, exprs } => {
                self.eval_tagged_template(tag, quasis, exprs)
            }
            Expr::Regex { pattern, flags } => self.make_regexp(pattern, flags),
            Expr::Array(els) => {
                let mut items = Vec::new();
                for el in els {
                    match el {
                        ArrayElement::Hole => items.push(Value::Undefined),
                        ArrayElement::Item(e) => items.push(self.eval_expr(e)?),
                        ArrayElement::Spread(e) => {
                            let v = self.eval_expr(e)?;
                            self.flatten_into(&mut items, &v)?;
                        }
                    }
                }
                Ok(self.new_array(items))
            }
            Expr::Object(props) => self.eval_object_lit(props),
            Expr::Function(f) => {
                Ok(self.make_function(f, f.is_async, f.is_generator, self.scope.clone()))
            }
            Expr::Arrow(f) => Ok(self.make_function(f, f.is_async, f.is_generator, self.scope.clone())),
            Expr::Class(c) => self.make_class(c, &self.scope.clone()),
            Expr::Unary { op, arg } => self.eval_unary(*op, arg),
            Expr::Update { op, arg, prefix } => self.eval_update(*op, arg, *prefix),
            Expr::Binary { op, left, right } => self.eval_binary(*op, left, right),
            Expr::Logical { op, left, right } => self.eval_logical(*op, left, right),
            Expr::Assignment { op, left, right } => self.eval_assignment(*op, left, right),
            Expr::Conditional { test, cons, alt } => {
                let t = self.eval_expr(test)?;
                if to_boolean(&t) {
                    self.eval_expr(cons)
                } else {
                    self.eval_expr(alt)
                }
            }
            Expr::Call { callee, args, optional } => {
                self.eval_call(callee, args, *optional)
            }
            Expr::New { callee, args } => self.eval_new(callee, args),
            Expr::Member { object, property, optional } => {
                let (this, val) = self.eval_member(object, property, *optional)?;
                let _ = this;
                Ok(val)
            }
            Expr::Yield { arg, delegate } => self.eval_yield(arg, *delegate),
            Expr::Await(arg) => self.eval_await(arg),
            Expr::Super => Err(error::throw_syntax("'super' keyword unexpected")),
        }
    }

    pub fn flatten_into(&mut self, items: &mut Vec<Value>, v: &Value) -> Result<(), Value> {
        // If iterable, spread via iterator; else if array, copy.
        if let Value::Object(o) = v {
            if let ObjectKind::Array(arr) = &o.borrow().kind {
                items.extend(arr.iter().cloned());
                return Ok(());
            }
        }
        if matches!(v, Value::String(_)) || self.is_iterable(v) {
            let iter = self.get_iterator(v)?;
            loop {
                let n = self.iterator_step(&iter)?;
                match n {
                    Some(x) => items.push(x),
                    None => break,
                }
            }
            return Ok(());
        }
        items.push(v.clone());
        Ok(())
    }

    fn eval_object_lit(&mut self, props: &[ObjectProp]) -> Result<Value, Value> {
        let obj = ObjectInner::new_object();
        obj.borrow_mut().proto = Some(Value::Object(self.realm().object_proto.clone()));
        for p in props {
            if matches!(p.kind, PropKindAst::Spread) {
                if let ObjectPropValue::Expr(e) = &p.value {
                    let v = self.eval_expr(e)?;
                    if let Value::Object(src) = &v {
                        let src_b = src.borrow();
                        let keys: Vec<(PropKey, Property)> = src_b
                            .props
                            .iter()
                            .filter(|(_, pr)| pr.enumerable)
                            .map(|(k, pr)| (k.clone(), pr.clone()))
                            .collect();
                        for (k, pr) in keys {
                            obj.borrow_mut().props.insert(k, pr);
                        }
                        if let ObjectKind::Array(items) = &src_b.kind {
                            for (i, it) in items.iter().enumerate() {
                                let k = index_to_key(i);
                                obj.borrow_mut().props.insert(
                                    PropKey::Str(k),
                                    Property::data(it.clone()),
                                );
                            }
                            obj.borrow_mut().props.insert(
                                PropKey::from_str("length"),
                                Property {
                                    kind: PropKind::Data(Value::from_int(items.len() as i32)),
                                    writable: true,
                                    enumerable: false,
                                    configurable: false,
                                },
                            );
                        }
                    }
                }
                continue;
            }
            let key = self.eval_property_key(&p.key, p.computed)?;
            match &p.value {
                ObjectPropValue::Expr(e) => {
                    if matches!(p.kind, PropKindAst::Get | PropKindAst::Set) {
                        // accessor
                        let func = self.eval_expr(e)?;
                        let existing = obj.borrow().props.get(&key).cloned();
                        let prop = match existing {
                            Some(mut ex) if matches!(ex.kind, PropKind::Accessor { .. }) => {
                                if matches!(p.kind, PropKindAst::Get) {
                                    ex.kind = PropKind::Accessor {
                                        get: Some(func),
                                        set: match ex.kind {
                                            PropKind::Accessor { set, .. } => set,
                                            _ => None,
                                        },
                                    };
                                } else {
                                    ex.kind = PropKind::Accessor {
                                        get: match ex.kind {
                                            PropKind::Accessor { get, .. } => get,
                                            _ => None,
                                        },
                                        set: Some(func),
                                    };
                                }
                                ex
                            }
                            _ => Property {
                                kind: if matches!(p.kind, PropKindAst::Get) {
                                    PropKind::Accessor { get: Some(func), set: None }
                                } else {
                                    PropKind::Accessor { get: None, set: Some(func) }
                                },
                                writable: false,
                                enumerable: true,
                                configurable: true,
                            },
                        };
                        obj.borrow_mut().props.insert(key, prop);
                    } else {
                        let v = self.eval_expr(e)?;
                        obj.borrow_mut().props.insert(key, Property::data(v));
                    }
                }
                ObjectPropValue::Pattern(_) => {
                    return Err(error::throw_syntax("pattern in object literal"));
                }
            }
        }
        Ok(Value::Object(obj))
    }

    fn eval_property_key(&mut self, key: &PropertyKey, computed: bool) -> Result<PropKey, Value> {
        if computed {
            if let PropertyKey::Computed(e) = key {
                let v = self.eval_expr(e)?;
                return Ok(to_property_key(&v));
            }
        }
        Ok(match key {
            PropertyKey::Ident(n) | PropertyKey::String(n) => PropKey::Str(n.clone()),
            PropertyKey::Number(n) => PropKey::Str(Rc::from(format_number(*n).as_str())),
            PropertyKey::Private(n) => PropKey::Str(n.clone()),
            PropertyKey::Computed(e) => {
                let v = self.eval_expr(e)?;
                to_property_key(&v)
            }
        })
    }

    fn eval_tagged_template(
        &mut self,
        tag: &Expr,
        quasis: &[Rc<str>],
        exprs: &[Expr],
    ) -> Result<Value, Value> {
        let tag_val = self.eval_expr(tag)?;
        // build strings array
        let mut strs = Vec::new();
        for q in quasis {
            strs.push(Value::String(q.clone()));
        }
        let arr = self.new_array(strs);
        // raw
        if let Value::Object(a) = &arr {
            let raw_arr = self.new_array(quasis.iter().map(|q| Value::String(q.clone())).collect());
            a.borrow_mut().props.insert(
                PropKey::from_str("raw"),
                Property::data(raw_arr),
            );
        }
        let mut args = vec![arr];
        for e in exprs {
            args.push(self.eval_expr(e)?);
        }
        let (this, _) = self.eval_member_receiver(tag)?;
        let _ = this;
        self.call_value(tag_val, Value::Undefined, &args)
    }

    fn eval_unary(&mut self, op: UnaryOp, arg: &Expr) -> Result<Value, Value> {
        match op {
            UnaryOp::TypeOf => {
                // typeof of undeclared identifier doesn't throw
                if let Expr::Ident(n) = arg {
                    if self.scope.resolve(n).is_none() {
                        return Ok(Value::from_str("undefined"));
                    }
                }
                let v = self.eval_expr(arg)?;
                Ok(Value::from_str(v.type_of()))
            }
            UnaryOp::Void => {
                self.eval_expr(arg)?;
                Ok(Value::Undefined)
            }
            UnaryOp::Delete => {
                match arg {
                    Expr::Member { object, property, .. } => {
                        let obj_v = self.eval_expr(object)?;
                        let key = match property {
                            MemberProp::Ident(n) => PropKey::Str(n.clone()),
                            MemberProp::Private(n) => PropKey::Str(n.clone()),
                            MemberProp::Computed(e) => {
                                let kv = self.eval_expr(&e)?;
                                to_property_key(&kv)
                            }
                        };
                        let obj = self.to_object(&obj_v)?;
                        // Proxy "deleteProperty" trap
                        if let Value::Object(o) = &obj {
                            if matches!(o.borrow().kind, ObjectKind::Proxy(_)) {
                                let pd = if let ObjectKind::Proxy(pd) = &o.borrow().kind { pd.clone() } else { unreachable!() };
                                if pd.revoked {
                                    return Err(error::throw_type("Cannot perform 'deleteProperty' on a proxy that has been revoked"));
                                }
                                let trap = self.get_property(&pd.handler, &PropKey::from_str("deleteProperty"))?;
                                if trap.is_callable() {
                                    let key_val = match &key {
                                        PropKey::Str(s) => Value::String(s.clone()),
                                        PropKey::Sym(s) => Value::Symbol(s.clone()),
                                    };
                                    let r = self.call_value(trap, pd.handler.clone(), &[pd.target.clone(), key_val])?;
                                    return Ok(Value::Bool(to_boolean(&r)));
                                }
                                // default: forward to target
                                return Ok(Value::Bool(self.delete_property(&pd.target, &key)?));
                            }
                        }
                        Ok(Value::Bool(self.delete_property(&obj, &key)?))
                    }
                    Expr::Ident(_) => Ok(Value::Bool(true)),
                    _ => {
                        self.eval_expr(arg)?;
                        Ok(Value::Bool(true))
                    }
                }
            }
            UnaryOp::Neg => {
                let v = self.eval_expr(arg)?;
                Ok(Value::Number(-to_number(&v)))
            }
            UnaryOp::Pos => {
                let v = self.eval_expr(arg)?;
                Ok(Value::Number(to_number(&v)))
            }
            UnaryOp::Not => {
                let v = self.eval_expr(arg)?;
                Ok(Value::Bool(!to_boolean(&v)))
            }
            UnaryOp::BitNot => {
                let v = self.eval_expr(arg)?;
                Ok(Value::Number((!to_int32(&v)) as f64))
            }
        }
    }

    fn eval_update(&mut self, op: UpdateOp, arg: &Expr, prefix: bool) -> Result<Value, Value> {
        let old = self.eval_expr(arg)?;
        let oldn = to_number(&old);
        let newn = if op == UpdateOp::Inc { oldn + 1.0 } else { oldn - 1.0 };
        let newv = Value::Number(newn);
        self.assign_to_expr(arg, newv.clone())?;
        Ok(if prefix { newv } else { Value::Number(oldn) })
    }

    fn eval_binary(&mut self, op: BinaryOp, l: &Expr, r: &Expr) -> Result<Value, Value> {
        let lv = self.eval_expr(l)?;
        let rv = self.eval_expr(r)?;
        self.binary_op(op, &lv, &rv)
    }

    pub fn binary_op(&mut self, op: BinaryOp, lv: &Value, rv: &Value) -> Result<Value, Value> {
        match op {
            BinaryOp::Add => {
                let lp = self.to_primitive(lv, "default")?;
                let rp = self.to_primitive(rv, "default")?;
                if matches!(lp, Value::String(_)) || matches!(rp, Value::String(_)) {
                    return Ok(Value::from_string(format!("{}{}", to_string(&lp), to_string(&rp))));
                }
                if matches!(lp, Value::BigInt(_)) && matches!(rp, Value::BigInt(_)) {
                    if let (Value::BigInt(a), Value::BigInt(b)) = (&lp, &rp) {
                        return Ok(Value::BigInt(bigint_add(a, b)));
                    }
                }
                Ok(Value::Number(to_number(&lp) + to_number(&rp)))
            }
            BinaryOp::Sub => {
                if let (Value::BigInt(a), Value::BigInt(b)) = (lv, rv) {
                    return Ok(Value::BigInt(bigint_sub(a, b)));
                }
                Ok(Value::Number(to_number(lv) - to_number(rv)))
            }
            BinaryOp::Mul => {
                if let (Value::BigInt(a), Value::BigInt(b)) = (lv, rv) {
                    return Ok(Value::BigInt(bigint_mul(a, b)));
                }
                Ok(Value::Number(to_number(lv) * to_number(rv)))
            }
            BinaryOp::Div => Ok(Value::Number(to_number(lv) / to_number(rv))),
            BinaryOp::Mod => {
                if let (Value::BigInt(a), Value::BigInt(b)) = (lv, rv) {
                    return Ok(Value::BigInt(bigint_rem(a, b)));
                }
                let a = to_number(lv);
                let b = to_number(rv);
                Ok(Value::Number(a % b))
            }
            BinaryOp::Exp => Ok(Value::Number(to_number(lv).powf(to_number(rv)))),
            BinaryOp::BitAnd => Ok(Value::Number((to_int32(lv) & to_int32(rv)) as f64)),
            BinaryOp::BitOr => Ok(Value::Number((to_int32(lv) | to_int32(rv)) as f64)),
            BinaryOp::BitXor => Ok(Value::Number((to_int32(lv) ^ to_int32(rv)) as f64)),
            BinaryOp::Shl => Ok(Value::Number(((to_int32(lv) << (to_uint32(rv) & 31)) as i32) as f64)),
            BinaryOp::Shr => Ok(Value::Number((to_int32(lv) >> (to_uint32(rv) & 31)) as f64)),
            BinaryOp::UShr => Ok(Value::Number((to_uint32(lv) >> (to_uint32(rv) & 31)) as f64)),
            BinaryOp::Eq => Ok(Value::Bool(loose_equals(lv, rv))),
            BinaryOp::NotEq => Ok(Value::Bool(!loose_equals(lv, rv))),
            BinaryOp::StrictEq => Ok(Value::Bool(strict_equals(lv, rv))),
            BinaryOp::StrictNotEq => Ok(Value::Bool(!strict_equals(lv, rv))),
            BinaryOp::Lt => self.cmp(lv, rv, |o| matches!(o, Some(std::cmp::Ordering::Less))),
            BinaryOp::Le => self.cmp(lv, rv, |o| !matches!(o, Some(std::cmp::Ordering::Greater))),
            BinaryOp::Gt => self.cmp(lv, rv, |o| matches!(o, Some(std::cmp::Ordering::Greater))),
            BinaryOp::Ge => self.cmp(lv, rv, |o| !matches!(o, Some(std::cmp::Ordering::Less))),
            BinaryOp::In => {
                let obj = self.to_object(rv)?;
                let key = to_string(lv);
                Ok(Value::Bool(self.has_property(&obj, &PropKey::from_str(&key))))
            }
            BinaryOp::InstanceOf => self.instance_of(lv, rv),
        }
    }

    fn cmp<F: Fn(Option<std::cmp::Ordering>) -> bool>(
        &mut self,
        l: &Value,
        r: &Value,
        f: F,
    ) -> Result<Value, Value> {
        let lp = self.to_primitive(l, "number")?;
        let rp = self.to_primitive(r, "number")?;
        if let (Value::String(a), Value::String(b)) = (&lp, &rp) {
            return Ok(Value::Bool(f(Some(a.cmp(b)))));
        }
        let a = to_number(&lp);
        let b = to_number(&rp);
        if a.is_nan() || b.is_nan() {
            return Ok(Value::Bool(false));
        }
        Ok(Value::Bool(f(a.partial_cmp(&b))))
    }

    fn eval_logical(&mut self, op: LogicalOp, l: &Expr, r: &Expr) -> Result<Value, Value> {
        let lv = self.eval_expr(l)?;
        match op {
            LogicalOp::And => {
                if to_boolean(&lv) {
                    self.eval_expr(r)
                } else {
                    Ok(lv)
                }
            }
            LogicalOp::Or => {
                if to_boolean(&lv) {
                    Ok(lv)
                } else {
                    self.eval_expr(r)
                }
            }
            LogicalOp::Nullish => {
                if lv.is_nullish() {
                    self.eval_expr(r)
                } else {
                    Ok(lv)
                }
            }
        }
    }

    fn eval_assignment(&mut self, op: AssignOp, left: &AssignTarget, right: &Expr) -> Result<Value, Value> {
        if op == AssignOp::Assign {
            let v = self.eval_expr(right)?;
            self.assign_target(left, v.clone())?;
            return Ok(v);
        }
        // compound assignment
        // Check logical assignment first (||=, &&=, ??=) — these short-circuit
        // and don't evaluate the RHS unless needed.
        let logical = match op {
            AssignOp::AndAssign => Some(LogicalOp::And),
            AssignOp::OrAssign => Some(LogicalOp::Or),
            AssignOp::NullishAssign => Some(LogicalOp::Nullish),
            _ => None,
        };
        let cur = self.eval_assign_target(left)?;
        if let Some(lop) = logical {
            // Short-circuit: only evaluate RHS if the condition holds.
            let should_assign = match lop {
                LogicalOp::And => to_boolean(&cur),
                LogicalOp::Or => !to_boolean(&cur),
                LogicalOp::Nullish => cur.is_nullish(),
            };
            let newv = if should_assign {
                let rhs = self.eval_expr(right)?;
                self.assign_target(left, rhs.clone())?;
                rhs
            } else {
                cur
            };
            return Ok(newv);
        }
        // Compound assignment (+=, -=, etc.)
        let rhs = self.eval_expr(right)?;
        let binop = match op {
            AssignOp::AddAssign => BinaryOp::Add,
            AssignOp::SubAssign => BinaryOp::Sub,
            AssignOp::MulAssign => BinaryOp::Mul,
            AssignOp::DivAssign => BinaryOp::Div,
            AssignOp::ModAssign => BinaryOp::Mod,
            AssignOp::ExpAssign => BinaryOp::Exp,
            AssignOp::BitAndAssign => BinaryOp::BitAnd,
            AssignOp::BitOrAssign => BinaryOp::BitOr,
            AssignOp::BitXorAssign => BinaryOp::BitXor,
            AssignOp::ShlAssign => BinaryOp::Shl,
            AssignOp::ShrAssign => BinaryOp::Shr,
            AssignOp::UShrAssign => BinaryOp::UShr,
            _ => unreachable!(),
        };
        let newv = self.binary_op(binop, &cur, &rhs)?;
        self.assign_target(left, newv.clone())?;
        Ok(newv)
    }

    fn eval_assign_target(&mut self, t: &AssignTarget) -> Result<Value, Value> {
        match t {
            AssignTarget::Ident(n) => self.scope.get(n),
            AssignTarget::Member { object, property } => {
                let (_, v) = self.eval_member(object, property, false)?;
                Ok(v)
            }
            AssignTarget::Pattern(_) => Err(error::throw_syntax("cannot read destructuring target")),
        }
    }

    fn assign_to_expr(&mut self, e: &Expr, v: Value) -> Result<(), Value> {
        match e {
            Expr::Ident(n) => self.put_variable(n, v),
            Expr::Member { object, property, .. } => {
                let (this, _) = self.eval_member(object, property, false)?;
                let key = match property {
                    MemberProp::Ident(n) => PropKey::Str(n.clone()),
                    MemberProp::Private(n) => PropKey::Str(n.clone()),
                    MemberProp::Computed(e) => {
                        let kv = self.eval_expr(e)?;
                        to_property_key(&kv)
                    }
                };
                self.set_property(&this, &key, v)
            }
            Expr::Paren(e) => self.assign_to_expr(e, v),
            _ => Err(error::throw_syntax("invalid assignment target")),
        }
    }

    fn assign_target(&mut self, t: &AssignTarget, v: Value) -> Result<(), Value> {
        match t {
            AssignTarget::Ident(n) => self.put_variable(n, v),
            AssignTarget::Member { object, property } => {
                let (this, _) = self.eval_member(object, property, false)?;
                let key = match property {
                    MemberProp::Ident(n) => PropKey::Str(n.clone()),
                    MemberProp::Private(n) => PropKey::Str(n.clone()),
                    MemberProp::Computed(e) => {
                        let kv = self.eval_expr(e)?;
                        to_property_key(&kv)
                    }
                };
                self.set_property(&this, &key, v)
            }
            AssignTarget::Pattern(p) => self.assign_pattern(&p, v),
        }
    }

    fn put_variable(&mut self, name: &Rc<str>, v: Value) -> Result<(), Value> {
        if self.scope.resolve(name).is_some() {
            self.scope.set(name, v)
        } else {
            // implicit global
            self.define_global(name, v);
            Ok(())
        }
    }

    fn define_global(&self, name: &Rc<str>, v: Value) {
        let g = &self.shared.realm.global;
        g.borrow_mut().props.insert(
            PropKey::Str(name.clone()),
            Property::data(v.clone()),
        );
        // also mirror into global env for fast lookup
        if !self.shared.realm.global_env.has_own(name) {
            self.shared.realm.global_env.create(name, Value::Undefined, true);
        }
        let _ = self.shared.realm.global_env.set(name, v);
    }

    // member access --------------------------------------------------

    fn eval_member(
        &mut self,
        object: &Expr,
        property: &MemberProp,
        optional: bool,
    ) -> Result<(Value, Value), Value> {
        let (this, obj) = self.eval_member_receiver(object)?;
        if optional && obj.is_nullish() {
            return Ok((Value::Undefined, Value::Undefined));
        }
        let key = match property {
            MemberProp::Ident(n) => PropKey::Str(n.clone()),
            MemberProp::Private(n) => PropKey::Str(n.clone()),
            MemberProp::Computed(e) => {
                let kv = self.eval_expr(e)?;
                to_property_key(&kv)
            }
        };
        let v = self.get_property(&obj, &key)?;
        Ok((this, v))
    }

    /// Evaluate the object expression and return (this, value).
    fn eval_member_receiver(&mut self, object: &Expr) -> Result<(Value, Value), Value> {
        match object {
            Expr::Super => {
                let home = self.scope.home_object().ok_or_else(|| {
                    error::throw_syntax("'super' keyword unexpected here")
                })?;
                let proto = if let Value::Object(o) = &home {
                    o.borrow().proto.clone().unwrap_or(Value::Undefined)
                } else {
                    Value::Undefined
                };
                Ok((home, proto))
            }
            _ => {
                let v = self.eval_expr(object)?;
                Ok((v.clone(), v))
            }
        }
    }

    fn eval_call(&mut self, callee: &Expr, args: &[CallArg], optional: bool) -> Result<Value, Value> {
        // super(...) constructor call
        if matches!(callee, Expr::Super) {
            let parent = self.scope.parent_constructor().ok_or_else(|| {
                error::throw_syntax("'super' call outside constructor")
            })?;
            let this = self.scope.this();
            let argv = self.eval_args(args)?;
            let nt = self.scope.new_target().unwrap_or(parent.clone());
            return self.construct_with_this(parent, this, &argv, nt);
        }
        // method call: need `this`
        let (this, func) = match callee {
            Expr::Member { object, property, optional: mopt } => {
                let (this, v) = self.eval_member(object, property, *mopt)?;
                // Short-circuit if the member access was optional (`?.`) and
                // the result is nullish — regardless of whether the call
                // itself is optional.
                if (*mopt || optional) && v.is_nullish() {
                    return Ok(Value::Undefined);
                }
                (this, v)
            }
            _ => {
                // super.method()
                if let Expr::Member { object, property, .. } = callee {
                    if matches!(**object, Expr::Super) {
                        let home = self.scope.home_object().ok_or_else(|| {
                            error::throw_syntax("'super' keyword unexpected here")
                        })?;
                        let proto = if let Value::Object(o) = &home {
                            o.borrow().proto.clone().unwrap_or(Value::Undefined)
                        } else {
                            Value::Undefined
                        };
                        let key = match property {
                            MemberProp::Ident(n) => PropKey::Str(n.clone()),
                            MemberProp::Private(n) => PropKey::Str(n.clone()),
                            MemberProp::Computed(e) => {
                                let kv = self.eval_expr(&e)?;
                                to_property_key(&kv)
                            }
                        };
                        let m = self.get_property(&proto, &key)?;
                        (home, m)
                    } else {
                        let f = self.eval_expr(callee)?;
                        (Value::Undefined, f)
                    }
                } else {
                    let f = self.eval_expr(callee)?;
                    (Value::Undefined, f)
                }
            }
        };
        if optional && func.is_nullish() {
            return Ok(Value::Undefined);
        }
        let argv = self.eval_args(args)?;
        // sloppy-mode `this` boxing
        let thisv = if this.is_nullish() {
            Value::Undefined
        } else {
            this
        };
        self.call_value(func, thisv, &argv)
    }

    fn eval_args(&mut self, args: &[CallArg]) -> Result<Vec<Value>, Value> {
        let mut out = Vec::new();
        for a in args {
            match a {
                CallArg::Expr(e) => out.push(self.eval_expr(e)?),
                CallArg::Spread(e) => {
                    let v = self.eval_expr(e)?;
                    self.flatten_into(&mut out, &v)?;
                }
            }
        }
        Ok(out)
    }

    fn eval_new(&mut self, callee: &Expr, args: &[CallArg]) -> Result<Value, Value> {
        let func = self.eval_expr(callee)?;
        let argv = self.eval_args(args)?;
        let nt = func.clone();
        self.construct(func, &argv, nt)
    }

    // -----------------------------------------------------------------
    // Property access
    // -----------------------------------------------------------------

    pub fn get_property(&mut self, obj: &Value, key: &PropKey) -> Result<Value, Value> {
        // Proxy "get" trap
        if let Value::Object(o) = obj {
            if let ObjectKind::Proxy(pd) = &o.borrow().kind {
                if pd.revoked {
                    return Err(error::throw_type("Cannot perform 'get' on a proxy that has been revoked"));
                }
                let handler_get = self.get_property(&pd.handler, &PropKey::from_str("get"))?;
                if handler_get.is_callable() {
                    let key_val = match key {
                        PropKey::Str(s) => Value::String(s.clone()),
                        PropKey::Sym(s) => Value::Symbol(s.clone()),
                    };
                    return self.call_value(handler_get, pd.handler.clone(), &[pd.target.clone(), key_val, obj.clone()]);
                }
                // default: forward to target
                return self.get_property(&pd.target, key);
            }
        }
        // Member access on primitives: autobox.
        match obj {
            Value::Undefined => {
                return Err(error::throw_type("Cannot read properties of undefined"));
            }
            Value::Null => {
                return Err(error::throw_type("Cannot read properties of null"));
            }
            Value::String(s) => {
                if let PropKey::Str(k) = key {
                    if &**k == "length" {
                        return Ok(Value::from_int(s.chars().count() as i32));
                    }
                    if let Some(idx) = key_to_index(k) {
                        if let Some(ch) = s.chars().nth(idx) {
                            return Ok(Value::from_string(ch.to_string()));
                        }
                        return Ok(Value::Undefined);
                    }
                }
                let wrapper = self.string_wrapper(s.clone());
                let v = self.get_property_own_chain(&Value::Object(wrapper), key)?;
                return Ok(v);
            }
            Value::Number(n) => {
                let wrapper = self.number_wrapper(*n);
                return self.get_property_own_chain(&Value::Object(wrapper), key);
            }
            Value::Bool(b) => {
                let wrapper = self.boolean_wrapper(*b);
                return self.get_property_own_chain(&Value::Object(wrapper), key);
            }
            Value::Symbol(_) | Value::BigInt(_) => {
                let wrapper = self.to_object(obj)?;
                return self.get_property_own_chain(&wrapper, key);
            }
            Value::Object(_) => {}
        }
        self.get_property_own_chain(obj, key)
    }

    /// Check if a value is a Proxy and extract its data.
    pub fn as_proxy(&self, v: &Value) -> Option<Rc<ProxyData>> {
        if let Value::Object(o) = v {
            if let ObjectKind::Proxy(pd) = &o.borrow().kind {
                return Some(pd.clone());
            }
        }
        None
    }

    fn get_property_own_chain(&mut self, obj: &Value, key: &PropKey) -> Result<Value, Value> {
        let mut cur = Some(obj.clone());
        while let Some(v) = cur {
            if let Value::Object(o) = &v {
                let kind_access;
                {
                    let b = o.borrow();
                    // fast array
                    if let ObjectKind::Array(items) = &b.kind {
                        if let PropKey::Str(k) = key {
                            if &**k == "length" {
                                return Ok(Value::from_int(items.len() as i32));
                            }
                            if let Some(idx) = key_to_index(k) {
                                if idx < items.len() {
                                    return Ok(items[idx].clone());
                                }
                                // fall through to proto
                            }
                        }
                    }
                    if let Some(p) = b.props.get(key) {
                        match &p.kind {
                            PropKind::Data(v) => return Ok(v.clone()),
                            PropKind::Accessor { get, .. } => {
                                if let Some(g) = get {
                                    kind_access = Some(g.clone());
                                } else {
                                    return Ok(Value::Undefined);
                                }
                            }
                        }
                    } else {
                        kind_access = None;
                    }
                    if let Some(g) = kind_access {
                        drop(b);
                        let this = Value::Object(o.clone());
                        return self.call_value(g, this, &[]);
                    }
                    cur = b.proto.clone();
                }
            } else {
                break;
            }
        }
        Ok(Value::Undefined)
    }

    pub fn set_property(&mut self, obj: &Value, key: &PropKey, value: Value) -> Result<(), Value> {
        match obj {
            Value::Object(o) => {
                // Proxy "set" trap
                let is_proxy;
                {
                    let b = o.borrow();
                    is_proxy = matches!(b.kind, ObjectKind::Proxy(_));
                }
                if is_proxy {
                    let pd = if let ObjectKind::Proxy(pd) = &o.borrow().kind { pd.clone() } else { unreachable!() };
                    if pd.revoked {
                        return Err(error::throw_type("Cannot perform 'set' on a proxy that has been revoked"));
                    }
                    let handler_set = self.get_property(&pd.handler, &PropKey::from_str("set"))?;
                    if handler_set.is_callable() {
                        let key_val = match key {
                            PropKey::Str(s) => Value::String(s.clone()),
                            PropKey::Sym(s) => Value::Symbol(s.clone()),
                        };
                        let _ = self.call_value(handler_set, pd.handler.clone(), &[pd.target.clone(), key_val, value, obj.clone()])?;
                        return Ok(());
                    }
                    // default: forward to target
                    return self.set_property(&pd.target, key, value);
                }
                // array length / index fast path
                let is_array;
                {
                    let b = o.borrow();
                    is_array = matches!(b.kind, ObjectKind::Array(_));
                }
                if is_array {
                    if let PropKey::Str(k) = key {
                        if &**k == "length" {
                            let n = to_length(&value);
                            if let ObjectKind::Array(items) = &mut o.borrow_mut().kind {
                                items.resize(n, Value::Undefined);
                            }
                            return Ok(());
                        }
                        if let Some(idx) = key_to_index(k) {
                            if let ObjectKind::Array(items) = &mut o.borrow_mut().kind {
                                if idx >= items.len() {
                                    items.resize(idx + 1, Value::Undefined);
                                }
                                items[idx] = value;
                                return Ok(());
                            }
                        }
                    }
                }
                // setter walk
                let setter = {
                    let mut cur = Some(Value::Object(o.clone()));
                    let mut found = None;
                    while let Some(Value::Object(c)) = cur {
                        let b = c.borrow();
                        if let Some(p) = b.props.get(key) {
                            if let PropKind::Accessor { set, .. } = &p.kind {
                                found = set.clone();
                                break;
                            } else {
                                found = None;
                                break;
                            }
                        }
                        cur = b.proto.clone();
                    }
                    found
                };
                if let Some(s) = setter {
                    let this = Value::Object(o.clone());
                    self.call_value(s, this, &[value])?;
                    return Ok(());
                }
                if !o.borrow().extensible {
                    // silently ignore in non-strict
                    return Ok(());
                }
                o.borrow_mut().props.insert(key.clone(), Property::data(value));
                Ok(())
            }
            _ => {
                // setting on primitive — ignored
                Ok(())
            }
        }
    }

    pub fn has_property(&mut self, obj: &Value, key: &PropKey) -> bool {
        // Proxy "has" trap
        if let Value::Object(o) = obj {
            let is_proxy = matches!(o.borrow().kind, ObjectKind::Proxy(_));
            if is_proxy {
                let pd = if let ObjectKind::Proxy(pd) = &o.borrow().kind { pd.clone() } else { unreachable!() };
                if pd.revoked { return false; }
                let handler_has = self.get_property(&pd.handler, &PropKey::from_str("has")).unwrap_or(Value::Undefined);
                if handler_has.is_callable() {
                    let key_val = match key {
                        PropKey::Str(s) => Value::String(s.clone()),
                        PropKey::Sym(s) => Value::Symbol(s.clone()),
                    };
                    if let Ok(r) = self.call_value(handler_has, pd.handler.clone(), &[pd.target.clone(), key_val]) {
                        return to_boolean(&r);
                    }
                    return false;
                }
                return self.has_property(&pd.target, key);
            }
        }
        let mut cur = Some(obj.clone());
        while let Some(v) = cur {
            if let Value::Object(o) = &v {
                let b = o.borrow();
                if let ObjectKind::Array(items) = &b.kind {
                    if let PropKey::Str(k) = key {
                        if &**k == "length" {
                            return true;
                        }
                        if let Some(idx) = key_to_index(k) {
                            if idx < items.len() {
                                return true;
                            }
                        }
                    }
                }
                if b.props.contains_key(key) {
                    return true;
                }
                cur = b.proto.clone();
            } else if let Value::String(s) = &v {
                if let PropKey::Str(k) = key {
                    if &**k == "length" {
                        return true;
                    }
                    if let Some(idx) = key_to_index(k) {
                        if idx < s.chars().count() {
                            return true;
                        }
                    }
                }
                cur = None;
            } else {
                break;
            }
        }
        false
    }

    /// Delete a property from an object (non-Proxy path). Returns true if removed.
    pub fn delete_property(&mut self, obj: &Value, key: &PropKey) -> Result<bool, Value> {
        if let Value::Object(o) = obj {
            // Proxy "deleteProperty" trap
            if matches!(o.borrow().kind, ObjectKind::Proxy(_)) {
                let pd = if let ObjectKind::Proxy(pd) = &o.borrow().kind { pd.clone() } else { unreachable!() };
                if pd.revoked {
                    return Err(error::throw_type("Cannot perform 'deleteProperty' on a proxy that has been revoked"));
                }
                let trap = self.get_property(&pd.handler, &PropKey::from_str("deleteProperty"))?;
                if trap.is_callable() {
                    let key_val = match key {
                        PropKey::Str(s) => Value::String(s.clone()),
                        PropKey::Sym(s) => Value::Symbol(s.clone()),
                    };
                    let r = self.call_value(trap, pd.handler.clone(), &[pd.target.clone(), key_val])?;
                    return Ok(to_boolean(&r));
                }
                return self.delete_property(&pd.target, key);
            }
            let mut b = o.borrow_mut();
            if let ObjectKind::Array(items) = &mut b.kind {
                if let PropKey::Str(k) = key {
                    if let Some(idx) = key_to_index(k) {
                        if idx < items.len() {
                            items[idx] = Value::Undefined;
                            return Ok(true);
                        }
                    }
                }
            }
            Ok(b.props.shift_remove(key).is_some())
        } else {
            Ok(false)
        }
    }

    /// Get the own property keys of an object. Honors the Proxy "ownKeys" trap.
    pub fn own_property_keys(&mut self, obj: &Value) -> Result<Vec<PropKey>, Value> {
        if let Value::Object(o) = obj {
            // Proxy "ownKeys" trap
            if matches!(o.borrow().kind, ObjectKind::Proxy(_)) {
                let pd = if let ObjectKind::Proxy(pd) = &o.borrow().kind { pd.clone() } else { unreachable!() };
                if pd.revoked {
                    return Err(error::throw_type("Cannot perform 'ownKeys' on a proxy that has been revoked"));
                }
                let trap = self.get_property(&pd.handler, &PropKey::from_str("ownKeys"))?;
                if trap.is_callable() {
                    let r = self.call_value(trap, pd.handler.clone(), &[pd.target.clone()])?;
                    let items = self.iterable_to_vec(&r)?;
                    let mut keys = Vec::new();
                    for it in items {
                        keys.push(to_property_key(&it));
                    }
                    return Ok(keys);
                }
                return self.own_property_keys(&pd.target);
            }
            let b = o.borrow();
            let mut keys = Vec::new();
            if let ObjectKind::Array(items) = &b.kind {
                for i in 0..items.len() {
                    keys.push(PropKey::Str(index_to_key(i)));
                }
            }
            for (k, _) in b.props.iter() {
                keys.push(k.clone());
            }
            Ok(keys)
        } else {
            Ok(Vec::new())
        }
    }

    /// Get the prototype of an object. Honors the Proxy "getPrototypeOf" trap.
    pub fn get_prototype_of(&mut self, obj: &Value) -> Result<Value, Value> {
        if let Value::Object(o) = obj {
            if matches!(o.borrow().kind, ObjectKind::Proxy(_)) {
                let pd = if let ObjectKind::Proxy(pd) = &o.borrow().kind { pd.clone() } else { unreachable!() };
                if pd.revoked {
                    return Err(error::throw_type("Cannot perform 'getPrototypeOf' on a proxy that has been revoked"));
                }
                let trap = self.get_property(&pd.handler, &PropKey::from_str("getPrototypeOf"))?;
                if trap.is_callable() {
                    return self.call_value(trap, pd.handler.clone(), &[pd.target.clone()]);
                }
                return self.get_prototype_of(&pd.target);
            }
            return Ok(o.borrow().proto.clone().unwrap_or(Value::Null));
        }
        Ok(Value::Null)
    }

    // -----------------------------------------------------------------
    // Coercions
    // -----------------------------------------------------------------

    pub fn to_object(&mut self, v: &Value) -> Result<Value, Value> {
        match v {
            Value::Undefined | Value::Null => {
                Err(error::throw_type("cannot convert undefined or null to object"))
            }
            Value::Bool(b) => Ok(Value::Object(self.boolean_wrapper(*b))),
            Value::Number(n) => Ok(Value::Object(self.number_wrapper(*n))),
            Value::String(s) => Ok(Value::Object(self.string_wrapper(s.clone()))),
            Value::Symbol(s) => {
                let o = ObjectInner::new_object();
                o.borrow_mut().proto = Some(Value::Object(self.realm().symbol_proto.clone()));
                o.borrow_mut().class = "Symbol";
                o.borrow_mut().kind = ObjectKind::Symbol(s.clone());
                o.borrow_mut().props.insert(
                    PropKey::from_str("description"),
                    Property {
                        kind: PropKind::Data(match s.description.clone() {
                            Some(d) => Value::String(d),
                            None => Value::Undefined,
                        }),
                        writable: false,
                        enumerable: false,
                        configurable: false,
                    },
                );
                Ok(Value::Object(o))
            }
            Value::BigInt(b) => {
                let o = ObjectInner::new_object();
                o.borrow_mut().proto = Some(Value::Object(self.realm().bigint_proto.clone()));
                o.borrow_mut().class = "BigInt";
                o.borrow_mut().kind = ObjectKind::Ordinary;
                o.borrow_mut().props.insert(
                    PropKey::from_str("[[BigIntData]]"),
                    Property::data(Value::BigInt(b.clone())),
                );
                Ok(Value::Object(o))
            }
            o @ Value::Object(_) => Ok(o.clone()),
        }
    }

    pub fn to_primitive(&mut self, v: &Value, hint: &str) -> Result<Value, Value> {
        match v {
            Value::Object(o) => {
                // Symbol.toPrimitive
                let tp = {
                    let b = o.borrow();
                    b.props.get(&PropKey::Sym(self.realm().wk.to_primitive.clone())).cloned()
                };
                if let Some(tp) = tp {
                    if let PropKind::Data(m) = tp.kind {
                        if m.is_callable() {
                            let r = self.call_value(m, v.clone(), &[Value::from_str(hint)])?;
                            if !r.is_object() {
                                return Ok(r);
                            }
                        }
                    }
                }
                // ordinaryToPrimitive
                self.ordinary_to_primitive(v, hint)
            }
            _ => Ok(v.clone()),
        }
    }

    fn ordinary_to_primitive(&mut self, v: &Value, hint: &str) -> Result<Value, Value> {
        let methods: [&str; 2] = if hint == "string" {
            ["toString", "valueOf"]
        } else {
            ["valueOf", "toString"]
        };
        for m in methods {
            let mv = self.get_property(v, &PropKey::from_str(m))?;
            if mv.is_callable() {
                let r = self.call_value(mv, v.clone(), &[])?;
                if !r.is_object() {
                    return Ok(r);
                }
            }
        }
        Err(error::throw_type("cannot convert object to primitive value"))
    }

    // -----------------------------------------------------------------
    // Function call / construct
    // -----------------------------------------------------------------

    pub fn call_value(&mut self, func: Value, this: Value, args: &[Value]) -> Result<Value, Value> {
        // Proxy "apply" trap
        if let Value::Object(o) = &func {
            if matches!(o.borrow().kind, ObjectKind::Proxy(_)) {
                let pd = if let ObjectKind::Proxy(pd) = &o.borrow().kind { pd.clone() } else { unreachable!() };
                if pd.revoked {
                    return Err(error::throw_type("Cannot perform 'apply' on a proxy that has been revoked"));
                }
                // target must be callable
                if !pd.target.is_callable() {
                    return Err(error::throw_type("proxy target is not a function"));
                }
                let trap = self.get_property(&pd.handler, &PropKey::from_str("apply"))?;
                if trap.is_callable() {
                    let args_arr = self.new_array(args.to_vec());
                    return self.call_value(trap, pd.handler.clone(), &[pd.target.clone(), this, args_arr]);
                }
                // default: forward to target
                return self.call_value(pd.target.clone(), this, args);
            }
        }
        match &func {
            Value::Object(o) => {
                let kind = {
                    let b = o.borrow();
                    b.kind.clone_for_call()
                };
                match kind {
                    CallKind::Native(f) => f(self, this, args),
                    CallKind::Js(f) => self.call_js_function(&f, this, args, None),
                    CallKind::Bound { target, this_arg, bound_args } => {
                        let mut all = bound_args;
                        all.extend_from_slice(args);
                        let t = if this.is_undefined() { this_arg } else { this };
                        self.call_value(target, t, &all)
                    }
                    CallKind::Other => Err(error::throw_type("value is not a function")),
                }
            }
            _ => Err(error::throw_type(&format!("{} is not a function", to_string(&func)))),
        }
    }

    pub fn construct(&mut self, func: Value, args: &[Value], new_target: Value) -> Result<Value, Value> {
        // Proxy "construct" trap
        if let Value::Object(o) = &func {
            if matches!(o.borrow().kind, ObjectKind::Proxy(_)) {
                let pd = if let ObjectKind::Proxy(pd) = &o.borrow().kind { pd.clone() } else { unreachable!() };
                if pd.revoked {
                    return Err(error::throw_type("Cannot perform 'construct' on a proxy that has been revoked"));
                }
                if !pd.target.is_constructor() {
                    return Err(error::throw_type("proxy target is not a constructor"));
                }
                let trap = self.get_property(&pd.handler, &PropKey::from_str("construct"))?;
                if trap.is_callable() {
                    let args_arr = self.new_array(args.to_vec());
                    return self.call_value(trap, pd.handler.clone(), &[pd.target.clone(), args_arr, new_target]);
                }
                // default: forward to target
                return self.construct(pd.target.clone(), args, new_target);
            }
        }
        match &func {
            Value::Object(o) => {
                let kind = {
                    let b = o.borrow();
                    b.kind.clone_for_call()
                };
                match kind {
                    CallKind::Native(f) => {
                        let ctor = {
                            let b = o.borrow();
                            if let ObjectKind::Function(f) = &b.kind {
                                if let FunctionBody::Native { constructor, .. } = &f.body {
                                    constructor.clone()
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        };
                        if let Some(c) = ctor {
                            let this = self.new_object_for(new_target.clone())?;
                            c(self, this.clone(), args, new_target)
                        } else {
                            // native without explicit ctor: call as function with this=undefined
                            let this = self.new_object_for(new_target.clone())?;
                            f(self, this.clone(), args)?;
                            Ok(this)
                        }
                    }
                    CallKind::Js(f) => {
                        if !f.is_constructor {
                            return Err(error::throw_type("not a constructor"));
                        }
                        let proto = self.get_property(&func, &PropKey::from_str("prototype"))?;
                        let this = if let Value::Object(p) = &proto {
                            let obj = ObjectInner::new_object();
                            obj.borrow_mut().proto = Some(Value::Object(p.clone()));
                            Value::Object(obj)
                        } else {
                            self.new_object_for(new_target.clone())?
                        };
                        let result = self.call_js_function(&f, this.clone(), args, Some(new_target))?;
                        if result.is_object() {
                            Ok(result)
                        } else {
                            Ok(this)
                        }
                    }
                    CallKind::Bound { target, bound_args, .. } => {
                        let mut all = bound_args;
                        all.extend_from_slice(args);
                        self.construct(target, &all, new_target)
                    }
                    CallKind::Other => Err(error::throw_type("not a constructor")),
                }
            }
            _ => Err(error::throw_type("not a constructor")),
        }
    }

    /// Invoke a constructor with a pre-existing `this` (used by `super(...)`).
    pub fn construct_with_this(
        &mut self,
        func: Value,
        this: Value,
        args: &[Value],
        new_target: Value,
    ) -> Result<Value, Value> {
        match &func {
            Value::Object(o) => {
                let kind = { o.borrow().kind.clone_for_call() };
                match kind {
                    CallKind::Native(f) => {
                        let ctor = {
                            let b = o.borrow();
                            if let ObjectKind::Function(f) = &b.kind {
                                if let FunctionBody::Native { constructor, .. } = &f.body {
                                    constructor.clone()
                                } else { None }
                            } else { None }
                        };
                        if let Some(c) = ctor {
                            c(self, this.clone(), args, new_target)?;
                            Ok(this)
                        } else {
                            f(self, this.clone(), args)?;
                            Ok(this)
                        }
                    }
                    CallKind::Js(f) => {
                        let r = self.call_js_function(&f, this.clone(), args, Some(new_target))?;
                        Ok(if r.is_object() { r } else { this })
                    }
                    CallKind::Bound { target, bound_args, .. } => {
                        let mut all = bound_args;
                        all.extend_from_slice(args);
                        self.construct_with_this(target, this, &all, new_target)
                    }
                    CallKind::Other => Err(error::throw_type("not a constructor")),
                }
            }
            _ => Err(error::throw_type("not a constructor")),
        }
    }

    fn new_object_for(&self, new_target: Value) -> Result<Value, Value> {
        let proto = if let Value::Object(o) = &new_target {
            let p = o.borrow().props.get(&PropKey::from_str("prototype")).cloned();
            match p {
                Some(p) => match p.kind {
                    PropKind::Data(v) if v.is_object() => v,
                    _ => Value::Object(self.realm().object_proto.clone()),
                },
                None => Value::Object(self.realm().object_proto.clone()),
            }
        } else {
            Value::Object(self.realm().object_proto.clone())
        };
        let obj = ObjectInner::new_object();
        obj.borrow_mut().proto = Some(proto);
        Ok(Value::Object(obj))
    }

    fn call_js_function(
        &mut self,
        f: &Rc<Function>,
        this: Value,
        args: &[Value],
        new_target: Option<Value>,
    ) -> Result<Value, Value> {
        if f.is_async {
            return self.start_async_function(f, this, args, new_target);
        }
        if f.is_generator {
            return self.start_generator(f, this, args);
        }
        let env = Env::new(Some(f.closure.clone()), EnvKind::Function);
        env.0.borrow_mut().this_val = this;
        let has_new_target = new_target.is_some();
        env.0.borrow_mut().new_target = new_target;
        env.0.borrow_mut().home_object = f.home_object.clone();
        env.0.borrow_mut().parent_constructor = f.parent_class.clone();
        // bind parameters
        self.bind_params(&f, args, &env)?;
        // hoist body
        if let FunctionBody::Js { body, decls, .. } = &f.body {
            for d in decls {
                if let Some(name) = &d.name {
                    let nf = self.make_function(&d.func, d.is_async, d.is_generator, env.clone());
                    env.create(name, nf, true);
                }
            }
            self.hoist(&body.stmts, &env, false)?;
        }
        let saved = self.scope.clone();
        self.scope = env;
        // Push a stack frame for Error.stack traces.
        let frame_name = if f.name.is_empty() {
            "<anonymous>".to_string()
        } else {
            f.name.to_string()
        };
        let frame = if f.line > 0 {
            format!("{} (line {})", frame_name, f.line)
        } else {
            frame_name
        };
        self.shared.stack.borrow_mut().push(frame);
        let result = (|| {
            if has_new_target {
                match &f.parent_class {
                    None => {
                        // Base class: initialize instance fields.
                        if !f.class_fields.is_empty() {
                            let this = self.scope.this();
                            for field in &f.class_fields {
                                let v = match &field.init {
                                    Some(e) => self.eval_expr(e)?,
                                    None => Value::Undefined,
                                };
                                if let Pattern::Ident(n) = &field.name {
                                    self.set_property(&this, &PropKey::Str(n.clone()), v)?;
                                }
                            }
                        }
                    }
                    Some(parent) => {
                        // Derived class: if the constructor body is empty
                        // (synthesized default), invoke super(...args) then
                        // initialize instance fields.
                        let empty = matches!(&f.body, FunctionBody::Js { body, .. } if body.stmts.is_empty());
                        if empty {
                            let this = self.scope.this();
                            let nt = self.scope.new_target().unwrap_or(parent.clone());
                            self.construct_with_this(parent.clone(), this, args, nt)?;
                            if !f.class_fields.is_empty() {
                                let this = self.scope.this();
                                for field in &f.class_fields {
                                    let v = match &field.init {
                                        Some(e) => self.eval_expr(e)?,
                                        None => Value::Undefined,
                                    };
                                    if let Pattern::Ident(n) = &field.name {
                                        self.set_property(&this, &PropKey::Str(n.clone()), v)?;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let FunctionBody::Js { body, .. } = &f.body {
                for s in &body.stmts {
                    match self.exec_stmt(s)? {
                        Completion::Normal(_) => {}
                        Completion::Return(v) => return Ok(v),
                        _ => {}
                    }
                }
            }
            Ok(Value::Undefined)
        })();
        self.shared.stack.borrow_mut().pop();
        self.scope = saved;
        result
    }

    fn bind_params(&mut self, f: &Rc<Function>, args: &[Value], env: &Env) -> Result<(), Value> {
        if let FunctionBody::Js { params, .. } = &f.body {
            let mut i = 0;
            for p in params {
                match p {
                    Pattern::Rest(inner) => {
                        let rest: Vec<Value> = args.iter().skip(i).cloned().collect();
                        let arr = self.new_array(rest);
                        self.bind_pattern(inner, arr, env, VarKind::Let, true)?;
                        break;
                    }
                    _ => {
                        let v = args.get(i).cloned().unwrap_or(Value::Undefined);
                        self.bind_pattern(p, v, env, VarKind::Let, true)?;
                        i += 1;
                    }
                }
            }
            // arguments object (non-arrow)
            if !f.is_arrow {
                let mut items = Vec::new();
                for a in args {
                    items.push(a.clone());
                }
                let arr = self.new_array(items);
                env.0.borrow_mut().bindings.insert(
                    Rc::from("arguments"),
                    crate::scope::Binding {
                        value: arr,
                        mutable: true,
                        initialized: true,
                    },
                );
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    // make_function / make_class
    // -----------------------------------------------------------------

    pub fn make_function(&self, fe: &FunctionExpr, is_async: bool, is_generator: bool, closure: Env) -> Value {
        let length = fe.params.iter().take_while(|p| !matches!(p, Pattern::Rest(_) | Pattern::Assignment { .. })).count();
        let f = Rc::new(Function {
            body: FunctionBody::Js {
                params: fe.params.clone(),
                body: fe.body.clone(),
                decls: fe.decls.clone(),
                strict: false,
            },
            name: fe.name.clone().unwrap_or(Rc::from("")),
            length,
            closure,
            is_arrow: fe.is_arrow,
            is_async,
            is_generator,
            is_method: false,
            is_constructor: !fe.is_arrow && !is_async && !is_generator,
            home_object: None,
            class_fields: Vec::new(),
            parent_class: None,
            line: fe.line,
        });
        let obj = ObjectInner::new_function(f);
        obj.borrow_mut().proto = Some(Value::Object(self.realm().function_proto.clone()));
        // prototype object for non-arrow non-method constructors
        if !fe.is_arrow && !is_async && !is_generator {
            let proto = ObjectInner::new_object();
            proto.borrow_mut().proto = Some(Value::Object(self.realm().object_proto.clone()));
            let proto_val = Value::Object(proto);
            obj.borrow_mut().props.insert(
                PropKey::from_str("prototype"),
                Property {
                    kind: PropKind::Data(proto_val),
                    writable: true,
                    enumerable: false,
                    configurable: false,
                },
            );
        }
        obj.borrow_mut().props.insert(
            PropKey::from_str("length"),
            Property {
                kind: PropKind::Data(Value::from_int(length as i32)),
                writable: false,
                enumerable: false,
                configurable: true,
            },
        );
        obj.borrow_mut().props.insert(
            PropKey::from_str("name"),
            Property {
                kind: PropKind::Data(Value::from_string(fe.name.as_deref().unwrap_or("").to_string())),
                writable: false,
                enumerable: false,
                configurable: true,
            },
        );
        Value::Object(obj)
    }

    pub fn make_native(&self, name: &str, length: usize, func: NativeFn) -> Value {
        make_native_value(self.realm(), name, length, func)
    }

    fn make_class(&mut self, c: &ClassDecl, closure: &Env) -> Result<Value, Value> {
        let parent = match &c.superclass {
            Some(e) => Some(self.eval_expr(e)?),
            None => None,
        };
        let mut ctor_func: Option<Rc<Function>> = None;
        let mut methods: Vec<(PropKey, Property, bool)> = Vec::new(); // (key, prop, is_static)
        let mut instance_fields: Vec<ClassField> = Vec::new();
        let mut static_fields: Vec<(PropKey, Option<Expr>, bool)> = Vec::new();
        let mut static_proto_props: Vec<(PropKey, Property)> = Vec::new();

        for m in &c.body {
            let key = self.eval_property_key(&m.key, m.computed)?;
            match &m.kind {
                ClassMemberKind::Method { func, kind } => {
                    let f = self.make_function(func, func.is_async, func.is_generator, closure.clone());
                    if let Value::Object(fo) = &f {
                        let new_kind = {
                            let b = fo.borrow();
                            if let ObjectKind::Function(rf) = &b.kind {
                                Some(ObjectKind::Function(Rc::new(Function {
                                    body: rf.body.clone_body(),
                                    name: rf.name.clone(),
                                    length: rf.length,
                                    closure: rf.closure.clone(),
                                    is_arrow: rf.is_arrow,
                                    is_generator: rf.is_generator,
                                    is_async: rf.is_async,
                                    is_method: true,
                                    is_constructor: matches!(kind, MethodKind::Constructor),
                                    home_object: None,
                                    class_fields: Vec::new(),
                                    parent_class: parent.clone(),
            line: 0,
                                })))
                            } else { None }
                        };
                        if let Some(nk) = new_kind {
                            fo.borrow_mut().kind = nk;
                        }
                    }
                    match kind {
                        MethodKind::Constructor => {
                            if let Value::Object(fo) = &f {
                                if let ObjectKind::Function(rf) = &fo.borrow().kind {
                                    let mut cf = (**rf).clone_struct();
                                    cf.is_constructor = true;
                                    cf.parent_class = parent.clone();
                                    // collect instance fields
                                    for mm in &c.body {
                                        if let ClassMemberKind::Field { init } = &mm.kind {
                                            if !mm.is_static {
                                                instance_fields.push(ClassField {
                                                    name: pattern_from_key(&mm.key, mm.computed),
                                                    init: init.clone(),
                                                });
                                            }
                                        }
                                    }
                                    cf.class_fields = instance_fields.clone();
                                    ctor_func = Some(Rc::new(cf));
                                }
                            }
                        }
                        MethodKind::Get => {
                            methods.push((key.clone(), Property {
                                kind: PropKind::Accessor { get: Some(f), set: None },
                                writable: false,
                                enumerable: false,
                                configurable: true,
                            }, m.is_static));
                        }
                        MethodKind::Set => {
                            methods.push((key.clone(), Property {
                                kind: PropKind::Accessor { get: None, set: Some(f) },
                                writable: false,
                                enumerable: false,
                                configurable: true,
                            }, m.is_static));
                        }
                        MethodKind::Normal => {
                            methods.push((key.clone(), Property::data(f), m.is_static));
                        }
                    }
                }
                ClassMemberKind::Field { init } => {
                    if m.is_static {
                        static_fields.push((key.clone(), init.clone(), m.computed));
                    } else {
                        instance_fields.push(ClassField {
                            name: pattern_from_key(&m.key, m.computed),
                            init: init.clone(),
                        });
                    }
                }
            }
        }
        // If no constructor, synthesize one.
        let ctor_func = ctor_func.unwrap_or_else(|| {
            let body = Block { stmts: vec![] };
            let params = Vec::new();
            Rc::new(Function {
                body: FunctionBody::Js {
                    params,
                    body,
                    decls: Vec::new(),
                    strict: false,
                },
                name: c.name.clone().unwrap_or(Rc::from("")),
                length: 0,
                closure: closure.clone(),
                is_arrow: false,
                is_async: false,
                is_generator: false,
                is_method: false,
                is_constructor: true,
                home_object: None,
                class_fields: instance_fields.clone(),
                parent_class: parent.clone(),
            line: 0,
            })
        });
        // build prototype object
        let proto = ObjectInner::new_object();
        proto.borrow_mut().proto = Some(match &parent {
            Some(p) => {
                let pp = self.get_property(p, &PropKey::from_str("prototype"))?;
                if matches!(pp, Value::Object(_)) { pp } else { Value::Object(self.realm().object_proto.clone()) }
            }
            None => Value::Object(self.realm().object_proto.clone()),
        });
        // install instance methods on proto with home_object = proto
        for (k, p, is_static) in &methods {
            if !is_static {
                if let PropKind::Data(v) = &p.kind {
                    if let Value::Object(fo) = v {
                        set_home_object(fo, Value::Object(proto.clone()));
                    }
                }
                if let PropKind::Accessor { get, set } = &p.kind {
                    if let Some(Value::Object(fo)) = get { set_home_object(fo, Value::Object(proto.clone())); }
                    if let Some(Value::Object(fo)) = set { set_home_object(fo, Value::Object(proto.clone())); }
                }
                proto.borrow_mut().props.insert(k.clone(), p.clone());
            } else {
                static_proto_props.push((k.clone(), p.clone()));
            }
        }
        // build constructor function object
        let ctor_obj = ObjectInner::new_function(ctor_func);
        ctor_obj.borrow_mut().proto = Some(match &parent {
            Some(p) => p.clone(),
            None => Value::Object(self.realm().function_proto.clone()),
        });
        ctor_obj.borrow_mut().props.insert(
            PropKey::from_str("prototype"),
            Property {
                kind: PropKind::Data(Value::Object(proto.clone())),
                writable: false,
                enumerable: false,
                configurable: false,
            },
        );
        ctor_obj.borrow_mut().props.insert(
            PropKey::from_str("length"),
            Property {
                kind: PropKind::Data(Value::from_int(0)),
                writable: false,
                enumerable: false,
                configurable: true,
            },
        );
        let name_val = c.name.clone().unwrap_or(Rc::from(""));
        ctor_obj.borrow_mut().props.insert(
            PropKey::from_str("name"),
            Property {
                kind: PropKind::Data(Value::from_string(name_val.to_string())),
                writable: false,
                enumerable: false,
                configurable: true,
            },
        );
        set_home_object(&ctor_obj, Value::Object(ctor_obj.clone()));
        // static methods/fields on ctor
        for (k, p) in static_proto_props {
            if let PropKind::Data(v) = &p.kind {
                if let Value::Object(fo) = v { set_home_object(fo, Value::Object(ctor_obj.clone())); }
            }
            ctor_obj.borrow_mut().props.insert(k, p);
        }
        for (k, init, _computed) in static_fields {
            let v = match init {
                Some(e) => self.eval_expr(&e)?,
                None => Value::Undefined,
            };
            ctor_obj.borrow_mut().props.insert(k, Property::data(v));
        }
        // class name binding inside
        if let Some(name) = &c.name {
            // class is a binding visible inside its own methods (via closure)
            closure.create(name, Value::Object(ctor_obj.clone()), true);
        }
        Ok(Value::Object(ctor_obj))
    }

    // -----------------------------------------------------------------
    // Destructuring binding
    // -----------------------------------------------------------------

    pub fn bind_pattern(
        &mut self,
        pat: &Pattern,
        value: Value,
        env: &Env,
        _kind: VarKind,
        mutable: bool,
    ) -> Result<(), Value> {
        match pat {
            Pattern::Ident(n) => {
                if env.has_own(n) {
                    env.0.borrow_mut().bindings.get_mut(n).map(|b| {
                        b.value = value.clone();
                        b.initialized = true;
                        b.mutable = mutable;
                    });
                } else {
                    env.create(n, value, mutable);
                }
                Ok(())
            }
            Pattern::Array { elements, rest } => {
                let iter = self.get_iterator(&value)?;
                for el in elements {
                    let v = self.iterator_step(&iter)?.unwrap_or(Value::Undefined);
                    if let Some(pe) = el {
                        self.bind_pattern(&pe.pattern, v, env, _kind, mutable)?;
                    }
                }
                if let Some(r) = rest {
                    let mut rest_items = Vec::new();
                    loop {
                        match self.iterator_step(&iter)? {
                            Some(v) => rest_items.push(v),
                            None => break,
                        }
                    }
                    let arr = self.new_array(rest_items);
                    self.bind_pattern(r, arr, env, _kind, mutable)?;
                }
                Ok(())
            }
            Pattern::Object { properties, rest } => {
                let mut used: std::collections::HashSet<PropKey> = std::collections::HashSet::new();
                for p in properties {
                    let key = self.eval_property_key(&p.key, p.computed)?;
                    used.insert(key.clone());
                    let v = self.get_property(&value, &key)?;
                    self.bind_pattern(&p.value, v, env, _kind, mutable)?;
                }
                if let Some(r) = rest {
                    let rest_obj = ObjectInner::new_object();
                    rest_obj.borrow_mut().proto = Some(Value::Object(self.realm().object_proto.clone()));
                    let keys = self.enumerate_for_in(&value)?;
                    for k in keys {
                        let pk = PropKey::Str(k);
                        if used.contains(&pk) {
                            continue;
                        }
                        let v = self.get_property(&value, &pk)?;
                        rest_obj.borrow_mut().props.insert(pk, Property::data(v));
                    }
                    env.create(r, Value::Object(rest_obj), true);
                }
                Ok(())
            }
            Pattern::Rest(inner) => self.bind_pattern(inner, value, env, _kind, mutable),
            Pattern::Assignment { pattern, default } => {
                if value.is_undefined() {
                    let dv = self.eval_expr(default)?;
                    self.bind_pattern(pattern, dv, env, _kind, mutable)
                } else {
                    self.bind_pattern(pattern, value, env, _kind, mutable)
                }
            }
            Pattern::ArrayHole => Ok(()),
        }
    }

    fn assign_pattern(&mut self, pat: &Pattern, value: Value) -> Result<(), Value> {
        match pat {
            Pattern::Ident(n) => self.put_variable(n, value),
            Pattern::Array { elements, rest } => {
                let iter = self.get_iterator(&value)?;
                for el in elements {
                    let v = self.iterator_step(&iter)?.unwrap_or(Value::Undefined);
                    if let Some(pe) = el {
                        self.assign_pattern(&pe.pattern, v)?;
                    }
                }
                if let Some(r) = rest {
                    let mut rest_items = Vec::new();
                    loop {
                        match self.iterator_step(&iter)? {
                            Some(v) => rest_items.push(v),
                            None => break,
                        }
                    }
                    let arr = self.new_array(rest_items);
                    self.assign_pattern(r, arr)?;
                }
                Ok(())
            }
            Pattern::Object { properties, rest } => {
                for p in properties {
                    let key = match &p.key {
                        PropertyKey::Ident(n) | PropertyKey::String(n) | PropertyKey::Private(n) => PropKey::Str(n.clone()),
                        PropertyKey::Number(n) => PropKey::Str(Rc::from(format_number(*n).as_str())),
                        _ => PropKey::from_str(""),
                    };
                    let v = self.get_property(&value, &key)?;
                    self.assign_pattern(&p.value, v)?;
                }
                if let Some(_) = rest {
                    // rest assignment not fully supported
                }
                Ok(())
            }
            Pattern::Rest(inner) => self.assign_pattern(inner, value),
            Pattern::Assignment { pattern, default } => {
                if value.is_undefined() {
                    let dv = self.eval_expr(default)?;
                    self.assign_pattern(pattern, dv)
                } else {
                    self.assign_pattern(pattern, value)
                }
            }
            Pattern::ArrayHole => Ok(()),
        }
    }

    // -----------------------------------------------------------------
    // Iteration protocol
    // -----------------------------------------------------------------

    pub fn is_iterable(&self, v: &Value) -> bool {
        let sym = self.realm().wk.iterator.clone();
        match v {
            Value::Object(o) => {
                let mut cur = Some(Value::Object(o.clone()));
                while let Some(Value::Object(c)) = cur {
                    if c.borrow().props.contains_key(&PropKey::Sym(sym.clone())) {
                        return true;
                    }
                    cur = c.borrow().proto.clone();
                }
                false
            }
            _ => false,
        }
    }

    pub fn get_iterator(&mut self, v: &Value) -> Result<Value, Value> {
        if matches!(v, Value::Undefined | Value::Null) {
            return Err(error::throw_type("Cannot read properties of null (iterating)"));
        }
        // strings are iterable
        if let Value::String(s) = v {
            let iter = self.make_string_iterator(s.clone());
            return Ok(iter);
        }
        let sym = self.realm().wk.iterator.clone();
        let fn_val = self.get_property(v, &PropKey::Sym(sym))?;
        if !fn_val.is_callable() {
            // arrays without Symbol.iterator? they always have it via proto
            return Err(error::throw_type("value is not iterable"));
        }
        let iter = self.call_value(fn_val, v.clone(), &[])?;
        // ensure it has next
        let next = self.get_property(&iter, &PropKey::from_str("next"))?;
        if !next.is_callable() {
            return Err(error::throw_type("iterator has no next"));
        }
        Ok(iter)
    }

    pub fn iterator_step(&mut self, iter: &Value) -> Result<Option<Value>, Value> {
        let next = self.get_property(iter, &PropKey::from_str("next"))?;
        let res = self.call_value(next, iter.clone(), &[])?;
        let done = self.get_property(&res, &PropKey::from_str("done"))?;
        if to_boolean(&done) {
            Ok(None)
        } else {
            let val = self.get_property(&res, &PropKey::from_str("value"))?;
            Ok(Some(val))
        }
    }

    // -----------------------------------------------------------------
    // instance_of
    // -----------------------------------------------------------------

    fn instance_of(&mut self, v: &Value, ctor: &Value) -> Result<Value, Value> {
        if !ctor.is_object() {
            return Err(error::throw_type("right-hand side of instanceof is not an object"));
        }
        // Symbol.hasInstance
        let hi = self.get_property(ctor, &PropKey::Sym(self.realm().wk.has_instance.clone()))?;
        if hi.is_callable() {
            let r = self.call_value(hi, ctor.clone(), &[v.clone()])?;
            return Ok(Value::Bool(to_boolean(&r)));
        }
        if !ctor.is_callable() {
            return Err(error::throw_type("right-hand side of instanceof is not callable"));
        }
        let proto = self.get_property(ctor, &PropKey::from_str("prototype"))?;
        let proto = if let Value::Object(p) = proto { p } else {
            return Err(error::throw_type("constructor.prototype is not an object"));
        };
        let mut cur = match v {
            Value::Object(o) => {
                let b = o.borrow();
                b.proto.clone()
            }
            _ => return Ok(Value::Bool(false)),
        };
        while let Some(Value::Object(o)) = cur {
            if Rc::ptr_eq(&o, &proto) {
                return Ok(Value::Bool(true));
            }
            cur = o.borrow().proto.clone();
        }
        Ok(Value::Bool(false))
    }

    // -----------------------------------------------------------------
    // wrappers
    // -----------------------------------------------------------------

    fn string_wrapper(&self, s: Rc<str>) -> ObjRef {
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(self.realm().string_proto.clone()));
        o.borrow_mut().class = "String";
        o.borrow_mut().kind = ObjectKind::String(s.clone());
        o.borrow_mut().props.insert(
            PropKey::from_str("length"),
            Property {
                kind: PropKind::Data(Value::from_int(s.chars().count() as i32)),
                writable: false,
                enumerable: false,
                configurable: false,
            },
        );
        o
    }
    fn number_wrapper(&self, n: f64) -> ObjRef {
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(self.realm().number_proto.clone()));
        o.borrow_mut().class = "Number";
        o.borrow_mut().kind = ObjectKind::Number(n);
        o
    }
    fn boolean_wrapper(&self, b: bool) -> ObjRef {
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(self.realm().boolean_proto.clone()));
        o.borrow_mut().class = "Boolean";
        o.borrow_mut().kind = ObjectKind::Boolean(b);
        o
    }

    pub fn new_array(&self, items: Vec<Value>) -> Value {
        let o = ObjectInner::new_array(items);
        o.borrow_mut().proto = Some(Value::Object(self.realm().array_proto.clone()));
        Value::Object(o)
    }

    fn make_string_iterator(&self, s: Rc<str>) -> Value {
        let chars: Vec<char> = s.chars().collect();
        let state = Rc::new(RefCell::new((chars, 0usize)));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(self.realm().string_iterator_proto.clone()));
        let realm = self.shared.clone();
        let next = make_native_value(
            &self.shared.realm,
            "next",
            0,
            Rc::new(move |interp: &mut Interpreter, _this, _args| {
                let (done, val) = {
                    let mut st = state.borrow_mut();
                    if st.1 >= st.0.len() {
                        (true, Value::Undefined)
                    } else {
                        let ch = st.0[st.1];
                        st.1 += 1;
                        (false, Value::from_string(ch.to_string()))
                    }
                };
                let obj = ObjectInner::new_object();
                obj.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
                obj.borrow_mut().props.insert(PropKey::from_str("value"), Property::data(val));
                obj.borrow_mut().props.insert(PropKey::from_str("done"), Property::data(Value::Bool(done)));
                Ok(Value::Object(obj))
            }),
        );
        o.borrow_mut().props.insert(PropKey::from_str("next"), Property::data(next));
        let _ = realm;
        Value::Object(o)
    }

    fn make_regexp(&mut self, pattern: &Rc<str>, flags: &Rc<str>) -> Result<Value, Value> {
        let re_str = translate_regex(pattern, flags);
        // Try the fast `regex` crate first; fall back to `fancy-regex` for
        // backreferences / lookaround (which `regex` doesn't support).
        let re = regex::Regex::new(&re_str).ok();
        let fancy = if re.is_none() {
            fancy_regex::Regex::new(&re_str).ok()
        } else {
            // Also try fancy-regex if the pattern contains backref/lookahead syntax
            // so captures work correctly for those features.
            if pattern.contains("\\K") || pattern.contains("(?<") || pattern.contains("(?=")
                || pattern.contains("(?!") || pattern.contains("(?<=") || pattern.contains("(?<!")
                || pattern.contains("\\1") || pattern.contains("\\2") || pattern.contains("\\3")
            {
                fancy_regex::Regex::new(&re_str).ok()
            } else {
                None
            }
        };
        if re.is_none() && fancy.is_none() {
            return Err(error::throw_syntax(&format!("invalid regular expression: /{}/{}", pattern, flags)));
        }
        let re = re.unwrap_or_else(|| {
            // Build a fallback empty regex (fancy will handle matching)
            regex::Regex::new(r"").unwrap()
        });
        let data = Rc::new(RegExpData {
            source: pattern.clone(),
            flags: flags.clone(),
            re,
            fancy,
            global: flags.contains('g'),
            last_index: Cell::new(0),
        });
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(self.realm().regexp_proto.clone()));
        o.borrow_mut().class = "RegExp";
        o.borrow_mut().kind = ObjectKind::RegExp(data);
        o.borrow_mut().props.insert(PropKey::from_str("source"), Property::data(Value::String(pattern.clone())));
        o.borrow_mut().props.insert(PropKey::from_str("flags"), Property::data(Value::String(flags.clone())));
        o.borrow_mut().props.insert(PropKey::from_str("global"), Property::data(Value::Bool(flags.contains('g'))));
        o.borrow_mut().props.insert(PropKey::from_str("ignoreCase"), Property::data(Value::Bool(flags.contains('i'))));
        o.borrow_mut().props.insert(PropKey::from_str("multiline"), Property::data(Value::Bool(flags.contains('m'))));
        o.borrow_mut().props.insert(PropKey::from_str("lastIndex"), Property::data(Value::from_int(0)));
        Ok(Value::Object(o))
    }

    fn make_import_meta(&self) -> ObjRef {
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(self.realm().object_proto.clone()));
        o
    }

    fn eval_import_call(&mut self, arg: &Expr) -> Result<Value, Value> {
        // dynamic import() — return a Promise that resolves to the module namespace.
        let p = self.new_promise();
        let specifier = self.eval_expr(arg)?;
        let spec_str = crate::value::to_string(&specifier);
        // Try to load and evaluate the module file.
        let result = self.load_module(&spec_str);
        match result {
            Ok(ns) => self.resolve_promise(p.clone(), ns),
            Err(e) => self.reject_promise(p.clone(), e),
        }
        Ok(p)
    }

    /// Load and evaluate a module from a file path (relative to CWD).
    /// Returns the module namespace object. Caches results.
    pub fn load_module(&mut self, specifier: &str) -> Result<Value, Value> {
        // Check cache first.
        if let Some(cached) = self.shared.realm.module_cache.borrow().get(specifier).cloned() {
            return Ok(cached);
        }
        // Resolve the file path. Add .js extension if missing.
        let path = if std::path::Path::new(specifier).exists() {
            specifier.to_string()
        } else if std::path::Path::new(&format!("{}.js", specifier)).exists() {
            format!("{}.js", specifier)
        } else {
            return Err(error::throw_type(&format!("Cannot find module '{}'", specifier)));
        };
        let src = std::fs::read_to_string(&path).map_err(|e| {
            error::throw_type(&format!("Cannot read module '{}': {}", specifier, e))
        })?;
        // Parse as a module.
        let prog = crate::parser::parse_module(&src).map_err(|e| {
            error::throw_syntax(&e.message)
        })?;
        // Evaluate in a fresh module scope (child of global).
        let saved = self.scope.clone();
        let ns = self.eval_module(&prog)?;
        self.scope = saved;
        // Cache the namespace.
        self.shared.realm.module_cache.borrow_mut().insert(specifier.to_string(), ns.clone());
        Ok(ns)
    }

    // -----------------------------------------------------------------
    // yield / await (coroutine glue)
    // -----------------------------------------------------------------

    fn current_yielder(&self) -> Option<*const Yielder> {
        let p = self.shared.yielder.get();
        if p.is_null() {
            None
        } else {
            Some(p as *const Yielder)
        }
    }

    fn eval_yield(&mut self, arg: &Option<Box<Expr>>, delegate: bool) -> Result<Value, Value> {
        let yielder = self.current_yielder().ok_or_else(|| {
            error::throw_syntax("yield is a reserved identifier")
        })?;
        let val = match arg {
            Some(e) => self.eval_expr(e)?,
            None => Value::Undefined,
        };
        if delegate {
            // yield* : delegate to an inner iterator.
            // For each value from the inner iterator, yield it to our caller.
            // When the inner iterator is done, its return value is the result
            // of the yield* expression.
            let iter = self.get_iterator(&val)?;
            let next_fn = self.get_property(&iter, &PropKey::from_str("next"))?;
            loop {
                let result = self.call_value(next_fn.clone(), iter.clone(), &[])?;
                let done = to_boolean(&self.get_property(&result, &PropKey::from_str("done"))?);
                let value = self.get_property(&result, &PropKey::from_str("value"))?;
                if done {
                    return Ok(value);
                }
                // Yield the inner value to our caller. The resume value from
                // the caller is ignored (simplified — doesn't forward to inner).
                let resume = unsafe { (*yielder).suspend(GeneratorYield::Yield(value)) };
                if let Err(e) = resume {
                    // throw into inner iterator if it supports throw
                    let throw_fn = self.get_property(&iter, &PropKey::from_str("throw"))?;
                    if throw_fn.is_callable() {
                        let r = self.call_value(throw_fn, iter.clone(), &[e])?;
                        let d = to_boolean(&self.get_property(&r, &PropKey::from_str("done"))?);
                        let v = self.get_property(&r, &PropKey::from_str("value"))?;
                        if d { return Ok(v); }
                        // continue yielding
                        continue;
                    }
                    return Err(e);
                }
            }
        } else {
            let input = unsafe { (*yielder).suspend(GeneratorYield::Yield(val)) };
            match input {
                Ok(v) => Ok(v),
                Err(e) => Err(e),
            }
        }
    }

    fn eval_await(&mut self, arg: &Expr) -> Result<Value, Value> {
        let yielder = self.current_yielder().ok_or_else(|| {
            error::throw_syntax("await is only valid in async functions")
        })?;
        let v = self.eval_expr(arg)?;
        let promise = self.to_promise(v)?;
        let result = unsafe { (*yielder).suspend(GeneratorYield::Await(promise)) };
        match result {
            Ok(v) => Ok(v),
            Err(e) => Err(e),
        }
    }

    pub fn to_promise(&mut self, v: Value) -> Result<Value, Value> {
        if let Value::Object(o) = &v {
            if matches!(o.borrow().kind, ObjectKind::Promise(_)) {
                return Ok(v);
            }
        }
        // thenable -> promise
        if self.get_property(&v, &PropKey::from_str("then"))?.is_callable() {
            let p = self.new_promise();
            let resolver = self.make_then_resolver(p.clone());
            let then_fn = self.get_property(&v, &PropKey::from_str("then"))?;
            self.call_value(then_fn, v, &[resolver.clone(), resolver])?;
            return Ok(p);
        }
        let p = self.new_promise();
        self.resolve_promise(p.clone(), v);
        Ok(p)
    }

    // -----------------------------------------------------------------
    // Generators / async (coroutine creation)
    // -----------------------------------------------------------------

    fn start_generator(&mut self, f: &Rc<Function>, this: Value, args: &[Value]) -> Result<Value, Value> {
        let env = Env::new(Some(f.closure.clone()), EnvKind::Function);
        env.0.borrow_mut().this_val = this;
        env.0.borrow_mut().home_object = f.home_object.clone();
        env.0.borrow_mut().parent_constructor = f.parent_class.clone();
        self.bind_params(f, args, &env)?;
        let body = match &f.body {
            FunctionBody::Js { body, decls, .. } => {
                for d in decls {
                    if let Some(name) = &d.name {
                        let nf = self.make_function(&d.func, d.is_async, d.is_generator, env.clone());
                        env.create(name, nf, true);
                    }
                }
                body.clone()
            }
            _ => Block { stmts: vec![] },
        };
        let interp = self.clone();
        let yielder_cell = Rc::new(Cell::new(std::ptr::null::<()>()));
        let yc = yielder_cell.clone();
        let func_env = env;
        let coro = corosensei::Coroutine::new(move |yielder: &Yielder, _input| {
            // set yielder pointers
            let ptr = yielder as *const Yielder as *const ();
            yc.set(ptr);
            let mut sub = interp;
            sub.shared.yielder.set(ptr);
            // Run the body in the function's own scope (params already bound).
            let _ = sub.hoist(&body.stmts, &func_env, false);
            let saved = sub.scope.clone();
            sub.scope = func_env.clone();
            let result = (|| {
                for s in &body.stmts {
                    match sub.exec_stmt(s) {
                        Ok(Completion::Return(v)) => return GeneratorResult::Done(v),
                        Ok(Completion::Normal(_)) => {}
                        Ok(_) => {}
                        Err(e) => return GeneratorResult::Throw(e),
                    }
                }
                GeneratorResult::Done(Value::Undefined)
            })();
            sub.scope = saved;
            sub.shared.yielder.set(std::ptr::null());
            result
        });
        let state = Rc::new(RefCell::new(GeneratorState {
            done: false,
            coro: Some(coro),
        }));
        let obj = ObjectInner::new_object();
        obj.borrow_mut().proto = Some(Value::Object(self.realm().generator_proto.clone()));
        obj.borrow_mut().class = "Generator";
        obj.borrow_mut().kind = ObjectKind::Generator(state.clone());
        // attach next/throw/return via the proto (defined in builtins), but also
        // store the yielder cell on the state so resume sites can read it.
        // We stash it in a private field of the state via a side map keyed by ptr.
        GEN_YIELDERS.with(|m| m.borrow_mut().insert(Rc::as_ptr(&state) as usize, yielder_cell));
        Ok(Value::Object(obj))
    }

    fn start_async_function(&mut self, f: &Rc<Function>, this: Value, args: &[Value], _new_target: Option<Value>) -> Result<Value, Value> {
        let env = Env::new(Some(f.closure.clone()), EnvKind::Function);
        env.0.borrow_mut().this_val = this;
        env.0.borrow_mut().home_object = f.home_object.clone();
        env.0.borrow_mut().parent_constructor = f.parent_class.clone();
        self.bind_params(f, args, &env)?;
        let body = match &f.body {
            FunctionBody::Js { body, decls, .. } => {
                for d in decls {
                    if let Some(name) = &d.name {
                        let nf = self.make_function(&d.func, d.is_async, d.is_generator, env.clone());
                        env.create(name, nf, true);
                    }
                }
                body.clone()
            }
            _ => Block { stmts: vec![] },
        };
        let promise = self.new_promise();
        let interp = self.clone();
        let yielder_cell = Rc::new(Cell::new(std::ptr::null::<()>()));
        let yc = yielder_cell.clone();
        let func_env = env;
        let coro = corosensei::Coroutine::new(move |yielder: &Yielder, _input| {
            let ptr = yielder as *const Yielder as *const ();
            yc.set(ptr);
            let mut sub = interp;
            sub.shared.yielder.set(ptr);
            let _ = sub.hoist(&body.stmts, &func_env, false);
            let saved = sub.scope.clone();
            sub.scope = func_env.clone();
            let result = (|| {
                for s in &body.stmts {
                    match sub.exec_stmt(s) {
                        Ok(Completion::Return(v)) => return GeneratorResult::AsyncReturn(v),
                        Ok(Completion::Normal(_)) => {}
                        Ok(_) => {}
                        Err(e) => return GeneratorResult::Throw(e),
                    }
                }
                GeneratorResult::AsyncReturn(Value::Undefined)
            })();
            sub.scope = saved;
            sub.shared.yielder.set(std::ptr::null());
            result
        });
        let promise_clone = promise.clone();
        let driver = AsyncDriver {
            coro,
            yielder_cell,
            promise: promise_clone,
        };
        driver.drive(self);
        Ok(promise)
    }

    // -----------------------------------------------------------------
    // Promises
    // -----------------------------------------------------------------

    pub fn new_promise(&self) -> Value {
        let state = Rc::new(RefCell::new(PromiseState {
            state: PromiseStatus::Pending,
            value: Value::Undefined,
            fulfill_reactions: Vec::new(),
            reject_reactions: Vec::new(),
            handled: false,
        }));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(self.realm().promise_proto.clone()));
        o.borrow_mut().class = "Promise";
        o.borrow_mut().kind = ObjectKind::Promise(state);
        Value::Object(o)
    }

    pub fn promise_state(&self, p: &Value) -> Option<Rc<RefCell<PromiseState>>> {
        if let Value::Object(o) = p {
            if let ObjectKind::Promise(s) = &o.borrow().kind {
                return Some(s.clone());
            }
        }
        None
    }

    pub fn resolve_promise(&mut self, promise: Value, value: Value) {
        let state = match self.promise_state(&promise) {
            Some(s) => s,
            None => return,
        };
        let reactions = {
            let mut b = state.borrow_mut();
            if !matches!(b.state, PromiseStatus::Pending) {
                return;
            }
            // resolve with thenable -> chain
            if let Value::Object(_) = &value {
                if self.get_property(&value, &PropKey::from_str("then")).map(|t| t.is_callable()).unwrap_or(false) {
                    // schedule then
                    let then = self.get_property(&value, &PropKey::from_str("then")).unwrap_or(Value::Undefined);
                    let p2 = promise.clone();
                    let resolver = self.make_then_resolver(p2.clone());
                    let interp_self = self.clone();
                    let then_clone = then.clone();
                    let value_clone = value.clone();
                    let resolver_clone = resolver.clone();
                    asyncrt::queue_microtask(&self.shared.async_rt, Box::new(move |interp| {
                        let _ = interp.call_value(then_clone, value_clone, &[resolver_clone.clone(), resolver_clone]);
                        let _ = p2;
                        let _ = interp_self;
                    }));
                    return;
                }
            }
            b.state = PromiseStatus::Fulfilled;
            b.value = value;
            std::mem::take(&mut b.fulfill_reactions)
        };
        for r in reactions {
            self.run_reaction(promise.clone(), r, true);
        }
    }

    pub fn reject_promise(&mut self, promise: Value, reason: Value) {
        let state = match self.promise_state(&promise) {
            Some(s) => s,
            None => return,
        };
        let reactions = {
            let mut b = state.borrow_mut();
            if !matches!(b.state, PromiseStatus::Pending) {
                return;
            }
            b.state = PromiseStatus::Rejected;
            b.value = reason.clone();
            std::mem::take(&mut b.reject_reactions)
        };
        if reactions.is_empty() {
            // Possibly unhandled rejection. Defer the check to a microtask so a
            // `.catch`/`.then` attached synchronously after rejection is honored.
            let p = promise.clone();
            let rt = self.shared.async_rt.clone();
            crate::asyncrt::queue_microtask(&rt, Box::new(move |interp| {
                let state = match interp.promise_state(&p) {
                    Some(s) => s,
                    None => return,
                };
                let unhandled = matches!(state.borrow().state, PromiseStatus::Rejected)
                    && state.borrow().reject_reactions.is_empty()
                    && !state.borrow().handled;
                if unhandled {
                    let reason = state.borrow().value.clone();
                    interp.report_unhandled_rejection(&reason);
                }
            }));
        }
        for r in reactions {
            self.run_reaction(promise.clone(), r, false);
        }
    }

    fn report_unhandled_rejection(&self, reason: &Value) {
        eprintln!("Unhandled promise rejection: {}", error::display_value(reason));
    }

    fn run_reaction(&mut self, promise: Value, reaction: Reaction, fulfilled: bool) {
        let Reaction { handler, resolve, reject } = reaction;
        let value = if let Value::Object(o) = &promise {
            if let ObjectKind::Promise(s) = &o.borrow().kind {
                s.borrow().value.clone()
            } else { Value::Undefined }
        } else { Value::Undefined };
        let fulfilled_v = fulfilled;
        let handler_v = handler.clone();
        let resolve_v = resolve.clone();
        let reject_v = reject.clone();
        let value_v = value.clone();
        asyncrt::queue_microtask(&self.shared.async_rt, Box::new(move |interp| {
            let _ = interp;
            let _ = promise;
            if handler_v.is_callable() {
                let r = interp.call_value(handler_v.clone(), Value::Undefined, &[value_v.clone()]);
                match r {
                    Ok(v) => interp.resolve_promise(resolve_v.clone(), v),
                    Err(e) => interp.reject_promise(reject_v.clone(), e),
                }
            } else if fulfilled_v {
                interp.resolve_promise(resolve_v.clone(), value_v);
            } else {
                interp.reject_promise(reject_v.clone(), value_v);
            }
        }));
    }

    fn make_then_resolver(&self, promise: Value) -> Value {
        let p = promise.clone();
        self.make_native("resolve", 1, Rc::new(move |interp, _this, args| {
            interp.resolve_promise(p.clone(), args.get(0).cloned().unwrap_or(Value::Undefined));
            Ok(Value::Undefined)
        }))
    }

    // -----------------------------------------------------------------
    // module / misc helpers
    // -----------------------------------------------------------------

    pub fn get_global(&mut self, name: &str) -> Value {
        self.get_property(&Value::Object(self.shared.realm.global.clone()), &PropKey::from_str(name))
            .unwrap_or(Value::Undefined)
    }
}

// ---------------------------------------------------------------------------
// Async driver — resumes a coroutine and wires await reactions to the promise.
// ---------------------------------------------------------------------------

struct AsyncDriver {
    coro: CoroutineHandle,
    yielder_cell: Rc<Cell<*const ()>>,
    promise: Value,
}

impl AsyncDriver {
    fn drive(mut self, interp: &mut Interpreter) {
        let prev = interp.shared.yielder.replace(self.yielder_cell.get());
        let result = self.coro.resume(Ok(Value::Undefined));
        interp.shared.yielder.set(prev);
        self.handle(result, interp);
    }

    fn handle(
        self,
        result: corosensei::CoroutineResult<GeneratorYield, GeneratorResult>,
        interp: &mut Interpreter,
    ) {
        match result {
            corosensei::CoroutineResult::Yield(GeneratorYield::Await(p)) => {
                let driver_yielder = self.yielder_cell.clone();
                let driver_promise = self.promise.clone();
                // Shared slot so exactly one of on_ok/on_err resumes the coroutine.
                let coro_slot = Rc::new(RefCell::new(Some(self.coro)));
                let then = interp.get_property(&p, &PropKey::from_str("then")).unwrap_or(Value::Undefined);
                if then.is_callable() {
                    let yc = driver_yielder.clone();
                    let pp = driver_promise.clone();
                    let slot_ok = coro_slot.clone();
                    let on_ok = interp.make_native("ok", 1, Rc::new(move |interp, _t, args| {
                        let v = args.get(0).cloned().unwrap_or(Value::Undefined);
                        if let Some(c) = slot_ok.borrow_mut().take() {
                            interp.continue_async(c, yc.clone(), pp.clone(), Ok(v));
                        }
                        Ok(Value::Undefined)
                    }));
                    let yc2 = driver_yielder.clone();
                    let pp2 = driver_promise.clone();
                    let slot_err = coro_slot.clone();
                    let on_err = interp.make_native("err", 1, Rc::new(move |interp, _t, args| {
                        let v = args.get(0).cloned().unwrap_or(Value::Undefined);
                        if let Some(c) = slot_err.borrow_mut().take() {
                            interp.continue_async(c, yc2.clone(), pp2.clone(), Err(v));
                        }
                        Ok(Value::Undefined)
                    }));
                    interp.call_value(then, p, &[on_ok, on_err]).ok();
                } else {
                    // Not a thenable: resume immediately with the value as Ok.
                    if let Some(c) = coro_slot.borrow_mut().take() {
                        interp.continue_async(c, driver_yielder, driver_promise, Ok(p));
                    }
                }
            }
            corosensei::CoroutineResult::Yield(GeneratorYield::Yield(_)) => {
                interp.reject_promise(self.promise, error::throw_syntax("yield in async function"));
            }
            corosensei::CoroutineResult::Return(GeneratorResult::AsyncReturn(v)) => {
                interp.resolve_promise(self.promise, v);
            }
            corosensei::CoroutineResult::Return(GeneratorResult::Done(v)) => {
                interp.resolve_promise(self.promise, v);
            }
            corosensei::CoroutineResult::Return(GeneratorResult::Throw(e)) => {
                interp.reject_promise(self.promise, e);
            }
        }
    }
}

impl Interpreter {
    fn continue_async(
        &mut self,
        mut coro: CoroutineHandle,
        yielder_cell: Rc<Cell<*const ()>>,
        promise: Value,
        input: Result<Value, Value>,
    ) {
        let prev = self.shared.yielder.replace(yielder_cell.get());
        let result = coro.resume(input);
        self.shared.yielder.set(prev);
        let driver = AsyncDriver { coro, yielder_cell, promise };
        driver.handle(result, self);
    }
}

// thread-local map from generator state pointer to its yielder cell
thread_local! {
    static GEN_YIELDERS: RefCell<HashMap<usize, Rc<Cell<*const ()>>>> = RefCell::new(HashMap::new());
}

pub fn get_generator_yielder(state: &Rc<RefCell<GeneratorState>>) -> Option<Rc<Cell<*const ()>>> {
    GEN_YIELDERS.with(|m| m.borrow().get(&(Rc::as_ptr(state) as usize)).cloned())
}

// ---------------------------------------------------------------------------
// Free helper functions / trait impls
// ---------------------------------------------------------------------------

/// Signature of a native function.
pub type NativeFn = Rc<dyn Fn(&mut Interpreter, Value, &[Value]) -> Result<Value, Value>>;

pub fn make_native_value(realm: &Rc<Realm>, name: &str, length: usize, func: NativeFn) -> Value {
    let f = Rc::new(Function {
        body: FunctionBody::Native { func, constructor: None },
        name: Rc::from(name),
        length,
        closure: realm.global_env.clone(),
        is_arrow: false,
        is_generator: false,
        is_async: false,
        is_method: false,
        is_constructor: false,
        home_object: None,
        class_fields: Vec::new(),
        parent_class: None,
        line: 0,
    });
    let o = ObjectInner::new_function(f);
    o.borrow_mut().proto = Some(Value::Object(realm.function_proto.clone()));
    o.borrow_mut().props.insert(PropKey::from_str("length"), Property {
        kind: PropKind::Data(Value::from_int(length as i32)),
        writable: false, enumerable: false, configurable: true,
    });
    o.borrow_mut().props.insert(PropKey::from_str("name"), Property {
        kind: PropKind::Data(Value::from_string(name.to_string())),
        writable: false, enumerable: false, configurable: true,
    });
    Value::Object(o)
}

/// Set the home_object on a function object (for super).
fn set_home_object(fo: &ObjRef, home: Value) {
    let new_kind = {
        let b = fo.borrow();
        if let ObjectKind::Function(rf) = &b.kind {
            let mut nf = (**rf).clone_struct();
            nf.home_object = Some(home);
            Some(ObjectKind::Function(Rc::new(nf)))
        } else {
            None
        }
    };
    if let Some(nk) = new_kind {
        fo.borrow_mut().kind = nk;
    }
}

impl Function {
    pub fn clone_struct(&self) -> Function {
        Function {
            body: self.body.clone_body(),
            name: self.name.clone(),
            length: self.length,
            closure: self.closure.clone(),
            is_arrow: self.is_arrow,
            is_generator: self.is_generator,
            is_async: self.is_async,
            is_method: self.is_method,
            is_constructor: self.is_constructor,
            home_object: self.home_object.clone(),
            class_fields: self.class_fields.iter().map(|f| ClassField {
                name: f.name.clone(),
                init: f.init.clone(),
            }).collect(),
            parent_class: self.parent_class.clone(),
            line: self.line,
        }
    }
}

impl FunctionBody {
    fn clone_body(&self) -> FunctionBody {
        match self {
            FunctionBody::Native { func, constructor } => FunctionBody::Native {
                func: func.clone(),
                constructor: constructor.clone(),
            },
            FunctionBody::Js { params, body, decls, strict } => FunctionBody::Js {
                params: params.clone(),
                body: body.clone(),
                decls: decls.clone(),
                strict: *strict,
            },
        }
    }
}

/// Extracted view of an object's callability (avoids holding a borrow).
pub enum CallKind {
    Native(Rc<dyn Fn(&mut Interpreter, Value, &[Value]) -> Result<Value, Value>>),
    Js(Rc<Function>),
    Bound { target: Value, this_arg: Value, bound_args: Vec<Value> },
    Other,
}

impl ObjectKind {
    fn clone_for_call(&self) -> CallKind {
        match self {
            ObjectKind::Function(f) => match &f.body {
                FunctionBody::Native { func, .. } => CallKind::Native(func.clone()),
                FunctionBody::Js { .. } => CallKind::Js(f.clone()),
            },
            ObjectKind::BoundFunction { target, this_arg, bound_args } => CallKind::Bound {
                target: target.clone(),
                this_arg: this_arg.clone(),
                bound_args: bound_args.clone(),
            },
            _ => CallKind::Other,
        }
    }
}

/// Translate a JS regex into a Rust regex (approximate; Rust regex lacks
/// backreferences/lookaround, which we ignore).
pub fn translate_regex(pattern: &str, flags: &str) -> String {
    let mut out = String::new();
    let ignore_case = flags.contains('i');
    if ignore_case {
        out.push_str("(?i)");
    }
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                out.push('\\');
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Parse a decimal/hex bigint literal string into a BigInt.
pub fn parse_bigint(s: &str) -> BigInt {
    let s = s.trim();
    let (neg, digits) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else {
        (false, s)
    };
    let digits = if let Some(h) = digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
        hex_to_dec(h)
    } else if let Some(o) = digits.strip_prefix("0o").or_else(|| digits.strip_prefix("0O")) {
        oct_to_dec(o)
    } else if let Some(b) = digits.strip_prefix("0b").or_else(|| digits.strip_prefix("0B")) {
        bin_to_dec(b)
    } else {
        digits.to_string()
    };
    let digits: String = digits.chars().filter(|c| *c != '_').collect();
    if digits.is_empty() || digits.chars().all(|c| c == '0') {
        return BigInt { negative: false, limbs: vec![] };
    }
    let mut limbs: Vec<u32> = vec![0];
    for ch in digits.chars() {
        let d = ch.to_digit(10).unwrap_or(0);
        // limbs *= 10
        let mut carry = 0u64;
        for limb in limbs.iter_mut() {
            let cur = *limb as u64 * 10 + carry;
            *limb = (cur & 0xffffffff) as u32;
            carry = cur >> 32;
        }
        while carry != 0 {
            limbs.push((carry & 0xffffffff) as u32);
            carry >>= 32;
        }
        // limbs += d
        let mut carry = d as u64;
        for limb in limbs.iter_mut() {
            let cur = *limb as u64 + carry;
            *limb = (cur & 0xffffffff) as u32;
            carry = cur >> 32;
            if carry == 0 {
                break;
            }
        }
        while carry != 0 {
            limbs.push((carry & 0xffffffff) as u32);
            carry >>= 32;
        }
    }
    // strip leading zeros (high-order)
    while limbs.len() > 1 && *limbs.last().unwrap() == 0 {
        limbs.pop();
    }
    BigInt { negative: neg && !limbs.is_empty(), limbs }
}

fn hex_to_dec(h: &str) -> String {
    let mut v: u128 = 0;
    for c in h.chars() {
        if let Some(d) = c.to_digit(16) {
            v = v * 16 + d as u128;
        }
    }
    v.to_string()
}
fn oct_to_dec(o: &str) -> String {
    let mut v: u128 = 0;
    for c in o.chars() {
        if let Some(d) = c.to_digit(8) {
            v = v * 8 + d as u128;
        }
    }
    v.to_string()
}
fn bin_to_dec(b: &str) -> String {
    let mut v: u128 = 0;
    for c in b.chars() {
        if let Some(d) = c.to_digit(2) {
            v = v * 2 + d as u128;
        }
    }
    v.to_string()
}

pub fn bigint_add(a: &BigInt, b: &BigInt) -> Rc<BigInt> {
    if a.negative == b.negative {
        let limbs = limbs_add(&a.limbs, &b.limbs);
        Rc::new(BigInt { negative: a.negative, limbs })
    } else {
        // subtract
        let cmp = limbs_cmp(&a.limbs, &b.limbs);
        if cmp >= 0 {
            let limbs = limbs_sub(&a.limbs, &b.limbs);
            Rc::new(BigInt { negative: a.negative, limbs })
        } else {
            let limbs = limbs_sub(&b.limbs, &a.limbs);
            Rc::new(BigInt { negative: b.negative, limbs })
        }
    }
}

pub fn bigint_sub(a: &BigInt, b: &BigInt) -> Rc<BigInt> {
    let nb = BigInt { negative: !b.negative, limbs: b.limbs.clone() };
    bigint_add(a, &nb)
}

pub fn bigint_mul(a: &BigInt, b: &BigInt) -> Rc<BigInt> {
    let mut res = vec![0u32; a.limbs.len() + b.limbs.len()];
    for i in 0..a.limbs.len() {
        let mut carry = 0u64;
        for j in 0..b.limbs.len() {
            let cur = res[i + j] as u64 + a.limbs[i] as u64 * b.limbs[j] as u64 + carry;
            res[i + j] = (cur & 0xffffffff) as u32;
            carry = cur >> 32;
        }
        res[i + b.limbs.len()] = res[i + b.limbs.len()].wrapping_add(carry as u32);
    }
    while res.len() > 1 && *res.last().unwrap() == 0 {
        res.pop();
    }
    Rc::new(BigInt { negative: a.negative ^ b.negative, limbs: res })
}

pub fn bigint_rem(a: &BigInt, b: &BigInt) -> Rc<BigInt> {
    if b.limbs.is_empty() {
        return Rc::new(BigInt { negative: false, limbs: vec![] });
    }
    let (q, r) = limbs_divmod(&a.limbs, &b.limbs);
    let _ = q;
    Rc::new(BigInt { negative: a.negative, limbs: r })
}

fn limbs_add(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut res = Vec::new();
    let mut carry = 0u64;
    let n = a.len().max(b.len());
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0) as u64;
        let y = b.get(i).copied().unwrap_or(0) as u64;
        let s = x + y + carry;
        res.push((s & 0xffffffff) as u32);
        carry = s >> 32;
    }
    if carry != 0 {
        res.push(carry as u32);
    }
    res
}

fn limbs_sub(a: &[u32], b: &[u32]) -> Vec<u32> {
    // a >= b assumed
    let mut res = Vec::new();
    let mut borrow = 0i64;
    for i in 0..a.len() {
        let x = a[i] as i64;
        let y = b.get(i).copied().unwrap_or(0) as i64;
        let mut s = x - y - borrow;
        if s < 0 {
            s += 1 << 32;
            borrow = 1;
        } else {
            borrow = 0;
        }
        res.push(s as u32);
    }
    while res.len() > 1 && *res.last().unwrap() == 0 {
        res.pop();
    }
    res
}

fn limbs_cmp(a: &[u32], b: &[u32]) -> i32 {
    if a.len() != b.len() {
        return if a.len() > b.len() { 1 } else { -1 };
    }
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return if a[i] > b[i] { 1 } else { -1 };
        }
    }
    0
}

fn limbs_divmod(a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
    // simple long division (base 2^32) — adequate for moderate sizes
    if limbs_cmp(a, b) < 0 {
        return (vec![], a.to_vec());
    }
    let mut rem: Vec<u32> = vec![];
    let mut quot: Vec<u32> = vec![0; a.len()];
    for i in (0..a.len()).rev() {
        rem.insert(0, a[i]);
        while rem.len() > 1 && *rem.last().unwrap() == 0 {
            rem.pop();
        }
        let mut q = 0u32;
        while limbs_cmp(&rem, b) >= 0 {
            rem = limbs_sub(&rem, b);
            q += 1;
        }
        quot[i] = q;
    }
    while quot.len() > 1 && *quot.last().unwrap() == 0 {
        quot.pop();
    }
    (quot, rem)
}

/// Convert a class member key to a binding pattern (for instance field names).
fn pattern_from_key(key: &PropertyKey, _computed: bool) -> Pattern {
    match key {
        PropertyKey::Ident(n) | PropertyKey::String(n) | PropertyKey::Private(n) => {
            Pattern::Ident(n.clone())
        }
        PropertyKey::Number(n) => Pattern::Ident(Rc::from(format_number(*n).as_str())),
        PropertyKey::Computed(_) => Pattern::Ident(Rc::from("__field")),
    }
}
