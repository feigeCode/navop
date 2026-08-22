mod real_databases {
    pub mod common;
    #[cfg(feature = "builtin-duckdb")]
    pub mod duckdb;
}
