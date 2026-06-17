//! JS exception values and helpers to construct them.

use crate::value::{ObjRef, ObjectInner, ObjectKind, Value};

/// Construct a thrown value for an Error object of a given class name.
pub fn throw_error(class: &str, message: &str) -> Value {
    let obj = make_error_object(class, message);
    Value::Object(obj)
}

pub fn make_error_object(class: &str, message: &str) -> ObjRef {
    let obj = ObjectInner::new_object();
    {
        let mut b = obj.borrow_mut();
        b.class = "Error";
        b.kind = ObjectKind::Error;
        b.props.insert(
            crate::value::PropKey::from_str("name"),
            crate::value::Property::data(Value::from_str(class)),
        );
        b.props.insert(
            crate::value::PropKey::from_str("message"),
            crate::value::Property::data(Value::from_str(message)),
        );
    }
    obj
}

pub fn throw_type(msg: &str) -> Value {
    throw_error("TypeError", msg)
}
pub fn throw_range(msg: &str) -> Value {
    throw_error("RangeError", msg)
}
pub fn throw_reference(msg: &str) -> Value {
    throw_error("ReferenceError", msg)
}
pub fn throw_syntax(msg: &str) -> Value {
    throw_error("SyntaxError", msg)
}
pub fn throw_uri(msg: &str) -> Value {
    throw_error("URIError", msg)
}
pub fn throw_eval(msg: &str) -> Value {
    throw_error("EvalError", msg)
}

/// Wrap any thrown value into a user-friendly display string.
pub fn display_value(v: &Value) -> String {
    if let Value::Object(o) = v {
        let b = o.borrow();
        if b.class == "Error" || matches!(b.kind, ObjectKind::Error) {
            let name = b
                .props
                .get(&crate::value::PropKey::from_str("name"))
                .map(|p| match &p.kind {
                    crate::value::PropKind::Data(v) => crate::value::to_string(v),
                    _ => "Error".to_string(),
                })
                .unwrap_or_else(|| "Error".to_string());
            let msg = b
                .props
                .get(&crate::value::PropKey::from_str("message"))
                .map(|p| match &p.kind {
                    crate::value::PropKind::Data(v) => crate::value::to_string(v),
                    _ => String::new(),
                })
                .unwrap_or_default();
            if msg.is_empty() {
                name
            } else {
                format!("{}: {}", name, msg)
            }
        } else {
            crate::value::to_string(v)
        }
    } else {
        crate::value::to_string(v)
    }
}

/// A Rust-side parse error (not a JS exception).
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: u32,
    pub col: u32,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SyntaxError: {} ({}:{})", self.message, self.line, self.col)
    }
}

impl std::error::Error for ParseError {}

/// Internal helper used by `make_error_object` — kept public for built-ins.
pub fn set_stack(obj: &ObjRef, stack: &str) {
    obj.borrow_mut().props.insert(
        crate::value::PropKey::from_str("stack"),
        crate::value::Property::data(Value::from_str(stack)),
    );
}
