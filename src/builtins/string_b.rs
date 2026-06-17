//! String constructor + String.prototype.

use crate::realm::Realm;
use crate::error;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{make_ctor, install_global_ctor, def_method, CtorFn};
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let call_fn: NativeFn = Rc::new(|_interp, _this, args| {
        match args.get(0) {
            Some(Value::Undefined) | None => Ok(Value::from_str("")),
            Some(v) => Ok(Value::from_string(crate::value::to_string(v))),
        }
    });
    let ctor_fn: CtorFn = Rc::new(|interp, _this, args, _nt| {
        let s = match args.get(0) {
            Some(Value::Undefined) | None => Rc::from(""),
            Some(v) => Rc::from(crate::value::to_string(v).as_str()),
        };
        Ok(Value::Object(interp_string_wrapper(interp.realm(), s)))
    });
    let ctor = make_ctor(realm, "String", 1, call_fn, ctor_fn);
    install_global_ctor(interp, realm, "String", ctor.clone(), realm.string_proto.clone());
    if let Value::Object(co) = &ctor {
        let co = co.clone();
        def_method(realm, &co, "fromCharCode", 1, Rc::new(|_interp, _this, args| {
            let mut s = String::new();
            for a in args {
                let code = to_int32(a) as u32;
                if let Some(c) = char::from_u32(code & 0xffff) { s.push(c); }
            }
            Ok(Value::from_string(s))
        }));
        def_method(realm, &co, "fromCodePoint", 1, Rc::new(|_interp, _this, args| {
            let mut s = String::new();
            for a in args {
                let code = to_int32(a) as u32;
                if let Some(c) = char::from_u32(code) { s.push(c); }
            }
            Ok(Value::from_string(s))
        }));
        def_method(realm, &co, "raw", 1, Rc::new(|interp, _this, args| {
            let tpl = args.get(0).cloned().unwrap_or(Value::Undefined);
            let raw = interp.get_property(&tpl, &PropKey::from_str("raw"))?;
            let items = interp.iterable_to_vec(&raw)?;
            let subs: Vec<Value> = args.iter().skip(1).cloned().collect();
            let mut s = String::new();
            for (i, it) in items.iter().enumerate() {
                s.push_str(&interp.coerce_to_string(it)?);
                if i + 1 < items.len() {
                    if let Some(v) = subs.get(i) { s.push_str(&interp.coerce_to_string(v)?); }
                }
            }
            Ok(Value::from_string(s))
        }));
    }
    let sp = realm.string_proto.clone();
    def_method(realm, &sp, "charAt", 1, Rc::new(|_interp, this, args| {
        let s = this_as_string(&this);
        let i = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
        if i < 0 { return Ok(Value::from_str("")); }
        match s.chars().nth(i as usize) { Some(c) => Ok(Value::from_string(c.to_string())), None => Ok(Value::from_str("")) }
    }));
    def_method(realm, &sp, "charCodeAt", 1, Rc::new(|_interp, this, args| {
        let s = this_as_string(&this);
        let i = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
        if i < 0 { return Ok(Value::Number(f64::NAN)); }
        match s.chars().nth(i as usize) { Some(c) => Ok(Value::Number(c as u32 as f64)), None => Ok(Value::Number(f64::NAN)) }
    }));
    def_method(realm, &sp, "codePointAt", 1, Rc::new(|_interp, this, args| {
        let s = this_as_string(&this);
        let i = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
        if i < 0 { return Ok(Value::Undefined); }
        match s.chars().nth(i as usize) { Some(c) => Ok(Value::Number(c as u32 as f64)), None => Ok(Value::Undefined) }
    }));
    def_method(realm, &sp, "at", 1, Rc::new(|_interp, this, args| {
        let s = this_as_string(&this);
        let chars: Vec<char> = s.chars().collect();
        let idx = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
        let i = if idx < 0 { chars.len() as i32 + idx } else { idx };
        if i >= 0 && (i as usize) < chars.len() { Ok(Value::from_string(chars[i as usize].to_string())) } else { Ok(Value::Undefined) }
    }));
    def_method(realm, &sp, "concat", 1, Rc::new(|interp, this, args| {
        let mut s = interp.coerce_to_string(&this)?.to_string();
        for a in args { s.push_str(&interp.coerce_to_string(a)?); }
        Ok(Value::from_string(s))
    }));
    def_method(realm, &sp, "includes", 1, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let needle = interp.coerce_to_string(args.get(0).unwrap_or(&Value::Undefined))?;
        Ok(Value::Bool(s.contains(&*needle)))
    }));
    def_method(realm, &sp, "startsWith", 1, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let needle = interp.coerce_to_string(args.get(0).unwrap_or(&Value::Undefined))?;
        Ok(Value::Bool(s.starts_with(&*needle)))
    }));
    def_method(realm, &sp, "endsWith", 1, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let needle = interp.coerce_to_string(args.get(0).unwrap_or(&Value::Undefined))?;
        Ok(Value::Bool(s.ends_with(&*needle)))
    }));
    def_method(realm, &sp, "indexOf", 1, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let needle = interp.coerce_to_string(args.get(0).unwrap_or(&Value::Undefined))?;
        Ok(s.find(&*needle).map(|i| Value::from_int(i as i32)).unwrap_or(Value::from_int(-1)))
    }));
    def_method(realm, &sp, "lastIndexOf", 1, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let needle = interp.coerce_to_string(args.get(0).unwrap_or(&Value::Undefined))?;
        Ok(s.rfind(&*needle).map(|i| Value::from_int(i as i32)).unwrap_or(Value::from_int(-1)))
    }));
    def_method(realm, &sp, "slice", 2, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let chars: Vec<char> = s.chars().collect();
        let (s_, e) = crate::builtins::array::normalize_slice(args.get(0), args.get(1), chars.len());
        let out: String = chars[s_..e].iter().collect();
        Ok(Value::from_string(out))
    }));
    def_method(realm, &sp, "substring", 2, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len();
        let mut a = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
        let mut b = match args.get(1) { Some(Value::Undefined) | None => len as i32, Some(v) => to_int32(v) };
        if a < 0 || a.is_negative() { a = 0; }
        if b < 0 { b = 0; }
        if a > len as i32 { a = len as i32; }
        if b > len as i32 { b = len as i32; }
        if a > b { std::mem::swap(&mut a, &mut b); }
        let out: String = chars[a as usize..b as usize].iter().collect();
        Ok(Value::from_string(out))
    }));
    def_method(realm, &sp, "substr", 2, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len();
        let start = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
        let start = if start < 0 { (len as i32 + start).max(0) as usize } else { (start as usize).min(len) };
        let length = to_int32(args.get(1).unwrap_or(&Value::from_int(len as i32)));
        let length = if length < 0 { 0 } else { length as usize };
        let end = (start + length).min(len);
        let out: String = chars[start..end].iter().collect();
        Ok(Value::from_string(out))
    }));
    def_method(realm, &sp, "toLowerCase", 0, Rc::new(|interp, this, _args| {
        Ok(Value::from_string(interp.coerce_to_string(&this)?.to_lowercase()))
    }));
    def_method(realm, &sp, "toUpperCase", 0, Rc::new(|interp, this, _args| {
        Ok(Value::from_string(interp.coerce_to_string(&this)?.to_uppercase()))
    }));
    def_method(realm, &sp, "toLocaleLowerCase", 0, Rc::new(|interp, this, _args| {
        Ok(Value::from_string(interp.coerce_to_string(&this)?.to_lowercase()))
    }));
    def_method(realm, &sp, "toLocaleUpperCase", 0, Rc::new(|interp, this, _args| {
        Ok(Value::from_string(interp.coerce_to_string(&this)?.to_uppercase()))
    }));
    def_method(realm, &sp, "trim", 0, Rc::new(|interp, this, _args| {
        Ok(Value::from_string(interp.coerce_to_string(&this)?.trim().to_string()))
    }));
    def_method(realm, &sp, "trimStart", 0, Rc::new(|interp, this, _args| {
        Ok(Value::from_string(interp.coerce_to_string(&this)?.trim_start().to_string()))
    }));
    def_method(realm, &sp, "trimEnd", 0, Rc::new(|interp, this, _args| {
        Ok(Value::from_string(interp.coerce_to_string(&this)?.trim_end().to_string()))
    }));
    def_method(realm, &sp, "repeat", 1, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let n = to_int32(args.get(0).unwrap_or(&Value::from_int(0)));
        if n < 0 { return Err(error::throw_range("Invalid count value")); }
        Ok(Value::from_string(s.repeat(n as usize)))
    }));
    def_method(realm, &sp, "padStart", 2, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let target = to_int32(args.get(0).unwrap_or(&Value::from_int(0))) as usize;
        let pad = match args.get(1) { Some(Value::Undefined) | None => " ".to_string(), Some(v) => interp.coerce_to_string(v)?.to_string() };
        Ok(Value::from_string(pad_str(&s, target, &pad, true)))
    }));
    def_method(realm, &sp, "padEnd", 2, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let target = to_int32(args.get(0).unwrap_or(&Value::from_int(0))) as usize;
        let pad = match args.get(1) { Some(Value::Undefined) | None => " ".to_string(), Some(v) => interp.coerce_to_string(v)?.to_string() };
        Ok(Value::from_string(pad_str(&s, target, &pad, false)))
    }));
    def_method(realm, &sp, "split", 2, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let sep = args.get(0).cloned().unwrap_or(Value::Undefined);
        let limit = match args.get(1) { Some(Value::Undefined) | None => usize::MAX, Some(v) => to_int32(v) as usize };
        if matches!(sep, Value::Undefined) {
            return Ok(interp.new_array(vec![Value::String(s)]));
        }
        // regex separator
        if let Value::Object(o) = &sep {
            if matches!(o.borrow().kind, ObjectKind::RegExp(_)) {
                let re = if let ObjectKind::RegExp(d) = &o.borrow().kind { d.re.clone() } else { unreachable!() };
                let mut out = Vec::new();
                let mut last = 0;
                for m in re.find_iter(&s) {
                    if m.start() > last {
                        out.push(Value::from_string(s[last..m.start()].to_string()));
                        if out.len() >= limit { return Ok(interp.new_array(out)); }
                    } else if m.start() == last && m.start() == 0 {
                        // leading empty match: push empty
                    }
                    last = m.end();
                }
                out.push(Value::from_string(s[last..].to_string()));
                out.truncate(limit);
                return Ok(interp.new_array(out));
            }
        }
        let sep_s = interp.coerce_to_string(&sep)?;
        if sep_s.is_empty() {
            let chars: Vec<Value> = s.chars().map(|c| Value::from_string(c.to_string())).collect();
            let mut c = chars; c.truncate(limit);
            return Ok(interp.new_array(c));
        }
        let parts: Vec<Value> = s.split(&*sep_s).take(limit).map(|p| Value::from_string(p.to_string())).collect();
        Ok(interp.new_array(parts))
    }));
    def_method(realm, &sp, "replace", 2, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let pat = args.get(0).cloned().unwrap_or(Value::Undefined);
        let repl = args.get(1).cloned().unwrap_or(Value::Undefined);
        // `replace` replaces all when the regex has the global flag.
        let all = if let Value::Object(o) = &pat {
            if let ObjectKind::RegExp(d) = &o.borrow().kind { d.global } else { false }
        } else { false };
        string_replace(interp, &s, &pat, &repl, all)
    }));
    def_method(realm, &sp, "replaceAll", 2, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let pat = args.get(0).cloned().unwrap_or(Value::Undefined);
        let repl = args.get(1).cloned().unwrap_or(Value::Undefined);
        string_replace(interp, &s, &pat, &repl, true)
    }));
    def_method(realm, &sp, "match", 1, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let re = args.get(0).cloned().unwrap_or(Value::Undefined);
        let reobj = ensure_regexp(interp, &re)?;
        // If the regex has the global flag, return an array of all full matches.
        let is_global = if let Value::Object(o) = &reobj {
            if let ObjectKind::RegExp(d) = &o.borrow().kind { d.global } else { false }
        } else { false };
        if is_global {
            let d = if let Value::Object(o) = &reobj {
                if let ObjectKind::RegExp(d) = &o.borrow().kind { d.clone() } else { return Err(error::throw_type("not a regexp")); }
            } else { return Err(error::throw_type("not a regexp")); };
            let mut out = Vec::new();
            // Use fancy-regex if available, else regex crate.
            let s_ref: &str = &s;
            if let Some(fr) = &d.fancy {
                for m in fr.find_iter(s_ref) {
                    if let Ok(m) = m {
                        out.push(Value::from_string(m.as_str().to_string()));
                    }
                }
            } else {
                for m in d.re.find_iter(s_ref) {
                    out.push(Value::from_string(m.as_str().to_string()));
                }
            }
            if out.is_empty() { return Ok(Value::Null); }
            return Ok(interp.new_array(out));
        }
        exec_regexp(interp, &reobj, &s)
    }));
    def_method(realm, &sp, "matchAll", 1, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let re = args.get(0).cloned().unwrap_or(Value::Undefined);
        let reobj = ensure_regexp(interp, &re)?;
        if let Value::Object(o) = &reobj {
            if let ObjectKind::RegExp(d) = &o.borrow().kind {
                if !d.global { return Err(error::throw_type("matchAll requires global flag")); }
                let mut out = Vec::new();
                for m in d.re.find_iter(&s) {
                    let arr = interp.new_array(vec![Value::from_string(m.as_str().to_string())]);
                    out.push(arr);
                }
                return Ok(interp.new_array(out));
            }
        }
        Ok(interp.new_array(vec![]))
    }));
    def_method(realm, &sp, "search", 1, Rc::new(|interp, this, args| {
        let s = interp.coerce_to_string(&this)?;
        let re = args.get(0).cloned().unwrap_or(Value::Undefined);
        let reobj = ensure_regexp(interp, &re)?;
        if let Value::Object(o) = &reobj {
            if let ObjectKind::RegExp(d) = &o.borrow().kind {
                return Ok(d.re.find(&s).map(|m| Value::from_int(m.start() as i32)).unwrap_or(Value::from_int(-1)));
            }
        }
        Ok(Value::from_int(-1))
    }));
    def_method(realm, &sp, "normalize", 0, Rc::new(|interp, this, _args| {
        Ok(Value::String(interp.coerce_to_string(&this)?))
    }));
    def_method(realm, &sp, "localeCompare", 1, Rc::new(|interp, this, args| {
        let a = interp.coerce_to_string(&this)?;
        let b = interp.coerce_to_string(args.get(0).unwrap_or(&Value::Undefined))?;
        Ok(Value::from_int(a.cmp(&b) as i32))
    }));
    def_method(realm, &sp, "toString", 0, Rc::new(|_interp, this, _args| {
        Ok(Value::String(this_as_string(&this)))
    }));
    def_method(realm, &sp, "valueOf", 0, Rc::new(|_interp, this, _args| {
        Ok(Value::String(this_as_string(&this)))
    }));
    let _ = interp;
}

