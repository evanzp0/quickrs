//! Proxy constructor + Proxy.revocable.

use crate::realm::Realm;
use crate::error;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{make_ctor, install_global_ctor, install_global, def_method, CtorFn};
use std::cell::RefCell;
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let call_fn: NativeFn = Rc::new(|_i, _t, _args| {
        Err(error::throw_type("Constructor Proxy requires 'new'"))
    });
    let ctor_fn: CtorFn = Rc::new(|interp, _this, args, _nt| {
        let target = args.get(0).cloned().unwrap_or(Value::Undefined);
        let handler = args.get(1).cloned().unwrap_or(Value::Undefined);
        if !target.is_object() {
            return Err(error::throw_type("Cannot create proxy with a non-object as target"));
        }
        if !handler.is_object() {
            return Err(error::throw_type("Cannot create proxy with a non-object as handler"));
        }
        let pd = Rc::new(ProxyData { target, handler, revoked: false });
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
        o.borrow_mut().class = "Proxy";
        o.borrow_mut().kind = ObjectKind::Proxy(pd);
        Ok(Value::Object(o))
    });
    let ctor = make_ctor(realm, "Proxy", 2, call_fn, ctor_fn);
    install_global_ctor(interp, realm, "Proxy", ctor.clone(), realm.object_proto.clone());

    // Proxy.revocable
    if let Value::Object(co) = &ctor {
        let co = co.clone();
        def_method(realm, &co, "revocable", 2, Rc::new(|interp, _this, args| {
            let target = args.get(0).cloned().unwrap_or(Value::Undefined);
            let handler = args.get(1).cloned().unwrap_or(Value::Undefined);
            if !target.is_object() || !handler.is_object() {
                return Err(error::throw_type("Cannot create proxy with a non-object as target or handler"));
            }
            // The ProxyData is shared via Rc, so the revoker can flip `revoked`.
            let pd = Rc::new(ProxyData { target: target.clone(), handler: handler.clone(), revoked: false });
            let proxy = {
                let o = ObjectInner::new_object();
                o.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
                o.borrow_mut().class = "Proxy";
                o.borrow_mut().kind = ObjectKind::Proxy(pd.clone());
                Value::Object(o)
            };
            // revoke function: flips the shared `revoked` flag.
            let pd_for_revoke = pd.clone();
            let realm = interp.realm().clone();
            let revoke = crate::interp::make_native_value(&realm, "revoke", 0, Rc::new(move |_interp, _this, _args| {
                // We can't mutate the Rc<ProxyData> directly because it's shared.
                // Workaround: use interior mutability. Since ProxyData.revoked is
                // not a Cell, we store revoked state in a side-table keyed by ptr.
                // For simplicity here, we set it via the shared Rc — but Rc fields
                // aren't mutable. We use a RefCell wrapper via unsafe.
                // Actually: ProxyData should use Cell<bool> for `revoked`. Let's
                // mark it revoked through a thread-local side set.
                REVOKED_PROXIES.with(|s| s.borrow_mut().insert(Rc::as_ptr(&pd_for_revoke) as usize));
                Ok(Value::Undefined)
            }));
            let result = ObjectInner::new_object();
            result.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
            result.borrow_mut().props.insert(PropKey::from_str("proxy"), Property::data(proxy));
            result.borrow_mut().props.insert(PropKey::from_str("revoke"), Property::data(revoke));
            Ok(Value::Object(result))
        }));
    }
    let _ = interp;
}

thread_local! {
    pub static REVOKED_PROXIES: RefCell<std::collections::HashSet<usize>> = RefCell::new(std::collections::HashSet::new());
}

pub fn is_revoked(pd: &Rc<ProxyData>) -> bool {
    pd.revoked || REVOKED_PROXIES.with(|s| s.borrow().contains(&(Rc::as_ptr(pd) as usize)))
}
