//! Aggregate-mode model naming rules.
//!
//! Aggregate mode exposes one Codex model list built from several sites. A site
//! is addressed by a *prefix token*: the user's alias when configured, otherwise
//! the auto-generated provider id (`76a6ef74`, `ccex`, …). Three templates are
//! supported:
//!
//! - `site_model` (default, the historical behaviour): `<prefix><sep><model>`
//! - `model_at_site`: `<model><sep><prefix>`
//! - `model_only`: `<model>` with a `#N` disambiguator when several sites declare
//!   the same upstream model.
//!
//! Design note (why `model_only` auto-numbers instead of rejecting duplicates):
//! sharing a model name across sites is the normal case for the aggregate mode
//! this feature exists for (several relays reselling the same upstream models).
//! Rejecting the combination would make the template unusable for exactly the
//! users who need it, and the failure would only surface at engage time. A
//! deterministic `#N` suffix keeps every slug addressable while remaining
//! trivially reproducible: both the Codex catalog and the request-time router
//! build the same table by walking the same ordered `(site, model)` pairs, so a
//! generated slug like `deepseek-v4-flash#2` always resolves back to the second
//! site that declared `deepseek-v4-flash`.
//!
//! Invariants:
//! - A prefix token must not contain the configured separator, whitespace or
//!   control characters; otherwise `<prefix><sep><model>` cannot be split back.
//!   Everything else is allowed, so CJK site names such as `思源888 pro` are
//!   valid prefixes once normalised.
//! - Aliases are unique case-insensitively because aggregate prefix matching is
//!   ASCII case-insensitive; the same applies to the effective prefix token
//!   (alias, else provider id) of every site.
//! - Slugs are globally unique. Collisions are never resolved by silently
//!   overwriting an earlier entry: `model_only` appends `#N`, and any residual
//!   collision returns an actionable error.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Codex only exposes the five lowest-priority visible entries as
/// `spawn_agent` model hints. Persisting more than that would make the tail
/// silently unreachable to the subagent, so callers must reject it rather than
/// truncating or widening the selection.
pub const SUBAGENT_EXPOSED_MODEL_LIMIT: usize = 5;

/// Maximum alias length accepted by the backend and the settings page.
pub const AGGREGATE_ALIAS_MAX_LEN: usize = 32;

/// How a `(site, model)` pair is turned into the model slug Codex sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateNamingMode {
    /// `<prefix><separator><model>` — the historical aggregate slug shape.
    #[default]
    SiteModel,
    /// `<model><separator><prefix>`.
    ModelAtSite,
    /// `<model>` only; duplicates get a `#N` suffix.
    ModelOnly,
}

impl AggregateNamingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SiteModel => "site_model",
            Self::ModelAtSite => "model_at_site",
            Self::ModelOnly => "model_only",
        }
    }

    /// Whether this template joins the model name with the site prefix.
    ///
    /// `separator` is the connector for the two prefix templates and is unused
    /// by `model_only` (there is nothing to join). Keeping one connector for
    /// both prefix templates avoids a second user-facing field and mirrors the
    /// pre-existing `separator` manifest key.
    pub fn uses_separator(self) -> bool {
        match self {
            Self::SiteModel | Self::ModelAtSite => true,
            Self::ModelOnly => false,
        }
    }
}

/// Effective naming configuration shared by catalog generation and routing.
///
/// Both sides must build slugs from this struct only: the `model_only` template
/// numbers duplicate models by walk order, so any divergence between the two
/// walk orders would silently route a `#N` slug to the wrong site. The table
/// produced at engage time is persisted in the manifest (`slug_table`) and is
/// what request-time routing consumes; this config only rebuilds a table for
/// manifests written before the table was persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateNamingConfig {
    /// Connector between prefix and model name (unused by `model_only`).
    pub separator: String,
    /// provider id -> user alias. Missing/blank entries fall back to the id.
    pub aliases: BTreeMap<String, String>,
    pub naming: AggregateNamingMode,
    /// Bare upstream model names promoted into Codex's visible model list.
    ///
    /// Empty (the default) keeps the historical behavior: every declared bare
    /// model remains addressable, but none is promoted into the picker. A name
    /// is an exact hidden alias unless its slug already matches a visible site
    /// row. A non-empty ordered list promotes those names; unticked names
    /// remain addressable as hidden aliases unless such a slug already exists.
    ///
    /// Order is the user's tick order and is the priority order of the five
    /// `spawn_agent` model hints: the first entry is promoted to the lowest
    /// priority. It is a `Vec`, not a `BTreeSet`, because deserializing into a
    /// set would silently re-sort the user's choice into byte order.
    #[serde(default)]
    pub subagent_exposed_models: Vec<String>,
}

impl Default for AggregateNamingConfig {
    fn default() -> Self {
        Self {
            separator: ".".to_string(),
            aliases: BTreeMap::new(),
            naming: AggregateNamingMode::SiteModel,
            subagent_exposed_models: Vec::new(),
        }
    }
}

