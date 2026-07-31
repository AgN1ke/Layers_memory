# Versioning Policy

## Purpose

This page defines how Memory Engine versions its distributable library,
adapters, Git releases, and persisted/wire schemas. It is the normative policy
for future releases; release notes and package manifests must not invent their
own versioning rules.

## Related pages

- [Project README](../../../README.md) - public entry point and short versioning summary.
- [Contracts](../foundation/contracts.md) - persisted and wire-level schema versions.
- [Roadmap](../planning/roadmap.md) - accepted product milestones and release scope.
- [HISTORY](../../../HISTORY.md) - compatibility and public-claim record.
- [DEVLOG](../../../DEVLOG.md) - working record of why this policy was adopted.
- [Release acceptance pages](../releases/v0.3-acceptance.md) - examples of evidence required before a tag.

## Decision

Memory Engine uses [Semantic Versioning](https://semver.org/) for library and
adapter releases. A release version has three numbers:

```text
MAJOR.MINOR.PATCH
```

The numbers describe compatibility, not the size of the diff:

- `MAJOR` changes when a public consumer must change its code or data handling
  to upgrade;
- `MINOR` changes when backward-compatible functionality is added;
- `PATCH` changes when a backward-compatible defect is fixed without adding a
  new public capability.

Removing a function is only one kind of breaking change. Renaming a method,
changing required JSON fields, changing a function signature, rejecting data
that was previously valid, or changing documented behavior that a host relies
on may also require a breaking release.

## Policy Before 1.0

Versions beginning with `0` are explicitly pre-stable. Before `1.0.0` we use:

| Change | Version rule |
|---|---|
| Backward-compatible bug fix | `0.Y.Z` -> `0.Y.(Z+1)` |
| Backward-compatible public feature | `0.Y.Z` -> `0.(Y+1).0` |
| Intentional breaking public change | `0.Y.Z` -> `0.(Y+1).0`, with an explicit migration note |

Pre-1.0 SemVer cannot distinguish a compatible feature from a breaking change
using the number alone. Therefore every breaking `0.Y.0` release must say
`BREAKING` in HISTORY/release notes and provide the consumer migration path.

After `1.0.0`, ordinary SemVer applies strictly:

| Change | Example |
|---|---|
| Breaking public change | `1.4.2` -> `2.0.0` |
| Backward-compatible feature | `1.4.2` -> `1.5.0` |
| Backward-compatible fix | `1.4.2` -> `1.4.3` |

## What Counts as the Public Surface

Compatibility includes every supported way a host consumes the library:

- public Rust types, methods, constructor options, and documented defaults;
- JSON requests and results at the engine boundary;
- Python adapter methods, arguments, return payloads, and package name;
- Godot/GDExtension entry points and their JSON boundary;
- documented behavior that changes which memory is returned or how lifecycle
  operations behave;
- persisted formats when an upgrade cannot read old data or migrate it safely.

Internal refactoring that preserves these boundaries does not require a feature
or breaking release by itself.

## Release Version vs Schema Version

Library versions and schema versions solve different problems.

Library/adapters/tags use SemVer:

```text
0.4.0
v0.4.0
```

Persisted and wire contracts use independent named versions:

```text
event.v1
archive_entry.v1
recall_result.v1
```

An additive optional field can remain in a `.v1` schema when old payloads are
still valid and old consumers can safely ignore the field. Removing or
reinterpreting a required field requires a new schema version and, for stored
data, a tested migration. A schema bump may also require a SemVer breaking
release when hosts must change to upgrade.

Example: adding optional `matched_memory_unit_id` was compatible within
`recall_result.v1`, but it is part of a new public Stage 2 capability and
therefore belongs in a MINOR library release rather than a PATCH.

## One Release Version Everywhere

The canonical release number is the root Cargo workspace
`[workspace.package].version`. For a release, the following must match exactly:

- root Cargo workspace version inherited by all Rust crates;
- `crates/python_adapter/pyproject.toml` package version;
- Git tag `vMAJOR.MINOR.PATCH`;
- release acceptance page and HISTORY entry.

A release is blocked when these values disagree. Tags are immutable: do not
move or rewrite an existing public tag to repair a historical mismatch.

The repository enforces this boundary with:

```powershell
python tools\check_release_versions.py
```

Normal commits must keep Cargo and Python manifests aligned. A release/tag gate
adds the exact tag and evidence checks:

```powershell
python tools\check_release_versions.py `
  --tag v0.4.0 `
  --require-release-evidence
```

For normalized releases, the evidence page is named with all three numbers,
for example `wiki/pages/releases/v0.4.0-acceptance.md`. It must contain exact
`Release: v0.4.0` and `Status: accepted` lines, and HISTORY must have a heading
containing the same tag. CI runs the strict form automatically on `v*` pushes.

## Historical State and Next Normalized Release

The repository historically used `v0.2.0` and `v0.3.0` as accepted project
milestones while Cargo and the Python package remained at `0.1.0`. We preserve
those tags as history and do not pretend the manifests were synchronized.

The vector Stage 2 and unit-focused recall work is a new public capability. If
its live acceptance passes, the next normalized release target is `0.4.0`. The
version is changed only in the release preparation commit, not merely because
the target has been named here.

## Release Checklist

Before creating `vX.Y.Z`:

1. Classify the change as breaking, feature, or fix against the public surface.
2. Update Cargo and Python package versions to the same `X.Y.Z`.
3. Update HISTORY with compatibility and migration notes.
4. Run `python tools/check_release_versions.py` and the full Rust, adapter,
   host-conformance, and wiki gates required by the
   relevant acceptance page.
5. Mark the exact three-number acceptance page `Status: accepted` only after
   its evidence has actually passed.
6. Confirm the release commit is the exact commit being tagged.
7. Create the immutable annotated tag `vX.Y.Z` only after the strict release
   gate accepts that commit.

Version numbers do not replace evidence. A feature is not released merely
because a manifest was edited, and a milestone is not complete merely because
a tag was created.
