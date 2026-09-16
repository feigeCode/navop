Support for reading and writing  ClickHouse's [Native columnar format].

# Type Mapping

This module implements conversions for many Rust types to/from their ClickHouse equivalents:

| Rust type                         | Clickhouse type           | Notes                                                    |
|-----------------------------------|---------------------------|----------------------------------------------------------|
| `bool`                            | `Bool`                    |                                                          |
| `u{N}` (`u8`, etc.)               | `UInt{N}` (`UInt8`, etc.) | `[u8]` and `Vec<u8>` map to `Array(UInt8)`, not `String` |
| `i{N}` (`i8`, etc.)               | `Int{N}` (`Int8`, etc.)   |                                                          |
| `f32`, `f64`                      | `Float32`, `Float64`      |                                                          |
| `String`                          | `String`, `FixedString`   | Decoding errors if the string is not valid UTF-8         |
| `&str`                            | `String`, `FixedString`   | See `String` above, zero-copy decode also supported      |
| `Option<T>`                       | `Nullable(T)`             | Some data types may not be nullable; see [`Nullable`].   |
| `[T]`, `Vec<T>`                   | `Array(T)`                |                                                          |
| `(T...)`                          | `Tuple(T...)`             | Tuples up to 16 elements supported                       | 
| `HashMap<K, V>`, `BTreeMap<K, V>` | `Map(K, V)`               | See [`Map`] for restrictions on key types.               |
| `Ipv4Addr`                        | `IPv4`                    |                                                          |
| `Ipv6Addr`                        | `IPv6`                    |                                                          |
| `IpAddr`                          | `IPv4`, `IPv6`            | Supported for decode only.                               |

`LowCardinality` is supported for any data types where it is valid. It is handled internally.

For details on the ClickHouse equivalents, see <https://clickhouse.com/docs/reference/data-types>.

## Optional Features
Some third-party types are also supported as optional features:

| Rust type      | Clickhouse type | Required Feature(s) | Notes |
|----------------|-----------------|---------------------|-------|
| [`uuid::Uuid`] | `UUID`          | `uuid`              |       |

[Native columnar format]: https://clickhouse.com/docs/reference/formats/Native
[`Nullable`]: https://clickhouse.com/docs/reference/data-types/nullable
[`Map`]: https://clickhouse.com/docs/reference/data-types/map
