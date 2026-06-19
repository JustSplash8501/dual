#!/usr/bin/env bash
set -euo pipefail

root="$(mktemp -d)"
cd "$root"

"$DUAL_BIN" init --name ci-integration
"$DUAL_BIN" add r jsonlite reticulate
"$DUAL_BIN" add py 'six==1.17.0'

python3 - <<'PY'
from pathlib import Path
path = Path("dual.toml")
path.write_text(path.read_text().replace(
    "[tasks]\n",
    """[tasks]
rcheck = "Rscript -e \\"cat(jsonlite::toJSON(list(ok=TRUE)))\\""
pycheck = "python -c \\"import six; print(six.__version__)\\""
""",
))
PY

"$DUAL_BIN" --trust-project up
"$DUAL_BIN" doctor
"$DUAL_BIN" run rcheck
"$DUAL_BIN" run pycheck
test -s dual.lock

mkdir -p scripts/nested
cd scripts/nested
"$DUAL_BIN" --trust-project doctor
cd ../..

"$DUAL_BIN" clean --yes
test -s dual.lock
test ! -e .dual
"$DUAL_BIN" up
"$DUAL_BIN" doctor | grep -q "reticulate uses the project Python"
"$DUAL_BIN" clean --yes