impl AggregateNamingConfig {
    /// Prefix token that addresses one site: its alias, else its provider id.
    pub fn prefix_for(&self, site_id: &str) -> String {
        aggregate_site_prefix(site_id, &self.aliases).to_string()
    }

    /// Allocate one slug using this configuration.
    pub fn allocate(
        &self,
        allocator: &mut AggregateSlugAllocator,
        site_id: &str,
        model: &str,
    ) -> Result<Option<String>, String> {
        allocator.allocate(
            site_id,
            model,
            &self.separator,
            self.naming,
            &self.prefix_for(site_id),
        )
    }
}

/// One bare upstream model name and the site that publishes it first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateBareModelSource {
    pub site_id: String,
    pub model: String,
}

/// Every bare upstream model name the ordered sites declare, first site wins.
///
/// This is the drawer's "universe", and it deliberately ignores
/// `AggregateNamingConfig::subagent_exposed_models`: ticking a name promotes it
/// but does not remove other names from the candidate universe.
pub fn build_aggregate_bare_model_universe(
    sites: &[(String, Vec<String>)],
) -> Vec<AggregateBareModelSource> {
    let mut seen = BTreeSet::new();
    let mut universe = Vec::new();
    for (site_id, models) in sites {
        if site_id.trim().is_empty() {
            continue;
        }
        for model in models {
            let model = model.trim();
            if model.is_empty() || !seen.insert(model.to_string()) {
                continue;
            }
            universe.push(AggregateBareModelSource {
                site_id: site_id.clone(),
                model: model.to_string(),
            });
        }
    }
    universe
}

/// Trim bare model names and drop blanks, mirroring the frontend normalizer.
///
/// Order is preserved (first appearance wins) because the caller treats the
/// list as the priority order of the promoted `spawn_agent` hints.
pub fn normalize_bare_model_names<I>(models: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut normalized = Vec::new();
    for model in models {
        let model = model.trim();
        if model.is_empty() || normalized.iter().any(|existing| existing == model) {
            continue;
        }
        normalized.push(model.to_string());
    }
    normalized
}

/// Reject a normalized exposure selection that cannot fit Codex's hint window.
///
/// This is intentionally fail-closed: callers must not truncate the user's
/// tick order because the persisted manifest/draft would then claim a
/// selection that the UI did not actually accept.
pub fn validate_subagent_exposed_model_limit(models: &[String]) -> Result<(), String> {
    if models.len() > SUBAGENT_EXPOSED_MODEL_LIMIT {
        return Err(format!(
            "At most {SUBAGENT_EXPOSED_MODEL_LIMIT} bare models may be exposed to subagents"
        ));
    }
    Ok(())
}

/// Normalize an exposure selection and reject any name the selected sites no
/// longer declare. A partial match is not enough: dropping a stale name would
/// silently change the user's selection.
pub fn validate_bare_model_names_declared(
    requested: &[String],
    declared: &[String],
) -> Result<Vec<String>, String> {
    let normalized = normalize_bare_model_names(requested.iter().cloned());
    if let Some(stale_model) = normalized
        .iter()
        .find(|model| !declared.iter().any(|candidate| candidate == *model))
    {
        return Err(format!(
            "Selected bare model '{stale_model}' is not declared by the selected sites"
        ));
    }
    validate_subagent_exposed_model_limit(&normalized)?;
    Ok(normalized)
}

/// Keep requested bare-model order while dropping names no selected site
/// declares. The catalog can only publish names from this universe.
pub fn filter_bare_model_names_to_declared(
    requested: &[String],
    declared: &[String],
) -> Vec<String> {
    requested
        .iter()
        .filter(|model| declared.iter().any(|candidate| candidate == *model))
        .cloned()
        .collect()
}

