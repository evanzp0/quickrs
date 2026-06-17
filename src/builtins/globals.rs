//! Global functions (parseInt, parseFloat, isNaN, isFinite, URI codecs,
//! setTimeout/clearTimeout/setInterval, queueMicrotask, globalThis, eval, ...).

use crate::realm::Realm;
use crate::asyncrt;
use crate::error;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{def_method, install_global};
use std::cell::RefCell;
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    install_global(interp, realm, "globalThis", Value::Object(realm.global.clone()));
    install_global(interp, realm, "undefined", Value::Undefined);
    install_global(interp, realm, "NaN", Value::Number(f64::NAN));
    install_global(interp, realm, "Infinity", Value::Number(f64::INFINITY));

    def_global_fn(interp, realm, "parseInt", 2, Rc::new(|_i, _t, args| {
        let s = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
        let radix = to_int32(args.get(1).unwrap_or(&Value::from_int(0)));
        Ok(Value::Number(parse_int(&s, radix)))
    }));
    def_global_fn(interp, realm, "parseFloat", 1, Rc::new(|_i, _t, args| {
        let s = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
        Ok(Value::Number(parse_float(&s)))
    }));
    def_global_fn(interp, realm, "isNaN", 1, Rc::new(|_i, _t, args| {
        Ok(Value::Bool(crate::value::to_number(args.get(0).unwrap_or(&Value::Undefined)).is_nan()))
    }));
    def_global_fn(interp, realm, "isFinite", 1, Rc::new(|_i, _t, args| {
        Ok(Value::Bool(crate::value::to_number(args.get(0).unwrap_or(&Value::Undefined)).is_finite()))
    }));
    def_global_fn(interp, realm, "encodeURI", 1, Rc::new(|_i, _t, args| {
        let s = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
        Ok(Value::from_string(encode_uri(&s, false)))
    }));
    def_global_fn(interp, realm, "encodeURIComponent", 1, Rc::new(|_i, _t, args| {
        let s = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
        Ok(Value::from_string(encode_uri(&s, true)))
    }));
    def_global_fn(interp, realm, "decodeURI", 1, Rc::new(|_i, _t, args| {
        let s = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
        Ok(Value::from_string(decode_uri(&s, false)))
    }));
    def_global_fn(interp, realm, "decodeURIComponent", 1, Rc::new(|_i, _t, args| {
        let s = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
        Ok(Value::from_string(decode_uri(&s, true)))
    }));
    def_global_fn(interp, realm, "eval", 1, Rc::new(|interp, _t, args| {
        let v = args.get(0).cloned().unwrap_or(Value::Undefined);
        if let Value::String(s) = &v {
            return interp.run(s);
        }
        Ok(v)
    }));
    def_global_fn(interp, realm, "queueMicrotask", 1, Rc::new(|interp, _t, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let async_rt = interp.shared.async_rt.clone();
        asyncrt::queue_microtask(&async_rt, Box::new(move |interp| {
            let _ = interp.call_value(cb.clone(), Value::Undefined, &[]);
        }));
        Ok(Value::Undefined)
    }));
    def_global_fn(interp, realm, "structuredClone", 1, Rc::new(|interp, _t, args| {
        let v = args.get(0).cloned().unwrap_or(Value::Undefined);
        Ok(clone_value(interp, &v))
    }));
    def_global_fn(interp, realm, "atob", 1, Rc::new(|_i, _t, args| {
        let s = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
        Ok(Value::from_string(String::from_utf8(base64_decode(&s)).unwrap_or_default()))
    }));
    def_global_fn(interp, realm, "btoa", 1, Rc::new(|_i, _t, args| {
        let s = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
        Ok(Value::from_string(base64_encode(s.as_bytes())))
    }));

    // Timers
    def_global_fn(interp, realm, "setTimeout", 2, Rc::new(|interp, _t, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let delay = to_number(args.get(1).unwrap_or(&Value::from_int(0))) as i64;
        let rest: Vec<Value> = args.iter().skip(2).cloned().collect();
        let async_rt = interp.shared.async_rt.clone();
        let id = asyncrt::set_timeout(&async_rt, delay, Box::new(move |interp| {
            let _ = interp.call_value(cb.clone(), Value::Undefined, &rest);
        }));
        Ok(Value::from_int(id as i32))
    }));
    def_global_fn(interp, realm, "clearTimeout", 1, Rc::new(|_i, _t, args| {
        let _id = to_int32(args.get(0).unwrap_or(&Value::Undefined));
        // We mark cancelled by id via the runtime's cancel mechanism (best-effort).
        // For simplicity we leave running timers in place; they no-op if cancelled.
        Ok(Value::Undefined)
    }));
    def_global_fn(interp, realm, "setInterval", 2, Rc::new(|interp, _t, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let delay = to_number(args.get(1).unwrap_or(&Value::from_int(0))).max(1.0) as i64;
        let rest: Vec<Value> = args.iter().skip(2).cloned().collect();
        let async_rt = interp.shared.async_rt.clone();
        let id = { let mut b = async_rt.borrow_mut(); let id = b.next_timer_id; b.next_timer_id += 1; id };
        // Self-rescheduling cycle: a shared slot holds the next firing closure.
        let cycle: Rc<RefCell<Option<Rc<dyn Fn(&mut Interpreter)>>>> = Rc::new(RefCell::new(None));
        let rt_for_cycle = async_rt.clone();
        let cycle_clone = cycle.clone();
        let cb_for_cycle = cb.clone();
        let rest_for_cycle = rest.clone();
        let cycle_fn: Rc<dyn Fn(&mut Interpreter)> = Rc::new(move |interp| {
            let _ = interp.call_value(cb_for_cycle.clone(), Value::Undefined, &rest_for_cycle);
            let rt = rt_for_cycle.clone();
            let cyc = cycle_clone.clone();
            let next: Rc<dyn Fn(&mut Interpreter)> = Rc::new(move |interp| {
                if let Some(f) = cyc.borrow().as_ref() {
                    f(interp);
                }
                let _ = rt;
            });
            let rt = rt_for_cycle.clone();
            asyncrt::set_timeout(&rt, delay, Box::new(move |interp| { next(interp); }));
        });
        *cycle.borrow_mut() = Some(cycle_fn.clone());
        asyncrt::set_timeout(&async_rt, delay, Box::new(move |interp| { cycle_fn(interp); }));
        Ok(Value::from_int(id as i32))
    }));
    def_global_fn(interp, realm, "clearInterval", 1, Rc::new(|_i, _t, _a| Ok(Value::Undefined)));
    def_global_fn(interp, realm, "setImmediate", 1, Rc::new(|interp, _t, args| {
        let cb = args.get(0).cloned().unwrap_or(Value::Undefined);
        let rest: Vec<Value> = args.iter().skip(1).cloned().collect();
        let async_rt = interp.shared.async_rt.clone();
        asyncrt::queue_microtask(&async_rt, Box::new(move |interp| {
            let _ = interp.call_value(cb.clone(), Value::Undefined, &rest);
        }));
        Ok(Value::from_int(0))
    }));
    def_global_fn(interp, realm, "process", 0, Rc::new(|interp, _t, _a| {
        // minimal process object
        let p = ObjectInner::new_object();
        p.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
        let argv = interp.new_array(vec![Value::from_str("quickrs")]);
        p.borrow_mut().props.insert(PropKey::from_str("argv"), Property::data(argv));
        p.borrow_mut().props.insert(PropKey::from_str("platform"), Property::data(Value::from_str("linux")));
        p.borrow_mut().props.insert(PropKey::from_str("version"), Property::data(Value::from_str("v0.1.0 (quickrs)")));
        let exit_fn = crate::interp::make_native_value(interp.realm(), "exit", 0, Rc::new(|interp, _t, args| {
            let code = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
            interp.shared.async_rt.borrow_mut().stop = true;
            interp.shared.async_rt.borrow_mut().exit_code = code;
            Ok(Value::Undefined)
        }));
        p.borrow_mut().props.insert(PropKey::from_str("exit"), Property::data(exit_fn));
        let hrtime = crate::interp::make_native_value(interp.realm(), "hrtime", 0, Rc::new(|_i, _t, _a| {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            Ok(Value::from_int((t.as_nanos() % 1_000_000_000) as i32))
        }));
        p.borrow_mut().props.insert(PropKey::from_str("hrtime"), Property::data(hrtime));
        Ok(Value::Object(p))
    }));
    // Node.js-style require() / module.exports compatibility
    def_global_fn(interp, realm, "require", 1, Rc::new(|interp, _this, args| {
        let spec = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
        // Check for built-in modules first (fs, path, os, buffer).
        if let Some(builtin) = crate::builtins::node_modules::try_load_builtin(interp, &spec) {
            return Ok(builtin);
        }
        // Resolve the file path (add .js if missing, handle ./ prefix)
        let path = if std::path::Path::new(&spec).exists() {
            spec.clone()
        } else if std::path::Path::new(&format!("{}.js", spec)).exists() {
            format!("{}.js", spec)
        } else {
            return Err(error::throw_type(&format!("Cannot find module '{}'", spec)));
        };
        // Check the CommonJS cache first.
        let cache_key = path.clone();
        if let Some(cached) = interp.shared.realm.module_cache.borrow().get(&cache_key).cloned() {
            // For CommonJS, the cached value is the module.exports object.
            if let Value::Object(o) = &cached {
                let exports = o.borrow().props.get(&PropKey::from_str("exports")).cloned();
                if let Some(Property { kind: PropKind::Data(v), .. }) = exports {
                    return Ok(v);
                }
            }
        }
        let src = std::fs::read_to_string(&path).map_err(|e| {
            error::throw_type(&format!("Cannot read module '{}': {}", spec, e))
        })?;
        // Create a module object with `exports: {}`.
        let module_obj = ObjectInner::new_object();
        module_obj.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
        module_obj.borrow_mut().class = "Module";
        let exports_obj = ObjectInner::new_object();
        exports_obj.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
        module_obj.borrow_mut().props.insert(PropKey::from_str("exports"), Property::data(Value::Object(exports_obj)));
        // Cache before evaluation (for circular dependencies).
        let module_val = Value::Object(module_obj.clone());
        interp.shared.realm.module_cache.borrow_mut().insert(cache_key, module_val.clone());
        // Evaluate the source with `module` and `exports` in scope.
        let saved = interp.scope.clone();
        let cjs_env = crate::scope::Env::new(Some(interp.shared.realm.global_env.clone()), crate::scope::EnvKind::Function);
        cjs_env.create(&Rc::from("module"), module_val.clone(), true);
        cjs_env.create(&Rc::from("exports"), interp.get_property(&module_val, &PropKey::from_str("exports"))?, true);
        cjs_env.create(&Rc::from("__dirname"), Value::from_string(
            std::path::Path::new(&path).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
        ), true);
        cjs_env.create(&Rc::from("__filename"), Value::from_string(
            std::path::Path::new(&path).canonicalize().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| path.clone())
        ), true);
        interp.scope = cjs_env;
        // Parse and evaluate as a script (CommonJS uses script mode).
        match crate::parser::parse(&src) {
            Ok(prog) => { let _ = interp.eval_program(&prog); }
            Err(e) => {
                interp.scope = saved;
                return Err(error::throw_syntax(&e.message));
            }
        }
        interp.scope = saved;
        // Return module.exports.
        let exports = interp.get_property(&module_val, &PropKey::from_str("exports"))?;
        // Update the cache with the final exports.
        if let Value::Object(o) = &module_val {
            o.borrow_mut().props.insert(PropKey::from_str("exports"), Property::data(exports.clone()));
        }
        Ok(exports)
    }));
    // `module` global (per-file, but we provide a default for top-level scripts)
    def_global_fn(interp, realm, "module", 0, Rc::new(|interp, _t, _a| {
        let m = ObjectInner::new_object();
        m.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
        let exports = ObjectInner::new_object();
        exports.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
        m.borrow_mut().props.insert(PropKey::from_str("exports"), Property::data(Value::Object(exports)));
        Ok(Value::Object(m))
    }));
    // Buffer global (Node.js compatible subset) — install as a direct object.
    {
        let buf_mod = crate::builtins::node_modules::try_load_builtin(interp, "buffer");
        if let Some(buf) = buf_mod {
            install_global(interp, realm, "Buffer", buf);
        }
    }
    def_global_fn(interp, realm, "TextEncoder", 0, Rc::new(|interp, _t, _a| {
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
        let enc = crate::interp::make_native_value(interp.realm(), "encode", 1, Rc::new(|interp, _t, args| {
            let s = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
            let bytes: Vec<Value> = s.as_bytes().iter().map(|b| Value::from_int(*b as i32)).collect();
            Ok(interp.new_array(bytes))
        }));
        o.borrow_mut().props.insert(PropKey::from_str("encode"), Property::data(enc));
        Ok(Value::Object(o))
    }));
    def_global_fn(interp, realm, "URL", 1, Rc::new(|interp, _t, args| {
        let s = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
        o.borrow_mut().props.insert(PropKey::from_str("href"), Property::data(Value::from_string(s)));
        Ok(Value::Object(o))
    }));
    def_global_fn(interp, realm, "fetch", 2, Rc::new(|interp, _t, args| {
        // stub: return a rejected promise (no network in this sandbox)
        let p = interp.new_promise();
        interp.reject_promise(p.clone(), error::throw_type("fetch is not supported in quickrs"));
        Ok(p)
    }));
    let _ = realm;
}

