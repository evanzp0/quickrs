//! Array constructor + Array.prototype.

use crate::realm::Realm;
use crate::error;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{make_ctor, install_global_ctor, def_method, CtorFn};
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let call_fn: NativeFn = Rc::new(|interp, _this, args| {
        if args.len() == 1 {
            if let Some(Value::Number(n)) = args.get(0) {
                if n.fract() == 0.0 && *n >= 0.0 && *n < 4294967296.0 {
                    let items = vec![Value::Undefined; *n as usize];
                    return Ok(interp.new_array(items));
                }
                if n.fract() != 0.0 || *n < 0.0 {
                    return Err(error::throw_range("Invalid array length"));
                }
            }
        }
        Ok(interp.new_array(args.to_vec()))
    });
    let ctor_fn: CtorFn = Rc::new(|interp, _this, args, _nt| {
        if args.len() == 1 {
            if let Some(Value::Number(n)) = args.get(0) {
                if n.fract() == 0.0 && *n >= 0.0 && *n < 4294967296.0 {
                    let items = vec![Value::Undefined; *n as usize];
                    return Ok(interp.new_array(items));
                }
                if n.fract() != 0.0 || *n < 0.0 {
                    return Err(error::throw_range("Invalid array length"));
                }
            }
        }
        Ok(interp.new_array(args.to_vec()))
    });
    let ctor = make_ctor(realm, "Array", 1, call_fn, ctor_fn);
    install_global_ctor(interp, realm, "Array", ctor.clone(), realm.array_proto.clone());

    // static
    if let Value::Object(co) = &ctor {
        let co = co.clone();
        def_method(realm, &co, "isArray", 1, Rc::new(|_interp, _this, args| {
            if let Some(Value::Object(o)) = args.get(0) {
                if matches!(o.borrow().kind, ObjectKind::Array(_)) { return Ok(Value::Bool(true)); }
            }
            Ok(Value::Bool(false))
        }));
        def_method(realm, &co, "from", 1, Rc::new(|interp, _this, args| {
            let src = args.get(0).cloned().unwrap_or(Value::Undefined);
            let map = args.get(1).cloned();
            // Try iterable first; fall back to array-like (object with `length`).
            let items = if interp.is_iterable(&src) || matches!(src, Value::String(_)) {
                interp.iterable_to_vec(&src)?
            } else {
                let len_v = interp.get_property(&src, &PropKey::from_str("length"))?;
                let len = to_length(&len_v);
                let mut v = Vec::with_capacity(len);
                for i in 0..len {
                    v.push(interp.get_property(&src, &PropKey::Str(crate::value::index_to_key(i)))?);
                }
                v
            };
            let out: Vec<Value> = if let Some(m) = map {
                if m.is_callable() {
                    let mut v = Vec::new();
                    for (i, it) in items.into_iter().enumerate() {
                        v.push(interp.call_value(m.clone(), Value::Undefined, &[it, Value::from_int(i as i32)])?);
                    }
                    v
                } else { items }
            } else { items };
            Ok(interp.new_array(out))
        }));
        def_method(realm, &co, "of", 0, Rc::new(|interp, _this, args| {
            Ok(interp.new_array(args.to_vec()))
        }));
        // Array.fromAsync (ES2024) — returns a Promise of an array
        def_method(realm, &co, "fromAsync", 1, Rc::new(|interp, _this, args| {
            let src = args.get(0).cloned().unwrap_or(Value::Undefined);
            let promise = interp.new_promise();
            let p_clone = promise.clone();
            let items = interp.iterable_to_vec(&src).unwrap_or_default();
            let arr = interp.new_array(items);
            interp.resolve_promise(p_clone, arr);
            Ok(promise)
        }));
    }

    let ap = realm.array_proto.clone();
    def_method(realm, &ap, "push", 1, Rc::new(|_interp, this, args| {
        if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &mut o.borrow_mut().kind {
                for a in args { items.push(a.clone()); }
                return Ok(Value::from_int(items.len() as i32));
            }
        }
        Ok(Value::from_int(0))
    }));
    def_method(realm, &ap, "pop", 0, Rc::new(|_interp, this, _args| {
        if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &mut o.borrow_mut().kind {
                return Ok(items.pop().unwrap_or(Value::Undefined));
            }
        }
        Ok(Value::Undefined)
    }));
    def_method(realm, &ap, "shift", 0, Rc::new(|_interp, this, _args| {
        if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &mut o.borrow_mut().kind {
                if items.is_empty() { return Ok(Value::Undefined); }
                return Ok(items.remove(0));
            }
        }
        Ok(Value::Undefined)
    }));
    def_method(realm, &ap, "unshift", 1, Rc::new(|_interp, this, args| {
        if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &mut o.borrow_mut().kind {
                for (i, a) in args.iter().enumerate() {
                    items.insert(i, a.clone());
                }
                return Ok(Value::from_int(items.len() as i32));
            }
        }
        Ok(Value::from_int(0))
    }));
    def_method(realm, &ap, "concat", 1, Rc::new(|interp, this, args| {
        let mut out = Vec::new();
        interp.flatten_into(&mut out, &this)?;
        for a in args {
            // spread arrays
            if let Value::Object(o) = a {
                if matches!(o.borrow().kind, ObjectKind::Array(_)) {
                    interp.flatten_into(&mut out, a)?;
                    continue;
                }
            }
            out.push(a.clone());
        }
        Ok(interp.new_array(out))
    }));
    def_method(realm, &ap, "join", 1, Rc::new(|interp, this, args| {
        let sep = match args.get(0) {
            Some(Value::Undefined) | None => ",".to_string(),
            Some(v) => interp.coerce_to_string(v)?.to_string(),
        };
        let items = interp.iterable_to_vec(&this)?;
        let parts: Vec<String> = items.iter().map(|v| {
            if v.is_nullish() { String::new() } else { crate::value::to_string(v) }
        }).collect();
        Ok(Value::from_string(parts.join(&sep)))
    }));
    def_method(realm, &ap, "reverse", 0, Rc::new(|_interp, this, _args| {
        if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &mut o.borrow_mut().kind {
                items.reverse();
            }
        }
        Ok(this)
    }));
    def_method(realm, &ap, "slice", 2, Rc::new(|interp, this, args| {
        let items = interp.iterable_to_vec(&this)?;
        let len = items.len();
        let (s, e) = normalize_slice(args.get(0), args.get(1), len);
        Ok(interp.new_array(items[s..e].to_vec()))
    }));
    def_method(realm, &ap, "splice", 2, Rc::new(|interp, this, args| {
        if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &mut o.borrow_mut().kind {
                let len = items.len();
                let start = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
                let start = if start < 0 { (len as i32 + start).max(0) as usize } else { (start as usize).min(len) };
                let del_count = if args.len() >= 2 {
                    let dc = to_int32(args.get(1).unwrap_or(&Value::from_int(0)));
                    if dc < 0 { 0 } else { (dc as usize).min(len - start) }
                } else {
                    len - start
                };
                let removed: Vec<Value> = items.drain(start..start + del_count).collect();
                for (i, a) in args.iter().skip(2).enumerate() {
                    items.insert(start + i, a.clone());
                }
                return Ok(interp.new_array(removed));
            }
        }
        Ok(interp.new_array(vec![]))
    }));
    def_method(realm, &ap, "indexOf", 1, Rc::new(|_interp, this, args| {
        let target = args.get(0).cloned().unwrap_or(Value::Undefined);
        let from = to_int32(args.get(1).unwrap_or(&Value::from_int(0)));
        if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &o.borrow().kind {
                let from = if from < 0 { (items.len() as i32 + from).max(0) as usize } else { from as usize };
                for i in from..items.len() {
                    if strict_equals(&items[i], &target) { return Ok(Value::from_int(i as i32)); }
                }
            }
        }
        Ok(Value::from_int(-1))
    }));
    def_method(realm, &ap, "lastIndexOf", 1, Rc::new(|_interp, this, args| {
        let target = args.get(0).cloned().unwrap_or(Value::Undefined);
        if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &o.borrow().kind {
                for i in (0..items.len()).rev() {
                    if strict_equals(&items[i], &target) { return Ok(Value::from_int(i as i32)); }
                }
            }
        }
        Ok(Value::from_int(-1))
    }));
    def_method(realm, &ap, "includes", 1, Rc::new(|_interp, this, args| {
        let target = args.get(0).cloned().unwrap_or(Value::Undefined);
        if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &o.borrow().kind {
                for it in items.iter() {
                    if same_value_zero(it, &target) { return Ok(Value::Bool(true)); }
                }
            }
        }
        Ok(Value::Bool(false))
    }));
    def_method(realm, &ap, "find", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = interp.iterable_to_vec(&this)?;
        for (i, it) in items.iter().enumerate() {
            let r = interp.call_value(cb.clone(), Value::Undefined, &[it.clone(), Value::from_int(i as i32), this.clone()])?;
            if to_boolean(&r) { return Ok(it.clone()); }
        }
        Ok(Value::Undefined)
    }));
    def_method(realm, &ap, "findIndex", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = interp.iterable_to_vec(&this)?;
        for (i, it) in items.iter().enumerate() {
            let r = interp.call_value(cb.clone(), Value::Undefined, &[it.clone(), Value::from_int(i as i32), this.clone()])?;
            if to_boolean(&r) { return Ok(Value::from_int(i as i32)); }
        }
        Ok(Value::from_int(-1))
    }));
    def_method(realm, &ap, "findLast", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = interp.iterable_to_vec(&this)?;
        for i in (0..items.len()).rev() {
            let r = interp.call_value(cb.clone(), Value::Undefined, &[items[i].clone(), Value::from_int(i as i32), this.clone()])?;
            if to_boolean(&r) { return Ok(items[i].clone()); }
        }
        Ok(Value::Undefined)
    }));
    def_method(realm, &ap, "findLastIndex", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = interp.iterable_to_vec(&this)?;
        for i in (0..items.len()).rev() {
            let r = interp.call_value(cb.clone(), Value::Undefined, &[items[i].clone(), Value::from_int(i as i32), this.clone()])?;
            if to_boolean(&r) { return Ok(Value::from_int(i as i32)); }
        }
        Ok(Value::from_int(-1))
    }));
    def_method(realm, &ap, "filter", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = interp.iterable_to_vec(&this)?;
        let mut out = Vec::new();
        for (i, it) in items.iter().enumerate() {
            let r = interp.call_value(cb.clone(), Value::Undefined, &[it.clone(), Value::from_int(i as i32), this.clone()])?;
            if to_boolean(&r) { out.push(it.clone()); }
        }
        Ok(interp.new_array(out))
    }));
    def_method(realm, &ap, "map", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = interp.iterable_to_vec(&this)?;
        let mut out = Vec::new();
        for (i, it) in items.iter().enumerate() {
            out.push(interp.call_value(cb.clone(), Value::Undefined, &[it.clone(), Value::from_int(i as i32), this.clone()])?);
        }
        Ok(interp.new_array(out))
    }));
    def_method(realm, &ap, "forEach", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = interp.iterable_to_vec(&this)?;
        for (i, it) in items.iter().enumerate() {
            interp.call_value(cb.clone(), Value::Undefined, &[it.clone(), Value::from_int(i as i32), this.clone()])?;
        }
        Ok(Value::Undefined)
    }));
    def_method(realm, &ap, "reduce", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = interp.iterable_to_vec(&this)?;
        let (mut acc, start) = if args.len() >= 2 {
            (args.get(1).cloned().unwrap(), 0)
        } else {
            if items.is_empty() { return Err(error::throw_type("Reduce of empty array with no initial value")); }
            (items[0].clone(), 1)
        };
        for i in start..items.len() {
            acc = interp.call_value(cb.clone(), Value::Undefined, &[acc, items[i].clone(), Value::from_int(i as i32), this.clone()])?;
        }
        Ok(acc)
    }));
    def_method(realm, &ap, "reduceRight", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = interp.iterable_to_vec(&this)?;
        let len = items.len();
        let (mut acc, start) = if args.len() >= 2 {
            (args.get(1).cloned().unwrap(), len as i32 - 1)
        } else {
            if items.is_empty() { return Err(error::throw_type("Reduce of empty array with no initial value")); }
            (items[len - 1].clone(), len as i32 - 2)
        };
        let mut i = start;
        while i >= 0 {
            acc = interp.call_value(cb.clone(), Value::Undefined, &[acc, items[i as usize].clone(), Value::from_int(i), this.clone()])?;
            i -= 1;
        }
        Ok(acc)
    }));
    def_method(realm, &ap, "some", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = interp.iterable_to_vec(&this)?;
        for (i, it) in items.iter().enumerate() {
            let r = interp.call_value(cb.clone(), Value::Undefined, &[it.clone(), Value::from_int(i as i32), this.clone()])?;
            if to_boolean(&r) { return Ok(Value::Bool(true)); }
        }
        Ok(Value::Bool(false))
    }));
    def_method(realm, &ap, "every", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = interp.iterable_to_vec(&this)?;
        for (i, it) in items.iter().enumerate() {
            let r = interp.call_value(cb.clone(), Value::Undefined, &[it.clone(), Value::from_int(i as i32), this.clone()])?;
            if !to_boolean(&r) { return Ok(Value::Bool(false)); }
        }
        Ok(Value::Bool(true))
    }));
    def_method(realm, &ap, "sort", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned();
        if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &mut o.borrow_mut().kind {
                let mut err: Option<Value> = None;
                items.sort_by(|a, b| {
                    if err.is_some() { return std::cmp::Ordering::Equal; }
                    if let Some(cb) = &cb {
                        if cb.is_callable() {
                            // need interp; but we can't get it here. Use a thread-local fallback comparator.
                            // For correctness, fall back to default compare when a comparator is given by
                            // using a separate pass below.
                            return std::cmp::Ordering::Equal;
                        }
                    }
                    let sa = if a.is_undefined() { String::new() } else { crate::value::to_string(a) };
                    let sb = if b.is_undefined() { String::new() } else { crate::value::to_string(b) };
                    sa.cmp(&sb)
                });
                // apply comparator if provided (simple bubble pass to keep &mut interp)
                if let Some(cb) = cb {
                    if cb.is_callable() {
                        let n = items.len();
                        let mut swapped = true;
                        let mut pass = 0;
                        while swapped && pass < n {
                            swapped = false;
                            for i in 0..n - 1 - pass.min(n.saturating_sub(1)) {
                                if i + 1 >= n { break; }
                                let a = items[i].clone();
                                let b = items[i + 1].clone();
                                let r = interp.call_value(cb.clone(), Value::Undefined, &[a, b]);
                                match r {
                                    Ok(v) => {
                                        if to_number(&v) > 0.0 {
                                            items.swap(i, i + 1);
                                            swapped = true;
                                        }
                                    }
                                    Err(e) => { err = Some(e); break; }
                                }
                            }
                            pass += 1;
                        }
                    }
                }
                if let Some(e) = err { return Err(e); }
            }
        }
        Ok(this)
    }));
    def_method(realm, &ap, "fill", 1, Rc::new(|_interp, this, args| {
        let val = args.get(0).cloned().unwrap_or(Value::Undefined);
        if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &mut o.borrow_mut().kind {
                let len = items.len();
                let (s, e) = normalize_slice(args.get(1), args.get(2), len);
                for i in s..e { items[i] = val.clone(); }
            }
        }
        Ok(this)
    }));
    def_method(realm, &ap, "flat", 0, Rc::new(|interp, this, args| {
        let depth = to_int32(args.get(0).unwrap_or(&Value::from_int(1))).max(0) as usize;
        let items = interp.iterable_to_vec(&this)?;
        let mut out = Vec::new();
        flat_into(&items, depth, &mut out);
        Ok(interp.new_array(out))
    }));
    def_method(realm, &ap, "flatMap", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = interp.iterable_to_vec(&this)?;
        let mut out = Vec::new();
        for (i, it) in items.iter().enumerate() {
            let r = interp.call_value(cb.clone(), Value::Undefined, &[it.clone(), Value::from_int(i as i32), this.clone()])?;
            let mut tmp = Vec::new();
            interp.flatten_into(&mut tmp, &r)?;
            out.extend(tmp);
        }
        Ok(interp.new_array(out))
    }));
    def_method(realm, &ap, "at", 1, Rc::new(|_interp, this, args| {
        let idx = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
        if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &o.borrow().kind {
                let i = if idx < 0 { items.len() as i32 + idx } else { idx };
                if i >= 0 && (i as usize) < items.len() { return Ok(items[i as usize].clone()); }
            }
        }
        Ok(Value::Undefined)
    }));
    def_method(realm, &ap, "copyWithin", 2, Rc::new(|_interp, this, args| {
        if let Value::Object(o) = &this {
            if let ObjectKind::Array(items) = &mut o.borrow_mut().kind {
                let len = items.len();
                let target = norm_index(args.get(0), len);
                let start = norm_index(args.get(1), len);
                // `end` defaults to the array length (not 0).
                let end = match args.get(2) {
                    Some(Value::Undefined) | None => len,
                    Some(v) => norm_index(Some(v), len).min(len),
                };
                if target >= len || start >= end { return Ok(this.clone()); }
                let to_copy = (end - start).min(len - target);
                let snapshot: Vec<Value> = items[start..start + to_copy].to_vec();
                for (k, v) in snapshot.into_iter().enumerate() {
                    items[target + k] = v;
                }
            }
        }
        Ok(this)
    }));
    def_method(realm, &ap, "toString", 0, Rc::new(|interp, this, _args| {
        let items = interp.iterable_to_vec(&this)?;
        let parts: Vec<String> = items.iter().map(|v| if v.is_nullish() { String::new() } else { crate::value::to_string(v) }).collect();
        Ok(Value::from_string(parts.join(",")))
    }));
    def_method(realm, &ap, "toLocaleString", 0, Rc::new(|interp, this, _args| {
        let f = interp.get_property(&this, &PropKey::from_str("toString"))?;
        interp.call_value(f, this, &[])
    }));
    // ES2023 immutable array methods (toSorted, toReversed, toSpliced, with)
    def_method(realm, &ap, "toSorted", 1, Rc::new(|interp, this, args| {
        let mut items = interp.iterable_to_vec(&this)?;
        let cb = args.get(0).cloned();
        if let Some(cb) = &cb {
            if cb.is_callable() {
                let n = items.len();
                let mut err: Option<Value> = None;
                items.sort_by(|a, b| {
                    if err.is_some() { return std::cmp::Ordering::Equal; }
                    match interp.call_value(cb.clone(), Value::Undefined, &[a.clone(), b.clone()]) {
                        Ok(v) => {
                            let n = to_number(&v);
                            if n < 0.0 { std::cmp::Ordering::Less }
                            else if n > 0.0 { std::cmp::Ordering::Greater }
                            else { std::cmp::Ordering::Equal }
                        }
                        Err(e) => { err = Some(e); std::cmp::Ordering::Equal }
                    }
                });
                if let Some(e) = err { return Err(e); }
            } else {
                items.sort_by(|a, b| {
                    let sa = if a.is_undefined() { String::new() } else { crate::value::to_string(a) };
                    let sb = if b.is_undefined() { String::new() } else { crate::value::to_string(b) };
                    sa.cmp(&sb)
                });
            }
        } else {
            items.sort_by(|a, b| {
                let sa = if a.is_undefined() { String::new() } else { crate::value::to_string(a) };
                let sb = if b.is_undefined() { String::new() } else { crate::value::to_string(b) };
                sa.cmp(&sb)
            });
        }
        Ok(interp.new_array(items))
    }));
    def_method(realm, &ap, "toReversed", 0, Rc::new(|interp, this, _args| {
        let mut items = interp.iterable_to_vec(&this)?;
        items.reverse();
        Ok(interp.new_array(items))
    }));
    def_method(realm, &ap, "with", 2, Rc::new(|interp, this, args| {
        let items = interp.iterable_to_vec(&this)?;
        let len = items.len() as i32;
        let mut idx = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
        if idx < 0 { idx += len; }
        if idx < 0 || idx >= len {
            return Err(error::throw_range("Invalid array index"));
        }
        let val = args.get(1).cloned().unwrap_or(Value::Undefined);
        let mut new_items = items.clone();
        new_items[idx as usize] = val;
        Ok(interp.new_array(new_items))
    }));
    def_method(realm, &ap, "toSpliced", 2, Rc::new(|interp, this, args| {
        let items = interp.iterable_to_vec(&this)?;
        let len = items.len() as i32;
        let mut start = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
        if start < 0 { start += len; }
        if start < 0 { start = 0; }
        if start > len { start = len; }
        let start = start as usize;
        let delete_count = if args.len() >= 2 {
            let dc = to_int32(args.get(1).unwrap_or(&Value::from_int(0)));
            if dc < 0 { 0 } else { (dc as usize).min(items.len() - start) }
        } else { 0 };
        let mut new_items = items.clone();
        new_items.drain(start..start + delete_count);
        for (i, a) in args.iter().skip(2).enumerate() {
            new_items.insert(start + i, a.clone());
        }
        Ok(interp.new_array(new_items))
    }));
    // ES2024 group (Object.groupBy / Array.prototype.group)
    def_method(realm, &ap, "group", 1, Rc::new(|interp, this, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let items = interp.iterable_to_vec(&this)?;
        let result = ObjectInner::new_object();
        result.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
        for (i, it) in items.iter().enumerate() {
            let key = interp.call_value(cb.clone(), Value::Undefined, &[it.clone(), Value::from_int(i as i32)])?;
            let key_str = to_string_coerce(&key);
            let pk = PropKey::from_str(&key_str);
            let needs_new = !result.borrow().props.contains_key(&pk);
            if needs_new {
                let a = ObjectInner::new_array(vec![]);
                a.borrow_mut().proto = Some(Value::Object(interp.realm().array_proto.clone()));
                result.borrow_mut().props.insert(pk.clone(), Property::data(Value::Object(a)));
            }
            // push to the array
            let arr_val = result.borrow().props.get(&pk).cloned();
            if let Some(Property { kind: PropKind::Data(Value::Object(o)), .. }) = arr_val {
                if let ObjectKind::Array(arr_items) = &mut o.borrow_mut().kind {
                    arr_items.push(it.clone());
                }
            }
        }
        Ok(Value::Object(result))
    }));
    let _ = interp;
}

