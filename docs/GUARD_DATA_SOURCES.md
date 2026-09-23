# Guard data-source manifest

The guard runtime needs one deployment-specific mapping from logical sources
(`PG`, `RD`, `AC`, `K`, `IDP`, and so on) to the endpoints supplied by the
container orchestrator. This mapping is intentionally separate from
`screens.toml`: screens declare which logical datasets they use, while this
manifest declares how the guard can reach those datasets.

The schema is [`GUARD_DATA_SOURCES_SCHEMA.json`](GUARD_DATA_SOURCES_SCHEMA.json).
The manifest is generated at deployment/startup from environment variables.
It must contain variable names, not resolved URLs, passwords, tokens, or
certificates. The guard loader resolves the variables in memory and fails
closed if a required variable is missing, a driver is incompatible, TLS is
required but absent, or the health check fails.

## TOML shape

```toml
[manifest]
schema = "nirdosha.guard-data-sources/v1"
application = "rtm"
generated_from_env = true
environment_namespace = "NIRDOSHA"
environment_revision = "deployment-2026-09-23.1"

[[source]]
id = "PG"
kind = "rdbms"
driver = "postgres"
url_env = "NIRDOSHA_PG_URL"
credential_env = "NIRDOSHA_PG_PASSWORD"
namespace = "rtm"
access = "read_write"
tls_required = true
healthcheck = "select_1"
connect_timeout_ms = 3000
pool_max = 32
required = true
lineage_label = "postgres-primary"

[[source]]
id = "K"
kind = "mq"
driver = "redis"
url_env = "NIRDOSHA_K_REDIS_URL"
access = "publish"
tls_required = true
healthcheck = "ping"
required = false
lineage_label = "guard-events"

[[source]]
id = "O"
kind = "object_storage"
driver = "s3"
url_env = "NIRDOSHA_O_ENDPOINT"
credential_env = "NIRDOSHA_O_CREDENTIAL"
region_env = "NIRDOSHA_O_REGION"
namespace = "rtm-artifacts"
access = "read_write"
tls_required = true
healthcheck = "head_bucket"
required = true
lineage_label = "evidence-object-store"
```

The logical classes cover the store vocabulary already used by the RTM
register: relational (`PG`), reference data (`RD`), audit chains (`AC`),
graphs (`GR`), message queues (`K`), sanctions (`L`), objects (`O`), compiled
catalogs (`CATALOG`), guard runtime state (`RUNTIME`), identity (`IDP`),
gateways (`GW`), static signed artifacts (`STATIC`), and external integrations
(`EXT`). A class may use a different concrete driver in another deployment;
that choice is explicit in `driver` and must be attested by the driver
capability check.

## Resolution and security rules

1. Parse and validate the TOML against the JSON schema.
2. Require every `url_env`, `credential_env`, and `region_env` variable named
   by the manifest to match the allowed deployment namespace.
3. Resolve values in memory only. Never include resolved values in logs,
   certificates, recipes, browser traces, panic messages, or generated source.
4. Normalize the URL for driver validation, but hash only the redacted
   manifest (variable names and non-secret metadata).
5. Perform the declared health check with the declared timeout and enforce
   `tls_required` before registering the source with the guard.
6. Expose only the logical `id`, `kind`, driver capability, and health status to
   policy evaluation and lineage. Endpoint contents remain runtime secrets.

This manifest describes connectivity and capability. It does not grant access:
`guard_policy!`, tenant scoping, field policy, purpose, and approval rules still
decide whether a particular operation is allowed.