fn interp_string_wrapper(realm: &Rc<Realm>, s: Rc<str>) -> ObjRef {
    let o = ObjectInner::new_object();
    o.borrow_mut().proto = Some(Value::Object(realm.string_proto.clone()));
    o.borrow_mut().class = "String";
    o.borrow_mut().kind = ObjectKind::String(s.clone());
    o.borrow_mut().props.insert(PropKey::from_str("length"), Property {
        kind: PropKind::Data(Value::from_int(s.chars().count() as i32)),
        writable: false, enumerable: false, configurable: false,
    });
    o
}

fn this_as_string(v: &Value) -> Rc<str> {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(o) => {
            if let ObjectKind::String(s) = &o.borrow().kind { s.clone() }
            else { Rc::from(crate::value::to_string(v).as_str()) }
        }
        _ => Rc::from(crate::value::to_string(v).as_str()),
    }
}

fn pad_str(s: &str, target: usize, pad: &str, start: bool) -> String {
    if s.chars().count() >= target || pad.is_empty() { return s.to_string(); }
    let need = target - s.chars().count();
    let pad_chars: Vec<char> = pad.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    if start {
        for _ in 0..need { out.push(pad_chars[i % pad_chars.len()]); i += 1; }
        out.push_str(s);
    } else {
        out.push_str(s);
        for _ in 0..need { out.push(pad_chars[i % pad_chars.len()]); i += 1; }
    }
    out
}

