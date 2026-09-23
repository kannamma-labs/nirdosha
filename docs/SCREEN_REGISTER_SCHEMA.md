# Nirdosha Screen Register v2

`SCREEN_REGISTER_SCHEMA.json` defines the inventory-side contract for
`screens.toml`. It is deliberately separate from
`UI-proof-specification.toml`: the screen register says *what is generated,
where it comes from, which policies apply, and how it is made reproducible*;
the proof specification says *what the generated UI must prove at runtime*.

The schema adds deterministic code-generation inputs that were not part of the
behavioral proof specification:

- `codegen`: template, renderer, mount symbol, artifact namespace, generated
  files, plugin pack, feature flags, **route path**, route ownership, and source globs.
- `data_binding`: entity/read-model/store names, schema version, primary key,
  field mapping, catalog references, **bound field list**, and an optional **guard**
  binding that forces generated reads/writes through `GuardedTable`.
- `policy`: purpose, policy IDs, allowed actions, masking, subject scope, and
  segregation class.
- `parameters`: **archetype-specific macro inputs** consumed by the selected
  template, such as `refresh_seconds`, `steps`, or `guard`.
- `dependencies`: pinned macro/plugin/dataset/policy/route/source inputs with
  optional versions and SHA-256 hashes.
- `provenance`: owner, reviewers, change ticket, source hash, and the last
  verified revision.
- `render_profile`: theme, density, responsive behavior, accessibility level,
  streaming, and offline support.
- `determinism`: canonical ordering key, non-negative seed, fixture namespace,
  stable ID, and whether generated names are allowed.

Every string-valued property and every string-valued enum now carries a JSON
Schema `description`. These descriptions are part of the authoring contract:
an LLM producing a register should use them to choose the value, while a
consumer should use them to interpret the value. They do not replace enum,
pattern, hash, or cross-register validation; they explain the semantics of a
value that has already passed those structural checks.

## Logging contract

Every v2 screen has a `logging` object. `domain` and `country` are required;
they are not inferred from the screen name. This makes the generated logging
contract agree with Nirdosha's `#[contract(logging(...))]` policy:

```toml
[screen.logging]
domain = "payments"
country = "IN"
entity = "PaymentInstruction"
event_class = "transaction"
region = "APAC"
subdomain = "approval"
level = "notice"
policy_id = "LOG-PAYMENTS-01"
retention_class = "regulated-7y"
required_fields = ["screen_id", "actor_id", "correlation_id"]
redaction_profile = "pci-minimal"
```

`country` accepts an ISO-like two-letter uppercase code, `EU`, or `*` for a
deliberately global policy. A consumer should still validate the value against
the deployed logging-policy register. The schema validates shape; it does not
authorize a domain or policy.

## Data binding contract

`data_binding.fields` is the ordered list of fields the screen actually binds
from the entity. Each field carries its name, type, and behavioral flags:

```toml
[[screen.data_binding.fields]]
name = "case_id"
type = "String"
required = true
sensitive = false
masked = false
display = true
editable = false

[[screen.data_binding.fields]]
name = "rationale"
type = "String"
required = true
sensitive = true
masked = true
display = true
editable = true
```

`data_binding.guard` is optional. When present, the generator MUST emit a macro
invocation whose reads and writes go through the named `GuardedTable` under the
given purpose. For singleton screens (e.g. `settings_screen!`) use `row_id` to
name the fixed logical row; for keyed entities (e.g. `crud_screens!`,
`wizard!`, `communication_feed!`) omit it.

```toml
[screen.data_binding.guard]
table = "case_table"
purpose = "Operations"

# For a settings singleton only:
# row_id = "current"
```

## Parameters

`screen.parameters` holds archetype-specific inputs that the selected template
passes through to the macro. Keys are macro-defined; values must be parseable by
the target macro. Common examples:

```toml
[screen.parameters]
refresh_seconds = 5

[screen.parameters.guard]
table = "message_table"
purpose = "Operations"
```

The schema permits any key under `parameters` because different archetypes
accept different clauses; generators should validate the key/value shape against
the selected archetype.

Two archetypes with fully-specified parameters (shipped 2026-09-23):

- **`static_embed`** — presentational, no `GuardedTable`; the route gate is the
  whole story. `parameters = { title, content_file, sha256, access }`, with
  `content_file` project-root-relative and `sha256` the content file's
  SHA-256 — **verified at macro expansion** (a content edit without a re-pin
  is a build error), and disclosed on the served page as a digest footer.
