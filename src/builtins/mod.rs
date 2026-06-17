//! Built-in objects, constructors, prototypes and global functions.
//!
//! `install` wires everything into the realm's global object and prototypes.

use crate::realm::Realm;
use crate::ast::Expr;
use crate::error;
use crate::interp::{make_native_value, Interpreter, NativeFn};
use crate::value::*;
use std::cell::RefCell;
use std::rc::Rc;

mod array;
mod console;
mod date;
mod errors;
mod globals;
mod json;
mod mapset;
mod math;
mod node_modules;
mod number;
mod object;
mod promise;
mod proxy;
mod regexp;
mod string_b;
mod symbol;
mod typed;

pub fn install(interp: &mut Interpreter) {
    let realm = interp.realm().clone();
    // Object
    object::install(interp, &realm);
    // Function.prototype bits (call/apply/bind/toString)
    install_function_proto(interp, &realm);
    // Array
    array::install(interp, &realm);
    // String
    string_b::install(interp, &realm);
    // Number
    number::install(interp, &realm);
    // Boolean
    install_boolean(interp, &realm);
    // Symbol
    symbol::install(interp, &realm);
    // BigInt
    install_bigint(interp, &realm);
    // Math
    math::install(interp, &realm);
    // JSON
    json::install(interp, &realm);
    // Errors
    errors::install(interp, &realm);
    // Map / Set
    mapset::install(interp, &realm);
    // Date
    date::install(interp, &realm);
    // RegExp
    regexp::install(interp, &realm);
    // Promise
    promise::install(interp, &realm);
    // Proxy
    proxy::install(interp, &realm);
    // TypedArrays / ArrayBuffer
    typed::install(interp, &realm);
    // Generator prototype
    install_generator_proto(interp, &realm);
    // Iterator prototype
    install_iterator_proto(interp, &realm);
    // Iterator helper methods (map/filter/take/drop/forEach/reduce/toArray)
    install_iterator_helpers(interp, &realm);
    // Reflect
    install_reflect(interp, &realm);
    // console
    console::install(interp, &realm);
    // globals (parseInt, etc.) + timers + globalThis
    globals::install(interp, &realm);
    // Symbol.iterator on Array/String/Map/Set prototypes
    install_well_known_iterators(interp, &realm);
}

fn native(realm: &Rc<Realm>, name: &str, len: usize, f: NativeFn) -> Value {
    make_native_value(realm, name, len, f)
}

fn def_method(realm: &Rc<Realm>, obj: &ObjRef, name: &str, len: usize, f: NativeFn) {
    obj.borrow_mut().props.insert(
        PropKey::from_str(name),
        Property {
            kind: PropKind::Data(native(realm, name, len, f)),
            writable: true,
            enumerable: false,
            configurable: true,
        },
    );
}

fn def_const(obj: &ObjRef, name: &str, v: Value) {
    obj.borrow_mut().props.insert(
        PropKey::from_str(name),
        Property {
            kind: PropKind::Data(v),
            writable: false,
            enumerable: false,
            configurable: false,
        },
    );
}

pub fn def_const_value(obj: &ObjRef, name: &str, v: Value) {
    def_const(obj, name, v);
}

fn install_function_proto(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let fp = realm.function_proto.clone();
    def_method(realm, &fp, "call", 1, Rc::new(|interp, this, args| {
        let this_arg = args.get(0).cloned().unwrap_or(Value::Undefined);
        let rest: Vec<Value> = args.iter().skip(1).cloned().collect();
        interp.call_value(this, this_arg, &rest)
    }));
    def_method(realm, &fp, "apply", 2, Rc::new(|interp, this, args| {
        let this_arg = args.get(0).cloned().unwrap_or(Value::Undefined);
        let arr = args.get(1).cloned().unwrap_or(Value::Undefined);
        let argv = interp.iterable_to_vec(&arr)?;
        interp.call_value(this, this_arg, &argv)
    }));
    let function_proto = realm.function_proto.clone();
    def_method(realm, &fp, "bind", 1, Rc::new(move |interp, this, args| {
        if !this.is_callable() {
            return Err(error::throw_type("bind called on non-callable"));
        }
        let this_arg = args.get(0).cloned().unwrap_or(Value::Undefined);
        let bound_args: Vec<Value> = args.iter().skip(1).cloned().collect();
        let nargs = bound_args.len();
        let bound = ObjectInner::new_object();
        bound.borrow_mut().proto = Some(Value::Object(function_proto.clone()));
        bound.borrow_mut().class = "Function";
        bound.borrow_mut().kind = ObjectKind::BoundFunction {
            target: this.clone(),
            this_arg,
            bound_args,
        };
        // length & name
        let (tlen, tname) = if let Value::Object(o) = &this {
            let b = o.borrow();
            let len = b.props.get(&PropKey::from_str("length")).and_then(|p| match &p.kind { PropKind::Data(Value::Number(n)) => Some(*n as i64), _ => None }).unwrap_or(0);
            let name = b.props.get(&PropKey::from_str("name")).and_then(|p| match &p.kind { PropKind::Data(Value::String(s)) => Some(s.to_string()), _ => None }).unwrap_or_default();
            (len, name)
        } else { (0i64, String::new()) };
        let blen = (tlen - nargs as i64).max(0);
        bound.borrow_mut().props.insert(PropKey::from_str("length"), Property::data(Value::from_int(blen as i32)));
        bound.borrow_mut().props.insert(PropKey::from_str("name"), Property::data(Value::from_string(format!("bound {}", tname))));
        let _ = interp;
        Ok(Value::Object(bound))
    }));
    def_method(realm, &fp, "toString", 0, Rc::new(|_interp, this, _args| {
        if this.is_callable() {
            Ok(Value::from_str("function () { [native code] }"))
        } else {
            Ok(Value::from_str("function () { [native code] }"))
        }
    }));
    def_method(realm, &fp, "Symbol.hasInstance", 0, Rc::new(|_interp, _this, _args| {
        Ok(Value::Undefined)
    }));
    // @@hasInstance for Function: o instanceof F
    let fp2 = fp.clone();
    let hi = native(realm, "[Symbol.hasInstance]", 1, Rc::new(move |interp, this, args| {
        let v = args.get(0).cloned().unwrap_or(Value::Undefined);
        // ordinary instanceof
        if !this.is_callable() {
            return Err(error::throw_type("instanceof rhs not callable"));
        }
        let proto = interp.get_property(&this, &PropKey::from_str("prototype"))?;
        let proto = if let Value::Object(p) = proto { p } else {
            return Ok(Value::Bool(false));
        };
        let mut cur = if let Value::Object(o) = &v { o.borrow().proto.clone() } else { return Ok(Value::Bool(false)); };
        while let Some(Value::Object(o)) = cur {
            if Rc::ptr_eq(&o, &proto) { return Ok(Value::Bool(true)); }
            cur = o.borrow().proto.clone();
        }
        let _ = &fp2;
        Ok(Value::Bool(false))
    }));
    fp.borrow_mut().props.insert(PropKey::Sym(realm.wk.has_instance.clone()), Property::data(hi));
    let _ = interp;
}

