//! Shared ownership and replacement rules for WSL/SSH skill targets.

#[derive(Clone, Copy)]
pub enum RemoteSkillTargetAction {
    Link,
    Copy,
    Remove,
}

impl RemoteSkillTargetAction {
    pub fn sync_for_tool(tool_key: &str) -> Self {
        if crate::coding::tools::builtin_tool_forces_skill_copy(tool_key) {
            Self::Copy
        } else {
            Self::Link
        }
    }
}

const MANAGE_TARGET_BODY: &str = r#"
skill_source=$1
skill_target=$2
central_root=$3
action=$4
marker=.ai-toolbox-skill-source

# These paths are generated from runtime roots and one skill name. Reject
# ambiguous traversal before any filesystem mutation.
case "/$skill_source/$skill_target/" in */../*|*/./*) echo 'Skill paths contain dot segments' >&2; exit 1;; esac
resolve_parent() {
    ancestor=$1
    suffix=
    while [ ! -d "$ancestor" ]; do
        suffix="/$(basename -- "$ancestor")$suffix"
        next_ancestor=$(dirname -- "$ancestor")
        [ "$next_ancestor" != "$ancestor" ] || return 1
        ancestor=$next_ancestor
    done
    resolved_ancestor=$(cd -- "$ancestor" && pwd -P) || return 1
    printf '%s%s\n' "$resolved_ancestor" "$suffix"
}

# Resolve the parent, not a target symlink that we may need to replace.
target_parent=$(dirname -- "$skill_target")
if [ "$action" = remove ] && [ ! -d "$target_parent" ]; then
    printf 'unchanged\n'
    exit 0
fi
target_parent=$(resolve_parent "$target_parent")
skill_target="$target_parent/$(basename -- "$skill_target")"
central_root=$(cd -- "$central_root" && pwd -P)
skill_source="$central_root/$(basename -- "$skill_source")"
case "$skill_target/" in "$central_root/"*) echo 'Skill target overlaps central repository' >&2; exit 1;; esac
case "$central_root/" in "$skill_target/"*) echo 'Central repository overlaps skill target' >&2; exit 1;; esac

kind=missing
if [ -L "$skill_target" ]; then
    link_source=$(readlink -- "$skill_target")
    case "$link_source" in "$central_root"/*|"$1") kind=link;; *) kind=foreign;; esac
    case "/$link_source/" in */../*|*/./*) kind=foreign;; esac
elif [ -e "$skill_target" ]; then
    kind=foreign
    if [ -d "$skill_target" ] && [ ! -L "$skill_target/$marker" ] && [ -f "$skill_target/$marker" ]; then
        if [ "$(cat -- "$skill_target/$marker")" = "$skill_source" ]; then kind=copy; fi
    fi
fi
if [ "$kind" = foreign ]; then printf 'foreign\n'; exit 0; fi

if [ "$action" = remove ]; then
    if [ "$kind" = missing ]; then printf 'unchanged\n'; exit 0; fi
    rm -rf -- "$skill_target"
    printf 'updated\n'
    exit 0
fi

# Finish preparing the new target before moving the previous one aside.
test -d "$skill_source"
if [ "$action" = link ] && [ "$kind" = link ] && [ "$link_source" = "$skill_source" ]; then
    printf 'unchanged\n'; exit 0
fi
if [ "$action" = copy ] && [ "$kind" = copy ] && [ -f "$skill_source/.synced_hash" ] &&
    cmp -s -- "$skill_source/.synced_hash" "$skill_target/.synced_hash"; then
    printf 'unchanged\n'; exit 0
fi
mkdir -p -- "$target_parent"
staging=$(mktemp -d "$target_parent/.ai-toolbox-skill.XXXXXX")
trap 'rm -rf -- "$staging"' EXIT
if [ "$action" = copy ]; then
    mkdir -- "$staging/next"
    cp -R -P -- "$skill_source"/. "$staging/next"/
    rm -f -- "$staging/next/$marker"
    printf '%s\n' "$skill_source" > "$staging/next/$marker"
else
    ln -s -- "$skill_source" "$staging/next"
fi
if [ "$kind" != missing ]; then mv -- "$skill_target" "$staging/previous"; fi
if ! mv -- "$staging/next" "$skill_target"; then
    if [ "$kind" != missing ] && ! mv -- "$staging/previous" "$skill_target"; then
        trap - EXIT
        echo "Previous skill target preserved at $staging/previous" >&2
    fi
    exit 1
fi
printf 'updated\n'
"#;

fn quote_shell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn remote_path_argument(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(relative) => format!("\"$HOME\"/{}", quote_shell(relative)),
        None => quote_shell(path),
    }
}

fn action_name(action: RemoteSkillTargetAction) -> &'static str {
    match action {
        RemoteSkillTargetAction::Link => "link",
        RemoteSkillTargetAction::Copy => "copy",
        RemoteSkillTargetAction::Remove => "remove",
    }
}

pub fn build_remote_skill_target_command(
    source: &str,
    target: &str,
    central_root: &str,
    action: RemoteSkillTargetAction,
) -> String {
    let script = format!("set -eu\n{}", MANAGE_TARGET_BODY);
    format!(
        "sh -c {} ai-toolbox-skills {} {} {} {}",
        quote_shell(&script),
        remote_path_argument(source),
        remote_path_argument(target),
        remote_path_argument(central_root),
        action_name(action),
    )
}

/// One target operation in a batched remote call.
pub struct SkillTargetOperation {
    pub source: String,
    pub target: String,
    pub central_root: String,
    pub action: RemoteSkillTargetAction,
}

/// Build one `sh -c` command that runs several target operations in a single
/// SSH round trip. Each operation runs in its own command substitution so an
/// `exit`/error in one cannot abort the rest.
pub fn build_remote_skill_target_batch_command(operations: &[SkillTargetOperation]) -> String {
    let mut script = String::from("set -u\nmanage_target() {\nset -eu\n");
    script.push_str(MANAGE_TARGET_BODY);
    script.push_str(
        "\n}\nrun_one() {\n  index=$1\n  shift\n  output=$(manage_target \"$@\" 2>&1) && status=0 || status=$?\n  if [ \"$status\" -eq 0 ]; then\n    printf '%s\\t%s\\n' \"$index\" \"$output\"\n  else\n    printf '%s\\terror\\t%s\\n' \"$index\" \"$(printf '%s' \"$output\" | tr '\\n' ' ')\"\n  fi\n}\n",
    );

    for (index, operation) in operations.iter().enumerate() {
        script.push_str(&format!(
            "run_one {} {} {} {} {}\n",
            index,
            remote_path_argument(&operation.source),
            remote_path_argument(&operation.target),
            remote_path_argument(&operation.central_root),
            action_name(operation.action),
        ));
    }

    format!("sh -c {} ai-toolbox-skills-batch", quote_shell(&script))
}

/// Parse the `index<TAB>status` lines emitted by the batch script.
pub fn parse_remote_skill_target_batch_result(
    output: &str,
    expected_count: usize,
) -> Result<Vec<Result<bool, String>>, String> {
    let mut results: Vec<Option<Result<bool, String>>> =
        (0..expected_count).map(|_| None).collect();

    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let mut parts = line.splitn(3, '\t');
        let index_text = parts
            .next()
            .ok_or_else(|| format!("Missing skill target batch index: {line}"))?;
        let index: usize = index_text
            .parse()
            .map_err(|_| format!("Invalid skill target batch index: {line}"))?;
        if index >= expected_count {
            return Err(format!("Skill target batch index out of range: {line}"));
        }
        if results[index].is_some() {
            return Err(format!("Duplicate skill target batch result: {line}"));
        }

        let status = parts
            .next()
            .ok_or_else(|| format!("Missing skill target batch status: {line}"))?;
        let result = match status {
            "updated" | "unchanged" => Ok(true),
            "foreign" => Ok(false),
            "error" => Err(parts.next().unwrap_or("").to_string()),
            _ => return Err(format!("Unexpected skill target batch status: {line}")),
        };
        results[index] = Some(result);
    }

    results
        .into_iter()
        .enumerate()
        .map(|(index, result)| {
            result.ok_or_else(|| format!("Missing skill target batch result for operation {index}"))
        })
        .collect()
}

/// False means a user-owned target was preserved, not a successful sync.
pub fn parse_remote_skill_target_result(output: &str) -> Result<bool, String> {
    match output.trim() {
        "updated" | "unchanged" => Ok(true),
        "foreign" => Ok(false),
        output => Err(format!("Unexpected skill target result: {output}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_path_arguments_only_expand_the_home_prefix() {
        assert_eq!(
            remote_path_argument("~/skills/a'b $name"),
            "\"$HOME\"/'skills/a'\\''b $name'"
        );
        assert_eq!(
            remote_path_argument("/tmp/$(touch unwanted)"),
            "'/tmp/$(touch unwanted)'"
        );
        assert_eq!(parse_remote_skill_target_result("foreign\n"), Ok(false));
        assert!(parse_remote_skill_target_result("").is_err());
    }

    #[cfg(unix)]
    fn run_target(
        source: &std::path::Path,
        target: &std::path::Path,
        central: &std::path::Path,
        action: RemoteSkillTargetAction,
    ) -> Result<bool, String> {
        let command = build_remote_skill_target_command(
            source.to_str().unwrap(),
            target.to_str().unwrap(),
            central.to_str().unwrap(),
            action,
        );
        let output = std::process::Command::new("sh")
            .args(["-c", &command])
            .output()
            .unwrap();
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        parse_remote_skill_target_result(&String::from_utf8_lossy(&output.stdout))
    }

    #[cfg(unix)]
    #[test]
    fn remote_copy_updates_and_removes_only_owned_targets() {
        let directory = tempfile::tempdir().unwrap();
        let central = directory.path().join("central");
        let source = central.join("test ' $skill");
        let target = directory.path().join("tool/test ' $skill");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("SKILL.md"), "first").unwrap();
        std::fs::write(source.join("removed.txt"), "old").unwrap();
        assert_eq!(
            run_target(&source, &target, &central, RemoteSkillTargetAction::Copy),
            Ok(true)
        );
        assert!(std::fs::read_link(&target).is_err());
        std::fs::write(source.join("SKILL.md"), "second").unwrap();
        std::fs::remove_file(source.join("removed.txt")).unwrap();
        assert_eq!(
            run_target(&source, &target, &central, RemoteSkillTargetAction::Copy),
            Ok(true)
        );
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "second"
        );
        assert!(!target.join("removed.txt").exists());
        std::fs::remove_dir_all(&source).unwrap();
        assert!(run_target(&source, &target, &central, RemoteSkillTargetAction::Copy).is_err());
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "second"
        );
        assert_eq!(
            run_target(&source, &target, &central, RemoteSkillTargetAction::Remove),
            Ok(true)
        );
        assert!(!target.exists());
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("SKILL.md"), "user").unwrap();
        assert_eq!(
            run_target(&source, &target, &central, RemoteSkillTargetAction::Remove),
            Ok(false)
        );
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "user"
        );
    }

    #[cfg(unix)]
    #[test]
    fn remote_copy_migrates_managed_links_and_protects_foreign_links_and_overlap() {
        let directory = tempfile::tempdir().unwrap();
        let central = directory.path().join("central");
        let source = central.join("skill");
        let target = directory.path().join("tool/skill");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("SKILL.md"), "content").unwrap();
        assert_eq!(
            run_target(&source, &target, &central, RemoteSkillTargetAction::Link),
            Ok(true)
        );
        assert_eq!(
            run_target(&source, &target, &central, RemoteSkillTargetAction::Copy),
            Ok(true)
        );
        assert!(std::fs::read_link(&target).is_err());
        assert_eq!(
            run_target(&source, &target, &central, RemoteSkillTargetAction::Remove),
            Ok(true)
        );
        std::os::unix::fs::symlink(directory.path(), &target).unwrap();
        for action in [
            RemoteSkillTargetAction::Copy,
            RemoteSkillTargetAction::Remove,
        ] {
            assert_eq!(run_target(&source, &target, &central, action), Ok(false));
        }
        assert!(run_target(&source, &source, &central, RemoteSkillTargetAction::Copy).is_err());
        let nested = source.join("missing/child/skill");
        assert!(run_target(&source, &nested, &central, RemoteSkillTargetAction::Copy).is_err());
        assert!(!source.join("missing").exists());
        let alias = directory.path().join("central-alias");
        std::os::unix::fs::symlink(&central, &alias).unwrap();
        assert!(run_target(
            &source,
            &alias.join("skill"),
            &central,
            RemoteSkillTargetAction::Copy
        )
        .is_err());
        assert!(source.join("SKILL.md").exists());
    }

    #[test]
    fn parses_batched_target_results_in_operation_order() {
        let results =
            parse_remote_skill_target_batch_result("0\tunchanged\n2\tforeign\n1\terror\tboom\n", 3)
                .unwrap();

        assert_eq!(results, vec![Ok(true), Err("boom".to_string()), Ok(false)]);
    }

    #[test]
    fn rejects_incomplete_or_duplicate_batch_results() {
        assert!(parse_remote_skill_target_batch_result("0\tunchanged\n", 2).is_err());
        assert!(parse_remote_skill_target_batch_result("0\tunchanged\n0\tupdated\n", 2).is_err());
        assert!(parse_remote_skill_target_batch_result("0\tunknown\n", 1).is_err());
    }

    #[cfg(unix)]
    fn run_batch(operations: &[SkillTargetOperation]) -> Vec<Result<bool, String>> {
        let command = build_remote_skill_target_batch_command(operations);
        let output = std::process::Command::new("sh")
            .args(["-c", &command])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "batch failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        parse_remote_skill_target_batch_result(
            &String::from_utf8_lossy(&output.stdout),
            operations.len(),
        )
        .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn batched_operations_report_each_result_and_isolate_failures() {
        let directory = tempfile::tempdir().unwrap();
        let central = directory.path().join("central");
        let source_a = central.join("skill-a");
        let source_b = central.join("skill-b");
        std::fs::create_dir_all(&source_a).unwrap();
        std::fs::create_dir_all(&source_b).unwrap();
        std::fs::write(source_a.join("SKILL.md"), "a").unwrap();
        std::fs::write(source_b.join("SKILL.md"), "b").unwrap();

        let target_a = directory.path().join("tool-a/skill-a");
        let target_b = directory.path().join("tool-b/skill-b");
        let foreign_target = directory.path().join("tool-c/skill-a");
        std::fs::create_dir_all(&foreign_target).unwrap();
        std::fs::write(foreign_target.join("SKILL.md"), "user").unwrap();
        let missing_target = directory.path().join("tool-d/skill-a");

        let operations = vec![
            SkillTargetOperation {
                source: source_a.to_string_lossy().to_string(),
                target: target_a.to_string_lossy().to_string(),
                central_root: central.to_string_lossy().to_string(),
                action: RemoteSkillTargetAction::Link,
            },
            SkillTargetOperation {
                source: source_b.to_string_lossy().to_string(),
                target: target_b.to_string_lossy().to_string(),
                central_root: central.to_string_lossy().to_string(),
                action: RemoteSkillTargetAction::Copy,
            },
            SkillTargetOperation {
                source: source_a.to_string_lossy().to_string(),
                target: foreign_target.to_string_lossy().to_string(),
                central_root: central.to_string_lossy().to_string(),
                action: RemoteSkillTargetAction::Link,
            },
            SkillTargetOperation {
                source: central.join("missing-source").to_string_lossy().to_string(),
                target: directory
                    .path()
                    .join("tool-e/skill-a")
                    .to_string_lossy()
                    .to_string(),
                central_root: central.to_string_lossy().to_string(),
                action: RemoteSkillTargetAction::Link,
            },
            SkillTargetOperation {
                source: source_a.to_string_lossy().to_string(),
                target: missing_target.to_string_lossy().to_string(),
                central_root: central.to_string_lossy().to_string(),
                action: RemoteSkillTargetAction::Remove,
            },
        ];

        let results = run_batch(&operations);

        assert_eq!(results[0], Ok(true));
        assert_eq!(results[1], Ok(true));
        assert_eq!(results[2], Ok(false));
        assert!(results[3].is_err(), "missing source should fail");
        assert_eq!(results[4], Ok(true));

        assert_eq!(std::fs::read_link(&target_a).unwrap(), source_a);
        assert!(target_b.join("SKILL.md").exists());
        assert_eq!(
            std::fs::read_to_string(foreign_target.join("SKILL.md")).unwrap(),
            "user"
        );
        assert!(!missing_target.exists());
    }
}
