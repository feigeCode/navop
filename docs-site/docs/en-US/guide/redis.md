# Redis workspace

The Redis workspace supports standalone, Sentinel, and Cluster deployments. It includes key browsing, common type editors, CLI, server information, and live diagnostics. Deletes and namespace cleanup take effect immediately and are not recoverable.

## Create a Redis connection

Choose standalone, Sentinel, or Cluster and enter nodes, port, user, password, and database where applicable. Enable TLS when required and use an SSH tunnel for restricted networks. Sentinel needs the correct service name. Cluster clients must reach every node address returned by redirects.

Test authentication and networking before saving. If the native Redis driver is not installed, opening the connection prompts you to download it; after a successful installation Navop continues opening the same connection without a restart. Prepare a compatible driver extension in advance for offline environments. Use a restricted production account and confirm whether INFO, Slow Log, scanning, and Pub/Sub commands are permitted.

## Search and inspect keys

Use local filtering for loaded entries or server scanning with Redis glob syntax: `*`, `?`, and `[]`. Broad patterns can return huge sets, so narrow the namespace. On an active instance the list can change during scanning.

Inspect type, TTL, memory, and length. Changing TTL changes lifecycle; renaming requires checking both destination collisions and application references.

## Edit Redis data types

Edit String values; add or remove List elements at the head or tail; manage Set members; update Sorted Set members and scores; and edit Hash fields. Verify type and position before saving, especially when other clients may write concurrently.

Non-UTF-8 values may appear as Raw, Hex, or Binary. Preserve their original bytes rather than converting them through a text editor. Large values require additional memory and network capacity.

## Use CLI and diagnostics

The CLI exposes raw commands and responses. Review DEL, FLUSH, CONFIG, EVAL, and pasted scripts before execution. Server Info covers version, clients, persistence, and replication; Memory explains allocation; Slow Log shows expensive commands; command statistics show frequency and time; live charts show trends.

Interpret metrics with sampling period and workload context. One spike is not proof of a persistent fault.

## Work with Pub/Sub and deletion

Subscribe to channels to observe new messages, or publish a reviewed test message when allowed. Pub/Sub has no history replay for offline subscribers. Confirm production message format and consumer effects before publishing.

Single-key delete, pattern delete, and namespace cleanup can break applications immediately. Back up or rehearse in a test instance, keep patterns narrow, and verify both remaining keys and application health afterward.
