//! Recursive-descent parser producing an AST.
//!
//! Implements a broad ES2020+ grammar: all statements, full expression
//! precedence, arrow functions, classes, destructuring, generators,
//! async/await, template literals, regex literals, and ES module syntax.

use crate::ast::*;
use crate::error::ParseError;
use crate::lexer::{Keyword, Lexer, Punct, Token, TokenWithPos};
use std::rc::Rc;

pub struct Parser<'a> {
    lex: Lexer<'a>,
    cur: TokenWithPos,
    next: TokenWithPos,
    /// Whether we are in a context where `yield` is a keyword (generator body).
    in_generator: bool,
    /// Whether we are in an async context (`await` is a keyword).
    in_async: bool,
    /// Track whether we're parsing a parameter list (for arrow disambiguation).
    allow_in: bool,
    /// Module mode (top-level await allowed, import/export allowed).
    is_module: bool,
}

impl<'a> Parser<'a> {
    pub fn new(src: &'a str) -> Self {
        Self::with_mode(src, false)
    }

    pub fn with_mode(src: &'a str, module: bool) -> Self {
        let mut lex = Lexer::new(src);
        let cur = lex.next(&Token::Punct(Punct::Semicolon));
        let next = lex.next(&cur.token);
        Parser {
            lex,
            cur,
            next,
            in_generator: false,
            in_async: false,
            allow_in: true,
            is_module: module,
        }
    }

    fn bump(&mut self) -> TokenWithPos {
        let t = std::mem::replace(&mut self.cur, self.next.clone());
        self.next = self.lex.next(&self.cur.token);
        t
    }

    fn expect_punct(&mut self, p: Punct) -> Result<(), ParseError> {
        if matches!(self.cur.token, Token::Punct(ref q) if *q == p) {
            self.bump();
            Ok(())
        } else {
            Err(self.err(&format!("expected `{}`", punct_str(p))))
        }
    }

    fn expect_kw(&mut self, k: Keyword) -> Result<(), ParseError> {
        if matches!(self.cur.token, Token::Keyword(ref q) if *q == k) {
            self.bump();
            Ok(())
        } else {
            Err(self.err(&format!("expected `{}`", kw_str(k))))
        }
    }

    fn eat_punct(&mut self, p: Punct) -> bool {
        if matches!(self.cur.token, Token::Punct(ref q) if *q == p) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, k: Keyword) -> bool {
        if matches!(self.cur.token, Token::Keyword(ref q) if *q == k) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn is_punct(&self, p: Punct) -> bool {
        matches!(self.cur.token, Token::Punct(ref q) if *q == p)
    }
    fn is_kw(&self, k: Keyword) -> bool {
        matches!(self.cur.token, Token::Keyword(ref q) if *q == k)
    }

    fn err(&self, msg: &str) -> ParseError {
        ParseError {
            message: msg.to_string(),
            line: self.cur.line,
            col: self.cur.col,
        }
    }

    /// Consume a semicolon or apply ASI.
    fn consume_semicolon(&mut self) -> Result<(), ParseError> {
        if self.is_punct(Punct::Semicolon) {
            self.bump();
            return Ok(());
        }
        if self.is_punct(Punct::RBrace)
            || matches!(self.cur.token, Token::Eof)
            || self.cur.preceded_by_newline
        {
            return Ok(());
        }
        Err(self.err("expected `;` or newline"))
    }

    // -------------------------------------------------------------------
    // Top-level
    // -------------------------------------------------------------------

    pub fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut body = Vec::new();
        let mut strict = false;
        // Detect "use strict" directive prologue.
        if let Token::String(s) = &self.cur.token {
            if &**s == "use strict" {
                strict = true;
            }
        }
        while !matches!(self.cur.token, Token::Eof) {
            body.push(self.parse_stmt()?);
        }
        Ok(Program {
            body,
            strict,
            source_type: if self.is_module {
                SourceType::Module
            } else {
                SourceType::Script
            },
        })
    }