fn install_boolean(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let call_fn: NativeFn = Rc::new(|_interp, _this, args| {
        Ok(Value::Bool(to_boolean(args.get(0).unwrap_or(&Value::Undefined))))
    });
    let ctor_fn: CtorFn = Rc::new(|interp, _this, args, _new_target| {
        let b = to_boolean(args.get(0).unwrap_or(&Value::Undefined));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().boolean_proto.clone()));
        o.borrow_mut().class = "Boolean";
        o.borrow_mut().kind = ObjectKind::Boolean(b);
        Ok(Value::Object(o))
    });
    let ctor = make_ctor(realm, "Boolean", 1, call_fn, ctor_fn);
    realm_set(realm, "boolean_ctor", ctor.clone());
    install_global_ctor(interp, realm, "Boolean", ctor.clone(), realm.boolean_proto.clone());
    // prototype
    def_method(realm, &realm.boolean_proto, "toString", 0, Rc::new(|_interp, this, _args| {
        let b = if let Value::Object(o) = &this {
            if let ObjectKind::Boolean(b) = o.borrow().kind { b } else { false }
        } else if let Value::Bool(b) = &this { *b } else { false };
        Ok(Value::from_str(if b { "true" } else { "false" }))
    }));
    def_method(realm, &realm.boolean_proto, "valueOf", 0, Rc::new(|_interp, this, _args| {
        if let Value::Object(o) = &this {
            if let ObjectKind::Boolean(b) = o.borrow().kind { return Ok(Value::Bool(b)); }
        }
        if let Value::Bool(b) = &this { return Ok(Value::Bool(*b)); }
        Ok(Value::Bool(false))
    }));
}

fn install_bigint(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let call_fn: NativeFn = Rc::new(|_interp, _this, args| {
        let v = args.get(0).cloned().unwrap_or(Value::Undefined);
        match &v {
            Value::Number(n) if n.is_finite() && n.fract() == 0.0 => {
                Ok(Value::BigInt(Rc::new(crate::interp::parse_bigint(&format!("{}", *n as i128)))))
            }
            Value::String(s) => Ok(Value::BigInt(Rc::new(crate::interp::parse_bigint(s)))),
            Value::Bool(b) => Ok(Value::BigInt(Rc::new(crate::interp::parse_bigint(if *b { "1" } else { "0" })))),
            Value::BigInt(b) => Ok(Value::BigInt(b.clone())),
            _ => Err(error::throw_type("cannot convert to BigInt")),
        }
    });
    let ctor_fn: CtorFn = Rc::new(|_interp, _this, _args, _nt| {
        Err(error::throw_type("BigInt is not a constructor"))
    });
    let ctor = make_ctor(realm, "BigInt", 1, call_fn, ctor_fn);
    realm_set(realm, "bigint_ctor", ctor.clone());
    install_global_ctor(interp, realm, "BigInt", ctor.clone(), realm.bigint_proto.clone());
    def_method(realm, &realm.bigint_proto, "toString", 0, Rc::new(|_interp, this, _args| {
        if let Value::Object(o) = &this {
            if let Some(PropKind::Data(Value::BigInt(b))) = o.borrow().props.get(&PropKey::from_str("[[BigIntData]]")).map(|p| &p.kind) {
                return Ok(Value::from_string(crate::value::bigint_to_string(b)));
            }
        }
        if let Value::BigInt(b) = &this { return Ok(Value::from_string(crate::value::bigint_to_string(b))); }
        Ok(Value::from_str("0"))
    }));
    def_method(realm, &realm.bigint_proto, "valueOf", 0, Rc::new(|_interp, this, _args| {
        if let Value::Object(o) = &this {
            if let Some(PropKind::Data(v)) = o.borrow().props.get(&PropKey::from_str("[[BigIntData]]")).map(|p| &p.kind) {
                if let Value::BigInt(b) = v { return Ok(Value::BigInt(b.clone())); }
            }
        }
        if let Value::BigInt(b) = &this { return Ok(Value::BigInt(b.clone())); }
        Ok(Value::BigInt(Rc::new(BigInt { negative: false, limbs: vec![] })))
    }));
}

