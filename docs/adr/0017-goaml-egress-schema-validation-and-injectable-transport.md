# 0017: goAML egress — real libxml2 XSD validation against a reduced schema, submission as a validation-gated injectable transport

Date: 2026-09-21
Status: accepted

## Context

Unlike ingestion/features/scoring/screening/alerts (`rfcs/0025-nirdosha-rtm-ecosystem.md`
§8.1-§8.5), the RFC doesn't give goAML SAR/CTR egress its own worked
trait-and-driver treatment — only Part 6/V10's SAR-filing findings mention
it. This session's own approved execution plan names the concrete scope
directly: "implement the XML schema validation + envelope shape against
goAML's published public XSD, with submission left as an injectable
transport (so a real gateway can be plugged in without code changes) —
proven by a schema-validation test, not a live submission." Nothing in the
workspace implemented any of this before now.

## Decision

**New crate `crates/nirdosha-egress-report-goaml`**, same naming
convention as `docs/adr/0013`/`0014`/`0015`/`0016`.

**Real XSD validation via `libxml2`, not a hand-rolled validator.** No
actively-maintained, complete pure-Rust XSD engine exists on crates.io.
Writing a validator that only checks the specific shapes this crate
happens to generate would be indistinguishable, from the outside, from
real schema conformance checking — until a report shaped differently but
still schema-invalid slipped through untested. The `libxml` crate
(bindings over the system `libxml2`, already present on this machine via
`pkg-config` — confirmed before adding the dependency, no new C-toolchain
build needed, unlike the `rdkafka` risk `docs/adr/0016` avoided a
different way) wraps `libxml2`'s own, real `xmlSchemaValidateDoc` engine.
`schema/goaml_report.xsd` is validated by that real engine, not simulated.

**`schema/goaml_report.xsd` is a reduced, representative subset of the
official goAML schema, not the complete document.** The real UNODC goAML
XML Schema spans hundreds of elements across multiple report types and
optional party/account/entity sub-structures neither this driver nor its
tests exercise. The reduced schema keeps the structure every report type
shares — report code (enumerated `STR`/`CTR`), submission code (enumerated
`E`/`A`/`C`), one reporting entity, one-or-more transactions each with
`from`/`to` parties — with real XSD constraints (`minOccurs`, enumerations,
`xs:date`/`xs:decimal` typing) a real validator can genuinely reject
violations of. This is the same fixture-not-production scoping
`docs/adr/0014`'s sanctions list and `docs/adr/0015`'s ONNX model apply to
their own checked-in test artifacts.

**Submission is gated by validation at the type level, not by convention.**
`GuardedSubmitter::submit_if_valid` is the only path this crate exposes
from a `ReportEnvelope` to a `SubmissionTransport` — it always calls
`GoamlSchemaValidator::validate` before ever calling `transport.submit`,
proven by a test asserting an invalid envelope never reaches the transport
(no file appears in the outbox). `FileOutboxTransport` — writing a
validated envelope to a local directory — is the one honest
`SubmissionTransport` this workspace can ship without live goAML gateway
credentials; a real gateway client implements the same trait and replaces
it without touching envelope construction or validation.

## Consequences

**What this makes possible.** `cargo test -p nirdosha-egress-report-goaml`
proves, with real `libxml2` schema validation and no network access: a
correctly-shaped report (including one with `&`/`"` in a party name,
proving real XML escaping round-trips through real parsing) validates; a
report missing a required element or using an out-of-enumeration code
fails with libxml2's own structured error messages, not a generic
rejection; and an invalid envelope demonstrably never reaches the
transport.

**What this does *not* make possible, stated so it's never misread later.**
This is not the complete official goAML XSD — a real production deployment
still needs the full official schema (or an explicitly narrower, formally
reviewed subset matching the actual regulator's accepted report types)
before this validator could gate a real filing. No live goAML gateway
submission happens anywhere in this crate; `FileOutboxTransport` is a
local, inspectable stand-in, and per the plan's own scope note, inventing
fake vendor credentials to reach a live endpoint was explicitly out of
scope for this phase.
