use crate::error::BoxedError;
use crate::native::builder::{BlockBuilder, BlockBuilderError};
use crate::native::encode::{Encode, ValueWriter};
use clickhouse_types::DataTypeNode;

use std::collections::HashMap;
use std::mem;

#[test]
fn array_writer_rolls_back() {
    struct BadArray<'a>(&'a [u32]);

    impl Encode for BadArray<'_> {
        fn produces() -> DataTypeNode {
            DataTypeNode::Array(Box::new(DataTypeNode::UInt32))
        }

        fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
            let mut writer = writer.write_array()?;

            for val in self.0 {
                writer.write(val)?;
            }

            // Deliberately don't call `writer.finish()`
            drop(writer);

            Ok(())
        }
    }

    let mut builder = BlockBuilder::new();

    builder
        .upsert_column("foo")
        .unwrap()
        .add(&[0u32, 1, 2, 3, 4, 5][..])
        .unwrap();

    builder
        .upsert_column("foo")
        .unwrap()
        .add(BadArray(&[6, 7, 8, 9, 10]))
        .unwrap();

    let block = builder.build().unwrap();

    assert_eq!(block.num_rows(), 1);
}

#[test]
fn tuple_writer_rolls_back() {
    struct BadTuple(i32, String, #[expect(dead_code)] Vec<i64>);

    impl Encode for BadTuple {
        fn produces() -> DataTypeNode {
            DataTypeNode::Tuple(vec![
                DataTypeNode::Int32,
                DataTypeNode::String,
                DataTypeNode::Array(Box::new(DataTypeNode::Int64)),
            ])
        }

        fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
            let mut writer = writer.write_tuple()?;

            writer.write(self.0)?;
            writer.write(&self.1)?;

            // Deliberately don't finish, this would put the block out of sync
            drop(writer);

            Ok(())
        }
    }

    let mut builder = BlockBuilder::new();

    builder
        .upsert_column("foo")
        .unwrap()
        .add((0i32, "0".to_string(), vec![0i64; 16]))
        .unwrap();

    builder
        .upsert_column("foo")
        .unwrap()
        .add(BadTuple(1, "1".to_string(), vec![1i64; 16]))
        .unwrap();

    let block = builder.build().unwrap();

    assert_eq!(block.num_rows(), 1);
}

#[test]
fn map_writer_rolls_back() {
    struct BadMap(Vec<(u32, String)>);

    impl Encode for BadMap {
        fn produces() -> DataTypeNode {
            DataTypeNode::Map([
                Box::new(DataTypeNode::UInt32),
                Box::new(DataTypeNode::String),
            ])
        }

        fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
            let mut writer = writer.write_map()?;

            for (key, val) in &self.0 {
                writer.write(key, val)?;
            }

            drop(writer);

            Ok(())
        }
    }

    let mut builder = BlockBuilder::new();

    builder
        .upsert_column("foo")
        .unwrap()
        .add(
            (0u32..5)
                .map(|i| (i, i.to_string()))
                .collect::<HashMap<_, _>>(),
        )
        .unwrap();

    builder
        .upsert_column("foo")
        .unwrap()
        .add(BadMap((5..10).map(|i| (i, i.to_string())).collect()))
        .unwrap();

    let block = builder.build().unwrap();

    assert_eq!(block.num_rows(), 1);
}

// Verify that `MapWriter` rolls back a successfully written key if the value fails to encode.
#[test]
fn map_writer_failed_write_rolls_back() {
    struct BadValue;

    impl Encode for BadValue {
        fn produces() -> DataTypeNode {
            DataTypeNode::String
        }

        fn encode(&self, _writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
            Err("BadValue".into())
        }
    }

    let mut builder = BlockBuilder::new();

    let mut column = builder.upsert_column("map_column").unwrap();

    let err = column
        .add(HashMap::from([("foo", BadValue)]))
        .expect_err("expected error");

    assert_eq!(err.to_string(), "error writing value at entry index 0");

    // Validation would fail if the column was out of sync.
    column.validate().unwrap();
}

