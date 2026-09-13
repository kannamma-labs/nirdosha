//! Runtime trace hooks `parser.rs` calls into when grammar tracing is
//! enabled (`Parser::new_traced`) — pure bookkeeping, inert unless a
//! caller opts in, with zero effect on parsing behavior either way.
//! `crate::grammar_gen` owns turning a completed trace (run over a
//! whole corpus of real `.nir` programs) into rendered EBNF/GBNF; this
//! module only owns collecting one, from the one true source of what
//! the grammar accepts: the parser actually running.
//!
//! **The mechanism**: every `parse_*` method starts with `let _g =
//! self.trace_enter("rule_name");` (a one-line, purely additive
//! insertion — see `parser.rs`'s own methods). That pushes a frame
//! recording which grammar rule is now active; the frame's `Vec<Item>`
//! accumulates, in order, every terminal token `Parser::bump` actually
//! consumes while this rule is on top of the stack, and every child
//! rule it calls into (recorded as a `NonTerminal` item in the
//! *parent* frame at the exact position the call happened, so
//! `enum ::= X (Y | Z)` shaped dispatch and `struct ::= "{" field*
//! "}"`-shaped repetition both fall out of the trace's literal shape,
//! not a guess about it). The frame closes (`TraceGuard::drop`) when
//! the `parse_*` call returns, successfully or via `?` — RAII, so a
//! bailout mid-rule still closes its frame correctly, and its exact
//! survived items so far are irrelevant, at the point of a real
//! parse it always matched a real grammar rule the exhaustive way.

use crate::token::Tok;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// One element of a rule invocation's observed sequence: either a
/// terminal token (labeled by `Tok::grammar_terminal_label`, so two
/// occurrences of the same *kind* of token — e.g. two different
/// integer literals — collapse to the same grammar terminal) or a
/// reference to another rule by name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Item {
    Terminal(String),
    NonTerminal(&'static str),
}

/// One shared trace, accumulated across every program a caller feeds
/// through a traced `Parser` — `crate::grammar_gen`'s corpus runner
/// creates one `GrammarTrace` and reuses it (via a fresh traced
/// `Parser` per program, same `Rc`) for the whole corpus, so
/// `sequences` ends up holding every real invocation of every rule
/// across every corpus program, not just one.
#[derive(Default)]
pub struct GrammarTrace {
    /// Currently-open `parse_*` calls, innermost (most recently
    /// entered) last: the rule name and the sequence of `Item`s
    /// observed so far during *this* invocation.
    stack: Vec<(&'static str, Vec<Item>)>,
    /// Every completed invocation's full sequence, keyed by rule name.
    pub sequences: HashMap<&'static str, Vec<Vec<Item>>>,
}

impl GrammarTrace {
    fn enter(&mut self, rule: &'static str) {
        if let Some((_, seq)) = self.stack.last_mut() {
            seq.push(Item::NonTerminal(rule));
        }
        self.stack.push((rule, Vec::new()));
    }

    fn exit(&mut self) {
        if let Some((rule, seq)) = self.stack.pop() {
            self.sequences.entry(rule).or_default().push(seq);
        }
    }

    /// Called from `Parser::bump` for every token it actually
    /// consumes -- the single choke point every terminal in the
    /// grammar passes through (see `parser.rs`'s own doc comment: no
    /// `parse_*` function ever advances `pos` except through `bump`).
    pub fn record_token(&mut self, tok: &Tok) {
        if let Some((_, seq)) = self.stack.last_mut() {
            seq.push(Item::Terminal(tok.grammar_terminal_label()));
        }
    }
}

/// A `GrammarTrace` shared between the corpus runner and every
/// `Parser` it hands the same underlying trace to.
pub type SharedTrace = Rc<RefCell<GrammarTrace>>;

/// RAII guard returned by `Parser::trace_enter` — closes the rule's
/// frame on drop, including on an early `?`-return, the same reason
/// any RAII guard exists. Inert (`None`) when tracing isn't enabled,
/// so the untraced path (every real `nirdosha build`) is one extra
/// `Option`-sized local per `parse_*` call and nothing else.
pub struct TraceGuard(Option<SharedTrace>);

impl TraceGuard {
    pub fn active(trace: &SharedTrace, rule: &'static str) -> Self {
        trace.borrow_mut().enter(rule);
        TraceGuard(Some(trace.clone()))
    }

    pub fn inactive() -> Self {
        TraceGuard(None)
    }
}

impl Drop for TraceGuard {
    fn drop(&mut self) {
        if let Some(t) = &self.0 {
            t.borrow_mut().exit();
        }
    }
}
