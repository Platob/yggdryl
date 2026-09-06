use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, Float64Array, Int32Array};
use yggdryl::{ArrowCast, ArrowCastOptions, DataType, Field, Representation, Scalar};

fn src() -> ArrayRef {
    Arc::new(Int32Array::from(vec![7_i32]))
}

#[test]
fn probe() {
    for rep in ["value", "bits"] {
        let opts = ArrowCastOptions::new()
            .with_safe(false)
            .with_representation(Representation::from_str(rep).unwrap());
        for target in [
            DataType::Binary,
            DataType::LargeBinary,
            DataType::BinaryView,
            DataType::FixedSizeBinary(4),
            DataType::Utf8,
        ] {
            let f = Field::new("c", target.clone(), true);
            match f.cast_arrow_array(src(), opts.clone()) {
                Ok(a) => {
                    let d = a.to_data();
                    println!("{rep} Int32->{target:?} OK {:?} buffers={:?}", a.data_type(), d.buffers().iter().map(|b| b.as_slice().to_vec()).collect::<Vec<_>>());
                }
                Err(e) => println!("{rep} Int32->{target:?} ERR {e}"),
            }
        }
        // float
        let f = Field::new("c", DataType::Binary, true);
        let fa: ArrayRef = Arc::new(Float64Array::from(vec![7.0_f64]));
        match f.cast_arrow_array(fa, opts.clone()) { Ok(a) => println!("{rep} Float64->Binary OK {:?}", a.data_type()), Err(e) => println!("{rep} Float64->Binary ERR {e}") }
    }
    // row tier
    for target in [DataType::Binary, DataType::LargeBinary, DataType::BinaryView, DataType::FixedSizeBinary(4), DataType::Utf8] {
        println!("row i32 -> {target:?} : {:?}", target.scalar(Scalar::from(7_i32)).map_err(|e| e.to_string()));
    }
    let b: ArrayRef = Arc::new(BinaryArray::from(vec![&[7_u8, 0, 0, 0][..]]));
    let _ = b.len();
}
