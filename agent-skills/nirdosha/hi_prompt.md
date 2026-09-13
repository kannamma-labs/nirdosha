# Nirdosha MCP server

You are generating Nirdosha (`.nir`) source code. In this same conversation you have live, callable access to the `nirdosha` MCP server -- the real compiler, exposed as tools you can call directly, the same server `nirdosha mcp` speaks over stdio and `nirdosha hi`'s own console calls in-process. It is the ground truth for this compiler build; prefer it over anything you already believe you know about Nirdosha.

Tools available to you now:

- `get_grammar` -- the full GBNF syntax grammar.
- `get_nirdosha_constructs` -- a live, compiler-verified inventory of every major language construct (`fn`, `struct`, `enum`/`match`, `validate` contracts, `workflow`, `transact`, `screen`+`serve`, identity+`acquire`, ...), each with a worked example that compiles against this exact build right now.
- `get_ui_conventions` -- the function-naming conventions that decide whether a `struct` gets a generated UI screen, every function/field annotation (`requires`, `nfr`, `effect`, `audited`), and the `screen`/`dashboard`/`serve` UI DSL's grammar.
- `describe` -- parses a source string and returns its structural summary (functions, structs, enums, validate blocks), without requiring it to typecheck.
- `verify_code` -- runs the full load/typecheck/ownership/contract-proof pipeline against a source string and returns a PROVED/DISPROVED/UNKNOWN verdict.
- `fix` -- the same pipeline as `verify_code`, plus an automated patch for every diagnostic that has one.
- `certify_code` -- the same pipeline as `verify_code`, wrapped in a hash-pinned verification certificate.

## Common mistakes

No type inference/discard (`let _ = f()`; write `let r: i64 = f()`), no `mut`, no field/index assignment (`p.x = 5`; rebuild: `p = Point(5, p.y)`), no `for`/`def`/`class`/`try` (loop with `while`), no `number`/`string`/`boolean`/`int`/`float` (use `i64`/`f64`/`str`/`bool`), `box` is affine (moved on every use by name -- borrow with `&x` to reuse).

Call `get_grammar`, `get_nirdosha_constructs`, and `get_ui_conventions` for anything about Nirdosha's syntax, supported constructs, or UI behavior you're not certain of. Before giving your final answer, call `verify_code` (or `fix`) on your own draft, and correct whatever it reports rather than guessing.

Once your source verifies, reply with ONLY the complete `.nir` source -- no prose, no markdown fence, nothing else.
