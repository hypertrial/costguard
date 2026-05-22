use costguard_platform::Platform;
use costguard_sql::{normalize_for_parse, try_parse_compiled_sql, try_parse_compiled_sql_error};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Write};

fn usage() -> ! {
    eprintln!("usage: audit-compiled-parse [--bucket] [--model NAME] [--json] MANIFEST.json");
    std::process::exit(2);
}

fn error_signature(error: &str) -> String {
    error
        .replace(|ch: char| ch.is_ascii_digit(), "N")
        .split_whitespace()
        .take(12)
        .collect::<Vec<_>>()
        .join(" ")
}

fn snippet_for_error(sql: &str, error: &str) -> String {
    if let Some(line) = error
        .split("Line: ")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .and_then(|value| value.trim().parse::<usize>().ok())
    {
        let lines: Vec<&str> = sql.lines().collect();
        let start = line.saturating_sub(3);
        let end = (line + 2).min(lines.len());
        return lines[start..end].join("\n");
    }
    sql.chars().take(400).collect()
}

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let mut bucket = false;
    let mut json = false;
    let mut model_filter: Option<String> = None;

    while let Some(arg) = args.first() {
        match arg.as_str() {
            "--bucket" => {
                bucket = true;
                args.remove(0);
            }
            "--json" => {
                json = true;
                args.remove(0);
            }
            "--model" => {
                args.remove(0);
                model_filter = Some(args.remove(0));
            }
            "--help" | "-h" => usage(),
            other if other.starts_with('-') => {
                eprintln!("unknown flag: {other}");
                usage();
            }
            _ => break,
        }
    }

    let manifest_path = args.first().map(String::as_str).unwrap_or_else(|| usage());
    let text = fs::read_to_string(manifest_path).expect("read manifest");
    let manifest: Value = serde_json::from_str(&text).expect("parse manifest");
    let nodes = manifest["nodes"]
        .as_object()
        .expect("manifest.nodes must be an object");

    let mut failures = Vec::new();
    let mut bucket_counts: HashMap<String, u32> = HashMap::new();

    for node in nodes.values() {
        if node["resource_type"] != "model" {
            continue;
        }
        let name = node["name"].as_str().unwrap_or("unknown");
        if let Some(filter) = &model_filter {
            if name != filter {
                continue;
            }
        }
        let Some(code) = node["compiled_code"].as_str() else {
            continue;
        };
        if try_parse_compiled_sql(code, Platform::Trino) {
            continue;
        }
        let normalized = normalize_for_parse(code, Platform::Trino);
        let error = try_parse_compiled_sql_error(code, Platform::Trino)
            .err()
            .unwrap_or_else(|| "unknown parse error".to_string());
        let signature = error_signature(&error);
        *bucket_counts.entry(signature.clone()).or_default() += 1;
        failures.push(serde_json::json!({
            "model": name,
            "error": error,
            "error_signature": signature,
            "snippet": snippet_for_error(&normalized, &error),
        }));
    }

    if bucket {
        let mut items: Vec<_> = bucket_counts.iter().collect();
        items.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
        for (signature, count) in items {
            println!("{count}\t{signature}");
        }
        eprintln!("fail={}", failures.len());
        return;
    }

    if json {
        let payload = serde_json::json!({
            "failures": failures.len(),
            "items": failures,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).expect("serialize json")
        );
        return;
    }

    for item in &failures {
        println!("=== {} ===", item["model"]);
        println!("ERR: {}", item["error"]);
        println!("{}", item["snippet"]);
        println!();
    }
    eprintln!("fail={}", failures.len());
    if !failures.is_empty() {
        io::stderr().flush().ok();
        std::process::exit(1);
    }
}
