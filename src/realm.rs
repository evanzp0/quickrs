//! Realm: the global environment and all intrinsic prototypes/constructors.

use crate::value::*;
use std::cell::{Cell, RefCell};
use indexmap::IndexMap;
use std::rc::Rc;

/// Well-known symbols (Symbol.iterator, etc.).
pub struct WellKnownSymbols {
    pub iterator: Rc<Symbol>,
    pub async_iterator: Rc<Symbol>,
    pub has_instance: Rc<Symbol>,
    pub to_primitive: Rc<Symbol>,
    pub to_string_tag: Rc<Symbol>,
    pub is_concat_spreadable: Rc<Symbol>,
}

pub struct Realm {
    pub global: ObjRef,
    pub global_env: crate::scope::Env,

    // Prototypes
    pub object_proto: ObjRef,
    pub function_proto: ObjRef,
    pub array_proto: ObjRef,
    pub string_proto: ObjRef,
    pub number_proto: ObjRef,
    pub boolean_proto: ObjRef,
    pub symbol_proto: ObjRef,
    pub bigint_proto: ObjRef,
    pub error_proto: ObjRef,
    pub type_error_proto: ObjRef,
    pub range_error_proto: ObjRef,
    pub syntax_error_proto: ObjRef,
    pub reference_error_proto: ObjRef,
    pub uri_error_proto: ObjRef,
    pub eval_error_proto: ObjRef,
    pub promise_proto: ObjRef,
    pub map_proto: ObjRef,
    pub set_proto: ObjRef,
    pub date_proto: ObjRef,
    pub regexp_proto: ObjRef,
    pub array_buffer_proto: ObjRef,
    pub typed_array_proto: ObjRef,
    pub generator_proto: ObjRef,
    pub iterator_proto: ObjRef,
    pub array_iterator_proto: ObjRef,
    pub map_iterator_proto: ObjRef,
    pub set_iterator_proto: ObjRef,
    pub string_iterator_proto: ObjRef,

    // Constructors (stored as Values)
    pub object_ctor: Value,
    pub array_ctor: Value,
    pub string_ctor: Value,
    pub number_ctor: Value,
    pub boolean_ctor: Value,
    pub symbol_ctor: Value,
    pub bigint_ctor: Value,
    pub error_ctor: Value,
    pub type_error_ctor: Value,
    pub range_error_ctor: Value,
    pub syntax_error_ctor: Value,
    pub reference_error_ctor: Value,
    pub uri_error_ctor: Value,
    pub eval_error_ctor: Value,
    pub promise_ctor: Value,
    pub map_ctor: Value,
    pub set_ctor: Value,
    pub date_ctor: Value,
    pub regexp_ctor: Value,
    pub array_buffer_ctor: Value,
    pub uint8_array_ctor: Value,
    pub int8_array_ctor: Value,
    pub uint16_array_ctor: Value,
    pub int16_array_ctor: Value,
    pub uint32_array_ctor: Value,
    pub int32_array_ctor: Value,
    pub float32_array_ctor: Value,
    pub float64_array_ctor: Value,

    pub wk: WellKnownSymbols,
    pub symbol_counter: Cell<u64>,
    pub modules: RefCell<IndexMap<String, ModuleEntry>>,
    pub module_cache: RefCell<IndexMap<String, Value>>,
}

pub struct ModuleEntry {
    pub source: Rc<str>,
}

