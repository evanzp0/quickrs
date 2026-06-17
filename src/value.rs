//! JavaScript value representation and object model.
//!
//! This is the heart of the engine. Values are reference-counted (`Rc`)
//! and objects are `Rc<RefCell<ObjectInner>>`. Execution is single-threaded,
//! which keeps the design simple and lets us use `Rc`/`RefCell` (and stackful
//! coroutines for generators / async-await) without `Send` requirements.

use crate::ast::{Block, Expr, FunctionDecl, Pattern};
use crate::scope::Env;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::rc::Rc;

use ignorable::Debug;

pub type ObjRef = Rc<RefCell<ObjectInner>>;

/// A JavaScript value.
#[derive(Clone, Debug)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    /// All JS numbers are f64 (matches ES Number type).
    Number(f64),
    String(Rc<str>),
    Symbol(Rc<Symbol>),
    Object(ObjRef),
    BigInt(Rc<BigInt>),
}

/// A unique symbol value.
#[derive(Clone, Debug)]
pub struct Symbol {
    pub description: Option<Rc<str>>,
    pub id: u64,
}

impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Symbol {}
impl std::hash::Hash for Symbol {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// A simple BigInt (sign-magnitude with a Vec of u32 limbs).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BigInt {
    pub negative: bool,
    pub limbs: Vec<u32>,
}

/// Property key: string or symbol. Integer indices are stored as strings
/// but the array fast-path bypasses the hashmap.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum PropKey {
    Str(Rc<str>),
    Sym(Rc<Symbol>),
}

impl PropKey {
    pub fn from_str(s: &str) -> PropKey {
        PropKey::Str(Rc::from(s))
    }
}

impl From<String> for PropKey {
    fn from(s: String) -> PropKey {
        PropKey::Str(Rc::from(s.as_str()))
    }
}

impl From<&str> for PropKey {
    fn from(s: &str) -> PropKey {
        PropKey::Str(Rc::from(s))
    }
}

/// A property descriptor (data or accessor).
#[derive(Clone, Debug)]
pub struct Property {
    pub kind: PropKind,
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
}

impl Property {
    pub fn data(value: Value) -> Property {
        Property {
            kind: PropKind::Data(value),
            writable: true,
            enumerable: true,
            configurable: true,
        }
    }
    pub fn data_named(value: Value) -> Property {
        Property::data(value)
    }
}

#[derive(Clone, Debug)]
pub enum PropKind {
    Data(Value),
    Accessor { get: Option<Value>, set: Option<Value> },
}

/// The runtime representation of a JS object.
#[derive(Clone, Debug)]
pub struct ObjectInner {
    pub props: IndexMap<PropKey, Property>,
    pub proto: Option<Value>,
    pub extensible: bool,
    pub kind: ObjectKind,
    /// Internal class name (e.g. "Object", "Array", "Function").
    pub class: &'static str,
}

/// Specialised object data.
#[derive(Clone, Debug)]
pub enum ObjectKind {
    Ordinary,
    /// Fast array storage. Holes are represented as `Value::Undefined` with a
    /// separate `holey` marker; for simplicity we don't track holes here.
    Array(Vec<Value>),
    Function(Rc<Function>),
    BoundFunction {
        target: Value,
        this_arg: Value,
        bound_args: Vec<Value>,
    },
    Error,
    String(Rc<str>),
    Number(f64),
    Boolean(bool),
    Symbol(Rc<Symbol>),
    Map(Vec<(Value, Value)>),
    Set(Vec<Value>),
    Date(f64),
    RegExp(Rc<RegExpData>),
    Promise(Rc<RefCell<PromiseState>>),
    Generator(Rc<RefCell<GeneratorState>>),
    ArrayBuffer(Rc<RefCell<Vec<u8>>>),
    TypedArray {
        buffer: Rc<RefCell<Vec<u8>>>,
        byte_offset: usize,
        length: usize,
        kind: TypedArrayKind,
    },
    Module(Rc<RefCell<ModuleState>>),
    /// ES6 Proxy: a target object + a handler with trap methods.
    Proxy(Rc<ProxyData>),
}

#[derive(Debug)]
pub struct ProxyData {
    pub target: Value,
    pub handler: Value,
    pub revoked: bool,
}

#[derive(Debug)]
pub struct RegExpData {
    pub source: Rc<str>,
    pub flags: Rc<str>,
    pub re: regex::Regex,
    pub fancy: Option<fancy_regex::Regex>,
    pub global: bool,
    pub last_index: std::cell::Cell<usize>,
}