fn ensure_regexp(interp: &mut Interpreter, re: &Value) -> Result<Value, Value> {
    if let Value::Object(o) = re {
        if matches!(o.borrow().kind, ObjectKind::RegExp(_)) { return Ok(re.clone()); }
    }
    let s = interp.coerce_to_string(re)?;
    interp.get_property(&Value::Object(interp.realm().regexp_proto.clone()), &PropKey::from_str("constructor"))
        .and_then(|c| interp.construct(c.clone(), &[Value::String(s)], c))
}

fn exec_regexp(interp: &mut Interpreter, re: &Value, s: &str) -> Result<Value, Value> {
    if let Value::Object(o) = re {
        if let ObjectKind::RegExp(d) = &o.borrow().kind {
            // Prefer fancy-regex (supports backrefs/lookaround) when available.
            if let Some(fr) = &d.fancy {
                if let Ok(Some(caps)) = fr.captures(s) {
                    let mut items = Vec::new();
                    for i in 0..caps.len() {
                        match caps.get(i) {
                            Some(m) => items.push(Value::from_string(m.as_str().to_string())),
                            None => items.push(Value::Undefined),
                        }
                    }
                    let arr = interp.new_array(items);
                    let start = caps.get(0).map(|m| m.start()).unwrap_or(0);
                    if let Value::Object(ao) = &arr {
                        ao.borrow_mut().props.insert(PropKey::from_str("index"), Property::data(Value::from_int(start as i32)));
                        ao.borrow_mut().props.insert(PropKey::from_str("input"), Property::data(Value::from_string(s.to_string())));
                    }
                    return Ok(arr);
                }
                return Ok(Value::Null);
            }
            if let Some(caps) = d.re.captures(s) {
                let mut items = Vec::new();
                for i in 0..caps.len() {
                    match caps.get(i) {
                        Some(m) => items.push(Value::from_string(m.as_str().to_string())),
                        None => items.push(Value::Undefined),
                    }
                }
                let arr = interp.new_array(items);
                let start = caps.get(0).map(|m| m.start()).unwrap_or(0);
                if let Value::Object(ao) = &arr {
                    ao.borrow_mut().props.insert(PropKey::from_str("index"), Property::data(Value::from_int(start as i32)));
                    ao.borrow_mut().props.insert(PropKey::from_str("input"), Property::data(Value::from_string(s.to_string())));
                }
                return Ok(arr);
            }
            return Ok(Value::Null);
        }
    }
    Ok(Value::Null)
}