fn install_generator_proto(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let gp = realm.generator_proto.clone();
    def_method(realm, &gp, "next", 1, Rc::new(|interp, this, args| {
        let arg = args.get(0).cloned().unwrap_or(Value::Undefined);
        generator_step(interp, this, Ok(arg))
    }));
    def_method(realm, &gp, "return", 1, Rc::new(|interp, this, args| {
        let v = args.get(0).cloned().unwrap_or(Value::Undefined);
        generator_return(interp, this, v)
    }));
    def_method(realm, &gp, "throw", 1, Rc::new(|interp, this, args| {
        let v = args.get(0).cloned().unwrap_or(Value::Undefined);
        generator_step(interp, this, Err(v))
    }));
    // Symbol.iterator returns itself
    let gi = native(realm, "[Symbol.iterator]", 0, Rc::new(|_interp, this, _args| Ok(this)));
    gp.borrow_mut().props.insert(PropKey::Sym(realm.wk.iterator.clone()), Property::data(gi));
    let _ = interp;
}

fn install_iterator_proto(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let ip = realm.iterator_proto.clone();
    let gi = native(realm, "[Symbol.iterator]", 0, Rc::new(|_interp, this, _args| Ok(this)));
    ip.borrow_mut().props.insert(PropKey::Sym(realm.wk.iterator.clone()), Property::data(gi));
    let _ = interp;
}

