//! JSON.parse / JSON.stringify.

use crate::realm::Realm;
use crate::error;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{def_method, install_global};
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let j = ObjectInner::new_object();
    j.borrow_mut().proto = Some(Value::Object(realm.object_proto.clone()));
    j.borrow_mut().class = "JSON";
    def_method(realm, &j, "parse", 2, Rc::new(|interp, _this, args| {
        let s = interp.coerce_to_string(args.get(0).unwrap_or(&Value::Undefined))?;
        let reviver = args.get(1).cloned();
        let v = json_parse(&s)?;
        if let Some(r) = reviver { if r.is_callable() {
            let holder = ObjectInner::new_object();
            holder.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
            holder.borrow_mut().props.insert(PropKey::from_str(""), Property::data(v));
            return walk(interp, &Value::Object(holder), &PropKey::from_str(""), &r);
        }}
        Ok(v)
    }));
    def_method(realm, &j, "stringify", 3, Rc::new(|interp, _this, args| {
        let value = args.get(0).cloned().unwrap_or(Value::Undefined);
        let replacer = args.get(1).cloned().unwrap_or(Value::Undefined);
        let space = args.get(2).cloned().unwrap_or(Value::Undefined);
        let indent = compute_indent(&space);
        let mut out = String::new();
        let res = stringify_value(interp, &value, &replacer, &indent, "", &mut Vec::new())?;
        match res {
            Some(s) => out.push_str(&s),
            None => return Ok(Value::Undefined),
        }
        Ok(Value::from_string(out))
    }));
    let _ = interp;
    install_global(interp, realm, "JSON", Value::Object(j));
}

fn json_parse(s: &str) -> Result<Value, Value> {
    let mut p = JsonParser { src: s.as_bytes(), pos: 0 };
    p.ws();
    let v = p.value()?;
    p.ws();
    Ok(v)
}

struct JsonParser<'a> { src: &'a [u8], pos: usize }
impl<'a> JsonParser<'a> {
    fn ws(&mut self) {
        while let Some(&c) = self.src.get(self.pos) {
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' { self.pos += 1; } else { break; }
        }
    }
    fn value(&mut self) -> Result<Value, Value> {
        self.ws();
        match self.src.get(self.pos).copied() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => { self.expect_lit("true")?; Ok(Value::Bool(true)) }
            Some(b'f') => { self.expect_lit("false")?; Ok(Value::Bool(false)) }
            Some(b'n') => { self.expect_lit("null")?; Ok(Value::Null) }
            Some(b'-') | Some(b'0'..=b'9') => self.number(),
            _ => Err(error::throw_syntax("unexpected token in JSON")),
        }
    }
    fn expect_lit(&mut self, lit: &str) -> Result<(), Value> {
        if self.src[self.pos..].starts_with(lit.as_bytes()) { self.pos += lit.len(); Ok(()) }
        else { Err(error::throw_syntax("invalid literal in JSON")) }
    }
    fn number(&mut self) -> Result<Value, Value> {
        let start = self.pos;
        if self.src.get(self.pos) == Some(&b'-') { self.pos += 1; }
        while let Some(&c) = self.src.get(self.pos) {
            if c.is_ascii_digit() || c == b'.' || c == b'e' || c == b'E' || c == b'+' || c == b'-' { self.pos += 1; } else { break; }
        }
        let s = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("0");
        s.parse::<f64>().map(Value::Number).map_err(|_| error::throw_syntax("invalid number in JSON"))
    }
    fn string(&mut self) -> Result<Rc<str>, Value> {
        self.pos += 1; // opening quote
        let mut s = String::new();
        loop {
            match self.src.get(self.pos).copied() {
                None => return Err(error::throw_syntax("unterminated string in JSON")),
                Some(b'"') => { self.pos += 1; break; }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.src.get(self.pos).copied() {
                        Some(b'"') => s.push('"'),
                        Some(b'\\') => s.push('\\'),
                        Some(b'/') => s.push('/'),
                        Some(b'n') => s.push('\n'),
                        Some(b't') => s.push('\t'),
                        Some(b'r') => s.push('\r'),
                        Some(b'b') => s.push('\u{8}'),
                        Some(b'f') => s.push('\u{c}'),
                        Some(b'u') => {
                            self.pos += 1;
                            let h = std::str::from_utf8(&self.src[self.pos..self.pos+4]).unwrap_or("0000");
                            let v = u32::from_str_radix(h, 16).unwrap_or(0);
                            self.pos += 4;
                            if let Some(ch) = char::from_u32(v) { s.push(ch); }
                            continue;
                        }
                        _ => return Err(error::throw_syntax("bad escape in JSON")),
                    }
                    self.pos += 1;
                }
                Some(_) => {
                    let start = self.pos;
                    while let Some(&c) = self.src.get(self.pos) { if c == b'\\' || c == b'"' { break; } self.pos += 1; }
                    s.push_str(std::str::from_utf8(&self.src[start..self.pos]).unwrap_or(""));
                }
            }
        }
        Ok(Rc::from(s.as_str()))
    }
    fn array(&mut self) -> Result<Value, Value> {
        self.pos += 1;
        let mut items = Vec::new();
        self.ws();
        if self.src.get(self.pos) == Some(&b']') { self.pos += 1; return Ok(Value::Object(make_array(items))); }
        loop {
            items.push(self.value()?);
            self.ws();
            match self.src.get(self.pos).copied() {
                Some(b',') => { self.pos += 1; }
                Some(b']') => { self.pos += 1; break; }
                _ => return Err(error::throw_syntax("expected , or ] in JSON")),
            }
        }
        Ok(Value::Object(make_array(items)))
    }
    fn object(&mut self) -> Result<Value, Value> {
        self.pos += 1;
        let o = ObjectInner::new_object();
        o.borrow_mut().class = "Object";
        self.ws();
        if self.src.get(self.pos) == Some(&b'}') { self.pos += 1; return Ok(Value::Object(o)); }
        loop {
            self.ws();
            if self.src.get(self.pos) != Some(&b'"') { return Err(error::throw_syntax("expected string key in JSON")); }
            let k = self.string()?;
            self.ws();
            if self.src.get(self.pos) != Some(&b':') { return Err(error::throw_syntax("expected : in JSON")); }
            self.pos += 1;
            let v = self.value()?;
            o.borrow_mut().props.insert(PropKey::Str(k), Property::data(v));
            self.ws();
            match self.src.get(self.pos).copied() {
                Some(b',') => { self.pos += 1; }
                Some(b'}') => { self.pos += 1; break; }
                _ => return Err(error::throw_syntax("expected , or } in JSON")),
            }
        }
        Ok(Value::Object(o))
    }
}

