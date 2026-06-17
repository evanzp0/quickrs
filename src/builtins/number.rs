//! Number constructor + Number.prototype + constants.

use crate::realm::Realm;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{make_ctor, install_global_ctor, def_method, def_const_value, CtorFn};
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let call_fn: NativeFn = Rc::new(|_interp, _this, args| {
        Ok(Value::Number(match args.get(0) {
            Some(Value::Undefined) | None => 0.0,
            Some(v) => crate::value::to_number(v),
        }))
    });
    let ctor_fn: CtorFn = Rc::new(|interp, _this, args, _nt| {
        let n = match args.get(0) {
            Some(Value::Undefined) | None => 0.0,
            Some(v) => crate::value::to_number(v),
        };
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().number_proto.clone()));
        o.borrow_mut().class = "Number";
        o.borrow_mut().kind = ObjectKind::Number(n);
        Ok(Value::Object(o))
    });
    let ctor = make_ctor(realm, "Number", 1, call_fn, ctor_fn);
    install_global_ctor(interp, realm, "Number", ctor.clone(), realm.number_proto.clone());
    if let Value::Object(co) = &ctor {
        let co = co.clone();
        def_method(realm, &co, "isInteger", 1, Rc::new(|_i, _t, args| {
            if let Some(Value::Number(n)) = args.get(0) {
                return Ok(Value::Bool(n.is_finite() && n.fract() == 0.0));
            }
            Ok(Value::Bool(false))
        }));
        def_method(realm, &co, "isFinite", 1, Rc::new(|_i, _t, args| {
            if let Some(Value::Number(n)) = args.get(0) { return Ok(Value::Bool(n.is_finite())); }
            Ok(Value::Bool(false))
        }));
        def_method(realm, &co, "isNaN", 1, Rc::new(|_i, _t, args| {
            if let Some(Value::Number(n)) = args.get(0) { return Ok(Value::Bool(n.is_nan())); }
            Ok(Value::Bool(false))
        }));
        def_method(realm, &co, "isSafeInteger", 1, Rc::new(|_i, _t, args| {
            if let Some(Value::Number(n)) = args.get(0) {
                return Ok(Value::Bool(n.is_finite() && n.fract() == 0.0 && n.abs() <= 9007199254740991.0));
            }
            Ok(Value::Bool(false))
        }));
        def_method(realm, &co, "parseFloat", 1, Rc::new(|_i, _t, args| {
            let s = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
            Ok(Value::Number(parse_float_local(&s)))
        }));
        def_method(realm, &co, "parseInt", 2, Rc::new(|_i, _t, args| {
            let s = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
            let radix = to_int32(args.get(1).unwrap_or(&Value::from_int(10)));
            Ok(Value::Number(parse_int_local(&s, radix)))
        }));
        def_const_value(&co, "MAX_SAFE_INTEGER", Value::Number(9007199254740991.0));
        def_const_value(&co, "MIN_SAFE_INTEGER", Value::Number(-9007199254740991.0));
        def_const_value(&co, "MAX_VALUE", Value::Number(f64::MAX));
        def_const_value(&co, "MIN_VALUE", Value::Number(5e-324));
        def_const_value(&co, "EPSILON", Value::Number(f64::EPSILON));
        def_const_value(&co, "POSITIVE_INFINITY", Value::Number(f64::INFINITY));
        def_const_value(&co, "NEGATIVE_INFINITY", Value::Number(f64::NEG_INFINITY));
        def_const_value(&co, "NaN", Value::Number(f64::NAN));
    }
    let np = realm.number_proto.clone();
    def_method(realm, &np, "toString", 1, Rc::new(|_i, this, args| {
        let n = num_of(&this);
        let radix = to_int32(args.get(0).unwrap_or(&Value::from_int(10)));
        if radix == 10 { return Ok(Value::from_string(crate::value::format_number(n))); }
        if radix < 2 || radix > 36 { return Ok(Value::from_string("NaN")); }
        Ok(Value::from_string(to_radix(n, radix as u32)))
    }));
    def_method(realm, &np, "valueOf", 0, Rc::new(|_i, this, _a| Ok(Value::Number(num_of(&this)))));
    def_method(realm, &np, "toFixed", 1, Rc::new(|_i, this, args| {
        let n = num_of(&this);
        let d = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
        Ok(Value::from_string(format!("{:.*}", d.max(0) as usize, n)))
    }));
    def_method(realm, &np, "toPrecision", 1, Rc::new(|_i, this, args| {
        let n = num_of(&this);
        match args.get(0) {
            Some(Value::Undefined) | None => Ok(Value::from_string(crate::value::format_number(n))),
            Some(v) => {
                let p = to_int32(v);
                if p < 1 || p > 100 { return Ok(Value::from_string(crate::value::format_number(n))); }
                Ok(Value::from_string(format!("{:.*e}", (p - 1) as usize, n).replace('e', "e")))
            }
        }
    }));
    def_method(realm, &np, "toExponential", 1, Rc::new(|_i, this, args| {
        let n = num_of(&this);
        let d = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
        Ok(Value::from_string(format!("{:.*e}", d.max(0) as usize, n)))
    }));
    def_method(realm, &np, "toLocaleString", 0, Rc::new(|_i, this, _a| Ok(Value::from_string(crate::value::format_number(num_of(&this))))));
    let _ = interp;
}

