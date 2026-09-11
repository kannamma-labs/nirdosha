//! Tokens and the lexer. Every token carries a `Span` so downstream errors
//! (parser, typeck, codegen) can report structured, machine-checkable
//! positions instead of prose — the diagnostic shape docs/goal.md row 9
//! asks for, started here rather than bolted on later.

// `Hash` is for `refine.rs`, which keys a `HashSet<Span>` of proven-safe
// sites — every other consumer only needed equality/ordering before this.
// `Serialize`/`Deserialize` are for `lib.rs::Diagnostic` — every
// structured diagnostic (docs/goal.md row 9) carries a `Span`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Span {
    pub line: usize,
    pub col: usize,
    /// Byte offset of this span's first byte into the source file,
    /// `0`-based. Added for `nirdosha fix` (byte-offset `FixPatch`es,
    /// `docs/PUBLIC_ROADMAP.md`'s master-plan Sprint 1 item) -- `line`/
    /// `col` are for human-readable diagnostics, `byte` is for a patch
    /// applier that needs to slice and splice the original source text
    /// exactly, the same reason `rustc`'s own `Span` is byte-addressed
    /// (`BytePos`), not line/column-addressed. Real for every span the
    /// lexer mints (`Lexer::span`, the source of every real `Span` in
    /// this compiler); `0` for compiler-synthesized spans that were
    /// never a real range in the original source (an implicit `main`
    /// call, a desugared default) -- those never need a patch applied
    /// against them, so a placeholder is honest, not a lie.
    pub byte: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    // literals & identifiers
    Int(i64),
    /// A decimal float literal (`3.14`) — digits, a `.`, digits, no
    /// scientific notation. Lexed as its own token, not `Int` followed by
    /// `Dot` followed by `Int`, so the parser never has to reassemble one
    /// (and never confuses it with, say, a future field-access `.`).
    Float(f64),
    Str(String),
    True,
    False,
    Ident(String),

    // keywords
    Fn,
    Let,
    Return,
    If,
    Else,
    While,
    Box,
    Froze,
    Spawn,
    Join,
    Thread,
    Chan,
    Send,
    Recv,
    Sandbox,
    Stop,
    Connect,
    Listen,
    Accept,
    /// `open(path, mode)` — see `Expr::Open`. A dedicated keyword, not
    /// folded into the generic builtin-call machinery, for the same
    /// reason `connect`/`listen` aren't: its result type (`Ty::File`)
    /// isn't inferable from a plain `ident(args)` call shape, so it needs
    /// its own grammar production the parser can special-case.
    Open,
    /// `effect(...)` — a `fn` declaration's optional effect annotation
    /// (`docs/PROTOLANG_PORT.md`'s "Locked design 1"). The five names inside
    /// the parens (`pure`/`rng`/`io`/`concurrent`/`network`) are
    /// deliberately **not** reserved keywords of their own — matched by
    /// identifier text inside `effect(...)`'s parens only
    /// (`parser.rs::parse_effect_annotation`), the same "keyword only
    /// within one specific syntactic slot" treatment `transact`'s own
    /// slot names (`network`/`verify`/`commit`/...) already get, so
    /// using `io` or `network` as an ordinary variable/function name
    /// elsewhere in a program stays legal.
    Effect,
    /// `requires(...)` — a `fn` declaration's optional privilege
    /// annotation (see `ast::Requirement`). `role`/`claim` inside the
    /// parens are deliberately **not** reserved keywords of their own —
    /// matched by identifier text inside `requires(...)`'s parens only
    /// (`parser.rs::parse_requires_annotation`), the same "keyword only
    /// within one specific syntactic slot" treatment `effect(...)`'s own
    /// names already get.
    Requires,
    /// `nfr(...)` — a `fn` declaration's optional non-functional-
    /// requirements annotation (see `ast::NfrSpec`). `latency_ms`/
    /// `error_rate_max`/`throughput_min_per_sec`/`concurrency_max` inside
    /// the parens are deliberately **not** reserved keywords of their
    /// own — matched by identifier text inside `nfr(...)`'s parens only
    /// (`parser.rs::parse_nfr_annotation`), the same "keyword only within
    /// one specific syntactic slot" treatment `effect(...)`/`requires(...)`'s
    /// own names already get.
    Nfr,
    /// `acquire name(proof)` — turns a `requires`-gated function into a
    /// first-class, callable value once `proof` (a `RoleView`/`ClaimView`)
    /// is checked against the requirement (`Expr::Acquire`). Shaped like
    /// `spawn`/`sandbox`: a keyword, then a plain call, destructured.
    Acquire,
    Audited,
    Transact,
    /// Row 11 (`docs/nirdosha_row11_amendment.md`) — `struct`/`enum`
    /// declarations and `match` expressions.
    Struct,
    Enum,
    Match,
    /// `screen <StructName> { ... }` — explicit, typechecked UI
    /// authoring for one struct, layered on top of (never replacing)
    /// `ui_gen.rs`'s pure convention-based inference for any struct with
    /// no `screen` block. A real reserved keyword (unlike `field`/
    /// `action`/`paginate`, matched by identifier text only *inside* a
    /// `screen`/`dashboard` body — the same "keyword only within one
    /// specific syntactic slot" treatment `effect(...)`/`transact`'s own
    /// inner names already get, and necessary here: `action` is already
    /// a real struct field/param name in `examples/trade-finance/
    /// trade_finance.nir`).
    Screen,
    /// `dashboard { tile "..." -> stat_fn  chart "..." -> chart_fn }` —
    /// supplements `ui_gen.rs`'s `stat_`/`chart_` naming-convention
    /// inference; see `Screen`'s doc comment for the same reserved-vs-
    /// contextual reasoning.
    Dashboard,
    /// `landing { role("admin") -> AdminScreen  default -> HomeScreen }`
    /// (`rfcs/0010-landing-and-serve-exposure.md`) — a real reserved
    /// keyword, same "no existing example could have used this as an
    /// identifier for something else" reasoning `Dashboard`/`Screen`
    /// already use, since `landing` names the per-role default-screen
    /// block a top-level item, not a contextual field inside one.
    /// `role`/`claim`/`default` *inside* the block are plain idents
    /// matched by string (`parse_landing_decl`), same as `tile`/`chart`/
    /// `visual` already are inside `dashboard { ... }` — only the block
    /// introducer itself needs reserving.
    Landing,
    /// `serve { expose fn_a, fn_b, ... }`
    /// (`rfcs/0010-landing-and-serve-exposure.md`) — the compiled-`serve`
    /// config section. A real reserved keyword for the same reason
    /// `Landing` just above is: it names a top-level block, not a
    /// contextual field. `expose` itself, inside the block, is a plain
    /// ident matched by string (`parse_serve_config_decl`), same
    /// contextual-keyword treatment as `role`/`claim`/`default` inside
    /// `landing { ... }`.
    Serve,
    /// `module "Display Name" { fn ... struct ... enum ... }` — pure
    /// nav-grouping sugar for `ui_gen.rs` (`docs/GRAMMAR.md`'s `module_decl`),
    /// not a real scoping/namespace construct: every declaration inside
    /// still registers into the exact same flat global namespace as a
    /// top-level one, just tagged with this module's display name. A
    /// real reserved keyword (like `Struct`/`Enum`/`Screen`/`Dashboard`
    /// above, unlike `field`/`action`/`paginate`'s contextual-only
    /// treatment) since no existing example uses "module" as an
    /// identifier.
    Module,
    /// `workflow Name { data { ... } state ... }` — top-level durable
    /// state-machine declaration (`docs/WORKFLOW.md`). A real reserved keyword
    /// like `Struct`/`Enum`/`Screen`/`Dashboard`/`Module` above; its body
    /// keywords (`data`/`on_entry`/`on_exit`/`on`/`terminal`/`link`) stay
    /// contextual-only (matched by identifier text inside
    /// `parser.rs::parse_workflow_decl` only), same "keyword only within
    /// one specific syntactic slot" treatment `transact`'s slot names and
    /// `screen`/`dashboard`'s inner names already get.
    Workflow,
    /// `state Name { ... }` — one state inside a `workflow` block. Real
    /// reserved keyword (unlike its own body's `on_entry`/`on_exit`/`on`,
    /// which are contextual) since `state` isn't otherwise used as an
    /// identifier anywhere in the existing examples, matching `Screen`'s
    /// own reserved-vs-contextual reasoning.
    State,
    /// `workspace Name { subject: Struct panel "..." { ... } }` — a
    /// composite multi-panel screen (`docs/ROADMAP.md` Track E1,
    /// `examples/ctms/UI_CONSTRUCTS.md` §1). A real reserved keyword,
    /// same reasoning as `Screen`/`Dashboard`/`Module`/`Workflow` above;
    /// its body's `panel` is contextual-only (matched by identifier text
    /// only inside `parser.rs::parse_workspace_decl`), the same
    /// "keyword only within one specific syntactic slot" treatment
    /// `screen`'s own `field`/`action`/`paginate` already get — disam-
    /// biguated from an ordinary `kv_entry` by its second token always
    /// being a `string`, never a `:`, so LL(1) holds with no
    /// second-token lookahead beyond that same one-token check.
    Workspace,
    /// `validate <fn_name> { pre: <expr>  post: <expr> ... }` — a Hoare
    /// contract on an existing fn (`docs/ROADMAP.md` Track F, F3;
    /// `docs/NEXT_GEN.md` §F3). A real reserved keyword, same reasoning as
    /// `Screen`/`Dashboard`/`Module`/`Workflow`/`Workspace` above (no
    /// existing example uses "validate" as an identifier); its body's
    /// `pre`/`post` are contextual-only `kv_entry` keys, the same
    /// "keyword only within one specific syntactic slot" treatment
    /// `screen`'s own `field`/`action`/`paginate` already get.
    Validate,
    /// `pub` — marks a declaration inside a real (identifier-named)
    /// `module Ident { ... }` block visible outside that module
    /// (`docs/ROADMAP.md` Track F, F2; `docs/NEXT_GEN.md` §F2). Meaningless
    /// (accepted, no effect) on a top-level or legacy string-named
    /// `module "Display Name" { ... }` declaration — those are always
    /// effectively public already, unchanged from before this existed.
    Pub,
    /// `use "relative/path.nir"` — a leading-only top-level item that
    /// imports another file's `pub` real-namespace declarations
    /// (`docs/ROADMAP.md` Track F, F2 piece 3). Only legal in the leading
    /// run at the very start of a program, before any other item.
    Use,
    /// `Vector`/`Matrix` in *type* position (`Vector(f64, 3)`) — deliberately
    /// capitalized, distinct from the lowercase `TypeName` scalars, matching
    /// the surface syntax the unified plan's architecture table already
    /// uses. Recognized by exact case-sensitive spelling in the identifier
    /// scanner (below), not folded into `TYPE_NAMES` since these two take
    /// `(...)` arguments — a genuinely different production, not another
    /// bare keyword.
    VectorKw,
    MatrixKw,
    /// `handle(KindName)` — `Ty::Handle`'s own syntax, the same
    /// `Name(args)` shape `Vector`/`Matrix` already use (a bare
    /// identifier argument, not a comma list, since a handle kind is
    /// nominal, not parameterized). Lowercase, unlike `Vector`/`Matrix`,
    /// since it names a *kind* of value (`box`/`thread`/`chan`/`sandbox`
    /// are all lowercase type-formers too), not a capitalized
    /// user-facing generic type.
    HandleKw,
    TypeName(String), // i8/i16/.../usize/f64/bool/unit/str — validated by the parser

    // symbols
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    Comma,
    /// `expr.field` — Row 11 field access (`Expr::FieldAccess`). Not
    /// produced by digit-scanning (a float literal's own `.` is a
    /// separate, earlier lexer branch — see `Lexer::tokenize`'s digit
    /// case), only by this literal one-character symbol.
    Dot,
    Arrow,    // ->
    FatArrow, // => (a `match` arm's separator)
    Assign,
    Plus,
    Minus,
    Star,
    Slash,
    /// `%` — truncating remainder (`ast::BinOp::Rem`), same precedence
    /// slot as `Star`/`Slash`.
    Percent,
    EqEq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    AndAnd,
    OrOr,
    Bang,
    Amp,
    DotStar,  // .*
    DotSlash, // ./
    /// `::` — qualified-name path separator (`Mod::Name`). Lexed as one
    /// token, not two `Colon`s, so `expr : Type` (an eventual annotation
    /// slot, none exists today) and `Mod::Name` can never be confused
    /// downstream — see `Lexer::tokenize`'s two-char dispatch, which
    /// checks this before falling back to a single `Colon`.
    ColonColon,

    Eof,
}

