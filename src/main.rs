//! quickrs CLI: run a file or start a REPL. The Tokio current-thread runtime
//! drives the microtask/timer event loop so async/await and setTimeout work.

use clap::{Parser as ClapParser, Subcommand};
use quickrs::Interpreter;
use std::path::PathBuf;

#[derive(ClapParser)]
#[command(name = "quickrs", version, about = "A QuickJS-inspired JS engine in Rust + Tokio")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Evaluate the given expression and print the result.
    #[arg(short, long)]
    eval: Option<String>,
    /// File to execute (positional fallback).
    file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Run a JavaScript file.
    Run { file: PathBuf },
    /// Start an interactive REPL.
    Repl,
    /// Print the version.
    Version,
}

fn main() {
    let cli = Cli::parse();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async move {
        match cli.command {
            Some(Command::Run { file }) => {
                run_file(&file).await;
            }
            Some(Command::Repl) => {
                run_repl().await;
            }
            Some(Command::Version) => {
                println!("quickrs 0.1.0 (rust + tokio)");
            }
            None => {
                if let Some(expr) = &cli.eval {
                    let code = quickrs::new_interpreter();
                    run_interp_eval(code, expr).await;
                } else if let Some(file) = &cli.file {
                    run_file(file).await;
                } else {
                    run_repl().await;
                }
            }
        }
    });
}

async fn run_file(path: &std::path::Path) {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Cannot read {}: {}", path.display(), e);
            std::process::exit(1);
        }
    };
    let mut interp = quickrs::new_interpreter();
    match interp.run(&src) {
        Ok(_) => {}
        Err(e) => {
            eprintln!("Uncaught {}", quickrs::error::display_value(&e));
            std::process::exit(1);
        }
    }
    let code = quickrs::asyncrt::run_event_loop(&mut interp).await;
    if code != 0 {
        std::process::exit(code);
    }
}

async fn run_interp_eval(mut interp: Interpreter, expr: &str) {
    match interp.run(expr) {
        Ok(v) => println!("{}", quickrs::value::to_string(&v)),
        Err(e) => eprintln!("Uncaught {}", quickrs::error::display_value(&e)),
    }
    quickrs::asyncrt::run_event_loop(&mut interp).await;
}

async fn run_repl() {
    use rustyline::error::ReadlineError;
    use rustyline::DefaultEditor;
    println!("quickrs 0.1.0 — a QuickJS-inspired JS engine in Rust + Tokio");
    println!("Type .help for help, .exit to quit.\n");
    let mut rl = DefaultEditor::new().expect("failed to init readline");
    let mut interp = quickrs::new_interpreter();
    let mut buf = String::new();
    loop {
        let prompt = if buf.is_empty() { "quickrs> " } else { "......> " };
        let line = match rl.readline(prompt) {
            Ok(l) => l,
            Err(ReadlineError::Interrupted) => { buf.clear(); continue; }
            Err(ReadlineError::Eof) => break,
            Err(e) => { eprintln!("error: {}", e); break; }
        };
        let trimmed = line.trim();
        if buf.is_empty() {
            if trimmed == ".exit" || trimmed == ".quit" { break; }
            if trimmed == ".help" {
                println!("Commands: .exit, .help, .reset, .load <file>");
                continue;
            }
            if trimmed == ".reset" {
                interp = quickrs::new_interpreter();
                println!("(realm reset)");
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix(".load ") {
                if let Ok(s) = std::fs::read_to_string(rest.trim()) {
                    match interp.run(&s) { Ok(_) => {}, Err(e) => eprintln!("Uncaught {}", quickrs::error::display_value(&e)) }
                    quickrs::asyncrt::run_event_loop(&mut interp).await;
                }
                continue;
            }
        }
        let _ = rl.add_history_entry(&line);
        buf.push_str(&line);
        buf.push('\n');
        // try to parse/eval; if incomplete, keep accumulating
        match interp.run(&buf) {
            Ok(v) => {
                if !v.is_undefined() {
                    println!("{}", quickrs::value::to_string(&v));
                }
                buf.clear();
            }
            Err(e) => {
                // Heuristic: if it's a syntax error, maybe incomplete; else print
                let s = quickrs::error::display_value(&e);
                if s.starts_with("SyntaxError") && is_likely_incomplete(&buf) {
                    // wait for more input
                } else {
                    eprintln!("Uncaught {}", s);
                    buf.clear();
                }
            }
        }
        quickrs::asyncrt::run_event_loop(&mut interp).await;
    }
}

fn is_likely_incomplete(_s: &str) -> bool {
    // Very rough heuristic: keep collecting if unmatched braces.
    false
}
