use crate::get_client;
use clickhouse::native::{Column, decode::Decode};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;

#[tokio::test]
async fn mixed_types_empty() {
    mixed_types(0).await
}

#[tokio::test]
async fn mixed_types_10() {
    mixed_types(10).await
}

#[tokio::test]
async fn mixed_types_100() {
    mixed_types(100).await
}

#[tokio::test]
async fn mixed_types_1000() {
    mixed_types(1000).await
}

// NOTE: requires >50GiB RAM on the server, likely because all the strings have to live in memory
#[ignore]
#[tokio::test]
async fn mixed_types_10000() {
    mixed_types(10000).await
}

async fn mixed_types(num_rows: u64) {
    let client = get_client();

    let mut cursor = client
        .query(
            "SELECT
                number,
                leftPad(toString(number), number, '.') AS text,
                number % 2 == 0 ?? number : NULL AS nullable_number,
                number % 2 != 0 ?? text : NULL AS nullable_text,
                tuple(number, text) AS number_text_tuple,
                tuple(nullable_number, nullable_text) AS nullable_tuple,
                arrayMap(x -> toUInt64(x), arrayEnumerate(arrayWithConstant(number, toUInt64(0)))) AS number_array,
                arrayMap(x -> rightPad(toString(x), x, '-'), number_array) AS text_array,
                arrayZip(number_array, text_array) AS number_text_tuple_array,
                mapFromArrays(number_array, text_array) AS number_to_text_map,
                toLowCardinality(text) AS low_cardinality_text,
                toLowCardinality(nullable_text) AS low_cardinality_nullable,
                sumSimpleState(number) AS simple_aggregate_number,
                anySimpleState(text) AS simple_aggregate_text
            FROM numbers({limit:UInt64})
            GROUP BY number
            ORDER BY number",
        )
        .param("limit", num_rows)
        .fetch_native()
        .unwrap();

    let Some(block) = cursor.next().await.unwrap() else {
        assert_eq!(
            num_rows, 0,
            "num_rows is nonzero but cursor returned no blocks"
        );
        return;
    };

    assert_eq!(block.num_rows() as u64, num_rows);

    assert_eq!(block.columns().len(), 14);

    assert_eq!(block[0].name(), "number");

    let mut number_iter = block["number"].iter::<u64>().unwrap();

    assert_eq!(
        number_iter.size_hint(),
        (num_rows as usize, Some(num_rows as usize))
    );

    for (number, row) in number_iter.by_ref().zip(0u64..num_rows) {
        assert_eq!(number.unwrap(), row);
    }

    assert!(number_iter.next().is_none());

    let mut text_iter = block["text"].iter::<&str>().unwrap();

    assert_eq!(
        text_iter.size_hint(),
        (num_rows as usize, Some(num_rows as usize))
    );

    for (text, row) in text_iter.by_ref().zip(0u64..num_rows) {
        let text = text.unwrap();

        assert_eq!(text.len() as u64, row);

        if row > 0 {
            assert_eq!(text, format!("{row:.>width$}", width = row as usize));
        } else {
            assert_eq!(text, "");
        }
    }

    assert!(text_iter.next().is_none());

    let mut nullable_number_iter = block["nullable_number"].iter::<Option<u64>>().unwrap();

    assert_eq!(
        nullable_number_iter.size_hint(),
        (num_rows as usize, Some(num_rows as usize))
    );

    for (opt_number, row) in nullable_number_iter.by_ref().zip(0u64..num_rows) {
        let opt_number = opt_number.unwrap();

        if row % 2 == 0 {
            assert_eq!(opt_number, Some(row));
        } else {
            assert_eq!(opt_number, None);
        }
    }

    assert!(nullable_number_iter.next().is_none());

    let mut nullable_text_iter = block["nullable_text"].iter::<Option<&str>>().unwrap();

    assert_eq!(
        nullable_text_iter.size_hint(),
        (num_rows as usize, Some(num_rows as usize))
    );

    for (opt_text, row) in nullable_text_iter.by_ref().zip(0u64..num_rows) {
        let opt_text = opt_text.unwrap();

        if row % 2 != 0 {
            assert_eq!(
                opt_text.unwrap().len() as u64,
                row,
                "incorrect nullable text: {opt_text:?}"
            );
        } else {
            assert_eq!(opt_text, None);
        }
    }

    assert!(nullable_text_iter.next().is_none());

    let mut tuple_iter = block["number_text_tuple"].iter::<(u64, &str)>().unwrap();

    assert_eq!(
        tuple_iter.size_hint(),
        (num_rows as usize, Some(num_rows as usize))
    );

    for (tuple, row) in tuple_iter.by_ref().zip(0u64..num_rows) {
        let (number, text) = tuple.unwrap();

        assert_eq!(number, row);
        assert_eq!(text.len() as u64, row);

        if row > 0 {
            assert_eq!(text, format!("{row:.>width$}", width = row as usize));
        }
    }

    assert!(tuple_iter.next().is_none());

    let mut nullable_tuple_iter = block["nullable_tuple"]
        .iter::<(Option<u64>, Option<&str>)>()
        .unwrap();

    assert_eq!(
        nullable_tuple_iter.size_hint(),
        (num_rows as usize, Some(num_rows as usize))
    );

    for (tuple, row) in nullable_tuple_iter.by_ref().zip(0u64..num_rows) {
        let (opt_number, opt_text) = tuple.unwrap();

        if row % 2 == 0 {
            assert_eq!(opt_number, Some(row));
        } else {
            assert_eq!(opt_number, None);
        }

        if row % 2 != 0 {
            assert_eq!(
                opt_text.unwrap().len() as u64,
                row,
                "incorrect nullable text: {opt_text:?}"
            );
        } else {
            assert_eq!(opt_text, None);
        }
    }

    let mut number_array_iter = block["number_array"].iter::<Vec<u64>>().unwrap();

    assert_eq!(
        number_array_iter.size_hint(),
        (num_rows as usize, Some(num_rows as usize))
    );

    for (number_array, row) in number_array_iter.by_ref().zip(0u64..num_rows) {
        let number_array = number_array.unwrap();

        assert_eq!(number_array.len() as u64, row);

        assert!(
            number_array.iter().copied().eq(1u64..=row),
            "invalid number_array for row #{row}: {number_array:?}"
        );
    }

    assert!(number_array_iter.next().is_none());

    let mut text_array_iter = block["text_array"].iter::<Vec<&str>>().unwrap();

    assert_eq!(
        text_array_iter.size_hint(),
        (num_rows as usize, Some(num_rows as usize))
    );

    for (text_array, row) in text_array_iter.by_ref().zip(0u64..num_rows) {
        let text_array = text_array.unwrap();

        assert_eq!(text_array.len() as u64, row);

        for (text, len) in text_array.iter().zip(1..=row) {
            assert_eq!(*text, format!("{len:-<width$}", width = len as usize));
        }
    }

    assert!(text_array_iter.next().is_none());

    let mut map_iter = block["number_to_text_map"]
        .iter::<HashMap<u64, String>>()
        .unwrap();

    assert_eq!(
        map_iter.size_hint(),
        (num_rows as usize, Some(num_rows as usize))
    );

    for (map, row) in map_iter.by_ref().zip(0u64..num_rows) {
        let map = map.unwrap();

        assert_eq!(map.len() as u64, row);

        for i in 1..=row {
            assert_eq!(map[&i], format!("{i:-<width$}", width = i as usize))
        }
    }

    assert!(map_iter.next().is_none());

    // Should be identical to `text` column
    let mut lc_text_iter = block["low_cardinality_text"].iter::<&str>().unwrap();

    assert_eq!(
        lc_text_iter.size_hint(),
        (num_rows as usize, Some(num_rows as usize))
    );

    for (text, row) in lc_text_iter.by_ref().zip(0u64..num_rows) {
        let text = text.unwrap();

        assert_eq!(text.len() as u64, row);

        if row > 0 {
            assert_eq!(text, format!("{row:.>width$}", width = row as usize));
        }
    }

    assert!(lc_text_iter.next().is_none());

    // Should be identical to `nullable_text` column
    let mut lc_nullable_text_iter = block["low_cardinality_nullable"]
        .iter::<Option<&str>>()
        .unwrap();

    assert_eq!(
        lc_nullable_text_iter.size_hint(),
        (num_rows as usize, Some(num_rows as usize))
    );

    for (opt_text, row) in lc_nullable_text_iter.by_ref().zip(0u64..num_rows) {
        let opt_text = opt_text.unwrap();

        if row % 2 != 0 {
            assert_eq!(
                opt_text.unwrap().len() as u64,
                row,
                "incorrect nullable text: {opt_text:?}"
            );
        } else {
            assert_eq!(opt_text, None);
        }
    }

    assert!(lc_nullable_text_iter.next().is_none());

    let mut simple_agg_number_iter = block["simple_aggregate_number"].iter::<u64>().unwrap();

    for (number, row) in simple_agg_number_iter.by_ref().zip(0u64..num_rows) {
        let number = number.unwrap();

        assert_eq!(number, row);
    }

    assert!(simple_agg_number_iter.next().is_none());

    let mut simple_agg_text_iter = block["simple_aggregate_text"].iter::<String>().unwrap();

    for (text, row) in simple_agg_text_iter.by_ref().zip(0u64..num_rows) {
        let text = text.unwrap();

        assert_eq!(text.len() as u64, row);

        if row > 0 {
            assert_eq!(text, format!("{row:.>width$}", width = row as usize));
        } else {
            assert_eq!(text, "");
        }
    }

    assert!(simple_agg_text_iter.next().is_none());
}

