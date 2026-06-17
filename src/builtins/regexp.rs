//! RegExp constructor + RegExp.prototype.

use crate::realm::Realm;
use crate::error;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{make_ctor, install_global_ctor, def_method, CtorFn};
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let call_fn: NativeFn = Rc::new(|interp, _this, args| {
        make_re(interp, args.get(0).cloned().unwrap_or(Value::Undefined), args.get(1).cloned().unwrap_or(Value::Undefined))
    });
    let ctor_fn: CtorFn = { let cf = call_fn.clone(); Rc::new(move |interp, _t, args, _nt| cf(interp, Value::Undefined, args)) };
    let ctor = make_ctor(realm, "RegExp", 2, call_fn, ctor_fn);
    install_global_ctor(interp, realm, "RegExp", ctor, realm.regexp_proto.clone());
    let rp = realm.regexp_proto.clone();
    def_method(realm, &rp, "exec", 1, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(args.get(0).unwrap_or(&Value::Undefined))?;
        exec_on(interp, &this, &s)
    }));
    def_method(realm, &rp, "test", 1, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(args.get(0).unwrap_or(&Value::Undefined))?;
        let r = exec_on(interp, &this, &s)?;
        Ok(Value::Bool(!r.is_null()))
    }));
    def_method(realm, &rp, "toString", 0, Rc::new(|_i, this, _a| {
        if let Value::Object(o) = &this {
            let b = o.borrow();
            if let ObjectKind::RegExp(d) = &b.kind {
                return Ok(Value::from_string(format!("/{}/{}", d.source, d.flags)));
            }
        }
        Ok(Value::from_str("/(?:)/"))
    }));
    // Symbol.match / Symbol.replace
    let sm = crate::interp::make_native_value(realm, "[Symbol.match]", 1, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(args.get(0).unwrap_or(&Value::Undefined))?;
        match_all(interp, &this, &s)
    }));
    rp.borrow_mut().props.insert(PropKey::Sym(realm.wk.iterator.clone()), Property::data(sm.clone()));
    rp.borrow_mut().props.insert(PropKey::from_str("Symbol.match"), Property::data(sm));
    let sr = crate::interp::make_native_value(realm, "[Symbol.replace]", 2, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(args.get(0).unwrap_or(&Value::Undefined))?;
        let repl = args.get(1).cloned().unwrap_or(Value::Undefined);
        replace_with(interp, &this, &s, &repl)
    }));
    rp.borrow_mut().props.insert(PropKey::from_str("Symbol.replace"), Property::data(sr));
    let _ = interp;
}

fn make_re(interp: &mut Interpreter, pattern: Value, flags: Value) -> Result<Value, Value> {
    // if already a regexp
    if let Value::Object(o) = &pattern {
        if matches!(o.borrow().kind, ObjectKind::RegExp(_)) {
            return Ok(pattern.clone());
        }
    }
    let p = interp.coerce_to_string(&pattern)?;
    let f = interp.coerce_to_string(&flags)?;
    let ctor = interp.get_property(&Value::Object(interp.realm().global.clone()), &PropKey::from_str("RegExp"))?;
    // delegate to the JS-level make_regexp via interpreter
    interp.construct(ctor.clone(), &[Value::String(p), Value::String(f)], ctor)
}

