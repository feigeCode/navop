# Schema, design, and comparison tools

Schema tools cover object browsing, table design, DDL, ER diagrams, Schema Compare, and Data Compare. They may generate DROP, ALTER, INSERT, UPDATE, or DELETE statements; every generated plan requires human review.

## Browse database objects

Expand databases and schemas to inspect tables, views, and other supported objects. A table can expose columns, types, defaults, nullability, primary keys, indexes, and DDL. Right-click an object row to see actions available for that object type and installed extensions. Visibility depends on permissions, context, and schema filters.

Refresh before a change so an old tab does not hide modifications made by another user or tool.

## Use the table designer

Create or modify columns and indexes, plus supported engine, character-set, and collation settings. Before narrowing a type or changing nullability, confirm that existing data can convert. Check for duplicates before creating a unique index.

Review the SQL/DDL preview. Navop warns about destructive statements such as DROP, but automated detection cannot understand every business impact. Validate migrations in a test database and prepare rollback or restore procedures.

## Build ER diagrams

ER diagrams visualize tables and metadata-defined relationships. Logical relationships without foreign keys may not appear, so combine the diagram with application knowledge. Select a focused group of objects for large schemas.

An ER diagram is a structural aid, not a live data-flow, permission, or auditing model.

## Run Schema Compare

Choose source and target connections, confirm the direction, and review added, changed, or missing objects. Select only intended objects and inspect dependency order and unsupported features. The generated synchronization SQL can be edited before execution.

DROP, narrowing conversions, index rebuilds, and large ALTER operations require backups and a maintenance plan. Cross-product comparisons need manual type and syntax mapping.

## Run Data Compare

Data Compare matches rows by a usable key and reports required INSERT, UPDATE, and DELETE operations. Without a stable primary or unique key, results may be unreliable. Verify key selection, filters, and volume before generating a plan.

Review the editable SQL, transaction setting, and stop-or-continue error policy. Use execution logs to diagnose failures, then query the target to prove the result. Test large synchronizations on a sample or copy first.
