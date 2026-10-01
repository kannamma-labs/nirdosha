# `nirdosha-guard-mcp`

Data-guard MCP server for LLM agents — a first-class guarded client.

Implements RFC 0023 §1C.3 and RFC 0024. Every MCP tool is a thin
wrapper over the same `evaluate` surface used by every other guard
client. Agents receive non-transferable, purpose-bound delegation
tokens; `submit_write` is default-off and requires evaluate-then-act
with a matching plan hash.

Current phase: scaffolding. Tool descriptions, delegation token shape,
and default-off `submit_write` are present; registry-generated tools,
real token minting, and full agent-default enforcement are future
work.
