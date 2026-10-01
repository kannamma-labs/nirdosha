#!/usr/bin/env python3
"""Generate RFC 0029 v1 JSON/JCS conformance vectors from reviewed YAML."""
from __future__ import annotations

import argparse
import datetime
import hashlib
import json
from pathlib import Path
import re
import sys

import jsonschema
import yaml

ROOT = Path(__file__).resolve().parent
SOURCE = ROOT.parent / "0029-influence-review-symbolic.yaml"
VECTOR_DIR = ROOT / "vectors"
JCS_DIR = ROOT / "jcs"
MANIFEST = ROOT / "manifest.json"
SCHEMA_PATH = ROOT / "schema.json"
SCHEMA_VERSION = "rfc0029.influence-review.fixture.v1"


def canonical_bytes(value):
    def check(v):
        if v is None or isinstance(v, (str, bool)):
            return
        if isinstance(v, int) and not isinstance(v, bool):
            if not -(2**53) + 1 <= v <= (2**53) - 1:
                raise ValueError("integer outside I-JSON exact range")
            return
        if isinstance(v, float):
            raise ValueError("floats are forbidden in RFC 0029 v1 vectors")
        if isinstance(v, list):
            for x in v: check(x)
            return
        if isinstance(v, dict):
            for k, x in v.items():
                if not isinstance(k, str): raise ValueError("non-string object key")
                check(x)
            return
        raise ValueError(f"unsupported JSON value: {type(v).__name__}")
    def utf16_key(text):
        raw = text.encode("utf-16-be")
        return tuple(int.from_bytes(raw[i:i + 2], "big") for i in range(0, len(raw), 2))
    def render(v):
        if v is None: return "null"
        if v is True: return "true"
        if v is False: return "false"
        if isinstance(v, str): return json.dumps(v, ensure_ascii=False, separators=(",", ":"))
        if isinstance(v, int): return str(v)
        if isinstance(v, list): return "[" + ",".join(render(x) for x in v) + "]"
        return "{" + ",".join(render(k) + ":" + render(v[k]) for k in sorted(v, key=utf16_key)) + "}"
    check(value)
    return render(value).encode("utf-8")


def digest(value):
    return "sha256:" + hashlib.sha256(canonical_bytes(value)).hexdigest()


def json_value(value):
    """Remove YAML-library-specific scalar types before canonicalization."""
    if isinstance(value, datetime.datetime):
        if value.tzinfo is None:
            raise ValueError("naive timestamp is forbidden")
        return value.astimezone(datetime.timezone.utc).isoformat().replace("+00:00", "Z")
    if isinstance(value, datetime.date):
        return value.isoformat()
    if isinstance(value, list):
        return [json_value(x) for x in value]
    if isinstance(value, dict):
        return {str(k): json_value(v) for k, v in value.items()}
    return value


def split_node(token):
    node_id, kind = token.split(":", 1)
    return node_id, kind


def split_edge(token):
    source, kind, target = token.split("-", 2)
    return source, kind, target


def attrs(fixture, node_id, kind):
    fid = fixture["id"]
    table = {
        "ModelInvocation": {"deployment_id": f"model:{fid}:v1", "receipt_id": f"receipt:{fid}:{node_id}:v1"},
        "Fact": {"authority_class": "profile_authority", "type_id": f"fact:{fid}:{node_id}", "version": 1},
        "Transform": {"transform_id": f"transform:{fid}:{node_id}", "version": 1},
        "Rule": {"rule_id": f"rule:{fid}:{node_id}", "bundle_hash": "symbolic:bundle-rfc0029-fixtures-v1"},
        "CandidateSet": {"candidate_type": f"candidate:{fid}:{node_id}"},
        "Presentation": {"surface_id": f"surface:{fid}:{node_id}", "version": 1},
        "Decision": {"decision_class": f"decision:{fid}:{node_id}"},
        "Obligation": {"obligation_type": f"obligation:{fid}:{node_id}"},
        "Evidence": {"evidence_type": f"evidence:{fid}:{node_id}"},
    }
    if kind == "Capability": return fixture["capabilities"][node_id]
    if kind == "Effect": return fixture["effects"][node_id]
    if kind == "AuthorityAssertion": return fixture.get("assertion", {"id": node_id})
    if kind == "Review":
        contract = fixture.get("review_contract") or fixture.get("assertion", {}).get("contract")
        return {"review_contract": contract}
    base = dict(table.get(kind, {}))
    if kind == "Fact" and node_id in fixture.get("fact_provenance", {}):
        provenance = dict(fixture["fact_provenance"][node_id])
        if "authorized_consumers" in provenance:
            provenance["authorized_consumers"] = [f"node:{fid}:{c}" for c in provenance["authorized_consumers"]]
        base.update(provenance)
    if kind == "Decision" and node_id in fixture.get("fact_requirement", {}):
        base.update(fixture["fact_requirement"][node_id])
    return base


