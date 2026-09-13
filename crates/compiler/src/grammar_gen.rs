//! Turns a `crate::grammar_trace::GrammarTrace` -- produced by running
//! the real, current parser over a corpus of real `.nir` programs --
//! into rendered EBNF and GBNF. The mechanical alternative to hand-
//! transliterating `docs/GRAMMAR.md`/`nirdosha.gbnf` from `parser.rs`
//! that both those files' own doc comments say doesn't exist.
//!
//! ## What "mechanical" means here, precisely
//!
//! This does not statically analyze `parser.rs`'s Rust source. It
//! *runs* the instrumented parser (`Parser::new_traced`) over every
//! corpus program and records, per grammar rule (one per `parse_*`
//! method -- see `KNOWN_RULES`), every distinct sequence of terminals/
//! child-rule-references that method actually walked. A rule reached
//! via a `match`/`if` dispatch in code shows up as several distinct
//! observed sequences, rendered as EBNF alternation (`|`). A rule
//! invoked in a loop (e.g. `parse_block`'s repeated `parse_stmt`)
//! shows up as the same item (or short block of items, for a
//! separator-joined list) repeating a variable number of times across
//! different invocations -- folded into `(...)*` by `compress_repeats`,
//! then `subsume` drops any alternative that's just a 0-or-1-rep
//! instance of a `*` some other observed alternative already covers
//! (an empty block and a one-statement block both collapse into the
//! same `"{" stmt* "}"` alternative as a five-statement block would).
//!
//! **The corpus-coverage caveat, stated plainly**: a rule this run
//! never actually walks (dead code, or a shape no corpus program
//! happens to exercise) renders as nothing at all, not as a guess --
//! `Report::uncovered` names every such rule explicitly, the same
//! "disclosed gap, not a hidden one" discipline `crates/grammar_export`'s
//! own README holds itself to for its own, differently-sourced, gaps.
//!
//! **What's still hand-supplied, disclosed rather than pretended
//! generated**: five lexical leaf productions (`int-lit`/`float-lit`/
//! `str-lit`/`ident`/`type-name`) that the trace can never derive on
//! its own -- by the time `Parser::bump` sees a token, the *lexer*
//! already collapsed its characters into one opaque `Tok::Int`/`Tok::
//! Str`/etc.; there is no parser-level rule whose trace could ever
//! reveal what a digit or a quote character looks like. `render_ebnf`/
//! `render_gbnf` append these five verbatim (see `LEXICAL_LEAVES`),
//! matching `nirdosha.gbnf`'s own hand-written versions of the same
//! productions.

use crate::grammar_trace::{GrammarTrace, Item, SharedTrace};
use crate::parser::Parser;
use crate::token::{Lexer, TYPE_NAMES};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::rc::Rc;

/// Every rule name `parser.rs`'s `trace_enter` calls currently use, in
/// file order. Used only to name a rule the corpus never reached
/// (`Report::uncovered`) -- this list going stale (a new `parse_*`
/// method whose `trace_enter` call isn't added here) only weakens that
/// report, it never changes what `render_ebnf`/`render_gbnf` print for
/// a rule that *was* reached, since those iterate the trace's own
/// keys, not this list.
pub const KNOWN_RULES: &[&str] = &[
    "action_block", "action_call", "action_decl", "additive", "assignment", "audited_stmt", "block", "call", "comparison", "dashboard_decl", "effect_annotation", "encode_channel_entries", "enum_decl", "equality", "expr", "field_mask_requires", "field_override", "fn_decl", "if_expr", "kv_entry", "landing_decl", "layout_container_body", "layout_decl", "layout_node", "let_stmt", "logic_and", "logic_or", "match_expr", "module_decl", "multiplicative", "namespace_module_decl", "nfr_annotation", "optional_int_modifier", "optional_transact_slot", "panel_decl", "postfix", "primary", "program", "qualified_name", "requires_annotation", "return_stmt", "screen_decl", "serve_config_decl", "state_decl", "stmt", "struct_decl", "transact_expr", "transact_slot", "transition", "type_param_list", "unary", "unary_inner", "validate_decl", "while_stmt", "workflow_data_block", "workflow_decl", "workspace_decl",
];

