---
name: "release-notes"
description: "Use whenever the user asks for release notes, changelog bullets, a GitHub Release body, a 'What's Changed' section, a summary from the last release or tag to now, or wants release copy grounded in real git history, PRs, issues, and candidate release changes. This skill is especially important when the user wants short publish-ready release bullets with repository-specific conventions."
---

# Release Notes

Draft release notes from repository evidence, not guesswork.

This skill is for producing publishable release copy for the next release draft. It is not limited to already published history. Start from the latest relevant tag or release boundary, then include committed and clearly related unreleased candidate changes when the user is drafting the upcoming release before publishing.

## Default target

Unless the user says otherwise, assume the target output is a GitHub Release body draft for the next release in this structure:

1. `## What's Changed`
2. A flat bullet list of user-facing changes
3. The repository-specific Mac install notice at the end

**Copy-friendly final output (required):** Always present the full publishable GitHub Release body (from `## What's Changed` through the Mac install notice, including the database upgrade notice when applicable) inside a single fenced markdown code block, so the user can copy-paste it in one step. Put any research context, range notes, or caveats **outside** that code block (above or below it). Do not leave the What's Changed body as plain markdown only.

Do not silently switch the task to updater notes or in-app update copy. If the repository has multiple release surfaces, identify which one the user wants before drafting. If unspecified, produce the GitHub Release body draft first and mention that updater notes may need a shorter variant.

In this repository, default to Chinese release copy unless the user explicitly asks for English or another language.

## Evidence order

Always work in this order:

1. Determine the release range.
2. Read the commit and diff summary for that range.
3. Check for clearly related staged or unstaged candidate changes that belong to the same upcoming release.
4. Check whether the range or candidate changes include a SQLite database schema upgrade.
5. Read merged PRs and candidate PR context when available.
6. Read related issues for confirmation and issue references.
7. Draft bullets from user-visible impact, not implementation detail.
8. Run a final issue-matching pass for every bullet before returning the publishable draft.

Prefer these sources:

- `git tag --sort=-creatordate` / `git describe --tags --abbrev=0` (then skip prerelease tags when selecting the default base)
- latest **stable** GitHub Release from `gh release list` (ignore titles/tags marked beta/prerelease unless the user asked for that boundary)
- `git diff --stat <stable-base>..HEAD`
- `git log --first-parent --oneline <stable-base>..HEAD`
- `gh repo view --json nameWithOwner,url,defaultBranchRef`
- `gh pr list --state merged ...`
- `gh issue list --state all ...`
- `gh release list` when release publication context matters

For SQLite database schema changes, check commits or diffs that touch `tauri/src/db/migrations.rs`, `TARGET_SCHEMA_VERSION`, `run_migration_step`, `PRAGMA user_version`, `tauri/src/db/sqlite_state.rs`, or migration tests.

If `gh` metadata is unavailable, degrade gracefully to commit-based notes. Do not invent PR numbers, issue numbers, or authors.

## Range rules

- By default, compare the latest **stable (non-prerelease)** tag to `HEAD`.
- In this repository, **beta / alpha / rc / pre tags do not count as the release base**. Treat tags matching `*-beta*`, `*-alpha*`, `*-rc*`, `*-pre*`, or similar prerelease suffixes as draft checkpoints only, not as the start of the next formal release range.
- Prefer the latest formal release tag such as `v1.0.2` over a newer prerelease such as `v1.0.3-beta1`. Example: if tags are `v1.0.2` and `v1.0.3-beta1`, the default range is `v1.0.2..HEAD`, and commits that landed during the beta period are still part of the next formal release notes.
- Only use a beta/prerelease tag as the range base when the user explicitly asks for that beta boundary (for example “from v1.0.3-beta1” or “beta 变更”).
- If the user explicitly names a range like `v0.8.4..v0.8.5`, use that exact range.
- If the latest **stable** tag points to `HEAD` and the user wants the notes for that released version, compare the previous **stable** tag to the current tag.
- Do not focus only on working tree changes. Released or tagged history is usually the real scope.
- When choosing the base with git, do not stop at `git describe --tags --abbrev=0` if that tag is a prerelease. Walk newer tags and pick the latest stable tag that is an ancestor of `HEAD` (or of the requested end ref).

