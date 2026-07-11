//! Tests for the typed native-type buffers (`I8Buffer` … `F64Buffer`,
//! `BooleanBuffer`).

use std::collections::HashSet;

use yggdryl_buffer::{BooleanBuffer, BufferError, F64Buffer, I32Buffer, I64Buffer, U8Buffer};
use yggdryl_core::{IOBase, Whence};

#[test]
fn numeric_construct_and_access() {
    let buffer = I64Buffer::from_slice(&[10, 20, 30]);
    assert_eq!(buffer.len(), 3);
    assert!(!buffer.is_empty());
    assert_eq!(buffer.as_slice(), &[10, 20, 30]);
    assert_eq!(buffer.get(1), Some(20));
    assert_eq!(buffer.get(3), None);
    assert_eq!(buffer.to_vec(), vec![10, 20, 30]);
    assert!(I64Buffer::new().is_empty());
}

#[test]
fn numeric_serialize_round_trip_and_width_validation() {
    let buffer = I32Buffer::from_slice(&[1, -2, 3]);
    let bytes = buffer.serialize_bytes();
    assert_eq!(bytes.len(), 12); // 3 × 4 bytes
    assert_eq!(I32Buffer::deserialize_bytes(&bytes).unwrap(), buffer);

    // little-endian layout
    assert_eq!(
        &U8Buffer::from_slice(&[1, 2, 3]).serialize_bytes(),
        &[1, 2, 3]
    );
    assert_eq!(
        I32Buffer::from_slice(&[0x0102_0304]).as_bytes(),
        [0x04, 0x03, 0x02, 0x01]
    );

    // a non-multiple length names the fix
    assert_eq!(
        I32Buffer::deserialize_bytes(&[0; 6]).unwrap_err(),
        BufferError::InvalidByteLength {
            len: 6,
            width: 4,
            ty: "i32"
        }
    );
    assert!(I32Buffer::deserialize_bytes(&[0; 6])
        .unwrap_err()
        .to_string()
        .contains("multiple of 4"));
}

#[test]
fn numeric_value_semantics() {
    let a = I64Buffer::from_slice(&[1, 2, 3]);
    let b = I64Buffer::from_vec(vec![1, 2, 3]);
    assert_eq!(a, b);
    let set: HashSet<I64Buffer> = [a.clone(), b, I64Buffer::from_slice(&[9])]
        .into_iter()
        .collect();
    assert_eq!(set.len(), 2);
}

#[test]
fn float_equality_is_bitwise() {
    // Same NaN bit-pattern compares equal (byte identity), unlike IEEE `==`.
    let nan1 = F64Buffer::from_slice(&[f64::NAN]);
    let nan2 = F64Buffer::from_slice(&[f64::NAN]);
    assert_eq!(nan1, nan2);
    assert_eq!(nan1.serialize_bytes(), nan2.serialize_bytes());

    // +0.0 and -0.0 have distinct bytes, so they are distinct buffers.
    let pos = F64Buffer::from_slice(&[0.0]);
    let neg = F64Buffer::from_slice(&[-0.0]);
    assert_ne!(pos, neg);
    assert_eq!(
        F64Buffer::deserialize_bytes(&neg.serialize_bytes()).unwrap(),
        neg
    );
}

#[test]
fn numeric_bridges_to_positioned_io() {
    let buffer = I64Buffer::from_slice(&[7, 8, 9]);
    let mut cursor = buffer.byte_cursor();
    assert_eq!(cursor.pread_i64_array(3, Whence::Start).unwrap(), [7, 8, 9]);

    // round-trip back through a ByteBuffer
    let round = I64Buffer::from_byte_buffer(&buffer.to_byte_buffer()).unwrap();
    assert_eq!(round, buffer);
}

#[test]
fn boolean_construct_and_access() {
    let buffer = BooleanBuffer::from_bits(&[true, false, true, true, false]);
    assert_eq!(buffer.len(), 5);
    assert!(!buffer.is_empty());
    assert_eq!(buffer.get(0), Some(true));
    assert_eq!(buffer.get(1), Some(false));
    assert_eq!(buffer.get(5), None);
    assert_eq!(buffer.count_set_bits(), 3);
    assert_eq!(buffer.to_vec(), vec![true, false, true, true, false]);
    assert!(BooleanBuffer::new().is_empty());
}

#[test]
fn boolean_trailing_bits_are_canonicalised() {
    // Only the low 3 bits are valid; the extra set high bits must be ignored.
    let buffer = BooleanBuffer::from_bytes(&[0xFF], 3).unwrap();
    assert_eq!(buffer.count_set_bits(), 3);
    assert_eq!(buffer.as_bytes(), &[0x07]);
    assert_eq!(buffer, BooleanBuffer::from_bits(&[true, true, true]));

    // wrong packed length names the fix
    assert_eq!(
        BooleanBuffer::from_bytes(&[0, 0], 3).unwrap_err(),
        BufferError::InvalidBitLength {
            bytes: 2,
            expected: 1,
            len: 3
        }
    );
}

#[test]
fn boolean_serialize_round_trip() {
    let buffer = BooleanBuffer::from_bits(&[true; 20]);
    let bytes = buffer.serialize_bytes();
    assert_eq!(bytes.len(), 8 + 3); // u64 length header + ceil(20/8) bytes
    assert_eq!(BooleanBuffer::deserialize_bytes(&bytes).unwrap(), buffer);

    assert_eq!(
        BooleanBuffer::deserialize_bytes(&[0, 0, 0]).unwrap_err(),
        BufferError::Truncated {
            needed: 8,
            available: 3
        }
    );

    // value semantics
    let set: HashSet<BooleanBuffer> = [
        BooleanBuffer::from_bits(&[true, false]),
        BooleanBuffer::from_bits(&[true, false]),
        BooleanBuffer::from_bits(&[false, false]),
    ]
    .into_iter()
    .collect();
    assert_eq!(set.len(), 2);
}

