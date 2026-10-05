"""Current-source index, independent of historical inventory acceptance states.

This is a source audit, not a runtime or browser acceptance runner. It reads
only repository source and writes the new recovery snapshot.
"""
import argparse
import datetime
import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "rust/inventory/recovery-current-20261005.json"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--verification-log", type=Path)
parser.add_argument("--verification-exit-code", type=int)
args = parser.parse_args()
verification = {"latest_full_receipt": None, "tests_passed": None, "failed": None,
                "ignored": None, "scope_limit": "Source audit only; no full-suite receipt supplied."}
if args.verification_log:
    raw = args.verification_log.read_text(encoding="utf-8-sig")
    results = re.findall(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored", raw)
    assert args.verification_exit_code == 0 and results and all(row[0] == "ok" and int(row[2]) == 0 for row in results)
    verification = {"latest_full_receipt": args.verification_log.name,
                    "tests_passed": sum(int(row[1]) for row in results), "failed": 0,
                    "ignored": sum(int(row[3]) for row in results), "summary_count": len(results),
                    "exit_code": args.verification_exit_code,
                    "scope_limit": "Local Windows all-target synthetic receipt; source binding and these tests do not establish production/native/browser/device acceptance."}

services = json.loads((ROOT / "rust/inventory/services.json").read_text(encoding="utf-8"))
cli = json.loads((ROOT / "rust/inventory/cli.json").read_text(encoding="utf-8"))
server = (ROOT / "internal/hub/server.go").read_text(encoding="utf-8")
registered = re.findall(r'mux\.Handle(?:Func)?\("([^"\n]+)"', server)
assert len(registered) == 155, "Pinned Go HTTP registration count changed; review source"
assert set(registered) == {r["path"] for r in services["routes"]}
historical = ROOT / "rust/src/hub/route_coverage.json"
historical_hash = hashlib.sha256(historical.read_bytes()).hexdigest()
sources = {str(p.relative_to(ROOT)).replace("\\", "/"): p.read_text(encoding="utf-8")
           for p in (ROOT / "rust/src").rglob("*.rs")}
prod = {p: s for p, s in sources.items()
        if not any(part in {"tests", "fixtures"} for part in Path(p).parts)
        and not Path(p).stem.endswith("tests") and "/inventory/" not in p}
main_path = "rust/src/application/main_program/serve.rs"
main = sources[main_path]
router_path = "rust/src/hub/router.rs"
router = sources[router_path]

def refs(needle, files=None, limit=8):
    result = []
    for path, source in (prod if files is None else {p: sources[p] for p in files}).items():
        for number, line in enumerate(source.splitlines(), 1):
            if needle in line:
                result.append({"file": path, "line": number})
                if len(result) >= limit:
                    return result
    return result

go_sources = {str(p.relative_to(ROOT)).replace("\\", "/"): p.read_text(encoding="utf-8")
              for p in (ROOT / "internal/hub").glob("*.go") if not p.name.endswith("_test.go")}

def regex_refs(pattern, source_map, limit=8):
    result=[]
    for path, source in source_map.items():
        for number,line in enumerate(source.splitlines(),1):
            if re.search(pattern,line):
                result.append({"file":path,"line":number})
                if len(result)>=limit:return result
    return result

# Actual owner and main constructor/builder pairs, reviewed against router
# dispatch. A binding string is evidence of composition only, never acceptance.
GROUPS = [
    (r"^/api/agent-(?:chat|log)|^/api/grok-history$", "hub/agent_history_routes.rs", "with_agent_history"),
    (r"^/api/approval-patterns|^/approval-patterns/", "hub/approval_pattern_routes.rs", "with_approval_patterns"),
    (r"^/api/spawn-grid$", "hub/grid_spawn_routes.rs", "with_grid"),
    (r"^/api/spawn$", "hub/spawn_routes.rs", "with_spawn"),
    (r"^/api/providers", "hub/provider_routes.rs", "with_provider_routes"),
    (r"^/api/provider-distributions/", "hub/distribution_routes.rs", "with_provider_assets"),
    (r"^/api/provider-icons/", "hub/icon_routes.rs", "with_provider_assets"),
    (r"^/api/(?:bug-report|doctor)", "hub/diagnostic_routes.rs", "with_diagnostics"),
    (r"^/api/mobile-connect", "hub/mobile_routes.rs", "with_mobile"),
    (r"^/api/nvidia-nim", "hub/nvidia_routes.rs", "with_nvidia"),
    (r"^/api/whisper/", "hub/whisper_routes.rs", "with_whisper"),
    (r"^/api/voice/transcribe$", "hub/voice_routes.rs", "with_voice"),
    (r"^/api/(?:servers|profiles/fetch)", "hub/server_routes.rs", "with_servers"),
    (r"^/api/models$", "hub/models.rs", "with_models"),
    (r"^/api/slash-(?:cmd-sources|commands)$", "hub/slash_routes.rs", "with_slash_commands"),
    (r"^/api/(?:pick-|path-exists|list-subdirs|open-default-file|open-folder|open-terminal|open-dir|terminal-app)", "hub/host_routes.rs", "with_host"),
    (r"^/api/subscription", "hub/subscription_routes.rs", "with_subscriptions"),
    (r"^/api/session-usage$", "hub/usage_routes.rs", "with_session_usage"),
    (r"^/api/cli-(?:updates|update-eligibility)", "hub/update_routes.rs", "with_cli_updates"),
    (r"^/api/cli-versions$", "hub/cli_version.rs", "with_cli_versions"),
    (r"^/api/(?:net-hint|encoding-check)$", "hub/runtime_routes.rs", "with_runtime_observations"),
    (r"^/api/(?:usage|install)-link-defaults$", "hub/link_defaults.rs", "with_link_defaults"),
    (r"^/api/jev/evaluate$", "hub/jev_routes.rs", "with_jev"),
    (r"^/api/push/", "hub/push_routes.rs", "with_push"),
    (r"^/api/auto-approval/", "hub/auto_approval.rs", "with_auto_approval"),
    (r"^/api/handoff(?:/|$)", "hub/handoff_routes.rs", "with_handoff"),
    (r"^/api/approval(?:/|-action/)", "hub/approval_actions.rs", "with_approval_actions"),
    (r"^/api/routine", "hub/routines.rs", "with_routines"),
    (r"^/api/memo", "memos/mod.rs", "with_memos"),
    (r"^/api/(?:files-|git-)", "files/mod.rs", "with_files"),
    (r"^/api/(?:info|session/|session-chat|session-log|session-search|session-history|approval-history|session-store/|logs/|attachments/)", "hub/session_routes.rs", "with_session_routes"),
    (r"^/api/sessions/", "hub/child_routes.rs", "with_children"),
    (r"^/api/kill-all$", "hub/lifecycle.rs", "with_session_routes"),
    (r"^/api/shutdown$", "hub/lifecycle.rs", "with_shutdown"),
    (r"^/api/notify-(?:test|generate-topic)$", "hub/notify_routes.rs", "with_notifications"),
    (r"^/api/user-prefs$", "hub/preferences.rs", None),
    (r"^/api/(?:avatar|user-prefs/(?:avatar|notify-sound-custom))$", "hub/preference_media.rs", None),
    (r"^/api/auth/", "hub/auth.rs", None),
    (r"^/api/(?:log-config|idle-timeout|terminal-color|handoff-intent-mode|reconnect-grace|input-config|orchestration-config|notify-config)$", "hub/settings.rs", "with_settings_published"),
    (r"^/api/attach$", "hub/attachments.rs", None),
    (r"^/ws$", "hub/websocket.rs", "WebSocketService::new"),
    (r"^/(?!api/)", "hub/assets.rs", "with_assets"),
]

http = []
for row in services["routes"]:
    path = row["path"]
    group = next((g for g in GROUPS if re.search(g[0], path)), None)
    assert group is not None, f"Unmapped route {path}; requires owner review"
    _, module, binding = group
    module = "rust/src/" + module
    # Some aggregate owner paths moved; require actual existing authoritative file.
    aliases = {"rust/src/hub/models.rs": "rust/src/application/main_program/model_catalog.rs",
               "rust/src/hub/link_defaults.rs": "rust/src/application/link_defaults.rs",
               "rust/src/hub/attachments.rs": "rust/src/files/attachments.rs",
               "rust/src/memos/mod.rs": "rust/src/routine/memo.rs"}
    module = aliases.get(module, module)
    assert module in sources, f"Owner source missing for {path}: {module}"
    bound = binding is None or binding in main
    evidence = refs(path) if path != "/" else refs('request.path == "/"', [router_path])
    http.append({"path": path, "go_registration": row["registration"],
                 "go_handler": row["handler"], "go_contract": row.get("handler_ids", []),
                 "rust_owner": module, "rust_route_literal_refs": evidence,
                 "dispatch_owner_present": True,
                 "main_binding": {"state": "source_bound" if bound else "source_binding_missing",
                                  "constructor_or_builder": binding or "ServiceRouter shared ConfigStore/RuntimePaths",
                                  "refs": refs(binding, [main_path]) if binding else refs("ServiceRouter::", [main_path])},
                 "status": "source_implemented_bound_unaccepted" if bound else "owner_present_unbound_unaccepted",
                 "acceptance": {"native_live": "pending", "browser_device": "pending", "accepted": False},
                 "branch_limit": "Prefix/suffix and optional configuration branches require source-specific fixtures; lexical evidence does not prove every branch."})

ws = []
for family in ["handleWS", "uiLoopAtEpoch", "wrapperMessageLoop"]:
    record = services["websocket"][family]
    for label in record["case_labels"]:
        kind = label.strip('"')
        ws.append({"type": kind, "direction": "incoming", "go_consumer": record["source"],
                   "rust_consumer_refs": refs('"' + kind + '"', ["rust/src/hub/websocket.rs"]),
                   "main_binding_refs": refs("WebSocketService::new", [main_path]),
                   "state": "source_consumer_bound_unaccepted", "accepted": False})
false_literals = {"dir": "files_list.go file entry classification",
                  "file": "files_list.go file entry classification",
                  "text": "agent_chat_parse.go Anthropic content-block classification"}
for kind in services["websocket"]["emitted_type_literals"]:
    pattern = r'r#type:\s*"' + re.escape(kind) + '"'
    evidence = regex_refs(pattern,prod)
    internal_request = kind == "session_dismiss"
    ws.append({"type": kind, "direction": "outgoing_lexical_inventory",
               "classification": "not_a_ws_message_type" if kind in false_literals else
                                  ("internal_request_not_ws_emission" if internal_request else "ws_message"),
               "source_caution": false_literals.get(kind), "rust_producer_refs": evidence,
               "go_literal_refs":regex_refs(r'Type:\s*"'+re.escape(kind)+'"',go_sources),
               "state": "excluded_non_message_literal" if kind in false_literals else
                        ("source_consumer_present_no_outgoing_contract" if internal_request else
                         ("source_producer_present_unaccepted" if evidence else "producer_requires_manual_resolution")),
               "main_binding": "WebSocketService + canonical SessionEngine/CoreEffectSink + SessionWorkers/EventObserver; see producer source",
               "accepted": False})
assert len(ws) == 49, "Historical 49-row lexical protocol inventory changed"
assert all(r["rust_consumer_refs"] for r in ws if r["direction"] == "incoming")
# Meaningful production boundaries: reject regressions to path reopening or
# successful unsupported Start. These checks do not constitute fixture tests.
worker = sources["rust/src/application/session_workers.rs"]
assert ".read_forward_file(file," in worker and ".read_tail_file(file," in worker
assert ".read_forward(&path," not in worker and ".read_tail(&path," not in worker
assert "start_wrapped(request" in sources["rust/src/application/grid_spawn.rs"]
assert "with_startup_owner" in main and "with_registration_hook" in main
assert "SessionObservation::Subagents(tree)" in sources["rust/src/application/session_workers/observations.rs"]

cli_rows = []
for command in cli["commands"]:
    route=command.get("route")
    dispatch_file="rust/src/bin/many-ai-cli.rs"
    if command["name"]=="many-ai-cli-launcher":
        dispatch_file="rust/src/bin/many-ai-cli-launcher.rs"
        dispatch_refs=refs("run_launcher_invocation",[dispatch_file])
    elif route=="usage-relay":
        dispatch_refs=refs("usage_relay::run_native",[dispatch_file])
    else:
        variant="".join(part.title() for part in (route or "").split("-"))
        dispatch_refs=refs("Command::"+variant,[dispatch_file])
    assert dispatch_refs,f"CLI dispatch owner missing for {command['name']}"
    cli_rows.append({"name": command["name"], "go_binary": command.get("binary"),
                     "go_route": command.get("route"), "go_source": command.get("source"),
                     "rust_parse_source": "rust/src/cli.rs", "rust_dispatch_source":dispatch_file,
                     "dispatch_refs":dispatch_refs,
                     "dispatch_state": "source_dispatch_present_unaccepted",
                     "native_acceptance": "pending", "accepted": False})
snapshot = {"schema_version": 2, "generated_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "scope": "Current DERIVED recovery source snapshot; read-only source audit, not acceptance",
            "baseline_go_sha": services["baseline_go_sha"], "pinned_go_checkpoint": 951,
            "historical_inventory_preserved": {"path": str(historical.relative_to(ROOT)).replace("\\", "/"), "sha256": historical_hash},
            "counts": {"http_registrations": len(http), "ws_inventory_rows": len(ws),
                       "ws_non_message_literal_rows": len(false_literals), "cli_inventory_rows": len(cli_rows),
                       "native_browser_device_accepted": 0},
            "verification": verification,
            "http": http, "websocket": ws, "cli": cli_rows,
            "unresolved_or_compatibility_limits": [
                "Native/browser/device/provider/GUI acceptance remains pending for every route; no production installation, notification, account, quota or destructive uninstall acceptance performed.",
                "Instruction rule owner is composed in main with recover/bind before listening and fresh-token cleanup after draining; product vendor-file acceptance remains pending.",
                "Windows uninstall directory-capability/root ConfigStore handle-release fixes and Unix platform source ports have local synthetic integration tests; actual uninstall/GUI and other-OS acceptance remain pending.",
                "Trial deliberately rejects real host GUI dispatch, external approval-pattern fetch, arbitrary external PID termination, real push delivery, and vendor artifact aliases outside held selected root.",
                "PID-only recovery compatibility cannot prove a recovered process identity against PID reuse; trial refuses actual recovered PID kills.",
                "Codex/Grok native subagent readers have synthetic fixtures; broad independent native Go corpora for those readers remain pending.",
                "Optional services can explicitly return503 when required config/dependency is absent; this is not successful service acceptance.",
                "HTTP prefix registration rows do not enumerate every method/suffix branch; original inventory retains49 dynamic suffix entries separately.",
                "Historical WS lexical inventory contains dir/file/text DTO discriminator false positives; all49 original rows retained with three marked non-message.",
                "Historical outgoing session_dismiss literal is an internal relay handleDismiss request, not a Go WebSocket emission; canonical Rust UI request consumer and session_removed response remain present.",
                "Parent owns final full-suite/clippy receipts and archive/provenance closure; static binding evidence cannot replace these receipts."],
            "source_hashes": [{"path": p, "sha256": hashlib.sha256(s.encode()).hexdigest()}
                              for p, s in sources.items()]}
OUT.write_text(json.dumps(snapshot, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
assert hashlib.sha256(historical.read_bytes()).hexdigest() == historical_hash
print(json.dumps({"http": len(http), "ws_rows": len(ws), "non_ws_rows": len(false_literals),
                  "cli_rows": len(cli_rows), "accepted": 0, "historical_unchanged": True}))
