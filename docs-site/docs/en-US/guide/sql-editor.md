# SQL editor and transactions

The SQL editor writes, saves, explains, and executes statements. Execution scope, active database, and transaction mode determine the impact. Recheck the connection, schema, and selected text before every production write.

## Create and save queries

Open a query from a database workspace, choose the active database or schema, and enter SQL. Queries can be saved, renamed, and reopened. Auto-save preserves editor content according to Settings, but it neither executes SQL nor replaces version control and backups.

Formatting and compacting change presentation, not correctness. Preserve an original copy of complex migrations and validate them in a test database.

## Choose the execution scope

Run selected SQL, the statement under the cursor, or the entire editor. Inspect selection boundaries before running. With no selection, place the cursor inside the intended statement. Run All is suitable only for a fully reviewed script, not a scratch file containing examples and temporary DELETE statements.

The result area shows rows, affected counts, or errors. The maximum-row setting prevents huge previews from exhausting memory. Use export for complete datasets rather than raising the limit without bounds.

## Use EXPLAIN responsibly

EXPLAIN shows plans for supported queries, including scans, indexes, join order, and estimated cost. Syntax and fields vary by database. Variants that include ANALYZE may actually execute the query, so understand side effects and load first.

Keep the original SQL and a baseline. A cost number alone is not enough reason to remove an index or change production logic; data distribution, cache, and concurrency matter.

## Control manual transactions

Automatic mode follows the driver's commit behavior. Manual mode groups multiple writes until you explicitly commit or roll back. Uncommitted work can hold locks and block other sessions.

Before changing database or schema, or closing the editor, Navop asks you to finish a manual transaction. Commit makes the changes durable; rollback discards uncommitted work. After a disconnect, query the server to verify state rather than assuming the transaction outcome.

## Recover from errors safely

Reduce syntax, permission, constraint, or timeout errors to one statement and read the server response. Do not repeatedly execute while transaction state is unknown. For UPDATE or DELETE, first run the same WHERE clause as SELECT and verify the target rows.

Remove passwords, tokens, personal data, and production literals before sharing saved queries or issue reports.