    // -------------------------------------------------------------------
    // Statements
    // -------------------------------------------------------------------

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match &self.cur.token {
            Token::Punct(Punct::LBrace) => Ok(Stmt::Block(self.parse_block()?)),
            Token::Punct(Punct::Semicolon) => {
                self.bump();
                Ok(Stmt::Empty)
            }
            Token::Keyword(k) => match k {
                Keyword::Var | Keyword::Let | Keyword::Const => {
                    let s = self.parse_var_stmt(*k)?;
                    self.consume_semicolon()?;
                    Ok(s)
                }
                Keyword::Function => self.parse_function_decl(false, false),
                Keyword::Class => {
                    let c = self.parse_class_decl()?;
                    Ok(Stmt::Class(c))
                }
                Keyword::If => self.parse_if(),
                Keyword::While => self.parse_while(),
                Keyword::Do => self.parse_do_while(),
                Keyword::For => self.parse_for(),
                Keyword::Return => self.parse_return(),
                Keyword::Break => {
                    self.bump();
                    let label = self.read_optional_label();
                    self.consume_semicolon()?;
                    Ok(Stmt::Break(label))
                }
                Keyword::Continue => {
                    self.bump();
                    let label = self.read_optional_label();
                    self.consume_semicolon()?;
                    Ok(Stmt::Continue(label))
                }
                Keyword::Switch => self.parse_switch(),
                Keyword::Throw => {
                    self.bump();
                    if self.cur.preceded_by_newline {
                        return Err(self.err("illegal newline after throw"));
                    }
                    let e = self.parse_expr()?;
                    self.consume_semicolon()?;
                    Ok(Stmt::Throw(e))
                }
                Keyword::Try => self.parse_try(),
                Keyword::Debugger => {
                    self.bump();
                    self.consume_semicolon()?;
                    Ok(Stmt::Debugger)
                }
                Keyword::With => self.parse_with(),
                Keyword::Import => {
                    // Distinguish static import from dynamic import(...)
                    if matches!(self.next.token, Token::Punct(Punct::LParen)) {
                        // Dynamic import() — parse as expression statement
                        let e = self.parse_expr()?;
                        self.consume_semicolon()?;
                        Ok(Stmt::Expr(e))
                    } else {
                        self.parse_import()
                    }
                }
                Keyword::Export => self.parse_export(),
                Keyword::Async
                    if matches!(
                        self.next.token,
                        Token::Keyword(Keyword::Function) | Token::Keyword(Keyword::Class)
                    ) =>
                {
                    self.bump();
                    if self.is_kw(Keyword::Function) {
                        self.parse_function_decl(true, false)
                    } else {
                        let c = self.parse_class_decl()?;
                        Ok(Stmt::Class(c))
                    }
                }
                _ => {
                    // labeled statement?
                    if let Token::Ident(name) = &self.cur.token {
                        if matches!(self.next.token, Token::Punct(Punct::Colon)) {
                            let label = name.clone();
                            self.bump();
                            self.bump();
                            let body = self.parse_stmt()?;
                            return Ok(Stmt::Labeled {
                                label,
                                body: Box::new(body),
                            });
                        }
                    }
                    let e = self.parse_expr()?;
                    self.consume_semicolon()?;
                    Ok(Stmt::Expr(e))
                }
            },
            Token::Ident(name) if matches!(self.next.token, Token::Punct(Punct::Colon)) => {
                let label = name.clone();
                self.bump();
                self.bump();
                let body = self.parse_stmt()?;
                Ok(Stmt::Labeled {
                    label,
                    body: Box::new(body),
                })
            }
            _ => {
                let e = self.parse_expr()?;
                self.consume_semicolon()?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    fn read_optional_label(&mut self) -> Option<Rc<str>> {
        if !self.cur.preceded_by_newline {
            if let Token::Ident(name) = &self.cur.token {
                let n = name.clone();
                self.bump();
                return Some(n);
            }
        }
        None
    }

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        self.expect_punct(Punct::LBrace)?;
        let mut stmts = Vec::new();
        while !self.is_punct(Punct::RBrace) && !matches!(self.cur.token, Token::Eof) {
            stmts.push(self.parse_stmt()?);
        }
        self.expect_punct(Punct::RBrace)?;
        Ok(Block { stmts })
    }

    fn parse_var_stmt(&mut self, k: Keyword) -> Result<Stmt, ParseError> {
        self.bump();
        let kind = match k {
            Keyword::Var => VarKind::Var,
            Keyword::Let => VarKind::Let,
            Keyword::Const => VarKind::Const,
            _ => unreachable!(),
        };
        let decls = self.parse_var_declarators(false)?;
        Ok(Stmt::Var(VarDecl { kind, decls }))
    }

    fn parse_var_declarators(&mut self, _in_for: bool) -> Result<Vec<VarDeclarator>, ParseError> {
        let mut decls = Vec::new();
        loop {
            let pattern = self.parse_binding_pattern()?;
            let init = if self.is_punct(Punct::Eq) {
                self.bump();
                Some(self.parse_assign()?)
            } else {
                None
            };
            decls.push(VarDeclarator { pattern, init });
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        Ok(decls)
    }

    fn parse_binding_pattern(&mut self) -> Result<Pattern, ParseError> {
        if self.is_punct(Punct::LBracket) {
            return self.parse_array_pattern();
        }
        if self.is_punct(Punct::LBrace) {
            return self.parse_object_pattern();
        }
        if let Token::Ident(name) = &self.cur.token {
            let n = name.clone();
            self.bump();
            return Ok(Pattern::Ident(n));
        }
        Err(self.err("expected binding identifier"))
    }

    fn parse_array_pattern(&mut self) -> Result<Pattern, ParseError> {
        self.expect_punct(Punct::LBracket)?;
        let mut elements = Vec::new();
        let mut rest = None;
        while !self.is_punct(Punct::RBracket) {
            if self.is_punct(Punct::Comma) {
                self.bump();
                elements.push(None);
                continue;
            }
            if self.is_punct(Punct::Spread) {
                self.bump();
                let inner = self.parse_binding_pattern()?;
                rest = Some(Box::new(inner));
                break;
            }
            let mut p = self.parse_binding_pattern()?;
            if self.is_punct(Punct::Eq) {
                self.bump();
                let default = self.parse_assign()?;
                p = Pattern::Assignment {
                    pattern: Box::new(p),
                    default,
                };
            }
            elements.push(Some(PatternElement { pattern: p }));
            if !self.is_punct(Punct::RBracket) {
                self.eat_punct(Punct::Comma);
            }
        }
        self.expect_punct(Punct::RBracket)?;
        Ok(Pattern::Array { elements, rest })
    }

    fn parse_object_pattern(&mut self) -> Result<Pattern, ParseError> {
        self.expect_punct(Punct::LBrace)?;
        let mut properties = Vec::new();
        let mut rest = None;
        while !self.is_punct(Punct::RBrace) {
            if self.is_punct(Punct::Spread) {
                self.bump();
                if let Token::Ident(name) = &self.cur.token {
                    let n = name.clone();
                    self.bump();
                    rest = Some(n);
                }
                break;
            }
            let computed = self.is_punct(Punct::LBracket);
            let key = self.parse_property_key()?;
            let (value, shorthand) = if self.eat_punct(Punct::Colon) {
                (self.parse_binding_pattern()?, false)
            } else {
                // shorthand
                if let PropertyKey::Ident(n) = &key {
                    (Pattern::Ident(n.clone()), true)
                } else {
                    (self.parse_binding_pattern()?, false)
                }
            };
            let value = if self.is_punct(Punct::Eq) {
                self.bump();
                let default = self.parse_assign()?;
                Pattern::Assignment {
                    pattern: Box::new(value),
                    default,
                }
            } else {
                value
            };
            properties.push(ObjectPatternProp {
                key,
                computed,
                value,
                shorthand,
            });
            if !self.is_punct(Punct::RBrace) {
                self.eat_punct(Punct::Comma);
            }
        }
        self.expect_punct(Punct::RBrace)?;
        Ok(Pattern::Object { properties, rest })
    }

    fn parse_property_key(&mut self) -> Result<PropertyKey, ParseError> {
        if self.is_punct(Punct::LBracket) {
            self.bump();
            let e = self.parse_assign()?;
            self.expect_punct(Punct::RBracket)?;
            return Ok(PropertyKey::Computed(e));
        }
        if self.is_kw(Keyword::Get) {
            self.bump();
            // could be a getter — but as a key, "get" identifier
            return Ok(PropertyKey::Ident(Rc::from("get")));
        }
        if self.is_kw(Keyword::Set) {
            self.bump();
            return Ok(PropertyKey::Ident(Rc::from("set")));
        }
        match &self.cur.token {
            Token::Ident(n) => {
                let n = n.clone();
                self.bump();
                Ok(PropertyKey::Ident(n))
            }
            Token::PrivateIdent(n) => {
                let n = n.clone();
                self.bump();
                Ok(PropertyKey::Private(n))
            }
            Token::String(s) => {
                let s = s.clone();
                self.bump();
                Ok(PropertyKey::String(s))
            }
            Token::Number(n) => {
                let n = *n;
                self.bump();
                Ok(PropertyKey::Number(n))
            }
            Token::Keyword(k) => {
                let name = Rc::from(kw_str(*k));
                self.bump();
                Ok(PropertyKey::Ident(name))
            }
            _ => Err(self.err("expected property key")),
        }
    }

    fn parse_function_decl(&mut self, is_async: bool, is_generator: bool) -> Result<Stmt, ParseError> {
        self.expect_kw(Keyword::Function)?;
        let is_generator = is_generator || self.eat_punct(Punct::Star);
        let name = self.parse_opt_ident()?;
        let func = self.parse_function_rest(name, is_async, is_generator, false)?;
        Ok(Stmt::Function(FunctionDecl {
            name: func.name.clone(),
            func,
            is_async,
            is_generator,
        }))
    }

    fn parse_opt_ident(&mut self) -> Result<Option<Rc<str>>, ParseError> {
        if let Token::Ident(n) = &self.cur.token {
            let n = n.clone();
            self.bump();
            Ok(Some(n))
        } else if let Token::Keyword(k) = &self.cur.token {
            // contextual / reserved-as-ident in some positions
            let n = Rc::from(kw_str(*k));
            self.bump();
            Ok(Some(n))
        } else {
            Ok(None)
        }
    }

    fn parse_function_rest(
        &mut self,
        name: Option<Rc<str>>,
        is_async: bool,
        is_generator: bool,
        is_arrow: bool,
    ) -> Result<FunctionExpr, ParseError> {
        let def_line = self.cur.line;
        let prev_gen = self.in_generator;
        let prev_async = self.in_async;
        self.in_generator = is_generator;
        self.in_async = is_async;
        self.expect_punct(Punct::LParen)?;
        let params = self.parse_params()?;
        self.expect_punct(Punct::LBrace)?;
        let mut stmts = Vec::new();
        let mut decls = Vec::new();
        while !self.is_punct(Punct::RBrace) && !matches!(self.cur.token, Token::Eof) {
            // hoist nested function declarations
            if self.is_kw(Keyword::Function)
                || (self.is_kw(Keyword::Async)
                    && matches!(self.next.token, Token::Keyword(Keyword::Function)))
            {
                let start_async = self.is_kw(Keyword::Async);
                if start_async {
                    self.bump();
                }
                let s = self.parse_function_decl(start_async, false)?;
                if let Stmt::Function(fd) = s {
                    decls.push(fd);
                }
            } else {
                stmts.push(self.parse_stmt()?);
            }
        }
        self.expect_punct(Punct::RBrace)?;
        self.in_generator = prev_gen;
        self.in_async = prev_async;
        Ok(FunctionExpr {
            name,
            params,
            body: Block { stmts },
            decls,
            is_async,
            is_generator,
            is_arrow,
            expr_body: false,
            line: def_line,
        })
    }

    fn parse_params(&mut self) -> Result<Vec<Pattern>, ParseError> {
        let mut params = Vec::new();
        while !self.is_punct(Punct::RParen) {
            if self.is_punct(Punct::Spread) {
                self.bump();
                let inner = self.parse_binding_pattern()?;
                params.push(Pattern::Rest(Box::new(inner)));
                break;
            }
            let mut p = self.parse_binding_pattern()?;
            if self.is_punct(Punct::Eq) {
                self.bump();
                let default = self.parse_assign()?;
                p = Pattern::Assignment {
                    pattern: Box::new(p),
                    default,
                };
            }
            params.push(p);
            if !self.is_punct(Punct::RParen) {
                self.eat_punct(Punct::Comma);
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(params)
    }

    fn parse_class_decl(&mut self) -> Result<ClassDecl, ParseError> {
        self.expect_kw(Keyword::Class)?;
        let name = self.parse_opt_ident()?;
        let superclass = if self.eat_kw(Keyword::Extends) {
            Some(self.parse_lhs_expr()?)
        } else {
            None
        };
        let body = self.parse_class_body()?;
        Ok(ClassDecl { name, superclass, body })
    }

    fn parse_class_body(&mut self) -> Result<Vec<ClassMember>, ParseError> {
        self.expect_punct(Punct::LBrace)?;
        let mut members = Vec::new();
        while !self.is_punct(Punct::RBrace) {
            if self.is_punct(Punct::Semicolon) {
                self.bump();
                continue;
            }
            let is_static = self.is_kw(Keyword::Static)
                && !matches!(self.next.token, Token::Punct(Punct::LParen));
            if is_static {
                self.bump();
            }
            // get/set
            let mut method_kind = MethodKind::Normal;
            if (self.is_kw(Keyword::Get) || self.is_kw(Keyword::Set))
                && !matches!(self.next.token, Token::Punct(Punct::LParen))
            {
                method_kind = if self.is_kw(Keyword::Get) {
                    MethodKind::Get
                } else {
                    MethodKind::Set
                };
                self.bump();
            }
            let computed = self.is_punct(Punct::LBracket);
            let key = self.parse_property_key()?;
            if self.is_punct(Punct::LParen) {
                // method
                let (params, body) = self.parse_method_rest()?;
                let func = FunctionExpr {
                    name: match &key {
                        PropertyKey::Ident(n) | PropertyKey::String(n) | PropertyKey::Private(n) => Some(n.clone()),
                        _ => None,
                    },
                    params,
                    body,
                    decls: Vec::new(),
                    is_async: false,
                    is_generator: false,
                    is_arrow: false,
                    expr_body: false,
            line: self.cur.line,
                };
                // detect constructor
                if !is_static {
                    if let PropertyKey::Ident(n) = &key {
                        if &**n == "constructor" {
                            method_kind = MethodKind::Constructor;
                        }
                    }
                }
                members.push(ClassMember {
                    kind: ClassMemberKind::Method { func, kind: method_kind },
                    key,
                    computed,
                    is_static,
                });
            } else {
                // field
                let init = if self.is_punct(Punct::Eq) {
                    self.bump();
                    Some(self.parse_assign()?)
                } else {
                    None
                };
                self.consume_semicolon().ok();
                members.push(ClassMember {
                    kind: ClassMemberKind::Field { init },
                    key,
                    computed,
                    is_static,
                });
            }
        }
        self.expect_punct(Punct::RBrace)?;
        Ok(members)
    }

    fn parse_method_rest(&mut self) -> Result<(Vec<Pattern>, Block), ParseError> {
        self.expect_punct(Punct::LParen)?;
        let params = self.parse_params()?;
        // already consumed RParen in parse_params
        // parse function body
        let prev_gen = self.in_generator;
        let prev_async = self.in_async;
        self.in_generator = false;
        self.in_async = false;
        self.expect_punct(Punct::LBrace)?;
        let mut stmts = Vec::new();
        let mut decls = Vec::new();
        while !self.is_punct(Punct::RBrace) && !matches!(self.cur.token, Token::Eof) {
            if self.is_kw(Keyword::Function) {
                let s = self.parse_function_decl(false, false)?;
                if let Stmt::Function(fd) = s {
                    decls.push(fd);
                }
            } else {
                stmts.push(self.parse_stmt()?);
            }
        }
        self.expect_punct(Punct::RBrace)?;
        self.in_generator = prev_gen;
        self.in_async = prev_async;
        Ok((params, Block { stmts }))
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        self.bump();
        self.expect_punct(Punct::LParen)?;
        let test = self.parse_expr()?;
        self.expect_punct(Punct::RParen)?;
        let cons = Box::new(self.parse_stmt()?);
        let alt = if self.eat_kw(Keyword::Else) {
            Some(Box::new(self.parse_stmt()?))
        } else {
            None
        };
        Ok(Stmt::If { test, cons, alt })
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        self.bump();
        self.expect_punct(Punct::LParen)?;
        let test = self.parse_expr()?;
        self.expect_punct(Punct::RParen)?;
        let body = Box::new(self.parse_stmt()?);
        Ok(Stmt::While { test, body })
    }

    fn parse_do_while(&mut self) -> Result<Stmt, ParseError> {
        self.bump();
        let body = Box::new(self.parse_stmt()?);
        self.expect_kw(Keyword::While)?;
        self.expect_punct(Punct::LParen)?;
        let test = self.parse_expr()?;
        self.expect_punct(Punct::RParen)?;
        self.eat_punct(Punct::Semicolon);
        Ok(Stmt::DoWhile { test, body })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        self.bump();
        let await_tok = self.in_async && self.eat_kw(Keyword::Await);
        self.expect_punct(Punct::LParen)?;
        // Determine init
        let init: Option<ForInit>;
        let for_in_of = false;
        if self.is_punct(Punct::Semicolon) {
            init = None;
            self.bump();
        } else if matches!(self.cur.token, Token::Keyword(Keyword::Var) | Token::Keyword(Keyword::Let) | Token::Keyword(Keyword::Const)) {
            let k = if let Token::Keyword(k) = &self.cur.token { *k } else { unreachable!() };
            self.bump();
            let kind = match k {
                Keyword::Var => VarKind::Var,
                Keyword::Let => VarKind::Let,
                Keyword::Const => VarKind::Const,
                _ => unreachable!(),
            };
            let pattern = self.parse_binding_pattern()?;
            // for-in / for-of?
            if self.is_kw(Keyword::In) {
                self.bump();
                let right = self.parse_expr()?;
                self.expect_punct(Punct::RParen)?;
                let body = Box::new(self.parse_stmt()?);
                return Ok(Stmt::ForIn {
                    left: ForTarget::Var(kind, pattern),
                    right,
                    body,
                });
            }
            if self.is_kw(Keyword::Of) {
                self.bump();
                let right = self.parse_assign()?;
                self.expect_punct(Punct::RParen)?;
                let body = Box::new(self.parse_stmt()?);
                return Ok(Stmt::ForOf {
                    left: ForTarget::Var(kind, pattern),
                    right,
                    body,
                    await_tok,
                });
            }
            let init_val = if self.is_punct(Punct::Eq) {
                self.bump();
                Some(self.parse_assign()?)
            } else {
                None
            };
            let mut decls = vec![VarDeclarator { pattern, init: init_val }];
            while self.eat_punct(Punct::Comma) {
                let p = self.parse_binding_pattern()?;
                let i = if self.is_punct(Punct::Eq) {
                    self.bump();
                    Some(self.parse_assign()?)
                } else {
                    None
                };
                decls.push(VarDeclarator { pattern: p, init: i });
            }
            init = Some(ForInit::Var(VarDecl { kind, decls }));
        } else {
            // expression init — but could be for-in/of with an assignable target
            let save = self.allow_in;
            self.allow_in = false;
            let e = self.parse_expr()?;
            self.allow_in = save;
            if self.is_kw(Keyword::In) {
                self.bump();
                let right = self.parse_expr()?;
                self.expect_punct(Punct::RParen)?;
                let body = Box::new(self.parse_stmt()?);
                let target = expr_to_assign_target(e)?;
                return Ok(Stmt::ForIn {
                    left: ForTarget::Pattern(target),
                    right,
                    body,
                });
            }
            if self.is_kw(Keyword::Of) {
                self.bump();
                let right = self.parse_assign()?;
                self.expect_punct(Punct::RParen)?;
                let body = Box::new(self.parse_stmt()?);
                let target = expr_to_assign_target(e)?;
                return Ok(Stmt::ForOf {
                    left: ForTarget::Pattern(target),
                    right,
                    body,
                    await_tok,
                });
            }
            init = Some(ForInit::Expr(e));
        }
        self.expect_punct(Punct::Semicolon)?;
        let test = if self.is_punct(Punct::Semicolon) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect_punct(Punct::Semicolon)?;
        let update = if self.is_punct(Punct::RParen) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect_punct(Punct::RParen)?;
        let body = Box::new(self.parse_stmt()?);
        let _ = for_in_of;
        Ok(Stmt::For {
            init,
            test,
            update,
            body,
        })
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        self.bump();
        let arg = if self.cur.preceded_by_newline
            || self.is_punct(Punct::Semicolon)
            || self.is_punct(Punct::RBrace)
            || matches!(self.cur.token, Token::Eof)
        {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.consume_semicolon()?;
        Ok(Stmt::Return(arg))
    }

    fn parse_switch(&mut self) -> Result<Stmt, ParseError> {
        self.bump();
        self.expect_punct(Punct::LParen)?;
        let disc = self.parse_expr()?;
        self.expect_punct(Punct::RParen)?;
        self.expect_punct(Punct::LBrace)?;
        let mut cases = Vec::new();
        while !self.is_punct(Punct::RBrace) {
            let is_case = self.eat_kw(Keyword::Case);
            let is_default = !is_case && self.eat_kw(Keyword::Default);
            if !is_case && !is_default {
                return Err(self.err("expected case/default"));
            }
            let test = if is_case {
                let e = self.parse_expr()?;
                self.expect_punct(Punct::Colon)?;
                Some(e)
            } else {
                self.expect_punct(Punct::Colon)?;
                None
            };
            let mut cons = Vec::new();
            while !self.is_punct(Punct::RBrace)
                && !self.is_kw(Keyword::Case)
                && !self.is_kw(Keyword::Default)
            {
                cons.push(self.parse_stmt()?);
            }
            cases.push(SwitchCase { test, cons });
        }
        self.expect_punct(Punct::RBrace)?;
        Ok(Stmt::Switch { disc, cases })
    }

    fn parse_try(&mut self) -> Result<Stmt, ParseError> {
        self.bump();
        let block = self.parse_block()?;
        let handler = if self.eat_kw(Keyword::Catch) {
            let param = if self.eat_punct(Punct::LParen) {
                let p = self.parse_binding_pattern()?;
                self.expect_punct(Punct::RParen)?;
                Some(p)
            } else {
                None
            };
            let body = self.parse_block()?;
            Some(CatchClause { param, body })
        } else {
            None
        };
        let finalizer = if self.eat_kw(Keyword::Finally) {
            Some(self.parse_block()?)
        } else {
            None
        };
        Ok(Stmt::Try {
            block,
            handler,
            finalizer,
        })
    }

    fn parse_with(&mut self) -> Result<Stmt, ParseError> {
        self.bump();
        self.expect_punct(Punct::LParen)?;
        let object = self.parse_expr()?;
        self.expect_punct(Punct::RParen)?;
        let body = Box::new(self.parse_stmt()?);
        Ok(Stmt::With { object, body })
    }

    fn parse_import(&mut self) -> Result<Stmt, ParseError> {
        self.bump();
        // import "mod"  /  import default from "mod" etc.
        let mut specifiers = Vec::new();
        if let Token::String(s) = &self.cur.token {
            let src = s.clone();
            self.bump();
            self.consume_semicolon()?;
            return Ok(Stmt::Import(ImportDecl { specifiers, source: src }));
        }
        // default
        if let Token::Ident(n) = &self.cur.token {
            let n = n.clone();
            self.bump();
            specifiers.push(ImportSpecifier::Default(n));
            if !self.eat_punct(Punct::Comma) {
                // bare default
            }
        }
        // namespace or named
        if self.is_punct(Punct::Star) {
            self.bump();
            self.expect_kw(Keyword::As)?;
            if let Token::Ident(n) = &self.cur.token {
                let n = n.clone();
                self.bump();
                specifiers.push(ImportSpecifier::Namespace(n));
            }
        } else if self.is_punct(Punct::LBrace) {
            self.bump();
            while !self.is_punct(Punct::RBrace) {
                let imported = self.parse_ident_name()?;
                let local = if self.eat_kw(Keyword::As) {
                    self.parse_ident_name()?
                } else {
                    imported.clone()
                };
                specifiers.push(ImportSpecifier::Named { imported, local });
                if !self.is_punct(Punct::RBrace) {
                    self.eat_punct(Punct::Comma);
                }
            }
            self.expect_punct(Punct::RBrace)?;
        }
        self.expect_kw(Keyword::From)?;
        let source = self.parse_string_lit()?;
        self.consume_semicolon()?;
        Ok(Stmt::Import(ImportDecl { specifiers, source }))
    }

    fn parse_export(&mut self) -> Result<Stmt, ParseError> {
        self.bump();
        if self.eat_kw(Keyword::Default) {
            // export default <expr> | function | class
            let expr = if self.is_kw(Keyword::Function) {
                self.bump();
                let name = self.parse_opt_ident()?;
                let func = self.parse_function_rest(name, false, false, false)?;
                Expr::Function(func)
            } else if self.is_kw(Keyword::Class) {
                let c = self.parse_class_decl()?;
                Expr::Class(Box::new(c))
            } else if self.is_kw(Keyword::Async)
                && matches!(self.next.token, Token::Keyword(Keyword::Function))
            {
                self.bump();
                self.expect_kw(Keyword::Function)?;
                let name = self.parse_opt_ident()?;
                let func = self.parse_function_rest(name, true, false, false)?;
                Expr::Function(func)
            } else {
                self.parse_assign()?
            };
            self.consume_semicolon()?;
            return Ok(Stmt::ExportDefault(ExportDefault { expr }));
        }
        if self.is_punct(Punct::Star) {
            self.bump();
            let exported = if self.eat_kw(Keyword::As) {
                Some(self.parse_ident_name()?)
            } else {
                None
            };
            self.expect_kw(Keyword::From)?;
            let source = self.parse_string_lit()?;
            self.consume_semicolon()?;
            return Ok(Stmt::ExportAll(ExportAll { source, exported }));
        }
        // export { ... } [from "..."]
        if self.is_punct(Punct::LBrace) {
            self.bump();
            let mut specifiers = Vec::new();
            while !self.is_punct(Punct::RBrace) {
                let local = self.parse_ident_name()?;
                let exported = if self.eat_kw(Keyword::As) {
                    self.parse_ident_name()?
                } else {
                    local.clone()
                };
                specifiers.push((local, exported));
                if !self.is_punct(Punct::RBrace) {
                    self.eat_punct(Punct::Comma);
                }
            }
            self.expect_punct(Punct::RBrace)?;
            let source = if self.eat_kw(Keyword::From) {
                Some(self.parse_string_lit()?)
            } else {
                None
            };
            self.consume_semicolon()?;
            return Ok(Stmt::ExportNamed(ExportNamed {
                declaration: None,
                specifiers,
                source,
            }));
        }
        // export var/let/const/function/class
        let decl = self.parse_stmt()?;
        Ok(Stmt::ExportNamed(ExportNamed {
            declaration: Some(Box::new(decl)),
            specifiers: Vec::new(),
            source: None,
        }))
    }

    fn parse_ident_name(&mut self) -> Result<Rc<str>, ParseError> {
        match &self.cur.token {
            Token::Ident(n) => {
                let n = n.clone();
                self.bump();
                Ok(n)
            }
            Token::Keyword(k) => {
                let n = Rc::from(kw_str(*k));
                self.bump();
                Ok(n)
            }
            _ => Err(self.err("expected identifier")),
        }
    }

    fn parse_string_lit(&mut self) -> Result<Rc<str>, ParseError> {
        if let Token::String(s) = &self.cur.token {
            let s = s.clone();
            self.bump();
            Ok(s)
        } else {
            Err(self.err("expected string literal"))
        }
    }

    // -------------------------------------------------------------------
    // Expressions
    // -------------------------------------------------------------------

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        let first = self.parse_assign()?;
        if self.is_punct(Punct::Comma) {
            let mut exprs = vec![first];
            while self.eat_punct(Punct::Comma) {
                exprs.push(self.parse_assign()?);
            }
            Ok(Expr::Sequence(exprs))
        } else {
            Ok(first)
        }
    }

    fn parse_assign(&mut self) -> Result<Expr, ParseError> {
        // Try arrow function detection
        if let Some(arrow) = self.try_parse_arrow()? {
            return Ok(arrow);
        }
        // async arrow
        if self.is_kw(Keyword::Async)
            && !self.cur.preceded_by_newline
            && matches!(
                self.next.token,
                Token::Ident(_) | Token::Punct(Punct::LParen)
            )
        {
            let lex_save = self.lex.save();
            let cur_save = self.cur.clone();
            let next_save = self.next.clone();
            self.bump(); // async
            if let Some(arrow) = self.try_parse_arrow_async()? {
                return Ok(arrow);
            }
            // not an async arrow — restore and fall through to parse async as identifier
            self.restore(lex_save, cur_save, next_save);
        }
        // yield
        if self.in_generator && self.is_kw(Keyword::Yield) {
            self.bump();
            let delegate = self.eat_punct(Punct::Star);
            let arg = if self.cur.preceded_by_newline
                || self.is_punct(Punct::Semicolon)
                || self.is_punct(Punct::RBrace)
                || self.is_punct(Punct::RParen)
                || self.is_punct(Punct::RBracket)
                || self.is_punct(Punct::Comma)
                || self.is_punct(Punct::Colon)
                || matches!(self.cur.token, Token::Eof)
            {
                None
            } else {
                Some(Box::new(self.parse_assign()?))
            };
            return Ok(Expr::Yield { arg, delegate });
        }
        let left = self.parse_conditional()?;
        let op = match &self.cur.token {
            Token::Punct(p) => match p {
                Punct::Eq => Some(AssignOp::Assign),
                Punct::PlusEq => Some(AssignOp::AddAssign),
                Punct::MinusEq => Some(AssignOp::SubAssign),
                Punct::StarEq => Some(AssignOp::MulAssign),
                Punct::SlashEq => Some(AssignOp::DivAssign),
                Punct::PercentEq => Some(AssignOp::ModAssign),
                Punct::StarStarEq => Some(AssignOp::ExpAssign),
                Punct::AmpEq => Some(AssignOp::BitAndAssign),
                Punct::PipeEq => Some(AssignOp::BitOrAssign),
                Punct::CaretEq => Some(AssignOp::BitXorAssign),
                Punct::ShlEq => Some(AssignOp::ShlAssign),
                Punct::ShrEq => Some(AssignOp::ShrAssign),
                Punct::UShrEq => Some(AssignOp::UShrAssign),
                Punct::AndEq => Some(AssignOp::AndAssign),
                Punct::OrEq => Some(AssignOp::OrAssign),
                Punct::NullishEq => Some(AssignOp::NullishAssign),
                _ => None,
            },
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let right = self.parse_assign()?;
            let target = expr_to_assign_target(left)?;
            return Ok(Expr::Assignment {
                op,
                left: target,
                right: Box::new(right),
            });
        }
        Ok(left)
    }

    fn parse_conditional(&mut self) -> Result<Expr, ParseError> {
        let test = self.parse_nullish()?;
        if self.is_punct(Punct::Question) && !matches!(self.cur.token, Token::Punct(Punct::Optional)) {
            self.bump();
            let cons = self.parse_assign()?;
            self.expect_punct(Punct::Colon)?;
            let alt = self.parse_assign()?;
            Ok(Expr::Conditional {
                test: Box::new(test),
                cons: Box::new(cons),
                alt: Box::new(alt),
            })
        } else {
            Ok(test)
        }
    }

    fn parse_nullish(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_logical_or()?;
        while self.is_punct(Punct::Nullish) {
            self.bump();
            let right = self.parse_logical_or()?;
            left = Expr::Logical {
                op: LogicalOp::Nullish,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_logical_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_logical_and()?;
        while self.is_punct(Punct::Or) {
            self.bump();
            let right = self.parse_logical_and()?;
            left = Expr::Logical {
                op: LogicalOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_logical_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_bit_or()?;
        while self.is_punct(Punct::And) {
            self.bump();
            let right = self.parse_bit_or()?;
            left = Expr::Logical {
                op: LogicalOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_bit_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_bit_xor()?;
        while self.is_punct(Punct::Pipe) {
            self.bump();
            let right = self.parse_bit_xor()?;
            left = Expr::Binary {
                op: BinaryOp::BitOr,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }
    fn parse_bit_xor(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_bit_and()?;
        while self.is_punct(Punct::Caret) {
            self.bump();
            let right = self.parse_bit_and()?;
            left = Expr::Binary {
                op: BinaryOp::BitXor,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }
    fn parse_bit_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_equality()?;
        while self.is_punct(Punct::Amp) {
            self.bump();
            let right = self.parse_equality()?;
            left = Expr::Binary {
                op: BinaryOp::BitAnd,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_relational()?;
        loop {
            let op = match &self.cur.token {
                Token::Punct(Punct::EqEq) => Some(BinaryOp::Eq),
                Token::Punct(Punct::NotEq) => Some(BinaryOp::NotEq),
                Token::Punct(Punct::EqEqEq) => Some(BinaryOp::StrictEq),
                Token::Punct(Punct::NotEqEq) => Some(BinaryOp::StrictNotEq),
                _ => None,
            };
            if let Some(op) = op {
                self.bump();
                let right = self.parse_relational()?;
                left = Expr::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_relational(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_shift()?;
        loop {
            let op = match &self.cur.token {
                Token::Punct(Punct::Lt) => Some(BinaryOp::Lt),
                Token::Punct(Punct::Le) => Some(BinaryOp::Le),
                Token::Punct(Punct::Gt) => Some(BinaryOp::Gt),
                Token::Punct(Punct::Ge) => Some(BinaryOp::Ge),
                Token::Keyword(Keyword::Instanceof) => Some(BinaryOp::InstanceOf),
                Token::Keyword(Keyword::In) if self.allow_in => Some(BinaryOp::In),
                _ => None,
            };
            if let Some(op) = op {
                self.bump();
                let right = self.parse_shift()?;
                left = Expr::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_shift(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match &self.cur.token {
                Token::Punct(Punct::Shl) => Some(BinaryOp::Shl),
                Token::Punct(Punct::Shr) => Some(BinaryOp::Shr),
                Token::Punct(Punct::UShr) => Some(BinaryOp::UShr),
                _ => None,
            };
            if let Some(op) = op {
                self.bump();
                let right = self.parse_additive()?;
                left = Expr::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match &self.cur.token {
                Token::Punct(Punct::Plus) => Some(BinaryOp::Add),
                Token::Punct(Punct::Minus) => Some(BinaryOp::Sub),
                _ => None,
            };
            if let Some(op) = op {
                self.bump();
                let right = self.parse_multiplicative()?;
                left = Expr::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_exponent()?;
        loop {
            let op = match &self.cur.token {
                Token::Punct(Punct::Star) => Some(BinaryOp::Mul),
                Token::Punct(Punct::Slash) => Some(BinaryOp::Div),
                Token::Punct(Punct::Percent) => Some(BinaryOp::Mod),
                _ => None,
            };
            if let Some(op) = op {
                self.bump();
                let right = self.parse_exponent()?;
                left = Expr::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_exponent(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_unary()?;
        if self.is_punct(Punct::StarStar) {
            self.bump();
            let right = self.parse_exponent()?;
            return Ok(Expr::Binary {
                op: BinaryOp::Exp,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        // prefix update
        if self.is_punct(Punct::PlusPlus) || self.is_punct(Punct::MinusMinus) {
            let op = if self.is_punct(Punct::PlusPlus) {
                UpdateOp::Inc
            } else {
                UpdateOp::Dec
            };
            self.bump();
            let arg = self.parse_unary()?;
            return Ok(Expr::Update {
                op,
                arg: Box::new(arg),
                prefix: true,
            });
        }
        let op = match &self.cur.token {
            Token::Punct(Punct::Plus) => Some(UnaryOp::Pos),
            Token::Punct(Punct::Minus) => Some(UnaryOp::Neg),
            Token::Punct(Punct::Bang) => Some(UnaryOp::Not),
            Token::Punct(Punct::Tilde) => Some(UnaryOp::BitNot),
            Token::Keyword(Keyword::Typeof) => Some(UnaryOp::TypeOf),
            Token::Keyword(Keyword::Void) => Some(UnaryOp::Void),
            Token::Keyword(Keyword::Delete) => Some(UnaryOp::Delete),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let arg = self.parse_unary()?;
            return Ok(Expr::Unary {
                op,
                arg: Box::new(arg),
            });
        }
        // await
        if (self.in_async || self.is_module) && self.is_kw(Keyword::Await) {
            self.bump();
            let arg = self.parse_unary()?;
            return Ok(Expr::Await(Box::new(arg)));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_lhs_expr()?;
        if !self.cur.preceded_by_newline
            && (self.is_punct(Punct::PlusPlus) || self.is_punct(Punct::MinusMinus))
        {
            let op = if self.is_punct(Punct::PlusPlus) {
                UpdateOp::Inc
            } else {
                UpdateOp::Dec
            };
            self.bump();
            e = Expr::Update {
                op,
                arg: Box::new(e),
                prefix: false,
            };
        }
        Ok(e)
    }

    fn parse_lhs_expr(&mut self) -> Result<Expr, ParseError> {
        // new / call chain
        let mut e = if self.is_kw(Keyword::New) {
            self.parse_new()?
        } else {
            self.parse_primary()?
        };
        loop {
            match &self.cur.token {
                Token::Punct(Punct::Dot) => {
                    self.bump();
                    let name = self.parse_member_name()?;
                    e = Expr::Member {
                        object: Box::new(e),
                        property: name,
                        optional: false,
                    };
                }
                Token::Punct(Punct::Optional) => {
                    // `?.` can be followed by a member name, `(` (optional
                    // call), or `[` (optional computed member).
                    if matches!(self.next.token, Token::Punct(Punct::LParen)) {
                        self.bump(); // consume `?.`
                        let args = self.parse_args()?;
                        e = Expr::Call {
                            callee: Box::new(e),
                            args,
                            optional: true,
                        };
                    } else if matches!(self.next.token, Token::Punct(Punct::LBracket)) {
                        self.bump(); // consume `?.`
                        self.bump(); // consume `[`
                        let idx = self.parse_expr()?;
                        self.expect_punct(Punct::RBracket)?;
                        e = Expr::Member {
                            object: Box::new(e),
                            property: MemberProp::Computed(Box::new(idx)),
                            optional: true,
                        };
                    } else {
                        self.bump();
                        let name = self.parse_member_name()?;
                        e = Expr::Member {
                            object: Box::new(e),
                            property: name,
                            optional: true,
                        };
                    }
                }
                Token::Punct(Punct::LBracket) => {
                    self.bump();
                    let idx = self.parse_expr()?;
                    self.expect_punct(Punct::RBracket)?;
                    e = Expr::Member {
                        object: Box::new(e),
                        property: MemberProp::Computed(Box::new(idx)),
                        optional: false,
                    };
                }
                Token::Punct(Punct::LParen) => {
                    let args = self.parse_args()?;
                    e = Expr::Call {
                        callee: Box::new(e),
                        args,
                        optional: false,
                    };
                }
                Token::TemplateNoSub { .. }
                | Token::TemplateHead { .. } => {
                    let (quasis, exprs) = self.parse_template_rest()?;
                    e = Expr::TaggedTemplate {
                        tag: Box::new(e),
                        quasis,
                        exprs,
                    };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_member_name(&mut self) -> Result<MemberProp, ParseError> {
        match &self.cur.token {
            Token::Ident(n) => {
                let n = n.clone();
                self.bump();
                Ok(MemberProp::Ident(n))
            }
            Token::PrivateIdent(n) => {
                let n = n.clone();
                self.bump();
                Ok(MemberProp::Private(n))
            }
            Token::Keyword(k) => {
                let n = Rc::from(kw_str(*k));
                self.bump();
                Ok(MemberProp::Ident(n))
            }
            _ => Err(self.err("expected member name")),
        }
    }

    fn parse_new(&mut self) -> Result<Expr, ParseError> {
        self.bump();
        // new.target
        if self.is_punct(Punct::Dot) {
            self.bump();
            if let Token::Ident(n) = &self.cur.token {
                if &**n == "target" {
                    self.bump();
                    return Ok(Expr::NewTarget);
                }
            }
        }
        let callee = if self.is_kw(Keyword::New) {
            self.parse_new()?
        } else {
            self.parse_primary()?
        };
        // optional member access (but not call) after new
        let mut callee = callee;
        loop {
            match &self.cur.token {
                Token::Punct(Punct::Dot) => {
                    self.bump();
                    let name = self.parse_member_name()?;
                    callee = Expr::Member {
                        object: Box::new(callee),
                        property: name,
                        optional: false,
                    };
                }
                Token::Punct(Punct::LBracket) => {
                    self.bump();
                    let idx = self.parse_expr()?;
                    self.expect_punct(Punct::RBracket)?;
                    callee = Expr::Member {
                        object: Box::new(callee),
                        property: MemberProp::Computed(Box::new(idx)),
                        optional: false,
                    };
                }
                _ => break,
            }
        }
        let args = if self.is_punct(Punct::LParen) {
            self.parse_args()?
        } else {
            Vec::new()
        };
        Ok(Expr::New {
            callee: Box::new(callee),
            args,
        })
    }

    fn parse_args(&mut self) -> Result<Vec<CallArg>, ParseError> {
        self.expect_punct(Punct::LParen)?;
        let mut args = Vec::new();
        while !self.is_punct(Punct::RParen) {
            if self.is_punct(Punct::Spread) {
                self.bump();
                let e = self.parse_assign()?;
                args.push(CallArg::Spread(e));
            } else {
                let e = self.parse_assign()?;
                args.push(CallArg::Expr(e));
            }
            if !self.is_punct(Punct::RParen) {
                self.eat_punct(Punct::Comma);
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match &self.cur.token {
            Token::Number(n) => {
                let n = *n;
                self.bump();
                Ok(Expr::Number(n))
            }
            Token::BigInt(s) => {
                let s = s.clone();
                self.bump();
                Ok(Expr::BigInt(s))
            }
            Token::String(s) => {
                let s = s.clone();
                self.bump();
                Ok(Expr::String(s))
            }
            Token::Regex { pattern, flags } => {
                let p = pattern.clone();
                let f = flags.clone();
                self.bump();
                Ok(Expr::Regex { pattern: p, flags: f })
            }
            Token::True => {
                self.bump();
                Ok(Expr::Bool(true))
            }
            Token::False => {
                self.bump();
                Ok(Expr::Bool(false))
            }
            Token::Null => {
                self.bump();
                Ok(Expr::Null)
            }
            Token::Undefined => {
                self.bump();
                Ok(Expr::Undefined)
            }
            Token::Ident(n) => {
                let n = n.clone();
                self.bump();
                Ok(Expr::Ident(n))
            }
            // Contextual keywords treated as identifiers when not in their
            // special context (yield outside generators, await outside async,
            // and always-contextual: async, of, as, from, static, get, set).
            Token::Keyword(k)
                if matches!(
                    k,
                    Keyword::Async | Keyword::Of | Keyword::As | Keyword::From | Keyword::Static
                ) =>
            {
                let n = Rc::from(kw_str(*k));
                self.bump();
                Ok(Expr::Ident(n))
            }
            Token::Keyword(Keyword::Yield) if !self.in_generator => {
                let n = Rc::from("yield");
                self.bump();
                Ok(Expr::Ident(n))
            }
            Token::Keyword(Keyword::Await) if !self.in_async && !self.is_module => {
                let n = Rc::from("await");
                self.bump();
                Ok(Expr::Ident(n))
            }
            Token::Keyword(Keyword::This) => {
                self.bump();
                Ok(Expr::This)
            }
            Token::Keyword(Keyword::Super) => {
                self.bump();
                Ok(Expr::Super)
            }
            Token::Keyword(Keyword::Function) => {
                self.bump();
                let is_generator = self.eat_punct(Punct::Star);
                let name = self.parse_opt_ident()?;
                let func = self.parse_function_rest(name, false, is_generator, false)?;
                Ok(Expr::Function(func))
            }
            Token::Keyword(Keyword::Class) => {
                let c = self.parse_class_decl()?;
                Ok(Expr::Class(Box::new(c)))
            }
            Token::Keyword(Keyword::Async)
                if matches!(self.next.token, Token::Keyword(Keyword::Function)) =>
            {
                self.bump();
                self.expect_kw(Keyword::Function)?;
                let is_generator = self.eat_punct(Punct::Star);
                let name = self.parse_opt_ident()?;
                let func = self.parse_function_rest(name, true, is_generator, false)?;
                Ok(Expr::Function(func))
            }
            Token::Keyword(Keyword::Import) => {
                self.bump();
                if self.is_punct(Punct::Dot) {
                    self.bump();
                    if let Token::Ident(n) = &self.cur.token {
                        if &**n == "meta" {
                            self.bump();
                            return Ok(Expr::ImportMeta);
                        }
                    }
                }
                // import(...)
                if self.is_punct(Punct::LParen) {
                    self.bump();
                    let e = self.parse_assign()?;
                    self.expect_punct(Punct::RParen)?;
                    return Ok(Expr::ImportCall(Box::new(e)));
                }
                Err(self.err("invalid import"))
            }
            Token::Punct(Punct::LParen) => {
                // Could be parenthesized expr or arrow; try arrow first via lookahead.
                self.bump();
                let e = self.parse_expr()?;
                self.expect_punct(Punct::RParen)?;
                Ok(Expr::Paren(Box::new(e)))
            }
            Token::Punct(Punct::LBracket) => self.parse_array_lit(),
            Token::Punct(Punct::LBrace) => self.parse_object_lit(),
            Token::TemplateNoSub { .. } | Token::TemplateHead { .. } => {
                let (quasis, exprs) = self.parse_template_rest()?;
                Ok(Expr::TemplateLit {
                    quasis,
                    exprs,
                    tag: None,
                })
            }
            _ => Err(self.err(&format!("unexpected token: {:?}", self.cur.token))),
        }
    }

    fn parse_template_rest(&mut self) -> Result<(Vec<Rc<str>>, Vec<Expr>), ParseError> {
        let mut quasis = Vec::new();
        let mut exprs = Vec::new();
        match &self.cur.token.clone() {
            Token::TemplateNoSub { cooked, .. } => {
                quasis.push(cooked.clone());
                self.bump();
                return Ok((quasis, exprs));
            }
            Token::TemplateHead { cooked, .. } => {
                quasis.push(cooked.clone());
                self.bump();
            }
            _ => return Err(self.err("expected template")),
        }
        loop {
            let e = self.parse_expr()?;
            exprs.push(e);
            match &self.cur.token.clone() {
                Token::TemplateMiddle { cooked, .. } => {
                    quasis.push(cooked.clone());
                    self.bump();
                }
                Token::TemplateTail { cooked, .. } => {
                    quasis.push(cooked.clone());
                    self.bump();
                    break;
                }
                _ => return Err(self.err("unterminated template")),
            }
        }
        Ok((quasis, exprs))
    }

    fn parse_array_lit(&mut self) -> Result<Expr, ParseError> {
        self.expect_punct(Punct::LBracket)?;
        let mut elements = Vec::new();
        while !self.is_punct(Punct::RBracket) {
            if self.is_punct(Punct::Comma) {
                self.bump();
                elements.push(ArrayElement::Hole);
                continue;
            }
            if self.is_punct(Punct::Spread) {
                self.bump();
                let e = self.parse_assign()?;
                elements.push(ArrayElement::Spread(e));
            } else {
                let e = self.parse_assign()?;
                elements.push(ArrayElement::Item(e));
            }
            if !self.is_punct(Punct::RBracket) {
                self.eat_punct(Punct::Comma);
            }
        }
        self.expect_punct(Punct::RBracket)?;
        Ok(Expr::Array(elements))
    }

    fn parse_object_lit(&mut self) -> Result<Expr, ParseError> {
        self.expect_punct(Punct::LBrace)?;
        let mut props = Vec::new();
        while !self.is_punct(Punct::RBrace) {
            if self.is_punct(Punct::Spread) {
                self.bump();
                let e = self.parse_assign()?;
                props.push(ObjectProp {
                    key: PropertyKey::Ident(Rc::from("")),
                    computed: false,
                    value: ObjectPropValue::Expr(e),
                    kind: PropKindAst::Spread,
                });
                if !self.is_punct(Punct::RBrace) {
                    self.eat_punct(Punct::Comma);
                }
                continue;
            }
            // get/set
            let mut kind = PropKindAst::Init;
            if (self.is_kw(Keyword::Get) || self.is_kw(Keyword::Set))
                && !matches!(self.next.token, Token::Punct(Punct::Colon) | Token::Punct(Punct::Comma) | Token::Punct(Punct::RBrace) | Token::Punct(Punct::LParen))
            {
                kind = if self.is_kw(Keyword::Get) {
                    PropKindAst::Get
                } else {
                    PropKindAst::Set
                };
                self.bump();
            }
            let computed = self.is_punct(Punct::LBracket);
            let key = self.parse_property_key()?;
            if matches!(kind, PropKindAst::Get) || matches!(kind, PropKindAst::Set) {
                let (params, body) = self.parse_method_rest()?;
                let func = FunctionExpr {
                    name: None,
                    params,
                    body,
                    decls: Vec::new(),
                    is_async: false,
                    is_generator: false,
                    is_arrow: false,
                    expr_body: false,
            line: self.cur.line,
                };
                props.push(ObjectProp {
                    key,
                    computed,
                    value: ObjectPropValue::Expr(Expr::Function(func)),
                    kind,
                });
            } else if self.is_punct(Punct::LParen) {
                // method shorthand
                let (params, body) = self.parse_method_rest()?;
                let func = FunctionExpr {
                    name: None,
                    params,
                    body,
                    decls: Vec::new(),
                    is_async: false,
                    is_generator: false,
                    is_arrow: false,
                    expr_body: false,
            line: self.cur.line,
                };
                props.push(ObjectProp {
                    key,
                    computed,
                    value: ObjectPropValue::Expr(Expr::Function(func)),
                    kind: PropKindAst::Method,
                });
            } else if self.eat_punct(Punct::Colon) {
                let e = self.parse_assign()?;
                props.push(ObjectProp {
                    key,
                    computed,
                    value: ObjectPropValue::Expr(e),
                    kind: PropKindAst::Init,
                });
            } else {
                // shorthand { foo, foo = default }
                let name = match &key {
                    PropertyKey::Ident(n) => n.clone(),
                    _ => return Err(self.err("invalid shorthand property")),
                };
                let e = if self.eat_punct(Punct::Eq) {
                    let default = self.parse_assign()?;
                    Expr::Conditional {
                        test: Box::new(Expr::Binary {
                            op: BinaryOp::StrictNotEq,
                            left: Box::new(Expr::Ident(name.clone())),
                            right: Box::new(Expr::Undefined),
                        }),
                        cons: Box::new(Expr::Ident(name.clone())),
                        alt: Box::new(default),
                    }
                } else {
                    Expr::Ident(name.clone())
                };
                props.push(ObjectProp {
                    key,
                    computed,
                    value: ObjectPropValue::Expr(e),
                    kind: PropKindAst::Init,
                });
            }
            if !self.is_punct(Punct::RBrace) {
                self.eat_punct(Punct::Comma);
            }
        }
        self.expect_punct(Punct::RBrace)?;
        Ok(Expr::Object(props))
    }

    // -------------------------------------------------------------------
    // Arrow function detection
    // -------------------------------------------------------------------

    fn try_parse_arrow(&mut self) -> Result<Option<Expr>, ParseError> {
        // Ident => ...
        if let Token::Ident(name) = &self.cur.token {
            if matches!(self.next.token, Token::Punct(Punct::Arrow)) {
                let n = name.clone();
                self.bump();
                self.bump();
                return Ok(Some(self.finish_arrow(vec![Pattern::Ident(n)], false, false)?));
            }
        }
        // (params) => ...
        if self.is_punct(Punct::LParen) {
            if let Some(params) = self.try_parse_arrow_params()? {
                if self.is_punct(Punct::Arrow) {
                    self.bump();
                    return Ok(Some(self.finish_arrow(params, false, false)?));
                }
            }
        }
        Ok(None)
    }

    fn try_parse_arrow_async(&mut self) -> Result<Option<Expr>, ParseError> {
        if let Token::Ident(name) = &self.cur.token {
            if matches!(self.next.token, Token::Punct(Punct::Arrow)) {
                let n = name.clone();
                self.bump();
                self.bump();
                return Ok(Some(self.finish_arrow(vec![Pattern::Ident(n)], true, false)?));
            }
        }
        if self.is_punct(Punct::LParen) {
            if let Some(params) = self.try_parse_arrow_params()? {
                if self.is_punct(Punct::Arrow) {
                    self.bump();
                    return Ok(Some(self.finish_arrow(params, true, false)?));
                }
            }
        }
        Ok(None)
    }

    fn try_parse_arrow_params(&mut self) -> Result<Option<Vec<Pattern>>, ParseError> {
        // Speculatively parse a parenthesised param list; back out on failure
        // (including parse errors) by restoring both the lexer and the cur/next
        // lookahead tokens.
        let lex_save = self.lex.save();
        let cur_save = self.cur.clone();
        let next_save = self.next.clone();
        let attempt: Result<Option<Vec<Pattern>>, ParseError> = (|| {
            if !self.is_punct(Punct::LParen) {
                return Ok(None);
            }
            self.bump(); // (
            let mut params = Vec::new();
            if self.is_punct(Punct::RParen) {
                self.bump();
            } else {
                loop {
                    if self.is_punct(Punct::Spread) {
                        self.bump();
                        let p = self.parse_binding_pattern()?;
                        params.push(Pattern::Rest(Box::new(p)));
                        break;
                    }
                    let mut p = self.parse_binding_pattern()?;
                    if self.is_punct(Punct::Eq) {
                        self.bump();
                        let d = self.parse_assign()?;
                        p = Pattern::Assignment {
                            pattern: Box::new(p),
                            default: d,
                        };
                    }
                    params.push(p);
                    if self.is_punct(Punct::Comma) {
                        self.bump();
                        if self.is_punct(Punct::RParen) {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                if !self.is_punct(Punct::RParen) {
                    return Ok(None);
                }
                self.bump(); // )
            }
            if !self.is_punct(Punct::Arrow) {
                return Ok(None);
            }
            Ok(Some(params))
        })();
        match attempt {
            Ok(Some(p)) => Ok(Some(p)),
            _ => {
                self.restore(lex_save, cur_save, next_save);
                Ok(None)
            }
        }
    }

    fn restore(
        &mut self,
        lex_save: crate::lexer::LexSave,
        cur: TokenWithPos,
        next: TokenWithPos,
    ) {
        self.lex.restore(lex_save);
        self.cur = cur;
        self.next = next;
    }

    fn finish_arrow(
        &mut self,
        params: Vec<Pattern>,
        is_async: bool,
        _is_gen: bool,
    ) -> Result<Expr, ParseError> {
        let prev_async = self.in_async;
        self.in_async = is_async;
        let (body, expr_body, decls) = if self.is_punct(Punct::LBrace) {
            self.bump();
            let mut stmts = Vec::new();
            let mut decls = Vec::new();
            while !self.is_punct(Punct::RBrace) && !matches!(self.cur.token, Token::Eof) {
                if self.is_kw(Keyword::Function)
                    || (self.is_kw(Keyword::Async)
                        && matches!(self.next.token, Token::Keyword(Keyword::Function)))
                {
                    let start_async = self.is_kw(Keyword::Async);
                    if start_async {
                        self.bump();
                    }
                    let s = self.parse_function_decl(start_async, false)?;
                    if let Stmt::Function(fd) = s {
                        decls.push(fd);
                    }
                } else {
                    stmts.push(self.parse_stmt()?);
                }
            }
            self.expect_punct(Punct::RBrace)?;
            (Block { stmts }, false, decls)
        } else {
            let e = self.parse_assign()?;
            (Block { stmts: vec![Stmt::Return(Some(e))] }, true, Vec::new())
        };
        self.in_async = prev_async;
        Ok(Expr::Arrow(FunctionExpr {
            name: None,
            params,
            body,
            decls,
            is_async,
            is_generator: false,
            is_arrow: true,
            expr_body,
            line: self.cur.line,
        }))
    }

    // (lexer save/restore handled via self.lex.save()/restore() + restore())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn expr_to_assign_target(e: Expr) -> Result<AssignTarget, ParseError> {
    match e {
        Expr::Ident(n) => Ok(AssignTarget::Ident(n)),
        Expr::Member {
            object,
            property,
            optional,
        } => {
            if optional {
                Err(ParseError {
                    message: "optional chaining cannot be assignment target".into(),
                    line: 0,
                    col: 0,
                })
            } else {
                Ok(AssignTarget::Member { object, property })
            }
        }
        Expr::Array(els) => {
            let mut elems = Vec::new();
            let mut rest = None;
            for el in els {
                match el {
                    ArrayElement::Item(e) => elems.push(Some(PatternElement {
                        pattern: expr_to_pattern(e)?,
                    })),
                    ArrayElement::Hole => elems.push(None),
                    ArrayElement::Spread(e) => {
                        rest = Some(Box::new(expr_to_pattern(e)?));
                    }
                }
            }
            Ok(AssignTarget::Pattern(Box::new(Pattern::Array { elements: elems, rest })))
        }
        Expr::Object(props) => {
            let mut properties = Vec::new();
            let mut rest = None;
            for p in props {
                if matches!(p.kind, PropKindAst::Spread) {
                    if let ObjectPropValue::Expr(Expr::Ident(n)) = p.value {
                        rest = Some(n);
                    }
                    continue;
                }
                let value = match p.value {
                    ObjectPropValue::Expr(e) => expr_to_pattern(e)?,
                    ObjectPropValue::Pattern(p) => p,
                };
                properties.push(ObjectPatternProp {
                    key: p.key,
                    computed: p.computed,
                    value,
                    shorthand: false,
                });
            }
            Ok(AssignTarget::Pattern(Box::new(Pattern::Object { properties, rest })))
        }
        Expr::Paren(e) => expr_to_assign_target(*e),
        _ => Err(ParseError {
            message: "invalid assignment target".into(),
            line: 0,
            col: 0,
        }),
    }
}

fn expr_to_pattern(e: Expr) -> Result<Pattern, ParseError> {
    match e {
        Expr::Ident(n) => Ok(Pattern::Ident(n)),
        Expr::Array(_) | Expr::Object(_) => expr_to_assign_target(e).and_then(|t| match t {
            AssignTarget::Pattern(p) => Ok(*p),
            _ => Err(ParseError {
                message: "invalid pattern".into(),
                line: 0,
                col: 0,
            }),
        }),
        Expr::Assignment { .. } => {
            // e.g. default via `x = 1` inside destructuring — handled by parser normally
            Err(ParseError {
                message: "unexpected assignment in pattern".into(),
                line: 0,
                col: 0,
            })
        }
        _ => Err(ParseError {
            message: "invalid destructuring pattern".into(),
            line: 0,
            col: 0,
        }),
    }
}

fn punct_str(p: Punct) -> &'static str {
    match p {
        Punct::LParen => "(",
        Punct::RParen => ")",
        Punct::LBrace => "{",
        Punct::RBrace => "}",
        Punct::LBracket => "[",
        Punct::RBracket => "]",
        Punct::Comma => ",",
        Punct::Semicolon => ";",
        Punct::Dot => ".",
        Punct::Question => "?",
        Punct::Colon => ":",
        Punct::Tilde => "~",
        Punct::Bang => "!",
        Punct::Plus => "+",
        Punct::Minus => "-",
        Punct::Star => "*",
        Punct::Slash => "/",
        Punct::Percent => "%",
        Punct::StarStar => "**",
        Punct::Amp => "&",
        Punct::Pipe => "|",
        Punct::Caret => "^",
        Punct::Shl => "<<",
        Punct::Shr => ">>",
        Punct::UShr => ">>>",
        Punct::Eq => "=",
        Punct::EqEq => "==",
        Punct::EqEqEq => "===",
        Punct::NotEq => "!=",
        Punct::NotEqEq => "!==",
        Punct::Lt => "<",
        Punct::Le => "<=",
        Punct::Gt => ">",
        Punct::Ge => ">=",
        Punct::And => "&&",
        Punct::Or => "||",
        Punct::Nullish => "??",
        Punct::PlusPlus => "++",
        Punct::MinusMinus => "--",
        Punct::PlusEq => "+=",
        Punct::MinusEq => "-=",
        Punct::StarEq => "*=",
        Punct::SlashEq => "/=",
        Punct::PercentEq => "%=",
        Punct::StarStarEq => "**=",
        Punct::AmpEq => "&=",
        Punct::PipeEq => "|=",
        Punct::CaretEq => "^=",
        Punct::ShlEq => "<<=",
        Punct::ShrEq => ">>=",
        Punct::UShrEq => ">>>=",
        Punct::AndEq => "&&=",
        Punct::OrEq => "||=",
        Punct::NullishEq => "??=",
        Punct::Arrow => "=>",
        Punct::Spread => "...",
        Punct::Optional => "?.",
        Punct::Hash => "#",
        Punct::At => "@",
    }
}

fn kw_str(k: Keyword) -> &'static str {
    match k {
        Keyword::Var => "var",
        Keyword::Let => "let",
        Keyword::Const => "const",
        Keyword::Function => "function",
        Keyword::Return => "return",
        Keyword::If => "if",
        Keyword::Else => "else",
        Keyword::While => "while",
        Keyword::Do => "do",
        Keyword::For => "for",
        Keyword::Break => "break",
        Keyword::Continue => "continue",
        Keyword::Switch => "switch",
        Keyword::Case => "case",
        Keyword::Default => "default",
        Keyword::Throw => "throw",
        Keyword::Try => "try",
        Keyword::Catch => "catch",
        Keyword::Finally => "finally",
        Keyword::New => "new",
        Keyword::Delete => "delete",
        Keyword::Typeof => "typeof",
        Keyword::Instanceof => "instanceof",
        Keyword::In => "in",
        Keyword::Of => "of",
        Keyword::This => "this",
        Keyword::Super => "super",
        Keyword::Class => "class",
        Keyword::Extends => "extends",
        Keyword::Static => "static",
        Keyword::Get => "get",
        Keyword::Set => "set",
        Keyword::Async => "async",
        Keyword::Await => "await",
        Keyword::Yield => "yield",
        Keyword::Import => "import",
        Keyword::Export => "export",
        Keyword::From => "from",
        Keyword::As => "as",
        Keyword::Default_ => "default",
        Keyword::With => "with",
        Keyword::Debugger => "debugger",
        Keyword::Void => "void",
        Keyword::Null => "null",
        Keyword::True => "true",
        Keyword::False => "false",
        Keyword::Undefined => "undefined",
        Keyword::Constructor => "constructor",
    }
}

/// Public entry: parse a script.
pub fn parse(src: &str) -> Result<Program, ParseError> {
    let mut p = Parser::new(src);
    p.parse_program()
}

/// Parse as a module.
pub fn parse_module(src: &str) -> Result<Program, ParseError> {
    let mut p = Parser::with_mode(src, true);
    p.parse_program()
}