## Dirty worktree rules

- After collecting the default release range, check whether the worktree contains relevant staged or unstaged changes that belong to the same upcoming release.
- For next-release drafting in this repository, relevant staged or unstaged changes are IN scope by default even if they are not yet tagged, merged, or published.
- Do not ask for confirmation before including clearly related uncommitted changes in the draft unless the user explicitly asks for committed-only notes.
- Do not confuse "in upcoming draft" with "already published". These changes can appear in the release draft, but the surrounding explanation should not claim the release has already happened.
- If relevant uncommitted work exists, merge it into the draft by default when the topic match is strong.
- If an issue is clearly tied to unreleased candidate work, it can appear in the draft even when the issue is still open.

## Writing rules

- Prefer PR titles as the primary basis for bullets because they already encode the change unit users can trace.
- Rewrite titles only as much as needed for clarity and consistency.
- Prefer user-visible impact over refactors, internal naming, or storage details.
- Drop purely internal changes unless they materially affect user behavior, reliability, compatibility, or supported providers/models.
- Keep bullets concise and publishable.
- If one commit or PR spans multiple user-facing themes, split the release wording by product surface instead of mechanically following the commit scope, module name, or title prefix.
- You MUST merge closely related commits or fixes into one bullet when they belong to the same user-facing theme.
- Prefer 2-4 strong bullets over a long fragmented list when drafting a normal release.
- Favor product-surface labels such as `模型预设`、`会话管理`、`Claude Code / WSL / SSH` over raw commit scopes like `feat(models)` or `fix(session)` when drafting Chinese release notes for this repository.
- By default, emit at most one bullet per product surface.
- Keep each bullet as short as possible. Prefer a compact noun phrase or short verb phrase over a full explanatory sentence.
- Avoid explanatory tails such as `让...`、`避免...`、`减少...`、`更稳...` unless removing them would make the change unclear.
- Use a flat list. Do not create nested groups unless the user explicitly asks for grouped sections.
- Preserve known regressions or unverified boundaries only when the user asks for them, or when omitting them would materially mislead the release notes.

## Attribution rules

Default bullet shape:

`- <type(scope)>: <user-facing summary> in #<pr_number>`

Contributor bullet shape:

`- <type(scope)>: <user-facing summary> by @<author_login> in #<pr_number>`

Apply these attribution rules:

- If the PR author is the repository owner or the user's own account, omit `by @...`.
- In this repository, treat `coulsontl` as self-authored and omit the `@` attribution.
- For other contributors, include `by @<login>`.
- If no PR can be confirmed, fall back to a commit-based bullet without fake attribution or fake PR numbers.

Commit fallback shape for research only:

`- <internal draft only> <summary> (from <short_sha>)`

Do not emit `(from <short_sha>)` in the final publishable release draft unless the user explicitly asks for evidence annotations.

## Issue rules

- Read issues as research evidence, not as automatic output lines.
- Only mention an issue number in the release notes when the PR, commit, candidate patch, or explicit user instruction clearly ties that issue to the change being drafted.
- Do not add a separate `Closed Issues` section by default.
- If the user wants known problems called out, add a `## Known Issues` section after `What's Changed`.
- Open issues can still matter during next-release drafting. When an open issue clearly matches relevant in-progress work in the dirty worktree, it may be included in the draft as a pending fix or candidate item.
- In this repository, use semantic issue matching more aggressively for final bullets. If an issue title/body strongly matches the user-facing change and the touched files or behavior also match, append `，关联 #<issue>` even when the commit or PR does not explicitly mention the issue number.
- Prefer appending at most one or two issue numbers to a bullet. Do not attach a long list of loosely related issues.
- If a bullet combines multiple subchanges, keep only the strongest issue links that still make sense for the merged wording.

### Default issue-matching pass

Before returning the final release draft, perform this pass for every bullet:

1. Extract the bullet's product surface, user-facing behavior, and 2-5 key phrases.
2. Search open and recently active issues for matching phrases in the title and body.
3. Cross-check the issue against touched files, touched module, and described behavior.
4. If one or two issues are a strong semantic match, append `，关联 #<issue>`.
5. If the match is weak, ambiguous, or only shares a broad area name, omit the issue number.