impl Realm {
    pub fn new() -> Rc<Realm> {
        let object_proto = ObjectInner::new_object();
        let function_proto = ObjectInner::new_function(Rc::new(Function {
            body: FunctionBody::Native {
                func: Rc::new(|_, _, _| Ok(Value::Undefined)),
                constructor: None,
            },
            name: Rc::from("Function"),
            length: 0,
            closure: crate::scope::Env::global(),
            is_arrow: false,
            is_generator: false,
            is_async: false,
            is_method: false,
            is_constructor: false,
            home_object: None,
            class_fields: Vec::new(),
            parent_class: None,
            line: 0,
        }));
        // function_proto's proto is object_proto
        function_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));

        let array_proto = ObjectInner::new_array(Vec::new());
        array_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        let string_proto = ObjectInner::new_object();
        string_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        string_proto.borrow_mut().class = "String";
        let number_proto = ObjectInner::new_object();
        number_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        number_proto.borrow_mut().class = "Number";
        let boolean_proto = ObjectInner::new_object();
        boolean_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        boolean_proto.borrow_mut().class = "Boolean";
        let symbol_proto = ObjectInner::new_object();
        symbol_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        symbol_proto.borrow_mut().class = "Symbol";
        let bigint_proto = ObjectInner::new_object();
        bigint_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        bigint_proto.borrow_mut().class = "BigInt";
        let error_proto = ObjectInner::new_object();
        error_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        error_proto.borrow_mut().class = "Error";
        error_proto.borrow_mut().props.insert(
            PropKey::from_str("name"),
            Property::data(Value::from_str("Error")),
        );
        error_proto.borrow_mut().props.insert(
            PropKey::from_str("message"),
            Property::data(Value::from_str("")),
        );
        let mk_err_proto = |name: &str| {
            let p = ObjectInner::new_object();
            p.borrow_mut().proto = Some(Value::Object(error_proto.clone()));
            p.borrow_mut().class = "Error";
            p.borrow_mut().props.insert(
                PropKey::from_str("name"),
                Property::data(Value::from_str(name)),
            );
            p.borrow_mut().props.insert(
                PropKey::from_str("message"),
                Property::data(Value::from_str("")),
            );
            p
        };
        let type_error_proto = mk_err_proto("TypeError");
        let range_error_proto = mk_err_proto("RangeError");
        let syntax_error_proto = mk_err_proto("SyntaxError");
        let reference_error_proto = mk_err_proto("ReferenceError");
        let uri_error_proto = mk_err_proto("URIError");
        let eval_error_proto = mk_err_proto("EvalError");

        let promise_proto = ObjectInner::new_object();
        promise_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        promise_proto.borrow_mut().class = "Promise";
        let map_proto = ObjectInner::new_object();
        map_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        map_proto.borrow_mut().class = "Map";
        let set_proto = ObjectInner::new_object();
        set_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        set_proto.borrow_mut().class = "Set";
        let date_proto = ObjectInner::new_object();
        date_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        date_proto.borrow_mut().class = "Date";
        let regexp_proto = ObjectInner::new_object();
        regexp_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        regexp_proto.borrow_mut().class = "RegExp";
        let array_buffer_proto = ObjectInner::new_object();
        array_buffer_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        array_buffer_proto.borrow_mut().class = "ArrayBuffer";
        let typed_array_proto = ObjectInner::new_object();
        typed_array_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        typed_array_proto.borrow_mut().class = "TypedArray";
        let iterator_proto = ObjectInner::new_object();
        iterator_proto.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        let generator_proto = ObjectInner::new_object();
        generator_proto.borrow_mut().proto = Some(Value::Object(iterator_proto.clone()));
        generator_proto.borrow_mut().class = "Generator";
        let array_iterator_proto = ObjectInner::new_object();
        array_iterator_proto.borrow_mut().proto = Some(Value::Object(iterator_proto.clone()));
        let map_iterator_proto = ObjectInner::new_object();
        map_iterator_proto.borrow_mut().proto = Some(Value::Object(iterator_proto.clone()));
        let set_iterator_proto = ObjectInner::new_object();
        set_iterator_proto.borrow_mut().proto = Some(Value::Object(iterator_proto.clone()));
        let string_iterator_proto = ObjectInner::new_object();
        string_iterator_proto.borrow_mut().proto = Some(Value::Object(iterator_proto.clone()));

        let global = ObjectInner::new_object();
        global.borrow_mut().proto = Some(Value::Object(object_proto.clone()));
        global.borrow_mut().class = "Object";
        let global_env = crate::scope::Env::global();

        let wk = WellKnownSymbols {
            iterator: Rc::new(Symbol { description: Some(Rc::from("Symbol.iterator")), id: 1 }),
            async_iterator: Rc::new(Symbol { description: Some(Rc::from("Symbol.asyncIterator")), id: 2 }),
            has_instance: Rc::new(Symbol { description: Some(Rc::from("Symbol.hasInstance")), id: 3 }),
            to_primitive: Rc::new(Symbol { description: Some(Rc::from("Symbol.toPrimitive")), id: 4 }),
            to_string_tag: Rc::new(Symbol { description: Some(Rc::from("Symbol.toStringTag")), id: 5 }),
            is_concat_spreadable: Rc::new(Symbol { description: Some(Rc::from("Symbol.isConcatSpreadable")), id: 6 }),
        };

        let realm = Rc::new(Realm {
            global,
            global_env,
            object_proto,
            function_proto,
            array_proto,
            string_proto,
            number_proto,
            boolean_proto,
            symbol_proto,
            bigint_proto,
            error_proto,
            type_error_proto,
            range_error_proto,
            syntax_error_proto,
            reference_error_proto,
            uri_error_proto,
            eval_error_proto,
            promise_proto,
            map_proto,
            set_proto,
            date_proto,
            regexp_proto,
            array_buffer_proto,
            typed_array_proto,
            generator_proto,
            iterator_proto,
            array_iterator_proto,
            map_iterator_proto,
            set_iterator_proto,
            string_iterator_proto,
            // placeholders for ctors; filled by builtins
            object_ctor: Value::Undefined,
            array_ctor: Value::Undefined,
            string_ctor: Value::Undefined,
            number_ctor: Value::Undefined,
            boolean_ctor: Value::Undefined,
            symbol_ctor: Value::Undefined,
            bigint_ctor: Value::Undefined,
            error_ctor: Value::Undefined,
            type_error_ctor: Value::Undefined,
            range_error_ctor: Value::Undefined,
            syntax_error_ctor: Value::Undefined,
            reference_error_ctor: Value::Undefined,
            uri_error_ctor: Value::Undefined,
            eval_error_ctor: Value::Undefined,
            promise_ctor: Value::Undefined,
            map_ctor: Value::Undefined,
            set_ctor: Value::Undefined,
            date_ctor: Value::Undefined,
            regexp_ctor: Value::Undefined,
            array_buffer_ctor: Value::Undefined,
            uint8_array_ctor: Value::Undefined,
            int8_array_ctor: Value::Undefined,
            uint16_array_ctor: Value::Undefined,
            int16_array_ctor: Value::Undefined,
            uint32_array_ctor: Value::Undefined,
            int32_array_ctor: Value::Undefined,
            float32_array_ctor: Value::Undefined,
            float64_array_ctor: Value::Undefined,
            wk,
            symbol_counter: Cell::new(100),
            modules: RefCell::new(IndexMap::new()),
            module_cache: RefCell::new(IndexMap::new()),
        });
        realm
    }

    pub fn new_symbol(&self, description: Option<Rc<str>>) -> Rc<Symbol> {
        let id = self.symbol_counter.get();
        self.symbol_counter.set(id + 1);
        Rc::new(Symbol { description, id })
    }

    pub fn proto_for(&self, class: &str) -> Option<ObjRef> {
        match class {
            "Object" => Some(self.object_proto.clone()),
            "Function" => Some(self.function_proto.clone()),
            "Array" => Some(self.array_proto.clone()),
            "String" => Some(self.string_proto.clone()),
            "Number" => Some(self.number_proto.clone()),
            "Boolean" => Some(self.boolean_proto.clone()),
            "Symbol" => Some(self.symbol_proto.clone()),
            "BigInt" => Some(self.bigint_proto.clone()),
            "Error" => Some(self.error_proto.clone()),
            "TypeError" => Some(self.type_error_proto.clone()),
            "RangeError" => Some(self.range_error_proto.clone()),
            "SyntaxError" => Some(self.syntax_error_proto.clone()),
            "ReferenceError" => Some(self.reference_error_proto.clone()),
            "URIError" => Some(self.uri_error_proto.clone()),
            "EvalError" => Some(self.eval_error_proto.clone()),
            "Promise" => Some(self.promise_proto.clone()),
            "Map" => Some(self.map_proto.clone()),
            "Set" => Some(self.set_proto.clone()),
            "Date" => Some(self.date_proto.clone()),
            "RegExp" => Some(self.regexp_proto.clone()),
            "ArrayBuffer" => Some(self.array_buffer_proto.clone()),
            "Generator" => Some(self.generator_proto.clone()),
            _ => None,
        }
    }
}
