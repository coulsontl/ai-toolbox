//! Shared post-processing for Skills sync warnings.

use std::collections::{HashMap, HashSet};

const FOREIGN_PATH_PREFIX: &str = "技能 '";
const FOREIGN_PATH_TOOL_SEPARATOR: &str = " 在工具 '";
const FOREIGN_PATH_PATH_SEPARATOR: &str = "' 的路径 '";
const FOREIGN_PATH_SUFFIXES: &[&str] = &[
    "' 不是 AI Toolbox 管理的同步目标，已保留原样",
    // Legacy wording persisted by older versions.
    "' 不是 AI Toolbox 管理的链接，已保留原样",
];

/// Merge per-skill foreign-path warnings that belong to the same tool into one
/// sentence. The merged message takes the place of the first warning for that
/// tool; the later warnings are removed. Warnings for other tools and
/// unrelated warnings keep their original order.
pub fn merge_foreign_path_warnings(warnings: &mut Vec<String>) {
    struct WarningGroup {
        tool: String,
        first_index: usize,
        skills: Vec<String>,
        indices: Vec<usize>,
    }

    let mut groups: Vec<WarningGroup> = Vec::new();
    let mut group_by_tool: HashMap<String, usize> = HashMap::new();

    for (index, warning) in warnings.iter().enumerate() {
        let Some((skill, tool)) = parse_foreign_path_warning(warning) else {
            continue;
        };

        let group_index = match group_by_tool.get(tool) {
            Some(&group_index) => group_index,
            None => {
                let group_index = groups.len();
                group_by_tool.insert(tool.to_string(), group_index);
                groups.push(WarningGroup {
                    tool: tool.to_string(),
                    first_index: index,
                    skills: Vec::new(),
                    indices: Vec::new(),
                });
                group_index
            }
        };

        let group = &mut groups[group_index];
        group.indices.push(index);
        if !group
            .skills
            .iter()
            .any(|existing| existing.as_str() == skill)
        {
            group.skills.push(skill.to_string());
        }
    }

    let mut merged_by_index: HashMap<usize, String> = HashMap::new();
    let mut removed_indices: HashSet<usize> = HashSet::new();
    for group in groups {
        if group.skills.len() <= 1 {
            continue;
        }

        merged_by_index.insert(
            group.first_index,
            format_merged_warning(&group.tool, &group.skills),
        );
        for index in group.indices {
            if index != group.first_index {
                removed_indices.insert(index);
            }
        }
    }

    if merged_by_index.is_empty() {
        return;
    }

    let mut merged_warnings = Vec::with_capacity(warnings.len());
    for (index, warning) in warnings.drain(..).enumerate() {
        if let Some(merged) = merged_by_index.remove(&index) {
            merged_warnings.push(merged);
        } else if !removed_indices.contains(&index) {
            merged_warnings.push(warning);
        }
    }
    *warnings = merged_warnings;
}

fn parse_foreign_path_warning(message: &str) -> Option<(&str, &str)> {
    let rest = message.strip_prefix(FOREIGN_PATH_PREFIX)?;
    let (skill, rest) = rest.split_once(FOREIGN_PATH_TOOL_SEPARATOR)?;
    let skill = skill.strip_suffix('\'')?;
    let (tool, rest) = rest.split_once(FOREIGN_PATH_PATH_SEPARATOR)?;
    FOREIGN_PATH_SUFFIXES
        .iter()
        .find_map(|suffix| rest.strip_suffix(suffix))?;

    if skill.is_empty() || tool.is_empty() {
        return None;
    }
    Some((skill, tool))
}

fn format_merged_warning(tool: &str, skills: &[String]) -> String {
    let skill_list = skills
        .iter()
        .map(|skill| format!("'{}'", skill))
        .collect::<Vec<_>>()
        .join("、");

    format!(
        "工具 '{}' 下有 {} 个技能路径不是 AI Toolbox 管理的同步目标，已保留原样：{}",
        tool,
        skills.len(),
        skill_list
    )
}

#[cfg(test)]
mod tests {
    use super::merge_foreign_path_warnings;

    fn foreign_path_warning(skill: &str, tool: &str, path: &str) -> String {
        format!(
            "技能 '{}' 在工具 '{}' 的路径 '{}' 不是 AI Toolbox 管理的同步目标，已保留原样",
            skill, tool, path
        )
    }

    #[test]
    fn merges_foreign_path_warnings_by_tool_and_preserves_order() {
        let mut warnings = vec![
            "unrelated warning".to_string(),
            foreign_path_warning("alpha", "ZCode", "~/.zcode/skills/alpha"),
            foreign_path_warning("beta", "ZCode", "~/.zcode/skills/beta"),
            foreign_path_warning("gamma", "Other Tool", "~/.other/skills/gamma"),
            foreign_path_warning("delta", "ZCode", "~/.zcode/skills/delta"),
        ];

        merge_foreign_path_warnings(&mut warnings);

        assert_eq!(
            warnings,
            vec![
                "unrelated warning".to_string(),
                "工具 'ZCode' 下有 3 个技能路径不是 AI Toolbox 管理的同步目标，已保留原样：'alpha'、'beta'、'delta'".to_string(),
                foreign_path_warning("gamma", "Other Tool", "~/.other/skills/gamma"),
            ]
        );
    }

    #[test]
    fn keeps_a_single_foreign_path_warning_unchanged() {
        let warning = foreign_path_warning("alpha", "ZCode", "~/.zcode/skills/alpha");
        let mut warnings = vec![warning.clone()];

        merge_foreign_path_warnings(&mut warnings);

        assert_eq!(warnings, vec![warning]);
    }

    #[test]
    fn deduplicates_repeated_skill_names_within_a_tool() {
        let mut warnings = vec![
            foreign_path_warning("alpha", "ZCode", "~/.zcode/skills/alpha"),
            foreign_path_warning("alpha", "ZCode", "~/.zcode/skills/alpha"),
            foreign_path_warning("beta", "ZCode", "~/.zcode/skills/beta"),
        ];

        merge_foreign_path_warnings(&mut warnings);

        assert_eq!(
            warnings,
            vec!["工具 'ZCode' 下有 2 个技能路径不是 AI Toolbox 管理的同步目标，已保留原样：'alpha'、'beta'".to_string()]
        );
    }

    #[test]
    fn merges_legacy_link_warnings_too() {
        let mut warnings = vec![
            "技能 'alpha' 在工具 'ZCode' 的路径 '~/.zcode/skills/alpha' 不是 AI Toolbox 管理的链接，已保留原样".to_string(),
            "技能 'beta' 在工具 'ZCode' 的路径 '~/.zcode/skills/beta' 不是 AI Toolbox 管理的链接，已保留原样".to_string(),
        ];

        merge_foreign_path_warnings(&mut warnings);

        assert_eq!(
            warnings,
            vec!["工具 'ZCode' 下有 2 个技能路径不是 AI Toolbox 管理的同步目标，已保留原样：'alpha'、'beta'".to_string()]
        );
    }

    #[test]
    fn leaves_non_matching_warnings_untouched() {
        let mut warnings = vec![
            "技能 'alpha' 的源目录不存在，已跳过同步：/tmp/alpha".to_string(),
            "技能 'alpha' 在工具 'ZCode' 的同步目标维护失败：Permission denied".to_string(),
        ];
        let original = warnings.clone();

        merge_foreign_path_warnings(&mut warnings);

        assert_eq!(warnings, original);
    }
}
