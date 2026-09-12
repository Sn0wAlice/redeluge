#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Supervises the two binaries. If either exits, so does this, and the container
# restart policy brings the pair back together.
set -euo pipefail

umask "${UMASK:-022}"

CONFIG_DIR=${DELUGE_CONFIG_DIR:-/config}
DAEMON_PORT=${DELUGE_DAEMON_PORT:-58846}

# Both binaries take their log level from RUST_LOG, which is the only one they
# read and the only one that can name a module. `DELUGE_LOGLEVEL` is what the
# Deluge container images use, and someone moving a compose file across should
# not have to find that out by getting no logs, so it is honoured when RUST_LOG
# says nothing. The image sets neither, so an explicit RUST_LOG always wins.
# Deluge's levels and Rust's are not quite the same set.
if [[ -z ${RUST_LOG:-} ]]; then
    case "${DELUGE_LOGLEVEL:-info}" in
        none | None | NONE) RUST_LOG=off ;;
        critical | CRITICAL | error | ERROR) RUST_LOG=error ;;
        warning | WARNING | warn | WARN) RUST_LOG=warn ;;
        info | INFO) RUST_LOG=info ;;
        debug | DEBUG | trace | TRACE) RUST_LOG=debug ;;
        *)
            echo "[run] DELUGE_LOGLEVEL=${DELUGE_LOGLEVEL} is not a level I know; using info" >&2
            RUST_LOG=info
            ;;
    esac
    export RUST_LOG
fi

# A configuration from the Python daemon holds its torrent list as a pickle,
# which this daemon cannot read. Converting it needs Python, and this image
# deliberately has none, so the conversion is a one-time thing the operator runs
# on the host. Starting anyway would present an empty torrent list, which looks
# exactly like having lost everything.
if [[ -f "${CONFIG_DIR}/state/torrents.state" && ! -f "${CONFIG_DIR}/state/torrents.json" ]]; then
    cat >&2 <<'MESSAGE'
[run] This configuration comes from the Python daemon and has to be converted
      once before redeluged can read it. Nothing is deleted by the conversion.

      From the host, with the config directory this container mounts:

          python3 tools/migrate_state.py <config dir>

      The script is standard library only and is also in this image at
      /usr/local/share/redeluge/migrate_state.py if you need a copy.

      Refusing to start: an empty torrent list would look like data loss.
MESSAGE
    exit 1
fi

redeluged &
daemon_pid=$!

# The Web UI shows a connection error if it starts before the daemon accepts
# RPC, and nothing retries that on its own.
echo "[run] waiting for the daemon on port ${DAEMON_PORT}"
for _ in $(seq 1 60); do
    if (exec 3<>"/dev/tcp/127.0.0.1/${DAEMON_PORT}") 2>/dev/null; then
        exec 3>&- 3<&-
        break
    fi
    if ! kill -0 "${daemon_pid}" 2>/dev/null; then
        echo "[run] the daemon exited during startup" >&2
        exit 1
    fi
    sleep 1
done

redeluge-web &
web_pid=$!

echo "[run] Web UI on ${DELUGE_WEB_INTERFACE:-0.0.0.0}:${DELUGE_WEB_PORT:-8112}"

shutdown() {
    trap - TERM INT
    echo "[run] stopping"
    # The daemon writes resume data on the way out, so it gets time to do it.
    kill -TERM "${web_pid}" "${daemon_pid}" 2>/dev/null || true
    wait "${web_pid}" "${daemon_pid}" 2>/dev/null || true
    exit 0
}
trap shutdown TERM INT

status=0
wait -n "${daemon_pid}" "${web_pid}" || status=$?
echo "[run] a service exited with status ${status}, shutting the container down" >&2
kill -TERM "${web_pid}" "${daemon_pid}" 2>/dev/null || true
wait 2>/dev/null || true
exit "${status}"
