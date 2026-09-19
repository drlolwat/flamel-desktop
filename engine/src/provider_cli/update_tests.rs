use super::*;
use std::os::unix::fs::PermissionsExt;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("flamel-cli-fixture-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn script(&self, path: &str, source: &str) -> PathBuf {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("#!/bin/sh\n{source}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    fn prior(&self) -> PathBuf {
        let path = self.script("codex/1.0.0/bin/codex", "exit 0");
        std::fs::write(self.0.join("codex/current"), "1.0.0").unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const CODEX_SUCCESS: &str = r#"
read -r input
case "$input" in *'"method":"initialize"'*) ;; *) exit 8;; esac
printf '%s\n' '{"id":"flamel-provider-compatibility","result":{"userAgent":"fixture/2.0.0"}}'
read -r initialized
"#;
const CLAUDE_SUCCESS: &str = r#"
read -r input
case "$input" in *'"subtype":"initialize"'*) ;; *) exit 8;; esac
printf '%s\n' '{"type":"control_response","response":{"subtype":"success","request_id":"flamel-provider-compatibility","response":{}}}'
"#;

#[tokio::test]
async fn accepts_actual_handshakes_for_both_protocols() {
    let fixture = Fixture::new();
    for (name, source) in [("codex", CODEX_SUCCESS), ("claude", CLAUDE_SUCCESS)] {
        let path = fixture.script(name, source);
        probe::verify(&path, name).await.unwrap();
    }
}

#[tokio::test]
async fn probes_use_an_isolated_home_without_provider_or_service_credentials() {
    let fixture = Fixture::new();
    let script = format!(
        r#"
test "$CODEX_HOME" = "$HOME/codex" || exit 3
test "$CLAUDE_CONFIG_DIR" = "$HOME/claude" || exit 3
test -z "$OPENAI_API_KEY$ANTHROPIC_API_KEY$CODEX_ACCESS_TOKEN$CLAUDE_CODE_OAUTH_TOKEN$FLAMEL_AUTH_TOKEN" || exit 3
test "$PWD" = "$HOME" || exit 3
{CODEX_SUCCESS}
"#
    );
    let binary = fixture.script("codex", &script);
    probe::verify(&binary, "codex").await.unwrap();
}

#[tokio::test]
#[ignore = "Explicit local installed-CLI handshake; no user prompt, credentials, or inference"]
async fn installed_provider_protocol_smoke() {
    for name in ["codex", "claude"] {
        let binary = resolve_executable(name)
            .expect("Requested provider must be installed for this smoke check");
        probe::verify(Path::new(&binary), name)
            .await
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[tokio::test]
async fn rejects_malformed_and_mismatched_replies_and_bounds_timeout() {
    let fixture = Fixture::new();
    for source in [
        "printf 'not-json\\n'",
        "printf '%s\\n' '{\"id\":\"wrong\",\"result\":{\"userAgent\":\"wrong\"}}'",
        "printf '%s\\n' '{\"id\":\"flamel-provider-compatibility\",\"error\":{\"code\":-1}}'",
    ] {
        let binary = fixture.script("codex", source);
        assert!(probe::verify(&binary, "codex").await.is_err());
    }
    let binary = fixture.script("slow", "exec sleep 5");
    let started = std::time::Instant::now();
    assert!(
        probe::verify_with_timeout(&binary, "codex", Duration::from_millis(40))
            .await
            .unwrap_err()
            .contains("timed out")
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn failed_install_keeps_prior_release_and_current_pointer() {
    let fixture = Fixture::new();
    let prior = fixture.prior();
    let npm = fixture.script(
        "npm",
        "case \"$1\" in view) printf '2.0.0';; install) exit 7;; *) exit 9;; esac",
    );
    assert!(
        update_one_using(&fixture.0, "codex", "@openai/codex", || Command::new(&npm))
            .await
            .is_err()
    );
    assert_eq!(
        managed_at(&fixture.0, "codex"),
        Some(prior.to_string_lossy().into_owned())
    );
    assert!(
        !std::fs::read_dir(fixture.0.join("codex"))
            .unwrap()
            .any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("staging-"))
    );
}

#[tokio::test]
async fn incompatible_existing_candidate_does_not_replace_working_cli() {
    let fixture = Fixture::new();
    let prior = fixture.prior();
    fixture.script("codex/2.0.0/bin/codex", "printf 'malformed\\n'");
    let npm = fixture.script("npm", "test \"$1\" = view || exit 9; printf '2.0.0'");
    assert!(
        update_one_using(&fixture.0, "codex", "@openai/codex", || Command::new(&npm))
            .await
            .is_err()
    );
    assert_eq!(
        managed_at(&fixture.0, "codex"),
        Some(prior.to_string_lossy().into_owned())
    );
}

#[tokio::test]
async fn compatible_candidate_is_promoted_without_deleting_previous_release() {
    let fixture = Fixture::new();
    let prior = fixture.prior();
    let candidate = fixture.script("codex/2.0.0/bin/codex", CODEX_SUCCESS);
    let npm = fixture.script("npm", "test \"$1\" = view || exit 9; printf '2.0.0'");
    update_one_using(&fixture.0, "codex", "@openai/codex", || Command::new(&npm))
        .await
        .unwrap();
    assert_eq!(
        managed_at(&fixture.0, "codex"),
        Some(candidate.to_string_lossy().into_owned())
    );
    assert!(prior.exists());
}
