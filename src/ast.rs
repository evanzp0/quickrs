//! Abstract Syntax Tree definitions produced by the parser.

use std::rc::Rc;

/// A complete program / script.
#[derive(Debug, Clone)]
pub struct Program {
    pub body: Vec<Stmt>,
    pub strict: bool,
    pub source_type: SourceType,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceType {
    Script,
    Module,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Empty,
    Block(Block),
    Expr(Expr),
    Var(VarDecl),
    Function(FunctionDecl),
    Class(ClassDecl),
    Return(Option<Expr>),
    If {
        test: Expr,
        cons: Box<Stmt>,
        alt: Option<Box<Stmt>>,
    },
    While { test: Expr, body: Box<Stmt> },
    DoWhile { test: Expr, body: Box<Stmt> },
    For {
        init: Option<ForInit>,
        test: Option<Expr>,
        update: Option<Expr>,
        body: Box<Stmt>,
    },
    ForIn {
        left: ForTarget,
        right: Expr,
        body: Box<Stmt>,
    },
    ForOf {
        left: ForTarget,
        right: Expr,
        body: Box<Stmt>,
        await_tok: bool,
    },
    Switch {
        disc: Expr,
        cases: Vec<SwitchCase>,
    },
    Break(Option<Rc<str>>),
    Continue(Option<Rc<str>>),
    Throw(Expr),
    Try {
        block: Block,
        handler: Option<CatchClause>,
        finalizer: Option<Block>,
    },
    Labeled {
        label: Rc<str>,
        body: Box<Stmt>,
    },
    Debugger,
    With { object: Expr, body: Box<Stmt> },
    // Module-level
    Import(ImportDecl),
    ExportNamed(ExportNamed),
    ExportDefault(ExportDefault),
    ExportAll(ExportAll),
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum ForInit {
    Var(VarDecl),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum ForTarget {
    Var(VarKind, Pattern),
    Pattern(AssignTarget),
}

#[derive(Debug, Clone)]
pub struct VarDecl {
    pub kind: VarKind,
    pub decls: Vec<VarDeclarator>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VarKind {
    Var,
    Let,
    Const,
}

#[derive(Debug, Clone)]
pub struct VarDeclarator {
    pub pattern: Pattern,
    pub init: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct FunctionDecl {
    pub name: Option<Rc<str>>,
    pub func: FunctionExpr,
    pub is_async: bool,
    pub is_generator: bool,
}

#[derive(Debug, Clone)]
pub struct FunctionExpr {
    pub name: Option<Rc<str>>,
    pub params: Vec<Pattern>,
    pub body: Block,
    pub decls: Vec<FunctionDecl>, // hoisted nested function declarations
    pub is_async: bool,
    pub is_generator: bool,
    pub is_arrow: bool,
    pub expr_body: bool, // arrow `() => expr`
    pub line: u32, // source line where the function is defined (for stack traces)
}

#[derive(Debug, Clone)]
pub struct ClassDecl {
    pub name: Option<Rc<str>>,
    pub superclass: Option<Expr>,
    pub body: Vec<ClassMember>,
}

#[derive(Debug, Clone)]
pub struct ClassMember {
    pub kind: ClassMemberKind,
    pub key: PropertyKey,
    pub computed: bool,
    pub is_static: bool,
}

#[derive(Debug, Clone)]
pub enum ClassMemberKind {
    Method {
        func: FunctionExpr,
        kind: MethodKind,
    },
    Field {
        init: Option<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MethodKind {
    Normal,
    Constructor,
    Get,
    Set,
}

#[derive(Debug, Clone)]
pub enum PropertyKey {
    Ident(Rc<str>),
    String(Rc<str>),
    Number(f64),
    Computed(Expr),
    Private(Rc<str>),
}

#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub test: Option<Expr>, // None => default
    pub cons: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct CatchClause {
    pub param: Option<Pattern>,
    pub body: Block,
}

#[derive(Debug, Clone)]
pub struct ImportDecl {
    pub specifiers: Vec<ImportSpecifier>,
    pub source: Rc<str>,
}

#[derive(Debug, Clone)]
pub enum ImportSpecifier {
    Default(Rc<str>),
    Namespace(Rc<str>),
    Named { imported: Rc<str>, local: Rc<str> },
}

#[derive(Debug, Clone)]
pub struct ExportNamed {
    pub declaration: Option<Box<Stmt>>,
    pub specifiers: Vec<(Rc<str>, Rc<str>)>, // (local, exported)
    pub source: Option<Rc<str>>,
}

#[derive(Debug, Clone)]
pub struct ExportDefault {
    pub expr: Expr,
}

#[derive(Debug, Clone)]
pub struct ExportAll {
    pub source: Rc<str>,
    pub exported: Option<Rc<str>>,
}

// ---------------------------------------------------------------------------
// Patterns (destructuring)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Pattern {
    Ident(Rc<str>),
    Array {
        elements: Vec<Option<PatternElement>>,
        rest: Option<Box<Pattern>>,
    },
    Object {
        properties: Vec<ObjectPatternProp>,
        rest: Option<Rc<str>>,
    },
    Rest(Box<Pattern>),
    Assignment {
        pattern: Box<Pattern>,
        default: Expr,
    },
    ArrayHole,
}

#[derive(Debug, Clone)]
pub struct PatternElement {
    pub pattern: Pattern,
}

#[derive(Debug, Clone)]
pub struct ObjectPatternProp {
    pub key: PropertyKey,
    pub computed: bool,
    pub value: Pattern,
    pub shorthand: bool,
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Expr {
    Number(f64),
    BigInt(Rc<str>),
    String(Rc<str>),
    Bool(bool),
    Null,
    Undefined,
    Ident(Rc<str>),
    This,
    Super,
    TemplateLit {
        quasis: Vec<Rc<str>>,
        exprs: Vec<Expr>,
        tag: Option<Box<Expr>>,
    },
    TaggedTemplate {
        tag: Box<Expr>,
        quasis: Vec<Rc<str>>,
        exprs: Vec<Expr>,
    },
    Regex {
        pattern: Rc<str>,
        flags: Rc<str>,
    },
    Array(Vec<ArrayElement>),
    Object(Vec<ObjectProp>),
    Function(FunctionExpr),
    Arrow(FunctionExpr),
    Class(Box<ClassDecl>),
    Unary {
        op: UnaryOp,
        arg: Box<Expr>,
    },
    Update {
        op: UpdateOp,
        arg: Box<Expr>,
        prefix: bool,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Logical {
        op: LogicalOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Assignment {
        op: AssignOp,
        left: AssignTarget,
        right: Box<Expr>,
    },
    Conditional {
        test: Box<Expr>,
        cons: Box<Expr>,
        alt: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<CallArg>,
        optional: bool,
    },
    New {
        callee: Box<Expr>,
        args: Vec<CallArg>,
    },
    Member {
        object: Box<Expr>,
        property: MemberProp,
        optional: bool,
    },
    Sequence(Vec<Expr>),
    Spread(Box<Expr>),
    Yield {
        arg: Option<Box<Expr>>,
        delegate: bool,
    },
    Await(Box<Expr>),
    /// Comma in some contexts; kept distinct for clarity.
    Paren(Box<Expr>),
    NewTarget,
    ImportMeta,
    ImportCall(Box<Expr>),
    Empty,
}

#[derive(Debug, Clone)]
pub enum ArrayElement {
    Item(Expr),
    Hole,
    Spread(Expr),
}

#[derive(Debug, Clone)]
pub struct ObjectProp {
    pub key: PropertyKey,
    pub computed: bool,
    pub value: ObjectPropValue,
    pub kind: PropKindAst,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropKindAst {
    Init,
    Get,
    Set,
    Spread,
    Method,
}

#[derive(Debug, Clone)]
pub enum ObjectPropValue {
    Expr(Expr),
    Pattern(Pattern),
}

#[derive(Debug, Clone)]
pub enum MemberProp {
    Ident(Rc<str>),
    Computed(Box<Expr>),
    Private(Rc<str>),
}

#[derive(Debug, Clone)]
pub enum CallArg {
    Expr(Expr),
    Spread(Expr),
}

#[derive(Debug, Clone)]
pub enum AssignTarget {
    Ident(Rc<str>),
    Member {
        object: Box<Expr>,
        property: MemberProp,
    },
    Pattern(Box<Pattern>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Neg,
    Pos,
    Not,
    BitNot,
    TypeOf,
    Void,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpdateOp {
    Inc,
    Dec,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
    Eq,
    NotEq,
    StrictEq,
    StrictNotEq,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    InstanceOf,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogicalOp {
    And,
    Or,
    Nullish,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AssignOp {
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    ExpAssign,
    BitAndAssign,
    BitOrAssign,
    BitXorAssign,
    ShlAssign,
    ShrAssign,
    UShrAssign,
    AndAssign,
    OrAssign,
    NullishAssign,
}

impl Expr {
    /// True if this expression has no side effects (used for `expr ?` short-circuit output).
    pub fn is_pure(&self) -> bool {
        matches!(
            self,
            Expr::Number(_)
                | Expr::String(_)
                | Expr::Bool(_)
                | Expr::Null
                | Expr::Undefined
                | Expr::Ident(_)
                | Expr::This
        )
    }
}

/// Build a block from a single statement (helper).
pub fn block(stmts: Vec<Stmt>) -> Block {
    Block { stmts }
}
