//! `smah tools list [--json]` — virtual-tool discovery (design decision D6).
//! Valid and invalid files both appear; invalid ones carry their reason
//! (also warned on stderr) and are never run.

use smah_core::virtual_tools::loader::{discover, ToolDef};

use crate::args::ToolsCommand;

/// Dispatch `smah tools <cmd>`; returns the process exit code.
pub fn run(cmd: &ToolsCommand) -> i32 {
    match cmd {
        ToolsCommand::List { json } => list(*json),
    }
}

fn list(json: bool) -> i32 {
    let (mut tools, invalid) = discover();
    tools.sort_by(|a, b| a.name.cmp(&b.name));

    for bad in &invalid {
        eprintln!(
            "smah: invalid tool file {}: {}",
            bad.path.display(),
            bad.reason
        );
    }

    if json {
        let rows = tools
            .iter()
            .map(|t| tool_row(t, "valid", String::new()))
            .chain(invalid.iter().map(
                |b| serde_json::json!({ "name": b.name, "status": "invalid", "reason": b.reason }),
            ))
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string())
        );
        return 0;
    }

    if tools.is_empty() && invalid.is_empty() {
        eprintln!(
            "smah: no virtual tools installed. Drop .md files into {} (see docs/ux/virtual-tools-cli-driver.md)",
            smah_core::virtual_tools::loader::tools_dir().display()
        );
        return 0;
    }

    println!("{:<20} {:<10} DESCRIPTION", "NAME", "STATUS");
    for tool in &tools {
        println!("{:<20} {:<10} {}", tool.name, "valid", tool.description);
    }
    for bad in &invalid {
        println!("{:<20} {:<10} {}", bad.name, "invalid", bad.reason);
    }
    0
}

fn tool_row(tool: &ToolDef, status: &str, reason: String) -> serde_json::Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "model": tool.model,
        "allow_roots": tool.allow_roots,
        "status": status,
        "reason": reason,
    })
}