#[derive(Copy, Clone, Debug)]
pub enum TypedArrayKind {
    Uint8,
    Int8,
    Uint16,
    Int16,
    Uint32,
    Int32,
    Float32,
    Float64,
}

#[derive(Debug)]
pub struct PromiseState {
    pub state: PromiseStatus,
    pub value: Value,
    /// Reactions to run when settled.
    pub fulfill_reactions: Vec<Reaction>,
    pub reject_reactions: Vec<Reaction>,
    /// Whether a rejection handler has ever been attached (for unhandled-report).
    pub handled: bool,
}

#[derive(Debug)]
pub enum PromiseStatus {
    Pending,
    Fulfilled,
    Rejected,
}

#[derive(Debug)]
pub struct Reaction {
    pub handler: Value,           // a function or Undefined/Null
    pub resolve: Value,           // the promise resolve fn for this reaction
    pub reject: Value,
}

#[derive(Debug)]
pub struct GeneratorState {
    pub done: bool,
    #[ignored(Debug)]
    pub coro: Option<CoroutineHandle>,
}

/// A handle to a suspended stackful coroutine (generators / async functions).
pub type CoroutineHandle =
    corosensei::Coroutine<Result<Value, Value>, GeneratorYield, GeneratorResult>;

/// What a coroutine yields out.
#[derive(Debug)]
pub enum GeneratorYield {
    /// `yield v` in a generator.
    Yield(Value),
    /// `await p` in an async function. The driver registers a reaction on `p`
    /// and resumes the coroutine with `Ok(resolved)` or `Err(rejected)`.
    Await(Value),
}

/// What a coroutine returns.
#[derive(Debug)]
pub enum GeneratorResult {
    /// Generator finished (`return v` or fall-off).
    Done(Value),
    /// Async function finished normally -> resolve its promise with `v`.
    AsyncReturn(Value),
    /// Coroutine threw a JS exception.
    Throw(Value),
}

/// A callable.
#[derive(Debug)]
pub struct Function {
    pub body: FunctionBody,
    pub name: Rc<str>,
    /// Declared formal parameter length.
    pub length: usize,
    /// Closure environment.
    pub closure: Env,
    pub is_arrow: bool,
    pub is_generator: bool,
    pub is_async: bool,
    pub is_method: bool,
    pub is_constructor: bool,
    /// For `super` lookups in methods.
    pub home_object: Option<Value>,
    /// Fields / methods for class instances created by this constructor.
    pub class_fields: Vec<ClassField>,
    pub parent_class: Option<Value>,
    /// Source line where the function was defined (for stack traces).
    pub line: u32,
}

/// A class field declaration.
#[derive(Clone, Debug)]
pub struct ClassField {
    pub name: Pattern,
    pub init: Option<Expr>,
}

#[derive(Debug)]
pub enum FunctionBody {
    /// A Rust native function.
    Native {
        #[ignored(Debug)]
        func: Rc<dyn Fn(&mut crate::interp::Interpreter, Value, &[Value]) -> Result<Value, Value>>,
        /// `Native` constructor support.
        #[ignored(Debug)]
        constructor: Option<
            Rc<
                dyn Fn(
                        &mut crate::interp::Interpreter,
                        Value,
                        &[Value],
                        Value,
                    ) -> Result<Value, Value>,
            >,
        >,
    },
    /// A JavaScript function: parameters, body, declared functions.
    Js {
        params: Vec<Pattern>,
        body: Block,
        decls: Vec<FunctionDecl>,
        /// Whether `arguments` / strict-mode specifics apply.
        strict: bool,
    },
}

impl ObjectInner {
    pub fn new_object() -> ObjRef {
        Rc::new(RefCell::new(ObjectInner {
            props: IndexMap::new(),
            proto: None,
            extensible: true,
            kind: ObjectKind::Ordinary,
            class: "Object",
        }))
    }

    pub fn new_array(items: Vec<Value>) -> ObjRef {
        Rc::new(RefCell::new(ObjectInner {
            props: IndexMap::new(),
            proto: None,
            extensible: true,
            kind: ObjectKind::Array(items),
            class: "Array",
        }))
    }