fn string_replace(interp: &mut Interpreter, s: &str, pat: &Value, repl: &Value, all: bool) -> Result<Value, Value> {
    let is_re = matches!(pat, Value::Object(o) if matches!(o.borrow().kind, ObjectKind::RegExp(_)));
    if is_re {
        let d = if let Value::Object(o) = pat { if let ObjectKind::RegExp(d) = &o.borrow().kind { d.clone() } else { unreachable!() } } else { unreachable!() };
        let mut result = String::new();
        let mut last = 0;
        // Use fancy-regex if available (supports backrefs/lookaround), else regex crate.
        let matches: Vec<(usize, usize, Vec<Option<String>>)> = if let Some(fr) = &d.fancy {
            let mut out = Vec::new();
            for m in fr.captures_iter(s) {
                if let Ok(caps) = m {
                    let full = caps.get(0);
                    if let Some(f) = full {
                        let groups: Vec<Option<String>> = (0..caps.len()).map(|i| {
                            caps.get(i).map(|m| m.as_str().to_string())
                        }).collect();
                        out.push((f.start(), f.end(), groups));
                    }
                }
            }
            out
        } else {
            let mut out = Vec::new();
            for caps in d.re.captures_iter(s) {
                let full = caps.get(0);
                let (start, end) = match full {
                    Some(m) => (m.start(), m.end()),
                    None => (0, 0),
                };
                let groups: Vec<Option<String>> = (0..caps.len()).map(|i| {
                    caps.get(i).map(|m| m.as_str().to_string())
                }).collect();
                out.push((start, end, groups));
            }
            out
        };
        for (start, end, groups) in matches {
            result.push_str(&s[last..start]);
            let matched = groups.get(0).and_then(|g| g.clone()).unwrap_or_default();
            result.push_str(&apply_replacement_with_captures(interp, repl, &matched, start, &groups)?);
            last = end;
            if !all { break; }
        }
        result.push_str(&s[last..]);
        return Ok(Value::from_string(result));
    }
    // string pattern
    let needle = interp.coerce_to_string(pat)?;
    if needle.is_empty() { return Ok(Value::from_string(s.to_string())); }
    if all {
        let r = apply_replacement(interp, repl, &needle, 0)?;
        return Ok(Value::from_string(s.replace(&*needle, &r)));
    } else {
        if let Some(idx) = s.find(&*needle) {
            let r = apply_replacement(interp, repl, &needle, idx)?;
            let mut out = String::with_capacity(s.len() + r.len());
            out.push_str(&s[..idx]);
            out.push_str(&r);
            out.push_str(&s[idx + needle.len()..]);
            return Ok(Value::from_string(out));
        }
        Ok(Value::from_string(s.to_string()))
    }
}