def referenced_catalogs(doc, fixture):
    refs = []
    names = set()
    for key in ("review_contract", "required_review_contract", "supplied_review_contract"):
        if fixture.get(key): names.add(fixture[key])
    if fixture.get("assertion", {}).get("contract"): names.add(fixture["assertion"]["contract"])
    for name in sorted(names):
        contract = doc["review_contract_catalog"][name]
        refs.append({"catalog": "review_contract", "name": name, "value": contract})
        for pred in contract.get("predicates", []):
            refs.append({"catalog": "predicate_definition", "name": pred, "value": doc["predicate_definitions"][pred]})
    cert = fixture.get("observe_certificate")
    if cert:
        refs.append({"catalog": "observe_certificate", "name": cert, "value": doc["observe_certificate_catalog"][cert]})
    final = fixture.get("expected", {}).get("final_authority")
    if final in doc["authority_catalog"]:
        refs.append({"catalog": "authority", "name": final, "value": doc["authority_catalog"][final]})
    unique = {(r["catalog"], r["name"]): r for r in refs}
    return [unique[k] for k in sorted(unique)]


def parse_status(value, default_status):
    if value is None: return {"status": default_status, "diagnostic": None}
    if ":" in value:
        status, detail = value.split(":", 1)
        return {"status": status, "diagnostic": detail}
    return {"status": value, "diagnostic": None}


def expected(doc, fixture):
    raw = dict(fixture["expected"])
    group = next((g for g in doc["equivalence_groups"] if fixture["id"] in g["fixtures"]), None)
    if group:
        value = group["normalized"]
        normalized = {"status": "asserted", "value": value, "sha256": digest(value)}
    else:
        normalized = {"status": "not_asserted"}
    admission = parse_status(raw.pop("admission", None), "not_evaluated")
    review_raw = raw.pop("review", None)
    review = parse_status(review_raw, "not_required" if fixture.get("nodes") else "not_evaluated")
    review = {"status": review["status"], "detail": review["diagnostic"]}
    cap_raw = raw.pop("capability", None)
    if cap_raw and cap_raw.startswith("issued:"):
        capability = {"status": "issued", "effect_class": cap_raw.split(":", 1)[1]}
    elif cap_raw == "not_issued": capability = {"status": "not_issued", "effect_class": None}
    else: capability = {"status": "not_evaluated", "effect_class": None}
    known = {"graph", "provenance", "level", "model_authorized", "final_authority", "invalidation"}
    extensions = {k: v for k, v in raw.items() if k not in known}
    return {
        "graph_id": raw.get("graph"), "normalized_graph": normalized,
        "provenance": raw.get("provenance", "unknown"),
        "model_level": raw.get("level", "not_evaluated"),
        "model_authorized": raw.get("model_authorized", "unknown"),
        "final_authority": raw.get("final_authority"),
        "admission": admission, "review": review, "capability": capability,
        "invalidation": raw.get("invalidation", "not_evaluated"), "extensions": extensions,
    }


