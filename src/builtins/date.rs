//! Date constructor + Date.prototype (subset).

use crate::realm::Realm;
use crate::interp::{Interpreter, NativeFn};
use crate::value::*;
use crate::builtins::{make_ctor, install_global_ctor, def_method, CtorFn};
use std::rc::Rc;

pub fn install(interp: &mut Interpreter, realm: &Rc<Realm>) {
    let call_fn: NativeFn = Rc::new(|_i, _t, args| {
        if args.is_empty() {
            return Ok(Value::Number(now_ms()));
        }
        // Date(year, month, ...)
        date_from_args(args).map(Value::Number)
    });
    let ctor_fn: CtorFn = Rc::new(|interp, _t, args, _nt| {
        let ms = if args.is_empty() { now_ms() } else { date_from_args(args)? };
        let o = ObjectInner::new_object();
        o.borrow_mut().proto = Some(Value::Object(interp.realm().date_proto.clone()));
        o.borrow_mut().class = "Date";
        o.borrow_mut().kind = ObjectKind::Date(ms);
        Ok(Value::Object(o))
    });
    let ctor = make_ctor(realm, "Date", 7, call_fn, ctor_fn);
    install_global_ctor(interp, realm, "Date", ctor.clone(), realm.date_proto.clone());
    if let Value::Object(co) = &ctor {
        let co = co.clone();
        def_method(realm, &co, "now", 0, Rc::new(|_i, _t, _a| Ok(Value::Number(now_ms()))));
        def_method(realm, &co, "UTC", 7, Rc::new(|_i, _t, args| date_from_args(args).map(Value::Number)));
        def_method(realm, &co, "parse", 1, Rc::new(|_i, _t, args| {
            let s = crate::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
            Ok(Value::Number(parse_date_string(&s)))
        }));
    }
    let dp = realm.date_proto.clone();
    def_method(realm, &dp, "getTime", 0, Rc::new(|_i, this, _a| Ok(Value::Number(date_ms(&this)))));
    def_method(realm, &dp, "valueOf", 0, Rc::new(|_i, this, _a| Ok(Value::Number(date_ms(&this)))));
    def_method(realm, &dp, "toISOString", 0, Rc::new(|_i, this, _a| Ok(Value::from_string(crate::value::date_format(date_ms(&this))))));
    def_method(realm, &dp, "toString", 0, Rc::new(|_i, this, _a| Ok(Value::from_string(crate::value::date_format(date_ms(&this))))));
    def_method(realm, &dp, "toJSON", 0, Rc::new(|_i, this, _a| Ok(Value::from_string(crate::value::date_format(date_ms(&this))))));
    def_method(realm, &dp, "toDateString", 0, Rc::new(|_i, this, _a| {
        let s = crate::value::date_format(date_ms(&this));
        Ok(Value::from_string(s.get(..10).unwrap_or(&s).to_string()))
    }));
    def_method(realm, &dp, "toTimeString", 0, Rc::new(|_i, this, _a| {
        let s = crate::value::date_format(date_ms(&this));
        Ok(Value::from_string(s.get(11..19).unwrap_or("").to_string()))
    }));
    def_method(realm, &dp, "getFullYear", 0, Rc::new(|_i, this, _a| Ok(Value::from_int(year_of(date_ms(&this)) as i32))));
    def_method(realm, &dp, "getUTCFullYear", 0, Rc::new(|_i, this, _a| Ok(Value::from_int(year_of(date_ms(&this)) as i32))));
    def_method(realm, &dp, "getMonth", 0, Rc::new(|_i, this, _a| Ok(Value::from_int(month_of(date_ms(&this)) as i32))));
    def_method(realm, &dp, "getDate", 0, Rc::new(|_i, this, _a| Ok(Value::from_int(day_of(date_ms(&this)) as i32))));
    def_method(realm, &dp, "getHours", 0, Rc::new(|_i, this, _a| Ok(Value::from_int(hours_of(date_ms(&this)) as i32))));
    def_method(realm, &dp, "getMinutes", 0, Rc::new(|_i, this, _a| Ok(Value::from_int(minutes_of(date_ms(&this)) as i32))));
    def_method(realm, &dp, "getSeconds", 0, Rc::new(|_i, this, _a| Ok(Value::from_int(seconds_of(date_ms(&this)) as i32))));
    def_method(realm, &dp, "getMilliseconds", 0, Rc::new(|_i, this, _a| Ok(Value::from_int(ms_of(date_ms(&this)) as i32))));
    def_method(realm, &dp, "getDay", 0, Rc::new(|_i, this, _a| Ok(Value::from_int(day_of_week(date_ms(&this)) as i32))));
    def_method(realm, &dp, "getTimezoneOffset", 0, Rc::new(|_i, _t, _a| Ok(Value::from_int(0))));
    let _ = interp;
}

fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

fn date_ms(v: &Value) -> f64 {
    match v {
        Value::Object(o) => { if let ObjectKind::Date(t) = o.borrow().kind { t } else { f64::NAN } }
        Value::Number(n) => *n,
        _ => f64::NAN,
    }
}

fn date_from_args(args: &[Value]) -> Result<f64, Value> {
    if args.is_empty() { return Ok(now_ms()); }
    let year = crate::value::to_number(&args[0]);
    let month = if args.len() > 1 { crate::value::to_number(&args[1]) } else { 0.0 };
    let day = if args.len() > 2 { crate::value::to_number(&args[2]) } else { 1.0 };
    let hour = if args.len() > 3 { crate::value::to_number(&args[3]) } else { 0.0 };
    let min = if args.len() > 4 { crate::value::to_number(&args[4]) } else { 0.0 };
    let sec = if args.len() > 5 { crate::value::to_number(&args[5]) } else { 0.0 };
    let ms = if args.len() > 6 { crate::value::to_number(&args[6]) } else { 0.0 };
    let y = if (0.0..=99.0).contains(&year) { 1900.0 + year } else { year } as i64;
    let dt = chrono::NaiveDate::from_ymd_opt(y as i32, (month as i64 + 1).clamp(1, 12) as u32, day as u32)
        .and_then(|d| d.and_hms_milli_opt(hour as u32, min as u32, sec as u32, ms as u32));
    Ok(dt.map(|d| d.and_utc().timestamp_millis() as f64).unwrap_or(f64::NAN))
}

fn year_of(ms: f64) -> i64 {
    chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| d.year() as i64).unwrap_or(0)
}
fn month_of(ms: f64) -> i64 { chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| d.month0() as i64).unwrap_or(0) }
fn day_of(ms: f64) -> i64 { chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| d.day() as i64).unwrap_or(0) }
fn hours_of(ms: f64) -> i64 { chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| d.hour() as i64).unwrap_or(0) }
fn minutes_of(ms: f64) -> i64 { chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| d.minute() as i64).unwrap_or(0) }
fn seconds_of(ms: f64) -> i64 { chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| d.second() as i64).unwrap_or(0) }
fn ms_of(ms: f64) -> i64 { (ms % 1000.0) as i64 }
fn day_of_week(ms: f64) -> i64 { chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| (d.weekday().num_days_from_sunday()) as i64).unwrap_or(0) }

fn parse_date_string(s: &str) -> f64 {
    // ISO 8601 subset
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis() as f64)
        .or_else(|| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0,0,0).map(|dt| dt.and_utc().timestamp_millis() as f64)))
        .unwrap_or(f64::NAN)
}

use chrono::Timelike;
use chrono::Datelike;
