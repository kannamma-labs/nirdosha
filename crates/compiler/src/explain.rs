//! `nirdosha explain <code>` -- `nirdosha-master-plan.md` Part 3
//! Sprint 1's "machine-learnable error index" (parity target: Kōdo,
//! Midspiral). A curated, hand-written registry, not a mechanical
//! one-entry-per-`TypeErrorKind`-variant dump: `rustc --explain` is the
//! real precedent for this shape (~130 long-form `E`-code docs against
//! several hundred internal diagnostic sites) -- most diagnostics never
//! get an explain entry, and that's fine, because the ones that do are
//! worth writing well rather than templating badly.
//!
//! Codes are `NIR0001`..`NIR0012`, one per rule in
//! `agent-skills/nirdosha/AGENTS.md`'s "rules that will break your
//! output" section, **numbered to match that section's own numbering,
//! on purpose** -- so a human or an agent cross-referencing "AGENTS.md
//! rule 9" and "NIR0009" never has to look anything up to know they're
//! the same rule. `NIR0013` is the one addition beyond those twelve:
//! the unknown-identifier/typo case `nirdosha fix`'s
//! `fix_unbound_identifier` already has real, tested automated-fix
//! analysis for (`main.rs`), so it earns its own explain entry despite
//! not being one of AGENTS.md's twelve. `NIR` (not `E`, not a bare
//! number) is deliberate: an agent debugging both Rust and Nirdosha in
//! the same repo already sees real `rustc` `E`-codes in the same
//! terminal, and a `NIR`-prefixed code can never be mistaken for one.
//!
//! `code` is auto-attached to a live diagnostic (`VerifyDiagnostic`
//! in `main.rs`) only where a diagnostic's own site can identify the
//! rule with certainty, today: `NIR0002` (`TypeErrorKind::
//! StrInFnSignature`, a real 1:1 match) and `NIR0012` (reserved word
//! used where an identifier was required -- detected from
//! `loader::load_program`'s already-formatted error string, since
//! `ParseError` doesn't carry a structured code field yet; see
//! `classify_load_error_code`'s own doc comment for why that's an
//! honest, bounded heuristic rather than a guess) and `NIR0013`
//! (`TypeErrorKind::UnknownVar`, alongside its own `Fix`). Every other
//! entry below is real, browsable reference material
//! (`nirdosha explain NIR0009` works today) that nothing in the
//! compiler auto-tags onto a diagnostic yet -- an honestly [PARTIAL]
//! boundary, not a silent gap (`docs/PUBLIC_ROADMAP.md`).

pub struct ExplainEntry {
    pub code: &'static str,
    pub title: &'static str,
    pub explanation: &'static str,
    pub wrong: &'static str,
    pub right: &'static str,
}