fn make_array(items: Vec<Value>) -> ObjRef {
    let o = ObjectInner::new_array(items);
    o
}

fn walk(interp: &mut Interpreter, holder: &Value, key: &PropKey, reviver: &Value) -> Result<Value, Value> {
    let v = interp.get_property(holder, key)?;
    if let Value::Object(o) = &v {
        let is_arr = matches!(o.borrow().kind, ObjectKind::Array(_));
        let keys: Vec<PropKey> = {
            let b = o.borrow();
            let mut ks = Vec::new();
            if let ObjectKind::Array(items) = &b.kind {
                for i in 0..items.len() { ks.push(PropKey::Str(crate::value::index_to_key(i))); }
            }
            for (k, _) in b.props.iter() { ks.push(k.clone()); }
            ks
        };
        for k in keys {
            let nv = walk(interp, &v, &k, reviver)?;
            if nv.is_undefined() {
                if let Value::Object(oo) = &v {
                    if matches!(oo.borrow().kind, ObjectKind::Array(_)) && matches!(k, PropKey::Str(_)) {
                        // can't easily remove from array fast path; set undefined
                        let _ = interp.set_property(&v, &k, Value::Undefined);
                    } else {
                        oo.borrow_mut().props.remove(&k);
                    }
                }
            } else {
                interp.set_property(&v, &k, nv)?;
            }
        }
        let _ = is_arr;
    }
    interp.call_value(reviver.clone(), holder.clone(), &[value_from_key(key), v])
}

fn value_from_key(k: &PropKey) -> Value {
    match k { PropKey::Str(s) => Value::String(s.clone()), PropKey::Sym(s) => Value::Symbol(s.clone()) }
}

fn compute_indent(space: &Value) -> String {
    match space {
        Value::Number(n) => " ".repeat((*n as usize).min(10)),
        Value::String(s) => s.chars().take(10).collect(),
        _ => String::new(),
    }
}

