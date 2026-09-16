#[cfg(feature = "lz4")]
pub(crate) mod lz4;
#[cfg(feature = "zstd")]
pub(crate) mod zstd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Compression {
    /// Disables any compression.
    /// Used by default if no compression feature is enabled.
    None,
    /// Uses `LZ4` codec to (de)compress.
    /// Used by default if the `lz4` feature is enabled.
    #[cfg(feature = "lz4")]
    Lz4,
    /// Uses `LZ4HC` codec to compress and `LZ4` to decompress.
    /// High compression levels are useful in networks with low bandwidth.
    /// Affects only `INSERT`s, because others are compressed by the server.
    /// Possible levels: `[1, 12]`. Recommended level range: `[4, 9]`.
    ///
    /// Deprecated: `lz4_flex` doesn't support HC mode yet: [lz4_flex#165].
    ///
    /// [lz4_flex#165]: https://github.com/PSeitz/lz4_flex/issues/165
    #[cfg(feature = "lz4")]
    #[deprecated(note = "use `Compression::Lz4` instead")]
    Lz4Hc(i32),
    /// Uses `ZSTD` codec to (de)compress.
    /// Used by default if the `zstd` feature is enabled and `lz4` is not.
    /// The `i32` parameter specifies the compression level.
    /// Use [`Compression::zstd()`] for the default level.
    ///
    /// **Note:** Extremely high compression levels (e.g. above 19) are very
    /// CPU-intensive and likely unsuitable for real-time networked
    /// applications. They can also block the async executor for a
    /// significant amount of time. Prefer moderate levels for online usage.
    #[cfg(feature = "zstd")]
    Zstd(i32),
}

impl Default for Compression {
    #[cfg(all(not(feature = "test-util"), feature = "lz4"))]
    #[inline]
    fn default() -> Self {
        Compression::Lz4
    }

    #[cfg(all(not(feature = "test-util"), not(feature = "lz4"), feature = "zstd"))]
    #[inline]
    fn default() -> Self {
        Compression::zstd()
    }

    #[cfg(any(feature = "test-util", not(any(feature = "lz4", feature = "zstd"))))]
    #[inline]
    fn default() -> Self {
        Compression::None
    }
}

impl Compression {
    /// Creates a `Zstd` compression with the default level.
    #[cfg(feature = "zstd")]
    pub fn zstd() -> Self {
        Compression::Zstd(::zstd::DEFAULT_COMPRESSION_LEVEL)
    }

    pub(crate) fn is_enabled(&self) -> bool {
        *self != Compression::None
    }
}
