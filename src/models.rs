//! The list of models offered for `claude --model`.
//!
//! ccsm ships no hard-coded model catalogue beyond the tier aliases, because a
//! release-pinned list goes stale the moment Claude Code adds or retires a
//! model. Instead the picker is assembled at runtime from two places:
//!
//! 1. The tier aliases (`opus`, `sonnet`, `haiku`, `fable`). Claude resolves
//!    these to whatever the current model for that tier is, so they stay
//!    correct across model launches without ccsm knowing anything.
//! 2. Concrete model ids read out of `~/.claude.json`, which Claude Code itself
//!    maintains: `additionalModelOptionsCache` (the extra entries it shows in
//!    its own model picker) and `projects.*.lastModelUsage` (every model that
//!    has actually billed tokens). New models appear here on their own; retired
//!    ones stop appearing once Claude Code drops them.

use serde::Deserialize;

/// One selectable entry in the model picker.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelOption {
    /// Value passed to `claude --model`. Empty means don't pass the flag at all.
    pub value: String,
    /// Short label shown in the picker row.
    pub label: String,
    /// One-line explanation shown beneath the picker.
    pub description: String,
}

impl ModelOption {
    /// Build an option from its three parts.
    fn new(value: &str, label: &str, description: &str) -> Self {
        Self {
            value: value.to_string(),
            label: label.to_string(),
            description: description.to_string(),
        }
    }
}

/// Shape of the entries Claude Code caches in `additionalModelOptionsCache`.
#[derive(Debug, Deserialize)]
struct CachedOption {
    #[serde(default)]
    value: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    description: String,
}

/// The always-available head of the list: claude's own default, followed by the
/// tier aliases that track the latest model in each tier.
fn tier_aliases() -> Vec<ModelOption> {
    vec![
        ModelOption::new(
            "",
            "(claude default)",
            "Let Claude Code pick, honouring your own model setting.",
        ),
        ModelOption::new(
            "opus",
            "opus",
            "Latest Opus. Most capable general-purpose tier.",
        ),
        ModelOption::new(
            "sonnet",
            "sonnet",
            "Latest Sonnet. Balanced speed and capability.",
        ),
        ModelOption::new(
            "haiku",
            "haiku",
            "Latest Haiku. Fastest and cheapest tier.",
        ),
        ModelOption::new(
            "fable",
            "fable",
            "Latest Fable. Built for long-running, hardest tasks.",
        ),
    ]
}

/// Path to Claude Code's own state file, `~/.claude.json`.
fn claude_json_path() -> Option<std::path::PathBuf> {
    Some(dirs::home_dir()?.join(".claude.json"))
}

/// Extract concrete model ids from a parsed `~/.claude.json`.
///
/// Pure so the discovery rules can be tested without a home directory. Ids are
/// returned in a stable order: Claude Code's own cached picker options first
/// (they carry real labels), then anything else seen in project usage, sorted.
fn discovered_from_json(root: &serde_json::Value) -> Vec<ModelOption> {
    let mut out: Vec<ModelOption> = Vec::new();

    if let Some(cached) = root.get("additionalModelOptionsCache") {
        if let Ok(entries) = serde_json::from_value::<Vec<CachedOption>>(cached.clone()) {
            for entry in entries {
                if entry.value.is_empty() {
                    continue;
                }
                let label = if entry.label.is_empty() {
                    entry.value.clone()
                } else {
                    format!("{} ({})", entry.label, entry.value)
                };
                let description = if entry.description.is_empty() {
                    "Offered by Claude Code.".to_string()
                } else {
                    entry.description
                };
                out.push(ModelOption::new(&entry.value, &label, &description));
            }
        }
    }

    let mut used: Vec<String> = Vec::new();
    if let Some(projects) = root.get("projects").and_then(|p| p.as_object()) {
        for project in projects.values() {
            let Some(usage) = project.get("lastModelUsage").and_then(|u| u.as_object()) else {
                continue;
            };
            for id in usage.keys() {
                if !id.is_empty() && !used.contains(id) {
                    used.push(id.clone());
                }
            }
        }
    }
    used.sort();
    for id in used {
        if out.iter().any(|o| o.value == id) {
            continue;
        }
        out.push(ModelOption::new(&id, &id, "Seen in your Claude Code usage."));
    }

    out
}