fn exec_on(interp: &mut Interpreter, re: &Value, s: &str) -> Result<Value, Value> {
    let data = if let Value::Object(o) = re {
        if let ObjectKind::RegExp(d) = &o.borrow().kind { d.clone() } else { return Err(error::throw_type("not a regexp")); }
    } else { return Err(error::throw_type("not a regexp")); };
    // Prefer fancy-regex (supports backrefs/lookaround) when available.
    let fancy_result = if let Some(fr) = &data.fancy {
        fr.captures(s).ok().flatten()
    } else {
        None
    };
    if let Some(caps) = fancy_result {
        let mut items = Vec::new();
        for i in 0..caps.len() {
            match caps.get(i) {
                Some(m) => items.push(Value::from_string(m.as_str().to_string())),
                None => items.push(Value::Undefined),
            }
        }
        let arr = interp.new_array(items);
        if let Value::Object(ao) = &arr {
            let start = caps.get(0).map(|m| m.start()).unwrap_or(0);
            ao.borrow_mut().props.insert(PropKey::from_str("index"), Property::data(Value::from_int(start as i32)));
            ao.borrow_mut().props.insert(PropKey::from_str("input"), Property::data(Value::from_string(s.to_string())));
        }
        if data.global {
            if let Value::Object(o) = re {
                if let ObjectKind::RegExp(d) = &o.borrow().kind {
                    let end = caps.get(0).map(|m| m.end()).unwrap_or(0);
                    d.last_index.set(end);
                }
            }
            if let Value::Object(o) = re {
                o.borrow_mut().props.insert(PropKey::from_str("lastIndex"), Property::data(Value::from_int(data.last_index.get() as i32)));
            }
        }
        return Ok(arr);
    }
    // Fallback: standard regex crate.
    if let Some(caps) = data.re.captures(s) {
        let mut items = Vec::new();
        for i in 0..caps.len() {
            match caps.get(i) {
                Some(m) => items.push(Value::from_string(m.as_str().to_string())),
                None => items.push(Value::Undefined),
            }
        }
        let arr = interp.new_array(items);
        if let Value::Object(ao) = &arr {
            let start = caps.get(0).map(|m| m.start()).unwrap_or(0);
            ao.borrow_mut().props.insert(PropKey::from_str("index"), Property::data(Value::from_int(start as i32)));
            ao.borrow_mut().props.insert(PropKey::from_str("input"), Property::data(Value::from_string(s.to_string())));
        }
        if data.global {
            if let Value::Object(o) = re {
                if let ObjectKind::RegExp(d) = &o.borrow().kind {
                    let end = caps.get(0).map(|m| m.end()).unwrap_or(0);
                    d.last_index.set(end);
                }
            }
            if let Value::Object(o) = re {
                o.borrow_mut().props.insert(PropKey::from_str("lastIndex"), Property::data(Value::from_int(data.last_index.get() as i32)));
            }
        }
        Ok(arr)
    } else {
        Ok(Value::Null)
    }
}

fn match_all(interp: &mut Interpreter, re: &Value, s: &str) -> Result<Value, Value> {
    let data = if let Value::Object(o) = re {
        if let ObjectKind::RegExp(d) = &o.borrow().kind { d.clone() } else { return Err(error::throw_type("not a regexp")); }
    } else { return Err(error::throw_type("not a regexp")); };
    let mut out = Vec::new();
    for m in data.re.find_iter(s) {
        out.push(Value::from_string(m.as_str().to_string()));
    }
    if out.is_empty() { Ok(Value::Null) } else { Ok(interp.new_array(out)) }
}

fn replace_with(interp: &mut Interpreter, re: &Value, s: &str, repl: &Value) -> Result<Value, Value> {
    let data = if let Value::Object(o) = re {
        if let ObjectKind::RegExp(d) = &o.borrow().kind { d.clone() } else { return Err(error::throw_type("not a regexp")); }
    } else { return Err(error::throw_type("not a regexp")); };
    let mut result = String::new();
    let mut last = 0;
    for m in data.re.find_iter(s) {
        result.push_str(&s[last..m.start()]);
        let matched = m.as_str();
        if repl.is_callable() {
            let r = interp.call_value(repl.clone(), Value::Undefined, &[Value::from_string(matched.to_string())])?;
            result.push_str(&interp.coerce_to_string(&r)?);
        } else {
            result.push_str(&interp.coerce_to_string(repl)?);
        }
        last = m.end();
        if !data.global { break; }
    }
    result.push_str(&s[last..]);
    Ok(Value::from_string(result))
}