/// Install ES2025 Iterator Helper methods on the Iterator prototype.
/// These work on any object with a `next()` method (the iterator protocol).
fn install_iterator_helpers(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let ip = realm.iterator_proto.clone();
    // Iterator.prototype.map(fn) -> new iterator that yields fn(value)
    def_method(realm, &ip, "map", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let source = this.clone();
        let realm = interp.realm().clone();
        let next = make_native_value(&realm, "next", 0, Rc::new(move |interp, _this, _args| {
            let next_fn = interp.get_property(&source, &PropKey::from_str("next"))?;
            let v = interp.call_value(next_fn, source.clone(), &[])?;
            let done = to_boolean(&interp.get_property(&v, &PropKey::from_str("done"))?);
            if done { return Ok(v); }
            let val = interp.get_property(&v, &PropKey::from_str("value"))?;
            let mapped = interp.call_value(cb.clone(), Value::Undefined, &[val])?;
            let r = ObjectInner::new_object();
            r.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
            r.borrow_mut().props.insert(PropKey::from_str("value"), Property::data(mapped));
            r.borrow_mut().props.insert(PropKey::from_str("done"), Property::data(Value::Bool(false)));
            Ok(Value::Object(r))
        }));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().iterator_proto.clone()));
        o.borrow_mut().props.insert(PropKey::from_str("next"), Property::data(next));
        Ok(Value::Object(o))
    }));
    // Iterator.prototype.filter(fn) -> new iterator that only yields values where fn(value) is truthy
    def_method(realm, &ip, "filter", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let source = this.clone();
        let realm = interp.realm().clone();
        let next = make_native_value(&realm, "next", 0, Rc::new(move |interp, _this, _args| {
            loop {
                let next_fn = interp.get_property(&source, &PropKey::from_str("next"))?;
                let v = interp.call_value(next_fn, source.clone(), &[])?;
                let done = to_boolean(&interp.get_property(&v, &PropKey::from_str("done"))?);
                if done { return Ok(v); }
                let val = interp.get_property(&v, &PropKey::from_str("value"))?;
                let keep = interp.call_value(cb.clone(), Value::Undefined, &[val.clone()])?;
                if to_boolean(&keep) {
                    let r = ObjectInner::new_object();
                    r.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
                    r.borrow_mut().props.insert(PropKey::from_str("value"), Property::data(val));
                    r.borrow_mut().props.insert(PropKey::from_str("done"), Property::data(Value::Bool(false)));
                    return Ok(Value::Object(r));
                }
            }
        }));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().iterator_proto.clone()));
        o.borrow_mut().props.insert(PropKey::from_str("next"), Property::data(next));
        Ok(Value::Object(o))
    }));
    // Iterator.prototype.take(n) -> new iterator that yields at most n values
    def_method(realm, &ip, "take", 1, Rc::new(|interp, this, args| {
        let limit = to_int32(args.get(0).unwrap_or(&Value::from_int(0))) as i64;
        let source = this.clone();
        let realm = interp.realm().clone();
        let count = Rc::new(std::cell::Cell::new(0i64));
        let count_clone = count.clone();
        let next = make_native_value(&realm, "next", 0, Rc::new(move |interp, _this, _args| {
            let n = count_clone.get();
            if n >= limit {
                let r = ObjectInner::new_object();
                r.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
                r.borrow_mut().props.insert(PropKey::from_str("value"), Property::data(Value::Undefined));
                r.borrow_mut().props.insert(PropKey::from_str("done"), Property::data(Value::Bool(true)));
                return Ok(Value::Object(r));
            }
            count_clone.set(n + 1);
            let next_fn = interp.get_property(&source, &PropKey::from_str("next"))?;
            interp.call_value(next_fn, source.clone(), &[])
        }));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().iterator_proto.clone()));
        o.borrow_mut().props.insert(PropKey::from_str("next"), Property::data(next));
        let _ = count;
        Ok(Value::Object(o))
    }));
    // Iterator.prototype.toArray() -> array of all values
    def_method(realm, &ip, "toArray", 0, Rc::new(|interp, this, _args| {
        let mut out = Vec::new();
        loop {
            let next_fn = interp.get_property(&this, &PropKey::from_str("next"))?;
            let v = interp.call_value(next_fn, this.clone(), &[])?;
            let done = to_boolean(&interp.get_property(&v, &PropKey::from_str("done"))?);
            if done { break; }
            out.push(interp.get_property(&v, &PropKey::from_str("value"))?);
        }
        Ok(interp.new_array(out))
    }));
    // Iterator.prototype.forEach(fn) -> call fn(value) for each value
    def_method(realm, &ip, "forEach", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        loop {
            let next_fn = interp.get_property(&this, &PropKey::from_str("next"))?;
            let v = interp.call_value(next_fn, this.clone(), &[])?;
            let done = to_boolean(&interp.get_property(&v, &PropKey::from_str("done"))?);
            if done { break; }
            let val = interp.get_property(&v, &PropKey::from_str("value"))?;
            interp.call_value(cb.clone(), Value::Undefined, &[val])?;
        }
        Ok(Value::Undefined)
    }));
    // Iterator.prototype.reduce(fn, init) -> reduce all values
    def_method(realm, &ip, "reduce", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let mut acc = args.get(1).cloned().unwrap_or(Value::Undefined);
        let has_init = args.len() >= 2;
        let mut first = true;
        loop {
            let next_fn = interp.get_property(&this, &PropKey::from_str("next"))?;
            let v = interp.call_value(next_fn, this.clone(), &[])?;
            let done = to_boolean(&interp.get_property(&v, &PropKey::from_str("done"))?);
            if done { break; }
            let val = interp.get_property(&v, &PropKey::from_str("value"))?;
            if !has_init && first {
                acc = val;
                first = false;
            } else {
                acc = interp.call_value(cb.clone(), Value::Undefined, &[acc, val])?;
            }
        }
        Ok(acc)
    }));
    // Iterator.prototype.drop(n) -> new iterator that skips first n values
    def_method(realm, &ip, "drop", 1, Rc::new(|interp, this, args| {
        let count = to_int32(args.get(0).unwrap_or(&Value::from_int(0))) as i64;
        let source = this.clone();
        let realm = interp.realm().clone();
        let dropped = Rc::new(std::cell::Cell::new(false));
        let dropped_clone = dropped.clone();
        let next = make_native_value(&realm, "next", 0, Rc::new(move |interp, _this, _args| {
            if !dropped_clone.get() {
                dropped_clone.set(true);
                for _ in 0..count {
                    let next_fn = interp.get_property(&source, &PropKey::from_str("next"))?;
                    let _ = interp.call_value(next_fn, source.clone(), &[])?;
                }
            }
            let next_fn = interp.get_property(&source, &PropKey::from_str("next"))?;
            interp.call_value(next_fn, source.clone(), &[])
        }));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().iterator_proto.clone()));
        o.borrow_mut().props.insert(PropKey::from_str("next"), Property::data(next));
        let _ = dropped;
        Ok(Value::Object(o))
    }));
    // Iterator.prototype.flatMap(fn) -> new iterator that flattens results
    def_method(realm, &ip, "flatMap", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let source = this.clone();
        let realm = interp.realm().clone();
        let current_iter = Rc::new(std::cell::RefCell::new(Value::Undefined));
        let current_clone = current_iter.clone();
        let next = make_native_value(&realm, "next", 0, Rc::new(move |interp, _this, _args| {
            loop {
                // If we have a current sub-iterator, try to get its next value.
                let has_cur = !current_clone.borrow().is_undefined();
                if has_cur {
                    let cur = current_clone.borrow().clone();
                    let cur_next = interp.get_property(&cur, &PropKey::from_str("next"))?;
                    let v = interp.call_value(cur_next, cur.clone(), &[])?;
                    let done = to_boolean(&interp.get_property(&v, &PropKey::from_str("done"))?);
                    if !done {
                        return Ok(v);
                    }
                    // Sub-iterator exhausted; clear and continue to get next source value.
                    *current_clone.borrow_mut() = Value::Undefined;
                }
                // Get next value from source iterator.
                let next_fn = interp.get_property(&source, &PropKey::from_str("next"))?;
                let v = interp.call_value(next_fn, source.clone(), &[])?;
                let done = to_boolean(&interp.get_property(&v, &PropKey::from_str("done"))?);
                if done { return Ok(v); }
                let val = interp.get_property(&v, &PropKey::from_str("value"))?;
                let mapped = interp.call_value(cb.clone(), Value::Undefined, &[val])?;
                // If mapped is iterable, set as current sub-iterator; else yield directly.
                if interp.is_iterable(&mapped) || mapped.is_nullish() {
                    if mapped.is_nullish() { continue; }
                    let iter = interp.get_iterator(&mapped)?;
                    *current_clone.borrow_mut() = iter;
                } else {
                    // Non-iterable: yield directly.
                    let r = ObjectInner::new_object();
                    r.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
                    r.borrow_mut().props.insert(PropKey::from_str("value"), Property::data(mapped));
                    r.borrow_mut().props.insert(PropKey::from_str("done"), Property::data(Value::Bool(false)));
                    return Ok(Value::Object(r));
                }
            }
        }));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().iterator_proto.clone()));
        o.borrow_mut().props.insert(PropKey::from_str("next"), Property::data(next));
        let _ = current_iter;
        Ok(Value::Object(o))
    }));
    // Iterator.prototype.find(fn) -> first value where fn(value) is truthy, or undefined
    def_method(realm, &ip, "find", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        loop {
            let next_fn = interp.get_property(&this, &PropKey::from_str("next"))?;
            let v = interp.call_value(next_fn, this.clone(), &[])?;
            let done = to_boolean(&interp.get_property(&v, &PropKey::from_str("done"))?);
            if done { return Ok(Value::Undefined); }
            let val = interp.get_property(&v, &PropKey::from_str("value"))?;
            let found = interp.call_value(cb.clone(), Value::Undefined, &[val.clone()])?;
            if to_boolean(&found) { return Ok(val); }
        }
    }));
    // Iterator.prototype.some(fn) -> true if any value satisfies fn
    def_method(realm, &ip, "some", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        loop {
            let next_fn = interp.get_property(&this, &PropKey::from_str("next"))?;
            let v = interp.call_value(next_fn, this.clone(), &[])?;
            let done = to_boolean(&interp.get_property(&v, &PropKey::from_str("done"))?);
            if done { return Ok(Value::Bool(false)); }
            let val = interp.get_property(&v, &PropKey::from_str("value"))?;
            let ok = interp.call_value(cb.clone(), Value::Undefined, &[val])?;
            if to_boolean(&ok) { return Ok(Value::Bool(true)); }
        }
    }));
    // Iterator.prototype.every(fn) -> true if all values satisfy fn
    def_method(realm, &ip, "every", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        loop {
            let next_fn = interp.get_property(&this, &PropKey::from_str("next"))?;
            let v = interp.call_value(next_fn, this.clone(), &[])?;
            let done = to_boolean(&interp.get_property(&v, &PropKey::from_str("done"))?);
            if done { return Ok(Value::Bool(true)); }
            let val = interp.get_property(&v, &PropKey::from_str("value"))?;
            let ok = interp.call_value(cb.clone(), Value::Undefined, &[val])?;
            if !to_boolean(&ok) { return Ok(Value::Bool(false)); }
        }
    }));
    let _ = interp;
}

