//! Map and Set constructors + prototypes.

use crate::realm::Realm;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{make_ctor, install_global_ctor, def_method, CtorFn};
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    install_map(interp, realm);
    install_set(interp, realm);
    install_weakmap(interp, realm);
    install_weakset(interp, realm);
}

fn install_weakmap(interp: &mut Interpreter, realm: &Rc<Realm>) {
    // WeakMap: simplified — same implementation as Map (no GC, but Rc keeps alive).
    let call_fn: NativeFn = Rc::new(|interp, _this, args| {
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().map_proto.clone()));
        o.borrow_mut().class = "WeakMap";
        o.borrow_mut().kind = ObjectKind::Map(Vec::new());
        let m = Value::Object(o);
        if let Some(it) = args.get(0) {
            if !it.is_nullish() {
                let iter = interp.get_iterator(it)?;
                loop {
                    match interp.iterator_step(&iter)? {
                        Some(entry) => {
                            let k = interp.get_property(&entry, &PropKey::from_str("0"))?;
                            let v = interp.get_property(&entry, &PropKey::from_str("1"))?;
                            map_set(&m, k, v);
                        }
                        None => break,
                    }
                }
            }
        }
        Ok(m)
    });
    let ctor_fn: CtorFn = { let cf = call_fn.clone(); Rc::new(move |interp, _t, args, _nt| cf(interp, Value::Undefined, args)) };
    let ctor = make_ctor(realm, "WeakMap", 0, call_fn, ctor_fn);
    install_global_ctor(interp, realm, "WeakMap", ctor, realm.map_proto.clone());
    let _ = interp;
}

fn install_weakset(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let call_fn: NativeFn = Rc::new(|interp, _this, args| {
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().set_proto.clone()));
        o.borrow_mut().class = "WeakSet";
        o.borrow_mut().kind = ObjectKind::Set(Vec::new());
        let s = Value::Object(o);
        if let Some(it) = args.get(0) {
            if !it.is_nullish() {
                let iter = interp.get_iterator(it)?;
                loop {
                    match interp.iterator_step(&iter)? {
                        Some(v) => set_add(&s, v),
                        None => break,
                    }
                }
            }
        }
        Ok(s)
    });
    let ctor_fn: CtorFn = { let cf = call_fn.clone(); Rc::new(move |interp, _t, args, _nt| cf(interp, Value::Undefined, args)) };
    let ctor = make_ctor(realm, "WeakSet", 0, call_fn, ctor_fn);
    install_global_ctor(interp, realm, "WeakSet", ctor, realm.set_proto.clone());
    let _ = interp;
}