    pub fn new_function(f: Rc<Function>) -> ObjRef {
        Rc::new(RefCell::new(ObjectInner {
            props: IndexMap::new(),
            proto: None,
            extensible: true,
            kind: ObjectKind::Function(f),
            class: "Function",
        }))
    }
}

// ---------------------------------------------------------------------------
// Value helpers
// ---------------------------------------------------------------------------

impl Value {
    pub fn undefined() -> Value {
        Value::Undefined
    }
    pub fn null() -> Value {
        Value::Null
    }
    pub fn from_bool(b: bool) -> Value {
        Value::Bool(b)
    }
    pub fn from_f64(n: f64) -> Value {
        Value::Number(n)
    }
    pub fn from_int(n: i32) -> Value {
        Value::Number(n as f64)
    }
    pub fn from_str(s: &str) -> Value {
        Value::String(Rc::from(s))
    }
    pub fn from_string<S: AsRef<str>>(s: S) -> Value {
        Value::String(Rc::from(s.as_ref()))
    }
    pub fn object(o: ObjRef) -> Value {
        Value::Object(o)
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, Value::Undefined)
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn is_nullish(&self) -> bool {
        matches!(self, Value::Null | Value::Undefined)
    }
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }
    pub fn is_callable(&self) -> bool {
        if let Value::Object(o) = self {
            match &o.borrow().kind {
                ObjectKind::Function(_) | ObjectKind::BoundFunction { .. } => true,
                ObjectKind::Proxy(pd) => pd.target.is_callable(),
                _ => false,
            }
        } else {
            false
        }
    }
    pub fn is_constructor(&self) -> bool {
        if let Value::Object(o) = self {
            match &o.borrow().kind {
                ObjectKind::Function(f) => f.is_constructor,
                ObjectKind::BoundFunction { .. } => true,
                ObjectKind::Proxy(pd) => pd.target.is_constructor(),
                _ => false,
            }
        } else {
            false
        }
    }
    pub fn as_object(&self) -> Option<ObjRef> {
        if let Value::Object(o) = self {
            Some(o.clone())
        } else {
            None
        }
    }

    pub fn type_of(&self) -> &'static str {
        match self {
            Value::Undefined => "undefined",
            Value::Null => "object",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Symbol(_) => "symbol",
            Value::BigInt(_) => "bigint",
            Value::Object(o) => {
                if o.borrow().kind.is_function() {
                    "function"
                } else {
                    "object"
                }
            }
        }
    }
}

impl ObjectKind {
    pub fn is_function(&self) -> bool {
        matches!(self, ObjectKind::Function(_) | ObjectKind::BoundFunction { .. })
    }
}

/// Convert a value to a property key.
pub fn to_property_key(v: &Value) -> PropKey {
    match v {
        Value::String(s) => PropKey::Str(s.clone()),
        Value::Symbol(s) => PropKey::Sym(s.clone()),
        Value::Number(n) => PropKey::Str(Rc::from(format_number(*n).as_str())),
        _ => PropKey::Str(Rc::from(to_string(v).as_str())),
    }
}

/// Canonical index string ("0", "1", ...).
pub fn index_to_key(i: usize) -> Rc<str> {
    Rc::from(i.to_string().as_str())
}

/// Try to interpret a property key as a canonical array index.
pub fn key_to_index(key: &str) -> Option<usize> {
    if key.is_empty() {
        return None;
    }
    if key == "0" {
        return Some(0);
    }
    if key.starts_with('0') {
        return None;
    }
    key.parse::<usize>().ok().filter(|&n| n < u32::MAX as usize)
}

/// ES `ToString` for numbers (subset; matches V8-ish output for common cases).
pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n == f64::INFINITY {
        return "Infinity".to_string();
    }
    if n == f64::NEG_INFINITY {
        return "-Infinity".to_string();
    }
    if n == 0.0 {
        return "0".to_string();
    }
    if n.fract() == 0.0 && n.abs() < 1e21 {
        return format!("{}", n as i64);
    }
    // Use Rust's default float formatting which is close to ES shortest round-trip.
    let s = format!("{}", n);
    // Rust prints e.g. 0.1 as "0.1", 1e21 as "1000000000000000000000"; ES wants "1e+21".
    if n.abs() >= 1e21 || (n.abs() != 0.0 && n.abs() < 1e-6) {
        return format_exponential(n);
    }
    s
}

