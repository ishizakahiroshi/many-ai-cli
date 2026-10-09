//! K01 lexical/dispatch contracts only. These tests do not start the application,
//! load a real home, run a provider, or assert that pending commands work.
use many_ai_cli::cli::{self, Command, LauncherInvocation};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::PathBuf};

fn source_oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/foundation/cli/source-oracle.json")).unwrap()
}
fn pure_oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/foundation/cli/pure-oracle.json")).unwrap()
}
fn inventory() -> Value {
    serde_json::from_str(include_str!("../inventory/cli.json")).unwrap()
}
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect()
}
fn route(command: Command) -> (&'static str, String, Vec<String>) {
    let no_args = |name| (name, String::new(), Vec::new());
    let delegated = |name, args| (name, String::new(), args);
    match command {
        Command::Default => no_args("default"),
        Command::Version => no_args("version"),
        Command::Help => no_args("help"),
        Command::Serve(args) => delegated("serve", args),
        Command::Connect(args) => delegated("connect", args),
        Command::Setup(args) => delegated("setup", args),
        Command::Doctor(args) => delegated("doctor", args),
        Command::Issue(args) => delegated("issue", args),
        Command::Provider(args) => delegated("provider", args),
        Command::Wrap { provider, args } => ("wrap", provider, args),
        Command::ShellInit => no_args("shell-init"),
        Command::Stop => no_args("stop"),
        Command::Status => no_args("status"),
        Command::Tray => no_args("tray"),
        Command::ProfileExport(args) => delegated("profile-export", args),
        Command::LogClean(args) => delegated("log-clean", args),
        Command::Uninstall(args) => delegated("uninstall", args),
        Command::UsageRelay(args) => delegated("usage-relay", args),
        Command::Orchestrate(args) => delegated("orchestrate", args),
    }
}