fn apply_replacement(interp: &mut Interpreter, repl: &Value, matched: &str, _offset: usize) -> Result<String, Value> {
    apply_replacement_with_captures(interp, repl, matched, _offset, &[])
}

fn apply_replacement_with_captures(
    interp: &mut Interpreter,
    repl: &Value,
    matched: &str,
    _offset: usize,
    groups: &[Option<String>],
) -> Result<String, Value> {
    if repl.is_callable() {
        let mut args = vec![Value::from_string(matched.to_string())];
        for g in groups.iter().skip(1) {
            args.push(match g {
                Some(s) => Value::from_string(s.clone()),
                None => Value::Undefined,
            });
        }
        let r = interp.call_value(repl.clone(), Value::Undefined, &args)?;
        Ok(interp.coerce_to_string(&r)?.to_string())
    } else {
        let r = interp.coerce_to_string(repl)?;
        // Handle $1, $2, ..., $&, $$, $`, $'
        let mut out = String::new();
        let chars: Vec<char> = r.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '$' && i + 1 < chars.len() {
                let next = chars[i + 1];
                match next {
                    '&' => { out.push_str(matched); i += 2; }
                    '$' => { out.push('$'); i += 2; }
                    '`' => { i += 2; }
                    '\'' => { i += 2; }
                    d if d.is_ascii_digit() => {
                        let mut n = (d as u8 - b'0') as usize;
                        let mut consumed = 2;
                        // Check for second digit (e.g. $12)
                        if i + 2 < chars.len() {
                            let d2 = chars[i + 2];
                            if d2.is_ascii_digit() {
                                let n2 = n * 10 + (d2 as u8 - b'0') as usize;
                                if n2 < groups.len() {
                                    n = n2;
                                    consumed = 3;
                                }
                            }
                        }
                        if n < groups.len() {
                            if let Some(g) = &groups[n] {
                                out.push_str(g);
                            }
                        }
                        i += consumed;
                    }
                    _ => { out.push('$'); i += 1; }
                }
            } else {
                out.push(c);
                i += 1;
            }
        }
        Ok(out)
    }
}