#[tokio::test]
async fn empty_arrays() {
    use std::fmt::Write;

    // test decoding empty arrays of various types
    let client = get_client();

    let types = [
        "Int8",
        "UInt64",
        "String",
        "Nullable(Int32)",
        "Nullable(String)",
        "LowCardinality(UInt64)",
        "LowCardinality(String)",
        "LowCardinality(Nullable(String))",
        "Tuple(UInt64, String, String)",
        "Nullable(Tuple(UInt64, String, String))",
        "Map(UInt64, String)",
        "Map(LowCardinality(UInt64), String)",
    ];

    let mut query = "SELECT \n".to_string();

    let mut comma = false;

    for ty in types {
        if comma {
            writeln!(query, ",").unwrap();
        }

        write!(query, "defaultValueOfTypeName('Array({ty})')").unwrap();

        comma = true;
    }

    let mut cursor = client.query(&query).fetch_native().unwrap();

    let block = cursor.next().await.unwrap().expect("expected one block");

    fn assert_array_empty<'a, T: Decode<'a> + Debug>(col: &'a Column) {
        let mut iter = col.iter::<Vec<T>>().unwrap();

        let vec = iter.next().expect("expected array").unwrap();

        assert!(vec.is_empty(), "got nonempty array: {vec:?}");

        let next = iter.next();

        assert!(next.is_none(), "unexpected second value: {next:?}");
    }

    assert_array_empty::<i8>(&block[0]);
    assert_array_empty::<u64>(&block[1]);
    assert_array_empty::<String>(&block[2]);
    assert_array_empty::<Option<i32>>(&block[3]);
    assert_array_empty::<Option<String>>(&block[4]);
    assert_array_empty::<u64>(&block[5]);
    assert_array_empty::<String>(&block[6]);
    assert_array_empty::<Option<String>>(&block[7]);
    assert_array_empty::<(u64, String, String)>(&block[8]);
    assert_array_empty::<Option<(u64, String, String)>>(&block[9]);
    assert_array_empty::<HashMap<u64, String>>(&block[10]);
    assert_array_empty::<HashMap<u64, String>>(&block[11]);
}

