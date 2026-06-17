//! ArrayBuffer + typed array views (Uint8Array, Int8Array, ...).

use crate::realm::Realm;
use crate::error;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{make_ctor, install_global_ctor, def_method, CtorFn};
use std::cell::RefCell;
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    // ArrayBuffer
    let ab_proto = realm.array_buffer_proto.clone();
    let ab_call: NativeFn = Rc::new(move |_i, _t, args| {
        let len = to_int32(args.get(0).unwrap_or(&Value::from_int(0))) as usize;
        let buf = Rc::new(RefCell::new(vec![0u8; len]));
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(ab_proto.clone()));
        o.borrow_mut().class = "ArrayBuffer";
        o.borrow_mut().kind = ObjectKind::ArrayBuffer(buf);
        Ok(Value::Object(o))
    });
    let ab_ctor: CtorFn = { let cf = ab_call.clone(); Rc::new(move |interp, _t, args, _nt| cf(interp, Value::Undefined, args)) };
    let ab = make_ctor(realm, "ArrayBuffer", 1, ab_call, ab_ctor);
    install_global_ctor(interp, realm, "ArrayBuffer", ab, realm.array_buffer_proto.clone());
    def_method(realm, &realm.array_buffer_proto, "slice", 2, Rc::new(|interp, this, args| {
        if let Value::Object(o) = &this {
            if let ObjectKind::ArrayBuffer(buf) = &o.borrow().kind {
                let data = buf.borrow().clone();
                let len = data.len();
                let (s, e) = crate::builtins::array::normalize_slice(args.get(0), args.get(1), len);
                let new_buf = Rc::new(RefCell::new(data[s..e].to_vec()));
                let no = ObjectInner::new_object();
                no.borrow_mut().proto = Some(Value::Object(interp.realm().array_buffer_proto.clone()));
                no.borrow_mut().class = "ArrayBuffer";
                no.borrow_mut().kind = ObjectKind::ArrayBuffer(new_buf);
                return Ok(Value::Object(no));
            }
        }
        Ok(Value::Undefined)
    }));
    def_method(realm, &realm.array_buffer_proto, "byteLength", 0, Rc::new(|_i, this, _a| {
        if let Value::Object(o) = &this {
            if let ObjectKind::ArrayBuffer(buf) = &o.borrow().kind { return Ok(Value::from_int(buf.borrow().len() as i32)); }
        }
        Ok(Value::from_int(0))
    }));

    // Typed arrays
    install_typed_array(interp, realm, "Uint8Array", TypedArrayKind::Uint8, 1);
    install_typed_array(interp, realm, "Int8Array", TypedArrayKind::Int8, 1);
    install_typed_array(interp, realm, "Uint16Array", TypedArrayKind::Uint16, 2);
    install_typed_array(interp, realm, "Int16Array", TypedArrayKind::Int16, 2);
    install_typed_array(interp, realm, "Uint32Array", TypedArrayKind::Uint32, 4);
    install_typed_array(interp, realm, "Int32Array", TypedArrayKind::Int32, 4);
    install_typed_array(interp, realm, "Float32Array", TypedArrayKind::Float32, 4);
    install_typed_array(interp, realm, "Float64Array", TypedArrayKind::Float64, 8);
}

fn install_typed_array(interp: &mut Interpreter, realm: &Rc<Realm>, name: &'static str, kind: TypedArrayKind, elem_size: usize) {
    let proto = ObjectInner::new_object();
    proto.borrow_mut().proto = Some(Value::Object(realm.typed_array_proto.clone()));
    proto.borrow_mut().class = name;
    let kind_for_ctor = kind;
    let elem_size_for_ctor = elem_size;
    let call_fn: NativeFn = Rc::new({
        let proto = proto.clone();
        move |interp, _this, args| {
            build_typed(interp, &proto, kind_for_ctor, elem_size_for_ctor, args)
        }
    });
    let ctor_fn: CtorFn = Rc::new({
        let proto = proto.clone();
        move |interp, _this, args, _nt| build_typed(interp, &proto, kind_for_ctor, elem_size_for_ctor, args)
    });
    let ctor = make_ctor(realm, name, 3, call_fn, ctor_fn);
    install_global_ctor(interp, realm, name, ctor, proto.clone());
    def_method(realm, &proto, "length", 0, Rc::new(|_i, this, _a| {
        if let Value::Object(o) = &this {
            if let ObjectKind::TypedArray { length, .. } = &o.borrow().kind { return Ok(Value::from_int(*length as i32)); }
        }
        Ok(Value::from_int(0))
    }));
    def_method(realm, &proto, "byteLength", 0, Rc::new(|_i, this, _a| {
        if let Value::Object(o) = &this {
            if let ObjectKind::TypedArray { length, kind, .. } = &o.borrow().kind {
                return Ok(Value::from_int((*length * element_size(kind)) as i32));
            }
        }
        Ok(Value::from_int(0))
    }));
    let _ = interp;
}

