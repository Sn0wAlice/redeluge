#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Runs the Rust build, lints and tests in a container, so the result does not
# depend on what happens to be installed on the machine.
#
#   docker/rust.sh            # fmt, clippy, build, test
#   docker/rust.sh shell      # a prompt inside the same environment
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
image=redeluge-rust:dev

# Before anything is built: on the host, because it asks git what the
# repository holds and the container has only a mounted working tree.
"${repo}/docker/check-assets.sh"

docker build -f "${repo}/docker/Dockerfile.rust" --target base -t "${image}" "${repo}"

if [[ ${1:-} == shell ]]; then
    exec docker run --rm -it -v "${repo}:/src" -w /src "${image}" bash
fi

exec docker run --rm -v "${repo}:/src" -w /src \
    -e CARGO_TERM_COLOR=always \
    -e CARGO_TARGET_DIR=/src/target-docker \
    "${image}" \
    bash -eux -c '
        docker/check-headers.sh
        docker/check-docs.sh
        ruff check tools
        ruff format --check tools
        python3 tools/draw_icons.py --check
        cargo fmt --all -- --check
        cargo clippy --workspace --all-targets -- -D warnings
        cargo test --workspace
        python3 tools/extract_contract.py --check
        python3 tools/gen_openapi.py --check
        python3 tools/migrate_state.py --self-test
    '
