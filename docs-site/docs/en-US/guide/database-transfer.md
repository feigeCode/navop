# Database import, export, and SQL files

Transfer tools import delimited text, export tables or query results, create SQL dumps, and run existing SQL files. Confirm the source format, target table, encoding, transaction policy, and recovery plan before moving data.

## Import CSV and delimited text

Choose the target table and file, then configure encoding, field delimiter, text quote, escaping, and whether the first row is a header. Preview column mapping and compare source values with database types and nullability. Dates, decimals, booleans, `NULL`, and empty strings differ across systems, so validate a small sample first.

Transactions can make a supported import atomic. Stop on error favors consistency; continue on error can collect bad rows, but the error report must be reviewed. Split very large files and monitor server logs, disk, and timeouts.

## Treat clearing as destructive

Clear Table Before Import removes existing target data before new rows are written and is normally irreversible. Enable it only after confirming backups, environment, foreign-key dependencies, and restore steps. Do not use it for a harmless preview.

A failed import may still have committed rows depending on driver, batch, and transaction settings. Query target counts before retrying to avoid duplicates.

## Export data safely

Export a table or SQL result after selecting columns, rows, and format. Verify encoding, delimiters, headers, line endings, date formatting, and NULL representation. CSV is portable, XLSX is convenient for review, and SQL creates inspectable insert statements.

Spreadsheet software may transform long identifiers and dates. Exported files leave database access controls; store sensitive results in managed locations and remove them when no longer needed.

## Create SQL dumps

Export schema only, data only, or both. Schema-only output helps reproduce objects, while data-only output expects a compatible target schema. Combined dumps still require correct object order, foreign keys, triggers, sequences, character sets, and dialect handling.

Inspect the dump and restore it into an empty test database first. Cross-product migrations generally require type and syntax conversion rather than direct execution.

## Run SQL files

Select the connection, database/schema, file encoding, transaction mode, and error policy. Review the beginning and end of large scripts for DROP, TRUNCATE, permission changes, or environment-specific statements.

Use the execution log to locate failures. After cancellation or disconnection, verify what committed. Prove success with object counts, row checks, and business validation rather than relying only on the progress indicator.
