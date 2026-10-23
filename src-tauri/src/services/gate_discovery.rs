//! What a project can be checked with — read from its own manifests, never
//! guessed from the model (the Vercel "verification contract" idea: the
//! harness names the gates, so "tests: not run" is a fact, not an opinion).
//! Used by the `verification` event (F1c) to say which gates the turn did
//! and did not exercise, and later by "ask for evidence" to type the right
//! command.

use std::path::Path;

/// (kind, command) — kind ∈ test · lint · typecheck · build · format.
pub type Gate = (String, String);

fn package_manager(dir: &Path) -> &'static str {
    if dir.join("pnpm-lock.yaml").exists() {
        "pnpm"
    } else if dir.join("yarn.lock").exists() {
        "yarn"
    } else if dir.join("bun.lockb").exists() || dir.join("bun.lock").exists() {
        "bun"
    } else {
        "npm"
    }
}

/// Gates in the fixed order test, lint, typecheck, build, format; one per
/// kind (the first manifest that names it wins). Empty when nothing is known.
pub fn discover(dir: &Path) -> Vec<Gate> {
    let mut found: Vec<Gate> = Vec::new();
    let mut add = |kind: &str, cmd: String| {
        if !found.iter().any(|(k, _)| k == kind) {
            found.push((kind.to_string(), cmd));
        }
    };
    if let Ok(txt) = std::fs::read_to_string(dir.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
            let pm = package_manager(dir);
            let run = |s: &str| {
                if s == "test" {
                    format!("{pm} test")
                } else {
                    format!("{pm} run {s}")
                }
            };
            if let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) {
                let has = |k: &str| scripts.contains_key(k);
                if has("test") {
                    add("test", run("test"));
                }
                if has("lint") {
                    add("lint", run("lint"));
                }
                if has("typecheck") {
                    add("typecheck", run("typecheck"));
                } else if has("check") {
                    add("typecheck", run("check"));
                }
                if has("build") {
                    add("build", run("build"));
                }
                if has("format:check") {
                    add("format", run("format:check"));
                }
            }
        }
    }
    if dir.join("Cargo.toml").exists() {
        add("test", "cargo test".into());
        add("lint", "cargo clippy -- -D warnings".into());
        add("typecheck", "cargo check".into());
        add("format", "cargo fmt --check".into());
    }
    if dir.join("pyproject.toml").exists()
        || dir.join("pytest.ini").exists()
        || dir.join("setup.cfg").exists()
        || dir.join("tox.ini").exists()
    {
        add("test", "pytest".into());
    }
    if dir.join("go.mod").exists() {
        add("test", "go test ./...".into());
        add("lint", "go vet ./...".into());
        add("build", "go build ./...".into());
    }
    if let Ok(mk) = std::fs::read_to_string(dir.join("Makefile")) {
        let target = |t: &str| {
            mk.lines()
                .any(|l| l.starts_with(&format!("{t}:")) && !l.starts_with(&format!("{t}:=")))
        };
        if target("test") {
            add("test", "make test".into());
        }
        if target("lint") {
            add("lint", "make lint".into());
        }
        if target("check") {
            add("typecheck", "make check".into());
        }
        if target("build") {
            add("build", "make build".into());
        }
    }
    let order = ["test", "lint", "typecheck", "build", "format"];
    found.sort_by_key(|(k, _)| order.iter().position(|o| o == k).unwrap_or(99));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_name_the_gates_in_a_fixed_order_one_per_kind() {
        let dir = std::env::temp_dir().join(format!("ac-gates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(discover(&dir).is_empty());
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"test":"vitest run","lint":"eslint .","build":"vite build","format:check":"prettier --check ."}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname=\"x\"").unwrap();
        std::fs::write(dir.join("Makefile"), "build:\n\techo\ntest:\n\techo\n").unwrap();
        let gates = discover(&dir);
        assert_eq!(
            gates,
            vec![
                ("test".to_string(), "pnpm test".to_string()),
                ("lint".to_string(), "pnpm run lint".to_string()),
                ("typecheck".to_string(), "cargo check".to_string()),
                ("build".to_string(), "pnpm run build".to_string()),
                ("format".to_string(), "pnpm run format:check".to_string()),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
