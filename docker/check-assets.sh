#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Every Web UI asset on disk must be one git will actually commit.
#
# This exists because of a bug that no amount of building or testing on a
# developer's machine could find. The whole `assets/icons` directory was
# ignored, so a clone of the repository had an interface with no icons in it,
# while every local build was fine: the tests run against the working tree,
# which has the files, not against what the repository holds.
#
# The cause was a `Icon?` line in a global ignore file, the usual macOS rule for
# the resource file the Finder writes. It has no slash, so git applies it at
# every level, and on a case-insensitive filesystem git compares ignore patterns
# case-insensitively, so it matched the directory `icons`. Nothing warns about
# this. A file that is ignored is simply not there, quietly, until someone
# checks the repository out somewhere else.
#
# Untracked files are fine: that is what a new asset looks like before it is
# added. Ignored ones are not, because nothing will ever add them.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "${repo}"

if ! git rev-parse --git-dir > /dev/null 2>&1; then
    echo "[assets] not a git checkout, skipping"
    exit 0
fi

assets="crates/redeluge-web/assets"
if [[ ! -d ${assets} ]]; then
    echo "[assets] no ${assets} directory"
    exit 1
fi

ignored=$(git status --porcelain --ignored=matching -- "${assets}" | sed -n 's/^!! //p')

if [[ -n ${ignored} ]]; then
    echo "::error::These Web UI assets are ignored by git, so a clone does not have them:" >&2
    printf '  %s\n' ${ignored} >&2
    cat >&2 <<'MESSAGE'

Check for a pattern that matches them, in this repository's .gitignore and in
the global one:

    git check-ignore -v <path>
    git config --get core.excludesFile

A pattern with no slash applies at every level, and matching is case
insensitive wherever core.ignorecase is on, which is the default on macOS.
MESSAGE
    exit 1
fi

# Untracked is a warning, not a failure: that is what a new asset looks like
# before it is added, and the gate gets run in the middle of writing one. It is
# still worth saying, because `git commit -a` does not pick these up either, so
# an asset can reach a local image and never reach the repository.
untracked=$(git ls-files --others --exclude-standard -- "${assets}")
if [[ -n ${untracked} ]]; then
    count=$(printf '%s\n' "${untracked}" | wc -l | tr -d ' ')
    echo "[assets] ${count} asset(s) are not added to git yet:"
    printf '%s\n' "${untracked}" | head -5 | sed 's/^/  /'
    if [[ ${count} -gt 5 ]]; then
        echo "  ... and $((count - 5)) more"
    fi
fi

echo "[assets] every asset is one git will commit"
