//! Object constructor + Object.prototype.

use crate::realm::Realm;
use crate::error;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{make_ctor, install_global, install_global_ctor, def_method, native, CtorFn};
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let call_fn: NativeFn = Rc::new(|interp, _this, args| {
        let v = args.get(0).cloned().unwrap_or(Value::Undefined);
        if v.is_nullish() {
            let o = ObjectInner::new_object();
            o.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
            Ok(Value::Object(o))
        } else {
            interp.to_object(&v)
        }
    });
    let ctor_fn: CtorFn = Rc::new(|interp, _this, args, _nt| {
        let v = args.get(0).cloned().unwrap_or(Value::Undefined);
        if v.is_nullish() {
            let o = ObjectInner::new_object();
            o.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
            Ok(Value::Object(o))
        } else {
            interp.to_object(&v)
        }
    });
    let ctor = make_ctor(realm, "Object", 1, call_fn, ctor_fn);
    install_global_ctor(interp, realm, "Object", ctor.clone(), realm.object_proto.clone());
    // store on realm
    realm.global.borrow_mut().props.insert(PropKey::from_str("__object_ctor"), Property::data(ctor.clone()));

    let op = realm.object_proto.clone();
    def_method(realm, &op, "hasOwnProperty", 1, Rc::new(|interp, this, args| {
        let key = to_property_key(args.get(0).unwrap_or(&Value::Undefined));
        let obj = interp.to_object(&this)?;
        let has = if let Value::Object(o) = &obj {
            let b = o.borrow();
            if let ObjectKind::Array(items) = &b.kind {
                if let PropKey::Str(k) = &key {
                    if &**k == "length" { return Ok(Value::Bool(true)); }
                    if let Some(idx) = key_to_index(k) { return Ok(Value::Bool(idx < items.len())); }
                }
            }
            b.props.contains_key(&key)
        } else { false };
        Ok(Value::Bool(has))
    }));
    def_method(realm, &op, "isPrototypeOf", 1, Rc::new(|_interp, this, args| {
        let v = args.get(0).cloned().unwrap_or(Value::Undefined);
        let mut cur = if let Value::Object(o) = &v { o.borrow().proto.clone() } else { return Ok(Value::Bool(false)); };
        let target = if let Value::Object(o) = &this { o.clone() } else { return Ok(Value::Bool(false)); };
        while let Some(Value::Object(o)) = cur {
            if Rc::ptr_eq(&o, &target) { return Ok(Value::Bool(true)); }
            cur = o.borrow().proto.clone();
        }
        Ok(Value::Bool(false))
    }));
    def_method(realm, &op, "propertyIsEnumerable", 1, Rc::new(|interp, this, args| {
        let key = to_property_key(args.get(0).unwrap_or(&Value::Undefined));
        let obj = interp.to_object(&this)?;
        if let Value::Object(o) = &obj {
            if let Some(p) = o.borrow().props.get(&key) {
                return Ok(Value::Bool(p.enumerable));
            }
        }
        Ok(Value::Bool(false))
    }));
    def_method(realm, &op, "toString", 0, Rc::new(|interp, this, _args| {
        // [object X] using Symbol.toStringTag or class
        let tag = interp.get_property(&this, &PropKey::Sym(interp.realm().wk.to_string_tag.clone()))?;
        let tag = if let Value::String(s) = &tag { s.to_string() }
            else if let Value::Object(o) = &this { o.borrow().class.to_string() }
            else if this.is_null() { "Null".to_string() }
            else if this.is_undefined() { "Undefined".to_string() }
            else { this.type_of().to_string() };
        Ok(Value::from_string(format!("[object {}]", tag)))
    }));
    def_method(realm, &op, "toLocaleString", 0, Rc::new(|interp, this, _args| {
        let to = interp.get_property(&this, &PropKey::from_str("toString"))?;
        interp.call_value(to, this, &[])
    }));
    def_method(realm, &op, "valueOf", 0, Rc::new(|_interp, this, _args| Ok(this)));

    // Object static methods
    if let Value::Object(co) = &ctor {
        let co = co.clone();
        def_static(realm, &co, "keys", 1, Rc::new(|interp, _this, args| {
            let o = args.get(0).cloned().unwrap_or(Value::Undefined);
            let obj = interp.to_object(&o)?;
            let all_keys = interp.own_property_keys(&obj)?;
            let vals: Vec<Value> = all_keys.into_iter().filter_map(|k| {
                if let PropKey::Str(s) = k { Some(Value::String(s)) } else { None }
            }).collect();
            Ok(interp.new_array(vals))
        }));
        def_static(realm, &co, "values", 1, Rc::new(|interp, _this, args| {
            let o = args.get(0).cloned().unwrap_or(Value::Undefined);
            let obj = interp.to_object(&o)?;
            let all_keys = interp.own_property_keys(&obj)?;
            let mut vals = Vec::new();
            for k in all_keys {
                if let PropKey::Str(_) = k {
                    let v = interp.get_property(&obj, &k)?;
                    vals.push(v);
                }
            }
            Ok(interp.new_array(vals))
        }));
        def_static(realm, &co, "entries", 1, Rc::new(|interp, _this, args| {
            let o = args.get(0).cloned().unwrap_or(Value::Undefined);
            let obj = interp.to_object(&o)?;
            let all_keys = interp.own_property_keys(&obj)?;
            let mut entries = Vec::new();
            for k in all_keys {
                if let PropKey::Str(_) = &k {
                    let v = interp.get_property(&obj, &k)?;
                    let pair = interp.new_array(vec![value_from_key(&k), v]);
                    entries.push(pair);
                }
            }
            Ok(interp.new_array(entries))
        }));
        def_static(realm, &co, "fromEntries", 1, Rc::new(|interp, _this, args| {
            let it = args.get(0).cloned().unwrap_or(Value::Undefined);
            let iter = interp.get_iterator(&it)?;
            let obj = ObjectInner::new_object();
            obj.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
            loop {
                match interp.iterator_step(&iter)? {
                    Some(pair) => {
                        let k = interp.get_property(&pair, &PropKey::from_str("0"))?;
                        let v = interp.get_property(&pair, &PropKey::from_str("1"))?;
                        obj.borrow_mut().props.insert(to_property_key(&k), Property::data(v));
                    }
                    None => break,
                }
            }
            Ok(Value::Object(obj))
        }));
        def_static(realm, &co, "assign", 2, Rc::new(|interp, _this, args| {
            let target = args.get(0).cloned().unwrap_or(Value::Undefined);
            let target_obj = interp.to_object(&target)?;
            for src in args.iter().skip(1) {
                if src.is_nullish() { continue; }
                let so = interp.to_object(src)?;
                let mut keys = Vec::new();
                collect_own_keys(&so, &mut keys, true, false);
                for k in keys {
                    let v = interp.get_property(&so, &k)?;
                    interp.set_property(&target_obj, &k, v)?;
                }
            }
            Ok(target_obj)
        }));
        def_static(realm, &co, "create", 2, Rc::new(|interp, _this, args| {
            let proto = args.get(0).cloned().unwrap_or(Value::Undefined);
            let o = ObjectInner::new_object();
            if matches!(proto, Value::Null) {
                o.borrow_mut().proto = None;
            } else if let Value::Object(p) = &proto {
                o.borrow_mut().proto = Some(Value::Object(p.clone()));
            } else {
                return Err(error::throw_type("Object prototype may only be an Object or null"));
            }
            // properties
            if let Some(props) = args.get(1) {
                if !props.is_undefined() {
                    let mut keys = Vec::new();
                    collect_own_keys(props, &mut keys, true, false);
                    for k in keys {
                        let desc = interp.get_property(props, &k)?;
                        let val = interp.get_property(&desc, &PropKey::from_str("value"))?;
                        o.borrow_mut().props.insert(k, Property::data(val));
                    }
                }
            }
            Ok(Value::Object(o))
        }));
        def_static(realm, &co, "freeze", 1, Rc::new(|_interp, _this, args| {
            if let Some(Value::Object(o)) = args.get(0) {
                let mut b = o.borrow_mut();
                b.extensible = false;
                for (_, p) in b.props.iter_mut() {
                    p.writable = false;
                    p.configurable = false;
                }
            }
            Ok(args.get(0).cloned().unwrap_or(Value::Undefined))
        }));
        def_static(realm, &co, "isFrozen", 1, Rc::new(|_interp, _this, args| {
            if let Some(Value::Object(o)) = args.get(0) {
                let b = o.borrow();
                if b.extensible { return Ok(Value::Bool(false)); }
                for (_, p) in b.props.iter() {
                    if p.configurable || (matches!(p.kind, PropKind::Data(_)) && p.writable) {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            } else {
                Ok(Value::Bool(true))
            }
        }));
        def_static(realm, &co, "seal", 1, Rc::new(|_interp, _this, args| {
            if let Some(Value::Object(o)) = args.get(0) {
                let mut b = o.borrow_mut();
                b.extensible = false;
                for (_, p) in b.props.iter_mut() {
                    p.configurable = false;
                }
            }
            Ok(args.get(0).cloned().unwrap_or(Value::Undefined))
        }));
        def_static(realm, &co, "isSealed", 1, Rc::new(|_interp, _this, args| {
            if let Some(Value::Object(o)) = args.get(0) {
                let b = o.borrow();
                if b.extensible { return Ok(Value::Bool(false)); }
                for (_, p) in b.props.iter() {
                    if p.configurable { return Ok(Value::Bool(false)); }
                }
                Ok(Value::Bool(true))
            } else { Ok(Value::Bool(true)) }
        }));
        def_static(realm, &co, "preventExtensions", 1, Rc::new(|_interp, _this, args| {
            if let Some(Value::Object(o)) = args.get(0) {
                o.borrow_mut().extensible = false;
            }
            Ok(args.get(0).cloned().unwrap_or(Value::Undefined))
        }));
        def_static(realm, &co, "isExtensible", 1, Rc::new(|_interp, _this, args| {
            if let Some(Value::Object(o)) = args.get(0) {
                Ok(Value::Bool(o.borrow().extensible))
            } else { Ok(Value::Bool(false)) }
        }));
        def_static(realm, &co, "getPrototypeOf", 1, Rc::new(|interp, _this, args| {
            let obj = args.get(0).cloned().unwrap_or(Value::Undefined);
            interp.get_prototype_of(&obj)
        }));
        def_static(realm, &co, "setPrototypeOf", 2, Rc::new(|_interp, _this, args| {
            if let Some(Value::Object(o)) = args.get(0) {
                let p = args.get(1).cloned().unwrap_or(Value::Undefined);
                if matches!(p, Value::Null) {
                    o.borrow_mut().proto = None;
                } else if let Value::Object(po) = &p {
                    o.borrow_mut().proto = Some(Value::Object(po.clone()));
                }
            }
            Ok(args.get(0).cloned().unwrap_or(Value::Undefined))
        }));
        def_static(realm, &co, "defineProperty", 3, Rc::new(|interp, _this, args| {
            let obj = args.get(0).cloned().unwrap_or(Value::Undefined);
            let key = to_property_key(args.get(1).unwrap_or(&Value::Undefined));
            let desc = args.get(2).cloned().unwrap_or(Value::Undefined);
            // Proxy "defineProperty" trap
            if let Value::Object(o) = &obj {
                if matches!(o.borrow().kind, ObjectKind::Proxy(_)) {
                    let pd = if let ObjectKind::Proxy(pd) = &o.borrow().kind { pd.clone() } else { unreachable!() };
                    if pd.revoked {
                        return Err(error::throw_type("Cannot perform 'defineProperty' on a proxy that has been revoked"));
                    }
                    let trap = interp.get_property(&pd.handler, &PropKey::from_str("defineProperty"))?;
                    if trap.is_callable() {
                        let key_val = match &key {
                            PropKey::Str(s) => Value::String(s.clone()),
                            PropKey::Sym(s) => Value::Symbol(s.clone()),
                        };
                        interp.call_value(trap, pd.handler.clone(), &[pd.target.clone(), key_val, desc])?;
                        return Ok(obj);
                    }
                    // default: forward to target
                    if let Value::Object(to) = &pd.target {
                        let prop = desc_to_property(&desc);
                        match prop {
                            Some(p) => { to.borrow_mut().props.insert(key, p); }
                            None => { to.borrow_mut().props.remove(&key); }
                        }
                    }
                    return Ok(obj);
                }
            }
            if let Value::Object(o) = &obj {
                let prop = desc_to_property(&desc);
                match prop {
                    Some(p) => { o.borrow_mut().props.insert(key, p); }
                    None => { o.borrow_mut().props.remove(&key); }
                }
            }
            Ok(obj)
        }));
        def_static(realm, &co, "defineProperties", 2, Rc::new(|interp, _this, args| {
            let obj = args.get(0).cloned().unwrap_or(Value::Undefined);
            let props = args.get(1).cloned().unwrap_or(Value::Undefined);
            let mut keys = Vec::new();
            collect_own_keys(&props, &mut keys, true, false);
            for k in keys {
                let desc = interp.get_property(&props, &k)?;
                let prop = desc_to_property(&desc);
                if let Value::Object(o) = &obj {
                    match prop {
                        Some(p) => { o.borrow_mut().props.insert(k, p); }
                        None => { o.borrow_mut().props.remove(&k); }
                    }
                }
            }
            Ok(obj)
        }));
        def_static(realm, &co, "getOwnPropertyDescriptor", 2, Rc::new(|interp, _this, args| {
            let obj = args.get(0).cloned().unwrap_or(Value::Undefined);
            let key = to_property_key(args.get(1).unwrap_or(&Value::Undefined));
            if let Value::Object(o) = &obj {
                if let Some(p) = o.borrow().props.get(&key).cloned() {
                    return Ok(property_to_desc(interp, &p));
                }
            }
            Ok(Value::Undefined)
        }));
        def_static(realm, &co, "getOwnPropertyDescriptors", 1, Rc::new(|interp, _this, args| {
            let obj = args.get(0).cloned().unwrap_or(Value::Undefined);
            let result = ObjectInner::new_object();
            result.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
            if let Value::Object(o) = &obj {
                let keys: Vec<PropKey> = {
                    let b = o.borrow();
                    let mut ks = Vec::new();
                    collect_own_keys(&Value::Object(o.clone()), &mut ks, false, true);
                    ks
                };
                for k in keys {
                    if let Some(p) = o.borrow().props.get(&k).cloned() {
                        let d = property_to_desc(interp, &p);
                        result.borrow_mut().props.insert(k, Property::data(d));
                    }
                }
            }
            Ok(Value::Object(result))
        }));
        def_static(realm, &co, "getOwnPropertyNames", 1, Rc::new(|_interp, _this, args| {
            let obj = args.get(0).cloned().unwrap_or(Value::Undefined);
            let mut keys = Vec::new();
            collect_own_keys(&obj, &mut keys, false, true);
            Ok(keys_to_array(&keys))
        }));
        def_static(realm, &co, "getOwnPropertySymbols", 1, Rc::new(|_interp, _this, args| {
            let obj = args.get(0).cloned().unwrap_or(Value::Undefined);
            let mut syms = Vec::new();
            if let Value::Object(o) = &obj {
                let b = o.borrow();
                for (k, _) in b.props.iter() {
                    if let PropKey::Sym(s) = k { syms.push(Value::Symbol(s.clone())); }
                }
            }
            // can't build array without interp; approximate by returning a Vec via wrapper
            // We'll attach as object. (Callers usually iterate.)
            let arr = ObjectInner::new_array(syms);
            Ok(Value::Object(arr))
        }));
        def_static(realm, &co, "hasOwn", 2, Rc::new(|interp, _this, args| {
            let obj = args.get(0).cloned().unwrap_or(Value::Undefined);
            let key = to_property_key(args.get(1).unwrap_or(&Value::Undefined));
            let o = interp.to_object(&obj)?;
            let has = if let Value::Object(oo) = &o { oo.borrow().props.contains_key(&key) } else { false };
            Ok(Value::Bool(has))
        }));
        def_static(realm, &co, "is", 2, Rc::new(|_interp, _this, args| {
            Ok(Value::Bool(same_value(args.get(0).unwrap_or(&Value::Undefined), args.get(1).unwrap_or(&Value::Undefined))))
        }));
        // ES2024 Object.groupBy
        def_static(realm, &co, "groupBy", 2, Rc::new(|interp, _this, args| {
            let it = args.get(0).cloned().unwrap_or(Value::Undefined);
            let cb = args.get(1).cloned().unwrap_or(Value::Undefined);
            let items = interp.iterable_to_vec(&it)?;
            let result = ObjectInner::new_object();
            result.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
            for (i, item) in items.iter().enumerate() {
                let key = interp.call_value(cb.clone(), Value::Undefined, &[item.clone(), Value::from_int(i as i32)])?;
                let key_str = match &key {
                    Value::String(s) => s.to_string(),
                    Value::Number(n) => crate::value::format_number(*n),
                    Value::Undefined => "undefined".to_string(),
                    Value::Null => "null".to_string(),
                    _ => crate::value::to_string(&key),
                };
                let pk = PropKey::from_str(&key_str);
                if !result.borrow().props.contains_key(&pk) {
                    let a = ObjectInner::new_array(vec![]);
                    a.borrow_mut().proto = Some(Value::Object(interp.realm().array_proto.clone()));
                    result.borrow_mut().props.insert(pk.clone(), Property::data(Value::Object(a)));
                }
                let arr_val = result.borrow().props.get(&pk).cloned();
                if let Some(Property { kind: PropKind::Data(Value::Object(o)), .. }) = arr_val {
                    if let ObjectKind::Array(arr_items) = &mut o.borrow_mut().kind {
                        arr_items.push(item.clone());
                    }
                }
            }
            Ok(Value::Object(result))
        }));
    }
    let _ = interp;
}

fn def_static(realm: &Rc<Realm>, obj: &ObjRef, name: &str, len: usize, f: NativeFn) {
    def_method(realm, obj, name, len, f);
}

fn collect_own_keys(v: &Value, out: &mut Vec<PropKey>, enumerable_only: bool, include_array_indices: bool) {
    if let Value::Object(o) = v {
        let b = o.borrow();
        if include_array_indices {
            if let ObjectKind::Array(items) = &b.kind {
                for i in 0..items.len() {
                    out.push(PropKey::Str(crate::value::index_to_key(i)));
                }
            }
        }
        for (k, p) in b.props.iter() {
            if enumerable_only && !p.enumerable { continue; }
            out.push(k.clone());
        }
    }
}

fn value_from_key(k: &PropKey) -> Value {
    match k {
        PropKey::Str(s) => Value::String(s.clone()),
        PropKey::Sym(s) => Value::Symbol(s.clone()),
    }
}

fn keys_to_array(keys: &[PropKey]) -> Value {
    let items: Vec<Value> = keys.iter().map(value_from_key).collect();
    let o = ObjectInner::new_array(items);
    Value::Object(o)
}

fn desc_to_property(desc: &Value) -> Option<Property> {
    let o = if let Value::Object(o) = desc { o } else { return None; };
    let b = o.borrow();
    let has_value = b.props.contains_key(&PropKey::from_str("value"));
    let has_get = b.props.contains_key(&PropKey::from_str("get"));
    let has_set = b.props.contains_key(&PropKey::from_str("set"));
    let writable = b.props.get(&PropKey::from_str("writable")).map(bool_of).unwrap_or(false);
    let enumerable = b.props.get(&PropKey::from_str("enumerable")).map(bool_of).unwrap_or(false);
    let configurable = b.props.get(&PropKey::from_str("configurable")).map(bool_of).unwrap_or(false);
    if has_get || has_set {
        let get = b.props.get(&PropKey::from_str("get")).and_then(|p| if let PropKind::Data(v) = &p.kind { Some(v.clone()) } else { None });
        let set = b.props.get(&PropKey::from_str("set")).and_then(|p| if let PropKind::Data(v) = &p.kind { Some(v.clone()) } else { None });
        Some(Property { kind: PropKind::Accessor { get, set }, writable: false, enumerable, configurable })
    } else if has_value {
        let val = b.props.get(&PropKey::from_str("value")).and_then(|p| if let PropKind::Data(v) = &p.kind { Some(v.clone()) } else { None }).unwrap_or(Value::Undefined);
        Some(Property { kind: PropKind::Data(val), writable, enumerable, configurable })
    } else {
        Some(Property { kind: PropKind::Data(Value::Undefined), writable, enumerable, configurable })
    }
}

fn bool_of(p: &Property) -> bool {
    if let PropKind::Data(Value::Bool(b)) = &p.kind { *b } else { false }
}

fn property_to_desc(interp: &mut Interpreter, p: &Property) -> Value {
    let o = ObjectInner::new_object();
    o.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
    match &p.kind {
        PropKind::Data(v) => {
            o.borrow_mut().props.insert(PropKey::from_str("value"), Property::data(v.clone()));
            o.borrow_mut().props.insert(PropKey::from_str("writable"), Property::data(Value::Bool(p.writable)));
        }
        PropKind::Accessor { get, set } => {
            o.borrow_mut().props.insert(PropKey::from_str("get"), Property::data(get.clone().unwrap_or(Value::Undefined)));
            o.borrow_mut().props.insert(PropKey::from_str("set"), Property::data(set.clone().unwrap_or(Value::Undefined)));
        }
    }
    o.borrow_mut().props.insert(PropKey::from_str("enumerable"), Property::data(Value::Bool(p.enumerable)));
    o.borrow_mut().props.insert(PropKey::from_str("configurable"), Property::data(Value::Bool(p.configurable)));
    Value::Object(o)
}
