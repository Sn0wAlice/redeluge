#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Runs as root: aligns the redeluge user with PUID/PGID, fixes ownership where it
# is actually wrong, then drops privileges and hands over to run.sh.
set -euo pipefail

CONFIG_DIR=${DELUGE_CONFIG_DIR:-/config}
DOWNLOAD_DIR=${DELUGE_DOWNLOAD_DIR:-/downloads}
PUID=${PUID:-1000}
PGID=${PGID:-1000}

here=$(dirname "$(readlink -f "$0")")

if [[ $(id -u) -ne 0 ]]; then
    # Started with `user:` in compose, or `docker run --user`. Nothing to set up.
    exec "${here}/run.sh" "$@"
fi

if [[ $(id -g redeluge) -ne ${PGID} ]]; then
    groupmod -o -g "${PGID}" redeluge
fi
if [[ $(id -u redeluge) -ne ${PUID} ]]; then
    usermod -o -u "${PUID}" -g "${PGID}" redeluge
fi

for dir in "${CONFIG_DIR}" "${DOWNLOAD_DIR}"; do
    mkdir -p "${dir}"
    owner=$(stat -c '%u:%g' "${dir}")
    if [[ ${owner} != "${PUID}:${PGID}" ]]; then
        # Recursive only when the top-level owner is wrong, so a large download
        # tree is not walked on every restart.
        echo "[entrypoint] taking ownership of ${dir} (was ${owner})"
        chown -R "${PUID}:${PGID}" "${dir}"
    fi
done

echo "[entrypoint] starting as uid=${PUID} gid=${PGID}"

# gosu on Debian, su-exec on Alpine. Both drop privileges without forking.
if command -v gosu >/dev/null 2>&1; then
    exec gosu redeluge "${here}/run.sh" "$@"
else
    exec su-exec redeluge "${here}/run.sh" "$@"
fi
