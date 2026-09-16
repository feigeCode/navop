use clickhouse_types::DataTypeNode;
use clickhouse_types::data_types::{DecimalType, EnumType};
use std::fmt::{Debug, Formatter, Write};

pub(super) struct DebugNullMap<'a>(pub &'a [u8]);

impl Debug for DebugNullMap<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for b in self.0 {
            match b {
                0 => f.write_char('0')?,
                1 => f.write_char('1')?,
                // Flag invalid bytes with brackets
                _ => write!(f, "{{{b:x}}}")?,
            }
        }

        Ok(())
    }
}

pub(super) struct DebugFixedData<'a> {
    pub(super) type_width: usize,
    pub(super) data: &'a [u8],
}

pub(super) struct DebugHex<'a>(&'a [u8]);

impl Debug for DebugFixedData<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.data.chunks(self.type_width).map(DebugHex))
            .finish()
    }
}

impl Debug for DebugHex<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("0x")?;

        // Write the data as it appears so there's no confusion;
        // for little-endian values this may be in reverse
        for b in self.0.iter() {
            write!(f, "{b:02x}")?;
        }

        Ok(())
    }
}

pub(super) struct DebugVariableData<'a> {
    pub end_offsets: &'a [usize],
    pub data: &'a [u8],
}

impl Debug for DebugVariableData<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut list = f.debug_list();

        let mut start_offset = 0;

        for &end_offset in self.end_offsets {
            // `impl Debug for fmt::Arguments` forwards to `Display`
            list.entry(&format_args!(
                "\"{}\"",
                &self.data[start_offset..end_offset].escape_ascii()
            ));
            start_offset = end_offset;
        }

        list.finish()
    }
}

pub(super) fn type_fixed_width(data_type: &DataTypeNode) -> Option<usize> {
    match data_type {
        DataTypeNode::Bool => Some(1),
        DataTypeNode::UInt8 => Some(1),
        DataTypeNode::UInt16 => Some(2),
        DataTypeNode::UInt32 => Some(4),
        DataTypeNode::UInt64 => Some(8),
        DataTypeNode::UInt128 => Some(16),
        DataTypeNode::UInt256 => Some(32),
        DataTypeNode::Int8 => Some(1),
        DataTypeNode::Int16 => Some(2),
        DataTypeNode::Int32 => Some(4),
        DataTypeNode::Int64 => Some(8),
        DataTypeNode::Int128 => Some(16),
        DataTypeNode::Int256 => Some(32),
        DataTypeNode::Float32 => Some(4),
        DataTypeNode::Float64 => Some(8),
        DataTypeNode::BFloat16 => Some(2),
        DataTypeNode::Decimal(_, _, type_) => match type_ {
            DecimalType::Decimal32 => Some(4),
            DecimalType::Decimal64 => Some(8),
            DecimalType::Decimal128 => Some(16),
            DecimalType::Decimal256 => Some(32),
        },
        DataTypeNode::String => None,
        DataTypeNode::FixedString(len) => Some(*len),
        DataTypeNode::UUID => Some(16),
        DataTypeNode::Date => Some(2),
        DataTypeNode::Date32 => Some(4),
        DataTypeNode::DateTime(_) => Some(4),
        DataTypeNode::DateTime64(_, _) => Some(8),
        DataTypeNode::Time => Some(4),
        DataTypeNode::Time64(_) => Some(8),
        DataTypeNode::Interval(_) => Some(8),
        DataTypeNode::IPv4 => Some(4),
        DataTypeNode::IPv6 => Some(16),
        // Nullable needs to be handled specially
        DataTypeNode::Nullable(_) => None,
        // Type width determined by metadata that comes before column data.
        DataTypeNode::LowCardinality(_) => None,
        DataTypeNode::Array(_) => None,
        // Tuples are serialized column-by-column and need a structural layout.
        DataTypeNode::Tuple(_) => None,
        DataTypeNode::Enum(type_, _) => match type_ {
            EnumType::Enum8 => Some(1),
            EnumType::Enum16 => Some(2),
        },
        DataTypeNode::Map(_) => None,
        DataTypeNode::AggregateFunction(_, _) => None,
        DataTypeNode::SimpleAggregateFunction(_, inner) => type_fixed_width(inner),
        DataTypeNode::Variant(_) => None,
        DataTypeNode::Dynamic => None,
        DataTypeNode::JSON => None,
        DataTypeNode::JsonWithHint(_) => None,
        DataTypeNode::Point => Some(16), // Tuple(Float64, Float64)
        DataTypeNode::Ring => None,
        DataTypeNode::LineString => None,
        DataTypeNode::MultiLineString => None,
        DataTypeNode::Polygon => None,
        DataTypeNode::MultiPolygon => None,
        _ => None,
    }
}