/// One `.nir` source item in the corpus, labeled for error reporting.
pub struct CorpusItem {
    pub label: String,
    pub source: String,
}

/// Every shipped `examples/**/*.nir` file plus every entry in
/// `crate::capabilities::compiler_capabilities()` -- real,
/// compiler-verified Nirdosha, not synthetic grammar-shaped text.
/// Reusing `capabilities`'s list also folds in every recipe
/// `paste-anywhere-prompt.md` currently teaches (workflow, transact,
/// screen+serve, ...), since that module already assembles them live
/// from the same doc.
pub fn default_corpus(repo_root: &Path) -> Vec<CorpusItem> {
    let mut items = Vec::new();
    collect_nir_files(&repo_root.join("examples"), &mut items);
    for cap in crate::capabilities::compiler_capabilities() {
        items.push(CorpusItem { label: format!("capabilities::{}", cap.name), source: cap.source });
    }
    items
}

fn collect_nir_files(dir: &Path, out: &mut Vec<CorpusItem>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort(); // deterministic trace order -> deterministic output
    for path in paths {
        if path.is_dir() {
            collect_nir_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "nir") {
            match std::fs::read_to_string(&path) {
                Ok(source) => out.push(CorpusItem { label: path.display().to_string(), source }),
                Err(e) => eprintln!("grammar_gen: skipping {}: {e}", path.display()),
            }
        }
    }
}

/// Runs every corpus item through the real lexer and a traced
/// `Parser`, accumulating one shared `GrammarTrace`. A corpus item
/// that fails to lex/parse loses only its own contribution to the
/// trace -- collected in the returned `Vec`, not fatal, so one broken
/// fixture doesn't blank out the whole run.
pub fn trace_corpus(corpus: &[CorpusItem]) -> (GrammarTrace, Vec<(String, String)>) {
    let trace: SharedTrace = Rc::new(RefCell::new(GrammarTrace::default()));
    let mut errors = Vec::new();
    for item in corpus {
        match Lexer::new(&item.source).tokenize() {
            Ok(toks) => {
                let mut parser = Parser::new_traced(toks, trace.clone());
                if let Err(e) = parser.parse_program() {
                    errors.push((item.label.clone(), format!("{e:?}")));
                }
            }
            Err(e) => errors.push((item.label.clone(), format!("{e:?}"))),
        }
    }
    let trace = Rc::try_unwrap(trace)
        .unwrap_or_else(|_| panic!("every traced Parser should have gone out of scope by the end of trace_corpus"))
        .into_inner();
    (trace, errors)
}

// ---------------------------------------------------------------------
// The generalized-sequence IR and the fold/subsume pipeline that turns
// a rule's raw observed sequences into a minimal-ish set of EBNF
// alternatives.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Node {
    Term(String),
    NonTerm(&'static str),
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Star(Box<Node>),
}

fn item_to_node(item: &Item) -> Node {
    match item {
        Item::Terminal(s) => Node::Term(s.clone()),
        Item::NonTerminal(n) => Node::NonTerm(n),
    }
}

/// The longest single repeated block `compress_repeats` will look
/// for. A `struct`/`enum` field entry (`ident ":" type field_mask_
/// requires ","`) is 5 items; 8 leaves headroom for the widest list
/// item this grammar actually has without the search blowing up (this
/// is `O(items x period)` per position, trivial at either size).
const MAX_REPEAT_PERIOD: usize = 8;

