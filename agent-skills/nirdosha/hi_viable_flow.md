# A viable flow for `hi` — converging the day's findings into one design

This is the single recommendation, not another set of options. It exists
because today surfaced four separate, real problems in one session and
they compose into one fix, not four:

1. A generated app can look role-gated and not be (the fintech incident
   — `requires(role:...)` written, never enforced; screens silently
   dropped for lacking a naming-convention getter — the latter now
   closed by `check_screen_derivation_coverage`, shipped today).
2. The product's *own* editing surface has the identical shape of bug:
   `:edit` and `:ask` are real, wired routes with **no click target** —
   a capability that exists and is invisible is the same failure class
   as a role that's declared and not enforced.
3. "Modes" are a real backend state machine the UI makes the user
   *be*, instead of hiding.
4. Certification/standards (FAPI, medical, aerospace, IoT) has a real,
   detailed backend design (RFC 0016) and zero front-of-house surface.

One flow, sequenced so each step ships on what the step before it
proved, addresses all four without waiting on the hardest pieces
first.

---

## The flow, end to end

### 1. Describe (unchanged underneath, new surface)

User types free text. `hi_llm::populate_candidates` runs exactly as
today — no backend change. The response is not a graph dump; it's a
short, plain-language recap of what was found ("3 users, 5 screens, 2
money-math rules"), with a **Review** action. This is Option 2's
opening screen, chosen over Option 1's full conversational thread
because Part 1's own tradeoff still holds: demoting the graph to "an
optional tab" risks losing the exact thing RFC 0014 was built to add.
Describe is the one moment a linear step genuinely helps — everything
after it is the canvas.

### 2. The canvas — one surface for build + generate, not two modes

This is Option 4, corrected by today's practicality check:

- **Confirm is a click**, same `/api/confirm` route, no `:confirm`
  needed. A soft dashed ring marks "still reviewing"; solid means
  confirmed. Matches what's already real in `hi_graph.rs`.
- **Every node gets three affordances that exist today and have never
  had a click target**: a pencil (→ `/api/edit`, unlocks the unit
  exactly as `edit_driving_text`'s own doc comment says it will —
  surfaced, not hidden), the existing Attach form (unchanged), and
  hover-revealed `+role`/`+nfr`/`+screen` buttons that open Part 2's
  plain-language micro-forms instead of the current six-item template
  dropdown.
- **Generate is a button, not a mode, and it is explicit — not
  auto-compile.** This is the one place the original Option 4 pitch
  was wrong: `generate_program` resends **the whole confirmed set**
  to the model every time (`hi_graph.rs:876-882`, RFC 0014's own
  disclosed "one combined file... regenerated in full each pass") —
  silent continuous background compilation would mean every small edit
  re-runs a real, costly, sometimes-slow LLM call and can rewrite
  unrelated working code. The honest version: a **"N changes since
  last build"** counter and a **Rebuild** button the user presses on
  purpose. This is not a downgrade from "continuous" — it's the
  correct reflection of what the compiler actually does today, and it
  is still a large improvement over today's console-typed `:generate`.
- **Publish is one always-present button**, enabled once a build is
  current, with a one-line disabled-reason otherwise ("rebuild first",
  "2 pieces still unconfirmed"). Same `/api/publish` underneath.

### 3. Ask — a rail, not a tab, present on every screen

Directly answering "chat window should be available at each tab":
**Ask is not one of the competing base-experience options — it's a
cross-cutting layer.** A slim, collapsible rail (docked right, between
canvas and node panel in the mockup) is present on the canvas, on the
certification panel, and on the Describe screen alike — same
`/api/ask` route (`hi_graph.rs::ask`, already real) behind all of
them. This is the direct fix for the second finding above: `:ask` is a
real capability that today only a person who already knows the console
exists will ever find.

Concretely: the rail is a small persistent tab at the canvas edge; one
click expands it into a short chat panel; it never modally interrupts
whatever the user is doing elsewhere on the canvas.

### 4. Governing rules — visible before it's a gallery

Before Tab 6's pack marketplace (which needs new backend work — no
pack-listing/install API exists today, `grep "/api/pack"` turns up
nothing), ship the part that costs nothing new: a **"Governing rules"**
panel that states, in plain language, exactly what RFC 0016 already
calls "what is already domain-baked" —

> Every exposed action that changes data needs a role check — enforced.
> Money never moves without recording both sides — enforced.
> No value is used after it's already been spent — enforced by
> construction.

— reading straight from what the compiler *already guarantees
unconditionally* (`0016-domain-packs...md`, "What is already
domain-baked", the transact/authorization/ownership list) plus,
per-project, whichever unsigned pack is installed (`banking-v0` today).
This ships the *legibility* half of certification immediately, and its
UI (a card list with plain-language claims) is the exact shape the
real pack gallery grows into once install-from-UI and signing (5b)
exist — nothing here gets thrown away later, it gets a "browse more"
button added to it.

### 5. Role-coverage, closing the fintech incident's actual root cause

The screen-derivation gate shipped today catches "a screen with no
data." It does not catch "a role that's declared and never checked" —
the bug that actually broke the demo login. `hi_enhance_phase_ux.md`'s
**Option B** (a `CoverageFailureClass` for `requires(role:...)` with no
matching `check_role` call, wired into the same self-repair loop as
contract coverage) is the compiler-side piece this flow assumes lands
alongside the UI work above — without it, the canvas's `+role` button
can still produce exactly today's silent failure. This is real
engineering (a new AST walker, per `hi_enhance_phase_ux.md`'s own
cost note) and belongs on its own timeline, not blocking 1-4.

---

## What ships without waiting on anything hard

| Step | What it needs | Backend change? |
|---|---|---|
| Describe recap screen | New UI over `populate_candidates` | None |
| Canvas confirm-by-click | New UI over `/api/confirm` | None |
| Edit pencil | New UI over `/api/edit` (real today) | None |
| Ask rail, everywhere | New UI over `/api/ask` (real today) | None |
| `+role`/`+nfr`/`+screen` micro-forms | New UI over `/api/attach` | None |
| Rebuild button + change counter | New UI; honest framing of existing `generate_program` cost | None |
| Governing-rules panel (plain-language, today's unsigned pack) | New UI reading `hi_plugin::installed_pack_ids` | None |
| Role-coverage gate | — | **Yes** — new `CoverageFailureClass`, AST walker (`hi_enhance_phase_ux.md` Option B) |
| Pack gallery / install-from-UI | — | **Yes** — new listing + install routes |
| Signed packs, trust badges, real "Get certificate" | — | **Yes** — RFC 0016 5b (Ed25519, transparency log; "zero code shipped" today) |
| Live-preview-first editing (Option 3) | — | **Yes** — unify editor and served-app runtime, which are separate processes today |

Seven of eleven rows are pure front-end work against routes that
already exist and already work — that's most of "no learning curve for
basics, intuitive for advanced" achievable without a single new
backend line. The remaining four are real, separately-budgeted
engineering, each already scoped in an existing document
(`hi_enhance_phase_ux.md` for role-coverage, RFC 0016 for packs/
signing, this session's Part 1 comparison for live-preview).

## The one thing this flow deliberately does not attempt yet

Silent, always-on background compilation. It is the most-requested
feel (an IDE that "just knows") and the least honest thing to promise
against a compiler that regenerates one whole file per pass through a
paid, sometimes-slow model call. Revisit it only once generation is
genuinely incremental (per-unit, not whole-file) — a real compiler
change RFC 0014 already names as future work, not a UI decision.
