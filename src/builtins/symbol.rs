//! Symbol constructor + Symbol.prototype + well-known symbols.

use crate::realm::Realm;
use crate::error;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{make_ctor, install_global_ctor, def_method, def_const_value, CtorFn};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    static SYMBOL_REGISTRY: RefCell<HashMap<String, Rc<Symbol>>> = RefCell::new(HashMap::new());
}

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let call_fn: NativeFn = Rc::new({
        let realm = realm.clone();
        move |_interp, _this, args| {
            let desc = match args.get(0) {
                Some(Value::Undefined) | None => None,
                Some(v) => Some(Rc::from(crate::value::to_string(v).as_str())),
            };
            Ok(Value::Symbol(realm.new_symbol(desc)))
        }
    });
    let ctor_fn: CtorFn = Rc::new(|_i, _t, _a, _nt| Err(error::throw_type("Symbol is not a constructor")));
    let ctor = make_ctor(realm, "Symbol", 0, call_fn, ctor_fn);
    install_global_ctor(interp, realm, "Symbol", ctor.clone(), realm.symbol_proto.clone());
    if let Value::Object(co) = &ctor {
        let co = co.clone();
        def_method(realm, &co, "for", 1, Rc::new(|interp, _this, args| {
            let key = interp.coerce_to_string(args.get(0).unwrap_or(&Value::Undefined))?.to_string();
            let sym = SYMBOL_REGISTRY.with(|r| {
                let mut b = r.borrow_mut();
                b.entry(key.clone()).or_insert_with(|| Rc::new(Symbol { description: Some(Rc::from(key.as_str())), id: 0 })).clone()
            });
            Ok(Value::Symbol(sym))
        }));
        def_method(realm, &co, "keyFor", 1, Rc::new(|_i, _this, args| {
            if let Some(Value::Symbol(s)) = args.get(0) {
                let key = SYMBOL_REGISTRY.with(|r| {
                    r.borrow().iter().find(|(_, v)| Rc::ptr_eq(v, s)).map(|(k, _)| k.clone())
                });
                return Ok(match key { Some(k) => Value::from_string(k), None => Value::Undefined });
            }
            Ok(Value::Undefined)
        }));
        // well-known
        def_const_value(&co, "iterator", Value::Symbol(realm.wk.iterator.clone()));
        def_const_value(&co, "asyncIterator", Value::Symbol(realm.wk.async_iterator.clone()));
        def_const_value(&co, "hasInstance", Value::Symbol(realm.wk.has_instance.clone()));
        def_const_value(&co, "toPrimitive", Value::Symbol(realm.wk.to_primitive.clone()));
        def_const_value(&co, "toStringTag", Value::Symbol(realm.wk.to_string_tag.clone()));
        def_const_value(&co, "isConcatSpreadable", Value::Symbol(realm.wk.is_concat_spreadable.clone()));
    }
    let sp = realm.symbol_proto.clone();
    def_method(realm, &sp, "toString", 0, Rc::new(|_i, this, _a| {
        let s = sym_of(&this);
        let desc = s.description.as_deref().unwrap_or("");
        Ok(Value::from_string(format!("Symbol({})", desc)))
    }));
    def_method(realm, &sp, "valueOf", 0, Rc::new(|_i, this, _a| Ok(Value::Symbol(sym_of(&this)))));
    def_method(realm, &sp, "description", 0, Rc::new(|_i, this, _a| {
        let s = sym_of(&this);
        Ok(match &s.description { Some(d) => Value::String(d.clone()), None => Value::Undefined })
    }));
    // Symbol.prototype[Symbol.toPrimitive] returns the symbol
    let tp = Rc::new(|_i: &mut Interpreter, this: Value, _a: &[Value]| Ok(Value::Symbol(sym_of(&this))));
    sp.borrow_mut().props.insert(PropKey::Sym(realm.wk.to_primitive.clone()), Property::data(make_native_value(realm, "[toPrimitive]", 0, tp)));
    let _ = interp;
}

fn sym_of(v: &Value) -> Rc<Symbol> {
    match v {
        Value::Symbol(s) => s.clone(),
        Value::Object(o) => {
            if let ObjectKind::Symbol(s) = &o.borrow().kind { s.clone() }
            else { Rc::new(Symbol { description: None, id: 0 }) }
        }
        _ => Rc::new(Symbol { description: None, id: 0 }),
    }
}

fn make_native_value(realm: &Rc<Realm>, name: &str, len: usize, f: NativeFn) -> Value {
    crate::interp::make_native_value(realm, name, len, f)
}