/// Folds maximal contiguous runs (period 1..=`MAX_REPEAT_PERIOD`,
/// repeated >=2 times) of one raw observed sequence into `Star` nodes
/// -- e.g. three consecutive `NonTerminal("stmt")` items become one
/// `Star(NonTerm("stmt"))`; a comma-separated list's `field "," field
/// "," field` becomes `(field ",")* field` (an equivalent, if
/// differently associated, EBNF idiom for the same list shape).
fn compress_repeats(items: &[Item]) -> Vec<Node> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let max_period = (items.len() - i) / 2;
        let mut best: Option<(usize, usize)> = None; // (period, total_len)
        for period in 1..=max_period.min(MAX_REPEAT_PERIOD) {
            let block = &items[i..i + period];
            let mut count = 1usize;
            let mut j = i + period;
            while j + period <= items.len() && items[j..j + period] == *block {
                count += 1;
                j += period;
            }
            if count >= 2 {
                let total = period * count;
                if best.is_none_or(|(_, bt)| total > bt) {
                    best = Some((period, total));
                }
            }
        }
        if let Some((period, total)) = best {
            let block: Vec<Node> = items[i..i + period].iter().map(item_to_node).collect();
            let inner = if block.len() == 1 { block.into_iter().next().unwrap() } else { Node::Seq(block) };
            out.push(Node::Star(Box::new(inner)));
            i += total;
        } else {
            out.push(item_to_node(&items[i]));
            i += 1;
        }
    }
    out
}

/// Does `candidate` (a plain, non-generalized sequence) fit inside
/// `general` (which may contain `Star` nodes) by choosing 0 or 1
/// repetitions for each `Star`? Used to drop e.g. an empty-block or
/// one-statement-block alternative once another observed alternative
/// already generalizes it via a `Star`. Not full language containment
/// (a `Star` here only ever absorbs 0 or 1 reps of `candidate`'s own
/// items, never re-splits a multi-rep candidate) -- sufficient for
/// what this corpus actually produces, since any candidate with 2+
/// real repetitions was already folded into its own `Star` by
/// `compress_repeats` before this ever runs.
fn covered_by(candidate: &[Node], general: &[Node]) -> bool {
    match (candidate.split_first(), general.split_first()) {
        (None, None) => true,
        (None, Some((Node::Star(_), grest))) => covered_by(candidate, grest),
        (None, _) => false,
        (Some((c0, crest)), Some((Node::Star(inner), grest))) => {
            covered_by(candidate, grest) || (*c0 == **inner && covered_by(crest, general))
        }
        (Some((c0, crest)), Some((g0, grest))) => c0 == g0 && covered_by(crest, grest),
        (Some(_), None) => false,
    }
}

/// One rule's final, rendered right-hand side: the distinct observed
/// (and folded) sequences, minus any exactly subsumed by another.
fn build_rule_node(sequences: &[Vec<Item>]) -> Node {
    let mut compressed: Vec<Vec<Node>> = sequences.iter().map(|s| compress_repeats(s)).collect();
    compressed.sort();
    compressed.dedup();
    let survivors: Vec<Vec<Node>> = compressed
        .iter()
        .filter(|cand| !compressed.iter().any(|other| *other != **cand && covered_by(cand, other)))
        .cloned()
        .collect();
    let mut branches: Vec<Node> = survivors.into_iter().map(Node::Seq).collect();
    branches.sort();
    branches.dedup();
    if branches.len() == 1 {
        branches.into_iter().next().unwrap()
    } else {
        Node::Alt(branches)
    }
}

/// The five lexical-leaf productions the trace can never derive (see
/// this module's own doc comment) -- kept in one place, identical
/// spelling in both EBNF and GBNF since both grammars use the same
/// character-class syntax for these. Sourced from `token::TYPE_NAMES`
/// for `type-name` so it can't silently drift from the lexer's own
/// reserved-type-name list the way a second hand-copied list could.
fn lexical_leaves() -> String {
    let type_name_alts = TYPE_NAMES.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(" | ");
    format!(
        "int-lit    ::= [0-9]+\n\
         float-lit  ::= [0-9]+ \".\" [0-9]+\n\
         str-lit    ::= \"\\\"\" str-char* \"\\\"\"\n\
         str-char   ::= [^\"\\\\] | \"\\\\\" [\"\\\\ntr]\n\
         ident      ::= [a-zA-Z_] [a-zA-Z0-9_]*\n\
         type-name  ::= {type_name_alts}\n"
    )
}