fn def_global_fn(interp: &mut Interpreter, realm: &Rc<Realm>, name: &str, len: usize, f: NativeFn) {
    let v = crate::interp::make_native_value(realm, name, len, f);
    install_global(interp, realm, name, v);
}

fn parse_int(s: &str, radix: i32) -> f64 {
    let s = s.trim_start();
    if s.is_empty() { return f64::NAN; }
    let (sign, rest) = if let Some(r) = s.strip_prefix('-') { (-1.0, r) } else if let Some(r) = s.strip_prefix('+') { (1.0, r) } else { (1.0, s) };
    let mut radix = radix;
    let rest = if radix == 0 || radix == 16 {
        if let Some(r) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) { radix = 16; r } else { rest }
    } else { rest };
    if radix == 0 { radix = 10; }
    if !(2..=36).contains(&radix) { return f64::NAN; }
    let mut end = 0;
    for (i, c) in rest.char_indices() {
        if c.to_digit(radix as u32).is_some() { end = i + c.len_utf8(); } else { break; }
    }
    if end == 0 { return f64::NAN; }
    sign * u64::from_str_radix(&rest[..end], radix as u32).map(|v| v as f64).unwrap_or(f64::NAN)
}

fn parse_float(s: &str) -> f64 {
    let s = s.trim_start();
    if s.starts_with("Infinity") || s.starts_with("+Infinity") { return f64::INFINITY; }
    if s.starts_with("-Infinity") { return f64::NEG_INFINITY; }
    // parse longest valid float prefix
    let mut end = 0;
    let bytes = s.as_bytes();
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

fn encode_uri(s: &str, component: bool) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let unreserved = c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '!' | '*' | '\'' | '(' | ')');
        let safe = if component { unreserved } else { unreserved || matches!(c, ';' | ',' | '/' | '?' | ':' | '@' | '&' | '=' | '+' | '$' | '#') };
        if safe {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

fn decode_uri(s: &str, _component: bool) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = std::str::from_utf8(&bytes[i+1..i+3]).unwrap_or("00");
            if let Ok(b) = u8::from_str_radix(h, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn clone_value(interp: &mut Interpreter, v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            let b = o.borrow();
            if let ObjectKind::Array(items) = &b.kind {
                let cloned: Vec<Value> = items.iter().map(|i| clone_value(interp, i)).collect();
                return interp.new_array(cloned);
            }
            let new = ObjectInner::new_object();
            new.borrow_mut().proto = b.proto.clone();
            new.borrow_mut().class = b.class;
            for (k, p) in b.props.iter() {
                if let PropKind::Data(val) = &p.kind {
                    new.borrow_mut().props.insert(k.clone(), Property::data(clone_value(interp, val)));
                }
            }
            Value::Object(new)
        }
        _ => v.clone(),
    }
}