fn install_map(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let call_fn: NativeFn = Rc::new(|interp, _this, args| {
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().map_proto.clone()));
        o.borrow_mut().class = "Map";
        o.borrow_mut().kind = ObjectKind::Map(Vec::new());
        let m = Value::Object(o);
        if let Some(it) = args.get(0) {
            if !it.is_nullish() {
                let iter = interp.get_iterator(it)?;
                loop {
                    match interp.iterator_step(&iter)? {
                        Some(entry) => {
                            let k = interp.get_property(&entry, &PropKey::from_str("0"))?;
                            let v = interp.get_property(&entry, &PropKey::from_str("1"))?;
                            map_set(&m, k, v);
                        }
                        None => break,
                    }
                }
            }
        }
        Ok(m)
    });
    let ctor_fn: CtorFn = {
        let cf = call_fn.clone();
        Rc::new(move |interp, _this, args, _nt| cf(interp, Value::Undefined, args))
    };
    let ctor = make_ctor(realm, "Map", 0, call_fn, ctor_fn);
    install_global_ctor(interp, realm, "Map", ctor, realm.map_proto.clone());
    let mp = realm.map_proto.clone();
    def_method(realm, &mp, "get", 1, Rc::new(|_i, this, args| {
        let k = args.get(0).cloned().unwrap_or(Value::Undefined);
        Ok(map_get(&this, &k))
    }));
    def_method(realm, &mp, "set", 2, Rc::new(|_i, this, args| {
        let k = args.get(0).cloned().unwrap_or(Value::Undefined);
        let v = args.get(1).cloned().unwrap_or(Value::Undefined);
        map_set(&this, k, v);
        Ok(this)
    }));
    def_method(realm, &mp, "has", 1, Rc::new(|_i, this, args| {
        let k = args.get(0).cloned().unwrap_or(Value::Undefined);
        Ok(Value::Bool(map_has(&this, &k)))
    }));
    def_method(realm, &mp, "delete", 1, Rc::new(|_i, this, args| {
        let k = args.get(0).cloned().unwrap_or(Value::Undefined);
        Ok(Value::Bool(map_delete(&this, &k)))
    }));
    def_method(realm, &mp, "clear", 0, Rc::new(|_i, this, _a| {
        if let Value::Object(o) = &this { if let ObjectKind::Map(e) = &mut o.borrow_mut().kind { e.clear(); } }
        Ok(Value::Undefined)
    }));
    def_method(realm, &mp, "forEach", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let entries = if let Value::Object(o) = &this { if let ObjectKind::Map(e) = &o.borrow().kind { e.clone() } else { vec![] } } else { vec![] };
        for (k, v) in entries {
            interp.call_value(cb.clone(), Value::Undefined, &[v, k, this.clone()])?;
        }
        Ok(Value::Undefined)
    }));
    def_method(realm, &mp, "keys", 0, Rc::new(|interp, this, _a| {
        let keys: Vec<Value> = if let Value::Object(o) = &this { if let ObjectKind::Map(e) = &o.borrow().kind { e.iter().map(|(k, _)| k.clone()).collect() } else { vec![] } } else { vec![] };
        interp.make_array_iterator_pub_value(keys)
    }));
    def_method(realm, &mp, "values", 0, Rc::new(|interp, this, _a| {
        let vals: Vec<Value> = if let Value::Object(o) = &this { if let ObjectKind::Map(e) = &o.borrow().kind { e.iter().map(|(_, v)| v.clone()).collect() } else { vec![] } } else { vec![] };
        interp.make_array_iterator_pub_value(vals)
    }));
    def_method(realm, &mp, "entries", 0, Rc::new(|interp, this, _a| {
        let entries: Vec<Value> = if let Value::Object(o) = &this { if let ObjectKind::Map(e) = &o.borrow().kind { e.iter().map(|(k, v)| interp.new_array(vec![k.clone(), v.clone()])).collect() } else { vec![] } } else { vec![] };
        interp.make_array_iterator_pub_value(entries)
    }));
    // size getter
    install_size_getter(realm, &mp, |this| {
        if let Value::Object(o) = &this { if let ObjectKind::Map(e) = &o.borrow().kind { e.len() } else { 0 } } else { 0 }
    });
    // Symbol.iterator -> entries
    let iter_fn = crate::interp::make_native_value(realm, "[Symbol.iterator]", 0, Rc::new(|interp, this, _a| {
        let entries: Vec<Value> = if let Value::Object(o) = &this { if let ObjectKind::Map(e) = &o.borrow().kind { e.iter().map(|(k, v)| interp.new_array(vec![k.clone(), v.clone()])).collect() } else { vec![] } } else { vec![] };
        interp.make_array_iterator_pub_value(entries)
    }));
    mp.borrow_mut().props.insert(PropKey::Sym(realm.wk.iterator.clone()), Property::data(iter_fn));
}

fn install_set(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let call_fn: NativeFn = Rc::new(|interp, _this, args| {
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().set_proto.clone()));
        o.borrow_mut().class = "Set";
        o.borrow_mut().kind = ObjectKind::Set(Vec::new());
        let s = Value::Object(o);
        if let Some(it) = args.get(0) {
            if !it.is_nullish() {
                let iter = interp.get_iterator(it)?;
                loop {
                    match interp.iterator_step(&iter)? {
                        Some(v) => set_add(&s, v),
                        None => break,
                    }
                }
            }
        }
        Ok(s)
    });
    let ctor_fn: CtorFn = { let cf = call_fn.clone(); Rc::new(move |interp, _t, args, _nt| cf(interp, Value::Undefined, args)) };
    let ctor = make_ctor(realm, "Set", 0, call_fn, ctor_fn);
    install_global_ctor(interp, realm, "Set", ctor, realm.set_proto.clone());
    let sp = realm.set_proto.clone();
    def_method(realm, &sp, "add", 1, Rc::new(|_i, this, args| { let v = args.get(0).cloned().unwrap_or(Value::Undefined); set_add(&this, v); Ok(this) }));
    def_method(realm, &sp, "has", 1, Rc::new(|_i, this, args| { let v = args.get(0).cloned().unwrap_or(Value::Undefined); Ok(Value::Bool(set_has(&this, &v))) }));
    def_method(realm, &sp, "delete", 1, Rc::new(|_i, this, args| { let v = args.get(0).cloned().unwrap_or(Value::Undefined); Ok(Value::Bool(set_delete(&this, &v))) }));
    def_method(realm, &sp, "clear", 0, Rc::new(|_i, this, _a| { if let Value::Object(o) = &this { if let ObjectKind::Set(e) = &mut o.borrow_mut().kind { e.clear(); } } Ok(Value::Undefined) }));
    def_method(realm, &sp, "forEach", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = if let Value::Object(o) = &this { if let ObjectKind::Set(e) = &o.borrow().kind { e.clone() } else { vec![] } } else { vec![] };
        for v in items { interp.call_value(cb.clone(), Value::Undefined, &[v.clone(), v, this.clone()])?; }
        Ok(Value::Undefined)
    }));
    def_method(realm, &sp, "keys", 0, Rc::new(|interp, this, _a| {
        let items = if let Value::Object(o) = &this { if let ObjectKind::Set(e) = &o.borrow().kind { e.clone() } else { vec![] } } else { vec![] };
        interp.make_array_iterator_pub_value(items)
    }));
    def_method(realm, &sp, "values", 0, Rc::new(|interp, this, _a| {
        let items = if let Value::Object(o) = &this { if let ObjectKind::Set(e) = &o.borrow().kind { e.clone() } else { vec![] } } else { vec![] };
        interp.make_array_iterator_pub_value(items)
    }));
    def_method(realm, &sp, "entries", 0, Rc::new(|interp, this, _a| {
        let items = if let Value::Object(o) = &this { if let ObjectKind::Set(e) = &o.borrow().kind { e.clone() } else { vec![] } } else { vec![] };
        let pairs: Vec<Value> = items.iter().map(|v| interp.new_array(vec![v.clone(), v.clone()])).collect();
        interp.make_array_iterator_pub_value(pairs)
    }));
    install_size_getter(realm, &sp, |this| {
        if let Value::Object(o) = &this { if let ObjectKind::Set(e) = &o.borrow().kind { e.len() } else { 0 } } else { 0 }
    });
    let iter_fn = crate::interp::make_native_value(realm, "[Symbol.iterator]", 0, Rc::new(|interp, this, _a| {
        let items = if let Value::Object(o) = &this { if let ObjectKind::Set(e) = &o.borrow().kind { e.clone() } else { vec![] } } else { vec![] };
        interp.make_array_iterator_pub_value(items)
    }));
    sp.borrow_mut().props.insert(PropKey::Sym(realm.wk.iterator.clone()), Property::data(iter_fn));
    let _ = interp;
}