Strong match signals:

- Same product surface or module family
- Same user-visible symptom or workflow
- Same key noun or action in title/body and draft bullet
- Touched files or implementation area line up with the issue description

Weak match signals:

- Only the module name overlaps
- The issue is about a broader feature area but not the same symptom
- Multiple candidate issues fit equally well and none is clearly best

Default behavior in this repository:

- Try to auto-match issues for all final bullets.
- Prefer open issues and recently active issues when drafting the next release.
- Omit the issue number only when no strong semantic match is found.

## Database Upgrade Notice

If the release range or included candidate changes increase the SQLite `TARGET_SCHEMA_VERSION` or add a new SQLite schema migration, append the database compatibility notice after the `What's Changed` bullets and before the Mac install notice.

Use this exact Chinese notice unless the user asks for a different surface or wording:

```md
⚠️ 数据库升级提示：本版本包含数据库结构升级。升级后请不要直接降级到旧版本，否则旧版本可能因数据库版本不匹配而无法启动。如需降级，请先在当前版本的设置里执行一次数据备份，确认备份完成后退出应用，再到应用数据目录的 `sqlite-migration-backups` 中找到升级前自动创建的 `.db` 备份，并用它替换当前的 `ai-toolbox.db`。如果升级后做过内容变更，替换数据库后会回到升级前状态；需要找回这些变更时，请在可兼容该备份的版本中通过设置的数据恢复导入刚才创建的备份。
```

Do not add this notice for ordinary data changes, seed data changes, model preset/resource updates, or non-SQLite runtime database files. The signal must be a SQLite schema/user_version upgrade, not merely a touched database-related file. Keep the normal in-app data backup and the migration backup distinct: the normal backup is a safety copy of the current state, while the downgrade-compatible database is the pre-migration `.db` under `sqlite-migration-backups`. Do not promise that an older app version can restore a backup created by a newer schema; say to restore it only in a version compatible with that backup.

## Output template

Use this default GitHub Release body template unless the user requests another surface.

In the chat reply, wrap the entire publishable body in one fenced code block labeled `md` (as shown below). Optional research notes stay outside the fence.

````
```md
## What's Changed

- 模型预设：一句话概括同一主题下的模型或预设更新
- 会话管理：一句话概括同一主题下的会话或删除流程修复
- Claude Code / WSL / SSH：一句话概括同一主题下的跨端同步或路径修复，可在末尾带 `，关联 #123`

⚠️ 数据库升级提示：本版本包含数据库结构升级。升级后请不要直接降级到旧版本，否则旧版本可能因数据库版本不匹配而无法启动。如需降级，请先在当前版本的设置里执行一次数据备份，确认备份完成后退出应用，再到应用数据目录的 `sqlite-migration-backups` 中找到升级前自动创建的 `.db` 备份，并用它替换当前的 `ai-toolbox.db`。如果升级后做过内容变更，替换数据库后会回到升级前状态；需要找回这些变更时，请在可兼容该备份的版本中通过设置的数据恢复导入刚才创建的备份。

⚠️ Mac安装提示：`"应用程序"已损坏` 解决方法：`xattr -cr /Applications/AI\ Toolbox.app`
```
````

Template rules:

- Replace placeholders with real repository data.
- Do not copy another project's release wording.
- Do not output placeholder bullets in the final answer.
- Always fence the final GitHub Release body in a single ` ```md ` code block for one-click copy; keep range/research notes outside the fence.
- Keep the Mac install notice exactly as written for this repository unless the user asks to change it.
- Do not include `Full Changelog` by default in this repository unless the user explicitly asks for it.
- Include the database upgrade notice only when a SQLite schema/user_version upgrade is in scope.
- Prefer the shortest acceptable wording that still preserves the user-facing meaning.

## Style rules

