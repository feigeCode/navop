pub mod db;
pub mod ui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub trait Extension {
    fn new() -> Self
    where
        Self: Sized;
}

#[macro_export]
macro_rules! register_extension {
    ($extension:ty) => {
        const _: fn() = || {
            fn assert_extension<T: $crate::Extension>() {}
            assert_extension::<$extension>();
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Demo;

    impl Extension for Demo {
        fn new() -> Self {
            Self
        }
    }

    #[test]
    fn extension_trait_can_be_implemented() {
        let _ = Demo::new();
    }
}