pub const REGISTRY: &[ExplainEntry] = &[
    ExplainEntry {
        code: "NIR0001",
        title: "Enum variants are flat, unqualified calls",
        explanation: "A variant is constructed by calling its name directly -- `Some(5)`, `None()`, `Circle(r)` -- never `EnumName::Variant(...)`, though the qualified spelling is also accepted as optional disambiguation sugar outside a `module` block. Inside a real `module Ident { ... }` block, the qualified `Mod::Name` form is required for anything declared in it. A zero-payload variant still needs `()` at the call site: `None()`, not bare `None` (a bare variant name is just an identifier to the parser, so `let s: Shape = Circle` is a type error, `unknown variable `Circle``, not a parse error).",
        wrong: "let s: Shape = Circle",
        right: "let s: Shape = Circle(1.0)",
    },
    ExplainEntry {
        code: "NIR0002",
        title: "`str` cannot be a function's parameter or return type",
        explanation: "The ban is checked recursively through `Result`/`Option`/generics/`box`/`&`/`thread`/`chan`/`Vector`/`Matrix`/`fn` types -- `str` anywhere inside a signature's shape is rejected, not just at the top level. `str` is completely fine as a `struct` field, a local `let` binding, or a literal; the restriction is only at `fn` parameter/return position. Use a real `enum` for categorical data (a status, a currency code), or wrap free text in a one-field struct (`struct Text { value: str }`) if it must cross a function boundary.",
        wrong: "fn greet(name: str) -> str {\n    return name\n}",
        right: "struct Text { value: str }\nfn greet(name: Text) -> Text {\n    return name\n}",
    },
    ExplainEntry {
        code: "NIR0003",
        title: "`str` has zero concatenation, zero formatting, zero slicing",
        explanation: "Every string a program produces is either a literal from source or comes back from a builtin (`json_get_str`, `db_query`, `http_get`, ...). There is no `+` for strings and no f-string/format equivalent -- build any dynamic text at the builtin/data layer, not by assembling it in `.nir` source.",
        wrong: "let msg: str = \"hello \" + name",
        right: "// build the string at the source of the data instead (e.g. a db/http\n// builtin's own formatting), or use an enum for the fixed set of\n// messages a program actually needs to distinguish",
    },
    ExplainEntry {
        code: "NIR0004",
        title: "No statement separator -- the parser always extends the current expression",
        explanation: "There are no semicolons and no significant newlines. Wherever a token could extend the current expression or start a new statement, the parser always extends it. `return x` followed on the next line by `-y` parses as one statement, `return (x - y)`, never two. Put unrelated statements on lines that can't be read as a continuation of the previous one -- this bites unary `-` and calls most often.",
        wrong: "return x\n-y",
        right: "let y_negated: i64 = -y\nreturn x + y_negated",
    },
    ExplainEntry {
        code: "NIR0005",
        title: "No `for` loops, no closures/lambdas, no tuples",
        explanation: "Use `while` for iteration. Use a real `struct`/`enum` instead of a tuple. Plain first-class functions exist (`let f: fn(i64) -> i64 = double`) but capture nothing -- there is no enclosing-scope capture at all.",
        wrong: "for i in 0..10 {\n    print(i)\n}",
        right: "let mut_i: i64 = 0\nwhile mut_i < 10 {\n    print(mut_i)\n    mut_i = mut_i + 1\n}",
    },
    ExplainEntry {
        code: "NIR0006",
        title: "No implicit conversions, ever, between two already-typed values",
        explanation: "Not even `i32` + `i64` mix implicitly. An integer literal flexes to fit its declared width (`let n: i8 = 100` needs no cast), but two typed variables never coerce, and there is no int-to-float conversion operator at all.",
        wrong: "let a: i32 = 1\nlet b: i64 = 2\nlet c: i64 = a + b",
        right: "let a: i64 = 1\nlet b: i64 = 2\nlet c: i64 = a + b",
    },
    ExplainEntry {
        code: "NIR0007",
        title: "Construction is an ordinary call, never brace-field syntax",
        explanation: "A `struct`'s name is its own positional constructor: `Product(1, \"Widget\", 999)`, not `Product { id: 1, name: \"Widget\", price: 999 }`. Field order follows declaration order.",
        wrong: "let p: Product = Product { id: 1, name: \"Widget\", price: 999 }",
        right: "let p: Product = Product(1, \"Widget\", 999)",
    },
    ExplainEntry {
        code: "NIR0008",
        title: "`match` is exhaustive, no wildcard binding patterns for variants",
        explanation: "Every enum variant needs its own arm -- coverage is checked by variant, not by value, so a variant arm can never use `_`. A separate literal-pattern form exists for `str`/`i64`/`bool` scrutinees only, and that form requires a trailing `_ =>` wildcard arm instead, since a literal match can never be exhaustive by construction.",
        wrong: "match shape {\n    Circle(r) => 1,\n    _ => 0,\n}",
        right: "match shape {\n    Circle(r) => 1,\n    Square(s) => 0,\n}",
    },
    ExplainEntry {
        code: "NIR0009",
        title: "A `match` arm's body must be a single expression, never a `{ stmt; stmt }` block or `return`",
        explanation: "This is the single most common mistake an LLM makes writing Nirdosha, because the block-arm form is valid in Rust. `Ok(conn) => { let x = f(conn) stop(conn) x }` is a parse error (`expected an expression, found `{``), full stop -- if an arm needs more than one step, extract a small helper function and call it as the arm's single expression instead. The same rule rules out `return` inside a match arm too, since `return` is a statement (`return_stmt`), completely separate from `expr`, and can never appear anywhere an expression is required. `if`/`else` bodies are a genuine exception: those are real multi-statement blocks, and the whole `if`/`else` construct counts as one expression.",
        wrong: "Ok(conn) => {\n    let x: i64 = f(conn)\n    stop(conn)\n    x\n}",
        right: "Ok(conn) => list_product_inner(conn),   // single expression: a call",
    },
    ExplainEntry {
        code: "NIR0010",
        title: "No compound assignment operators",
        explanation: "`total += i` is a parse error (`expected an expression, found `=``) -- there is no `+=`, `-=`, `*=`, `/=` at all. Write the right-hand side out in full instead.",
        wrong: "total += i",
        right: "total = total + i",
    },
    ExplainEntry {
        code: "NIR0011",
        title: "Only `//` line comments exist -- no `/* ... */` block comments",
        explanation: "The lexer doesn't recognize `/*` as the start of anything; a leading `/*` produces a parse error wherever it appears (the parser just sees a stray `/` where a top-level item or expression was expected). Use `//` for every comment, including multi-line ones -- one `//` per line, since there is no multi-line comment syntax at all.",
        wrong: "/* a multi-line\n   comment */",
        right: "// a multi-line\n// comment",
    },
    ExplainEntry {
        code: "NIR0012",
        title: "Reserved words can never be identifiers",
        explanation: "Nirdosha has a fixed reserved-word list, and none of them can be a variable, field, parameter, `fn`, `struct`, `enum`, `screen`, or `module` name -- not a style rule, but a lexer fact: each one lexes as its own keyword token, so the parser sees a keyword where a name is required and stops. Every scalar type name (`i8`..`i64`, `str`, `bool`, `tcp`, `db`, `mq`, ...) is equally unusable as an identifier. The full keyword list: `fn let return if else while box froze spawn join thread chan send recv sandbox stop connect listen accept open effect requires nfr acquire audited transact struct enum match screen dashboard landing serve module workflow state workspace validate pub use Vector Matrix handle true false`. The ones that actually collide with ordinary app vocabulary: `state`, `open`, `serve`, `screen`, `landing`, `handle`, `send`, `recv`, `connect`, `listen`, `accept`, `match`, `use`, `effect`, `stop`. If you need one of those words as a name, pick a synonym (`game_state`, `open_order`, ...) -- there is no quoting or escaping mechanism.",
        wrong: "struct Player { state: str }",
        right: "struct Player { game_state: str }",
    },
    ExplainEntry {
        code: "NIR0013",
        title: "Unknown identifier -- usually a typo of a name actually in scope",
        explanation: "A reference to a name that's neither a local binding, a function parameter, nor a known function -- most often a plain typo (`ammount` for `amount`). Unlike the twelve rules above (`AGENTS.md`'s own numbering), this one has real automated-fix support today: `nirdosha fix <file.nir>` runs an edit-distance check against every name actually in scope and reports one of three outcomes -- an unambiguous single closest candidate is `auto` (apply with `--apply`), a tie between equally-close candidates is `assisted` (names all of them, picks none), and nothing close enough is `manual`.",
        wrong: "fn charge(amount: i64) -> bool {\n    return amount > ammount\n}",
        right: "fn charge(amount: i64) -> bool {\n    return amount > 0\n}",
    },
];

pub fn lookup(code: &str) -> Option<&'static ExplainEntry> {
    let needle = code.trim().to_ascii_uppercase();
    REGISTRY.iter().find(|e| e.code == needle)
}
