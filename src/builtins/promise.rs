//! Promise constructor + Promise.prototype.

use crate::realm::Realm;
use crate::error;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{make_ctor, install_global_ctor, def_method, CtorFn};
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let ctor_fn: CtorFn = Rc::new(|interp, _this, args, _nt| {
        let executor = args.get(0).cloned().unwrap_or(Value::Undefined);
        if !executor.is_callable() {
            return Err(error::throw_type("Promise resolver is not a function"));
        }
        let promise = interp.new_promise();
        let p_clone = promise.clone();
        let resolve = interp.make_native("resolve", 1, Rc::new(move |interp, _t, a| {
            interp.resolve_promise(p_clone.clone(), a.get(0).cloned().unwrap_or(Value::Undefined));
            Ok(Value::Undefined)
        }));
        let p_clone2 = promise.clone();
        let reject = interp.make_native("reject", 1, Rc::new(move |interp, _t, a| {
            interp.reject_promise(p_clone2.clone(), a.get(0).cloned().unwrap_or(Value::Undefined));
            Ok(Value::Undefined)
        }));
        match interp.call_value(executor, Value::Undefined, &[resolve, reject]) {
            Ok(_) => Ok(promise),
            Err(e) => {
                interp.reject_promise(promise.clone(), e);
                Ok(promise)
            }
        }
    });
    let call_fn: NativeFn = Rc::new(|interp, _t, args| {
        // Promise called as function behaves like constructor
        let ctor = interp.get_global("Promise");
        interp.construct(ctor.clone(), args, ctor)
    });
    let ctor = make_ctor(realm, "Promise", 1, call_fn, ctor_fn);
    install_global_ctor(interp, realm, "Promise", ctor.clone(), realm.promise_proto.clone());

    if let Value::Object(co) = &ctor {
        let co = co.clone();
        def_method(realm, &co, "resolve", 1, Rc::new(|interp, _t, args| {
            let v = args.get(0).cloned().unwrap_or(Value::Undefined);
            if let Value::Object(o) = &v {
                if matches!(o.borrow().kind, ObjectKind::Promise(_)) { return Ok(v); }
            }
            let p = interp.new_promise();
            interp.resolve_promise(p.clone(), v);
            Ok(p)
        }));
        def_method(realm, &co, "reject", 1, Rc::new(|interp, _t, args| {
            let v = args.get(0).cloned().unwrap_or(Value::Undefined);
            let p = interp.new_promise();
            interp.reject_promise(p.clone(), v);
            Ok(p)
        }));
        def_method(realm, &co, "all", 1, Rc::new(|interp, _t, args| {
            let it = args.get(0).cloned().unwrap_or(Value::Undefined);
            let items = interp.iterable_to_vec(&it)?;
            let items_empty = items.is_empty();
            let result = interp.new_promise();
            let results = Rc::new(std::cell::RefCell::new(vec![Value::Undefined; items.len()]));
            let remaining = Rc::new(std::cell::Cell::new(items.len()));
            let result_clone = result.clone();
            for (i, p) in items.into_iter().enumerate() {
                let r = interp.to_promise(p)?;
                let resolve = interp.make_native("r", 1, Rc::new({
                    let results = results.clone();
                    let remaining = remaining.clone();
                    let result_clone = result_clone.clone();
                    move |interp, _t, args| {
                        let v = args.get(0).cloned().unwrap_or(Value::Undefined);
                        results.borrow_mut()[i] = v;
                        let n = remaining.get();
                        if n <= 1 {
                            let arr = interp.new_array(results.borrow().clone());
                            interp.resolve_promise(result_clone.clone(), arr);
                        } else {
                            remaining.set(n - 1);
                        }
                        Ok(Value::Undefined)
                    }
                }));
                let reject = interp.make_native("r", 1, Rc::new({
                    let result_clone = result_clone.clone();
                    move |interp, _t, args| {
                        interp.reject_promise(result_clone.clone(), args.get(0).cloned().unwrap_or(Value::Undefined));
                        Ok(Value::Undefined)
                    }
                }));
                let then = interp.get_property(&r, &PropKey::from_str("then"))?;
                interp.call_value(then, r, &[resolve, reject])?;
            }
            if items_empty {
                interp.resolve_promise(result.clone(), interp.new_array(vec![]));
            }
            Ok(result)
        }));
        def_method(realm, &co, "allSettled", 1, Rc::new(|interp, _t, args| {
            let it = args.get(0).cloned().unwrap_or(Value::Undefined);
            let items = interp.iterable_to_vec(&it)?;
            let items_empty = items.is_empty();
            let result = interp.new_promise();
            let results = Rc::new(std::cell::RefCell::new(vec![Value::Undefined; items.len()]));
            let remaining = Rc::new(std::cell::Cell::new(items.len()));
            let result_clone = result.clone();
            for (i, p) in items.into_iter().enumerate() {
                let r = interp.to_promise(p)?;
                let on_ok = interp.make_native("ok", 1, Rc::new({
                    let results = results.clone(); let remaining = remaining.clone(); let result_clone = result_clone.clone();
                    move |interp, _t, args| {
                        let o = ObjectInner::new_object();
                        o.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
                        o.borrow_mut().props.insert(PropKey::from_str("status"), Property::data(Value::from_str("fulfilled")));
                        o.borrow_mut().props.insert(PropKey::from_str("value"), Property::data(args.get(0).cloned().unwrap_or(Value::Undefined)));
                        results.borrow_mut()[i] = Value::Object(o);
                        let n = remaining.get(); if n <= 1 { interp.resolve_promise(result_clone.clone(), interp.new_array(results.borrow().clone())); } else { remaining.set(n - 1); }
                        Ok(Value::Undefined)
                    }
                }));
                let on_err = interp.make_native("err", 1, Rc::new({
                    let results = results.clone(); let remaining = remaining.clone(); let result_clone = result_clone.clone();
                    move |interp, _t, args| {
                        let o = ObjectInner::new_object();
                        o.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
                        o.borrow_mut().props.insert(PropKey::from_str("status"), Property::data(Value::from_str("rejected")));
                        o.borrow_mut().props.insert(PropKey::from_str("reason"), Property::data(args.get(0).cloned().unwrap_or(Value::Undefined)));
                        results.borrow_mut()[i] = Value::Object(o);
                        let n = remaining.get(); if n <= 1 { interp.resolve_promise(result_clone.clone(), interp.new_array(results.borrow().clone())); } else { remaining.set(n - 1); }
                        Ok(Value::Undefined)
                    }
                }));
                let then = interp.get_property(&r, &PropKey::from_str("then"))?;
                interp.call_value(then, r, &[on_ok, on_err])?;
            }
            if items_empty { interp.resolve_promise(result.clone(), interp.new_array(vec![])); }
            Ok(result)
        }));
        def_method(realm, &co, "race", 1, Rc::new(|interp, _t, args| {
            let it = args.get(0).cloned().unwrap_or(Value::Undefined);
            let items = interp.iterable_to_vec(&it)?;
            let result = interp.new_promise();
            let rc = result.clone();
            for p in items {
                let r = interp.to_promise(p)?;
                let on_ok = interp.make_native("ok", 1, Rc::new({ let rc = rc.clone(); move |interp, _t, a| { interp.resolve_promise(rc.clone(), a.get(0).cloned().unwrap_or(Value::Undefined)); Ok(Value::Undefined) } }));
                let on_err = interp.make_native("err", 1, Rc::new({ let rc = rc.clone(); move |interp, _t, a| { interp.reject_promise(rc.clone(), a.get(0).cloned().unwrap_or(Value::Undefined)); Ok(Value::Undefined) } }));
                let then = interp.get_property(&r, &PropKey::from_str("then"))?;
                interp.call_value(then, r, &[on_ok, on_err])?;
            }
            Ok(result)
        }));
        def_method(realm, &co, "any", 1, Rc::new(|interp, _t, args| {
            let it = args.get(0).cloned().unwrap_or(Value::Undefined);
            let items = interp.iterable_to_vec(&it)?;
            let result = interp.new_promise();
            let errors = Rc::new(std::cell::RefCell::new(vec![Value::Undefined; items.len()]));
            let remaining = Rc::new(std::cell::Cell::new(items.len()));
            let rc = result.clone();
            for (i, p) in items.into_iter().enumerate() {
                let r = interp.to_promise(p)?;
                let on_ok = interp.make_native("ok", 1, Rc::new({ let rc = rc.clone(); move |interp, _t, a| { interp.resolve_promise(rc.clone(), a.get(0).cloned().unwrap_or(Value::Undefined)); Ok(Value::Undefined) } }));
                let on_err = interp.make_native("err", 1, Rc::new({ let errors = errors.clone(); let remaining = remaining.clone(); let rc = rc.clone(); move |interp, _t, a| {
                    errors.borrow_mut()[i] = a.get(0).cloned().unwrap_or(Value::Undefined);
                    let n = remaining.get(); if n <= 1 { let arr = interp.new_array(errors.borrow().clone()); interp.reject_promise(rc.clone(), arr); } else { remaining.set(n - 1); }
                    Ok(Value::Undefined)
                }}));
                let then = interp.get_property(&r, &PropKey::from_str("then"))?;
                interp.call_value(then, r, &[on_ok, on_err])?;
            }
            Ok(result)
        }));
    }
    let pp = realm.promise_proto.clone();
    def_method(realm, &pp, "then", 2, Rc::new(|interp, this, args| {
        let on_fulfilled = args.get(0).cloned().unwrap_or(Value::Undefined);
        let on_rejected = args.get(1).cloned().unwrap_or(Value::Undefined);
        let derived = interp.new_promise();
        let reaction_resolve = derived.clone();
        let reaction_reject = derived.clone();
        let resolve_fn = interp.make_native("resolve", 1, Rc::new(move |interp, _t, a| { interp.resolve_promise(reaction_resolve.clone(), a.get(0).cloned().unwrap_or(Value::Undefined)); Ok(Value::Undefined) }));
        let reject_fn = interp.make_native("reject", 1, Rc::new(move |interp, _t, a| { interp.reject_promise(reaction_reject.clone(), a.get(0).cloned().unwrap_or(Value::Undefined)); Ok(Value::Undefined) }));
        // wrap handlers to forward into derived promise
        let derived2 = derived.clone();
        let on_f = if on_fulfilled.is_callable() {
            interp.make_native("onFulfilled", 1, Rc::new(move |interp, _t, a| {
                match interp.call_value(on_fulfilled.clone(), Value::Undefined, &[a.get(0).cloned().unwrap_or(Value::Undefined)]) {
                    Ok(v) => { interp.resolve_promise(derived2.clone(), v); }
                    Err(e) => { interp.reject_promise(derived2.clone(), e); }
                }
                Ok(Value::Undefined)
            }))
        } else { resolve_fn.clone() };
        let derived3 = derived.clone();
        let on_r = if on_rejected.is_callable() {
            interp.make_native("onRejected", 1, Rc::new(move |interp, _t, a| {
                match interp.call_value(on_rejected.clone(), Value::Undefined, &[a.get(0).cloned().unwrap_or(Value::Undefined)]) {
                    Ok(v) => { interp.resolve_promise(derived3.clone(), v); }
                    Err(e) => { interp.reject_promise(derived3.clone(), e); }
                }
                Ok(Value::Undefined)
            }))
        } else { reject_fn.clone() };
        // register reactions on this promise
        if let Value::Object(o) = &this {
            if let ObjectKind::Promise(s) = &o.borrow().kind {
                let mut b = s.borrow_mut();
                let already_fulfilled = matches!(b.state, PromiseStatus::Fulfilled);
                let already_rejected = matches!(b.state, PromiseStatus::Rejected);
                // Mark that a rejection handler has been attached (suppresses the
                // spurious unhandled-rejection report).
                b.handled = true;
                if matches!(b.state, PromiseStatus::Pending) {
                    b.fulfill_reactions.push(Reaction { handler: on_f, resolve: resolve_fn.clone(), reject: reject_fn.clone() });
                    b.reject_reactions.push(Reaction { handler: on_r, resolve: resolve_fn, reject: reject_fn });
                } else if already_fulfilled {
                    let val = b.value.clone();
                    drop(b);
                    let derived_f = derived.clone();
                    let on_f_clone = on_f.clone();
                    let resolve_f = resolve_fn.clone();
                    asyncrt_queue(interp, move |interp| {
                        let r = Reaction { handler: on_f_clone, resolve: resolve_f, reject: Value::Undefined };
                        run_reaction_inline(interp, r, true, val, derived_f);
                    });
                } else if already_rejected {
                    let val = b.value.clone();
                    drop(b);
                    let derived_r = derived.clone();
                    let on_r_clone = on_r.clone();
                    let reject_f = reject_fn.clone();
                    asyncrt_queue(interp, move |interp| {
                        let r = Reaction { handler: on_r_clone, resolve: Value::Undefined, reject: reject_f };
                        run_reaction_inline(interp, r, false, val, derived_r);
                    });
                }
            }
        }
        Ok(derived)
    }));
    def_method(realm, &pp, "catch", 1, Rc::new(|interp, this, args| {
        let on_rejected = args.get(0).cloned().unwrap_or(Value::Undefined);
        let then = interp.get_property(&this, &PropKey::from_str("then"))?;
        interp.call_value(then, this, &[Value::Undefined, on_rejected])
    }));
    def_method(realm, &pp, "finally", 1, Rc::new(|interp, this, args| {
        let on_finally = args.get(0).cloned().unwrap_or(Value::Undefined);
        let then = interp.get_property(&this, &PropKey::from_str("then"))?;
        let f1 = on_finally.clone();
        let on_f = interp.make_native("f", 0, Rc::new(move |interp, _t, _a| { if f1.is_callable() { let _ = interp.call_value(f1.clone(), Value::Undefined, &[]); } Ok(Value::Undefined) }));
        let f2 = on_finally;
        let on_r = interp.make_native("f", 0, Rc::new(move |interp, _t, _a| { if f2.is_callable() { let _ = interp.call_value(f2.clone(), Value::Undefined, &[]); } Ok(Value::Undefined) }));
        interp.call_value(then, this, &[on_f, on_r])
    }));
    let _ = interp;
}

fn asyncrt_queue(interp: &Interpreter, f: impl FnOnce(&mut Interpreter) + 'static) {
    let rt = interp.shared.async_rt.clone();
    crate::asyncrt::queue_microtask(&rt, Box::new(f));
}

fn run_reaction_inline(interp: &mut Interpreter, reaction: Reaction, fulfilled: bool, value: Value, _derived: Value) {
    let Reaction { handler, resolve, reject } = reaction;
    if handler.is_callable() {
        match interp.call_value(handler, Value::Undefined, &[value]) {
            Ok(v) => { if resolve.is_callable() { let _ = interp.call_value(resolve, Value::Undefined, &[v]); } }
            Err(e) => { if reject.is_callable() { let _ = interp.call_value(reject, Value::Undefined, &[e]); } }
        }
    } else if fulfilled {
        if resolve.is_callable() { let _ = interp.call_value(resolve, Value::Undefined, &[value]); }
    } else {
        if reject.is_callable() { let _ = interp.call_value(reject, Value::Undefined, &[value]); }
    }
}
