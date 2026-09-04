#!/usr/bin/env bash

set -euo pipefail

# Keep release automation non-interactive when Git or related tools would
# otherwise open a pager such as less.
export GIT_PAGER=cat
export GH_PAGER=cat
export PAGER=cat
export LESS=FRX

usage() {
  cat <<'EOF'
Prepare and push a stable cleanr release tag.

Usage:
  scripts/release.sh <version> [options]

Examples:
  scripts/release.sh 0.2.0
  scripts/release.sh 0.2.0 --prepare
  scripts/release.sh 0.2.0 --check

Options:
  --prepare     Update version files and promote Unreleased notes; do not publish.
  --check       Validate that files already match <version>; do not edit files.
  --publish     Explicitly select the default publish mode.
  -h, --help    Show this help.

By default the script requires a clean worktree, updates all version files,
creates a release commit and annotated v<version> tag, then pushes the branch
and tag to origin. The tag starts release.yml, where CI validates, builds, and
publishes the release after same-commit quality and installation checks pass.
Write concrete changes under CHANGELOG.md's Unreleased heading before preparing
a new version; review the promoted version notes alongside the version changes.
EOF
}

die() {
  echo "release: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

is_release_file() {
  case "$1" in
    Cargo.toml|Cargo.lock|CHANGELOG.md|npm/cleanr/package.json|crates/*/Cargo.toml|plugins/index.json|plugins/*/plugin.toml|plugins/*/rules/*.toml|plugins/*/locations/*.toml|plugins/*/locales/*.yml|plugins/*/locales/*.yaml|crates/rules/builtin-plugins/*/plugin.toml|crates/rules/builtin-plugins/*/rules/*.toml|crates/rules/builtin-plugins/*/locations/*.toml|crates/rules/builtin-plugins/*/locales/*.yml|crates/rules/builtin-plugins/*/locales/*.yaml)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

changed_files() {
  {
    git diff --name-only
    git diff --cached --name-only
    git ls-files --others --exclude-standard
  } | sort -u
}

check_release_files() {
  while IFS= read -r file; do
    [ -z "$file" ] && continue
    is_release_file "$file" ||
      die "release preparation changed an unexpected file: ${file}"
  done
}

version=""
mode="publish"
mode_selected=false

while [ "$#" -gt 0 ]; do
  case "$1" in
    --prepare)
      [ "$mode_selected" = false ] || die "only one mode may be selected"
      mode="prepare"
      mode_selected=true
      ;;
    --check)
      [ "$mode_selected" = false ] || die "only one mode may be selected"
      mode="check"
      mode_selected=true
      ;;
    --publish)
      [ "$mode_selected" = false ] || die "only one mode may be selected"
      mode="publish"
      mode_selected=true
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      die "unknown option: $1"
      ;;
    *)
      [ -z "$version" ] || die "only one version may be specified"
      version="${1#v}"
      ;;
  esac
  shift
done

[ -n "$version" ] || {
  usage >&2
  exit 1
}

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  die "version must be stable semantic versioning, for example 1.2.3"
fi

tag="v${version}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

need cargo
need git
need node

prepared=false
if [ "$mode" = "publish" ]; then
  git remote get-url origin >/dev/null 2>&1 ||
    die "publish requires a configured origin remote"
  branch="$(git branch --show-current)"
  [ -n "$branch" ] || die "publish cannot run from a detached HEAD"
  git rev-parse --verify "refs/tags/${tag}" >/dev/null 2>&1 &&
    die "local tag ${tag} already exists"
  remote_tag="$(git ls-remote --tags origin "refs/tags/${tag}")" ||
    die "unable to query tags from origin"
  [ -z "$remote_tag" ] ||
    die "remote tag ${tag} already exists"

  existing_changes="$(changed_files)"
  if [ -n "$existing_changes" ]; then
    check_release_files <<EOF
${existing_changes}
EOF
    node .github/scripts/check-release-version.mjs "$version"
    prepared=true
  fi
fi

if [ "$mode" = "check" ]; then
  node .github/scripts/check-release-version.mjs "$version"
  git diff --check
  echo "release: ${tag} metadata and packages are valid"
  exit 0
fi

if [ "$prepared" = false ]; then
  node .github/scripts/set-release-version.mjs "$version"
fi
git diff --check

echo
echo "release: prepared ${tag}"
git status --short

if [ "$mode" != "publish" ]; then
  echo
  echo "Review the changes, commit or stash them, then publish with:"
  echo "  scripts/release.sh ${version}"
  exit 0
fi

current_changes="$(changed_files)"
check_release_files <<EOF
${current_changes}
EOF

git add \
  Cargo.toml \
  Cargo.lock \
  CHANGELOG.md \
  crates/*/Cargo.toml \
  crates/rules/builtin-plugins \
  plugins \
  npm/cleanr/package.json

if git diff --cached --quiet; then
  echo "release: versions already match ${tag}; tagging the current commit"
else
  git commit -m "release: ${tag}"
fi
git tag -a "$tag" -m "Release ${tag}"
if ! git push --atomic origin "HEAD:refs/heads/${branch}" "refs/tags/${tag}"; then
  git tag -d "$tag" >/dev/null
  die "push failed; removed local ${tag}, but kept the release commit for retry"
fi

echo "release: pushed ${tag}; GitHub Actions will validate, build, and publish the release"