/// Turn a provider's user-facing name into a usable prefix token.
///
/// Aggregate mode prefers the user's own site name over the opaque provider id
/// (the reported UX bug: `76a6ef74af6c4151812787cc519b534b.model` instead of
/// `思源888 pro.model`). Names are free-form, so normalise instead of rejecting:
/// trim, drop control characters, fold runs of whitespace to a single `-`, and
/// replace the configured separator with `-` so the result can still be split
/// back out of a slug. Returns `None` when nothing usable is left, in which case
/// callers fall back to the provider id.
pub fn derive_site_prefix_from_name(name: &str, separator: &str) -> Option<String> {
    let replaced = if separator.is_empty() {
        name.to_string()
    } else {
        name.replace(separator, "-")
    };
    let mut out = String::new();
    let mut pending_dash = false;
    for ch in replaced.trim().chars() {
        // Whitespace is checked before the control check: tab/newline are both,
        // and folding them into '-' reads far better than deleting them.
        if ch.is_whitespace() {
            pending_dash = !out.is_empty();
            continue;
        }
        if ch.is_control() {
            continue;
        }
        if pending_dash {
            out.push('-');
            pending_dash = false;
        }
        out.push(ch);
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Validate one configured alias.
///
/// An empty string means "no alias configured" and must be filtered out by the
/// caller before validating; reaching this function with an empty value is a
/// programming error the caller surfaces as a validation failure.
///
/// Charset is deliberately permissive (CJK site names are a first-class case):
/// only the *structural* characters are refused — whitespace, control
/// characters and the configured separator. A prefix may not contain the
/// separator, otherwise `<prefix><sep><model>` cannot be split back.
pub fn validate_aggregate_alias(alias: &str, separator: &str) -> Result<(), String> {
    if alias.is_empty() {
        return Err("Aggregate site alias must not be empty".to_string());
    }
    if alias.chars().count() > AGGREGATE_ALIAS_MAX_LEN {
        return Err(format!(
            "Aggregate site alias must be at most {AGGREGATE_ALIAS_MAX_LEN} characters"
        ));
    }
    if let Some(ch) = alias.chars().find(|ch| ch.is_whitespace()) {
        return Err(format!(
            "Aggregate site alias must not contain whitespace (found {ch:?})"
        ));
    }
    if let Some(ch) = alias.chars().find(|ch| ch.is_control()) {
        return Err(format!(
            "Aggregate site alias must not contain control characters (found {ch:?})"
        ));
    }
    if !separator.is_empty() && alias.contains(separator) {
        return Err(format!(
            "Aggregate site alias must not contain the separator '{separator}'"
        ));
    }
    Ok(())
}

/// Validate every configured alias: charset/length plus case-insensitive
/// uniqueness, because prefix matching is ASCII case-insensitive.
pub fn validate_aggregate_aliases(
    aliases: &BTreeMap<String, String>,
    separator: &str,
) -> Result<(), String> {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    for (provider_id, alias) in aliases {
        let alias = alias.trim();
        if alias.is_empty() {
            // Blank entries are dropped before validation; treat a stored blank
            // as invalid so a hand-edited manifest cannot silently change names.
            return Err(format!(
                "Aggregate site alias for '{provider_id}' must not be empty"
            ));
        }
        validate_aggregate_alias(alias, separator)
            .map_err(|error| format!("Aggregate site alias '{alias}': {error}"))?;
        let key = alias.to_ascii_lowercase();
        if let Some(previous) = seen.get(&key) {
            return Err(format!(
                "Aggregate site alias '{alias}' is used by more than one site ('{previous}' and '{provider_id}'); aliases must be globally unique"
            ));
        }
        seen.insert(key, provider_id);
    }
    Ok(())
}

/// The prefix token that addresses one site: its alias, else its provider id.
pub fn aggregate_site_prefix<'a>(
    site_id: &'a str,
    aliases: &'a BTreeMap<String, String>,
) -> &'a str {
    aliases
        .get(site_id)
        .map(|alias| alias.trim())
        .filter(|alias| !alias.is_empty())
        .unwrap_or(site_id)
}

/// Compute the alias map aggregate mode should actually publish.
///
/// Explicit aliases win unchanged. A selected site without one falls back to
/// its normalised display name — the reported UX bug was a catalog full of
/// opaque provider ids (`76a6ef74af6c4151812787cc519b534b.model`) when the user
/// had already named the site (`思源888 pro.model`).
///
/// A derived name is only used when it cannot steal another site's address:
/// derived prefixes must stay unique (case-insensitively) against every
/// enabled provider id and against every explicit alias. Anything unusable —
/// empty after normalisation, too long, or colliding — silently falls back to
/// the provider id, so engaging never fails just because two sites share a
/// display name. Only *explicit* aliases are allowed to fail validation.
///
/// `selected` is `(provider_id, display_name)` in the user's priority order;
/// `all_ids` is every enabled candidate id (unselected sites stay addressable
/// by their ids at request time).
pub fn resolve_effective_site_aliases(
    selected: &[(String, String)],
    explicit: &BTreeMap<String, String>,
    all_ids: &[String],
    separator: &str,
) -> BTreeMap<String, String> {
    let mut taken: BTreeMap<String, String> = all_ids
        .iter()
        .map(|id| (id.to_ascii_lowercase(), id.clone()))
        .collect();
    let mut effective = explicit.clone();
    for (id, alias) in &effective {
        taken.insert(alias.to_ascii_lowercase(), id.clone());
    }

    for (id, name) in selected {
        if effective.contains_key(id) {
            continue;
        }
        let Some(prefix) = derive_site_prefix_from_name(name, separator) else {
            continue;
        };
        if prefix == *id || prefix.chars().count() > AGGREGATE_ALIAS_MAX_LEN {
            continue;
        }
        if validate_aggregate_alias(&prefix, separator).is_err() {
            continue;
        }
        let key = prefix.to_ascii_lowercase();
        if taken.contains_key(&key) {
            continue;
        }
        taken.insert(key, id.clone());
        effective.insert(id.clone(), prefix);
    }
    effective
}

/// Validate that every site in `site_ids` has a distinct effective prefix.
///
/// `site_ids` must contain every addressable site (selected and unselected),
/// because unselected sites stay available as aggregate fallbacks and their ids
/// are still resolvable when a request carries a prefix.
pub fn validate_aggregate_site_prefixes(
    site_ids: &[String],
    aliases: &BTreeMap<String, String>,
) -> Result<(), String> {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    for site_id in site_ids {
        let prefix = aggregate_site_prefix(site_id, aliases);
        if prefix.is_empty() {
            continue;
        }
        let key = prefix.to_ascii_lowercase();
        if let Some(previous) = seen.get(&key) {
            return Err(format!(
                "Aggregate site name '{prefix}' addresses both '{previous}' and '{site_id}'; site aliases must be unique across every site"
            ));
        }
        seen.insert(key, site_id);
    }
    Ok(())
}

/// One `(site, model)` pair with the slug it must be published under.
///
/// Persisted in the manifest so request-time routing replays the exact table the
/// Codex catalog was built from, instead of re-deriving slugs from whatever
/// sites happen to be enabled later (see `AggregateNamingConfig`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateSlugEntry {
    pub site_id: String,
    pub upstream_model: String,
    pub slug: String,
}

/// Allocates slugs for `(site, model)` pairs in iteration order.
///
/// Both the Codex catalog generator and the request-time router walk the same
/// ordered pairs (selected sites in display order, then each site's declared
/// models in declaration order), which is what makes `#N` numbering stable.
#[derive(Debug, Default)]
pub struct AggregateSlugAllocator {
    pairs: BTreeSet<(String, String)>,
    slugs: BTreeSet<String>,
}

impl AggregateSlugAllocator {
    /// Allocate the slug for one pair.
    ///
    /// Returns `Ok(None)` when this exact `(site, model)` pair was already
    /// emitted, so duplicated catalog rows collapse instead of producing a
    /// second entry. Two *different* pairs never share a slug: `model_only`
    /// appends `#N`, and every other mode reports the conflict as an error
    /// instead of overwriting the earlier entry.
    pub fn allocate(
        &mut self,
        site_id: &str,
        model: &str,
        separator: &str,
        naming: AggregateNamingMode,
        prefix: &str,
    ) -> Result<Option<String>, String> {
        let pair = (site_id.to_string(), model.to_string());
        if self.pairs.contains(&pair) {
            return Ok(None);
        }
        let candidate = match naming {
            AggregateNamingMode::SiteModel => format!("{prefix}{separator}{model}"),
            AggregateNamingMode::ModelAtSite => format!("{model}{separator}{prefix}"),
            AggregateNamingMode::ModelOnly => model.to_string(),
        };
        let slug = match naming {
            AggregateNamingMode::ModelOnly => {
                let mut slug = candidate.clone();
                let mut index = 2_u32;
                while self.slugs.contains(&slug) {
                    slug = format!("{candidate}#{index}");
                    index += 1;
                }
                slug
            }
            _ => {
                if self.slugs.contains(&candidate) {
                    return Err(format!(
                        "Aggregate model name '{candidate}' would be produced by more than one site; adjust the site aliases or the naming mode"
                    ));
                }
                candidate
            }
        };
        self.pairs.insert(pair);
        self.slugs.insert(slug.clone());
        Ok(Some(slug))
    }
}

/// Build the full slug table for an ordered list of `(site_id, models)`.
pub fn build_aggregate_slug_table(
    sites: &[(String, Vec<String>)],
    separator: &str,
    aliases: &BTreeMap<String, String>,
    naming: AggregateNamingMode,
) -> Result<Vec<AggregateSlugEntry>, String> {
    let mut allocator = AggregateSlugAllocator::default();
    let mut entries = Vec::new();
    for (site_id, models) in sites {
        if site_id.trim().is_empty() {
            continue;
        }
        let prefix = aggregate_site_prefix(site_id, aliases);
        for model in models {
            let model = model.trim();
            if model.is_empty() {
                continue;
            }
            let Some(slug) = allocator.allocate(site_id, model, separator, naming, prefix)? else {
                continue;
            };
            entries.push(AggregateSlugEntry {
                site_id: site_id.clone(),
                upstream_model: model.to_string(),
                slug,
            });
        }
    }
    Ok(entries)
}

/// Split `<prefix><separator><model>` (site-model template) into its parts.
///
/// Only the configured separator after a known prefix token is stripped, so
/// dots/separators inside the upstream model name survive untouched.
pub fn split_site_model_slug<'a>(
    requested_model: &'a str,
    separator: &str,
    sites: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<(String, String)> {
    if separator.is_empty() {
        return None;
    }
    for (site_id, prefix) in sites {
        if prefix.is_empty() {
            continue;
        }
        let prefix_len = prefix.len() + separator.len();
        if requested_model.len() <= prefix_len {
            continue;
        }
        // The head has to be an ASCII prefix, so an index that lands inside a
        // multi-byte character of the model name can only be a non-match. Guard
        // it explicitly: `split_at` panics on a non-char-boundary index, and the
        // release profile builds with `panic = "abort"`.
        if !requested_model.is_char_boundary(prefix.len()) {
            continue;
        }
        let (head, rest) = requested_model.split_at(prefix.len());
        if !head.eq_ignore_ascii_case(prefix) {
            continue;
        }
        let Some(model) = rest.strip_prefix(separator) else {
            continue;
        };
        let model = model.trim();
        if model.is_empty() {
            continue;
        }
        return Some((site_id.to_string(), model.to_string()));
    }
    None
}

/// Split `<model><separator><prefix>` (model-at-site template) into its parts.
///
/// The prefix token can never contain the separator, so the suffix match at the
/// end of the string is unambiguous.
pub fn split_model_at_site_slug<'a>(
    requested_model: &'a str,
    separator: &str,
    sites: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<(String, String)> {
    if separator.is_empty() {
        return None;
    }
    for (site_id, prefix) in sites {
        if prefix.is_empty() {
            continue;
        }
        let suffix_len = prefix.len() + separator.len();
        if requested_model.len() <= suffix_len {
            continue;
        }
        let split_at = requested_model.len() - suffix_len;
        // The *tail* is ASCII by construction (separator + prefix), but the cut
        // index can still land inside a multi-byte character of the model name;
        // `split_at` would panic there, so treat it as a non-match.
        if !requested_model.is_char_boundary(split_at) {
            continue;
        }
        let (model, tail) = requested_model.split_at(split_at);
        if !tail.eq_ignore_ascii_case(&format!("{separator}{prefix}")) {
            continue;
        }
        let model = model.trim();
        if model.is_empty() {
            continue;
        }
        return Some((site_id.to_string(), model.to_string()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aliases(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(id, alias)| (id.to_string(), alias.to_string()))
            .collect()
    }

    fn sites(pairs: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        pairs
            .iter()
            .map(|(id, models)| {
                (
                    id.to_string(),
                    models.iter().map(|model| model.to_string()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn alias_validation_accepts_backend_charset_only() {
        assert!(validate_aggregate_alias("unsee", ".").is_ok());
        assert!(validate_aggregate_alias("chain888", ".").is_ok());
        assert!(validate_aggregate_alias("my-site_1", ".").is_ok());
        // CJK site names are a first-class case: the reported UX bug was the
        // catalog showing an opaque provider id instead of the user's name.
        // (An explicit alias must still be a single token; a *display name*
        // with spaces goes through `derive_site_prefix_from_name` instead.)
        assert!(validate_aggregate_alias("中文", ".").is_ok());
        assert!(validate_aggregate_alias("思源888", ".").is_ok());
        assert!(validate_aggregate_alias("思源888 pro", ".").is_err());
        assert!(validate_aggregate_alias(&"a".repeat(AGGREGATE_ALIAS_MAX_LEN), ".").is_ok());

        assert!(validate_aggregate_alias("", ".").is_err());
        assert!(validate_aggregate_alias(&"a".repeat(AGGREGATE_ALIAS_MAX_LEN + 1), ".").is_err());
        // Structural characters only: whitespace, control chars, separator.
        assert!(validate_aggregate_alias("with space", ".").is_err());
        assert!(validate_aggregate_alias("with\nnewline", ".").is_err());
        assert!(validate_aggregate_alias("with.dot", ".").is_err());
        // A character that merely *looks* like the separator is fine.
        assert!(validate_aggregate_alias("with:colon", ".").is_ok());
        assert!(validate_aggregate_alias("with/slash", ".").is_ok());
        // ...but it is refused when the separator actually is `/`.
        assert!(validate_aggregate_alias("with/slash", "/").is_err());
    }

    #[test]
    fn alias_validation_rejects_case_insensitive_duplicates() {
        assert!(
            validate_aggregate_aliases(&aliases(&[("a", "unsee"), ("b", "chain888")]), ".").is_ok()
        );
        assert!(
            validate_aggregate_aliases(&aliases(&[("a", "unsee"), ("b", "UNSEE")]), ".").is_err()
        );
        assert!(
            validate_aggregate_aliases(&aliases(&[("a", "unsee"), ("b", "unsee")]), ".").is_err()
        );
        assert!(validate_aggregate_aliases(&aliases(&[("a", "")]), ".").is_err());
    }

    #[test]
    fn derived_site_prefix_uses_the_user_name_and_normalises_it() {
        assert_eq!(
            derive_site_prefix_from_name("Unsee Relay", ".").as_deref(),
            Some("Unsee-Relay")
        );
        // The reported case: CJK + digits + a space survive as a usable token.
        assert_eq!(
            derive_site_prefix_from_name("思源888 pro", ".").as_deref(),
            Some("思源888-pro")
        );
        // A name that contains the separator must not produce an unsplittable
        // slug, so the separator becomes a dash.
        assert_eq!(
            derive_site_prefix_from_name("ai.nexfaro.com", ".").as_deref(),
            Some("ai-nexfaro-com")
        );
        // Control characters are dropped; whitespace runs collapse.
        assert_eq!(
            derive_site_prefix_from_name("  a\t\tb  ", ".").as_deref(),
            Some("a-b")
        );
        // Nothing usable → caller keeps the provider id.
        assert_eq!(derive_site_prefix_from_name("   ", "."), None);
        assert_eq!(derive_site_prefix_from_name("\u{7}", "."), None);
    }

    #[test]
    fn effective_site_aliases_prefer_names_but_never_steal_an_address() {
        let selected = vec![
            ("76a6ef74".to_string(), "思源888 pro".to_string()),
            ("ccex".to_string(), "Unsee Relay".to_string()),
        ];
        let all_ids = vec!["76a6ef74".to_string(), "ccex".to_string()];

        let effective = resolve_effective_site_aliases(&selected, &BTreeMap::new(), &all_ids, ".");
        assert_eq!(
            effective.get("76a6ef74").map(String::as_str),
            Some("思源888-pro")
        );
        assert_eq!(
            effective.get("ccex").map(String::as_str),
            Some("Unsee-Relay")
        );

        // An explicit alias always wins over the derived display name.
        let explicit = aliases(&[("ccex", "relay")]);
        let effective = resolve_effective_site_aliases(&selected, &explicit, &all_ids, ".");
        assert_eq!(effective.get("ccex").map(String::as_str), Some("relay"));

        // The derived name collides with the other site's id → keep the id, do
        // not fail the engage (names are user-facing, ids are addressable).
        let colliding = vec![
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "Bee".to_string()),
        ];
        let effective = resolve_effective_site_aliases(
            &colliding,
            &BTreeMap::new(),
            &["a".to_string(), "b".to_string()],
            ".",
        );
        assert_eq!(effective.get("a"), None);
        assert_eq!(effective.get("b").map(String::as_str), Some("Bee"));

        // Two sites with the same display name: the first keeps it, the second
        // falls back to its provider id instead of silently shadowing it.
        let duplicate_names = vec![
            ("a".to_string(), "Relay".to_string()),
            ("b".to_string(), "Relay".to_string()),
        ];
        let effective = resolve_effective_site_aliases(
            &duplicate_names,
            &BTreeMap::new(),
            &["a".to_string(), "b".to_string()],
            ".",
        );
        assert_eq!(effective.get("a").map(String::as_str), Some("Relay"));
        assert_eq!(effective.get("b"), None);
    }

    #[test]
    fn site_prefixes_prefer_alias_and_must_stay_unique() {
        let with_alias = aliases(&[("76a6ef74", "unsee")]);
        assert_eq!(aggregate_site_prefix("76a6ef74", &with_alias), "unsee");
        assert_eq!(aggregate_site_prefix("ccex", &with_alias), "ccex");

        // An alias may not shadow another site's id: both sites would answer to
        // the same prefix and the request could not be routed deterministically.
        let collapsed = aliases(&[("76a6ef74", "ccex")]);
        assert!(validate_aggregate_site_prefixes(
            &["76a6ef74".to_string(), "ccex".to_string()],
            &collapsed
        )
        .is_err());
        assert!(validate_aggregate_site_prefixes(
            &["76a6ef74".to_string(), "ccex".to_string()],
            &with_alias
        )
        .is_ok());
    }

    #[test]
    fn alias_cannot_shadow_an_unselected_enabled_provider_id() {
        let selected = vec!["site-a".to_string()];
        let all_enabled = vec!["site-a".to_string(), "site-b".to_string()];
        let aliases = aliases(&[("site-a", "site-b")]);

        assert!(validate_aggregate_site_prefixes(&selected, &aliases).is_ok());
        assert!(validate_aggregate_site_prefixes(&all_enabled, &aliases).is_err());
    }

    #[test]
    fn slug_table_names_every_mode() {
        let table = build_aggregate_slug_table(
            &sites(&[("76a6ef74", &["deepseek-v4-flash"])]),
            ".",
            &aliases(&[("76a6ef74", "unsee")]),
            AggregateNamingMode::SiteModel,
        )
        .unwrap();
        assert_eq!(table[0].slug, "unsee.deepseek-v4-flash");
        assert_eq!(table[0].upstream_model, "deepseek-v4-flash");

        let table = build_aggregate_slug_table(
            &sites(&[("76a6ef74", &["deepseek-v4-flash"])]),
            "@",
            &aliases(&[("76a6ef74", "unsee")]),
            AggregateNamingMode::ModelAtSite,
        )
        .unwrap();
        assert_eq!(table[0].slug, "deepseek-v4-flash@unsee");

        let table = build_aggregate_slug_table(
            &sites(&[("76a6ef74", &["deepseek-v4-flash"])]),
            ".",
            &BTreeMap::new(),
            AggregateNamingMode::ModelOnly,
        )
        .unwrap();
        assert_eq!(table[0].slug, "deepseek-v4-flash");
    }

    #[test]
    fn model_only_numbers_duplicate_models_in_site_order() {
        let table = build_aggregate_slug_table(
            &sites(&[
                ("site-a", &["deepseek-v4-flash", "glm-5"]),
                ("site-b", &["deepseek-v4-flash"]),
                ("site-c", &["deepseek-v4-flash"]),
            ]),
            ".",
            &BTreeMap::new(),
            AggregateNamingMode::ModelOnly,
        )
        .unwrap();

        let slugs: Vec<&str> = table.iter().map(|entry| entry.slug.as_str()).collect();
        assert_eq!(
            slugs,
            vec![
                "deepseek-v4-flash",
                "glm-5",
                "deepseek-v4-flash#2",
                "deepseek-v4-flash#3"
            ]
        );
        // Numbering keeps pointing at the original upstream model name.
        assert_eq!(table[2].site_id, "site-b");
        assert_eq!(table[2].upstream_model, "deepseek-v4-flash");
    }

    #[test]
    fn slug_table_dedupes_repeated_pairs_and_reports_prefix_collisions() {
        let table = build_aggregate_slug_table(
            &sites(&[("site-a", &["m", "m"])]),
            ".",
            &BTreeMap::new(),
            AggregateNamingMode::SiteModel,
        )
        .unwrap();
        assert_eq!(table.len(), 1);

        // Two prefixes that differ only by the separator can never collide in
        // the prefix modes, so this can only be produced by handing the
        // allocator the same prefix twice — the guard must error, never
        // overwrite the first entry.
        let mut allocator = AggregateSlugAllocator::default();
        allocator
            .allocate("site-a", "m", ".", AggregateNamingMode::SiteModel, "same")
            .unwrap();
        assert!(allocator
            .allocate("site-b", "m", ".", AggregateNamingMode::SiteModel, "same")
            .is_err());
    }

    #[test]
    fn bare_model_universe_keeps_the_first_site_and_ignores_the_exposure_set() {
        let declared = sites(&[
            ("site-a", &["gpt-5.6-luna", "glm-5"]),
            ("site-b", &["gpt-5.6-luna", "deepseek-v4-flash"]),
            ("  ", &["ignored"]),
        ]);

        let universe = build_aggregate_bare_model_universe(&declared);

        // Names follow first-appearance order and the first declaring site wins.
        // The universe is the drawer's candidate list, so it must not depend on
        // which names are promoted.
        assert_eq!(
            universe,
            vec![
                AggregateBareModelSource {
                    site_id: "site-a".to_string(),
                    model: "gpt-5.6-luna".to_string(),
                },
                AggregateBareModelSource {
                    site_id: "site-a".to_string(),
                    model: "glm-5".to_string(),
                },
                AggregateBareModelSource {
                    site_id: "site-b".to_string(),
                    model: "deepseek-v4-flash".to_string(),
                },
            ]
        );
        assert!(build_aggregate_bare_model_universe(&[]).is_empty());
    }

    #[test]
    fn normalize_bare_model_names_trims_and_drops_blanks() {
        assert_eq!(
            normalize_bare_model_names(vec![
                " gpt-5.6-luna ".to_string(),
                "".to_string(),
                "glm-5".to_string(),
                "gpt-5.6-luna".to_string(),
            ]),
            vec!["gpt-5.6-luna".to_string(), "glm-5".to_string()]
        );
        assert!(normalize_bare_model_names(Vec::<String>::new()).is_empty());
    }

    #[test]
    fn subagent_exposure_limit_rejects_without_truncating() {
        let models = (1..=6)
            .map(|index| format!("model-{index}"))
            .collect::<Vec<_>>();

        let error = validate_subagent_exposed_model_limit(&models).unwrap_err();

        assert!(error.contains("At most 5"), "{error}");
        assert_eq!(models.len(), 6);
        assert!(validate_subagent_exposed_model_limit(&models[..5]).is_ok());
    }

    #[test]
    fn filter_bare_model_names_keeps_only_models_declared_by_selected_sites() {
        let requested = vec![
            "gpt-5.6-terra".to_string(),
            "not-selected".to_string(),
            "gpt-5.6-luna".to_string(),
        ];
        let declared = vec!["gpt-5.6-luna".to_string(), "gpt-5.6-terra".to_string()];

        assert_eq!(
            filter_bare_model_names_to_declared(&requested, &declared),
            vec!["gpt-5.6-terra".to_string(), "gpt-5.6-luna".to_string()]
        );
    }

    #[test]
    fn validation_rejects_mixed_declared_and_stale_bare_model_selection() {
        let requested = vec![
            " model-1 ".to_string(),
            "stale-model".to_string(),
            "model-2".to_string(),
        ];
        let declared = vec!["model-1".to_string(), "model-2".to_string()];

        let error = validate_bare_model_names_declared(&requested, &declared).unwrap_err();

        assert!(error.contains("stale-model"), "{error}");
        assert!(
            error.contains("not declared by the selected sites"),
            "{error}"
        );
    }

    #[test]
    fn split_helpers_match_only_their_own_template() {
        let entries = [("site-a", "unsee"), ("site-b", "chain888")];

        assert_eq!(
            split_site_model_slug("unsee.deepseek-v4.1-flash", ".", entries),
            Some(("site-a".to_string(), "deepseek-v4.1-flash".to_string()))
        );
        assert_eq!(
            split_site_model_slug("deepseek-v4-flash", ".", entries),
            None
        );
        assert_eq!(split_site_model_slug("unsee", ".", entries), None);
        assert_eq!(split_site_model_slug("unsee.", ".", entries), None);

        assert_eq!(
            split_model_at_site_slug("deepseek-v4-flash@chain888", "@", entries),
            Some(("site-b".to_string(), "deepseek-v4-flash".to_string()))
        );
        // A model name that itself contains the separator stays intact.
        assert_eq!(
            split_model_at_site_slug("deepseek-v4@1-flash@unsee", "@", entries),
            Some(("site-a".to_string(), "deepseek-v4@1-flash".to_string()))
        );
        assert_eq!(
            split_model_at_site_slug("deepseek-v4-flash", "@", entries),
            None
        );
    }

    #[test]
    fn split_helpers_never_panic_on_a_cut_inside_a_multibyte_model_name() {
        // Byte 5 of "1234中.x" is inside '中'; a bare `split_at(5)` would panic
        // (and release builds abort the process). Same for the suffix helper,
        // where byte 5 of "中文site1" is inside '文'.
        assert_eq!(
            split_site_model_slug("1234中.x", ".", [("site-a", "site1")]),
            None
        );
        assert_eq!(
            split_model_at_site_slug("中文site1", ".", [("site-a", "site1")]),
            None
        );
        // A well-formed multi-byte model with the full prefix still splits.
        assert_eq!(
            split_site_model_slug("site1.中模型", ".", [("site-a", "site1")]),
            Some(("site-a".to_string(), "中模型".to_string()))
        );
        assert_eq!(
            split_model_at_site_slug("中模型.site1", ".", [("site-a", "site1")]),
            Some(("site-a".to_string(), "中模型".to_string()))
        );
    }

    #[test]
    fn split_helpers_are_case_insensitive_but_keep_model_case() {
        assert_eq!(
            split_site_model_slug("UNSEE.DeepSeek-V4", ".", [("site-a", "unsee")]),
            Some(("site-a".to_string(), "DeepSeek-V4".to_string()))
        );
        assert_eq!(
            split_model_at_site_slug("DeepSeek-V4@UNSEE", "@", [("site-a", "unsee")]),
            Some(("site-a".to_string(), "DeepSeek-V4".to_string()))
        );
    }

    #[test]
    fn site_prefix_never_steals_a_longer_model_name() {
        // A site aliased `luna` must not answer for a *different* site's model
        // that merely starts with the same letters; only the exact
        // `<prefix><separator>` head may be split off.
        let sites = [("site-a", "luna"), ("site-b", "xxx")];

        assert_eq!(
            split_site_model_slug("luna/luna-plus", "/", sites),
            Some(("site-a".to_string(), "luna-plus".to_string()))
        );
        // `xxx/luna-plus` belongs to site-b and stays untouched by site-a.
        assert_eq!(
            split_site_model_slug("xxx/luna-plus", "/", sites),
            Some(("site-b".to_string(), "luna-plus".to_string()))
        );
        // `luna-plus` alone has no separator after the prefix, so no site owns
        // it — it must fall through to the bare-model-name path instead.
        assert_eq!(split_site_model_slug("luna-plus", "/", sites), None);
        assert_eq!(split_site_model_slug("lunaXplus", "/", sites), None);
    }

    #[test]
    fn unicode_prefixes_round_trip_without_panicking() {
        let sites = [("76a6ef74", "思源888-pro"), ("ccex", "备用")];

        assert_eq!(
            split_site_model_slug("思源888-pro.gpt-5.6-luna", ".", sites),
            Some(("76a6ef74".to_string(), "gpt-5.6-luna".to_string()))
        );
        assert_eq!(
            split_model_at_site_slug("gpt-5.6-luna.备用", ".", sites),
            Some(("ccex".to_string(), "gpt-5.6-luna".to_string()))
        );
        // Cut inside a multi-byte character of the *prefix* must be a
        // non-match, never a panic (release builds abort on panic).
        assert_eq!(split_site_model_slug("思源.gpt-5.6-luna", ".", sites), None);
        assert_eq!(split_model_at_site_slug("gpt.备", ".", sites), None);
    }
}