fn install_reflect(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let r = ObjectInner::new_object();
    r.borrow_mut().proto = Some(Value::Object(realm.object_proto.clone()));
    r.borrow_mut().class = "Reflect";
    def_method(realm, &r, "get", 2, Rc::new(|interp, _this, args| {
        let target = args.get(0).cloned().unwrap_or(Value::Undefined);
        let key = args.get(1).cloned().unwrap_or(Value::Undefined);
        interp.get_property(&target, &to_property_key(&key))
    }));
    def_method(realm, &r, "set", 3, Rc::new(|interp, _this, args| {
        let target = args.get(0).cloned().unwrap_or(Value::Undefined);
        let key = args.get(1).cloned().unwrap_or(Value::Undefined);
        let val = args.get(2).cloned().unwrap_or(Value::Undefined);
        interp.set_property(&target, &to_property_key(&key), val)?;
        Ok(Value::Bool(true))
    }));
    def_method(realm, &r, "has", 2, Rc::new(|interp, _this, args| {
        let target = args.get(0).cloned().unwrap_or(Value::Undefined);
        let key = args.get(1).cloned().unwrap_or(Value::Undefined);
        Ok(Value::Bool(interp.has_property(&target, &to_property_key(&key))))
    }));
    def_method(realm, &r, "deleteProperty", 2, Rc::new(|_interp, _this, args| {
        let target = args.get(0).cloned().unwrap_or(Value::Undefined);
        let key = to_property_key(&args.get(1).cloned().unwrap_or(Value::Undefined));
        if let Value::Object(o) = &target {
            o.borrow_mut().props.shift_remove(&key);
            Ok(Value::Bool(true))
        } else { Ok(Value::Bool(false)) }
    }));
    def_method(realm, &r, "ownKeys", 1, Rc::new(|interp, _this, args| {
        let target = args.get(0).cloned().unwrap_or(Value::Undefined);
        let mut keys = Vec::new();
        if let Value::Object(o) = &target {
            let b = o.borrow();
            if let ObjectKind::Array(items) = &b.kind {
                for i in 0..items.len() { keys.push(Value::from_string(i.to_string())); }
            }
            keys.push(Value::from_str("length"));
            for (k, _) in b.props.iter() {
                if let PropKey::Str(s) = k { keys.push(Value::String(s.clone())); }
            }
        }
        Ok(interp.new_array(keys))
    }));
    def_method(realm, &r, "getPrototypeOf", 1, Rc::new(|_this_unused, _this, args| {
        let _ = _this_unused;
        let target = args.get(0).cloned().unwrap_or(Value::Undefined);
        if let Value::Object(o) = &target { Ok(o.borrow().proto.clone().unwrap_or(Value::Null)) } else { Ok(Value::Null) }
    }));
    def_method(realm, &r, "apply", 3, Rc::new(|interp, _this, args| {
        let f = args.get(0).cloned().unwrap_or(Value::Undefined);
        let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
        let arr = args.get(2).cloned().unwrap_or(Value::Undefined);
        let argv = interp.iterable_to_vec(&arr)?;
        interp.call_value(f, this_arg, &argv)
    }));
    def_method(realm, &r, "construct", 2, Rc::new(|interp, _this, args| {
        let f = args.get(0).cloned().unwrap_or(Value::Undefined);
        let arr = args.get(1).cloned().unwrap_or(Value::Undefined);
        let argv = interp.iterable_to_vec(&arr)?;
        interp.construct(f.clone(), &argv, f)
    }));
    install_global(interp, realm, "Reflect", Value::Object(r));
}

fn install_well_known_iterators(interp: &mut Interpreter, realm: &Rc<Realm>) {
    // Array.prototype[Symbol.iterator]
    let arr_iter = native(realm, "values", 0, Rc::new(|interp, this, _args| {
        interp.make_array_iterator(this)
    }));
    realm.array_proto.borrow_mut().props.insert(PropKey::Sym(realm.wk.iterator.clone()), Property::data(arr_iter.clone()));
    realm.array_proto.borrow_mut().props.insert(PropKey::from_str("values"), Property::data(arr_iter.clone()));
    realm.array_proto.borrow_mut().props.insert(PropKey::from_str("keys"), Property::data(native(realm, "keys", 0, Rc::new(|interp, this, _args| {
        interp.make_array_key_iterator(this)
    }))));
    realm.array_proto.borrow_mut().props.insert(PropKey::from_str("entries"), Property::data(native(realm, "entries", 0, Rc::new(|interp, this, _args| {
        interp.make_array_entry_iterator(this)
    }))));
    // String.prototype[Symbol.iterator]
    let str_iter = native(realm, "[Symbol.iterator]", 0, Rc::new(|interp, this, _args| {
        let s = interp.coerce_to_string(&this)?;
        interp.make_string_iterator_pub(s)
    }));
    realm.string_proto.borrow_mut().props.insert(PropKey::Sym(realm.wk.iterator.clone()), Property::data(str_iter));
    // Map/Set iterators handled in mapset
    let _ = interp;
}