fn format_exponential(n: f64) -> String {
    if n == 0.0 {
        return "0".to_string();
    }
    let abs = n.abs();
    let exp = abs.log10().floor() as i32;
    let mantissa = n / 10f64.powi(exp);
    let m = format_number(mantissa);
    let sign = if exp >= 0 { "+" } else { "-" };
    format!("{}e{}{}", m, sign, exp.abs())
}

/// ES `ToString` (used by property access, string concatenation, etc.).
pub fn to_string(v: &Value) -> String {
    match v {
        Value::Undefined => "undefined".to_string(),
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format_number(*n),
        Value::String(s) => s.to_string(),
        Value::Symbol(_) => "Symbol()".to_string(),
        Value::BigInt(b) => bigint_to_string(b),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

pub fn bigint_to_string(b: &BigInt) -> String {
    if b.limbs.is_empty() {
        return "0".to_string();
    }
    // Convert via base-10 long division.
    let mut limbs = b.limbs.clone();
    let mut digits = String::new();
    while !limbs.is_empty() && !limbs.iter().all(|&x| x == 0) {
        let mut rem: u64 = 0;
        for limb in limbs.iter_mut() {
            let cur = (rem << 32) | *limb as u64;
            *limb = (cur / 10) as u32;
            rem = cur % 10;
        }
        digits.push((b'0' + rem as u8) as char);
        if limbs.last() == Some(&0) {
            limbs.pop();
        }
    }
    if digits.is_empty() {
        digits.push('0');
    }
    let mut s: String = digits.chars().rev().collect();
    if b.negative && s != "0" {
        s.insert(0, '-');
    }
    s
}

/// ES `ToBoolean`.
pub fn to_boolean(v: &Value) -> bool {
    match v {
        Value::Undefined | Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0 && !n.is_nan(),
        Value::String(s) => !s.is_empty(),
        Value::Symbol(_) | Value::Object(_) | Value::BigInt(_) => true,
    }
}

/// ES `ToNumber`.
pub fn to_number(v: &Value) -> f64 {
    match v {
        Value::Undefined => f64::NAN,
        Value::Null => 0.0,
        Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Value::Number(n) => *n,
        Value::String(s) => string_to_number(s),
        Value::Symbol(_) | Value::Object(_) | Value::BigInt(_) => f64::NAN,
    }
}

/// Parse a JS numeric string (handles hex/bin/octal, infinity, decimals).
pub fn string_to_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    if t.eq_ignore_ascii_case("Infinity") || t == "+Infinity" {
        return f64::INFINITY;
    }
    if t == "-Infinity" {
        return f64::NEG_INFINITY;
    }
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).map(|v| v as f64).unwrap_or(f64::NAN);
    }
    if let Some(oct) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        return u64::from_str_radix(oct, 8).map(|v| v as f64).unwrap_or(f64::NAN);
    }
    if let Some(bin) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        return u64::from_str_radix(bin, 2).map(|v| v as f64).unwrap_or(f64::NAN);
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// ES `ToIntegerOrInfinity`.
pub fn to_integer(v: &Value) -> f64 {
    let n = to_number(v);
    if n.is_nan() {
        0.0
    } else if n == f64::INFINITY {
        f64::INFINITY
    } else if n == f64::NEG_INFINITY {
        f64::NEG_INFINITY
    } else {
        n.trunc()
    }
}

/// ES `ToInt32`.
pub fn to_int32(v: &Value) -> i32 {
    let n = to_number(v);
    if !n.is_finite() || n == 0.0 {
        return 0;
    }
    let n = n.trunc();
    let m = n.rem_euclid(4294967296.0); // 2^32
    let m = if m >= 2147483648.0 { m - 4294967296.0 } else { m };
    m as i32
}

/// ES `ToUint32`.
pub fn to_uint32(v: &Value) -> u32 {
    let n = to_number(v);
    if !n.is_finite() || n == 0.0 {
        return 0;
    }
    let n = n.trunc();
    (n.rem_euclid(4294967296.0)) as u32
}

/// ES `ToLength`.
pub fn to_length(v: &Value) -> usize {
    let n = to_integer(v);
    if n <= 0.0 {
        0
    } else if n > 9007199254740991.0 {
        9007199254740991
    } else {
        n as usize
    }
}

