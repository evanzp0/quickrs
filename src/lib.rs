//! quickrs — a QuickJS-inspired JavaScript engine in Rust with Tokio async.
//!
//! Public entry points: [`interp::Interpreter`] (engine) and the [`run`]
//! binary in `src/main.rs`.

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

/// Create a fully-initialized interpreter (realm + builtins installed).
pub fn new_interpreter() -> Interpreter {
    let realm = Realm::new();
    let mut interp = Interpreter::new(realm);
    builtins::install(&mut interp);
    interp
}
