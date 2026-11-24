use std::{collections::BTreeSet, process::Command};

#[test]
fn api_only_build_resolves_no_telegram_or_reqwest_dependencies() {
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "--locked",
            "--offline",
            "-p",
            "market-app",
            "--no-default-features",
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let graph = String::from_utf8(output.stdout).unwrap();
    for dependency in ["market-telegram", "market-miniapp", "grammers-", "reqwest"] {
        assert!(
            !graph.lines().any(|line| line.starts_with(dependency)),
            "API-only graph includes {dependency}"
        );
    }
}

#[test]
fn production_dependencies_follow_the_service_boundaries() {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--locked",
            "--offline",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let boundaries: &[(&str, &[&str])] = &[
        (
            "market-app",
            &[
                "market-actors",
                "market-api",
                "market-hub-client",
                "market-miniapp",
                "market-media",
                "market-replica",
                "market-telegram",
                "market-telegram-client",
                "market-telegram-bot",
                "market-invocations",
                "market-valuation",
            ],
        ),
        (
            "market-api",
            &[
                "market-actors",
                "market-gifts",
                "market-miniapp",
                "market-media",
                "market-valuation",
            ],
        ),
        (
            "market-telegram",
            &[
                "market-telegram-client",
                "market-media",
                "market-invocations",
            ],
        ),
        (
            "market-telegram-client",
            &["market-gifts", "market-media", "market-invocations"],
        ),
        ("market-invocations", &[]),
        (
            "market-valuation",
            &[
                "market-replica",
                "market-gifts",
                "market-gifts-hub",
                "market-hub-protocol",
            ],
        ),
        (
            "market-portfolio",
            &["market-gifts", "market-invocations", "market-valuation"],
        ),
        (
            "market-telegram-bot",
            &[
                "market-telegram",
                "market-actors",
                "market-gifts",
                "market-invocations",
                "market-media",
                "market-valuation",
            ],
        ),
        ("market-media", &[]),
        ("market-miniapp", &[]),
        (
            "market-actors",
            &[
                "market-gifts",
                "market-gifts-hub",
                "market-replica",
                "market-hub-protocol",
            ],
        ),
        ("market-gifts", &[]),
        ("market-gifts-hub", &["market-gifts", "market-hub-protocol"]),
        (
            "market-replica",
            &["market-hub-client", "market-hub-protocol"],
        ),
        ("market-hub-client", &["market-hub-protocol"]),
        ("market-hub-protocol", &[]),
    ];
    for (name, allowed) in boundaries {
        let package = metadata["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|package| package["name"] == *name)
            .unwrap();
        if matches!(
            *name,
            "market-telegram-bot" | "market-invocations" | "market-valuation"
        ) {
            assert!(
                !package["dependencies"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|dependency| {
                        dependency["kind"].is_null()
                            && dependency["name"]
                                .as_str()
                                .is_some_and(|name| name.starts_with("grammers-"))
                    }),
                "{name} must remain independent of grammers"
            );
        }
        let dependencies: BTreeSet<_> = package["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|dependency| dependency["kind"].is_null())
            .filter_map(|dependency| dependency["name"].as_str())
            .filter(|dependency| dependency.starts_with("market-"))
            .collect();
        assert_eq!(
            dependencies,
            allowed.iter().copied().collect(),
            "{name} crossed its production dependency boundary"
        );
    }
}

#[test]
fn portfolio_floor_strategies_are_opt_in_for_portfolio_consumers() {
    for (package, enabled) in [("market-app", false), ("market-portfolio", true)] {
        let output = Command::new(env!("CARGO"))
            .args([
                "tree",
                "--locked",
                "--offline",
                "-p",
                package,
                "--all-features",
                "--edges",
                "normal",
                "--prefix",
                "none",
                "--format",
                "{p} features=[{f}]",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let graph = String::from_utf8(output.stdout).unwrap();
        let valuation = graph
            .lines()
            .find(|line| line.starts_with("market-valuation "))
            .expect("valuation dependency");
        assert_eq!(
            valuation.contains("portfolio"),
            enabled,
            "{package}: {valuation}"
        );
    }
}
