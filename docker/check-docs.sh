#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The wiki is mirrored to GitHub, where a link to a page that does not exist
# renders as an invitation to create it rather than as an error. So the broken
# link is invisible on the site and has to be caught here.
#
# Checks two things: every wiki-internal link points at a page that exists, and
# every page is reachable from the sidebar.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
wiki="${repo}/wiki"
problems=0

if [[ ! -d ${wiki} ]]; then
    echo "no wiki/ folder, nothing to check"
    exit 0
fi

pages=$(cd "${wiki}" && ls ./*.md | sed 's#^\./##; s#\.md$##')

# A wiki link is [text](Page-Name): no scheme, no slash, no extension. Anything
# with a scheme is external and not ours to verify; anything with a dot is a
# file reference into the repository.
while IFS= read -r line; do
    file=${line%%:*}
    target=${line#*:}
    if ! grep -qxF "${target}" <<<"${pages}"; then
        echo "broken wiki link: ${file} -> ${target}" >&2
        problems=$((problems + 1))
    fi
done < <(
    cd "${wiki}" &&
    grep -oHE '\]\([A-Za-z0-9_-]+\)' ./*.md |
        sed 's#^\./##; s#:\](#:#; s#)$##'
)

sidebar="${wiki}/_Sidebar.md"
if [[ -f ${sidebar} ]]; then
    while IFS= read -r page; do
        case ${page} in
            _Sidebar | _Footer | _Header) continue ;;
        esac
        if ! grep -qE "\\(${page}\\)" "${sidebar}"; then
            echo "not linked from the sidebar: ${page}" >&2
            problems=$((problems + 1))
        fi
    done <<<"${pages}"
else
    echo "wiki/_Sidebar.md is missing" >&2
    problems=$((problems + 1))
fi

if [[ ${problems} -gt 0 ]]; then
    echo "${problems} problem(s) in the wiki" >&2
    exit 1
fi
echo "wiki: $(wc -l <<<"${pages}" | tr -d " ") pages, links and sidebar consistent"