fn num_of(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        Value::Object(o) => {
            if let ObjectKind::Number(n) = o.borrow().kind { n } else { f64::NAN }
        }
        _ => f64::NAN,
    }
}

fn to_radix(n: f64, radix: u32) -> String {
    if n.is_nan() { return "NaN".to_string(); }
    if n == f64::INFINITY { return "Infinity".to_string(); }
    if n == f64::NEG_INFINITY { return "-Infinity".to_string(); }
    let neg = n < 0.0;
    let mut n = n.abs();
    let int_part = n.trunc();
    let mut frac = n - int_part;
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut int_str = String::new();
    let mut ip = int_part as u64;
    if ip == 0 { int_str.push('0'); }
    while ip > 0 {
        let d = (ip % radix as u64) as usize;
        int_str.insert(0, digits[d] as char);
        ip /= radix as u64;
    }
    let mut s = int_str;
    if frac > 0.0 {
        s.push('.');
        let mut count = 0;
        while frac > 0.0 && count < 52 {
            frac *= radix as f64;
            let d = frac.trunc() as usize;
            s.push(digits[d] as char);
            frac -= d as f64;
            count += 1;
        }
    }
    if neg { s.insert(0, '-'); }
    s
}

fn parse_float_local(s: &str) -> f64 {
    let s = s.trim_start();
    if s.starts_with("Infinity") || s.starts_with("+Infinity") { return f64::INFINITY; }
    if s.starts_with("-Infinity") { return f64::NEG_INFINITY; }
    let bytes = s.as_bytes();
    let mut end = 0;
    if !bytes.is_empty() && (bytes[0] == b'+' || bytes[0] == b'-') { end = 1; }
    let mut seen_dot = false;
    let mut seen_exp = false;
    let mut seen_digit = false;
    while end < bytes.len() {
        let c = bytes[end];
        if c.is_ascii_digit() { seen_digit = true; end += 1; }
        else if c == b'.' && !seen_dot && !seen_exp { seen_dot = true; end += 1; }
        else if (c == b'e' || c == b'E') && !seen_exp && seen_digit {
            seen_exp = true; end += 1;
            if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') { end += 1; }
        } else { break; }
    }
    s[..end].parse::<f64>().unwrap_or(f64::NAN)
}

fn parse_int_local(s: &str, mut radix: i32) -> f64 {
    let s = s.trim_start();
    if s.is_empty() { return f64::NAN; }
    let (sign, rest) = if let Some(r) = s.strip_prefix('-') { (-1.0, r) } else if let Some(r) = s.strip_prefix('+') { (1.0, r) } else { (1.0, s) };
    let rest = if radix == 0 || radix == 16 {
        if let Some(r) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
            radix = 16;
            r
        } else { rest }
    } else { rest };
    if radix == 0 { radix = 10; }
    if !(2..=36).contains(&radix) { return f64::NAN; }
    let mut end = 0;
    for (i, c) in rest.char_indices() {
        if c.to_digit(radix as u32).is_some() { end = i + c.len_utf8(); } else { break; }
    }
    if end == 0 { return f64::NAN; }
    let digits = &rest[..end];
    sign * u64::from_str_radix(digits, radix as u32).map(|v| v as f64).unwrap_or(f64::NAN)
}
