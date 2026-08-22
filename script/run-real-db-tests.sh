#!/usr/bin/env bash

set -u

failed=0

run_test() {
    local name=$1
    shift

    printf '\n==> %s\n' "$name"
    if "$@"; then
        printf '==> PASSED %s\n' "$name"
    else
        printf '==> FAILED %s\n' "$name" >&2
        failed=1
    fi
}

skip_test() {
    local name=$1
    local reason=$2
    printf '\n==> SKIPPED %s: %s\n' "$name" "$reason"
}

run_test \
    "SQLite real database tests" \
    cargo test -p db --test real_sqlite -- --nocapture

run_test \
    "Data compare integration test" \
    cargo test -p db --test real_compare -- --nocapture

run_test \
    "DuckDB real database tests" \
    cargo test -p db --features builtin-duckdb --test real_duckdb -- --nocapture

if [[ ${ONETCLI_TEST_MYSQL_PASSWORD+x} == x ]]; then
    run_test \
        "MySQL real database tests" \
        cargo test -p db --test real_mysql -- --nocapture
else
    skip_test \
        "MySQL real database tests" \
        "ONETCLI_TEST_MYSQL_PASSWORD is not set"
fi

if [[ ${ONETCLI_TEST_POSTGRES_PASSWORD+x} == x ]]; then
    run_test \
        "PostgreSQL real database tests" \
        cargo test -p db --test real_postgres -- --nocapture
else
    skip_test \
        "PostgreSQL real database tests" \
        "ONETCLI_TEST_POSTGRES_PASSWORD is not set"
fi

if (( failed )); then
    printf '\nReal database test suite FAILED.\n' >&2
    exit 1
fi

printf '\nReal database test suite PASSED.\n'