fn stringify_value(
    interp: &mut Interpreter,
    value: &Value,
    replacer: &Value,
    indent: &str,
    gap: &str,
    seen: &mut Vec<usize>,
) -> Result<Option<String>, Value> {
    let v = if let Value::Object(o) = value {
        let b = o.borrow();
        match &b.kind {
            ObjectKind::Number(n) => Value::Number(*n),
            ObjectKind::String(s) => Value::String(s.clone()),
            ObjectKind::Boolean(bl) => Value::Bool(*bl),
            _ => value.clone(),
        }
    } else { value.clone() };
    // toJSON
    if let Value::Object(_) = &v {
        let tj = interp.get_property(&v, &PropKey::from_str("toJSON"))?;
        if tj.is_callable() {
            let r = interp.call_value(tj, v.clone(), &[Value::from_str("")])?;
            return stringify_value(interp, &r, replacer, indent, gap, seen);
        }
    }
    match &v {
        Value::Undefined => Ok(None),
        Value::Null => Ok(Some("null".to_string())),
        Value::Bool(b) => Ok(Some(b.to_string())),
        Value::Number(n) => if n.is_finite() { Ok(Some(crate::value::format_number(*n))) } else { Ok(Some("null".to_string())) },
        Value::String(s) => Ok(Some(quote_string(s))),
        Value::BigInt(_) => Err(error::throw_type("Do not know how to serialize a BigInt")),
        Value::Symbol(_) => Ok(None),
        Value::Object(o) => {
            let ptr = Rc::as_ptr(o) as usize;
            if seen.contains(&ptr) { return Err(error::throw_type("Converting circular structure to JSON")); }
            seen.push(ptr);
            let result = if matches!(o.borrow().kind, ObjectKind::Array(_)) {
                stringify_array(interp, o, replacer, indent, gap, seen)
            } else {
                stringify_object(interp, o, replacer, indent, gap, seen)
            };
            seen.pop();
            result.map(Some)
        }
    }
}

fn stringify_array(interp: &mut Interpreter, o: &ObjRef, replacer: &Value, indent: &str, gap: &str, seen: &mut Vec<usize>) -> Result<String, Value> {
    let len = if let ObjectKind::Array(items) = &o.borrow().kind { items.len() } else { 0 };
    let new_gap = format!("{}{}", gap, indent);
    let mut parts = Vec::new();
    for i in 0..len {
        let elem = interp.get_property(&Value::Object(o.clone()), &PropKey::Str(crate::value::index_to_key(i)))?;
        let s = stringify_value(interp, &elem, replacer, indent, &new_gap, seen)?;
        parts.push(s.unwrap_or_else(|| "null".to_string()));
    }
    if parts.is_empty() {
        Ok("[]".to_string())
    } else if indent.is_empty() {
        Ok(format!("[{}]", parts.join(",")))
    } else {
        Ok(format!("[\n{}{}\n{}]", new_gap, parts.join(&format!(",\n{}", new_gap)), gap))
    }
}

fn stringify_object(interp: &mut Interpreter, o: &ObjRef, replacer: &Value, indent: &str, gap: &str, seen: &mut Vec<usize>) -> Result<String, Value> {
    let keys: Vec<PropKey> = {
        let b = o.borrow();
        let mut ks = Vec::new();
        if let ObjectKind::Array(items) = &b.kind { for i in 0..items.len() { ks.push(PropKey::Str(crate::value::index_to_key(i))); } }
        for (k, p) in b.props.iter() { if p.enumerable { ks.push(k.clone()); } }
        ks
    };
    let new_gap = format!("{}{}", gap, indent);
    let mut parts = Vec::new();
    for k in keys {
        let v = interp.get_property(&Value::Object(o.clone()), &k)?;
        let s = stringify_value(interp, &v, replacer, indent, &new_gap, seen)?;
        if let Some(s) = s {
            let ks = match &k { PropKey::Str(s) => quote_string(s), PropKey::Sym(_) => continue };
            if indent.is_empty() {
                parts.push(format!("{}:{}", ks, s));
            } else {
                parts.push(format!("{}: {}", ks, s));
            }
        }
    }
    if parts.is_empty() {
        Ok("{}".to_string())
    } else if indent.is_empty() {
        Ok(format!("{{{}}}", parts.join(",")))
    } else {
        Ok(format!("{{\n{}{}\n{}}}", new_gap, parts.join(&format!(",\n{}", new_gap)), gap))
    }
}

fn quote_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
