use crate::claude::provider::ProviderKind;
use crate::db::settings::UserSettings;
use crate::routes::providers::ProviderAvailability;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingIntent {
    Planning,
    Execution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRoutingDecision {
    pub requested_model: String,
    pub requested_reasoning_effort: String,
    pub provider: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub plan_reasoning_floor: Option<String>,
    pub intent: RoutingIntent,
    pub automatic: bool,
    pub fallback_used: bool,
    pub reason: String,
}

fn provider_ready(availability: ProviderAvailability, provider: ProviderKind) -> bool {
    match provider {
        ProviderKind::Claude => availability.claude_available && availability.claude_authenticated,
        ProviderKind::Codex => availability.codex_available && availability.codex_authenticated,
    }
}

fn detect_intent(message: &str, planning_mode: Option<&str>) -> RoutingIntent {
    if planning_mode.is_some_and(|mode| matches!(mode, "plan" | "ultraplan")) {
        return RoutingIntent::Planning;
    }
    let normalized = message.trim().to_ascii_lowercase();
    let asks_for_plan = normalized.starts_with("plan ")
        || normalized.starts_with("create a plan")
        || normalized.starts_with("make a plan")
        || normalized.starts_with("write a plan")
        || normalized.starts_with("audit and plan")
        || normalized.contains("go to plan mode");
    if asks_for_plan {
        RoutingIntent::Planning
    } else {
        RoutingIntent::Execution
    }
}

fn valid_effort(value: Option<&str>) -> Option<&str> {
    value.filter(|value| {
        matches!(
            *value,
            "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
        )
    })
}

pub(crate) fn effort_rank(value: &str) -> u8 {
    match value {
        "low" => 0,
        "medium" => 1,
        "high" => 2,
        "xhigh" => 3,
        "max" => 4,
        "ultra" => 5,
        _ => 1,
    }
}

pub(crate) fn automatic_reasoning(requested: Option<&str>) -> bool {
    valid_effort(requested).is_none()
}

fn effort_at_least<'a>(baseline: &'a str, floor: Option<&'a str>) -> &'a str {
    let Some(floor) = valid_effort(floor) else {
        return baseline;
    };
    if effort_rank(floor) > effort_rank(baseline) {
        floor
    } else {
        baseline
    }
}

fn planning_effort<'a>(settings: &'a UserSettings, provider: ProviderKind) -> &'a str {
    let configured = match provider {
        ProviderKind::Codex => settings.codex_planning_reasoning_effort.as_str(),
        ProviderKind::Claude => settings.claude_planning_reasoning_effort.as_str(),
    };
    valid_effort(Some(configured))
        .filter(|effort| matches!(*effort, "high" | "xhigh" | "max" | "ultra"))
        .unwrap_or("xhigh")
}

fn model_effort_ceiling(model: &str) -> &'static str {
    match model {
        "gpt-6-astra" | "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6" => "ultra",
        "gpt-5.6-luna" | "claude-opus-4-6" | "claude-opus-4-7" => "max",
        "claude-haiku-4-5" => "high",
        "claude-sonnet-4-6" => "xhigh",
        _ => "xhigh",
    }
}

fn effort_for_model<'a>(model: &str, desired: &'a str) -> &'a str {
    let ceiling = model_effort_ceiling(model);
    if effort_rank(desired) > effort_rank(ceiling) {
        ceiling
    } else {
        desired
    }
}