// ---------------------------------------------------------------------------
// helpers shared across builtins
// ---------------------------------------------------------------------------

pub type CtorFn = Rc<dyn Fn(&mut Interpreter, Value, &[Value], Value) -> Result<Value, Value>>;

pub fn make_ctor(realm: &Rc<Realm>, name: &str, len: usize, call_fn: NativeFn, ctor_fn: CtorFn) -> Value {
    let func = Rc::new(Function {
        body: FunctionBody::Native { func: call_fn, constructor: Some(ctor_fn) },
        name: Rc::from(name),
        length: len,
        closure: realm.global_env.clone(),
        is_arrow: false,
        is_generator: false,
        is_async: false,
        is_method: false,
        is_constructor: true,
        home_object: None,
        class_fields: Vec::new(),
        parent_class: None,
        line: 0,
    });
    let o = ObjectInner::new_function(func);
    o.borrow_mut().proto = Some(Value::Object(realm.function_proto.clone()));
    o.borrow_mut().props.insert(PropKey::from_str("length"), Property::data(Value::from_int(len as i32)));
    o.borrow_mut().props.insert(PropKey::from_str("name"), Property::data(Value::from_string(name.to_string())));
    Value::Object(o)
}

pub fn install_global(interp: &mut Interpreter, realm: &Rc<Realm>, name: &str, v: Value) {
    realm.global.borrow_mut().props.insert(PropKey::from_str(name), Property::data(v.clone()));
    if !realm.global_env.has_own(name) {
        realm.global_env.create(&Rc::from(name), v, true);
    } else {
        let _ = realm.global_env.set(name, v);
    }
    let _ = interp;
}

pub fn install_global_ctor(interp: &mut Interpreter, realm: &Rc<Realm>, name: &str, ctor: Value, proto: ObjRef) {
    // ctor.prototype = proto (non-writable, non-enumerable, non-configurable)
    if let Value::Object(co) = &ctor {
        co.borrow_mut().props.insert(PropKey::from_str("prototype"), Property {
            kind: PropKind::Data(Value::Object(proto.clone())),
            writable: false, enumerable: false, configurable: false,
        });
    }
    // proto.constructor = ctor (non-enumerable, like real JS)
    proto.borrow_mut().props.insert(PropKey::from_str("constructor"), Property {
        kind: PropKind::Data(ctor.clone()),
        writable: true,
        enumerable: false,
        configurable: true,
    });
    install_global(interp, realm, name, ctor);
}

fn realm_set(realm: &Rc<Realm>, field: &str, v: Value) {
    // We can't mutate Rc<Realm> fields directly; store via a side table on global.
    // Instead, we re-stash onto the realm via interior cell if available. Since the
    // realm's ctor fields are not RefCell, we keep a parallel map on the global object
    // under "__intrinsics__".
    let g = realm.global.clone();
    let intr = {
        let b = g.borrow();
        b.props.get(&PropKey::from_str("__intrinsics__")).cloned()
    };
    let intr = match intr {
        Some(Property { kind: PropKind::Data(Value::Object(o)), .. }) => o,
        _ => {
            let o = ObjectInner::new_object();
            g.borrow_mut().props.insert(PropKey::from_str("__intrinsics__"), Property::data(Value::Object(o.clone())));
            o
        }
    };
    intr.borrow_mut().props.insert(PropKey::from_str(field), Property::data(v));
}

pub fn realm_get(realm: &Rc<Realm>, field: &str) -> Value {
    let g = realm.global.clone();
    let b = g.borrow();
    if let Some(Property { kind: PropKind::Data(Value::Object(o)), .. }) = b.props.get(&PropKey::from_str("__intrinsics__")) {
        if let Some(Property { kind: PropKind::Data(v), .. }) = o.borrow().props.get(&PropKey::from_str(field)) {
            return v.clone();
        }
    }
    Value::Undefined
}

// ---------------------------------------------------------------------------
// Generator stepping
// ---------------------------------------------------------------------------

fn generator_step(interp: &mut Interpreter, this: Value, input: Result<Value, Value>) -> Result<Value, Value> {
    let state = if let Value::Object(o) = &this {
        if let ObjectKind::Generator(s) = &o.borrow().kind { s.clone() } else {
            return Err(error::throw_type("not a generator"));
        }
    } else {
        return Err(error::throw_type("not a generator"));
    };
    if state.borrow().done {
        return Ok(make_iter_result(Value::Undefined, true, interp));
    }
    let mut coro = match state.borrow_mut().coro.take() {
        Some(c) => c,
        None => return Ok(make_iter_result(Value::Undefined, true, interp)),
    };
    let yc = crate::interp::get_generator_yielder(&state).ok_or_else(|| error::throw_type("generator state missing"))?;
    let prev = interp.shared.yielder.replace(yc.get());
    let result = coro.resume(input);
    interp.shared.yielder.set(prev);
    match result {
        corosensei::CoroutineResult::Yield(GeneratorYield::Yield(v)) => {
            state.borrow_mut().coro = Some(coro);
            Ok(make_iter_result(v, false, interp))
        }
        corosensei::CoroutineResult::Return(GeneratorResult::Done(v)) => {
            state.borrow_mut().done = true;
            Ok(make_iter_result(v, true, interp))
        }
        corosensei::CoroutineResult::Return(GeneratorResult::Throw(e)) => {
            state.borrow_mut().done = true;
            Err(e)
        }
        corosensei::CoroutineResult::Return(GeneratorResult::AsyncReturn(v)) => {
            state.borrow_mut().done = true;
            Ok(make_iter_result(v, true, interp))
        }
        corosensei::CoroutineResult::Yield(GeneratorYield::Await(_)) => {
            state.borrow_mut().done = true;
            Err(error::throw_type("await in generator not supported via this path"))
        }
    }
}

