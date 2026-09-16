use std::fmt;

use super::{escape, ser};
use serde::Serialize;

pub trait Bind: sealed::Sealed {
    #[doc(hidden)]
    fn write(&self, dst: &mut impl fmt::Write) -> Result<(), String>;
}

impl<S: Serialize> sealed::Sealed for S {}

impl<S: Serialize> Bind for S {
    #[inline]
    fn write(&self, mut dst: &mut impl fmt::Write) -> Result<(), String> {
        ser::write_arg(&mut dst, self)
    }
}

/// Bound the provided string as an identifier.
/// It can be used for table names, for instance.
#[derive(Copy, Clone)]
pub struct Identifier<'a>(pub &'a str);

impl sealed::Sealed for Identifier<'_> {}

impl Bind for Identifier<'_> {
    #[inline]
    fn write(&self, dst: &mut impl fmt::Write) -> Result<(), String> {
        escape::identifier(self.0, dst).map_err(|err| err.to_string())
    }
}

mod sealed {
    pub trait Sealed {}
}
