//! Bundled Ponytail MCP instructions.
//!
//! Source: https://github.com/DietrichGebert/ponytail
//! Licensed under MIT; see resources/ponytail/LICENSE.

pub const ACTIVE_MARKER: &str = "PONYTAIL MODE ACTIVE — level:";
pub const DEFAULT_MODE: &str = "full";
pub const MODES: [&str; 3] = ["lite", "full", "ultra"];

pub fn resolve_mode(requested: Option<&str>) -> &'static str {
    match requested.map(str::trim) {
        Some("lite") => "lite",
        Some("ultra") => "ultra",
        _ => DEFAULT_MODE,
    }
}

pub fn build_instructions(requested: Option<&str>) -> String {
    let mode = resolve_mode(requested);
    let body = skill_body()
        .lines()
        .filter(|line| line_matches_mode(line, mode))
        .collect::<Vec<_>>()
        .join("\n");

    format!("PONYTAIL MODE ACTIVE — level: {mode}\n\n{}", body.trim())
}

fn skill_body() -> &'static str {
    let source = include_str!("../resources/ponytail/SKILL.md");
    let Some(frontmatter) = source.strip_prefix("---\n") else {
        return source;
    };
    frontmatter
        .split_once("\n---\n")
        .map(|(_, body)| body)
        .unwrap_or(source)
}

fn line_matches_mode(line: &str, active_mode: &str) -> bool {
    let trimmed = line.trim_start();
    for mode in MODES {
        if trimmed.starts_with(&format!("| **{mode}** |"))
            || trimmed.starts_with(&format!("- {mode}: \""))
        {
            return mode == active_mode;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_unknown_modes_to_full() {
        assert_eq!(resolve_mode(None), "full");
        assert_eq!(resolve_mode(Some("off")), "full");
        assert_eq!(resolve_mode(Some(" full ")), "full");
    }

    #[test]
    fn filters_mode_specific_rows_and_examples() {
        for mode in MODES {
            let instructions = build_instructions(Some(mode));
            assert!(instructions.starts_with(&format!("PONYTAIL MODE ACTIVE — level: {mode}")));
            for candidate in MODES {
                let row = format!("| **{candidate}** |");
                let example = format!("- {candidate}: \"");
                assert_eq!(
                    instructions.contains(&row),
                    candidate == mode,
                    "unexpected intensity row for {candidate} in {mode}"
                );
                assert_eq!(
                    instructions.contains(&example),
                    candidate == mode,
                    "unexpected example for {candidate} in {mode}"
                );
            }
        }
    }

    #[test]
    fn strips_skill_frontmatter_but_keeps_core_instructions() {
        let instructions = build_instructions(Some("full"));
        assert!(!instructions.contains("name: ponytail"));
        assert!(instructions.contains("# Ponytail"));
        assert!(instructions.contains("The shortest path to done is the right path."));
    }
}