fn install_size_getter(realm: &Rc<Realm>, proto: &ObjRef, getter: impl Fn(&Value) -> usize + 'static) {
    let g = crate::interp::make_native_value(realm, "size", 0, Rc::new(move |_i, this, _a| {
        Ok(Value::from_int(getter(&this) as i32))
    }));
    proto.borrow_mut().props.insert(PropKey::from_str("size"), Property {
        kind: PropKind::Accessor { get: Some(g), set: None },
        writable: false, enumerable: false, configurable: true,
    });
}

fn map_set(m: &Value, k: Value, v: Value) {
    if let Value::Object(o) = m {
        if let ObjectKind::Map(e) = &mut o.borrow_mut().kind {
            if let Some(slot) = e.iter_mut().find(|(ek, _)| same_value_zero(ek, &k)) {
                slot.1 = v;
            } else {
                e.push((k, v));
            }
        }
    }
}
fn map_get(m: &Value, k: &Value) -> Value {
    if let Value::Object(o) = m {
        if let ObjectKind::Map(e) = &o.borrow().kind {
            for (ek, ev) in e.iter() { if same_value_zero(ek, k) { return ev.clone(); } }
        }
    }
    Value::Undefined
}
fn map_has(m: &Value, k: &Value) -> bool {
    if let Value::Object(o) = m {
        if let ObjectKind::Map(e) = &o.borrow().kind {
            return e.iter().any(|(ek, _)| same_value_zero(ek, k));
        }
    }
    false
}
fn map_delete(m: &Value, k: &Value) -> bool {
    if let Value::Object(o) = m {
        if let ObjectKind::Map(e) = &mut o.borrow_mut().kind {
            if let Some(pos) = e.iter().position(|(ek, _)| same_value_zero(ek, k)) {
                e.remove(pos);
                return true;
            }
        }
    }
    false
}
fn set_add(s: &Value, v: Value) {
    if let Value::Object(o) = s {
        if let ObjectKind::Set(e) = &mut o.borrow_mut().kind {
            if !e.iter().any(|ev| same_value_zero(ev, &v)) { e.push(v); }
        }
    }
}
fn set_has(s: &Value, v: &Value) -> bool {
    if let Value::Object(o) = s {
        if let ObjectKind::Set(e) = &o.borrow().kind {
            return e.iter().any(|ev| same_value_zero(ev, v));
        }
    }
    false
}
fn set_delete(s: &Value, v: &Value) -> bool {
    if let Value::Object(o) = s {
        if let ObjectKind::Set(e) = &mut o.borrow_mut().kind {
            if let Some(pos) = e.iter().position(|ev| same_value_zero(ev, v)) { e.remove(pos); return true; }
        }
    }
    false
}

// Extension trait used by Map/Set iterators.
impl Interpreter {
    pub fn make_array_iterator_pub_value(&mut self, items: Vec<Value>) -> Result<Value, Value> {
        let arr = self.new_array(items);
        self.make_array_iterator(arr)
    }
}
