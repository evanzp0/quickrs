//! Math object.

use crate::realm::Realm;
use crate::interp::Interpreter;
use crate::value::*;
use crate::builtins::{def_method, def_const_value};
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let m = ObjectInner::new_object();
    m.borrow_mut().proto = Some(Value::Object(realm.object_proto.clone()));
    m.borrow_mut().class = "Math";
    let one = Rc::new(|n: f64| n);
    macro_rules! math1 { ($name:expr, $f:expr) => {
        def_method(realm, &m, $name, 1, Rc::new(|_i, _t, args| {
            let n = crate::value::to_number(args.get(0).unwrap_or(&Value::Undefined));
            Ok(Value::Number($f(n)))
        }));
    }}
    math1!("abs", f64::abs);
    math1!("ceil", f64::ceil);
    math1!("floor", f64::floor);
    math1!("round", round_half_up);
    math1!("trunc", f64::trunc);
    math1!("sign", sign);
    math1!("sqrt", f64::sqrt);
    math1!("cbrt", f64::cbrt);
    math1!("exp", f64::exp);
    math1!("expm1", f64::exp_m1);
    math1!("log", f64::ln);
    math1!("log1p", f64::ln_1p);
    math1!("log2", f64::log2);
    math1!("log10", f64::log10);
    math1!("sin", f64::sin);
    math1!("cos", f64::cos);
    math1!("tan", f64::tan);
    math1!("asin", f64::asin);
    math1!("acos", f64::acos);
    math1!("atan", f64::atan);
    math1!("sinh", f64::sinh);
    math1!("cosh", f64::cosh);
    math1!("tanh", f64::tanh);
    math1!("fround", |n: f64| n as f32 as f64);
    math1!("clz32", |n: f64| (to_uint32(&Value::Number(n)) as u32).leading_zeros() as f64);

    def_method(realm, &m, "pow", 2, Rc::new(|_i, _t, args| {
        let a = crate::value::to_number(args.get(0).unwrap_or(&Value::Undefined));
        let b = crate::value::to_number(args.get(1).unwrap_or(&Value::Undefined));
        Ok(Value::Number(a.powf(b)))
    }));
    def_method(realm, &m, "atan2", 2, Rc::new(|_i, _t, args| {
        let y = crate::value::to_number(args.get(0).unwrap_or(&Value::Undefined));
        let x = crate::value::to_number(args.get(1).unwrap_or(&Value::Undefined));
        Ok(Value::Number(y.atan2(x)))
    }));
    def_method(realm, &m, "hypot", 2, Rc::new(|_i, _t, args| {
        let mut s = 0.0;
        for a in args { let n = crate::value::to_number(a); s += n * n; }
        Ok(Value::Number(s.sqrt()))
    }));
    def_method(realm, &m, "max", 2, Rc::new(|_i, _t, args| {
        if args.is_empty() { return Ok(Value::Number(f64::NEG_INFINITY)); }
        let mut m = f64::NEG_INFINITY;
        for a in args { let n = crate::value::to_number(a); if n.is_nan() { return Ok(Value::Number(f64::NAN)); } if n > m { m = n; } }
        Ok(Value::Number(m))
    }));
    def_method(realm, &m, "min", 2, Rc::new(|_i, _t, args| {
        if args.is_empty() { return Ok(Value::Number(f64::INFINITY)); }
        let mut m = f64::INFINITY;
        for a in args { let n = crate::value::to_number(a); if n.is_nan() { return Ok(Value::Number(f64::NAN)); } if n < m { m = n; } }
        Ok(Value::Number(m))
    }));
    def_method(realm, &m, "random", 0, Rc::new(|_i, _t, _a| {
        // simple PRNG (xorshift) seeded by time
        use std::cell::Cell;
        thread_local! { static STATE: Cell<u64> = Cell::new(0x9e3779b97f4a7c15); }
        let v = STATE.with(|s| {
            let mut x = s.get();
            if x == 0 { x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64 | 1; }
            x ^= x << 13; x ^= x >> 7; x ^= x << 17;
            s.set(x);
            x
        });
        Ok(Value::Number((v >> 11) as f64 / (1u64 << 53) as f64))
    }));
    def_method(realm, &m, "imul", 2, Rc::new(|_i, _t, args| {
        let a = to_int32(args.get(0).unwrap_or(&Value::Undefined));
        let b = to_int32(args.get(1).unwrap_or(&Value::Undefined));
        Ok(Value::Number((a.wrapping_mul(b)) as f64))
    }));

    def_const_value(&m, "PI", Value::Number(std::f64::consts::PI));
    def_const_value(&m, "E", Value::Number(std::f64::consts::E));
    def_const_value(&m, "LN2", Value::Number(std::f64::consts::LN_2));
    def_const_value(&m, "LN10", Value::Number(std::f64::consts::LN_10));
    def_const_value(&m, "LOG2E", Value::Number(std::f64::consts::LOG2_E));
    def_const_value(&m, "LOG10E", Value::Number(std::f64::consts::LOG10_E));
    def_const_value(&m, "SQRT2", Value::Number(std::f64::consts::SQRT_2));
    def_const_value(&m, "SQRT1_2", Value::Number(std::f64::consts::FRAC_1_SQRT_2));
    let _ = one;
    let _ = interp;
    crate::builtins::install_global(interp, realm, "Math", Value::Object(m));
}

fn round_half_up(n: f64) -> f64 {
    // ES Math.round: round half toward +Infinity
    if n.is_nan() || n.is_infinite() { return n; }
    let r = (n + 0.5).floor();
    // preserve -0
    if r == 0.0 && n < 0.0 { -0.0 } else { r }
}

fn sign(n: f64) -> f64 {
    if n.is_nan() { f64::NAN }
    else if n > 0.0 { 1.0 }
    else if n < 0.0 { -1.0 }
    else { n }
}