async fn nested_arrays(num_rows: usize) {
    let client = get_client();

    let mut cursor = client.query(
        "SELECT
            arrayMap(x -> arrayEnumerate(arrayWithConstant(x, x)), arrayEnumerate(arrayWithConstant(number, number))) AS nested_number_array,
            arrayMap(arr -> mapFromArrays(arr, arrayMap(x -> toString(x), arr)), nested_number_array) AS array_of_maps,
            arrayMap(x -> toLowCardinality(x), arrayEnumerate(arrayWithConstant(number, number))) AS lc_array
        FROM system.numbers
        LIMIT {limit:UInt64}"
    )
        .param("limit", num_rows)
        .fetch_native()
        .unwrap();

    let Some(block) = cursor.next().await.unwrap() else {
        assert_eq!(
            num_rows, 0,
            "num_rows is nonzero but cursor returned no blocks"
        );
        return;
    };

    let mut nested_array_iter = block["nested_number_array"]
        .iter::<Vec<Vec<u32>>>()
        .unwrap();

    for (nested_array, row) in nested_array_iter.by_ref().zip(0..num_rows) {
        let nested_array = nested_array.unwrap();

        assert_eq!(nested_array.len(), row);

        for (array, i) in nested_array.iter().zip(1..=row) {
            assert_eq!(*array, (1..=i as u32).collect::<Vec<_>>());
        }
    }

    assert!(nested_array_iter.next().is_none());

    let mut nested_map_iter = block["array_of_maps"]
        // Doubles as coverage for `impl Decode for BTreeMap`
        .iter::<Vec<BTreeMap<u32, String>>>()
        .unwrap();

    for (nested_map, row) in nested_map_iter.by_ref().zip(0..num_rows) {
        let nested_map = nested_map.unwrap();

        assert_eq!(nested_map.len(), row);

        for (map, i) in nested_map.iter().zip(1..=row) {
            assert_eq!(
                *map,
                (1..=i as u32)
                    .map(|i| (i, i.to_string()))
                    .collect::<BTreeMap<_, _>>()
            );
        }
    }

    assert!(nested_map_iter.next().is_none());

    let mut lc_array_iter = block["lc_array"].iter::<Vec<u32>>().unwrap();

    for (array, row) in lc_array_iter.by_ref().zip(0..num_rows) {
        let array = array.unwrap();

        assert_eq!(array, (1..=row as u32).collect::<Vec<_>>());
    }

    assert!(lc_array_iter.next().is_none());
}