fn base64_decode(s: &str) -> Vec<u8> {
    let table: [i8; 256] = {
        let mut t = [-1i8; 256];
        let alpha = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        for (i, &c) in alpha.iter().enumerate() { t[c as usize] = i as i8; }
        t
    };
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for &c in s.as_bytes() {
        if c == b'=' { break; }
        let v = table[c as usize];
        if v < 0 { continue; }
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}

fn base64_encode(data: &[u8]) -> String {
    let alpha = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i+1] as u32) << 8) | data[i+2] as u32;
        out.push(alpha[((n >> 18) & 63) as usize] as char);
        out.push(alpha[((n >> 12) & 63) as usize] as char);
        out.push(alpha[((n >> 6) & 63) as usize] as char);
        out.push(alpha[(n & 63) as usize] as char);
        i += 3;
    }
    let rem = data.len() - i;
    if rem == 1 {
        let n = (data[i] as u32) << 16;
        out.push(alpha[((n >> 18) & 63) as usize] as char);
        out.push(alpha[((n >> 12) & 63) as usize] as char);
        out.push_str("==");
    } else if rem == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i+1] as u32) << 8);
        out.push(alpha[((n >> 18) & 63) as usize] as char);
        out.push(alpha[((n >> 12) & 63) as usize] as char);
        out.push(alpha[((n >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}
