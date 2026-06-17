//! 把 Rust 异步函数注册为 JS 可调用函数的支持模块。
//!
//! ## 设计原理
//!
//! `quickrs` 引擎本身是单线程的（所有 `Value` 都是 `Rc` 计数，`!Send`），
//! 它的事件循环 `asyncrt::run_event_loop` 运行在一个 `tokio::task::LocalSet`
//! 之上。我们利用这个事实，用 **`tokio::task::spawn_local` + `queue_microtask`**
//! 的组合把 Rust `async fn` 接入 JS 的 Promise 体系，全过程无轮询：
//!
//! 1. JS 调用注册的函数 → 立即创建一个 pending `Promise` 返回给 JS。
//! 2. 用 `tokio::task::spawn_local` 在同一个 `LocalSet` 上驱动用户的 Rust future。
//!    （`spawn_local` 接受 `!Send` future，正好匹配 `quickrs` 的 `Rc` 值模型。）
//! 3. future 完成后，把结果（`Result<Value, Value>`）通过闭包捕获，
//!    用 `asyncrt::queue_microtask` 排一个微任务。微任务里拿得到 `&mut Interpreter`，
//!    于是可以调 `resolve_promise` / `reject_promise`。
//! 4. 排完微任务后调 `notify.notify_one()` 唤醒事件循环（见 `asyncrt::run_event_loop`
//!    里的 `select!`），事件循环立即排空微任务，Promise 被 settle，
//!    `.then` / `.catch` 反应随之触发。
//!
//! 这样 Rust 异步函数的完成能 **立刻** 被事件循环感知，不需要任何定时轮询。
//!
//! ## 使用前提
//!
//! 因为用了 `spawn_local`，`Interpreter::run` / `asyncrt::run_event_loop` 必须
//! 跑在一个 `tokio::task::LocalSet` 上下文里（`quickrs` CLI 的 `main.rs` 已经这么做了）。
//!
//! ## 示例
//!
//! ```no_run
//! use std::time::Duration;
//!
//! async fn hello() -> String {
//!     tokio::time::sleep(Duration::from_secs(3)).await;
//!     "hello".into()
//! }
//!
//! # async fn demo() {
//! let mut interp = quickrs::new_interpreter();
//! interp.register_async_string_ok("hello", |_| {
//!     Box::pin(async move { hello().await })
//! });
//! interp.run(r#"hello().then(s => console.log(s))"#).ok();
//! quickrs::asyncrt::run_event_loop(&mut interp).await;
//! # }
//! ```

use crate::builtins::install_global;
use crate::interp::{make_native_value, Interpreter, NativeFn};
use crate::value::Value;
use std::future::Future;
use std::rc::Rc;

impl Interpreter {
    /// 创建一个包装 Rust 异步函数的 **原生函数值**（不挂到全局）。
    ///
    /// 调用时立即返回一个 pending `Promise`；用户 future 在当前 `LocalSet`
    /// 上被 `spawn_local` 驱动；完成后用微任务 settle 该 Promise。
    ///
    /// 适合需要把异步函数挂到某个对象上做方法（而不是全局函数）的场景：
    ///
    /// ```ignore
    /// let f = interp.make_async_fn("query", |args| { /* ... */ });
    /// interp.set_property(&obj, &PropKey::from_str("query"), f).ok();
    /// ```
    pub fn make_async_fn<F, Fut>(&self, name: &str, f: F) -> Value
    where
        F: Fn(&[Value]) -> Fut + 'static,
        Fut: Future<Output = Result<Value, Value>> + 'static,
    {
        let f = Rc::new(f);
        let realm = self.realm().clone();
        let native: NativeFn = Rc::new(move |interp, _this, args| {
            // JS 调用进来：先把参数 clone 出来（future 必须 'static，不能借用 args）
            let args_owned: Vec<Value> = args.to_vec();
            let f_clone = f.clone();
            // 立即建一个 Promise 返给 JS
            let promise = interp.new_promise();
            let promise_clone = promise.clone();
            let rt = interp.shared.async_rt.clone();
            // 登记：有一个 Rust future 在飞，事件循环不能提前退出
            rt.borrow_mut().pending_rust_futures += 1;
            // ★ 在当前 LocalSet 上 spawn future。run_event_loop 会和它协作调度。
            tokio::task::spawn_local(async move {
                let fut = f_clone(&args_owned);
                let result = fut.await;
                // future 完成：排一个微任务去 settle Promise
                // （微任务能拿到 &mut Interpreter，所以能调 resolve/reject）
                let rt_for_mt = rt.clone();
                crate::asyncrt::queue_microtask(&rt_for_mt, Box::new(move |interp| {
                    match result {
                        Ok(v) => interp.resolve_promise(promise_clone.clone(), v),
                        Err(e) => interp.reject_promise(promise_clone.clone(), e),
                    }
                }));
                // 通知事件循环有活干了（它会醒来排空上面的微任务）
                rt.borrow_mut().pending_rust_futures -= 1;
                rt.borrow().notify.notify_one();
            });
            Ok(promise)
        });
        make_native_value(&realm, name, 0, native)
    }

    /// 注册一个 Rust 异步函数为全局 JS 函数。
    ///
    /// 闭包 `f` 接收 JS 参数切片 `&[Value]`，返回一个 future；
    /// future 产出 `Result<Value, Value>`：`Ok` → Promise resolve，
    /// `Err` → Promise reject（`Err` 的 `Value` 通常是 `error::throw_*` 构造的 Error 对象）。
    ///
    /// # 要求
    /// 调用 `interp.run(...)` 的代码必须跑在 `tokio::task::LocalSet` 上下文里。
    ///
    /// # 例子
    /// ```no_run
    /// use quickrs::value::Value;
    /// use std::time::Duration;
    ///
    /// # async fn _demo() {
    /// let mut interp = quickrs::new_interpreter();
    /// interp.register_async_fn("fetchData", |args| {
    ///     let n = quickrs::value::to_number(args.get(0).unwrap_or(&Value::Undefined)) as u64;
    ///     Box::pin(async move {
    ///         tokio::time::sleep(Duration::from_millis(n)).await;
    ///         Ok(Value::from_str("done"))
    ///     })
    /// });
    /// # }
    /// ```
    pub fn register_async_fn<F, Fut>(&mut self, name: &str, f: F)
    where
        F: Fn(&[Value]) -> Fut + 'static,
        Fut: Future<Output = Result<Value, Value>> + 'static,
    {
        let v = self.make_async_fn(name, f);
        let realm = self.realm().clone();
        install_global(self, &realm, name, v);
    }

    /// 便捷封装：注册一个 **返回 `String`** 的 Rust 异步函数（不可失败版本）。
    ///
    /// Promise 总是 resolve 为该字符串。如果需要失败路径（reject），
    /// 请直接用 [`register_async_fn`](Self::register_async_fn) 并返回
    /// `Err(error::throw_type(...))` 等。
    ///
    /// # 例子（对应用户示例里的 `hello`）
    /// ```no_run
    /// use std::time::Duration;
    ///
    /// async fn hello() -> String {
    ///     tokio::time::sleep(Duration::from_secs(3)).await;
    ///     "hello".into()
    /// }
    ///
    /// # async fn _demo() {
    /// let mut interp = quickrs::new_interpreter();
    /// interp.register_async_string_ok("hello", |_| {
    ///     Box::pin(async move { hello().await })
    /// });
    /// # }
    /// ```
    pub fn register_async_string_ok<F, Fut>(&mut self, name: &str, f: F)
    where
        F: Fn(&[Value]) -> Fut + 'static,
        Fut: Future<Output = String> + 'static,
    {
        self.register_async_fn(name, move |args| {
            let fut = f(args);
            Box::pin(async move {
                let s = fut.await;
                Ok(Value::from_string(s))
            })
        });
    }

    /// 便捷封装：注册一个 **返回 `f64`** 的 Rust 异步函数（不可失败版本）。
    pub fn register_async_number_ok<F, Fut>(&mut self, name: &str, f: F)
    where
        F: Fn(&[Value]) -> Fut + 'static,
        Fut: Future<Output = f64> + 'static,
    {
        self.register_async_fn(name, move |args| {
            let fut = f(args);
            Box::pin(async move {
                let n = fut.await;
                Ok(Value::from_f64(n))
            })
        });
    }
}
