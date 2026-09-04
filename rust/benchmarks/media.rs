#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "media/avro.rs"]
mod avro;
#[cfg(feature = "iceberg")]
#[path = "media/iceberg.rs"]
mod iceberg;
#[cfg(feature = "parquet")]
#[path = "media/io.rs"]
mod io;

use criterion::{Criterion, criterion_group};

fn io_benchmarks(_criterion: &mut Criterion) {
    #[cfg(feature = "parquet")]
    {
        io::benchmarks::record::round_trip_benchmarks(_criterion);
        io::benchmarks::dimensions::dimension_benchmarks(_criterion);
        io::benchmarks::write::write_surface_benchmarks(_criterion);
        io::benchmarks::value::structured_scalar_benchmarks(_criterion);
        io::benchmarks::pushdown::projection_benchmarks(_criterion);
    }
}

fn iceberg_benchmarks(_criterion: &mut Criterion) {
    #[cfg(feature = "iceberg")]
    iceberg::benchmarks(_criterion);
}

criterion_group!(
    media,
    avro::container::avro_benchmarks,
    avro::format::format_benchmarks,
    avro::codecs::codec_benchmarks,
    avro::projection::projection_benchmarks,
    avro::resolution::resolution_benchmarks,
    io_benchmarks,
    iceberg_benchmarks,
);

fn main() {
    media();
    #[cfg(feature = "iceberg")]
    iceberg::cleanup();
}