/// What a diagnostic should print for a token — its own source text,
/// never the derived `Debug` name. A real, root-caused `hi` :generate
/// failure (the same class RFC 0012's `main`-less prompt fix was, one
/// run later): a confirmed-candidates generate pass produced a struct
/// field named `state`, the parser answered `expected identifier,
/// found State` — `Tok`'s *Rust variant name*, because every "found ..."
/// site formatted the token with `{:?}` — and the bounded self-repair
/// loop in `hi_llm.rs` fed that back to the model verbatim. The model
/// had no way to know `State` meant the lowercase keyword `state` it
/// had written (its own type names were `GameState`-shaped, and
/// "State" plausibly *looked* like one of them), so it "fixed"
/// something else and failed identically on all 3 attempts. That is
/// exactly the row-9 failure mode `docs/goal.md` spells out — an
/// agent's self-repair loop only works where the compiler can say
/// something structured back — so this `impl` renders every token as
/// the text the programmer actually typed, and reserved words say so
/// outright ("the reserved keyword `state`"): the complete repair
/// instruction, not just the fact of failure. Kept on `Tok` rather than
/// patched into one parser site because all fifteen "found ..."
/// messages in `parser.rs` leak the same internals.
///
/// Deliberately one big exhaustive match with no catch-all arm: adding
/// a `Tok` variant is a compile error here until it gets a rendering,
/// so this mapping can never silently drift from the enum the way a
/// `_ =>` arm would let it. The keyword spellings below must match
/// `Lexer::tokenize`'s identifier-case match (the same table written
/// in the opposite direction) — `token.rs`'s own tests round-trip every
/// one of them to keep that honest.
impl std::fmt::Display for Tok {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tok::Int(n) => write!(f, "`{n}`"),
            // `{:?}` (not `{}`) on the f64 so `3.0` stays visibly a
            // float, not the integer `3` — the same reason the lexer
            // keeps them distinct tokens in the first place.
            Tok::Float(x) => write!(f, "`{x:?}`"),
            Tok::Str(s) => write!(f, "`\"{s}\"`"),
            Tok::Ident(s) => write!(f, "`{s}`"),
            // `i64`/`str`/`db`/... — reserved like the keywords below
            // (the lexer makes them their own `TypeName` token, so they
            // can never be an identifier either), flagged as type names
            // because a bare "found `str`" would read as a perfectly
            // legal identifier to a repairing agent that doesn't know
            // better.
            Tok::TypeName(s) => write!(f, "the reserved type name `{s}`"),
            Tok::LParen => write!(f, "`(`"),
            Tok::RParen => write!(f, "`)`"),
            Tok::LBrace => write!(f, "`{{`"),
            Tok::RBrace => write!(f, "`}}`"),
            Tok::LBracket => write!(f, "`[`"),
            Tok::RBracket => write!(f, "`]`"),
            Tok::Colon => write!(f, "`:`"),
            Tok::Comma => write!(f, "`,`"),
            Tok::Dot => write!(f, "`.`"),
            Tok::Arrow => write!(f, "`->`"),
            Tok::FatArrow => write!(f, "`=>`"),
            Tok::Assign => write!(f, "`=`"),
            Tok::Plus => write!(f, "`+`"),
            Tok::Minus => write!(f, "`-`"),
            Tok::Star => write!(f, "`*`"),
            Tok::Slash => write!(f, "`/`"),
            Tok::Percent => write!(f, "`%`"),
            Tok::EqEq => write!(f, "`==`"),
            Tok::NotEq => write!(f, "`!=`"),
            Tok::Lt => write!(f, "`<`"),
            Tok::Gt => write!(f, "`>`"),
            Tok::LtEq => write!(f, "`<=`"),
            Tok::GtEq => write!(f, "`>=`"),
            Tok::AndAnd => write!(f, "`&&`"),
            Tok::OrOr => write!(f, "`||`"),
            Tok::Bang => write!(f, "`!`"),
            Tok::Amp => write!(f, "`&`"),
            Tok::DotStar => write!(f, "`.*`"),
            Tok::DotSlash => write!(f, "`./`"),
            Tok::ColonColon => write!(f, "`::`"),
            Tok::Eof => write!(f, "end of file"),
            Tok::Fn => write!(f, "the reserved keyword `fn`"),
            Tok::Let => write!(f, "the reserved keyword `let`"),
            Tok::Return => write!(f, "the reserved keyword `return`"),
            Tok::If => write!(f, "the reserved keyword `if`"),
            Tok::Else => write!(f, "the reserved keyword `else`"),
            Tok::While => write!(f, "the reserved keyword `while`"),
            Tok::Box => write!(f, "the reserved keyword `box`"),
            Tok::Froze => write!(f, "the reserved keyword `froze`"),
            Tok::Spawn => write!(f, "the reserved keyword `spawn`"),
            Tok::Join => write!(f, "the reserved keyword `join`"),
            Tok::Thread => write!(f, "the reserved keyword `thread`"),
            Tok::Chan => write!(f, "the reserved keyword `chan`"),
            Tok::Send => write!(f, "the reserved keyword `send`"),
            Tok::Recv => write!(f, "the reserved keyword `recv`"),
            Tok::Sandbox => write!(f, "the reserved keyword `sandbox`"),
            Tok::Stop => write!(f, "the reserved keyword `stop`"),
            Tok::Connect => write!(f, "the reserved keyword `connect`"),
            Tok::Listen => write!(f, "the reserved keyword `listen`"),
            Tok::Accept => write!(f, "the reserved keyword `accept`"),
            Tok::Open => write!(f, "the reserved keyword `open`"),
            Tok::Effect => write!(f, "the reserved keyword `effect`"),
            Tok::Requires => write!(f, "the reserved keyword `requires`"),
            Tok::Nfr => write!(f, "the reserved keyword `nfr`"),
            Tok::Acquire => write!(f, "the reserved keyword `acquire`"),
            Tok::Audited => write!(f, "the reserved keyword `audited`"),
            Tok::Transact => write!(f, "the reserved keyword `transact`"),
            Tok::Struct => write!(f, "the reserved keyword `struct`"),
            Tok::Enum => write!(f, "the reserved keyword `enum`"),
            Tok::Match => write!(f, "the reserved keyword `match`"),
            Tok::Screen => write!(f, "the reserved keyword `screen`"),
            Tok::Dashboard => write!(f, "the reserved keyword `dashboard`"),
            Tok::Landing => write!(f, "the reserved keyword `landing`"),
            Tok::Serve => write!(f, "the reserved keyword `serve`"),
            Tok::Module => write!(f, "the reserved keyword `module`"),
            Tok::Workflow => write!(f, "the reserved keyword `workflow`"),
            Tok::State => write!(f, "the reserved keyword `state`"),
            Tok::Workspace => write!(f, "the reserved keyword `workspace`"),
            Tok::Validate => write!(f, "the reserved keyword `validate`"),
            Tok::Pub => write!(f, "the reserved keyword `pub`"),
            Tok::Use => write!(f, "the reserved keyword `use`"),
            Tok::VectorKw => write!(f, "the reserved keyword `Vector`"),
            Tok::MatrixKw => write!(f, "the reserved keyword `Matrix`"),
            Tok::HandleKw => write!(f, "the reserved keyword `handle`"),
            Tok::True => write!(f, "the reserved keyword `true`"),
            Tok::False => write!(f, "the reserved keyword `false`"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

const TYPE_NAMES: &[&str] = &[
    "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "usize", "f64", "dec128", "bool",
    "unit", "str", "tcp", "tcp_listener", "file", "json", "db", "mq",
];

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LexError {
    pub message: String,
    pub span: Span,
}

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer { src: src.as_bytes(), pos: 0, line: 1, col: 1 }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<u8> {
        self.src.get(self.pos + 1).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn span(&self) -> Span {
        Span { line: self.line, col: self.col, byte: self.pos }
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n') => {
                    self.bump();
                }
                Some(b'/') if self.peek2() == Some(b'/') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                _ => break,
            }
        }
    }

    /// Produce the whole token stream in one pass. Single-token-lookahead
    /// parsing downstream never needs to re-enter the lexer mid-stream.
    pub fn tokenize(mut self) -> Result<Vec<Token>, LexError> {
        let mut out = Vec::new();
        loop {
            self.skip_ws_and_comments();
            let span = self.span();
            let c = match self.peek() {
                None => {
                    out.push(Token { tok: Tok::Eof, span });
                    break;
                }
                Some(c) => c,
            };

            if c == b'"' {
                self.bump(); // opening quote
                // Raw bytes, not `char`-by-`char` -- this lexer works over
                // `&[u8]` throughout (see `src`'s type), and pushing bytes
                // one at a time as `as char` would silently corrupt any
                // multi-byte UTF-8 content (each continuation byte would
                // become its own bogus Latin-1 codepoint). Collecting raw
                // bytes and validating the whole run as UTF-8 once, at the
                // end, is the only correct way to do this byte-wise.
                let mut bytes = Vec::new();
                loop {
                    match self.bump() {
                        None => {
                            return Err(LexError { message: "unterminated string literal".to_string(), span });
                        }
                        Some(b'"') => break,
                        // Minimal, deliberately small escape set -- just
                        // enough to write a quote, a backslash, a
                        // newline/tab/carriage-return (`\r\n` matters for
                        // real reasons -- e.g. HTTP request lines over
                        // `tcp` genuinely need it, not a hypothetical);
                        // not a full escape grammar (no \u{...}, no \x..,
                        // no \0). Nothing needing those exists yet.
                        Some(b'\\') => match self.bump() {
                            Some(b'"') => bytes.push(b'"'),
                            Some(b'\\') => bytes.push(b'\\'),
                            Some(b'n') => bytes.push(b'\n'),
                            Some(b't') => bytes.push(b'\t'),
                            Some(b'r') => bytes.push(b'\r'),
                            Some(other) => {
                                return Err(LexError {
                                    message: format!("unknown escape `\\{}`", other as char),
                                    span,
                                });
                            }
                            None => {
                                return Err(LexError {
                                    message: "unterminated string literal".to_string(),
                                    span,
                                });
                            }
                        },
                        Some(b) => bytes.push(b),
                    }
                }
                let s = String::from_utf8(bytes)
                    .map_err(|_| LexError { message: "string literal is not valid UTF-8".to_string(), span })?;
                out.push(Token { tok: Tok::Str(s), span });
                continue;
            }

            if c.is_ascii_digit() {
                let start = self.pos;
                while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    self.bump();
                }
                // A `.` immediately followed by a digit turns this into a
                // float literal instead — checked with `peek2`, not just
                // `peek`, so a bare trailing `.` (nothing this language
                // has a use for yet — no method-call/field-access syntax)
                // doesn't get swallowed into a malformed float. `1..` or
                // `1.` alone stays a plain `Int(1)` followed by whatever
                // the `.` actually is (currently: a lex error, same as
                // today — no production accepts a bare `.` yet).
                let is_float = self.peek() == Some(b'.') && self.peek2().map(|c| c.is_ascii_digit()).unwrap_or(false);
                if is_float {
                    self.bump(); // '.'
                    while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        self.bump();
                    }
                    let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
                    let f: f64 = text.parse().map_err(|_| LexError {
                        message: format!("float literal `{text}` is not valid"),
                        span,
                    })?;
                    out.push(Token { tok: Tok::Float(f), span });
                    continue;
                }
                let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
                let n: i64 = text.parse().map_err(|_| LexError {
                    message: format!("integer literal `{text}` out of range"),
                    span,
                })?;
                out.push(Token { tok: Tok::Int(n), span });
                continue;
            }

            if c.is_ascii_alphabetic() || c == b'_' {
                let start = self.pos;
                while self
                    .peek()
                    .map(|c| c.is_ascii_alphanumeric() || c == b'_')
                    .unwrap_or(false)
                {
                    self.bump();
                }
                let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
                let tok = match text {
                    "fn" => Tok::Fn,
                    "let" => Tok::Let,
                    "return" => Tok::Return,
                    "if" => Tok::If,
                    "else" => Tok::Else,
                    "while" => Tok::While,
                    "box" => Tok::Box,
                    "froze" => Tok::Froze,
                    "spawn" => Tok::Spawn,
                    "join" => Tok::Join,
                    "thread" => Tok::Thread,
                    "chan" => Tok::Chan,
                    "send" => Tok::Send,
                    "recv" => Tok::Recv,
                    "sandbox" => Tok::Sandbox,
                    "stop" => Tok::Stop,
                    "connect" => Tok::Connect,
                    "listen" => Tok::Listen,
                    "accept" => Tok::Accept,
                    "open" => Tok::Open,
                    "effect" => Tok::Effect,
                    "requires" => Tok::Requires,
                    "nfr" => Tok::Nfr,
                    "acquire" => Tok::Acquire,
                    "audited" => Tok::Audited,
                    "transact" => Tok::Transact,
                    "struct" => Tok::Struct,
                    "enum" => Tok::Enum,
                    "match" => Tok::Match,
                    "screen" => Tok::Screen,
                    "dashboard" => Tok::Dashboard,
                    "landing" => Tok::Landing,
                    "serve" => Tok::Serve,
                    "module" => Tok::Module,
                    "workflow" => Tok::Workflow,
                    "state" => Tok::State,
                    "workspace" => Tok::Workspace,
                    "validate" => Tok::Validate,
                    "pub" => Tok::Pub,
                    "use" => Tok::Use,
                    "Vector" => Tok::VectorKw,
                    "Matrix" => Tok::MatrixKw,
                    "handle" => Tok::HandleKw,
                    "true" => Tok::True,
                    "false" => Tok::False,
                    t if TYPE_NAMES.contains(&t) => Tok::TypeName(t.to_string()),
                    t => Tok::Ident(t.to_string()),
                };
                out.push(Token { tok, span });
                continue;
            }

            // symbols — check two-char forms before falling back to one-char
            let two = self.peek2();
            let (tok, len) = match (c, two) {
                (b'-', Some(b'>')) => (Tok::Arrow, 2),
                (b'=', Some(b'>')) => (Tok::FatArrow, 2),
                (b'=', Some(b'=')) => (Tok::EqEq, 2),
                (b'!', Some(b'=')) => (Tok::NotEq, 2),
                (b'<', Some(b'=')) => (Tok::LtEq, 2),
                (b'>', Some(b'=')) => (Tok::GtEq, 2),
                (b'&', Some(b'&')) => (Tok::AndAnd, 2),
                (b'|', Some(b'|')) => (Tok::OrOr, 2),
                (b'.', Some(b'*')) => (Tok::DotStar, 2),
                (b'.', Some(b'/')) => (Tok::DotSlash, 2),
                (b':', Some(b':')) => (Tok::ColonColon, 2),
                _ => {
                    let single = match c {
                        b'(' => Tok::LParen,
                        b')' => Tok::RParen,
                        b'{' => Tok::LBrace,
                        b'}' => Tok::RBrace,
                        b'[' => Tok::LBracket,
                        b']' => Tok::RBracket,
                        b':' => Tok::Colon,
                        b',' => Tok::Comma,
                        b'.' => Tok::Dot,
                        b'=' => Tok::Assign,
                        b'+' => Tok::Plus,
                        b'-' => Tok::Minus,
                        b'*' => Tok::Star,
                        b'/' => Tok::Slash,
                        b'%' => Tok::Percent,
                        b'<' => Tok::Lt,
                        b'>' => Tok::Gt,
                        b'!' => Tok::Bang,
                        b'&' => Tok::Amp,
                        other => {
                            return Err(LexError {
                                message: format!(
                                    "unexpected character `{}`",
                                    other as char
                                ),
                                span,
                            })
                        }
                    };
                    (single, 1)
                }
            };
            for _ in 0..len {
                self.bump();
            }
            out.push(Token { tok, span });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every reserved word, exactly as the lexer's identifier-case
    /// match spells it -- the one list both that match and `Display
    /// for Tok` must agree with. The lexer maps `text -> Tok`;
    /// `Display` maps `Tok -> text`; this round-trips one through the
    /// other so the two can never drift apart silently (a mismatched
    /// spelling would render a diagnostic naming a word that isn't in
    /// the language, sending a repairing agent hunting for it).
    const ALL_KEYWORDS: &[&str] = &[
        "fn", "let", "return", "if", "else", "while", "box", "froze", "spawn", "join", "thread", "chan", "send",
        "recv", "sandbox", "stop", "connect", "listen", "accept", "open", "effect", "requires", "nfr", "acquire",
        "audited", "transact", "struct", "enum", "match", "screen", "dashboard", "landing", "serve", "module",
        "workflow", "state", "workspace", "validate", "pub", "use", "Vector", "Matrix", "handle", "true", "false",
    ];

    #[test]
    fn every_keyword_round_trips_lexer_to_display() {
        for &word in ALL_KEYWORDS {
            let toks = Lexer::new(word).tokenize().expect("lexing a lone keyword cannot fail");
            assert_eq!(toks.len(), 2, "`{word}` must lex as one token plus Eof");
            let rendered = toks[0].tok.to_string();
            assert_eq!(rendered, format!("the reserved keyword `{word}`"), "`{word}` must render as its own source text, got: {rendered}");
        }
    }

    #[test]
    fn every_keyword_is_not_an_identifier_token() {
        // The whole point of the reserved-word rendering: each of
        // these lexes to its own keyword token, never `Tok::Ident`, so
        // none of them can ever be a variable/field/function name --
        // which is exactly what `expected identifier, found the
        // reserved keyword ...` reports.
        for &word in ALL_KEYWORDS {
            let toks = Lexer::new(word).tokenize().expect("lexing a lone keyword cannot fail");
            assert!(!matches!(toks[0].tok, Tok::Ident(_)), "`{word}` must not lex as an identifier");
        }
    }

    #[test]
    fn literals_idents_symbols_and_eof_render_as_source_text() {
        let render = |src: &str| -> String {
            let toks = Lexer::new(src).tokenize().expect("lex should succeed");
            toks[0].tok.to_string()
        };
        assert_eq!(render("42"), "`42`");
        assert_eq!(render("3.0"), "`3.0`");
        assert_eq!(render("\"hi\""), "`\"hi\"`");
        assert_eq!(render("some_name"), "`some_name`");
        assert_eq!(render("i64"), "the reserved type name `i64`");
        assert_eq!(render("->"), "`->`");
        assert_eq!(render("::"), "`::`");
        assert_eq!(Lexer::new("").tokenize().expect("empty lex")[0].tok.to_string(), "end of file");
    }
}