/// The full picker list: tier aliases followed by whatever concrete models
/// Claude Code currently knows about. Falls back to the aliases alone when
/// `~/.claude.json` is missing or unreadable.
pub fn available() -> Vec<ModelOption> {
    let mut options = tier_aliases();
    let discovered = claude_json_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .map(|root| discovered_from_json(&root))
        .unwrap_or_default();
    for option in discovered {
        if !options.iter().any(|o| o.value == option.value) {
            options.push(option);
        }
    }
    options
}

/// Position of `value` in `options`, or 0 when it isn't one of them (which is
/// how a hand-typed custom id behaves: cycling from it starts over at the top).
pub fn index_of(options: &[ModelOption], value: &str) -> usize {
    options.iter().position(|o| o.value == value).unwrap_or(0)
}

/// Human-readable label for a stored model value, for detail panes. A value
/// that isn't in the list is shown verbatim, since it was typed by hand.
pub fn label_for(options: &[ModelOption], value: &str) -> String {
    options
        .iter()
        .find(|o| o.value == value)
        .map(|o| o.label.clone())
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_aliases_lead_with_the_claude_default() {
        let options = tier_aliases();
        assert_eq!(options[0].value, "");
        assert!(options.iter().any(|o| o.value == "opus"));
        assert!(options.iter().any(|o| o.value == "fable"));
    }

    #[test]
    fn discovers_cached_picker_options_with_their_labels() {
        let root = serde_json::json!({
            "additionalModelOptionsCache": [
                {"value": "claude-fable-5[1m]", "label": "Fable", "description": "Long tasks"}
            ]
        });
        let found = discovered_from_json(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].value, "claude-fable-5[1m]");
        assert_eq!(found[0].label, "Fable (claude-fable-5[1m])");
        assert_eq!(found[0].description, "Long tasks");
    }

    #[test]
    fn discovers_models_from_project_usage_sorted_and_deduped() {
        let root = serde_json::json!({
            "projects": {
                "/a": {"lastModelUsage": {"claude-opus-5": {}, "claude-sonnet-5": {}}},
                "/b": {"lastModelUsage": {"claude-opus-5": {}, "claude-haiku-4-5": {}}}
            }
        });
        let found = discovered_from_json(&root);
        let values: Vec<&str> = found.iter().map(|o| o.value.as_str()).collect();
        assert_eq!(values, vec!["claude-haiku-4-5", "claude-opus-5", "claude-sonnet-5"]);
    }

    #[test]
    fn cached_options_win_over_usage_entries_for_the_same_id() {
        let root = serde_json::json!({
            "additionalModelOptionsCache": [
                {"value": "claude-opus-5", "label": "Opus", "description": "Cached"}
            ],
            "projects": {"/a": {"lastModelUsage": {"claude-opus-5": {}}}}
        });
        let found = discovered_from_json(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].description, "Cached");
    }

    #[test]
    fn malformed_json_yields_no_discoveries_rather_than_panicking() {
        assert!(discovered_from_json(&serde_json::json!({})).is_empty());
        assert!(discovered_from_json(&serde_json::json!({"projects": 3})).is_empty());
        assert!(
            discovered_from_json(&serde_json::json!({"additionalModelOptionsCache": "nope"}))
                .is_empty()
        );
    }

    #[test]
    fn index_of_finds_known_values_and_falls_back_to_zero() {
        let options = tier_aliases();
        assert_eq!(index_of(&options, ""), 0);
        assert_eq!(index_of(&options, "sonnet"), 2);
        assert_eq!(index_of(&options, "some-custom-id"), 0);
    }

    #[test]
    fn label_for_shows_custom_values_verbatim() {
        let options = tier_aliases();
        assert_eq!(label_for(&options, "opus"), "opus");
        assert_eq!(label_for(&options, "my-model"), "my-model");
    }

    #[test]
    fn available_always_contains_the_tier_aliases() {
        let options = available();
        assert!(options.iter().any(|o| o.value.is_empty()));
        assert!(options.iter().any(|o| o.value == "opus"));
        // No duplicate values, whatever the local ~/.claude.json contains.
        let mut values: Vec<&str> = options.iter().map(|o| o.value.as_str()).collect();
        let before = values.len();
        values.sort();
        values.dedup();
        assert_eq!(values.len(), before);
    }
}
