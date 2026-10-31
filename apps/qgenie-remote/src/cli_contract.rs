//! These tests parse command lines and render help. They never dispatch an operation.
use super::Args;
use clap::{Command, CommandFactory, Parser};
use serde_json::{Value, json};
use std::path::Path;

fn collect(command: &mut Command, path: &str, reference: &mut String, shapes: &mut Vec<Value>) {
    *command = command
        .clone()
        .color(clap::ColorChoice::Never)
        .term_width(100)
        .mut_args(|arg| arg.hide_env_values(true));
    reference.push_str(&format!(
        "## `{path}`\n\n```text\n{}```\n\n",
        command.render_long_help()
    ));
    let arguments:Vec<_>=command.get_arguments().map(|arg| {
        let range=arg.get_num_args().unwrap_or_default();
        json!({"id":arg.get_id().as_str(),"long":arg.get_long(),"short":arg.get_short(),"index":arg.get_index(),
            "required":arg.is_required_set(),"action":format!("{:?}",arg.get_action()),"min_values":range.min_values(),"max_values":range.max_values(),
            "default":arg.get_default_values().iter().map(|v|v.to_string_lossy().to_string()).collect::<Vec<_>>(),
            "possible_values":arg.get_value_parser().possible_values().map(|values|values.map(|v|v.get_name().to_owned()).collect::<Vec<_>>())})
    }).collect();
    shapes.push(json!({"path":path,"subcommand_required":command.is_subcommand_required_set(),"arguments":arguments}));
    for subcommand in command.get_subcommands_mut() {
        if !subcommand.is_hide_set() && subcommand.get_name() != "help" {
            let path = format!("{path} {}", subcommand.get_name());
            collect(subcommand, &path, reference, shapes);
        }
    }
}
fn generated(path: &Path, text: &str) {
    if std::env::var_os("QGR_UPDATE_CLI_DOCS").as_deref() == Some(std::ffi::OsStr::new("1")) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    } else {
        assert_eq!(
            std::fs::read_to_string(path).unwrap_or_default(),
            text,
            "{} is stale; run QGR_UPDATE_CLI_DOCS=1 cargo test -p qgenie-remote --bin qgenie-remote cli_contract",
            path.display()
        );
    }
}
#[test]
fn cli_contract_reference_and_documented_examples() {
    let mut command = Args::command().bin_name("qgenie-remote");
    command.build();
    let mut reference = String::from(
        "# Command reference\n\nThis file is generated from the executable Clap definitions. Do not edit it by hand.\nSee [interface checks](interface-checks.md) for the update procedure and baseline policy.\n\n",
    );
    let mut shapes = Vec::new();
    collect(&mut command, "qgenie-remote", &mut reference, &mut shapes);
    let shape = format!(
        "{}\n",
        serde_json::to_string_pretty(&json!({"commands":shapes})).unwrap()
    );
    if let Some(path) = std::env::var_os("QGR_CAPTURE_CLI_BASELINE") {
        std::fs::write(path, shape).unwrap();
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    generated(&root.join("docs/integration/cli-reference.md"), &reference);
    generated(&root.join("conformance/cli/current.json"), &shape);
    let output = std::process::Command::new("python3")
        .arg(root.join("tools/cli_examples.py"))
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let examples: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        examples.len() >= 40,
        "documented command examples were not scanned"
    );
    for example in &examples {
        let args: Vec<_> = example["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        let parsed = Args::try_parse_from(args);
        assert!(
            parsed.is_ok(),
            "{}:{}: {}",
            example["file"],
            example["line"],
            parsed.err().unwrap()
        );
    }
    println!(
        "Validated {} documented commands without dispatch",
        examples.len()
    );
}
