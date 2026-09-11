#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Every source file carries its licence. GPL asks for the notice on the file,
# not only on the repository, and a file that loses it is invisible in review.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
missing=0

while IFS= read -r file; do
    if ! head -3 "${repo}/${file}" | grep -q 'SPDX-License-Identifier: GPL-3.0-or-later'; then
        echo "no SPDX header: ${file}" >&2
        missing=$((missing + 1))
    fi
done < <(cd "${repo}" && find crates tools docker \
    -path 'crates/*/assets' -prune -o \
    \( -name '*.rs' -o -name '*.cc' -o -name '*.h' -o -name '*.py' -o -name '*.sh' \) -print)

if [[ ${missing} -gt 0 ]]; then
    echo "${missing} file(s) without a licence header" >&2
    exit 1
fi
echo "licence headers: all present"