#[test]
fn every_main_dispatch_form_matches_source_ast_projection() {
    let oracle = source_oracle();
    let cases = oracle["dispatch_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 65);
    for case in cases {
        let args = strings(&case["args"]);
        let custom = strings(&case["custom_providers"]);
        let actual = cli::parse(&args, &custom);
        let expected_error = case["error"].as_str().unwrap();
        if !expected_error.is_empty() {
            assert_eq!(actual.unwrap_err(), expected_error, "{}", case["id"]);
            continue;
        }
        let invocation = actual.unwrap_or_else(|error| panic!("{}: {error}", case["id"]));
        assert!(invocation.trial.is_none());
        let (name, provider, forwarded) = route(invocation.command);
        assert_eq!(name, case["route"], "{}", case["id"]);
        assert_eq!(
            provider,
            case["provider"].as_str().unwrap_or(""),
            "{}",
            case["id"]
        );
        assert_eq!(forwarded, strings(&case["forwarded"]), "{}", case["id"]);
    }
}

#[test]
fn standalone_launcher_matches_real_go_flag_lexical_goldens() {
    let oracle = source_oracle();
    let cases: Vec<_> = oracle["flag_cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["flag_set"] == "many-ai-cli-launcher")
        .collect();
    assert!(cases.len() >= 60);
    for case in cases {
        let actual = cli::parse_launcher(&strings(&case["args"]));
        let error = case["error"].as_str().unwrap();
        let help = case["help"].as_bool().unwrap();
        if !error.is_empty() && !help {
            assert_eq!(actual.unwrap_err(), error, "{}", case["id"]);
        } else {
            let values = &case["values"];
            assert_eq!(
                actual.unwrap_or_else(|error| panic!("{}: {error}", case["id"])),
                LauncherInvocation {
                    profile: values["profile"].as_str().unwrap().into(),
                    use_last: values["last"].as_bool().unwrap(),
                    open_ui: values["ui"].as_bool().unwrap(),
                    help,
                },
                "{}",
                case["id"]
            );
        }
    }
}

#[test]
fn inventory_covers_both_binaries_every_source_flag_and_delegated_family() {
    let oracle = source_oracle();
    let inventory = inventory();
    assert_eq!(
        inventory["binaries"],
        json!(["many-ai-cli", "many-ai-cli-launcher"])
    );
    assert_eq!(
        inventory["builtin_providers"],
        json!(cli::BUILTIN_PROVIDERS)
    );
    assert_eq!(inventory["usage"], cli::USAGE);
    assert_eq!(inventory["manual_options"].as_array().unwrap().len(), 4);
    let hidden: BTreeSet<_> = inventory["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["hidden"] == true)
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(hidden, BTreeSet::from(["usage-relay", "orchestrate"]));
    let recorded = inventory["flag_sets"].as_array().unwrap();
    let source = oracle["flag_sets"].as_array().unwrap();
    assert_eq!(recorded.len(), 16);
    assert_eq!(recorded.len(), source.len());
    for expected in source {
        let found = recorded
            .iter()
            .find(|entry| entry["name"] == expected["name"])
            .unwrap();
        for key in ["source", "function", "line", "flags"] {
            assert_eq!(found[key], expected[key], "{} {key}", found["name"]);
        }
    }
    let expected: BTreeSet<_> = oracle["dispatch"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|entry| strings(&entry["names"]))
        .collect();
    let recorded: BTreeSet<_> = inventory["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| {
            let name = entry["name"].as_str().unwrap();
            (!name.starts_with('<') && name != "many-ai-cli-launcher").then(|| name.to_owned())
        })
        .collect();
    assert_eq!(recorded, expected);
    let families: BTreeSet<_> = inventory["subcommands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    for required in [
        "provider backup list",
        "provider backup verify",
        "provider backup restore",
        "provider reset",
        "provider recover",
        "provider recover --list",
        "orchestrate spawn",
        "orchestrate send",
        "orchestrate relay",
        "orchestrate relay status",
        "orchestrate relay stop",
        "usage-relay claude",
        "usage-relay codex",
        "wrap <provider>",
    ] {
        assert!(families.contains(required), "missing {required}");
    }
    assert!(
        inventory["entry_sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source["source"] == "cmd/many-ai-cli/provider_command.go")
    );
}

#[test]
fn lexical_observation_never_claims_runtime_integration() {
    let inventory = inventory();
    for command in inventory["commands"]
        .as_array()
        .unwrap()
        .iter()
        .chain(inventory["subcommands"].as_array().unwrap())
    {
        assert!(command["parse_status"].as_str().is_some());
        assert_eq!(command["runtime_status"], "pending");
    }
    assert_eq!(inventory["acceptance"]["runtime_integration"], "pending");
    assert_eq!(inventory["acceptance"]["native_os_acceptance"], "pending");
    let flags = inventory["flag_sets"].as_array().unwrap();
    for name in [
        "connect",
        "profile-export",
        "setup",
        "issue",
        "many-ai-cli-launcher",
    ] {
        let item = flags.iter().find(|item| item["name"] == name).unwrap();
        assert_eq!(item["help_process_exit"], 0);
    }
    for name in [
        "serve",
        "doctor",
        "log-clean",
        "uninstall",
        "usage-relay",
        "orchestrate spawn",
    ] {
        let item = flags.iter().find(|item| item["name"] == name).unwrap();
        assert_eq!(item["help_process_exit"], 1);
    }
    let wrap = flags.iter().find(|item| item["name"] == "wrap").unwrap();
    assert!(wrap["help_process_exit"].is_null());
    assert!(
        wrap["parse_error_handling"]
            .as_str()
            .unwrap()
            .starts_with("ignored")
    );
}

#[test]
fn delegated_goldens_pin_non_gnu_grammar_and_partial_failure_state() {
    let oracle = source_oracle();
    let cases = oracle["flag_cases"].as_array().unwrap();
    let find = |id: &str| cases.iter().find(|case| case["id"] == id).unwrap();
    let stop = find("wrap/positional-stops");
    assert_eq!(stop["remaining"], json!(["日本語", "--unknown"]));
    assert_eq!(stop["error"], "");
    let unknown = find("wrap/unknown");
    assert_eq!(unknown["error"], "flag provided but not defined: -unknown");
    assert_eq!(unknown["remaining"], json!([]));
    assert_eq!(find("serve/port-inline-010")["values"]["port"], 8);
    assert_eq!(find("serve/port-inline-0x10")["values"]["port"], 16);
    assert_eq!(find("serve/port-inline-1_000")["values"]["port"], 1000);
    assert_ne!(find("serve/port-inline-08")["error"], "");
    assert_eq!(find("connect/last-separate-false")["values"]["last"], true);
    assert_eq!(
        find("connect/last-separate-false")["remaining"],
        json!(["false", "--unknown"])
    );
    assert_eq!(
        find("many-ai-cli-launcher/profile-flag-value")["values"]["profile"],
        "--help"
    );
}

#[test]
fn provider_parse_fixture_stops_at_error_only_storage_boundary() {
    let oracle = pure_oracle();
    for family in ["provider_backup", "provider_reset", "provider_recover"] {
        for case in oracle["cases"][family].as_array().unwrap() {
            assert_ne!(case["error"], "");
            assert_eq!(case["stdout"], "");
            if case["boundary_call"] != "" {
                assert_eq!(
                    case["error"],
                    "oracle: storage boundary reached (not executed)"
                );
            }
        }
    }
    let cases = &oracle["cases"];
    let reset = cases["provider_reset"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["args"] == json!(["--distributed", "claude", "--expected-revision", ""]))
        .unwrap();
    assert_eq!(reset["boundary_call"], "Reset");
    assert_eq!(reset["boundary_args"], json!(["claude", ""]));
    let restore = cases["provider_backup"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["args"] == json!(["restore", "claude", "r1", "--expected-revision", ""]))
        .unwrap();
    assert_eq!(restore["boundary_call"], "");
    assert_eq!(restore["error"], "--expected-revision is required");
    let recover = cases["provider_recover"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["args"] == json!(["claude", "r1", "--list"]))
        .unwrap();
    assert_eq!(recover["boundary_call"], "RecoverHead");
    assert_eq!(recover["boundary_args"], json!(["claude", "r1"]));
}

#[test]
fn environment_inventory_includes_constants_dynamic_keys_and_implicit_reads() {
    let inventory = inventory();
    let reads = inventory["environment_reads"].as_array().unwrap();
    let names: BTreeSet<_> = reads
        .iter()
        .filter_map(|read| read["name"].as_str())
        .collect();
    let covered: BTreeSet<_> = inventory["environment_precedence"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|entry| strings(&entry["names"]))
        .collect();
    for name in &names {
        assert!(
            covered.contains(*name),
            "environment {name} lacks consumer/precedence rule"
        );
    }
    for name in [
        "MANY_AI_CLI_HUB_PORT",
        "MANY_AI_CLI_HUB_TOKEN",
        "MANY_AI_CLI_SESSION_ID",
        "MANY_AI_CLI_SPAWN_TIMEOUT",
        "MANY_AI_CLI_DELEGATION",
        "MANY_AI_CLI_PARENT_SHELL",
        "MANY_AI_CLI_SUBSCRIPTION_ID",
        "MANY_AI_CLI_AUTO",
        "GROK_HOME",
    ] {
        assert!(names.contains(name), "missed literal or constant {name}");
    }
    assert_eq!(
        reads
            .iter()
            .filter(|read| read.get("name").is_none())
            .count(),
        3
    );
    assert_eq!(
        inventory["dynamic_environment_reads"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        inventory["implicit_environment"][0]["names"],
        json!(["HOME", "USERPROFILE"])
    );
    assert!(
        inventory["environment_constants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["value"] == "MANY_AI_CLI=1")
    );
}

#[test]
fn source_function_fixtures_pin_env_validation_order_and_stdio() {
    let oracle = pure_oracle();
    let cases = &oracle["cases"];
    assert!(
        cases["hub_env"][0]["error"]
            .as_str()
            .unwrap()
            .contains("MANY_AI_CLI_SESSION_ID")
    );
    assert!(
        cases["hub_env"][5]["error"]
            .as_str()
            .unwrap()
            .contains("MANY_AI_CLI_HUB_PORT")
    );
    assert!(
        cases["hub_env"][6]["error"]
            .as_str()
            .unwrap()
            .contains("MANY_AI_CLI_HUB_TOKEN")
    );
    assert_eq!(cases["hub_env"][8]["url"], "http://127.0.0.1:not-a-number");
    assert_eq!(cases["hub_env"][8]["error"], "");
    assert_eq!(
        cases["spawn_timeout"][0]["nanoseconds"],
        300_000_000_000_u64
    );
    let yes = cases["issue_confirmation"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["stdin"] == " yes\n")
        .unwrap();
    assert_eq!(yes["confirmed"], false);
    let y = cases["issue_confirmation"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["stdin"] == " y \n")
        .unwrap();
    assert_eq!(y["confirmed"], true);
    assert_eq!(
        y["stderr"],
        "この内容で GitHub の Issue 作成画面を開きますか? [y/N]: "
    );
    let shell = cases["shell_init"][2]["stdout"].as_str().unwrap();
    assert!(shell.contains("safe-id(){ many-ai-cli safe-id \"$@\"; }"));
    assert!(!shell.contains("bad;id"));
    assert!(shell.contains("MANY_AI_CLI_AUTO:-0"));
}

#[test]
fn evidence_paths_anchors_and_referenced_go_tests_are_real() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let inventory = inventory();
    for group in [
        "behavior_contracts",
        "environment_precedence",
        "dynamic_environment_reads",
    ] {
        for item in inventory[group].as_array().unwrap() {
            let evidence = &item["evidence"];
            let text = fs::read_to_string(root.join(evidence["source"].as_str().unwrap())).unwrap();
            let anchor = evidence["anchor"].as_str().unwrap();
            let offset = text
                .find(anchor)
                .unwrap_or_else(|| panic!("{} missing anchor", item["id"]));
            assert_eq!(
                text[..offset].bytes().filter(|byte| *byte == b'\n').count() + 1,
                evidence["line"].as_u64().unwrap() as usize
            );
            if let Some(test) = evidence["baseline_test"].as_str() {
                let (path, name) = test.split_once("::").unwrap();
                assert!(
                    fs::read_to_string(root.join(path))
                        .unwrap()
                        .contains(&format!("func {name}("))
                );
            }
        }
    }
}
