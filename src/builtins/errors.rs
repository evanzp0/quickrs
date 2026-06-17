//! Error constructors + shared Error.prototype methods.

use crate::realm::Realm;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{make_ctor, install_global_ctor, def_method, CtorFn};
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    install_one(interp, realm, "Error", realm.error_proto.clone());
    install_one(interp, realm, "TypeError", realm.type_error_proto.clone());
    install_one(interp, realm, "RangeError", realm.range_error_proto.clone());
    install_one(interp, realm, "SyntaxError", realm.syntax_error_proto.clone());
    install_one(interp, realm, "ReferenceError", realm.reference_error_proto.clone());
    install_one(interp, realm, "URIError", realm.uri_error_proto.clone());
    install_one(interp, realm, "EvalError", realm.eval_error_proto.clone());
    install_aggregate_error(interp, realm);
    // Error.captureStackTrace (V8-style extension)
    let err_ctor = interp.get_global("Error");
    if let Value::Object(co) = &err_ctor {
        let co = co.clone();
        def_method(realm, &co, "captureStackTrace", 2, Rc::new(|interp, _this, args| {
            let target = args.get(0).cloned().unwrap_or(Value::Undefined);
            let frames = interp.shared.stack.borrow();
            let mut stack = String::from("Error\n");
            // Skip the top frame (captureStackTrace itself) — simplified.
            for f in frames.iter().rev() {
                stack.push_str(&format!("    at {}\n", f));
            }
            if let Value::Object(o) = &target {
                o.borrow_mut().props.insert(PropKey::from_str("stack"), Property::data(Value::from_string(stack)));
            }
            Ok(Value::Undefined)
        }));
        co.borrow_mut().props.insert(PropKey::from_str("stackTraceLimit"), Property::data(Value::from_int(10)));
    }
    let _ = interp;
}

fn install_aggregate_error(interp: &mut Interpreter, realm: &Rc<Realm>) {
    // AggregateError reuses Error.prototype (simplified — no dedicated proto).
    let proto = realm.error_proto.clone();
    let pname: &'static str = "AggregateError";
    let call_fn: NativeFn = Rc::new({
        let proto = proto.clone();
        move |interp, _this, args| {
            // AggregateError(errors, message)
            let errors = args.get(0).cloned().unwrap_or(Value::Undefined);
            let msg = match args.get(1) { Some(Value::Undefined) | None => None, Some(v) => Some(Rc::from(crate::value::to_string(v).as_str())) };
            let err = make_err(&proto, pname, msg);
            if let Value::Object(o) = &err {
                let errors_arr = if errors.is_undefined() { interp.new_array(vec![]) } else { errors };
                o.borrow_mut().props.insert(PropKey::from_str("errors"), Property::data(errors_arr));
            }
            attach_stack(interp, &err, pname);
            Ok(err)
        }
    });
    let ctor_fn: CtorFn = Rc::new({
        let proto = proto.clone();
        move |interp, _this, args, _nt| {
            let errors = args.get(0).cloned().unwrap_or(Value::Undefined);
            let msg = match args.get(1) { Some(Value::Undefined) | None => None, Some(v) => Some(Rc::from(crate::value::to_string(v).as_str())) };
            let err = make_err(&proto, pname, msg);
            if let Value::Object(o) = &err {
                let errors_arr = if errors.is_undefined() { interp.new_array(vec![]) } else { errors };
                o.borrow_mut().props.insert(PropKey::from_str("errors"), Property::data(errors_arr));
            }
            attach_stack(interp, &err, pname);
            Ok(err)
        }
    });
    let ctor = make_ctor(realm, "AggregateError", 2, call_fn, ctor_fn);
    crate::builtins::install_global_ctor(interp, realm, "AggregateError", ctor, realm.error_proto.clone());
}

