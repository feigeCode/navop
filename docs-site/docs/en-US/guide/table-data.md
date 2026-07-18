# Browse and edit table data

Table data views provide paging, filtering, copying, and row editing. Committing additions, edits, or deletions changes the real database. Confirm keys, filters, and environment before submitting.

## Browse and filter

Open Data from a table or view and use paging to control load. Search on the current page filters only rows already loaded; use SQL for server-wide filtering or aggregation. Refresh to read the latest server state.

Keep page size and maximum rows reasonable for large tables. Whether a view is editable depends on the database and view definition; use the base table or explicit SQL when it is read-only.

## Edit values correctly

Edit cells directly and use the large-text editor for long strings, JSON, or multiline content. `NULL` and an empty string are different values, so choose the explicit NULL action rather than inferring it from a blank field.

New rows can be entered from scratch or cloned. Change primary and unique fields on a clone; generate a UUID only when the column and business rules expect one. Binary, generated, and driver-specific types may have editing limits. Refresh after commit to verify the stored representation.

## Add, clone, and delete rows

Local row changes remain pending until submitted. Before deleting, confirm the count and primary keys, especially while sorted, paged, or filtered. Tables without a stable key may not be safe for single-row editing.

If the selection is wrong, discard pending changes before submission. Production deletions should use a controlled query, transaction, and backup rather than relying on manual recovery.

## Preview SQL and submit

Open SQL Preview to inspect generated INSERT, UPDATE, and DELETE statements, including table, columns, values, and row predicates. The preview confirms the generated operation but cannot validate business intent. Submit only after review.

On partial failure, inspect the exact statement and transaction behavior before retrying. Refresh and query key rows after a successful commit.

## Copy and export

Copy cells, rows, or results in supported formats. Export to `.xlsx`, `.csv`, or `.sql`, and verify column range, encoding, date formatting, and NULL representation. Spreadsheet programs may silently transform long numbers or dates.

Exported files leave database access controls and may contain sensitive data. Store them in controlled locations and delete them according to policy.