pub fn resolve_model_route(
    requested_model: Option<&str>,
    requested_reasoning_effort: Option<&str>,
    message: &str,
    planning_mode: Option<&str>,
    plan_reasoning_floor: Option<&str>,
    settings: &UserSettings,
    availability: ProviderAvailability,
) -> Result<ModelRoutingDecision, String> {
    let requested_model = requested_model.unwrap_or("auto").trim();
    let requested_model = if requested_model.is_empty() {
        "auto"
    } else {
        requested_model
    };
    let requested_effort = requested_reasoning_effort.unwrap_or("auto").trim();
    let requested_effort = if requested_effort.is_empty() {
        "auto"
    } else {
        requested_effort
    };
    let intent = detect_intent(message, planning_mode);
    // Model Auto and reasoning Auto are independent controls. A manually
    // selected effort is authoritative even if model selection is automatic.
    let plan_reasoning_floor = automatic_reasoning(requested_reasoning_effort)
        .then_some(plan_reasoning_floor)
        .flatten();

    if requested_model != "auto" {
        let provider = ProviderKind::from_model(requested_model);
        if !provider_ready(availability, provider) {
            return Err(format!(
                "{} is not available and authenticated for this account",
                provider.as_str()
            ));
        }
        let reasoning_effort = if let Some(manual) = valid_effort(requested_reasoning_effort) {
            Some(effort_for_model(requested_model, manual).to_string())
        } else {
            match intent {
                RoutingIntent::Planning => {
                    let desired =
                        effort_at_least(planning_effort(settings, provider), plan_reasoning_floor);
                    Some(effort_for_model(requested_model, desired).to_string())
                }
                RoutingIntent::Execution => {
                    let desired = effort_at_least("medium", plan_reasoning_floor);
                    Some(effort_for_model(requested_model, desired).to_string())
                }
            }
        };
        return Ok(ModelRoutingDecision {
            requested_model: requested_model.to_string(),
            requested_reasoning_effort: requested_effort.to_string(),
            provider: provider.as_str().to_string(),
            model: requested_model.to_string(),
            reasoning_effort,
            plan_reasoning_floor: valid_effort(plan_reasoning_floor).map(str::to_string),
            intent,
            automatic: false,
            fallback_used: false,
            reason: if intent == RoutingIntent::Planning && requested_effort == "auto" {
                "The explicitly selected model used the account's capability-adjusted automatic planning profile."
                    .to_string()
            } else {
                "The explicitly selected model was used without provider fallback.".to_string()
            },
        });
    }

    let preferred =
        ProviderKind::from_str(&settings.preferred_provider).unwrap_or(ProviderKind::Codex);
    let alternate = match preferred {
        ProviderKind::Claude => ProviderKind::Codex,
        ProviderKind::Codex => ProviderKind::Claude,
    };
    let (provider, fallback_used) = if provider_ready(availability, preferred) {
        (preferred, false)
    } else if settings.cross_provider_fallback && provider_ready(availability, alternate) {
        (alternate, true)
    } else {
        return Err(if settings.cross_provider_fallback {
            "No configured AI provider is both installed and authenticated".to_string()
        } else {
            format!(
                "Preferred provider {} is unavailable and cross-provider fallback is disabled",
                preferred.as_str()
            )
        });
    };

    let manual_reasoning_effort = valid_effort(requested_reasoning_effort);
    let (model, automatic_reasoning_effort) = match (provider, intent) {
        (ProviderKind::Codex, RoutingIntent::Planning) => {
            let model = "gpt-5.6-sol";
            let desired =
                effort_at_least(planning_effort(settings, provider), plan_reasoning_floor);
            (
                model.to_string(),
                Some(effort_for_model(model, desired).to_string()),
            )
        }
        (ProviderKind::Codex, RoutingIntent::Execution) => {
            let model = "gpt-5.6-sol";
            let desired = effort_at_least("medium", plan_reasoning_floor);
            (
                model.to_string(),
                Some(effort_for_model(model, desired).to_string()),
            )
        }
        (ProviderKind::Claude, RoutingIntent::Planning) => {
            let model = "claude-opus-4-7";
            let desired =
                effort_at_least(planning_effort(settings, provider), plan_reasoning_floor);
            (
                model.to_string(),
                Some(effort_for_model(model, desired).to_string()),
            )
        }
        (ProviderKind::Claude, RoutingIntent::Execution)
            if valid_effort(plan_reasoning_floor)
                .is_some_and(|floor| effort_rank(floor) >= effort_rank("high")) =>
        {
            (
                "claude-opus-4-7".to_string(),
                Some(effort_at_least("medium", plan_reasoning_floor).to_string()),
            )
        }
        (ProviderKind::Claude, RoutingIntent::Execution) => {
            ("claude-sonnet-4-6".to_string(), Some("medium".into()))
        }
    };
    let reasoning_effort = match (provider, manual_reasoning_effort) {
        (_, Some(manual)) => Some(effort_for_model(&model, manual).to_string()),
        _ => automatic_reasoning_effort.map(|effort| effort_for_model(&model, &effort).to_string()),
    };
    let reason = if manual_reasoning_effort.is_some() {
        if fallback_used {
            "Automatic model routing selected the available fallback and preserved the explicit reasoning selection."
        } else {
            "Automatic model routing selected the preferred provider and preserved the explicit reasoning selection."
        }
    } else {
        match (intent, fallback_used) {
            (RoutingIntent::Planning, false) => {
                "Planning intent selected the strongest planning profile on the preferred provider."
            }
            (RoutingIntent::Planning, true) => {
                "Planning intent selected the strongest available fallback because the preferred provider was unavailable."
            }
            (RoutingIntent::Execution, false) if valid_effort(plan_reasoning_floor).is_some() => {
                "Implementation intent honored the active Plan item's planner-selected reasoning floor on the preferred provider."
            }
            (RoutingIntent::Execution, false) => {
                "Implementation intent selected the balanced execution profile on the preferred provider."
            }
            (RoutingIntent::Execution, true) => {
                "Implementation intent selected the balanced available fallback because the preferred provider was unavailable."
            }
        }
    };
    Ok(ModelRoutingDecision {
        requested_model: "auto".to_string(),
        requested_reasoning_effort: requested_effort.to_string(),
        provider: provider.as_str().to_string(),
        model,
        reasoning_effort,
        plan_reasoning_floor: valid_effort(plan_reasoning_floor).map(str::to_string),
        intent,
        automatic: true,
        fallback_used,
        reason: reason.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(provider: &str, fallback: bool) -> UserSettings {
        UserSettings {
            git_name: None,
            git_email: None,
            git_signing_key: None,
            preferred_provider: provider.to_string(),
            cross_provider_fallback: fallback,
            codex_planning_reasoning_effort: "xhigh".to_string(),
            claude_planning_reasoning_effort: "xhigh".to_string(),
        }
    }

    fn settings_with_planning_effort(
        provider: &str,
        codex_effort: &str,
        claude_effort: &str,
    ) -> UserSettings {
        UserSettings {
            codex_planning_reasoning_effort: codex_effort.to_string(),
            claude_planning_reasoning_effort: claude_effort.to_string(),
            ..settings(provider, true)
        }
    }

    fn all_ready() -> ProviderAvailability {
        ProviderAvailability {
            claude_available: true,
            claude_authenticated: true,
            codex_available: true,
            codex_authenticated: true,
        }
    }

    #[test]
    fn auto_routes_planning_and_execution_differently() {
        let planning = resolve_model_route(
            Some("auto"),
            None,
            "create a plan",
            Some("ultraplan"),
            None,
            &settings("codex", true),
            all_ready(),
        )
        .unwrap();
        let execution = resolve_model_route(
            Some("auto"),
            None,
            "implement the plan",
            None,
            None,
            &settings("codex", true),
            all_ready(),
        )
        .unwrap();
        assert_eq!(planning.reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(execution.reasoning_effort.as_deref(), Some("medium"));
    }

    #[test]
    fn explicit_models_never_cross_provider_fallback() {
        let availability = ProviderAvailability {
            claude_available: true,
            claude_authenticated: true,
            codex_available: false,
            codex_authenticated: false,
        };
        assert!(
            resolve_model_route(
                Some("gpt-5.6-sol"),
                Some("high"),
                "work",
                None,
                None,
                &settings("codex", true),
                availability,
            )
            .is_err()
        );
    }

    #[test]
    fn explicit_codex_model_uses_intent_when_reasoning_is_auto() {
        let planning = resolve_model_route(
            Some("gpt-5.6-sol"),
            None,
            "create a plan for the release system",
            None,
            None,
            &settings("codex", true),
            all_ready(),
        )
        .unwrap();
        let execution = resolve_model_route(
            Some("gpt-5.6-sol"),
            None,
            "implement the release system",
            None,
            None,
            &settings("codex", true),
            all_ready(),
        )
        .unwrap();

        assert_eq!(planning.model, "gpt-5.6-sol");
        assert_eq!(planning.reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(execution.model, "gpt-5.6-sol");
        assert_eq!(execution.reasoning_effort.as_deref(), Some("medium"));
    }

    #[test]
    fn explicit_auto_planning_uses_the_provider_preference() {
        let planning = resolve_model_route(
            Some("gpt-5.6-sol"),
            None,
            "create a plan",
            None,
            None,
            &settings_with_planning_effort("codex", "ultra", "high"),
            all_ready(),
        )
        .unwrap();

        assert_eq!(planning.reasoning_effort.as_deref(), Some("ultra"));
    }

    #[test]
    fn non_frontier_models_cap_the_automatic_planning_profile() {
        let codex = resolve_model_route(
            Some("gpt-5.4-mini"),
            None,
            "create a plan",
            None,
            None,
            &settings_with_planning_effort("codex", "ultra", "ultra"),
            all_ready(),
        )
        .unwrap();
        let claude = resolve_model_route(
            Some("claude-haiku-4-5"),
            None,
            "create a plan",
            None,
            None,
            &settings_with_planning_effort("claude", "ultra", "ultra"),
            all_ready(),
        )
        .unwrap();

        assert_eq!(codex.reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(claude.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn explicit_reasoning_overrides_auto_intent_and_plan_floor() {
        let planning = resolve_model_route(
            Some("gpt-5.6-sol"),
            Some("low"),
            "create a plan",
            Some("ultraplan"),
            Some("ultra"),
            &settings("codex", true),
            all_ready(),
        )
        .unwrap();

        assert_eq!(planning.reasoning_effort.as_deref(), Some("low"));
        assert_eq!(planning.plan_reasoning_floor, None);
    }

    #[test]
    fn manual_high_is_not_overridden_by_an_xhigh_plan_in_either_model_mode() {
        for model in ["gpt-6-astra", "auto"] {
            let decision = resolve_model_route(
                Some(model),
                Some("high"),
                "finish the plan",
                None,
                Some("xhigh"),
                &settings("codex", true),
                all_ready(),
            )
            .unwrap();
            assert_eq!(decision.reasoning_effort.as_deref(), Some("high"));
            assert_eq!(decision.requested_reasoning_effort, "high");
            assert_eq!(decision.plan_reasoning_floor, None);
        }
    }

    #[test]
    fn auto_raises_itself_after_a_plan_floor_change_without_flipping_selection() {
        let first = resolve_model_route(
            Some("gpt-6-astra"),
            Some("auto"),
            "finish the plan",
            None,
            Some("high"),
            &settings("codex", true),
            all_ready(),
        )
        .unwrap();
        let next = resolve_model_route(
            Some("gpt-6-astra"),
            Some("auto"),
            "finish the plan",
            None,
            Some("xhigh"),
            &settings("codex", true),
            all_ready(),
        )
        .unwrap();
        assert_eq!(first.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(next.reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(next.requested_reasoning_effort, "auto");
        assert_eq!(next.model, "gpt-6-astra");
    }

    #[test]
    fn explicit_auto_reasoning_honors_active_plan_floor() {
        let execution = resolve_model_route(
            Some("gpt-5.6-sol"),
            None,
            "continue implementation",
            None,
            Some("max"),
            &settings("codex", true),
            all_ready(),
        )
        .unwrap();

        assert_eq!(execution.reasoning_effort.as_deref(), Some("max"));
    }

    #[test]
    fn auto_fallback_obeys_account_policy() {
        let availability = ProviderAvailability {
            claude_available: true,
            claude_authenticated: true,
            codex_available: false,
            codex_authenticated: false,
        };
        let fallback = resolve_model_route(
            Some("auto"),
            None,
            "work",
            None,
            None,
            &settings("codex", true),
            availability,
        )
        .unwrap();
        assert_eq!(fallback.provider, "claude");
        assert!(fallback.fallback_used);
        assert!(
            resolve_model_route(
                Some("auto"),
                None,
                "work",
                None,
                None,
                &settings("codex", false),
                availability,
            )
            .is_err()
        );
    }

    #[test]
    fn auto_execution_never_routes_below_the_plan_floor() {
        let decision = resolve_model_route(
            Some("auto"),
            None,
            "implement the active item",
            None,
            Some("high"),
            &settings("codex", true),
            all_ready(),
        )
        .unwrap();
        assert_eq!(decision.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(decision.plan_reasoning_floor.as_deref(), Some("high"));
    }

    #[test]
    fn auto_model_preserves_max_reasoning_across_routing() {
        let decision = resolve_model_route(
            Some("auto"),
            Some("max"),
            "implement the active item",
            None,
            Some("max"),
            &settings("codex", true),
            all_ready(),
        )
        .unwrap();

        assert_eq!(decision.requested_reasoning_effort, "max");
        assert_eq!(decision.reasoning_effort.as_deref(), Some("max"));
    }

    #[test]
    fn high_floor_promotes_automatic_claude_execution_to_opus() {
        let decision = resolve_model_route(
            Some("auto"),
            None,
            "implement the active item",
            None,
            Some("high"),
            &settings("claude", true),
            all_ready(),
        )
        .unwrap();
        assert_eq!(decision.model, "claude-opus-4-7");
    }

    #[test]
    fn manual_effort_selection_remains_an_explicit_override() {
        let decision = resolve_model_route(
            Some("gpt-5.6-sol"),
            Some("low"),
            "implement the active item",
            None,
            Some("high"),
            &settings("codex", true),
            all_ready(),
        )
        .unwrap();
        assert_eq!(decision.reasoning_effort.as_deref(), Some("low"));
    }

    #[test]
    fn claude_reports_the_native_ceiling_without_losing_the_requested_profile() {
        for model in ["auto", "claude-opus-4-7"] {
            for effort in ["auto", "ultra"] {
                let decision = resolve_model_route(
                    Some(model),
                    Some(effort),
                    "create a plan",
                    None,
                    Some("ultra"),
                    &settings_with_planning_effort("claude", "ultra", "ultra"),
                    all_ready(),
                )
                .unwrap();
                assert_eq!(decision.reasoning_effort.as_deref(), Some("max"));
                assert_eq!(decision.requested_reasoning_effort, effort);
            }
        }
    }

    #[test]
    fn explicit_codex_with_auto_effort_honors_the_plan_floor() {
        let decision = resolve_model_route(
            Some("gpt-5.6-sol"),
            None,
            "implement the active item",
            None,
            Some("xhigh"),
            &settings("codex", true),
            all_ready(),
        )
        .unwrap();
        assert_eq!(decision.reasoning_effort.as_deref(), Some("xhigh"));
    }
}