def make_vector(doc, fixture):
    nodes = []
    for token in fixture.get("nodes", []):
        nid, kind = split_node(token)
        nodes.append({"id": f"node:{fixture['id']}:{nid}", "kind": kind, "attributes": attrs(fixture, nid, kind)})
    edges = []
    for token in fixture.get("edges", []):
        source, kind, target = split_edge(token)
        edges.append({"source": f"node:{fixture['id']}:{source}", "kind": kind, "target": f"node:{fixture['id']}:{target}"})
    parameters = {k: v for k, v in fixture.items() if k not in {"id", "profile", "nodes", "edges", "expected", "capabilities", "effects", "assertion", "observe_certificate", "fact_provenance", "fact_requirement"}}
    assertion_contract = fixture.get("assertion", {}).get("contract")
    if assertion_contract and "review_contract" not in parameters:
        parameters["review_contract"] = assertion_contract
        evaluation_fixture = doc["review_contract_catalog"][assertion_contract].get("evaluation_fixture")
        if evaluation_fixture:
            source = next(f for f in doc["fixtures"] if f["id"] == evaluation_fixture)
            parameters["evaluations"] = source.get("evaluations", [])
    effect_class = None
    if fixture.get("effects"):
        effect_class = next(iter(fixture["effects"].values()))["effect_id"]
    elif fixture.get("capabilities"):
        effect_class = next(iter(fixture["capabilities"].values()))["effect_class"]
    if effect_class:
        parameters["admission_policy"] = {"effect_class": effect_class, "maximum_model_level": doc["effect_authority_ceilings"][effect_class]}
    if "evaluations" in parameters:
        expanded = []
        for compact in parameters["evaluations"]:
            name, result = next(iter(compact.items()))
            stale = result == "stale:Deny"
            expanded.append({
                "predicate": name,
                "definition_id": doc["predicate_definitions"][name]["id"],
                "definition_version": doc["predicate_definitions"][name]["version"],
                "evaluator_authority": "authority:fixture-review-evaluator:v1",
                "input_commitment": f"symbolic:{fixture['id']}:{name}:inputs",
                "result": "Unknown" if stale else ("True" if result is True else "False"),
                "evidence_commitment": f"symbolic:{fixture['id']}:{name}:evidence",
                "evaluated_at": "2026-09-27T00:00:00Z",
                "valid_until": "2026-09-26T00:00:00Z" if stale else "2026-09-28T00:00:00Z",
                "revoked": False,
                "failure_behavior": "Deny",
            })
        parameters["evaluations"] = expanded
    return {
        "schema_version": SCHEMA_VERSION,
        "fixture_id": fixture["id"],
        "profile_id": f"profile:rfc0029:{fixture['profile']}:candidate-v1",
        "input": {
            "bundle": {"id": "bundle:rfc0029-fixtures", "version": 1, "hash": "symbolic:bundle-rfc0029-fixtures-v1"},
            "nodes": sorted(nodes, key=lambda x: (x["kind"], x["id"])),
            "edges": sorted(edges, key=lambda x: (x["kind"], x["source"], x["target"])),
            "catalog_entries": referenced_catalogs(doc, fixture), "parameters": parameters,
        },
        "expected": expected(doc, fixture),
    }


def load_yaml():
    class UniqueLoader(yaml.SafeLoader): pass
    def mapping(loader, node, deep=False):
        result = {}
        for key_node, value_node in node.value:
            key = loader.construct_object(key_node, deep=deep)
            if key in result: raise ValueError(f"duplicate YAML key {key!r} at line {key_node.start_mark.line + 1}")
            result[key] = loader.construct_object(value_node, deep=deep)
        return result
    UniqueLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, mapping)
    return json_value(yaml.load(SOURCE.read_text(), Loader=UniqueLoader))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    doc = load_yaml()
    vectors = {f["id"]: make_vector(doc, f) for f in doc["fixtures"]}
    schema = json.loads(SCHEMA_PATH.read_text())
    validator = jsonschema.Draft202012Validator(schema)
    entries = []
    for fid, vector in sorted(vectors.items()):
        errors = list(validator.iter_errors(vector))
        if errors:
            detail = "; ".join(f"{'/'.join(str(p) for p in e.path)}: {e.message}" for e in errors[:5])
            raise SystemExit(f"{fid} does not conform to schema.json ({len(errors)} error(s)): {detail}")
        data = canonical_bytes(vector)
        entries.append({"fixture_id": fid, "json_path": f"vectors/{fid}.json", "jcs_path": f"jcs/{fid}.jcs.json", "sha256": "sha256:" + hashlib.sha256(data).hexdigest(), "bytes": len(data)})
        path = VECTOR_DIR / f"{fid}.json"
        rendered = json.dumps(vector, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
        if args.check:
            if not path.exists() or path.read_text() != rendered: raise SystemExit(f"stale generated vector: {path}")
        else:
            VECTOR_DIR.mkdir(parents=True, exist_ok=True); path.write_text(rendered)
        jcs_path = JCS_DIR / f"{fid}.jcs.json"
        if args.check:
            if not jcs_path.exists() or jcs_path.read_bytes() != data: raise SystemExit(f"stale JCS vector: {jcs_path}")
        else:
            JCS_DIR.mkdir(parents=True, exist_ok=True); jcs_path.write_bytes(data)
    manifest = {"schema_version": SCHEMA_VERSION, "canonicalization": "RFC 8785 with v1 integers-only numeric profile", "fixture_count": len(entries), "fixtures": entries}
    rendered = json.dumps(manifest, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
    if args.check:
        if not MANIFEST.exists() or MANIFEST.read_text() != rendered: raise SystemExit(f"stale generated manifest: {MANIFEST}")
    else: MANIFEST.write_text(rendered)
    print(f"validated {len(entries)} canonical vectors against schema.json")


if __name__ == "__main__": main()
