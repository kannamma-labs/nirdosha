# Nirdosha Red-Team Starter Kit

Generated from adversarial review of the `feature-parity/roadmap` branch (Sep 2026).

## Quick start

```bash
git clone --branch feature-parity/roadmap https://github.com/kannamma-labs/nirdosha.git
cd nirdosha

# Build the toolchain (or use Codespaces)
cargo build -p nirdosha --release

# Run the official attack demo (baseline)
cargo run -p nirdosha --release -- build examples/attack_demo/agent_b/hr_assistant.nir -o /tmp/hr
/tmp/hr
```

## High-priority surfaces (from SECURITY.md)

1. `nirdosha build --serve` — request parsing, routing, `check_serve_exposure`
2. `codegen.rs` lowering of `requires` / `acquire` / field masking
3. `ownership.rs` / `typeck.rs` static guarantees
4. `emit-ui` client vs server enforcement gap
5. LLM-generated `.nir` that still type-checks + certifies yet violates a security property

## Adversarial probes included

| File | Target claim |
|------|--------------|
| `adversarial/01_forge_roleview.nir` | RoleView unforgeability |
| `adversarial/02_mask_bypass_attempt.nir` | Field masking fail-closed |
| `adversarial/03_serve_exposure_sketch.nir` | Deny-by-default serve exposure |
| `adversarial/04_ownership_alias.nir` | Incomplete `&` model |

## Recommended workflow

1. For each probe: `nirdosha verify <file>.nir` then `nirdosha certify <file>.nir`
2. For serve probes: `nirdosha build --serve <file>.nir -o /tmp/serve && /tmp/serve` then attack with curl/httpx
3. Record: verdict, evidence_tier, any counter-example, and whether a certificate was issued for a unsafe outcome
4. File findings via GitHub private vulnerability reporting (see SECURITY.md)

## Continuous suite sketch

- Hand-written probes (this kit)
- LLM-generated variants using the paste-anywhere prompt + adversarial system instructions
- CI gate that treats `evidence_tier: unknown` as non-passing by default
- Explicit negative tests that a forged RoleView can never appear in a successful compile

## Out of scope (per project policy)

- Local code execution with same privileges as the nirdosha process
- Attacks against the deleted interpreter / old serve.rs / sandbox path
