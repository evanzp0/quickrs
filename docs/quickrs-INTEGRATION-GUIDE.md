# quickrs —— Rust 集成教程与 API 参考手册

> 本文档基于对 `quickrs` 源码（约 1.5 万行 Rust，含 `src/` 11 个顶层模块 + `src/builtins/` 16 个内置模块）的完整分析整理而成。
> 适用于 `quickrs 0.1.0`（Cargo edition 2021）。

---

## 目录

1. [项目概述](#1-项目概述)
2. [目录结构与模块职责](#2-目录结构与模块职责)
3. [集成前的关键约束（必读）](#3-集成前的关键约束必读)
4. [快速开始：5 分钟跑起来](#4-快速开始5-分钟跑起来)
5. [核心 API 参考](#5-核心-api-参考)
   - 5.1 [`lib.rs` 顶层入口](#51-librs-顶层入口)
   - 5.2 [`Interpreter` —— 引擎主对象](#52-interpreter--引擎主对象)
   - 5.3 [`Value` —— JS 值的 Rust 表示](#53-value--js-值的-rust-表示)
   - 5.4 [`ObjectInner` / `ObjectKind` / `Property` / `PropKey`](#54-objectinner--objectkind--property--propkey)
   - 5.5 [`Function` / `FunctionBody` / `NativeFn` / `CtorFn`](#55-function--functionbody--nativefn--ctorfn)
   - 5.6 [`Realm` —— 全局执行环境](#56-realm--全局执行环境)
   - 5.7 [`scope::Env` —— 作用域链](#57-scopeenv--作用域链)
   - 5.8 [`asyncrt` —— 异步运行时](#58-asyncrt--异步运行时)
   - 5.9 [`error` —— 异常与解析错误](#59-error--异常与解析错误)
   - 5.10 [`parser` —— 词法/语法分析](#510-parser--词法语法分析)
   - 5.11 [`builtins` —— 内置对象安装器与工具函数](#511-builtins--内置对象安装器与工具函数)
6. [典型集成场景（含完整可运行代码）](#6-典型集成场景含完整可运行代码)
7. [完整示例工程](#7-完整示例工程)
8. [注意事项与已知坑](#8-注意事项与已知坑)

---

## 1. 项目概述

`quickrs` 是一个 **用纯 Rust 实现的 JavaScript 引擎**，灵感来自 QuickJS，但执行模型有以下特色：

| 维度 | 实现 |
|---|---|
| **解析** | 手写递归下降 parser（`src/parser.rs` + `src/lexer.rs`），覆盖 ES2020+ 大部分语法：箭头函数、类、解构、生成器、async/await、模板字符串、正则字面量、ESM `import/export` |
| **执行** | AST-walking 树遍历解释器（`src/interp.rs`），无 JIT/字节码 |
| **协程** | 用 `corosensei` 提供 **stackful coroutine**，实现 `function*` 生成器与 `async function` |
| **异步** | 基于 `tokio` 的 `current_thread` runtime + `LocalSet`，单线程驱动微任务队列 / 定时器 / Promise |
| **值模型** | `Rc` 引用计数 + `RefCell` 内部可变；对象是 `Rc<RefCell<ObjectInner>>` |
| **模块** | 同时支持 ESM（`import/export`）与 CommonJS（`require/module.exports`，内置 `fs/path/os/buffer/util/crypto/events/url/querystring`） |
| **内置对象** | Object/Function/Array/String/Number/Boolean/Symbol/BigInt/Math/JSON/Error 全家桶、Map/Set/Date/RegExp/Promise/Proxy/Reflect/ArrayBuffer/TypedArray、`console`、`setTimeout` 等 |
| **CLI** | `src/main.rs` 提供 `quickrs run file.js`、`quickrs repl`、`quickrs -e "expr"` |

### 已支持的 JS 语言特性（摘自 `examples/suite.js` / `examples/smoke.js`）

- 完整表达式运算符（含 `**`、`??`、`?.`、位运算、比较）
- `let/const/var`、TDZ、闭包、IIFE
- 解构（数组/对象/嵌套/默认值/rest）
- 模板字符串与标签模板
- 类（`extends`、`super`、getter/setter、静态成员、字段）
- 生成器（`function*` / `yield*`）
- `async/await` + Promise 链
- `for..of` / `for..in` / `switch` / `try/catch/finally` / `with`
- ESM `import/export default/*`
- Symbol / 迭代器协议 / `@@iterator` / `@@asyncIterator`
- BigInt、TypedArray、ArrayBuffer
- Proxy / Reflect

---

## 2. 目录结构与模块职责

```
quickrs/
├── Cargo.toml              # 包定义；同时声明 [lib] 与 [[bin]]
├── src/
│   ├── lib.rs              # 库 crate 入口：re-export + new_interpreter()
│   ├── main.rs             # CLI 二进制：run / repl / -e
│   ├── lexer.rs            # 词法分析（Token / Keyword / Punct）
│   ├── parser.rs           # 递归下降 parser，输出 ast::Program
│   ├── ast.rs              # AST 类型定义（Stmt / Expr / Pattern / ...）
│   ├── interp.rs           # ★ 解释器主体（3833 行，引擎核心）
│   ├── value.rs            # ★ Value / ObjectInner / Function / PropKey ...
│   ├── realm.rs            # Realm：全局对象 + 所有 intrinsic 原型/构造器
│   ├── scope.rs            # 词法环境（Env / Binding / EnvKind）
│   ├── asyncrt.rs          # 微任务队列 + 定时器 + run_event_loop
│   ├── error.rs            # JS 异常值构造 + ParseError + display_value
│   └── builtins/
│       ├── mod.rs          # install() 总装 + 共享 helper（make_ctor / install_global ...）
│       ├── object.rs  array.rs  string_b.rs  number.rs  symbol.rs
│       ├── math.rs    json.rs    errors.rs    mapset.rs   date.rs
│       ├── regexp.rs  promise.rs proxy.rs     typed.rs
│       ├── console.rs globals.rs node_modules.rs
└── examples/
    ├── smoke.js            # 语言特性烟雾测试
    └── suite.js            # 152 项断言测试套件
```

### 模块依赖关系（自顶向下）

```
        lib.rs
          │
          ├── new_interpreter() ─→ Realm::new() + builtins::install()
          │
          ├── Interpreter (interp.rs)
          │     ├── Realm (realm.rs)
          │     ├── Env (scope.rs)
          │     ├── AsyncRt (asyncrt.rs)
          │     └── Value (value.rs)
          │
          ├── parser (parser.rs + lexer.rs + ast.rs)
          │
          └── error (error.rs)
```

---

## 3. 集成前的关键约束（必读）

`quickrs` 的设计有几个**硬性约束**，集成时必须遵守，否则会编译报错或运行时 panic：

### 3.1 单线程 + `!Send`

引擎内部所有值都是 `Rc<...>` / `RefCell<...>`，**整个 `Interpreter` / `Value` / `Realm` 都是 `!Send` 且 `!Sync`**。

- ❌ 不能把 `Interpreter` 放到 `tokio::spawn(async move {...})` 这类要求 `Send` 的多线程任务里。
- ❌ 不能跨 `await` 点持有 `Interpreter`（除非整段都在 `LocalSet` 内）。
- ✅ 必须运行在 `tokio::runtime::Builder::new_current_thread()` + `tokio::task::LocalSet` 里。

### 3.2 异步必须驱动事件循环

`async function`、`setTimeout`、`Promise.then` 都是**懒**的，必须显式调用 `asyncrt::run_event_loop(&mut interp).await` 才会执行。只 `interp.run(src)` 是不够的。

### 3.3 借用冲突

`ObjectInner` 是 `RefCell`，`interp.get_property(&obj, &key)` 内部会 `borrow()`。如果你已经 `borrow_mut()` 了同一个对象还没释放，再调解释器方法会 panic。**最佳实践：先用 `let b = o.borrow()` 把需要的字段 clone 出来，drop 掉 borrow，再调解释器。**

### 3.4 错误是 `Value` 而非 `Error`

JS 抛出的异常在 Rust 侧表现为 `Result<Value, Value>`——`Err(Value)` 里那个 `Value` 通常是 `ObjectKind::Error` 的对象。要把它转成人读的字符串，用 `quickrs::error::display_value(&e)`。

### 3.5 递归深度限制

`Interpreter` 内置 `MAX_DEPTH = 1200`，超过会抛 `RangeError: Maximum call stack size exceeded`。可在构造后改 `interp.shared.max_depth`。

---

## 4. 快速开始：5 分钟跑起来

### 4.1 引入依赖

`quickrs` 是本地 crate（未发布 crates.io），用 path 依赖：

```toml
# 你的项目的 Cargo.toml
[dependencies]
quickrs = { path = "../quickrs" }   # 按实际相对路径
tokio = { version = "1", features = ["full"] }
```

> `quickrs` 自身依赖：`tokio (full)`、`clap`、`rustyline`、`regex`、`fancy-regex`、`chrono`、`serde_json`、`corosensei`、`indexmap`、`md-5/sha1/sha2/digest/hex`。这些会自动拉入。

### 4.2 最小例子：eval 一段 JS

```rust
// src/main.rs (你的项目)
use quickrs::Interpreter;

fn main() {
    // ① 创建带全部内置对象的解释器
    let mut interp = quickrs::new_interpreter();

    // ② 跑一段 JS
    let result = interp.run("1 + 2 * 3").expect("JS threw");
    println!("结果: {}", quickrs::value::to_string(&result));
    // → 结果: 7
}
```

### 4.3 跑带 `setTimeout` / `async` 的 JS

需要 Tokio runtime + LocalSet + 事件循环：

```rust
use quickrs;

#[tokio::main(flavor = "current_thread")]   // ★ 必须 current_thread
async fn main() {
    let local = tokio::task::LocalSet::new();
    local.run_until(async {
        let mut interp = quickrs::new_interpreter();
        interp.run(r#"
            setTimeout(() => console.log("100ms 后打印"), 100);
            async function f(){ return 42; }
            f().then(v => console.log("async:", v));
        "#).unwrap();
        // ★ 不调这一行，上面两个回调都不会执行
        quickrs::asyncrt::run_event_loop(&mut interp).await;
    }).await;
}
```

> 提示：`#[tokio::main(flavor = "current_thread")]` 等价于 `Builder::new_current_thread().enable_all().build()`。`LocalSet` 是为了让 `!Send` 的 future 能在 current-thread runtime 上跑。

---

## 5. 核心 API 参考

下面按模块列出**对外 `pub`** 的 API（私有/内部函数不在文档范围内，但会标注哪些虽是 `pub` 但实际只给 builtins 用的）。

### 5.1 `lib.rs` 顶层入口

```rust
pub mod ast;
pub mod asyncrt;
pub mod builtins;
pub mod error;
pub mod interp;
pub mod lexer;
pub mod parser;
pub mod realm;
pub mod scope;
pub mod value;

pub use interp::Interpreter;
pub use realm::Realm;

/// 创建一个完整初始化的解释器：Realm + 所有内置对象已安装。
/// 这是你 99% 场景下要调用的入口。
pub fn new_interpreter() -> Interpreter;
```

所有模块都是 `pub mod`，所以你也可以 `use quickrs::value::Value;` 直接拿到子模块里的类型。

---

### 5.2 `Interpreter` —— 引擎主对象

> 定义于 `src/interp.rs`。这是整个库最核心的类型。

#### 5.2.1 结构

```rust
#[derive(Clone)]
pub struct Interpreter {
    pub shared: Rc<Shared>,   // 共享的 realm + 异步状态 + 调用栈
    pub scope: Env,           // 当前作用域（clone 后用于协程切换）
}

pub struct Shared {
    pub realm: Rc<Realm>,
    pub async_rt: Rc<RefCell<AsyncRt>>,
    pub yielder: Cell<*const ()>,     // 当前协程的 yielder（生成器/async 用）
    pub depth: Cell<usize>,           // 当前调用深度
    pub max_depth: usize,             // 默认 1200
    pub stack: RefCell<Vec<String>>,  // 调用栈标签（用于 stack trace）
}
```

`Interpreter` 是 `Clone` 的，**clone 出来的副本共享同一份 realm 与异步状态**，但有自己的 `scope`。一般你不需要 clone，除非要写自定义协程驱动。

#### 5.2.2 构造与入口

| 方法 | 签名 | 说明 |
|---|---|---|
| `new` | `(realm: Rc<Realm>) -> Self` | 裸构造，**不装内置对象**。通常用 `new_interpreter()` 代替 |
| `realm` | `(&self) -> &Rc<Realm>` | 取 realm 引用 |
| `run` | `(&mut self, src: &str) -> Result<Value, Value>` | **主入口**：自动检测 ESM（含 `import/export`）→ 走 `eval_module`；否则走 `eval_program`。返回最后一个表达式的值，`Err` 是 JS 抛出的异常值 |
| `eval_program` | `(&mut self, prog: &Program) -> Result<Value, Value>` | 直接执行预解析的 AST（Script 模式） |
| `eval_module` | `(&mut self, prog: &Program) -> Result<Value, Value>` | ESM 模式执行，返回模块命名空间对象（含所有 `export`） |

#### 5.2.3 全局/属性访问

| 方法 | 签名 | 说明 |
|---|---|---|
| `get_global` | `(&mut self, name: &str) -> Value` | 读全局变量（如 `get_global("Math")`）；不存在返回 `Undefined` |
| `get_property` | `(&mut self, obj: &Value, key: &PropKey) -> Result<Value, Value>` | 读属性（走原型链 + Proxy + getter） |
| `set_property` | `(&mut self, obj: &Value, key: &PropKey, value: Value) -> Result<(), Value>` | 写属性（走 setter / Proxy） |
| `has_property` | `(&mut self, obj: &Value, key: &PropKey) -> bool` | `in` 操作符语义 |
| `delete_property` | `(&mut self, obj: &Value, key: &PropKey) -> Result<bool, Value>` | `delete` 操作符 |
| `own_property_keys` | `(&mut self, obj: &Value) -> Result<Vec<PropKey>, Value>` | 自有属性键（含 symbol） |
| `get_prototype_of` | `(&mut self, obj: &Value) -> Result<Value, Value>` | 取 `[[Prototype]]` |
| `as_proxy` | `(&self, v: &Value) -> Option<Rc<ProxyData>>` | 若是 Proxy 返回其内部数据 |

#### 5.2.4 函数调用与构造

| 方法 | 签名 | 说明 |
|---|---|---|
| `call_value` | `(&mut self, func: Value, this: Value, args: &[Value]) -> Result<Value, Value>` | 调用任意 callable（JS 函数 / 原生函数 / BoundFunction / Proxy of function） |
| `construct` | `(&mut self, func: Value, args: &[Value], new_target: Value) -> Result<Value, Value>` | `new` 语义；`new_target` 通常传 `func.clone()` |
| `construct_with_this` | | `construct` 的变体，允许显式指定 `this`（用于 `Reflect.construct`） |

#### 5.2.5 类型转换（ES 抽象操作）

| 方法 | 签名 | 对应 ES 抽象操作 |
|---|---|---|
| `to_object` | `(&mut self, v: &Value) -> Result<Value, Value>` | ToObject |
| `to_primitive` | `(&mut self, v: &Value, hint: &str) -> Result<Value, Value>` | ToPrimitive（`"number"` / `"string"` / `"default"`） |
| `to_promise` | `(&mut self, v: Value) -> Result<Value, Value>` | Promise.resolve 语义 |

#### 5.2.6 工厂方法

| 方法 | 签名 | 说明 |
|---|---|---|
| `new_array` | `(&self, items: Vec<Value>) -> Value` | 创建数组对象（设好 `Array.prototype`） |
| `new_promise` | `(&self) -> Value` | 创建 pending Promise |
| `promise_state` | `(&self, p: &Value) -> Option<Rc<RefCell<PromiseState>>>` | 取 Promise 内部状态 |
| `resolve_promise` | `(&mut self, promise: Value, value: Value)` | resolve（自动处理 thenable 链） |
| `reject_promise` | `(&mut self, promise: Value, reason: Value)` | reject |
| `make_native` | `(&self, name: &str, length: usize, func: NativeFn) -> Value` | 用 Rust 闭包创建一个原生函数对象 |
| `make_function` | `(&self, fe: &FunctionExpr, is_async: bool, is_generator: bool, closure: Env) -> Value` | 从 AST 创建 JS 函数对象（高级用法） |

#### 5.2.7 迭代器协议

| 方法 | 签名 | 说明 |
|---|---|---|
| `is_iterable` | `(&self, v: &Value) -> bool` | 是否有 `@@iterator` |
| `get_iterator` | `(&mut self, v: &Value) -> Result<Value, Value>` | 调 `obj[Symbol.iterator]()` 拿迭代器对象 |
| `iterator_step` | `(&mut self, iter: &Value) -> Result<Option<Value>, Value>` | 调 `iter.next()`，返回 `Some(value)` 或 `None`（done） |
| `iterable_to_vec` | `(&mut self, v: &Value) -> Result<Vec<Value>, Value>` | 把可迭代物展开成 `Vec<Value>`（数组/字符串有快路径） |
| `coerce_to_string` | `(&mut self, v: &Value) -> Result<Rc<str>, Value>` | ToString（处理对象 `toString()`） |
| `coerce_to_number` | `(&mut self, v: &Value) -> Result<f64, Value>` | ToNumber（处理 `valueOf`） |

> 注：`iterable_to_vec` / `coerce_to_string` / `coerce_to_number` 定义在 `builtins/mod.rs` 的 `impl Interpreter` 块里，但都是 `pub`，外部可用。

#### 5.2.8 模块加载

| 方法 | 签名 | 说明 |
|---|---|---|
| `load_module` | `(&mut self, specifier: &str) -> Result<Value, Value>` | 从文件系统加载 ESM 模块（`./` / `/` 开头），返回命名空间对象。会缓存到 `realm.module_cache` |

#### 5.2.9 执行辅助

| 方法 | 签名 | 说明 |
|---|---|---|
| `exec_stmt` | `(&mut self, s: &Stmt) -> Result<Completion, Value>` | 执行单条语句 |
| `eval_expr` | `(&mut self, e: &Expr) -> Result<Value, Value>` | 求值单个表达式 |
| `binary_op` | `(&mut self, op: BinaryOp, lv: &Value, rv: &Value) -> Result<Value, Value>` | 二元运算 |
| `bind_pattern` | `(&mut self, pat: &Pattern, val: &Value, env: &Env, ...) -> Result<(), Value>` | 解构绑定 |
| `flatten_into` | `(&mut self, items: &mut Vec<Value>, v: &Value) -> Result<(), Value>` | `...spread` 展开辅助 |

#### 5.2.10 关联类型

```rust
pub enum Completion {
    Normal(Value),
    Return(Value),
    Break(Option<Rc<str>>),
    Continue(Option<Rc<str>>),
}
impl Completion {
    pub fn unwrap_value(self) -> Value;
}
```

---

### 5.3 `Value` —— JS 值的 Rust 表示

> 定义于 `src/value.rs`。

#### 5.3.1 枚举定义

```rust
#[derive(Clone)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),          // 所有 JS number 都是 f64
    String(Rc<str>),
    Symbol(Rc<Symbol>),
    Object(ObjRef),       // ObjRef = Rc<RefCell<ObjectInner>>
    BigInt(Rc<BigInt>),
}
```

#### 5.3.2 构造辅助方法（`impl Value`）

| 方法 | 等价于 |
|---|---|
| `Value::undefined()` | `Value::Undefined` |
| `Value::null()` | `Value::Null` |
| `Value::from_bool(b)` | `Value::Bool(b)` |
| `Value::from_f64(n)` | `Value::Number(n)` |
| `Value::from_int(n: i32)` | `Value::Number(n as f64)` |
| `Value::from_str(s: &str)` | `Value::String(Rc::from(s))` |
| `Value::from_string<S: AsRef<str>>(s)` | 同上但泛型 |
| `Value::object(o: ObjRef)` | `Value::Object(o)` |

#### 5.3.3 判断方法

| 方法 | 说明 |
|---|---|
| `is_undefined()` / `is_null()` / `is_nullish()` | undefined / null / 两者皆是 |
| `is_object()` | 是 Object 变体 |
| `is_callable()` | 是函数（含 BoundFunction、Proxy of function） |
| `is_constructor()` | 可作 `new` 调用 |
| `as_object()` | `Option<ObjRef>` |
| `type_of()` | 返回 `typeof` 字符串（`"undefined"` / `"object"` / `"function"` ...） |

#### 5.3.4 全局转换函数（`pub fn`，非方法）

```rust
// ES ToString / ToBoolean / ToNumber 等抽象操作的纯函数实现
pub fn to_string(v: &Value) -> String;       // ToString
pub fn to_boolean(v: &Value) -> bool;        // ToBoolean
pub fn to_number(v: &Value) -> f64;          // ToNumber
pub fn to_integer(v: &Value) -> f64;         // ToIntegerOrInfinity
pub fn to_int32(v: &Value) -> i32;           // ToInt32
pub fn to_uint32(v: &Value) -> u32;          // ToUint32
pub fn to_length(v: &Value) -> usize;        // ToLength
pub fn string_to_number(s: &str) -> f64;     // 字符串→数字（处理 0x/0o/0b/Infinity）
pub fn format_number(n: f64) -> String;      // 数字→最短往返字符串

// 比较
pub fn strict_equals(a: &Value, b: &Value) -> bool;   // ===
pub fn loose_equals(a: &Value, b: &Value) -> bool;    // ==
pub fn same_value(a: &Value, b: &Value) -> bool;      // Object.is
pub fn same_value_zero(a: &Value, b: &Value) -> bool; // Map/Set 用的

// 属性键
pub fn to_property_key(v: &Value) -> PropKey;
pub fn index_to_key(i: usize) -> Rc<str>;
pub fn key_to_index(key: &str) -> Option<usize>;

// BigInt
pub fn bigint_to_string(b: &BigInt) -> String;

// Date
pub fn date_format(ms: f64) -> String;   // 毫秒时间戳 → ISO-8601
```

#### 5.3.5 类型别名

```rust
pub type ObjRef = Rc<RefCell<ObjectInner>>;
```

---

### 5.4 `ObjectInner` / `ObjectKind` / `Property` / `PropKey`

```rust
pub struct ObjectInner {
    pub props: IndexMap<PropKey, Property>,
    pub proto: Option<Value>,
    pub extensible: bool,
    pub kind: ObjectKind,
    pub class: &'static str,        // "Object" / "Array" / "Function" ...
}

pub enum ObjectKind {
    Ordinary,
    Array(Vec<Value>),              // 快数组
    Function(Rc<Function>),
    BoundFunction { target, this_arg, bound_args },
    Error,
    String(Rc<str>),                // String 包装对象
    Number(f64),                    // Number 包装对象
    Boolean(bool),
    Symbol(Rc<Symbol>),
    Map(Vec<(Value, Value)>),
    Set(Vec<Value>),
    Date(f64),                      // 毫秒时间戳
    RegExp(Rc<RegExpData>),
    Promise(Rc<RefCell<PromiseState>>),
    Generator(Rc<RefCell<GeneratorState>>),
    ArrayBuffer(Rc<RefCell<Vec<u8>>>),
    TypedArray { buffer, byte_offset, length, kind: TypedArrayKind },
    Module(Rc<RefCell<ModuleState>>),
    Proxy(Rc<ProxyData>),
}

pub enum TypedArrayKind {
    Uint8, Int8, Uint16, Int16, Uint32, Int32, Float32, Float64,
}

pub struct Property {
    pub kind: PropKind,
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
}
impl Property {
    pub fn data(value: Value) -> Property;   // 默认 writable/enumerable/configurable = true
}

pub enum PropKind {
    Data(Value),
    Accessor { get: Option<Value>, set: Option<Value> },
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum PropKey {
    Str(Rc<str>),
    Sym(Rc<Symbol>),
}
impl PropKey {
    pub fn from_str(s: &str) -> PropKey;
}
impl From<String> for PropKey;
impl From<&str> for PropKey;
```

#### `ObjectInner` 工厂方法

```rust
impl ObjectInner {
    pub fn new_object() -> ObjRef;       // 普通 {} 对象，proto=None
    pub fn new_array(items: Vec<Value>) -> ObjRef;
    pub fn new_function(f: Rc<Function>) -> ObjRef;
}
```

> ⚠️ 这些工厂创建的对象 **`proto` 都是 `None`**。要挂原型，必须手动 `o.borrow_mut().proto = Some(Value::Object(realm.object_proto.clone()));`。在原生代码里造对象最稳妥的方式是模仿 `builtins/mod.rs` 里的写法。

---

### 5.5 `Function` / `FunctionBody` / `NativeFn` / `CtorFn`

#### 5.5.1 函数对象内部表示

```rust
pub struct Function {
    pub body: FunctionBody,
    pub name: Rc<str>,
    pub length: usize,           // 形参个数
    pub closure: Env,
    pub is_arrow: bool,
    pub is_generator: bool,
    pub is_async: bool,
    pub is_method: bool,
    pub is_constructor: bool,
    pub home_object: Option<Value>,    // 用于 super
    pub class_fields: Vec<ClassField>,
    pub parent_class: Option<Value>,
    pub line: u32,
}

pub enum FunctionBody {
    Native {
        func: Rc<dyn Fn(&mut Interpreter, Value, &[Value]) -> Result<Value, Value>>,
        constructor: Option<Rc<dyn Fn(&mut Interpreter, Value, &[Value], Value) -> Result<Value, Value>>>,
    },
    Js {
        params: Vec<Pattern>,
        body: Block,
        decls: Vec<FunctionDecl>,
        strict: bool,
    },
}
```

#### 5.5.2 原生函数签名

```rust
// 普通原生函数：&mut interp, this, args -> Result<Value, Value>
pub type NativeFn = Rc<dyn Fn(&mut Interpreter, Value, &[Value]) -> Result<Value, Value>>;

// 原生构造器：&mut interp, this(新对象), args, new_target -> Result<Value, Value>
// 定义在 builtins/mod.rs，但 pub
pub type CtorFn = Rc<dyn Fn(&mut Interpreter, Value, &[Value], Value) -> Result<Value, Value>>;
```

#### 5.5.3 创建原生函数的两种方式

```rust
// 方式 1：用 Interpreter 上的方法（推荐，自动设 prototype）
let f = interp.make_native("myFn", 2, Rc::new(|interp, this, args| {
    Ok(Value::from_int(args.len() as i32))
}));

// 方式 2：用自由函数（需要 realm）
let f = quickrs::interp::make_native_value(&realm, "myFn", 2, Rc::new(...));
```

#### 5.5.4 创建原生构造器

```rust
use quickrs::builtins::{make_ctor, install_global_ctor};

let call_fn: NativeFn = Rc::new(|_i, _t, args| {
    Ok(Value::from_int(to_int32(args.get(0).unwrap_or(&Value::Undefined))))
});
let ctor_fn: CtorFn = Rc::new(|interp, _this, args, _nt| {
    let o = ObjectInner::new_object();
    o.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
    o.borrow_mut().class = "MyClass";
    o.borrow_mut().props.insert(
        PropKey::from_str("value"),
        Property::data(Value::from_int(to_int32(args.get(0).unwrap_or(&Value::Undefined)))),
    );
    Ok(Value::Object(o))
});
let ctor = make_ctor(&realm, "MyClass", 1, call_fn, ctor_fn);
install_global_ctor(&mut interp, &realm, "MyClass", ctor.clone(), realm.object_proto.clone());
```

---

### 5.6 `Realm` —— 全局执行环境

> 定义于 `src/realm.rs`。一个 Realm = 一套全局对象 + 所有 intrinsic 原型/构造器。多 Realm 隔离用得着。

```rust
pub struct Realm {
    pub global: ObjRef,                  // 全局对象 (globalThis)
    pub globalenv: Env,                  // 全局词法环境
    // 原型
    pub object_proto, function_proto, array_proto, string_proto, number_proto,
       boolean_proto, symbol_proto, bigint_proto, error_proto,
       type_error_proto, range_error_proto, syntax_error_proto, reference_error_proto,
       uri_error_proto, eval_error_proto,
       promise_proto, map_proto, set_proto, date_proto, regexp_proto,
       array_buffer_proto, typed_array_proto, generator_proto,
       iterator_proto, array_iterator_proto, map_iterator_proto,
       set_iterator_proto, string_iterator_proto: ObjRef,
    // 构造器（Value 形式）
    pub object_ctor, array_ctor, string_ctor, /* ... */ float64_array_ctor: Value,
    pub wk: WellKnownSymbols,
    pub symbol_counter: Cell<u64>,
    pub modules: RefCell<IndexMap<String, ModuleEntry>>,
    pub module_cache: RefCell<IndexMap<String, Value>>,
}

impl Realm {
    pub fn new() -> Rc<Realm>;                       // 创建空 Realm（无内置对象）
    pub fn new_symbol(&self, desc: Option<Rc<str>>) -> Rc<Symbol>;
    pub fn proto_for(&self, class: &str) -> Option<ObjRef>;
}

pub struct WellKnownSymbols {
    pub iterator, async_iterator, has_instance, to_primitive,
       to_string_tag, is_concat_spreadable: Rc<Symbol>,
}
```

> ⚠️ `Realm::new()` 只建空壳，**内置对象要靠 `builtins::install(&mut interp)` 安装**。`new_interpreter()` 帮你做了这两步。

---

### 5.7 `scope::Env` —— 作用域链

> 定义于 `src/scope.rs`。表示词法环境记录。

```rust
#[derive(Clone)]
pub struct Env(pub Rc<RefCell<EnvInner>>);

pub struct EnvInner {
    pub bindings: IndexMap<Rc<str>, Binding>,
    pub parent: Option<Env>,
    pub kind: EnvKind,             // Global / Function / Block / Module / Class / With
    pub this_val: Value,
    pub new_target: Option<Value>,
    pub home_object: Option<Value>,
    pub parent_constructor: Option<Value>,
    pub with_object: Option<Value>,
    pub func_decls: Vec<Rc<str>>,
}

pub struct Binding {
    pub value: Value,
    pub mutable: bool,            // const = false
    pub initialized: bool,        // TDZ：let/const 在初始化前 = false
}

impl Env {
    pub fn new(parent: Option<Env>, kind: EnvKind) -> Env;
    pub fn global() -> Env;
    pub fn create(&self, name: &Rc<str>, value: Value, mutable: bool);
    pub fn create_uninit(&self, name: &Rc<str>, mutable: bool);  // TDZ
    pub fn has_own(&self, name: &str) -> bool;
    pub fn resolve(&self, name: &str) -> Option<Env>;
    pub fn get(&self, name: &str) -> Result<Value, Value>;       // 沿链查找，遵守 TDZ
    pub fn set(&self, name: &str, value: Value) -> Result<(), Value>;
    pub fn this(&self) -> Value;
    pub fn new_target(&self) -> Option<Value>;
    pub fn home_object(&self) -> Option<Value>;
    pub fn parent_constructor(&self) -> Option<Value>;
}
```

集成时一般不直接用 `Env`，除非你要**手写原生函数并访问/创建作用域**（例如实现 `eval`、`with`、自定义模块包装器）。`globals.rs` 里 `require()` 的实现就是一个完整范例。

---

### 5.8 `asyncrt` —— 异步运行时

> 定义于 `src/asyncrt.rs`。微任务队列 + 定时器，由 Tokio current_thread + LocalSet 驱动。

#### 5.8.1 核心结构

```rust
pub type Microtask = Box<dyn FnOnce(&mut Interpreter)>;

pub struct AsyncRt {
    pub microtasks: VecDeque<Microtask>,
    pub next_timer_id: u64,
    pub timers: Vec<MacroTask>,
    pub stop: bool,
    pub exit_code: i32,
}

impl AsyncRt {
    pub fn new() -> Rc<RefCell<AsyncRt>>;
}
```

#### 5.8.2 API

```rust
/// 把一个微任务排入队列（对应 Promise.then 的 reaction）
pub fn queue_microtask(rt: &Rc<RefCell<AsyncRt>>, t: Microtask);

/// 安排一个宏任务（setTimeout），返回 timer id
pub fn set_timeout(rt: &Rc<RefCell<AsyncRt>>, delay_ms: i64, task: Microtask) -> u64;

/// 取消定时器（best-effort，见源码注释）
pub fn clear_timeout(rt: &Rc<RefCell<AsyncRt>>, id: u64);

/// ★ 事件循环主入口：循环 → 排空微任务 → 跑到期定时器 → 等 Tokio reactor
///     直到 microtasks 与 timers 都空。返回 process.exit 的退出码。
pub async fn run_event_loop(interp: &mut Interpreter) -> i32;
```

#### 5.8.3 在原生函数里调度异步任务

```rust
use quickrs::{asyncrt, interp::Interpreter, value::*};
use std::rc::Rc;

// 在原生函数里：返回一个 Promise，1 秒后 resolve
let f = interp.make_native("delayed", 0, Rc::new(|interp, _this, _args| {
    let p = interp.new_promise();
    let p_clone = p.clone();
    let rt = interp.shared.async_rt.clone();
    asyncrt::set_timeout(&rt, 1000, Box::new(move |interp| {
        interp.resolve_promise(p_clone.clone(), Value::from_str("done"));
    }));
    Ok(p)
}));
```

> `interp.shared.async_rt` 是 `Rc<RefCell<AsyncRt>>`，可以直接 clone 出来在闭包里持有。

---

### 5.9 `error` —— 异常与解析错误

> 定义于 `src/error.rs`。

#### 5.9.1 构造 JS 异常值

```rust
pub fn throw_error(class: &str, message: &str) -> Value;
pub fn throw_type(msg: &str) -> Value;        // TypeError
pub fn throw_range(msg: &str) -> Value;       // RangeError
pub fn throw_reference(msg: &str) -> Value;   // ReferenceError
pub fn throw_syntax(msg: &str) -> Value;      // SyntaxError
pub fn throw_uri(msg: &str) -> Value;         // URIError
pub fn throw_eval(msg: &str) -> Value;        // EvalError

pub fn make_error_object(class: &str, message: &str) -> ObjRef;
pub fn set_stack(obj: &ObjRef, stack: &str);
```

#### 5.9.2 显示异常

```rust
/// 把任意 Value（尤其是 Error 对象）转成人读字符串
/// 例：Error: oops / TypeError: Cannot read properties of undefined
pub fn display_value(v: &Value) -> String;
```

#### 5.9.3 解析错误（Rust 侧）

```rust
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: u32,
    pub col: u32,
}
impl std::fmt::Display for ParseError;   // "SyntaxError: <msg> (line:col)"
impl std::error::Error for ParseError;
```

> 注意区分：`ParseError` 是 Rust 侧的 `Result<_, ParseError>`；JS 运行时抛出的 `SyntaxError` 是 `Value`。`Interpreter::run` 会把前者转成后者。

---

### 5.10 `parser` —— 词法/语法分析

> 定义于 `src/parser.rs` + `src/lexer.rs` + `src/ast.rs`。

```rust
/// 以 Script 模式解析
pub fn parse(src: &str) -> Result<Program, ParseError>;

/// 以 Module 模式解析（允许顶层 import/export/await）
pub fn parse_module(src: &str) -> Result<Program, ParseError>;

pub struct Parser<'a> {
    pub fn new(src: &'a str) -> Self;            // Script 模式
    pub fn with_mode(src: &'a str, module: bool) -> Self;
}
```

#### `Program` / `Stmt` / `Expr`（部分）

```rust
pub struct Program {
    pub body: Vec<Stmt>,
    pub strict: bool,
    pub source_type: SourceType,    // Script | Module
}

pub enum Stmt {
    Empty, Block(Block), Expr(Expr), Var(VarDecl),
    Function(FunctionDecl), Class(ClassDecl),
    Return(Option<Expr>),
    If { test, cons, alt },
    While { test, body }, DoWhile { test, body },
    For { init, test, update, body },
    ForIn { left, right, body },
    ForOf { left, right, body, await_tok },
    Switch { disc, cases: Vec<SwitchCase> },
    Break(Option<Rc<str>>), Continue(Option<Rc<str>>),
    Throw(Expr),
    Try { block, handler, finalizer },
    Labeled { label, body },
    Debugger, With { object, body },
    Import(ImportDecl), ExportNamed(ExportNamed),
    ExportDefault(ExportDefault), ExportAll(ExportAll),
}

pub enum VarKind { Var, Let, Const }
pub enum Pattern { Ident(Rc<str>), Array(...), Object(...), Rest(...), Assign(...) }
```

完整 AST 定义见 `src/ast.rs`（490 行）。集成时一般用 `interp.run(src)` 一步到位；只有需要**缓存解析结果多次执行**、**自定义预处理**或**做静态分析**时才直接用 `parser::parse`。

---

### 5.11 `builtins` —— 内置对象安装器与工具函数

> 定义于 `src/builtins/mod.rs`。这是写原生扩展时**最常用**的模块。

#### 5.11.1 总安装器

```rust
/// 把所有内置对象（Object/Array/.../console/setTimeout/require/...）挂到 realm 上
pub fn install(interp: &mut Interpreter);
```

#### 5.11.2 共享 helper（写扩展必用）

```rust
/// 原生构造器签名
pub type CtorFn = Rc<dyn Fn(&mut Interpreter, Value, &[Value], Value) -> Result<Value, Value>>;

/// 创建一个原生构造器（同时是函数对象）
pub fn make_ctor(
    realm: &Rc<Realm>,
    name: &str,
    len: usize,
    call_fn: NativeFn,
    ctor_fn: CtorFn,
) -> Value;

/// 把任意值挂到全局对象上（同时建到 globalEnv）
pub fn install_global(interp: &mut Interpreter, realm: &Rc<Realm>, name: &str, v: Value);

/// 把构造器挂到全局，并双向绑定 ctor.prototype ↔ proto.constructor
pub fn install_global_ctor(
    interp: &mut Interpreter,
    realm: &Rc<Realm>,
    name: &str,
    ctor: Value,
    proto: ObjRef,
);

/// 在某个对象上定义方法（non-enumerable, configurable）
pub fn def_method(realm: &Rc<Realm>, obj: &ObjRef, name: &str, len: usize, f: NativeFn);

/// 在某个对象上定义只读常量
pub fn def_const(obj: &ObjRef, name: &str, v: Value);
pub fn def_const_value(obj: &ObjRef, name: &str, v: Value);

/// 取/存 intrinsic 到 realm.global.__intrinsics__（hack：绕过 Realm 字段非 RefCell）
pub fn realm_get(realm: &Rc<Realm>, field: &str) -> Value;
// realm_set 是私有的，仅 builtins 内部用
```

#### 5.11.3 `Interpreter` 上的扩展方法（定义在 builtins/mod.rs）

```rust
impl Interpreter {
    pub fn iterable_to_vec(&mut self, v: &Value) -> Result<Vec<Value>, Value>;
    pub fn coerce_to_string(&mut self, v: &Value) -> Result<Rc<str>, Value>;
    pub fn coerce_to_number(&mut self, v: &Value) -> Result<f64, Value>;
}

/// 漂亮打印一个 Value（console.log 用）
pub fn pretty_print(v: &Value, interp: &Interpreter, depth: usize) -> String;
```

#### 5.11.4 内置 Node 模块

`builtins::node_modules::try_load_builtin(interp, specifier)` 返回 `Option<Value>`，支持：

| specifier | 内容 |
|---|---|
| `"fs"` | readFileSync / writeFileSync / existsSync / mkdirSync / readdirSync / statSync / unlinkSync / ... |
| `"path"` | join / resolve / basename / dirname / extname / normalize / parse / ... |
| `"os"` | platform / hostname / cpus / totalmem / freemem / ... |
| `"buffer"` / `"Buffer"` | Buffer 构造器 + from / concat / isBuffer / ... |
| `"util"` | inspect / format / inherits / ... |
| `"crypto"` | createHash (md5/sha1/sha256) / randomBytes / ... |
| `"events"` | EventEmitter |
| `"url"` | URL / parse / format |
| `"querystring"` | parse / stringify / escape / unescape |

---

## 6. 典型集成场景（含完整可运行代码）

### 场景 1：执行 JS 文件并打印结果

```rust
use quickrs;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let local = tokio::task::LocalSet::new();
    local.run_until(async {
        let src = std::fs::read_to_string("app.js").unwrap();
        let mut interp = quickrs::new_interpreter();
        match interp.run(&src) {
            Ok(_) => {}
            Err(e) => eprintln!("Uncaught {}", quickrs::error::display_value(&e)),
        }
        let code = quickrs::asyncrt::run_event_loop(&mut interp).await;
        std::process::exit(code);
    }).await;
}
```

### 场景 2：从 Rust 调用 JS 函数

```rust
use quickrs::value::*;

let mut interp = quickrs::new_interpreter();
interp.run(r#"
    function add(a, b) { return a + b; }
"#).unwrap();

// 拿到 add 函数
let add = interp.get_global("add");
// 调用
let result = interp.call_value(
    add,
    Value::Undefined,                          // this
    &[Value::from_int(3), Value::from_int(4)], // args
).unwrap();
assert_eq!(quickrs::value::to_string(&result), "7");
```

### 场景 3：把 Rust 函数暴露给 JS

```rust
use quickrs::value::*;
use quickrs::builtins::install_global;
use std::rc::Rc;

let mut interp = quickrs::new_interpreter();
let realm = interp.realm().clone();

// 暴露一个 rust_add(a, b) 给 JS
let f = interp.make_native("rust_add", 2, Rc::new(|_i, _t, args| {
    let a = quickrs::value::to_number(args.get(0).unwrap_or(&Value::Undefined));
    let b = quickrs::value::to_number(args.get(1).unwrap_or(&Value::Undefined));
    Ok(Value::from_f64(a + b))
}));
install_global(&mut interp, &realm, "rust_add", f);

let v = interp.run("rust_add(10, 20)").unwrap();
assert_eq!(quickrs::value::to_string(&v), "30");
```

### 场景 4：把 Rust 对象（带方法）注入 JS

```rust
use quickrs::value::*;
use quickrs::builtins::{def_method, install_global};
use std::rc::Rc;

let mut interp = quickrs::new_interpreter();
let realm = interp.realm().clone();

let db = ObjectInner::new_object();
db.borrow_mut().proto = Some(Value::Object(realm.object_proto.clone()));
db.borrow_mut().class = "Database";

// db.query(sql)
def_method(&realm, &db, "query", 1, Rc::new(|_i, _t, args| {
    let sql = quickrs::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
    // ...这里调真实 Rust DB 客户端...
    let rows = vec![
        interp.new_array(vec![Value::from_int(1), Value::from_str("Alice")]),
        interp.new_array(vec![Value::from_int(2), Value::from_str("Bob")]),
    ];
    Ok(interp.new_array(rows))
}));

install_global(&mut interp, &realm, "db", Value::Object(db));

interp.run(r#"
    const rows = db.query("SELECT id, name FROM users");
    for (const r of rows) console.log(r[0], r[1]);
"#).unwrap();
```

### 场景 5：自定义原生构造器 + 类

```rust
use quickrs::value::*;
use quickrs::builtins::{make_ctor, install_global_ctor, def_method};
use quickrs::interp::NativeFn;
use quickrs::builtins::CtorFn;
use std::rc::Rc;

let mut interp = quickrs::new_interpreter();
let realm = interp.realm().clone();

// 构造器原型
let proto = ObjectInner::new_object();
proto.borrow_mut().proto = Some(Value::Object(realm.object_proto.clone()));
proto.borrow_mut().class = "Vec2";

// 原型方法：add
def_method(&realm, &proto, "add", 1, Rc::new(|_i, this, args| {
    let other = args.get(0).cloned().unwrap_or(Value::Undefined);
    let (ax, ay) = read_vec2(&this);
    let (bx, by) = read_vec2(&other);
    Ok(make_vec2(_i, ax + bx, ay + by))
}));
def_method(&realm, &proto, "toString", 0, Rc::new(|_i, this, _a| {
    let (x, y) = read_vec2(&this);
    Ok(Value::from_string(format!("Vec2({}, {})", x, y)))
}));

// 构造器
let call_fn: NativeFn = Rc::new(|_i, _t, args| {
    let x = quickrs::value::to_number(args.get(0).unwrap_or(&Value::from_int(0)));
    let y = quickrs::value::to_number(args.get(1).unwrap_or(&Value::from_int(0)));
    Ok(make_vec2(_i, x, y))
});
let ctor_fn: CtorFn = Rc::new(|_i, _this, args, _nt| {
    let x = quickrs::value::to_number(args.get(0).unwrap_or(&Value::from_int(0)));
    let y = quickrs::value::to_number(args.get(1).unwrap_or(&Value::from_int(0)));
    Ok(make_vec2(_i, x, y))
});
let ctor = make_ctor(&realm, "Vec2", 2, call_fn, ctor_fn);
install_global_ctor(&mut interp, &realm, "Vec2", ctor, proto);

interp.run(r#"
    const a = new Vec2(1, 2);
    const b = new Vec2(3, 4);
    console.log(a.add(b).toString());  // Vec2(4, 6)
"#).unwrap();

// —— helpers ——
fn read_vec2(v: &Value) -> (f64, f64) {
    if let Value::Object(o) = v {
        let b = o.borrow();
        let x = b.props.get(&PropKey::from_str("_x")).map(|p| match &p.kind { PropKind::Data(Value::Number(n)) => *n, _ => 0.0 }).unwrap_or(0.0);
        let y = b.props.get(&PropKey::from_str("_y")).map(|p| match &p.kind { PropKind::Data(Value::Number(n)) => *n, _ => 0.0 }).unwrap_or(0.0);
        (x, y)
    } else { (0.0, 0.0) }
}
fn make_vec2(interp: &quickrs::Interpreter, x: f64, y: f64) -> Value {
    let o = ObjectInner::new_object();
    o.borrow_mut().proto = Some(Value::Object(interp.realm().object_proto.clone()));
    o.borrow_mut().class = "Vec2";
    o.borrow_mut().props.insert(PropKey::from_str("_x"), Property::data(Value::from_f64(x)));
    o.borrow_mut().props.insert(PropKey::from_str("_y"), Property::data(Value::from_f64(y)));
    Value::Object(o)
}
```

### 场景 6：异步任务（Rust 侧触发 JS 的 Promise resolve）

```rust
use quickrs::{asyncrt, value::*, builtins::install_global};
use std::rc::Rc;
use std::time::Duration;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let local = tokio::task::LocalSet::new();
    local.run_until(async {
        let mut interp = quickrs::new_interpreter();
        let realm = interp.realm().clone();

        // 暴露 fetchRust(url) -> Promise<string>
        let f = interp.make_native("fetchRust", 1, Rc::new(|interp, _t, args| {
            let url = quickrs::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
            let p = interp.new_promise();
            let p_clone = p.clone();
            let rt = interp.shared.async_rt.clone();
            // 50ms 后 resolve
            asyncrt::set_timeout(&rt, 50, Box::new(move |interp| {
                interp.resolve_promise(p_clone.clone(),
                    Value::from_string(format!("response from {}", url)));
            }));
            Ok(p)
        }));
        install_global(&mut interp, &realm, "fetchRust", f);

        interp.run(r#"
            fetchRust("https://example.com").then(r => console.log("got:", r));
        "#).unwrap();

        asyncrt::run_event_loop(&mut interp).await;
    }).await;
}
```

### 场景 7：在 Rust 里 await 一个 JS Promise

`quickrs` 没有直接提供 `await_promise` API，但可以用 `promise_state` 轮询 + 事件循环驱动：

```rust
use quickrs::{asyncrt, value::*};

async fn await_promise(interp: &mut quickrs::Interpreter, p: Value) -> Result<Value, Value> {
    loop {
        // 排一次事件循环（处理 microtask + 到期 timer）
        asyncrt::run_event_loop(interp).await;
        if let Some(state) = interp.promise_state(&p) {
            let s = state.borrow();
            return match &s.state {
                quickrs::value::PromiseStatus::Fulfilled => Ok(s.value.clone()),
                quickrs::value::PromiseStatus::Rejected => Err(s.value.clone()),
                quickrs::value::PromiseStatus::Pending => {
                    drop(s);
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    continue;
                }
            };
        }
        return Err(quickrs::error::throw_type("not a promise"));
    }
}
```

> 更高效的做法是直接注册一个 `then` 回调，让它在 resolve 时通过 `tokio::sync::oneshot` 通知 Rust future。这需要把 `oneshot::Sender` 放进原生函数闭包里。

### 场景 8：错误处理

```rust
let mut interp = quickrs::new_interpreter();
match interp.run("throw new Error('boom')") {
    Ok(v) => println!("ok: {}", quickrs::value::to_string(&v)),
    Err(e) => {
        // e 是 Value，通常是 Error 对象
        let msg = quickrs::error::display_value(&e);
        eprintln!("JS threw: {}", msg);   // → "JS threw: Error: boom"

        // 也可以直接读属性
        let name = interp.get_property(&e, &PropKey::from_str("name"));
        let stack = interp.get_property(&e, &PropKey::from_str("stack"));
        println!("name={}, stack={}", quickrs::value::to_string(&name), quickrs::value::to_string(&stack));
    }
}
```

### 场景 9：加载 ES Module

```rust
let mut interp = quickrs::new_interpreter();
// 假设 ./math.js 里 export function add(a,b){return a+b}
let ns = interp.load_module("./math.js").unwrap();
let add = interp.get_property(&ns, &PropKey::from_str("add")).unwrap();
let r = interp.call_value(add, Value::Undefined, &[Value::from_int(1), Value::from_int(2)]).unwrap();
assert_eq!(quickrs::value::to_string(&r), "3");
```

> `load_module` 内部会调 `std::fs::read_to_string`，路径是**相对当前工作目录**。要换 base dir，先 `std::env::set_current_dir(...)`。

### 场景 10：在多线程宿主程序里隔离使用

`quickrs` 是 `!Send`，多线程宿主（如 Web 服务器）应**每个 worker 线程一个独立 Interpreter**：

```rust
use std::thread;

fn handle_request(src: String) -> String {
    // 每个 thread 起一个独立 runtime + LocalSet
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build().unwrap();
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async move {
        let mut interp = quickrs::new_interpreter();
        let v = interp.run(&src).unwrap_or_else(|e| {
            Value::from_string(quickrs::error::display_value(&e))
        });
        quickrs::asyncrt::run_event_loop(&mut interp).await;
        quickrs::value::to_string(&v)
    })
}

// 主线程派活
let h = thread::spawn(move || handle_request("1+1".into()));
println!("{}", h.join().unwrap());  // → 2
```

> ⚠️ 不要用 `tokio::spawn`（默认 multi-thread runtime）跑 JS，要用 `thread::spawn` + 各自的 current-thread runtime。

---

## 7. 完整示例工程

下面是一个可直接 `cargo run` 的最小工程，综合演示：执行 JS、注册 Rust 函数、调用 JS 函数、跑事件循环。

#### `Cargo.toml`

```toml
[package]
name = "quickrs-demo"
version = "0.1.0"
edition = "2021"

[dependencies]
quickrs = { path = "../quickrs" }
tokio = { version = "1", features = ["full"] }
```

#### `src/main.rs`

```rust
use quickrs::value::*;
use quickrs::builtins::{install_global, def_method};
use std::rc::Rc;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let local = tokio::task::LocalSet::new();
    local.run_until(async {
        let mut interp = quickrs::new_interpreter();
        let realm = interp.realm().clone();

        // 1) 暴露 Rust 函数
        let greet = interp.make_native("greet", 1, Rc::new(|_i, _t, args| {
            let name = quickrs::value::to_string(args.get(0).unwrap_or(&Value::Undefined));
            Ok(Value::from_string(format!("Hello, {}!", name)))
        }));
        install_global(&mut interp, &realm, "greet", greet);

        // 2) 暴露 Rust 对象
        let logger = quickrs::value::ObjectInner::new_object();
        logger.borrow_mut().proto = Some(Value::Object(realm.object_proto.clone()));
        def_method(&realm, &logger, "info", 0, Rc::new(|_i, _t, args| {
            let parts: Vec<String> = args.iter().map(quickrs::value::to_string).collect();
            println!("[INFO] {}", parts.join(" "));
            Ok(Value::Undefined)
        }));
        install_global(&mut interp, &realm, "logger", Value::Object(logger));

        // 3) 跑 JS：用 Rust 函数 + 异步
        interp.run(r#"
            logger.info(greet("world"));
            async function task() {
                await new Promise(r => setTimeout(r, 100));
                logger.info("after 100ms");
                return 42;
            }
            task().then(v => logger.info("done:", v));
        "#).unwrap();

        // 4) 从 Rust 调 JS 定义的函数
        let task = interp.get_global("task");
        let p = interp.call_value(task, Value::Undefined, &[]).unwrap();
        logger_info(&interp, &p).await;

        // 5) 跑事件循环
        let code = quickrs::asyncrt::run_event_loop(&mut interp).await;
        if code != 0 { std::process::exit(code); }
    }).await;
}

async fn logger_info(_interp: &quickrs::Interpreter, _v: &Value) {
    // 略：参考场景 7 把 promise 转成 Rust future
}
```

#### 预期输出

```
[INFO] Hello, world!
[INFO] after 100ms
[INFO] done: 42
```

---

## 8. 注意事项与已知坑

### 8.1 必须 `current_thread` + `LocalSet`

```rust
// ✅ 正确
#[tokio::main(flavor = "current_thread")]
async fn main() {
    tokio::task::LocalSet::new().run_until(async { /* 用 interp */ }).await;
}

// ❌ 错误：multi-thread runtime，interp 是 !Send，编译报错
#[tokio::main]
async fn main() { /* ... */ }
```

### 8.2 `borrow_mut` 冲突

`ObjectInner` 是 `RefCell`，**在持有 `borrow_mut()` 时调解释器方法可能 panic**：

```rust
// ❌ 危险：borrow 还没释放就调 set_property（内部又会 borrow）
let mut b = obj.borrow_mut();
b.props.insert(...);
interp.set_property(&Value::Object(obj.clone()), &key, val).ok();

// ✅ 安全：先改完，drop borrow，再调解释器
{
    let mut b = obj.borrow_mut();
    b.props.insert(...);
}  // borrow 释放
interp.set_property(&Value::Object(obj.clone()), &key, val).ok();
```

### 8.3 异步不跑事件循环 = 静默不执行

```rust
interp.run("setTimeout(()=>console.log('hi'), 100)").unwrap();
// ❌ 这里直接退出，"hi" 永远不会打印
std::process::exit(0);

// ✅ 必须驱动
quickrs::asyncrt::run_event_loop(&mut interp).await;
```

### 8.4 `Realm` 字段不可变

`Realm` 的 `object_ctor`、`array_ctor` 等字段不是 `RefCell`，构造后无法直接改。`builtins/mod.rs` 用 `realm.global.__intrinsics__` 这个 side table 绕过（见 `realm_get`/`realm_set`）。如果你要扩展 intrinsic，沿用这个 hack。

### 8.5 递归深度

默认 `MAX_DEPTH = 1200`。深度递归 JS（如手写 fibonacci(40) 不带 memo）会触发 `RangeError`。可在构造后调：

```rust
let mut interp = quickrs::new_interpreter();
interp.shared.max_depth = 5000;   // Rc<Shared> 内部是 usize，可直接改
```

### 8.6 `process.exit()` 与事件循环

JS 里调 `process.exit(n)` 会设 `async_rt.stop = true; exit_code = n`，`run_event_loop` 下一轮检查到就返回 `n`。你的宿主程序应该尊重这个码：

```rust
let code = quickrs::asyncrt::run_event_loop(&mut interp).await;
if code != 0 { std::process::exit(code); }
```

### 8.7 模块路径

- ESM `import './foo.js'` 与 `load_module("./foo.js")`：相对**当前工作目录**。
- CommonJS `require('./foo')`：会自动尝试加 `.js` 后缀；`__dirname` / `__filename` 已注入。
- 裸说明符 `require('fs')`：命中内置模块表，不会读磁盘。

### 8.8 没有网络

`fetch` 在 `quickrs` 里是 **stub**，调用会 reject 一个 `TypeError: fetch is not supported`。要联网就在 Rust 侧实现一个真 `fetch` 注入进去（参考场景 6 + `reqwest`）。

### 8.9 `Value` 的 `Clone` 是廉价的

所有变体要么是 `Copy`，要么是 `Rc`，所以 `value.clone()` 只是增加引用计数，放心 clone。

### 8.10 不要在原生函数里 `panic`

原生函数 `NativeFn` 的返回类型是 `Result<Value, Value>`，`Err` 是 JS 异常。如果你 Rust 侧 `panic`，会直接 unwind 出去，**不会**被 JS 的 `try/catch` 捕获。把可恢复错误转成 `Err(error::throw_type(...))`，不可恢复的才 `panic`。

---

## 附录 A：完整公开 API 速查表

```
quickrs
├── new_interpreter() -> Interpreter
├── Interpreter           (interp.rs)
│   ├── new(realm) / realm() / run(src) / eval_program / eval_module
│   ├── get_global(name)
│   ├── get_property / set_property / has_property / delete_property
│   ├── own_property_keys / get_prototype_of / as_proxy
│   ├── call_value / construct / construct_with_this
│   ├── to_object / to_primitive / to_promise
│   ├── new_array / new_promise / promise_state / resolve_promise / reject_promise
│   ├── make_native / make_function
│   ├── is_iterable / get_iterator / iterator_step / iterable_to_vec
│   ├── coerce_to_string / coerce_to_number
│   ├── load_module(specifier)
│   ├── exec_stmt / eval_expr / binary_op / bind_pattern / flatten_into
│   └── shared: Rc<Shared { realm, async_rt, yielder, depth, max_depth, stack }>
├── Value                 (value.rs)
│   ├── Undefined / Null / Bool / Number / String / Symbol / Object / BigInt
│   ├── from_bool / from_f64 / from_int / from_str / from_string / object / undefined / null
│   ├── is_undefined / is_null / is_nullish / is_object / is_callable / is_constructor / as_object / type_of
│   └── (全局) to_string / to_boolean / to_number / to_integer / to_int32 / to_uint32 / to_length
│            string_to_number / format_number / bigint_to_string / date_format
│            strict_equals / loose_equals / same_value / same_value_zero
│            to_property_key / index_to_key / key_to_index
├── ObjectInner           (value.rs)
│   ├── new_object / new_array / new_function
│   └── { props, proto, extensible, kind, class }
├── ObjectKind            (value.rs)
│   ├── Ordinary / Array(Vec) / Function(Rc) / BoundFunction / Error
│   ├── String / Number / Boolean / Symbol / BigInt-data
│   ├── Map / Set / Date / RegExp / Promise / Generator
│   ├── ArrayBuffer / TypedArray / Module / Proxy
├── Property / PropKind / PropKey / Symbol / BigInt
├── Function / FunctionBody / ClassField
├── NativeFn = Rc<dyn Fn(&mut Interpreter, Value, &[Value]) -> Result<Value, Value>>
├── Realm                 (realm.rs)
│   ├── new() -> Rc<Realm>
│   ├── new_symbol(desc) / proto_for(class)
│   └── global / global_env / 各原型 / 各构造器 / wk / modules / module_cache
├── scope::Env            (scope.rs)
│   ├── new(parent, kind) / global()
│   ├── create / create_uninit / has_own / resolve / get / set
│   └── this / new_target / home_object / parent_constructor
├── asyncrt               (asyncrt.rs)
│   ├── AsyncRt::new() / queue_microtask / set_timeout / clear_timeout
│   └── run_event_loop(&mut interp) -> i32
├── error                 (error.rs)
│   ├── throw_error / throw_type / throw_range / throw_reference / throw_syntax / throw_uri / throw_eval
│   ├── make_error_object / set_stack / display_value
│   └── ParseError { message, line, col }
├── parser                (parser.rs)
│   ├── parse(src) -> Result<Program, ParseError>
│   ├── parse_module(src) -> Result<Program, ParseError>
│   └── Parser::new / with_mode
├── ast                   (ast.rs)
│   └── Program / Stmt / Expr / Pattern / VarDecl / FunctionDecl / ClassDecl / ...
└── builtins              (builtins/mod.rs + 子模块)
    ├── install(&mut interp)
    ├── NativeFn / CtorFn
    ├── make_ctor / install_global / install_global_ctor
    ├── def_method / def_const / def_const_value
    ├── realm_get
    ├── pretty_print
    └── node_modules::try_load_builtin(interp, spec) -> Option<Value>
```

---

## 附录 B：参考实现位置

| 想做的事 | 看源码哪里 |
|---|---|
| 写一个原生全局函数 | `src/builtins/globals.rs`（`parseInt` / `setTimeout`） |
| 写一个原生构造器 + 原型方法 | `src/builtins/mod.rs::install_boolean` |
| 在原生函数里调 JS 函数 | `src/builtins/promise.rs`（`Promise.prototype.then`） |
| 在原生函数里返回 Promise + 异步 resolve | `src/builtins/globals.rs::setTimeout` |
| 实现 `require()` / 模块加载 | `src/builtins/globals.rs::require` + `src/interp.rs::load_module` |
| 处理迭代器 | `src/interp.rs::iterable_to_vec` / `get_iterator` |
| 错误对象构造 | `src/error.rs` + `src/builtins/errors.rs` |
| 多 Realm 隔离 | `src/realm.rs::Realm::new` + `builtins::install` |
| 事件循环细节 | `src/asyncrt.rs::run_event_loop` |

---

**文档完。** 如需进一步了解某个内置模块（如 `Map/Set`、`RegExp`、`TypedArray`）的实现细节，直接读 `src/builtins/<name>.rs`，每个文件顶部都有模块注释说明覆盖范围。
