use std::fs;
use std::path::Path;

fn main() {
    let bins = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin");
    let forbidden = [
        "command(\"claude\")",
        "command(\"codex\")",
        "Command::new(\"claude\")",
        "Command::new(\"codex\")",
    ];

    for entry in fs::read_dir(&bins).expect("read src/bin") {
        let path = entry.expect("read bin entry").path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with("_mcp.rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("read MCP source");
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "MCP server {name} launches a provider executable through {needle}; provider access belongs to the authenticated session runtime"
            );
        }
    }

    println!("mcp provider boundary checks passed");
}
