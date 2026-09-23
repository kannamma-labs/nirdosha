# UI assurance certificates

Nirdosha projects may declare a finite UI model in `.nir/ui-proof.json`.
`nirdosha-hi publish` loads this declaration and exhaustively checks its
states, actions, roles, postconditions, and reachability before writing the
certificate. The proof is a mathematical proof of the declared finite model;
it is not a claim that an arbitrary browser, backend, or deployment is
correct without runtime evidence.

Minimal example:

```json
{
  "schema": "nirdosha.ui-proof/v1",
  "screens": [{
    "id": "task_list",
    "initial_state": "empty",
    "states": ["empty", "loaded"],
    "actions": [{
      "id": "load",
      "from": "empty",
      "to": "loaded",
      "roles": ["user"],
      "ensures": ["results_rendered"]
    }]
  }]
}
```

For an existing screen register, generate the conservative starting point with:

```bash
cargo nirdosha ui-proof \
  --register examples/rtm/screens.toml \
  --output examples/rtm/.nir/ui-proof.json
```

The generator creates one `observe` self-transition per registered screen,
using the register's lifecycle stage and role vocabulary. This is deliberately
an inventory proof, not fabricated business logic: maintainers must replace or
extend those observations with real actions, postconditions, and browser trace
evidence before treating the result as behavioral assurance.

The authoring shape is defined by
[`UI_PROOF_SPEC_SCHEMA.json`](UI_PROOF_SPEC_SCHEMA.json). TOML is parsed into
the same object shape; the schema is JSON so standard schema validators can
check it before Nirdosha merges it with `screens.toml`.

Every string-valued field and string enum in the schema has a description. The
description is authoring guidance for an LLM and consumption guidance for the
runner—for example, it distinguishes a stable screen ID from a display name,
and a Z3 formula from a human-readable invariant description. Structural
constraints (patterns, enums, hashes, and cross-references) remain normative;
descriptions explain semantics but do not weaken those constraints.

An invalid declaration fails closed. If no declaration exists, the certificate
reports `ui_assurance.status = "not_declared"`; it never upgrades missing UI
evidence into a guarantee. The proof summary includes a canonical hash, the
counts checked, and any counterexamples.

The next layer is the runtime adapter: UI automation must attach each action to
the declared screen/action identity and independently assert persisted state,
authorization, audit events, and rendered output. Those observations should
be added to the certificate before a deployment policy treats it as verified.

## Browser automation and trace verification

The current-phase browser adapter uses a real Chromium-family browser through
Playwright (WebDriver is an alternative adapter). It must use semantic Nirdosha
identifiers rather than coordinates or CSS guesses:

```text
screen=approval_review
control=approve_button
field=approval_status
```

For every action it records a `RuntimeTrace` event containing the screen,
action, role, before/after state, observed postconditions, and hashes for the
request, response, and audit event. The shared verifier in
`nirdosha-contract-core::ui_assurance::verify_trace` checks that:

- the screen and action are declared;
- the trace follows the declared state transition;
- the role is permitted;
- every declared postcondition was observed;
- a sequence starts in the declared initial state.

Screenshots, DOM snapshots, accessibility trees, console logs, and network
captures are evidence, not the oracle by themselves. The independent oracle
must compare the browser observation with the backend response, persisted state,
authorization decision, and append-only audit event.

## Z3 boundary

Z3 does not prove pixels or arbitrary browser behavior. It proves the finite
abstraction: states, roles, actions, data predicates, and forbidden
transitions. For example, the verifier asks whether this counterexample is
satisfiable:

```text
state = InReview
and role = Approver
and action = approve
and next_state != Approved
```

`unsat` proves that no such transition exists in the declared model. The
browser trace then supplies evidence that the deployed build produced a
transition covered by that model. A future `ui_assurance_smt` adapter will emit
the solver version, `rlimit`, formula hash, query result, and any model-derived
counterexample into the Recipe v1 artifact. Z3 `unknown`, missing evidence,
selector drift, or build/environment mismatch is `INCONCLUSIVE`, never pass.

## Test ownership

Product/domain owners define requirements, roles, and invariants. The Solution
Finder agent generates normal journeys; the Problem Finder agent generates
negative, stale, replay, privilege, and concurrency cases. A deterministic
Nirdosha runner compiles those cases into executable browser tests and decides
the result. Agents propose tests but never certify their own proposals. Humans
approve the model and claims, not every repeated execution.

The strongest supported claim is therefore bounded:

> For build X, model Y, browser/runtime Z, declared data, and listed
> invariants, the browser trace and independent backend evidence matched a
> solver-checked UI model.

This does not claim that the model captures every business requirement or that
an arbitrary future browser, backend, or deployment is correct.
