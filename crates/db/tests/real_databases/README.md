# Real database integration tests

These tests exercise the built-in database plugins against real database engines rather than mocks. They cover all-type fixtures, direct SQL scripts, errors and transactions, metadata, paginated table data, generated row CRUD, table designer CRUD, import/export, and compare/sync primitives.

## Files

- `real_sqlite.rs`: SQLite integration flow.
- `real_duckdb.rs`: DuckDB integration flow.
- `real_mysql.rs`: MySQL integration flow.
- `real_postgres.rs`: PostgreSQL integration flow.
- `real_compare.rs`: SQLite source/target schema compare, data compare, generated sync SQL, selected statement execution, and destructive statement safety.

## Runner

SQLite, the SQLite compare test, and DuckDB run by default:

```bash
./script/run-real-db-tests.sh
```

The script does not start a database server and never provides default credentials. MySQL and PostgreSQL tests skip automatically (with a note on stderr) when their password environment variable is absent, so `cargo test --all` stays green on machines and CI runners without a local database server. When the variable is present the tests run for real. An empty PostgreSQL password is valid and is handled correctly.

## Environment variables

### MySQL

```bash
export ONETCLI_TEST_MYSQL_HOST=127.0.0.1
export ONETCLI_TEST_MYSQL_PORT=3306
export ONETCLI_TEST_MYSQL_USER=root
export ONETCLI_TEST_MYSQL_PASSWORD='your-password'
```

All variables except `ONETCLI_TEST_MYSQL_PASSWORD` have defaults. Tests create and drop isolated `navop_real_mysql_<pid>_<flow>` databases.

### PostgreSQL

```bash
export ONETCLI_TEST_POSTGRES_HOST=127.0.0.1
export ONETCLI_TEST_POSTGRES_PORT=5432
export ONETCLI_TEST_POSTGRES_USER=postgres
export ONETCLI_TEST_POSTGRES_PASSWORD='' # empty is intentionally valid
export ONETCLI_TEST_POSTGRES_DATABASE=postgres
```

All variables except `ONETCLI_TEST_POSTGRES_PASSWORD` have defaults. Tests create and drop isolated `navop_real_pg_<pid>_<flow>` schemas.

## Direct commands

```bash
cargo test -p db --test real_sqlite -- --nocapture
cargo test -p db --test real_compare -- --nocapture
cargo test -p db --features builtin-duckdb --test real_duckdb -- --nocapture
ONETCLI_TEST_MYSQL_PASSWORD='your-password' \
  cargo test -p db --test real_mysql -- --nocapture
ONETCLI_TEST_POSTGRES_PASSWORD='' \
  cargo test -p db --test real_postgres -- --nocapture
```

Run tests from the repository root. Keep any leaked test databases out of shared servers by dropping them if a test is interrupted.