fn to_string_coerce(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        Value::Number(n) => crate::value::format_number(*n),
        Value::Undefined => "undefined".to_string(),
        Value::Null => "null".to_string(),
        _ => crate::value::to_string(v),
    }
}

fn flat_into(items: &[Value], depth: usize, out: &mut Vec<Value>) {
    for it in items {
        if depth > 0 {
            if let Value::Object(o) = it {
                if matches!(o.borrow().kind, ObjectKind::Array(_)) {
                    let sub = if let ObjectKind::Array(s) = &o.borrow().kind { s.clone() } else { vec![] };
                    flat_into(&sub, depth - 1, out);
                    continue;
                }
            }
        }
        out.push(it.clone());
    }
}

fn norm_index(v: Option<&Value>, len: usize) -> usize {
    let n = to_int32(v.unwrap_or(&Value::from_int(0)));
    if n < 0 { (len as i32 + n).max(0) as usize } else { (n as usize).min(len) }
}

pub fn normalize_slice(start: Option<&Value>, end: Option<&Value>, len: usize) -> (usize, usize) {
    let s = to_int32(start.unwrap_or(&Value::from_int(0)));
    let s = if s < 0 { (len as i32 + s).max(0) as usize } else { (s as usize).min(len) };
    let e = match end {
        None => len,
        Some(Value::Undefined) => len,
        Some(v) => {
            let n = to_int32(v);
            if n < 0 { (len as i32 + n).max(0) as usize } else { (n as usize).min(len) }
        }
    };
    (s.min(len), e.max(s).min(len))
}
