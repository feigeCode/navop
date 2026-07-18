# MongoDB Explorer

MongoDB Explorer manages connections, databases, collections, documents, indexes, validation rules, and Find, Aggregate, Count, or Explain operations. Deleting a database, collection, or document changes the server immediately.

## Connect securely

Use a connection string or host, port, and authentication fields. Configure authentication database, timeout, and TLS as required, or add an SSH tunnel. Replica-set and sharded connections need correct discovery parameters and network access to the nodes selected by the driver.

Test before saving. Validate production TLS certificates. A server-selection timeout can come from DNS, discovery, firewall, authentication source, or topology; simply increasing timeout may hide the cause.

## Browse databases and collections

Expand a database and collection, enter a JSON filter, and configure sort and paging. Filters must use valid JSON or supported extended JSON, and types must match BSON. Current-page search is not the same as a server query.

Creating an object may not become visible until data exists. Dropping a database or collection is irreversible, so verify connection, name, and backup.

## Edit documents

Create and edit documents in JSON form. Preserve types for `_id`, ObjectId, dates, arrays, and nested values; turning a BSON value into a plain string changes its meaning. Reload recent state before editing to avoid overwriting concurrent changes.

Before deletion, run the same filter with Find or Count and confirm the exact number of matches. Start bulk updates with a small range, then query critical documents afterward.

## Run Find, Aggregate, and Count

Find handles filtering, projection, sorting, and paging. Count verifies scale. Aggregate builds multi-stage transformations, grouping, joins, and statistics. Validate each new pipeline stage and watch the shape and volume of its output.

Avoid unbounded scans or unfamiliar large aggregations during production peaks. Add limits and inspect server load.

## Manage indexes and validation

Review, create, and remove indexes. Eliminate duplicates before a unique index and confirm query dependencies before deleting an index. Large builds consume CPU, memory, and disk.

Validation rules constrain future writes and may reject existing application behavior. Test JSON Schema or expressions on sample data. Explain shows index use and scanned versus returned documents, helping you tune queries based on evidence.
