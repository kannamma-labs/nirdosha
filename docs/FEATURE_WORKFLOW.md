# Feature & bug workflow

How a discussion becomes tracked, designed, built, and closed in this
repo, from 2026-09-14 on. This governs how Claude-assisted sessions in
this repo operate by default — no per-session confirmation needed
before following it.

## The cycle

```
discuss --> GitHub issue --> RFC (new or amended) --> implementation --> commit --> close the issue, citing the commit
```

1. **Discuss.** A feature idea or a bug surfaces in conversation (or in
   review, or in the field).
2. **GitHub issue.** Before design or implementation starts, an issue
   is opened in `kannamma-labs/nirdosha` — `enhancement` or `bug`, plus
   whatever area labels apply (`compiler`, `ui`, `llm`, `infra`, ...).
   The issue is the durable handle everything else references.
   - **A multi-part idea becomes multiple issues, not one umbrella
     issue.** If a single discussion covers several independently
     shippable pieces, each gets its own issue — an umbrella issue
     that bundles unrelated work never has a single commit that closes
     it, which defeats the point of tracking it at all.
   - A dependency between two pieces of work is stated in the blocked
     issue's body and gets the `blocked` label, naming the issue it
     waits on.
3. **RFC.** Anything non-trivial gets a new `rfcs/00NN-*.md` or an
   amendment to an existing one (this repo's own established
   convention — see `rfcs/0014-generative-build-console.md`'s dated
   amendment sections, `rfcs/0016-...md`'s dated Open-Questions notes).
   The RFC text names the issue number; the issue body links the RFC
   section. A small fix (a message, a one-line bug) can skip a
   standalone RFC and just cite the issue in the commit — not every
   issue needs a document, but every non-trivial design decision needs
   one *somewhere* traceable, per this repo's existing practice.
4. **Implementation.** Ordinary work against the issue and its RFC.
5. **Commit.** Elaborate commit message per this repo's own standing
   rule (what changed, why, what it replaces) — the body names the
   issue(s) the commit addresses.
6. **Close, citing the commit.** When a feature or bug is actually
   done, the issue is closed with a comment naming the exact commit
   SHA that resolved it — not "fixed in latest," the literal short
   hash, so anyone reading the issue later can `git show` the exact
   change. A commit that only partially addresses an issue is noted as
   such in the issue, and the issue stays open.

## What this replaces

Nothing existing — `CONTRIBUTING.md`'s "open an issue first" and this
repo's existing RFC-amendment convention were already most of this. This
document makes the missing link explicit: **the commit that closes an
issue is always named on the issue itself**, and a multi-feature
conversation is decomposed into separate issues at discussion time, not
after the fact.
