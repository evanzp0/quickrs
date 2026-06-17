//! Integration tests for `Interpreter::register_async_fn` and friends.
//!
//! These run inside a `tokio::task::LocalSet` (required by `spawn_local`).

use quickrs::value::{to_number, to_string, Value};
use std::time::Duration;

/// 用户示例里的异步函数：睡 3 秒返回 "hello"。
/// 测试里缩短到 50ms 以加快跑测。
async fn hello() -> String {
    tokio::time::sleep(Duration::from_millis(50)).await;
    "hello".into()
}

/// `interp.run(...)` 返回 `Result<Value, Value>`，而 `Value` 没实现 `Debug`，
/// 所以不能直接 `.unwrap()`。这个 helper 把 JS 异常转成可读字符串后 panic。
fn run(interp: &mut quickrs::Interpreter, src: &str) {
    if let Err(e) = interp.run(src) {
        panic!("JS threw: {}", quickrs::error::display_value(&e));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn test_async_string_resolves() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut interp = quickrs::new_interpreter();
            interp.register_async_string_ok("hello", |_| {
                Box::pin(async move { hello().await })
            });

            run(&mut interp, r#"hello().then(s => { globalThis.__r = s; });"#);

            quickrs::asyncrt::run_event_loop(&mut interp).await;

            let r = interp.get_global("__r");
            assert_eq!(to_string(&r), "hello");
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn test_async_fn_with_args() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut interp = quickrs::new_interpreter();
            interp.register_async_fn("addAsync", |args| {
                let a = to_number(args.get(0).unwrap_or(&Value::Undefined));
                let b = to_number(args.get(1).unwrap_or(&Value::Undefined));
                Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Ok(Value::from_f64(a + b))
                })
            });

            run(&mut interp, r#"addAsync(3, 4).then(v => { globalThis.__sum = v; });"#);

            quickrs::asyncrt::run_event_loop(&mut interp).await;

            let s = interp.get_global("__sum");
            assert_eq!(to_number(&s), 7.0);
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn test_async_fn_rejects() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut interp = quickrs::new_interpreter();
            interp.register_async_fn("failAsync", |_| {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Err(quickrs::error::throw_type("boom"))
                })
            });

            run(&mut interp, r#"failAsync().catch(e => { globalThis.__err = e.message; });"#);

            quickrs::asyncrt::run_event_loop(&mut interp).await;

            let e = interp.get_global("__err");
            assert_eq!(to_string(&e), "boom");
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn test_async_fn_await_in_js() {
    // JS 侧用 async/await 调用 Rust 异步函数
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut interp = quickrs::new_interpreter();
            interp.register_async_string_ok("greet", |args| {
                let name = to_string(args.get(0).unwrap_or(&Value::Undefined));
                Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    format!("hi, {}", name)
                })
            });

            run(
                &mut interp,
                r#"
                async function main() {
                    let s = await greet("world");
                    globalThis.__msg = s;
                }
                main();
                "#,
            );

            quickrs::asyncrt::run_event_loop(&mut interp).await;

            let m = interp.get_global("__msg");
            assert_eq!(to_string(&m), "hi, world");
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn test_async_fn_concurrent_promise_all() {
    // 多个 Rust 异步函数并发执行，Promise.all 等所有完成
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut interp = quickrs::new_interpreter();
            interp.register_async_fn("delay", |args| {
                let ms = to_number(args.get(0).unwrap_or(&Value::from_int(0))) as u64;
                let label = args.get(1).cloned().unwrap_or(Value::Undefined);
                Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    Ok(label)
                })
            });

            // 故意让 b(50ms) 先完成、a(100ms) 后完成，但 Promise.all 保持顺序
            run(
                &mut interp,
                r#"
                Promise.all([
                    delay(100, "a"),
                    delay(50, "b"),
                    delay(80, "c"),
                ]).then(arr => {
                    globalThis.__len = arr.length;
                    globalThis.__first = arr[0];
                    globalThis.__second = arr[1];
                    globalThis.__third = arr[2];
                });
                "#,
            );

            quickrs::asyncrt::run_event_loop(&mut interp).await;

            assert_eq!(to_number(&interp.get_global("__len")), 3.0);
            assert_eq!(to_string(&interp.get_global("__first")), "a");
            assert_eq!(to_string(&interp.get_global("__second")), "b");
            assert_eq!(to_string(&interp.get_global("__third")), "c");
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn test_async_fn_interleaved_with_settimeout() {
    // Rust 异步函数与 JS setTimeout 交错执行，验证事件循环 select! 正确唤醒
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut interp = quickrs::new_interpreter();
            interp.register_async_string_ok("rustDelay", |_| {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_millis(60)).await;
                    "rust".to_string()
                })
            });

            run(
                &mut interp,
                r#"
                let log = [];
                setTimeout(() => log.push("t1"), 30);   // 30ms
                rustDelay().then(s => log.push(s));      // 60ms
                setTimeout(() => log.push("t2"), 90);    // 90ms
                globalThis.__log = log;
                "#,
            );

            quickrs::asyncrt::run_event_loop(&mut interp).await;

            let log = interp.get_global("__log");
            let len = interp
                .get_property(&log, &quickrs::value::PropKey::from_str("length"))
                .unwrap_or(Value::Undefined);
            assert_eq!(to_number(&len), 3.0);
            // 顺序应为 t1(30) < rust(60) < t2(90)
            let e0 = interp
                .get_property(&log, &quickrs::value::PropKey::from_str("0"))
                .unwrap_or(Value::Undefined);
            let e1 = interp
                .get_property(&log, &quickrs::value::PropKey::from_str("1"))
                .unwrap_or(Value::Undefined);
            let e2 = interp
                .get_property(&log, &quickrs::value::PropKey::from_str("2"))
                .unwrap_or(Value::Undefined);
            assert_eq!(to_string(&e0), "t1");
            assert_eq!(to_string(&e1), "rust");
            assert_eq!(to_string(&e2), "t2");
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn test_async_fn_number_ok() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut interp = quickrs::new_interpreter();
            interp.register_async_number_ok("pi", |_| {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    3.14159
                })
            });

            run(&mut interp, r#"pi().then(v => { globalThis.__pi = v; });"#);

            quickrs::asyncrt::run_event_loop(&mut interp).await;

            assert!((to_number(&interp.get_global("__pi")) - 3.14159).abs() < 1e-9);
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn test_async_fn_called_multiple_times() {
    // 同一个注册函数被调用多次，每次都正确 resolve
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut interp = quickrs::new_interpreter();
            interp.register_async_fn("echo", |args| {
                let v = args.get(0).cloned().unwrap_or(Value::Undefined);
                Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    Ok(v)
                })
            });

            run(
                &mut interp,
                r#"
                let n = 0;
                let p1 = echo("a").then(v => { globalThis.__a = v; n++; });
                let p2 = echo("b").then(v => { globalThis.__b = v; n++; });
                let p3 = echo("c").then(v => { globalThis.__c = v; n++; });
                Promise.all([p1, p2, p3]).then(() => { globalThis.__n = n; });
                "#,
            );

            quickrs::asyncrt::run_event_loop(&mut interp).await;

            assert_eq!(to_string(&interp.get_global("__a")), "a");
            assert_eq!(to_string(&interp.get_global("__b")), "b");
            assert_eq!(to_string(&interp.get_global("__c")), "c");
            assert_eq!(to_number(&interp.get_global("__n")), 3.0);
        })
        .await;
}
