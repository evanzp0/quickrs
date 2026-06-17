//! console global + methods.

use crate::realm::Realm;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{def_method, install_global};
use std::rc::Rc;
use std::cell::Cell;
use std::collections::HashMap;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let c = ObjectInner::new_object();
    c.borrow_mut().proto = Some(Value::Object(realm.object_proto.clone()));
    c.borrow_mut().class = "Object";
    let log_fn: NativeFn = Rc::new(|interp, _this, args| {
        let parts: Vec<String> = args.iter().map(|a| crate::builtins::pretty_print(a, interp, 0)).collect();
        println!("{}", parts.join(" "));
        Ok(Value::Undefined)
    });
    def_method(realm, &c, "log", 0, log_fn.clone());
    def_method(realm, &c, "info", 0, log_fn.clone());
    def_method(realm, &c, "debug", 0, log_fn.clone());
    def_method(realm, &c, "trace", 0, log_fn.clone());
    def_method(realm, &c, "dir", 0, log_fn.clone());
    def_method(realm, &c, "dirxml", 0, log_fn.clone());
    def_method(realm, &c, "table", 0, log_fn.clone());
    def_method(realm, &c, "group", 0, log_fn.clone());
    def_method(realm, &c, "groupCollapsed", 0, log_fn.clone());
    def_method(realm, &c, "groupEnd", 0, Rc::new(|_i, _t, _a| Ok(Value::Undefined)));
    def_method(realm, &c, "error", 0, Rc::new(|interp, _this, args| {
        let parts: Vec<String> = args.iter().map(|a| crate::builtins::pretty_print(a, interp, 0)).collect();
        eprintln!("{}", parts.join(" "));
        Ok(Value::Undefined)
    }));
    def_method(realm, &c, "warn", 0, Rc::new(|interp, _this, args| {
        let parts: Vec<String> = args.iter().map(|a| crate::builtins::pretty_print(a, interp, 0)).collect();
        eprintln!("{}", parts.join(" "));
        Ok(Value::Undefined)
    }));
    def_method(realm, &c, "assert", 0, Rc::new(|interp, _this, args| {
        let cond = to_boolean(args.get(0).unwrap_or(&Value::Undefined));
        if !cond {
            let parts: Vec<String> = args.iter().skip(1).map(|a| crate::builtins::pretty_print(a, interp, 0)).collect();
            eprintln!("Assertion failed: {}", parts.join(" "));
        }
        Ok(Value::Undefined)
    }));
    def_method(realm, &c, "count", 0, Rc::new(|_i, _t, args| {
        let label = crate::value::to_string(args.get(0).unwrap_or(&Value::from_str("default")));
        thread_local! { static COUNTERS: std::cell::RefCell<HashMap<String, u64>> = std::cell::RefCell::new(HashMap::new()); }
        let n = COUNTERS.with(|m| { let mut b = m.borrow_mut(); let e = b.entry(label.clone()).or_insert(0); *e += 1; *e });
        println!("{}: {}", label, n);
        Ok(Value::Undefined)
    }));
    def_method(realm, &c, "countReset", 0, Rc::new(|_i, _t, args| {
        let label = crate::value::to_string(args.get(0).unwrap_or(&Value::from_str("default")));
        thread_local! { static COUNTERS: std::cell::RefCell<HashMap<String, u64>> = std::cell::RefCell::new(HashMap::new()); }
        COUNTERS.with(|m| { m.borrow_mut().remove(&label); });
        Ok(Value::Undefined)
    }));
    def_method(realm, &c, "time", 0, Rc::new(|_i, _t, args| {
        let label = crate::value::to_string(args.get(0).unwrap_or(&Value::from_str("default")));
        thread_local! { static TIMERS: std::cell::RefCell<HashMap<String, std::time::Instant>> = std::cell::RefCell::new(HashMap::new()); }
        TIMERS.with(|m| { m.borrow_mut().insert(label, std::time::Instant::now()); });
        Ok(Value::Undefined)
    }));
    def_method(realm, &c, "timeEnd", 0, Rc::new(|_i, _t, args| {
        let label = crate::value::to_string(args.get(0).unwrap_or(&Value::from_str("default")));
        thread_local! { static TIMERS: std::cell::RefCell<HashMap<String, std::time::Instant>> = std::cell::RefCell::new(HashMap::new()); }
        let dur = TIMERS.with(|m| m.borrow_mut().remove(&label).map(|t| t.elapsed()));
        if let Some(d) = dur { println!("{}: {:?}", label, d); }
        Ok(Value::Undefined)
    }));
    def_method(realm, &c, "clear", 0, Rc::new(|_i, _t, _a| Ok(Value::Undefined)));
    let _ = interp;
    install_global(interp, realm, "console", Value::Object(c));
}
