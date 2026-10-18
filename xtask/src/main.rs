use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, process::Command};

fn allowed_edges() -> BTreeMap<&'static str, Vec<&'static str>> {
    BTreeMap::from([
        ("proto", vec![]),
        ("transport", vec!["proto"]),
        ("policy", vec!["proto"]),
        ("adapters", vec!["proto"]),
        ("sessiond", vec!["proto", "transport", "policy", "adapters"]),
        ("host", vec!["proto", "transport", "policy"]),
        ("client", vec!["proto", "transport"]),
        ("tui", vec!["proto", "client"]),
        ("gateway", vec!["proto", "transport", "policy"]),
    ])
}

fn check_edges(metadata: &Value) -> Result<()> {
    let graph = allowed_edges();
    for package in metadata["packages"]
        .as_array()
        .context("missing packages")?
    {
        let name = package["name"].as_str().context("missing package name")?;
        let Some(owner) = name.strip_prefix("qgenie-remote-") else {
            continue;
        };
        let Some(allowed) = graph.get(owner) else {
            continue;
        };
        for dependency in package["dependencies"]
            .as_array()
            .context("missing dependencies")?
        {
            let dependency = dependency["name"]
                .as_str()
                .context("missing dependency name")?;
            if let Some(target) = dependency.strip_prefix("qgenie-remote-")
                && !allowed.contains(&target)
            {
                bail!("forbidden dependency: {name} -> {dependency}");
            }
            if owner == "gateway"
                && ["portable-pty", "keyring", "ratatui", "crossterm"].contains(&dependency)
            {
                bail!("gateway has forbidden execution/UI dependency {dependency}");
            }
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    match args.first().map(String::as_str) {
        Some("boundaries") => {
            let output = Command::new("cargo")
                .args(["metadata", "--format-version", "1", "--no-deps", "--locked"])
                .current_dir(root)
                .output()?;
            anyhow::ensure!(output.status.success(), "cargo metadata failed");
            check_edges(&serde_json::from_slice(&output.stdout)?)?;
            println!("Dependency boundaries passed");
        }
        Some("schema") | Some("rest-schema") | Some("recording-schema") => {
            let rest = args.first().is_some_and(|command| command == "rest-schema");
            let recording = args
                .first()
                .is_some_and(|command| command == "recording-schema");
            let path = root.join(if recording {
                "conformance/schema/recording-v1.json"
            } else if rest {
                "conformance/schema/rest-v1.json"
            } else {
                "conformance/schema/public-v1.json"
            });
            let expected = format!(
                "{}\n",
                serde_json::to_string_pretty(&if recording {
                    qgenie_remote_proto::recording_schema()
                } else if rest {
                    qgenie_remote_proto::rest_schema()
                } else {
                    qgenie_remote_proto::public_schema()
                })?
            );
            if args.iter().any(|arg| arg == "--check") {
                anyhow::ensure!(
                    std::fs::read_to_string(path)? == expected,
                    "schema differs; run cargo xtask schema"
                );
            } else {
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(path, expected)?;
            }
        }
        _ => bail!(
            "usage: cargo xtask boundaries | schema [--check] | rest-schema [--check] | recording-schema [--check]"
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_forbidden_edges_and_accepts_protocol_consumers() {
        let metadata = |dependency| json!({"packages":[{"name":"qgenie-remote-gateway","dependencies":[{"name":dependency}]}]});
        assert!(check_edges(&metadata("qgenie-remote-proto")).is_ok());
        assert!(check_edges(&metadata("qgenie-remote-adapters")).is_err());
        assert!(check_edges(&metadata("keyring")).is_err());
    }
}