/// Loose equality (==).
pub fn loose_equals(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (Undefined, Undefined) | (Null, Null) => true,
        (Undefined, Null) | (Null, Undefined) => true,
        (Number(x), Number(y)) => x == y,
        (String(x), String(y)) => x == y,
        (Bool(x), Bool(y)) => x == y,
        (Symbol(x), Symbol(y)) => Rc::ptr_eq(x, y),
        (Object(x), Object(y)) => Rc::ptr_eq(x, y),
        (BigInt(x), BigInt(y)) => x == y,
        // Number / String
        (Number(_), String(_)) => loose_equals(a, &Number(to_number(b))),
        (String(_), Number(_)) => loose_equals(&Number(to_number(a)), b),
        // Bool -> Number
        (Bool(_), _) => loose_equals(&Number(to_number(a)), b),
        (_, Bool(_)) => loose_equals(a, &Number(to_number(b))),
        // BigInt <-> Number
        (BigInt(b), Number(n)) | (Number(n), BigInt(b)) => {
            n.is_finite() && n.fract() == 0.0 && bigint_f64_eq(b, *n)
        }
        (BigInt(b), String(s)) | (String(s), BigInt(b)) => {
            if let Ok(n) = s.trim().parse::<f64>() {
                n.is_finite() && n.fract() == 0.0 && bigint_f64_eq(b, n)
            } else {
                false
            }
        }
        // Object <-> primitive: compare object's primitive value
        (Object(_), _) => {
            let p = primitive_hint(a);
            loose_equals(&p, b)
        }
        (_, Object(_)) => {
            let p = primitive_hint(b);
            loose_equals(a, &p)
        }
        _ => false,
    }
}

fn bigint_f64_eq(b: &BigInt, n: f64) -> bool {
    // Compare via string round-trip (simple, correct for integers).
    bigint_to_string(b) == format!("{}", n as i128)
}

fn primitive_hint(v: &Value) -> Value {
    // Only called for objects in loose equals; use DefaultToPrimitive (number).
    // We approximate by returning the object's valueOf result if it's a wrapper.
    if let Value::Object(o) = v {
        let b = o.borrow();
        match &b.kind {
            ObjectKind::Number(n) => Value::Number(*n),
            ObjectKind::String(s) => Value::String(s.clone()),
            ObjectKind::Boolean(b) => Value::Bool(*b),
            ObjectKind::Symbol(s) => Value::Symbol(s.clone()),
            ObjectKind::Date(t) => Value::Number(*t),
            _ => Value::String(Rc::from("[object Object]")),
        }
    } else {
        v.clone()
    }
}

/// Strict equality (===).
pub fn strict_equals(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (Undefined, Undefined) => true,
        (Null, Null) => true,
        (Number(x), Number(y)) => x == y,
        (String(x), String(y)) => x == y,
        (Bool(x), Bool(y)) => x == y,
        (Symbol(x), Symbol(y)) => Rc::ptr_eq(x, y),
        (Object(x), Object(y)) => Rc::ptr_eq(x, y),
        (BigInt(x), BigInt(y)) => x == y,
        _ => false,
    }
}

/// SameValue (Object.is) — like strict equals but distinguishes +0/-0 and NaN.
pub fn same_value(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (Number(x), Number(y)) => {
            if x.is_nan() && y.is_nan() {
                true
            } else if *x == 0.0 && *y == 0.0 {
                x.is_sign_positive() == y.is_sign_positive()
            } else {
                x == y
            }
        }
        _ => strict_equals(a, b),
    }
}

/// SameValueZero (used by Array.includes, Map/Set) — like SameValue but +0 == -0.
pub fn same_value_zero(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (Number(x), Number(y)) => {
            if x.is_nan() && y.is_nan() {
                true
            } else {
                x == y
            }
        }
        _ => strict_equals(a, b),
    }
}

pub fn module_state() -> Rc<RefCell<ModuleState>> {
    Rc::new(RefCell::new(ModuleState {
        evaluated: false,
        exports: IndexMap::new(),
    }))
}

#[derive(Debug)]
pub struct ModuleState {
    pub evaluated: bool,
    pub exports: IndexMap<PropKey, Value>,
}

/// Format a Date timestamp (ms since epoch) as an ISO-8601 string.
pub fn date_format(ms: f64) -> String {
    if ms.is_nan() {
        return "Invalid Date".to_string();
    }
    let secs = (ms / 1000.0) as i64;
    let nanos = ((ms - secs as f64 * 1000.0).abs() as u64) * 1_000_000;
    match chrono::DateTime::from_timestamp(secs, nanos as u32) {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        None => "Invalid Date".to_string(),
    }
}
