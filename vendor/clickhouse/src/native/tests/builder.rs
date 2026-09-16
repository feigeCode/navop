use crate::native::builder::{BlockBuilder, BlockBuilderError, UpsertColumnError};
use clickhouse_types::DataTypeNode;
use std::collections::BTreeMap;

#[test]
fn forbids_incompatible_upsert() {
    let mut builder = BlockBuilder::new();

    builder.upsert_column::<i32>("foo").unwrap();

    let Err(e) = builder.upsert_column::<u32>("foo") else {
        panic!("expected error")
    };

    let UpsertColumnError::ColumnExists {
        name,
        existing_type,
        new_type,
    } = e
    else {
        panic!("unexpected error variant: {e:?}")
    };

    assert_eq!(*name, *"foo");
    assert_eq!(*existing_type, DataTypeNode::Int32);
    assert_eq!(*new_type, DataTypeNode::UInt32);
}

#[test]
fn forbids_mismatched_lengths() {
    let mut builder = BlockBuilder::new();

    builder
        .upsert_column::<i32>("foo")
        .unwrap()
        .add_all([0, 1, 2, 3, 4, 5])
        .unwrap();

    builder
        .upsert_column::<u32>("bar")
        .unwrap()
        .add_all([0, 1, 2, 3])
        .unwrap();

    let Err(err) = builder.build() else {
        panic!("expected error");
    };

    let BlockBuilderError::MismatchedLengths {
        longest_column,
        longest_len,
        shortest_column,
        shortest_len,
    } = err
    else {
        panic!("unexpected error variant: {err:?}")
    };

    assert_eq!(*longest_column, *"foo");
    assert_eq!(longest_len, 6);

    assert_eq!(*shortest_column, *"bar");
    assert_eq!(shortest_len, 4);
}

#[test]
fn debug() {
    let mut builder = BlockBuilder::new();

    assert_eq!(format!("{builder:?}"), "BlockBuilder { columns: [] }");

    builder
        .upsert_column::<i32>("foo")
        .unwrap()
        .add_all([0, 1, 2, 3, 4])
        .unwrap();

    // Saves us having to write these out by hand
    insta::assert_debug_snapshot!(builder);

    builder
        .upsert_column::<&str>("bar")
        .unwrap()
        .add_all(["lorem", "ipsum", "dolor", "sit", "amet"])
        .unwrap();

    insta::assert_debug_snapshot!(builder);

    builder
        .upsert_column::<&[u64]>("baz")
        .unwrap()
        .add_all([
            &[][..],
            &[0],
            &[0, 1],
            &[0, 1, 2],
            &[0, 1, 2, 3],
            &[0, 1, 2, 3, 4],
        ])
        .unwrap();

    insta::assert_debug_snapshot!(builder);

    builder
        .upsert_column("quux")
        .unwrap()
        .add_all((0u32..5).map(|i| (i, i.to_string())))
        .unwrap();

    insta::assert_debug_snapshot!(builder);

    builder
        .upsert_column("foobar")
        .unwrap()
        // `HashMap` order is not deterministic
        .add_all((0i64..5).map(|i| {
            (0..i)
                .map(|j| (j, j.to_string()))
                .collect::<BTreeMap<_, _>>()
        }))
        .unwrap();

    insta::assert_debug_snapshot!(builder);

    builder
        .upsert_column("foo_with_nulls")
        .unwrap()
        .add_all((0..5).map(|i| (i % 2 != 0).then_some(i)))
        .unwrap();

    insta::assert_debug_snapshot!(builder);
}

#[test]
fn errors_on_forbidden_types() {
    let mut builder = BlockBuilder::new();

    // https://clickhouse.com/docs/reference/data-types/nullable
    let Err(e) = builder.upsert_column::<Option<Vec<i32>>>("foo") else {
        panic!("expected error");
    };

    let UpsertColumnError::UnsupportedType {
        column_name,
        data_type,
    } = e
    else {
        panic!("unexpected error kind: {e:?}")
    };

    assert_eq!(*column_name, *"foo");
    assert_eq!(
        *data_type,
        DataTypeNode::Nullable(DataTypeNode::Array(DataTypeNode::Int32.into()).into())
    );

    let Err(e) = builder.upsert_column::<Option<BTreeMap<i32, String>>>("foo") else {
        panic!("expected error");
    };

    let UpsertColumnError::UnsupportedType {
        column_name,
        data_type,
    } = e
    else {
        panic!("unexpected error kind: {e:?}")
    };

    assert_eq!(*column_name, *"foo");
    assert_eq!(
        *data_type,
        DataTypeNode::Nullable(
            DataTypeNode::Map([DataTypeNode::Int32.into(), DataTypeNode::String.into(),]).into()
        )
    );
}

#[test]
fn errors_on_empty_block() {
    let mut builder = BlockBuilder::new();

    let Err(e) = builder.build() else {
        panic!("expected error");
    };

    assert!(
        matches!(e, BlockBuilderError::BlockEmpty),
        "unexpected error kind: {e:?}"
    );

    builder.upsert_column::<i32>("foo").unwrap();
    builder.upsert_column::<String>("bar").unwrap();

    let Err(e) = builder.build() else {
        panic!("expected error");
    };

    assert!(
        matches!(e, BlockBuilderError::BlockEmpty),
        "unexpected error kind: {e:?}"
    );
}