fn build_typed(_interp: &mut Interpreter, proto: &ObjRef, kind: TypedArrayKind, elem_size: usize, args: &[Value]) -> Result<Value, Value> {
    let buf;
    let byte_offset;
    let length;
    match args.get(0) {
        Some(Value::Number(n)) => {
            length = *n as usize;
            byte_offset = 0;
            buf = Rc::new(RefCell::new(vec![0u8; length * elem_size]));
        }
        Some(Value::Object(o)) => {
            let b = o.borrow();
            if let ObjectKind::ArrayBuffer(b2) = &b.kind {
                buf = b2.clone();
                byte_offset = to_int32(args.get(1).unwrap_or(&Value::from_int(0))) as usize;
                let given = args.get(2).map(to_int32).unwrap_or(((buf.borrow().len() - byte_offset) / elem_size) as i32) as usize;
                length = given;
            } else if let ObjectKind::Array(items) = &b.kind {
                length = items.len();
                byte_offset = 0;
                let mut data = vec![0u8; length * elem_size];
                for (i, v) in items.iter().enumerate() {
                    write_elem(&mut data, i, elem_size, kind, v);
                }
                buf = Rc::new(RefCell::new(data));
            } else if let ObjectKind::TypedArray { buffer, byte_offset: bo, length: l, kind: k, .. } = &b.kind {
                let _ = (bo, l, k);
                buf = buffer.clone();
                byte_offset = *bo;
                length = *l;
            } else {
                return Err(error::throw_type("invalid typed array argument"));
            }
        }
        _ => {
            length = 0; byte_offset = 0;
            buf = Rc::new(RefCell::new(vec![]));
        }
    }
    let o = ObjectInner::new_object();
    o.borrow_mut().proto = Some(Value::Object(proto.clone()));
    o.borrow_mut().class = "TypedArray";
    o.borrow_mut().kind = ObjectKind::TypedArray { buffer: buf, byte_offset, length, kind };
    Ok(Value::Object(o))
}

fn write_elem(data: &mut [u8], i: usize, elem_size: usize, kind: TypedArrayKind, v: &Value) {
    let off = i * elem_size;
    match kind {
        TypedArrayKind::Uint8 => { data[off] = to_int32(v) as u8; }
        TypedArrayKind::Int8 => { data[off] = to_int32(v) as i8 as u8; }
        TypedArrayKind::Uint16 => { let n = to_int32(v) as u16; data[off..off+2].copy_from_slice(&n.to_le_bytes()); }
        TypedArrayKind::Int16 => { let n = to_int32(v) as i16; data[off..off+2].copy_from_slice(&n.to_le_bytes()); }
        TypedArrayKind::Uint32 => { let n = to_int32(v) as u32; data[off..off+4].copy_from_slice(&n.to_le_bytes()); }
        TypedArrayKind::Int32 => { let n = to_int32(v); data[off..off+4].copy_from_slice(&n.to_le_bytes()); }
        TypedArrayKind::Float32 => { let n = to_number(v) as f32; data[off..off+4].copy_from_slice(&n.to_le_bytes()); }
        TypedArrayKind::Float64 => { let n = to_number(v); data[off..off+8].copy_from_slice(&n.to_le_bytes()); }
    }
}

fn element_size(kind: &TypedArrayKind) -> usize {
    match kind {
        TypedArrayKind::Uint8 | TypedArrayKind::Int8 => 1,
        TypedArrayKind::Uint16 | TypedArrayKind::Int16 => 2,
        TypedArrayKind::Uint32 | TypedArrayKind::Int32 | TypedArrayKind::Float32 => 4,
        TypedArrayKind::Float64 => 8,
    }
}