fn install_one(interp: &mut Interpreter, realm: &Rc<Realm>, name: &str, proto: ObjRef) {
    let pname: &'static str = name.into_static();
    let call_fn: NativeFn = Rc::new({
        let proto = proto.clone();
        let pname = pname;
        move |interp, _this, args| {
            let msg = match args.get(0) { Some(Value::Undefined) | None => None, Some(v) => Some(Rc::from(crate::value::to_string(v).as_str())) };
            let err = make_err(&proto, pname, msg);
            // ES2022 Error cause: new Error(msg, { cause: ... })
            if let Some(opts) = args.get(1) {
                if let Value::Object(_) = opts {
                    let cause = interp.get_property(opts, &PropKey::from_str("cause")).unwrap_or(Value::Undefined);
                    if !cause.is_undefined() {
                        if let Value::Object(o) = &err {
                            o.borrow_mut().props.insert(PropKey::from_str("cause"), Property::data(cause));
                        }
                    }
                }
            }
            attach_stack(interp, &err, pname);
            Ok(err)
        }
    });
    let ctor_fn: CtorFn = Rc::new({
        let proto = proto.clone();
        let pname = pname;
        move |interp, _this, args, _nt| {
            let msg = match args.get(0) { Some(Value::Undefined) | None => None, Some(v) => Some(Rc::from(crate::value::to_string(v).as_str())) };
            let err = make_err(&proto, pname, msg);
            if let Some(opts) = args.get(1) {
                if let Value::Object(_) = opts {
                    let cause = interp.get_property(opts, &PropKey::from_str("cause")).unwrap_or(Value::Undefined);
                    if !cause.is_undefined() {
                        if let Value::Object(o) = &err {
                            o.borrow_mut().props.insert(PropKey::from_str("cause"), Property::data(cause));
                        }
                    }
                }
            }
            attach_stack(interp, &err, pname);
            Ok(err)
        }
    });
    let ctor = make_ctor(realm, name, 1, call_fn, ctor_fn);
    install_global_ctor(interp, realm, name, ctor, proto.clone());
    def_method(realm, &proto, "toString", 0, Rc::new(|_i, this, _a| {
        if let Value::Object(o) = &this {
            let b = o.borrow();
            let name = b.props.get(&PropKey::from_str("name")).and_then(|p| if let PropKind::Data(Value::String(s)) = &p.kind { Some(s.to_string()) } else { None }).unwrap_or_else(|| "Error".to_string());
            let msg = b.props.get(&PropKey::from_str("message")).and_then(|p| if let PropKind::Data(Value::String(s)) = &p.kind { Some(s.to_string()) } else { None }).unwrap_or_default();
            return Ok(Value::from_string(if msg.is_empty() { name } else { format!("{}: {}", name, msg) }));
        }
        Ok(Value::from_str("Error"))
    }));
    let _ = interp;
}

/// Attach a `.stack` string to an Error object, built from the interpreter's
/// current call stack.
fn attach_stack(interp: &Interpreter, err: &Value, name: &str) {
    let msg = if let Value::Object(o) = err {
        o.borrow().props.get(&PropKey::from_str("message")).and_then(|p| if let PropKind::Data(Value::String(s)) = &p.kind { Some(s.to_string()) } else { None }).unwrap_or_default()
    } else { String::new() };
    let header = if msg.is_empty() { name.to_string() } else { format!("{}: {}", name, msg) };
    let frames = interp.shared.stack.borrow();
    let mut stack = header;
    stack.push_str("\n");
    if frames.is_empty() {
        stack.push_str("    at <anonymous>\n");
    } else {
        for f in frames.iter().rev() {
            stack.push_str(&format!("    at {}\n", f));
        }
    }
    if let Value::Object(o) = err {
        o.borrow_mut().props.insert(PropKey::from_str("stack"), Property::data(Value::from_string(stack)));
    }
}

fn make_err(proto: &ObjRef, name: &str, msg: Option<Rc<str>>) -> Value {
    let o = ObjectInner::new_object();
    o.borrow_mut().proto = Some(Value::Object(proto.clone()));
    o.borrow_mut().class = "Error";
    o.borrow_mut().kind = ObjectKind::Error;
    o.borrow_mut().props.insert(PropKey::from_str("name"), Property::data(Value::from_str(name)));
    o.borrow_mut().props.insert(PropKey::from_str("message"), Property::data(match msg { Some(m) => Value::String(m), None => Value::from_str("") }));
    Value::Object(o)
}

trait StaticStr { fn into_static(self) -> &'static str; }
impl StaticStr for &str {
    fn into_static(self) -> &'static str {
        // leak once per distinct name; we only use a small fixed set
        match self {
            "Error" => "Error",
            "TypeError" => "TypeError",
            "RangeError" => "RangeError",
            "SyntaxError" => "SyntaxError",
            "ReferenceError" => "ReferenceError",
            "URIError" => "URIError",
            "EvalError" => "EvalError",
            _ => "Error",
        }
    }
}
