---
name: Bug report
about: Something in the compiler, UI engine, or docs is wrong
title: ""
labels: bug
assignees: ""
---

**What happened**

A clear description of what went wrong.

**Diagnostic output**

Run the failing case with `nirdosha emit-ui <file.nir> -o /tmp/out.html`
(typecheck/ownership errors) or `nirdosha build <file.nir> -o /tmp/out`
(codegen "unsupported" errors) and paste the error output here — it's
the fastest way to pin down exactly where things went wrong. (There is
no interpreter and no `--format=json` flag anymore — see
[`SECURITY.md`](../../SECURITY.md) / `docs/API_TRUST_MODEL.md` §4a.)

```json
paste here
```

**Minimal repro**

The smallest `.nir` snippet that reproduces it, if the diagnostic alone
doesn't make the bug obvious.

```nirdosha
paste here
```

**Expected vs. actual**

What you expected to happen, and what actually happened instead.

**Environment**

- Nirdosha version/commit:
- OS:
- Built from source or prebuilt binary:
