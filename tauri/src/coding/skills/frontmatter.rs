//! Minimal YAML frontmatter parser for SKILL.md files.
//!
//! Only handles the subset used by skill frontmatter: top-level `key: value`
//! pairs between `---` markers, with plain/quoted scalars and block scalars
//! (`|`, `>`, and their `-`/`+` chomping variants). Block content is collected
//! from the indented continuation lines so a `description: |` value does not
//! leak the literal `|` indicator into the cached description.

use std::path::Path;

/// Parse the `name` and `description` fields from a SKILL.md YAML frontmatter.
/// Returns `(name, description)`.
pub fn parse_skill_md_frontmatter(path: &Path) -> (Option<String>, Option<String>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (None, None);
    };
    parse_frontmatter_text(&text)
}

/// Parse name/description from frontmatter text (the raw SKILL.md content).
pub fn parse_frontmatter_text(text: &str) -> (Option<String>, Option<String>) {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }

    let mut name = None;
    let mut description = None;
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        // Only match top-level keys so nested fields (e.g. `metadata.name`)
        // cannot overwrite the Skill's own name/description, matching
        // `rewrite_frontmatter_name_text`.
        if line.starts_with(|c: char| c.is_whitespace()) {
            continue;
        }
        if let Some(value) = line.strip_prefix("name:") {
            let v = normalize_scalar(value);
            if !v.is_empty() {
                name = Some(v);
            }
        } else if let Some(value) = line.strip_prefix("description:") {
            let v = normalize_scalar(value);
            if is_block_scalar_indicator(&v) {
                if let Some(block) = collect_block_scalar(&mut lines) {
                    if !block.is_empty() {
                        description = Some(block);
                    }
                }
            } else if !v.is_empty() {
                description = Some(v);
            }
        }
    }
    (name, description)
}

fn normalize_scalar(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        return unescape_double_quoted(inner).trim().to_string();
    }
    if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        return trimmed[1..trimmed.len() - 1]
            .replace("''", "'")
            .trim()
            .to_string();
    }
    trimmed.to_string()
}

fn unescape_double_quoted(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn is_plain_yaml_number(value: &str) -> bool {
    if value.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    value
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit())
        && value.parse::<f64>().is_ok()
}

fn is_yaml_special_literal(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~"
    )
}

fn format_yaml_scalar(value: &str) -> String {
    let is_safe_plain = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !value.starts_with('-')
        && !value.starts_with('.')
        && !is_yaml_special_literal(value)
        && !is_plain_yaml_number(value);

    if is_safe_plain {
        value.to_string()
    } else {
        let escaped = value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        format!("\"{}\"", escaped)
    }
}

fn split_line_ending(raw_line: &str) -> (&str, &str) {
    if let Some(content) = raw_line.strip_suffix("\r\n") {
        (content, "\r\n")
    } else if let Some(content) = raw_line.strip_suffix('\n') {
        (content, "\n")
    } else {
        (raw_line, "")
    }
}

/// Losslessly rewrite the top-level `name:` field inside a SKILL.md YAML
/// frontmatter block. Returns `None` when frontmatter or `name:` is missing, or
/// when the existing parsed `name` already equals `new_name`.
pub fn rewrite_frontmatter_name_text(text: &str, new_name: &str) -> Option<String> {
    if new_name.trim().is_empty() {
        return None;
    }

    let (bom, body) = if let Some(rest) = text.strip_prefix('\u{FEFF}') {
        ("\u{FEFF}", rest)
    } else {
        ("", text)
    };

    let mut lines: Vec<&str> = body.split_inclusive('\n').collect();
    if lines.is_empty() {
        return None;
    }

    let (first_content, _) = split_line_ending(lines[0]);
    if first_content.trim() != "---" {
        return None;
    }

    let mut closing_idx: Option<usize> = None;
    let mut name_idx: Option<usize> = None;
    let mut existing_name: Option<String> = None;

    for (idx, raw_line) in lines.iter().enumerate().skip(1) {
        let (content, _) = split_line_ending(raw_line);
        if !content.starts_with(|c: char| c.is_whitespace()) {
            if content.trim_end() == "---" {
                closing_idx = Some(idx);
                break;
            }
            if let Some(raw_value) = content.strip_prefix("name:") {
                name_idx = Some(idx);
                let parsed = normalize_scalar(raw_value);
                existing_name = if parsed.is_empty() {
                    None
                } else {
                    Some(parsed)
                };
            }
        }
    }

    closing_idx?;
    let name_idx = name_idx?;
    if existing_name.as_deref() == Some(new_name) {
        return None;
    }

    let (_, line_ending) = split_line_ending(lines[name_idx]);
    let replacement = format!("name: {}{}", format_yaml_scalar(new_name), line_ending);

    let mut out = String::with_capacity(text.len() + replacement.len());
    out.push_str(bom);
    for (idx, raw_line) in lines.drain(..).enumerate() {
        if idx == name_idx {
            out.push_str(&replacement);
        } else {
            out.push_str(raw_line);
        }
    }
    Some(out)
}

/// Rewrite the top-level frontmatter `name:` field of a SKILL.md file on disk.
/// Returns `Ok(false)` without modifying the file if the path does not exist,
/// has no valid frontmatter / `name:`, or already has `new_name`.
pub fn rewrite_skill_md_name(path: &Path, new_name: &str) -> std::io::Result<bool> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    let Some(updated) = rewrite_frontmatter_name_text(&text, new_name) else {
        return Ok(false);
    };
    std::fs::write(path, updated)?;
    Ok(true)
}

/// A YAML block scalar indicator: `|`, `>`, optionally followed by `-`/`+` or a
/// digit (e.g. `|-`, `>+`, `|2-`). Such a value means the real content lives
/// on the following indented lines.
fn is_block_scalar_indicator(value: &str) -> bool {
    let v = value.trim();
    v.starts_with('|') || v.starts_with('>')
}

/// Collect the indented continuation lines of a block scalar. Stops at the
/// first non-blank, non-indented line or the closing `---`. The content is
/// returned joined with newlines (common indentation stripped per line); the
/// caller decides whether to flatten it for one-line display.
fn collect_block_scalar<'a>(lines: &mut std::str::Lines<'a>) -> Option<String> {
    let mut content_lines: Vec<String> = Vec::new();
    for line in lines.by_ref() {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        // A block content line is indented (leading whitespace) or blank.
        // A non-blank, non-indented line ends the block.
        if !line.starts_with(|c: char| c.is_whitespace()) && !trimmed.is_empty() {
            break;
        }
        content_lines.push(trimmed.to_string());
    }
    // Strip trailing blank lines (noise for display).
    while content_lines.last().map(|s| s.is_empty()).unwrap_or(false) {
        content_lines.pop();
    }
    if content_lines.iter().all(|s| s.is_empty()) {
        return None;
    }
    Some(content_lines.join("\n").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_scalar() {
        let text = "---\nname: review\ndescription: Review frontend code\n---\n";
        let (name, desc) = parse_frontmatter_text(text);
        assert_eq!(name.as_deref(), Some("review"));
        assert_eq!(desc.as_deref(), Some("Review frontend code"));
    }

    #[test]
    fn parses_quoted_scalar() {
        let text = "---\nname: \"foo\"\ndescription: \"A quoted desc\"\n---\n";
        let (name, desc) = parse_frontmatter_text(text);
        assert_eq!(name.as_deref(), Some("foo"));
        assert_eq!(desc.as_deref(), Some("A quoted desc"));
    }

    #[test]
    fn parses_literal_block_scalar_pipe() {
        // Mimics the MiniMax frontend-dev SKILL.md: `description: |` followed
        // by indented continuation lines. The `|` must NOT leak into the value.
        let text = concat!(
            "---\n",
            "name: frontend-dev\n",
            "description: |\n",
            "  Full-stack frontend development combining premium UI design, cinematic animations,\n",
            "  AI-generated media assets, persuasive copywriting, and visual art.\n",
            "license: MIT\n",
            "---\n",
        );
        let (name, desc) = parse_frontmatter_text(text);
        assert_eq!(name.as_deref(), Some("frontend-dev"));
        let d = desc.expect("description");
        assert!(!d.contains('|'), "block indicator must not leak: {d}");
        assert!(d.contains("Full-stack frontend development"));
        assert!(d.contains("cinematic animations"));
        assert!(d.contains("visual art."));
    }

    #[test]
    fn parses_folded_block_scalar_gt() {
        let text = concat!(
            "---\n",
            "name: foo\n",
            "description: >\n",
            "  First line\n",
            "  second line\n",
            "---\n",
        );
        let (name, desc) = parse_frontmatter_text(text);
        assert_eq!(name.as_deref(), Some("foo"));
        let d = desc.expect("description");
        assert!(!d.contains('>'));
        assert!(d.contains("First line"));
        assert!(d.contains("second line"));
    }

    #[test]
    fn parses_block_scalar_with_chomping_indicator() {
        // `|-` strips the trailing newline.
        let text = concat!(
            "---\n",
            "name: foo\n",
            "description: |-\n",
            "  line one\n",
            "  line two\n",
            "---\n",
        );
        let (_, desc) = parse_frontmatter_text(text);
        let d = desc.expect("description");
        assert!(!d.contains('|'));
        assert!(d.contains("line one"));
        assert!(d.contains("line two"));
    }

    #[test]
    fn missing_frontmatter_returns_none() {
        let text = "# just a heading, no frontmatter\nsome body\n";
        let (name, desc) = parse_frontmatter_text(text);
        assert_eq!(name, None);
        assert_eq!(desc, None);
    }

    #[test]
    fn rewrite_plain_scalar_name() {
        let text = "---\nname: original-skill\ndescription: Keep me\n---\n\n# Body\n";
        let updated =
            rewrite_frontmatter_name_text(text, "my-alias").expect("should rewrite plain scalar");
        assert_eq!(
            updated,
            "---\nname: my-alias\ndescription: Keep me\n---\n\n# Body\n"
        );
    }

    #[test]
    fn rewrite_quoted_scalar_and_ignore_indented_name() {
        let text = "---\nname: \"old-name\"\nmetadata:\n  name: nested-name\n---\nBody\n";
        assert_eq!(parse_frontmatter_text(text).0.as_deref(), Some("old-name"));
        let updated =
            rewrite_frontmatter_name_text(text, "new-name").expect("should rewrite quoted scalar");
        assert_eq!(
            updated,
            "---\nname: new-name\nmetadata:\n  name: nested-name\n---\nBody\n"
        );
        assert_eq!(
            parse_frontmatter_text(&updated).0.as_deref(),
            Some("new-name")
        );
    }

    #[test]
    fn rewrite_preserves_crlf_and_no_trailing_newline() {
        let text = "---\r\nname: old\r\ndescription: desc\r\n---\r\nBody without newline";
        let updated =
            rewrite_frontmatter_name_text(text, "alias-1").expect("should rewrite with CRLF");
        assert_eq!(
            updated,
            "---\r\nname: alias-1\r\ndescription: desc\r\n---\r\nBody without newline"
        );
    }

    #[test]
    fn rewrite_returns_none_when_name_missing_or_no_frontmatter_or_unchanged() {
        assert_eq!(
            rewrite_frontmatter_name_text("---\ndescription: only desc\n---\n", "alias"),
            None
        );
        assert_eq!(
            rewrite_frontmatter_name_text("# No frontmatter\nname: foo\n", "alias"),
            None
        );
        assert_eq!(
            rewrite_frontmatter_name_text("---\nname: foo\n", "alias"),
            None
        );
        assert_eq!(
            rewrite_frontmatter_name_text("---\nname: \"same-name\"\n---\n", "same-name"),
            None
        );
    }

    #[test]
    fn rewrite_quotes_special_chars_spaces_colons_and_yaml_literals() {
        let base = "---\nname: old\n---\n";
        assert_eq!(
            rewrite_frontmatter_name_text(base, "my skill: v1").as_deref(),
            Some("---\nname: \"my skill: v1\"\n---\n")
        );
        assert_eq!(
            rewrite_frontmatter_name_text(base, "-leading-dash").as_deref(),
            Some("---\nname: \"-leading-dash\"\n---\n")
        );
        assert_eq!(
            rewrite_frontmatter_name_text(base, "true").as_deref(),
            Some("---\nname: \"true\"\n---\n")
        );
        assert_eq!(
            rewrite_frontmatter_name_text(base, "123").as_deref(),
            Some("---\nname: \"123\"\n---\n")
        );
        assert_eq!(
            rewrite_frontmatter_name_text(base, "a \"quoted\" \\ name").as_deref(),
            Some("---\nname: \"a \\\"quoted\\\" \\\\ name\"\n---\n")
        );
        let rewritten = rewrite_frontmatter_name_text(base, "a \"quoted\" \\ name").unwrap();
        assert_eq!(
            parse_frontmatter_text(&rewritten).0.as_deref(),
            Some("a \"quoted\" \\ name")
        );
        assert_eq!(
            rewrite_frontmatter_name_text(&rewritten, "a \"quoted\" \\ name"),
            None
        );
    }

    #[test]
    fn rewrite_preserves_block_scalar_description_and_body_and_bom() {
        let text = concat!(
            "\u{FEFF}---\n",
            "name: frontend-dev\n",
            "description: |\n",
            "  Line one\n",
            "  Line two\n",
            "license: MIT\n",
            "---\n",
            "\n",
            "# Heading\n",
            "Body stays identical.\n",
        );
        let updated = rewrite_frontmatter_name_text(text, "frontend-dev-copy").unwrap();
        let expected = concat!(
            "\u{FEFF}---\n",
            "name: frontend-dev-copy\n",
            "description: |\n",
            "  Line one\n",
            "  Line two\n",
            "license: MIT\n",
            "---\n",
            "\n",
            "# Heading\n",
            "Body stays identical.\n",
        );
        assert_eq!(updated, expected);
    }

    #[test]
    fn rewrite_skill_md_name_handles_disk_file_and_missing_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let missing = temp.path().join("missing.md");
        assert!(!rewrite_skill_md_name(&missing, "alias").expect("missing file ok"));

        let skill_md = temp.path().join("SKILL.md");
        std::fs::write(&skill_md, "---\nname: foo\n---\nbody\n").expect("write SKILL.md");
        assert!(rewrite_skill_md_name(&skill_md, "bar").expect("rewrite ok"));
        assert_eq!(
            std::fs::read_to_string(&skill_md).expect("read back"),
            "---\nname: bar\n---\nbody\n"
        );
        assert!(!rewrite_skill_md_name(&skill_md, "bar").expect("same name no-op"));
    }
}