fn class_ref(term: &str) -> &str {
    match term {
        "INT" => "int-lit",
        "FLOAT" => "float-lit",
        "STR" => "str-lit",
        "IDENT" => "ident",
        "TYPE_NAME" => "type-name",
        "EOF" => "\"<EOF>\"",
        other => other,
    }
}

fn render_node(node: &Node) -> String {
    match node {
        Node::Term(s) => class_ref(s).to_string(),
        Node::NonTerm(n) => n.to_string(),
        Node::Seq(items) if items.is_empty() => "\u{03b5}".to_string(), // epsilon: this alternative is empty
        Node::Seq(items) => items.iter().map(render_atom).collect::<Vec<_>>().join(" "),
        Node::Alt(branches) => branches.iter().map(render_node).collect::<Vec<_>>().join("\n    | "),
        Node::Star(inner) => format!("({})*", render_node(inner)),
    }
}

/// Like `render_node`, but wraps a `Seq`/`Alt` child in parens -- used
/// for a `Seq`'s own items, where an unparenthesized nested `Alt`
/// would silently change precedence.
fn render_atom(node: &Node) -> String {
    match node {
        Node::Seq(_) | Node::Alt(_) => format!("({})", render_node(node)),
        _ => render_node(node),
    }
}

/// What one `generate` run found: the rendered grammar in both
/// formats, plus the corpus-coverage disclosure `KNOWN_RULES`'s doc
/// comment promises.
pub struct Report {
    pub ebnf: String,
    pub gbnf: String,
    pub covered: Vec<&'static str>,
    pub uncovered: Vec<&'static str>,
    pub parse_errors: Vec<(String, String)>,
}

/// The end-to-end pipeline: trace `corpus`, fold+render every reached
/// rule, and report what wasn't reached at all.
pub fn generate(corpus: &[CorpusItem]) -> Report {
    let (trace, parse_errors) = trace_corpus(corpus);
    let mut rules: BTreeMap<&'static str, Node> = BTreeMap::new();
    for (rule, sequences) in &trace.sequences {
        rules.insert(rule, build_rule_node(sequences));
    }
    let covered_set: HashSet<&'static str> = rules.keys().copied().collect();
    let covered: Vec<&'static str> = rules.keys().copied().collect();
    let uncovered: Vec<&'static str> = KNOWN_RULES.iter().copied().filter(|r| !covered_set.contains(r)).collect();

    let header = "# Mechanically generated by `nirdosha::grammar_gen` -- run the real,\n\
                  # current parser over crates/compiler's example corpus and trace which\n\
                  # productions actually fired (see grammar_gen.rs's own doc comment for\n\
                  # exactly what \"mechanical\" means and what it doesn't cover). Do not\n\
                  # hand-edit -- regenerate instead (`nirdosha grammar-export`).\n\n";
    let mut ebnf = String::from(header);
    for (name, node) in &rules {
        ebnf.push_str(&format!("{name} ::= {}\n", render_node(node)));
    }
    ebnf.push('\n');
    ebnf.push_str(&lexical_leaves());

    let mut gbnf = String::from(header);
    gbnf.push_str("root ::= program\n\n");
    for (name, node) in &rules {
        gbnf.push_str(&format!("{name} ::= {}\n", render_node(node)));
    }
    gbnf.push('\n');
    gbnf.push_str(&lexical_leaves());

    Report { ebnf, gbnf, covered, uncovered, parse_errors }
}