fn generator_return(interp: &mut Interpreter, this: Value, v: Value) -> Result<Value, Value> {
    let state = if let Value::Object(o) = &this {
        if let ObjectKind::Generator(s) = &o.borrow().kind { s.clone() } else {
            return Err(error::throw_type("not a generator"));
        }
    } else {
        return Err(error::throw_type("not a generator"));
    };
    state.borrow_mut().done = true;
    state.borrow_mut().coro = None; // drop the coroutine
    Ok(make_iter_result(v, true, interp))
}

fn make_iter_result(value: Value, done: bool, interp: &mut Interpreter) -> Value {
    let o = ObjectInner::new_object();
    o.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
    o.borrow_mut().props.insert(PropKey::from_str("value"), Property::data(value));
    o.borrow_mut().props.insert(PropKey::from_str("done"), Property::data(Value::Bool(done)));
    Value::Object(o)
}

// ---------------------------------------------------------------------------
// Interpreter helper extensions used by builtins
// ---------------------------------------------------------------------------

impl Interpreter {
    pub fn iterable_to_vec(&mut self, v: &Value) -> Result<Vec<Value>, Value> {
        if v.is_nullish() {
            return Ok(Vec::new());
        }
        if let Value::Object(o) = v {
            if let ObjectKind::Array(items) = &o.borrow().kind {
                return Ok(items.clone());
            }
        }
        if let Value::String(s) = v {
            return Ok(s.chars().map(|c| Value::from_string(c.to_string())).collect());
        }
        let iter = self.get_iterator(v)?;
        let mut out = Vec::new();
        loop {
            match self.iterator_step(&iter)? {
                Some(x) => out.push(x),
                None => break,
            }
        }
        Ok(out)
    }

    pub fn coerce_to_string(&mut self, v: &Value) -> Result<Rc<str>, Value> {
        match v {
            Value::String(s) => Ok(s.clone()),
            Value::Object(o) => {
                if let ObjectKind::String(s) = &o.borrow().kind { return Ok(s.clone()); }
                let p = self.to_primitive(v, "string")?;
                Ok(Rc::from(crate::value::to_string(&p).as_str()))
            }
            _ => Ok(Rc::from(crate::value::to_string(v).as_str())),
        }
    }

    pub fn coerce_to_number(&mut self, v: &Value) -> Result<f64, Value> {
        match v {
            Value::Object(_) => {
                let p = self.to_primitive(v, "number")?;
                Ok(crate::value::to_number(&p))
            }
            _ => Ok(crate::value::to_number(v)),
        }
    }

    pub fn make_array_iterator(&mut self, this: Value) -> Result<Value, Value> {
        let arr = if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &o.borrow().kind { items.clone() } else {
                return Err(error::throw_type("not an array"));
            }
        } else { return Err(error::throw_type("not an array")); };
        let state = Rc::new(RefCell::new((arr, 0usize)));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(self.realm().array_iterator_proto.clone()));
        let realm = self.realm().clone();
        let next = make_native_value(&realm, "next", 0, Rc::new(move |interp, _t, _a| {
            let (done, val) = {
                let mut st = state.borrow_mut();
                if st.1 >= st.0.len() { (true, Value::Undefined) }
                else { let v = st.0[st.1].clone(); st.1 += 1; (false, v) }
            };
            Ok(make_iter_result(val, done, interp))
        }));
        o.borrow_mut().props.insert(PropKey::from_str("next"), Property::data(next));
        Ok(Value::Object(o))
    }
    pub fn make_array_key_iterator(&mut self, this: Value) -> Result<Value, Value> {
        let len = if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &o.borrow().kind { items.len() } else { 0 }
        } else { 0 };
        let state = Rc::new(RefCell::new((0usize, len)));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(self.realm().array_iterator_proto.clone()));
        let realm = self.realm().clone();
        let next = make_native_value(&realm, "next", 0, Rc::new(move |interp, _t, _a| {
            let (done, val) = {
                let mut st = state.borrow_mut();
                if st.0 >= st.1 { (true, Value::Undefined) }
                else { let v = Value::from_int(st.0 as i32); st.0 += 1; (false, v) }
            };
            Ok(make_iter_result(val, done, interp))
        }));
        o.borrow_mut().props.insert(PropKey::from_str("next"), Property::data(next));
        Ok(Value::Object(o))
    }
    pub fn make_array_entry_iterator(&mut self, this: Value) -> Result<Value, Value> {
        let arr = if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &o.borrow().kind { items.clone() } else { vec![] }
        } else { vec![] };
        let state = Rc::new(RefCell::new((arr, 0usize)));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(self.realm().array_iterator_proto.clone()));
        let realm = self.realm().clone();
        let next = make_native_value(&realm, "next", 0, Rc::new(move |interp, _t, _a| {
            let (done, val) = {
                let mut st = state.borrow_mut();
                if st.1 >= st.0.len() { (true, Value::Undefined) }
                else {
                    let pair = interp.new_array(vec![Value::from_int(st.1 as i32), st.0[st.1].clone()]);
                    st.1 += 1;
                    (false, pair)
                }
            };
            Ok(make_iter_result(val, done, interp))
        }));
        o.borrow_mut().props.insert(PropKey::from_str("next"), Property::data(next));
        Ok(Value::Object(o))
    }
    pub fn make_string_iterator_pub(&mut self, s: Rc<str>) -> Result<Value, Value> {
        // reuse the private one via a small wrapper
        let chars: Vec<char> = s.chars().collect();
        let state = Rc::new(RefCell::new((chars, 0usize)));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(self.realm().string_iterator_proto.clone()));
        let realm = self.realm().clone();
        let next = make_native_value(&realm, "next", 0, Rc::new(move |interp, _t, _a| {
            let (done, val) = {
                let mut st = state.borrow_mut();
                if st.1 >= st.0.len() { (true, Value::Undefined) }
                else { let c = st.0[st.1]; st.1 += 1; (false, Value::from_string(c.to_string())) }
            };
            Ok(make_iter_result(val, done, interp))
        }));
        o.borrow_mut().props.insert(PropKey::from_str("next"), Property::data(next));
        Ok(Value::Object(o))
    }
}