mod field_and_headers {
    use yggdryl_buffer::{BooleanBuffer, I64Buffer};
    use yggdryl_field::{Field, TypedField};
    use yggdryl_http::{Headers, HeadersBased};

    #[test]
    fn buffer_hands_out_the_matching_typed_field() {
        // The return type is the concrete `I64Field` for `I64Buffer` (compile-time).
        let field: yggdryl_field::I64Field = I64Buffer::from_slice(&[1, 2, 3]).field("id", false);
        assert_eq!(field.name(), "id");
        assert!(!field.is_nullable());
        let _ = TypedField::data_type(&field); // the typed data type is reachable

        // The boolean buffer hands out a `BooleanField`.
        let bfield: yggdryl_field::BooleanField =
            BooleanBuffer::from_bits(&[true, false]).field("flag", true);
        assert_eq!(bfield.name(), "flag");
        assert!(bfield.is_nullable());
    }

    #[test]
    fn headers_are_attached_and_carried_into_the_field() {
        let headers = Headers::from_pairs([(b"unit".to_vec(), b"ms".to_vec())]);
        let buffer = I64Buffer::from_slice(&[10, 20]).with_headers(headers.clone());
        assert_eq!(buffer.headers(), Some(&headers));

        let field = buffer.field("ts", true);
        assert_eq!(field.headers(), Some(&headers));

        // A buffer without headers hands out a field without headers.
        assert_eq!(
            I64Buffer::from_slice(&[10, 20]).field("ts", true).headers(),
            None
        );
    }

    #[test]
    fn common_header_accessors_and_zero_copy_mutation() {
        let mut buffer = I64Buffer::from_slice(&[1]);
        buffer.set_content_type("application/x.int64");
        assert_eq!(
            buffer.content_type(),
            Some(b"application/x.int64".as_slice())
        );
        // Zero-copy: extend the value bytes in place (no re-insert, no map clone).
        buffer
            .get_header_mut(Headers::CONTENT_TYPE)
            .unwrap()
            .extend_from_slice(b"; le");
        assert_eq!(
            buffer.content_type(),
            Some(b"application/x.int64; le".as_slice())
        );
        // Carried into the field it hands out.
        assert_eq!(
            buffer.field("v", false).content_type(),
            Some(b"application/x.int64; le".as_slice())
        );
    }

    #[test]
    fn headers_do_not_affect_buffer_equality_or_bytes() {
        let plain = I64Buffer::from_slice(&[1, 2, 3]);
        let annotated = I64Buffer::from_slice(&[1, 2, 3])
            .with_headers(Headers::from_pairs([(b"k".to_vec(), b"v".to_vec())]));
        // Byte identity ignores the annotation (rule 7 is over the data bytes).
        assert_eq!(plain, annotated);
        assert_eq!(plain.serialize_bytes(), annotated.serialize_bytes());
    }
}

mod arrow_interop {
    use yggdryl_buffer::{BooleanBuffer, I64Buffer};
    use yggdryl_core::arrow_buffer::{BooleanBuffer as ArrowBooleanBuffer, Buffer, ScalarBuffer};

    #[test]
    fn numeric_from_and_to_arrow_zero_copy() {
        let scalar = ScalarBuffer::<i64>::from(vec![1, 2, 3, 4]);
        let buffer = I64Buffer::from_arrow(scalar);
        assert_eq!(buffer.as_slice(), &[1, 2, 3, 4]);
        // export shares the allocation
        assert_eq!(buffer.to_arrow().as_ref(), &[1, 2, 3, 4]);
    }

    #[test]
    fn numeric_from_sliced_scalar_buffer() {
        let scalar = ScalarBuffer::<i64>::from(vec![0, 1, 2, 3, 4]);
        let sliced = scalar.slice(1, 3); // [1, 2, 3]
        let buffer = I64Buffer::from_arrow(sliced);
        assert_eq!(buffer.as_slice(), &[1, 2, 3]);
        assert_eq!(buffer, I64Buffer::from_slice(&[1, 2, 3]));
    }

    #[test]
    fn boolean_from_arrow_offset_zero_and_offset() {
        // bits 1,0,1 packed as 0b101
        let arrow = ArrowBooleanBuffer::new(Buffer::from_vec(vec![0b0000_0101u8]), 0, 3);
        let buffer = BooleanBuffer::from_arrow(arrow);
        assert_eq!(buffer.to_vec(), vec![true, false, true]);
        assert_eq!(buffer.count_set_bits(), 2);

        // an offset view is materialised into canonical bits
        let base = ArrowBooleanBuffer::new(Buffer::from_vec(vec![0b0000_1010u8]), 0, 4);
        let offset = base.slice(1, 3); // bits 1,0,1 of 0,1,0,1
        let materialised = BooleanBuffer::from_arrow(offset);
        assert_eq!(materialised.len(), 3);
        assert_eq!(materialised.to_vec(), vec![true, false, true]);
    }

    #[test]
    fn boolean_to_arrow_round_trips() {
        let buffer =
            BooleanBuffer::from_bits(&[true, true, false, true, false, false, false, true]);
        let arrow = buffer.to_arrow();
        assert_eq!(arrow.len(), 8);
        assert_eq!(BooleanBuffer::from_arrow(arrow), buffer);
    }
}