- **`report_builder`** — ad-hoc aggregate report (RTM's 15.2 archetype).
  `parameters = { access, dimensions = ["status", "priority"] }`; reads
  `GuardedTable::guarded_aggregate` under the **aggregate** guard action, so
  the screen's `policy.allowed_actions` must name `"aggregate"`. Counts only;
  dimensions must be `String`-typed fields of the entity; guard-required by
  design (no unguarded path exists).
- **`tree_view`** — parent/child hierarchy (RTM's 5.6 archetype).
  `parameters = { access, id_field, parent_field, label_field }`, all
  `String`-typed; nests the guard-decoded snapshot in-process, cycle-safe,
  orphans rendered honestly. Guard-required by design.

Field flags in `data_binding.fields` are **security inputs** the generator
synthesizes into the screen's `guard_policy!` records (shipped 2026-09-23):
`masked = true` (alias `sensitive`) becomes `forbidden(...)` — dropped from
reads as genuine absence (the field's type must be `Option<...>`, enforced at
generation time) and refused on writes via the fail-closed allowed set;
every other field the screen declares becomes `allowed(...)`; `required = true`
becomes `required(...)` on **create** policies only (v1 scope). No flags → no
clause, exactly the pre-synthesis shape. Because subjects come from each
screen's own `roles`, two screens over the same table with different field
flags synthesize different policies — per-subject masking with masks that
follow the subject, not the route.

## Minimal v2 shape

```toml
[metadata]
schema = "nirdosha.screen-register/v2"
register_id = "rtm"
version = "2.0.0"
source_inventory = "examples/rtm/screen.md"
total_screens = 152
canonical_order = "module_then_id"
codegen_profile = "rtm-web"
generator_version = "nirdosha-screen-codegen/1"
default_timezone = "UTC"
default_locale = "en-IN"

[[screen]]
id = "1.1"
name = "Login"
module = "M1"
archetype = "login!"
stage = "built"
roles = ["AllRoles:R"]
datasets = ["IDP.users_file"]
blocked_by = []

[screen.codegen]
template = "auth/login"
mount_symbol = "mount_login"
renderer = "web"
artifact_namespace = "rtm.m01.login"
generated_files = ["src/screens/m01_auth.nir"]
route_path = "/login"
route_owner = "M1"

[screen.data_binding]
entities = ["Identity"]
store = "IDP.users_file"
schema_version = "identity/v1"
primary_key = "user_id"
catalog_ref = "CATALOG.identity.login"

[[screen.data_binding.fields]]
name = "user_id"
type = "String"
required = true

[screen.parameters]
mode = "demo"

[screen.policy]
policy_ids = ["AUTH-LOGIN-01"]
purpose = "authenticate_user"
allowed_actions = ["read", "propose", "confirm"]
subject_scope = "self"
segregation_class = "identity-boundary"

[screen.logging]
domain = "identity"
country = "IN"
entity = "LoginSession"
event_class = "access"
level = "notice"
policy_id = "LOG-IDENTITY-01"
required_fields = ["screen_id", "actor_id", "correlation_id"]
redaction_profile = "identity-minimal"

[[screen.dependencies]]
kind = "policy"
name = "AUTH-LOGIN-01"
version = "1"
hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
required = true

[screen.provenance]
owner = "identity-platform"
reviewers = ["security", "ui-platform"]
change_ticket = "RTM-1234"
last_verified_revision = "git:0123456"

[screen.render_profile]
theme = "nirdosha-default"
density = "comfortable"
responsive = true
accessibility_level = "AA"
supports_streaming = false
supports_offline = false

[screen.determinism]
ordering_key = "module,id"
seed = 0
fixture_namespace = "rtm.m01.login"
stable_id = "screen:1.1"
allow_generated_names = false
```

## Migration and validation

The checked-in RTM register is currently the historical v1 inventory. It does
not become v2 merely because this schema exists: it must be migrated by adding
the v2 metadata and the required per-screen `codegen`, `logging`, and
`determinism` data (with the other sections added where applicable). During
migration, keep the existing `id`, `module`, `stage`, role, dataset, and
`blocked_by` values unchanged and record the migration in `provenance`.

Validation should happen before code generation and before a recipe is issued:

1. Validate the TOML-to-JSON projection against
   `docs/SCREEN_REGISTER_SCHEMA.json`.
2. Resolve every dependency and verify supplied hashes.
3. Check `total_screens` and canonical ordering against the actual `screen`
   array.
4. Resolve each logging `policy_id` against the deployed logging-policy
   register, including its domain and country.
5. Include the canonical register hash in the recipe inputs so a generated
   certificate cannot silently use a different screen inventory.

The existing `cargo nirdosha ui-proof` command remains intentionally
conservative: it generates a structural proof inventory from the register. It
does not invent behavioral assertions or claim that browser automation has
executed them.

The full code generator (`cargo nirdosha generate-screens <project-dir>`,
shipped 2026-09-23) goes further and emits the crate's whole `src/*.nir` tree
from this register plus `menus.toml`. Its data rule is the guard-posture rule:
**every entity a generated screen touches must have a `data_binding.guard`
somewhere in the register** — the generator refuses an entity with none, so a
generated app cannot contain an unguarded data screen. On guarded screens the
generator emits only `*_with_auth` routes; the screen's declared `access_*`
strings become vestigial route metadata (OpenAPI summaries), never a second
authorization check — the synthesized `guard_policy!` corpus is the single
authority for row visibility, field masking, required/forbidden write fields,
caps, tenant scoping, and audit. Unsupported archetypes hard-error; mark such
screens `stage = "blocked"` with `blocked_by = ["archetype:<name>"]`.
`examples/helpdesk/` is the end-to-end proof: 8 screens → generated `.nir`
tree → compiled crate → 8 passing guard-authority smoke tests.