// A leak writer could put a block out of sync and result in data corruption;
// it's better if we catch it during validation.
#[test]
fn leaked_array_writer_fails_validation() {
    struct LeakWriter<'a>(&'a [u32]);

    impl Encode for LeakWriter<'_> {
        fn produces() -> DataTypeNode {
            DataTypeNode::Array(Box::new(DataTypeNode::UInt32))
        }

        fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
            let mut writer = writer.write_array()?;

            for val in self.0 {
                writer.write(val)?;
            }

            // Deliberately leak the writer to put the block out of sync
            mem::forget(writer);

            Ok(())
        }
    }

    let mut builder = BlockBuilder::new();

    builder
        .upsert_column("foo")
        .unwrap()
        .add(&[0u32, 1, 2, 3, 4, 5][..])
        .unwrap();

    builder
        .upsert_column("foo")
        .unwrap()
        .add(LeakWriter(&[6, 7, 8, 9, 10]))
        .unwrap();

    let err = builder.build().err().expect("expected block builder error");

    let BlockBuilderError::ColumnDataInvalid {
        column_name,
        column_type,
        message,
    } = err
    else {
        panic!("unexpected error kind: {err}");
    };

    assert_eq!(*column_name, *"foo");
    assert_eq!(*column_type, <[u32] as Encode>::produces());

    assert_eq!(
        *message,
        *"last array index (6) out of sync with total elements: 11"
    );
}

#[test]
fn leaked_tuple_writer_fails_validation() {
    struct LeakWriter(i32, String, #[expect(dead_code)] Vec<i64>);

    impl Encode for LeakWriter {
        fn produces() -> DataTypeNode {
            DataTypeNode::Tuple(vec![
                DataTypeNode::Int32,
                DataTypeNode::String,
                DataTypeNode::Array(Box::new(DataTypeNode::Int64)),
            ])
        }

        fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
            let mut writer = writer.write_tuple()?;

            writer.write(self.0)?;
            writer.write(&self.1)?;

            // Deliberately leak writer, this would put the block out of sync
            mem::forget(writer);

            Ok(())
        }
    }

    let mut builder = BlockBuilder::new();

    builder
        .upsert_column("foo")
        .unwrap()
        .add((0i32, "0".to_string(), vec![0i64; 16]))
        .unwrap();

    builder
        .upsert_column("foo")
        .unwrap()
        .add(LeakWriter(1, "1".to_string(), vec![1i64; 16]))
        .unwrap();

    let err = builder.build().err().expect("expected block builder error");

    let BlockBuilderError::ColumnDataInvalid {
        column_name,
        column_type,
        message,
    } = err
    else {
        panic!("unexpected error kind: {err}");
    };

    assert_eq!(*column_name, *"foo");
    assert_eq!(
        *column_type,
        <(i32, String, Vec<i64>) as Encode>::produces()
    );

    assert_eq!(
        *message,
        *"tuple index 2 (type Array(Int64)) total elements out of sync: 1 vs 2"
    );
}

#[test]
fn leaked_map_writer_fails_validation() {
    struct LeakWriter(Vec<(u32, String)>);

    impl Encode for LeakWriter {
        fn produces() -> DataTypeNode {
            DataTypeNode::Map([
                Box::new(DataTypeNode::UInt32),
                Box::new(DataTypeNode::String),
            ])
        }

        fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
            let mut writer = writer.write_map()?;

            for (key, val) in &self.0 {
                writer.write(key, val)?;
            }

            mem::forget(writer);

            Ok(())
        }
    }

    let mut builder = BlockBuilder::new();

    builder
        .upsert_column("foo")
        .unwrap()
        .add(
            (0u32..5)
                .map(|i| (i, i.to_string()))
                .collect::<HashMap<_, _>>(),
        )
        .unwrap();

    builder
        .upsert_column("foo")
        .unwrap()
        .add(LeakWriter((5..10).map(|i| (i, i.to_string())).collect()))
        .unwrap();

    let err = builder.build().err().expect("expected block builder error");

    let BlockBuilderError::ColumnDataInvalid {
        column_name,
        column_type,
        message,
    } = err
    else {
        panic!("unexpected error kind: {err}");
    };

    assert_eq!(*column_name, *"foo");
    assert_eq!(*column_type, <HashMap<u32, String> as Encode>::produces());

    assert_eq!(
        *message,
        *"last map index (5) out of sync with total elements: 10"
    );
}
