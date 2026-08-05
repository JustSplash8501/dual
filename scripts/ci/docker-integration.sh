#!/usr/bin/env bash
set -euo pipefail

if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required for the export integration test." >&2
    exit 1
fi

root="$(mktemp -d)"
image="dual-export-smoke:${GITHUB_RUN_ID:-local}"
cleanup() {
    docker image rm --force "$image" >/dev/null 2>&1 || true
    rm -rf "$root"
}
trap cleanup EXIT

cd "$root"
"$DUAL_BIN" init docker-smoke --python 3.12 --r 4.5
"$DUAL_BIN" add py 'six==1.17.0'
"$DUAL_BIN" add r jsonlite curl
"$DUAL_BIN" export --dockerfile

docker build \
    --pull \
    --build-arg 'DUAL_SYSTEM_PACKAGES=libcurl4-openssl-dev' \
    --tag "$image" \
    .
docker run --rm "$image" python -c 'import six; assert six.__version__ == "1.17.0"'
docker run --rm "$image" Rscript -e \
    'stopifnot(requireNamespace("jsonlite", quietly=TRUE), requireNamespace("curl", quietly=TRUE))'