- Keep the tone factual, compact, and release-ready.
- Do not over-explain implementation details inside the bullets.
- Avoid marketing language.
- Avoid claiming a fix is shipped and verified unless the evidence supports it.
- When evidence is incomplete, say so briefly before the draft or return a research summary first.
- In this repository, prefer concise Chinese wording for the final draft unless the user requests another language.
- When drafting the next release, it is acceptable to include not-yet-closed issues if the linked work is clearly intended for that release.
- Favor direct short phrases that can be pasted into the release page with minimal editing.
- Always put the publishable What's Changed body (plus required notices) in a fenced `md` code block so the user can copy it without selecting surrounding prose.
- The final release draft should read like a polished changelog, not like traced research notes or commit summaries.
- Do not use hedge words like `candidate`, `pending`, or similar qualifiers in the final release draft unless the user explicitly asks for that uncertainty to be surfaced.

## Good and bad examples

Bad:

`- 会话管理：优化 Session Manager 路径选项加载与 OpenCode 会话删除流程，删除缺失底层记录时不再误报失败，批量清理也更稳`

Good:

`- 会话管理：优化路径加载与会话删除流程`

Bad:

`- 会话管理：修复 KeepAlive 隐藏页操作的反馈竞态，避免 loading 状态卡住或提示不一致`

Good:

`- 会话管理：修复 KeepAlive 状态反馈问题`

Bad:

`- 模型预设：新增 Qwen3.6 Plus、Kimi K2.6 和 GPT-5.5 预设，并调整相关展示逻辑，让 Codex 卡片按配置显示 reasoning effort`

Good:

`- 模型预设：新增 GPT-5.5、Qwen3.6 Plus 和 Kimi K2.6`

Bad:

`- 会话管理：优化路径加载、OpenCode 会话删除流程与 KeepAlive 状态反馈`

Good:

`- 会话管理：优化路径加载、OpenCode 会话删除流程与 KeepAlive 状态反馈，关联 #158`

Bad:

`- Claude Code / WSL / SSH：同步插件元数据时自动改写远端安装路径`

Good:

`- Claude Code / WSL / SSH：同步插件元数据时自动改写远端安装路径，关联 #161`

Bad:

`- 会话管理：优化路径加载、OpenCode 会话删除流程与 KeepAlive 状态反馈`

Reason:

`OpenCode 会话删除` 与 issue 症状强匹配，却漏掉了应追加的 issue 关联。

Good:

`- 会话管理：优化路径加载、OpenCode 会话删除流程与 KeepAlive 状态反馈，关联 #158`

Bad:

`- Claude Code / WSL / SSH：同步插件元数据时自动改写远端安装路径，关联 #158`

Reason:

`#158` 只匹配 OpenCode 删除慢，不匹配 Claude plugins 路径同步；这是错误挂号。

Good:

`- Claude Code / WSL / SSH：同步插件元数据时自动改写远端安装路径，关联 #161`

Bad:

`- 模型预设：新增 GPT-5.5、Qwen3.6 Plus 和 Kimi K2.6，关联 #158`

Reason:

模型预设更新与 `#158` 的用户问题不强匹配，不应为了“每条都带号”而乱挂 issue。

Good:

`- 模型预设：新增 GPT-5.5、Qwen3.6 Plus 和 Kimi K2.6`

## Minimal checklist

Before finalizing, verify:

1. The compare range is explicit, and the default base is a **stable** tag (not a beta/prerelease) unless the user asked otherwise.
2. Every bullet is backed by a PR or commit in that range.
3. Every `#123` actually exists.
4. Every `by @user` is real and omitted for self-authored PRs.
5. The Mac install notice is present at the end for GitHub Release body output.
6. The full publishable body is wrapped in one fenced `md` code block for easy copy; research notes stay outside the fence.
7. Any unreleased candidate items are worded as next-release draft content, not falsely described as already published.
8. The final draft does not expose raw commit hashes, `(from ...)` tails, or commit-scope-first wording unless the user explicitly asks for that format.
9. Relevant uncommitted changes are included by default for next-release drafting unless the user explicitly asks for committed-only notes.
10. If the release includes a SQLite schema/user_version upgrade, the database upgrade notice is present before the Mac install notice.
11. Beta-period commits between the last stable tag and `HEAD` are included in the next formal release notes, not omitted just because a beta tag exists.
