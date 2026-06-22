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

# Project-backed Quarto metadata must merge with dual.toml without replacing
# the project's environment manifest or shared lockfile.
project_manifest_hash="$(python3 - <<'PY'
from hashlib import sha256
from pathlib import Path
print(sha256(Path(".dual/workspace/pyproject.toml").read_bytes()).hexdigest())
PY
)"
project_lock_hash="$(python3 - <<'PY'
from hashlib import sha256
from pathlib import Path
print(sha256(Path("dual.lock").read_bytes()).hexdigest())
PY
)"
"$DUAL_BIN" init --script report.qmd --python 3.12
"$DUAL_BIN" add --script report.qmd --python matplotlib
"$DUAL_BIN" --trust-project run report.qmd
test -s report.html
grep -q "Hello from dual" report.html
test "$project_manifest_hash" = "$(python3 - <<'PY'
from hashlib import sha256
from pathlib import Path
print(sha256(Path(".dual/workspace/pyproject.toml").read_bytes()).hexdigest())
PY
)"
test "$project_lock_hash" = "$(python3 - <<'PY'
from hashlib import sha256
from pathlib import Path
print(sha256(Path("dual.lock").read_bytes()).hexdigest())
PY
)"
"$DUAL_BIN" --trust-project doctor >/dev/null

# A synchronized script must run without installation, including through the
# documented Unix shebang form.
cat > executable.py <<'PY'
#!/usr/bin/env -S dual run
print("dual shebang ok")
PY
"$DUAL_BIN" init --script executable.py --python 3.12
"$DUAL_BIN" --trust-project sync --script executable.py
"$DUAL_BIN" run executable.py --no-install | grep -q "dual shebang ok"
chmod +x executable.py
PATH="$(dirname "$DUAL_BIN"):$PATH" DUAL_TRUST_PROJECT=1 ./executable.py |
    grep -q "dual shebang ok"

# Standalone documents must work without an ancestor dual.toml.
project_root="$root"
standalone_quarto="$(mktemp -d)"
cd "$standalone_quarto"
"$DUAL_BIN" init --script standalone.qmd --python 3.12
"$DUAL_BIN" --trust-project run standalone.qmd
test -s standalone.html
grep -q "Hello from dual" standalone.html

standalone_rmarkdown="$(mktemp -d)"
cd "$standalone_rmarkdown"
"$DUAL_BIN" init --script standalone.Rmd --r 4.5
"$DUAL_BIN" --trust-project run standalone.Rmd
test -s standalone.html
grep -q "Hello from dual" standalone.html
cd "$project_root"

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