/// Pretty-print a value for console.log (Node-ish formatting).
pub fn pretty_print(v: &Value, interp: &Interpreter, depth: usize) -> String {
    use std::collections::HashSet;
    fn pp(v: &Value, interp: &Interpreter, depth: usize, seen: &mut HashSet<usize>) -> String {
        match v {
            Value::Undefined => "undefined".to_string(),
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => crate::value::format_number(*n),
            Value::BigInt(b) => crate::value::bigint_to_string(b) + "n",
            Value::String(s) => if depth == 0 { s.to_string() } else { format!("'{}'", s) },
            Value::Symbol(s) => {
                let desc = s.description.as_deref().unwrap_or("");
                format!("Symbol({})", desc)
            }
            Value::Object(o) => {
                let ptr = Rc::as_ptr(o) as usize;
                if seen.contains(&ptr) {
                    return "[Circular]".to_string();
                }
                seen.insert(ptr);
                let b = o.borrow();
                if let ObjectKind::Function(_) | ObjectKind::BoundFunction { .. } = &b.kind {
                    let name = b.props.get(&PropKey::from_str("name")).and_then(|p| if let PropKind::Data(Value::String(s)) = &p.kind { Some(s.to_string()) } else { None }).unwrap_or_default();
                    return format!("[Function: {}]", name);
                }
                if let ObjectKind::Array(items) = &b.kind {
                    let inner: Vec<String> = items.iter().map(|i| pp(i, interp, depth + 1, seen)).collect();
                    return format!("[ {} ]", inner.join(", "));
                }
                if let ObjectKind::Error = b.kind {
                    let name = b.props.get(&PropKey::from_str("name")).and_then(|p| if let PropKind::Data(Value::String(s)) = &p.kind { Some(s.to_string()) } else { None }).unwrap_or_else(|| "Error".to_string());
                    let msg = b.props.get(&PropKey::from_str("message")).and_then(|p| if let PropKind::Data(Value::String(s)) = &p.kind { Some(s.to_string()) } else { None }).unwrap_or_default();
                    if msg.is_empty() { return name; }
                    return format!("{}: {}", name, msg);
                }
                if let ObjectKind::String(s) = &b.kind {
                    return format!("[String: '{}']", s);
                }
                if let ObjectKind::Number(n) = &b.kind {
                    return format!("[Number: {}]", crate::value::format_number(*n));
                }
                if let ObjectKind::Boolean(b) = &b.kind {
                    return format!("[Boolean: {}]", b);
                }
                if let ObjectKind::Date(t) = &b.kind {
                    return format!("{} (ISO)", crate::value::date_format(*t));
                }
                if let ObjectKind::RegExp(d) = &b.kind {
                    return format!("/{}/{}", d.source, d.flags);
                }
                if let ObjectKind::Promise(_) = &b.kind {
                    return "[Promise]".to_string();
                }
                if let ObjectKind::Map(entries) = &b.kind {
                    let inner: Vec<String> = entries.iter().map(|(k, v)| format!("{} => {}", pp(k, interp, depth + 1, seen), pp(v, interp, depth + 1, seen))).collect();
                    return format!("Map({}) {{ {} }}", entries.len(), inner.join(", "));
                }
                if let ObjectKind::Set(items) = &b.kind {
                    let inner: Vec<String> = items.iter().map(|i| pp(i, interp, depth + 1, seen)).collect();
                    return format!("Set({}) {{ {} }}", items.len(), inner.join(", "));
                }
                let tag = b.props.get(&PropKey::Sym(interp.realm().wk.to_string_tag.clone())).and_then(|p| if let PropKind::Data(Value::String(s)) = &p.kind { Some(s.to_string()) } else { None });
                if let Some(t) = tag {
                    return format!("[{}]", t);
                }
                // generic object
                let mut entries: Vec<(String, String)> = Vec::new();
                for (k, p) in b.props.iter() {
                    if !p.enumerable { continue; }
                    let ks = match k { PropKey::Str(s) => s.to_string(), PropKey::Sym(s) => format!("[Symbol({})]", s.description.as_deref().unwrap_or("")) };
                    let vs = match &p.kind {
                        PropKind::Data(v) => pp(v, interp, depth + 1, seen),
                        PropKind::Accessor { .. } => "[Getter]".to_string(),
                    };
                    entries.push((ks, vs));
                }
                if entries.is_empty() {
                    "{}".to_string()
                } else {
                    let inner: Vec<String> = entries.iter().map(|(k, v)| format!("{}: {}", k, v)).collect();
                    format!("{{ {} }}", inner.join(", "))
                }
            }
        }
    }
    let mut seen = HashSet::new();
    pp(v, interp, depth, &mut seen)
}

// keep Expr import used
fn _unused(_: &Expr) {}
