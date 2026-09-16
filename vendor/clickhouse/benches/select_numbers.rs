use serde::Deserialize;

use crate::common_select::{
    BenchmarkOpts, BenchmarkRow, WithAccessType, WithId, do_select_bench, print_header,
    print_results,
};
use clickhouse::Row;

mod common_select;

#[derive(Row, Deserialize)]
struct Data {
    number: u64,
}

impl_benchmark_row_no_access_type!(Data, number);

async fn bench(opts: BenchmarkOpts) {
    let stats =
        do_select_bench::<Data>("SELECT number FROM system.numbers_mt LIMIT 500000000", opts).await;
    assert_eq!(stats.result, 124999999750000000);
    print_results::<Data>(&stats, opts);
}

#[tokio::main]
async fn main() {
    print_header(None);

    for opts in BenchmarkOpts::permutations() {
        bench(opts).await;
    }
}