#[tokio::test]
async fn nested_arrays_empty() {
    nested_arrays(0).await;
}

#[tokio::test]
async fn nested_arrays_10() {
    nested_arrays(10).await;
}

#[tokio::test]
async fn nested_arrays_100() {
    nested_arrays(100).await;
}

#[ignore] // requires a lot of memory
#[tokio::test]
async fn nested_arrays_1000() {
    nested_arrays(1000).await;
}

#[tokio::test]
async fn fixed_width_tuples() {
    let client = get_client();

    let mut cursor = client
        .query(
            "SELECT
                tuple(toUInt64(number), toUInt32(number + 1)) AS fixed_tuple,
                arrayMap(
                    x -> tuple(toUInt64(x), toUInt32(x + 1)),
                    range(number)
                ) AS fixed_tuple_array
            FROM system.numbers
            LIMIT 3",
        )
        .fetch_native()
        .unwrap();

    let block = cursor.next().await.unwrap().expect("expected one block");

    let tuples = block["fixed_tuple"]
        .iter::<(u64, u32)>()
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(tuples, [(0, 1), (1, 2), (2, 3)]);

    let tuple_arrays = block["fixed_tuple_array"]
        .iter::<Vec<(u64, u32)>>()
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(tuple_arrays, [vec![], vec![(0, 1)], vec![(0, 1), (1, 2)]]);
}
