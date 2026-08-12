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
renvcheck = "Rscript -e \\"cat(Sys.getenv('PROJECT_SHARED_ENV'))\\""
pyenvcheck = "python -c \\"import os; print(os.environ['PROJECT_SHARED_ENV'])\\""
""",
))
PY
printf '%s\n' 'PROJECT_SHARED_ENV=dotenv-ok' > .env

"$DUAL_BIN" --trust-project up
"$DUAL_BIN" --json cache info | grep -q '"exists": true'
test -f "$DUAL_CACHE_DIR/.dual-cache"
test -d "$DUAL_CACHE_DIR/v1/environment"
"$DUAL_BIN" doctor
"$DUAL_BIN" run rcheck
"$DUAL_BIN" run pycheck
"$DUAL_BIN" run renvcheck | grep -q "dotenv-ok"
"$DUAL_BIN" run pyenvcheck | grep -q "dotenv-ok"
test -s dual.lock

# Project initialization must produce complete, runnable single-language
# environments without installing or reporting the omitted runtime.
mixed_root="$root"
python_only="$(mktemp -d)"
cd "$python_only"
"$DUAL_BIN" init python-only --python 3.12
"$DUAL_BIN" add py 'six==1.17.0'
python3 - <<'PY'
from pathlib import Path
path = Path("dual.toml")
path.write_text(path.read_text().replace(
    "[tasks]\n",
    "[tasks]\npycheck = \"python -c \\\"import six; print(six.__version__)\\\"\"\n",
))
PY
printf '%s\n' 'cat("R enabled")' > scripts/enabled.R
python_up="$("$DUAL_BIN" --trust-project up)"
printf '%s\n' "$python_up" | grep -q "Python 3.12 requested"
if printf '%s\n' "$python_up" | grep -q "R packages configured"; then
    echo "Python-only preparation reported R packages" >&2
    exit 1
fi
"$DUAL_BIN" run pycheck | grep -q "1.17.0"
"$DUAL_BIN" doctor | grep -q "R is not required by this environment"
test -s dual.lock
grep -q 'python = ' .dual/workspace/pyproject.toml
if grep -q 'r-base' .dual/workspace/pyproject.toml; then
    echo "Python-only manifest contains R" >&2
    exit 1
fi

# A single-language project must move to both runtimes and back without being
# recreated. Existing packages and tasks in the original runtime stay usable.
"$DUAL_BIN" enable r --version 4.5
"$DUAL_BIN" up --refresh
"$DUAL_BIN" run scripts/enabled.R | grep -q "R enabled"
"$DUAL_BIN" run pycheck | grep -q "1.17.0"
grep -q 'r-base = ' .dual/workspace/pyproject.toml
grep -q '^python = ' .dual/workspace/pyproject.toml
"$DUAL_BIN" disable r
"$DUAL_BIN" up --refresh
grep -q '^python = ' .dual/workspace/pyproject.toml
if grep -q 'r-base' .dual/workspace/pyproject.toml; then
    echo "Python-only manifest retained disabled R" >&2
    exit 1
fi

r_only="$(mktemp -d)"
cd "$r_only"
"$DUAL_BIN" init r-only --r 4.5
"$DUAL_BIN" add r jsonlite
python3 - <<'PY'
from pathlib import Path
path = Path("dual.toml")
path.write_text(path.read_text().replace(
    "[tasks]\n",
    "[tasks]\nrcheck = \"Rscript -e \\\"cat(jsonlite::toJSON(list(ok=TRUE)))\\\"\"\n",
))
PY
printf '%s\n' 'print("Python enabled")' > scripts/enabled.py
r_up="$("$DUAL_BIN" --trust-project up)"
printf '%s\n' "$r_up" | grep -q "R 4.5 requested"
if printf '%s\n' "$r_up" | grep -q "Python packages configured"; then
    echo "R-only preparation reported Python packages" >&2
    exit 1
fi
"$DUAL_BIN" run rcheck | grep -q '"ok":\[true\]'
"$DUAL_BIN" doctor | grep -q "Python is not required by this environment"
test -s dual.lock
grep -q 'r-base = ' .dual/workspace/pyproject.toml
if grep -q '^python = ' .dual/workspace/pyproject.toml; then
    echo "R-only manifest contains Python" >&2
    exit 1
fi

"$DUAL_BIN" enable py --version 3.12
"$DUAL_BIN" up --refresh
"$DUAL_BIN" run scripts/enabled.py | grep -q "Python enabled"
"$DUAL_BIN" run rcheck | grep -q '"ok":\[true\]'
grep -q 'r-base = ' .dual/workspace/pyproject.toml
grep -q '^python = ' .dual/workspace/pyproject.toml
"$DUAL_BIN" disable py
"$DUAL_BIN" up --refresh
grep -q 'r-base = ' .dual/workspace/pyproject.toml
if grep -q '^python = ' .dual/workspace/pyproject.toml; then
    echo "R-only manifest retained disabled Python" >&2
    exit 1
fi
cd "$mixed_root"

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
