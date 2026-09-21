//! `rust/src/avro/single.rs`: single-object encoding - one datum framed by its
//! schema fingerprint.

mod avro {

    mod single_object {
        use yggdryl::Scalar;
        use yggdryl::avro;
        use yggdryl::avro::Schema;

        #[test]
        fn a_datum_round_trips_through_the_single_object_framing() {
            let schema = Schema::from_str(
                r#"{"type":"record","name":"trade","fields":[
                {"name":"symbol","type":"string"},
                {"name":"qty","type":"long"}
            ]}"#,
            )
            .unwrap();
            let value = Scalar::from_struct([
                ("symbol", Scalar::from("AAPL")),
                ("qty", Scalar::from(100_i64)),
            ])
            .unwrap();
            let framed = avro::into_single_object_vec(&schema, &value).unwrap();
            assert_eq!(&framed[..2], &[0xC3, 0x01]);
            assert_eq!(
                avro::from_single_object_slice(&framed, &schema).unwrap(),
                value
            );
        }

        #[test]
        fn a_wrong_fingerprint_is_refused_naming_both() {
            let schema = Schema::from_str("\"long\"").unwrap();
            let other = Schema::from_str("\"string\"").unwrap();
            let framed = avro::into_single_object_vec(&schema, &Scalar::from(1)).unwrap();
            let message = avro::from_single_object_slice(&framed, &other)
                .unwrap_err()
                .to_string();
            assert!(message.contains("fingerprint"), "{message}");
        }

        #[test]
        fn bytes_after_the_datum_are_refused() {
            let schema = Schema::from_str("\"long\"").unwrap();
            let mut framed = avro::into_single_object_vec(&schema, &Scalar::from(1)).unwrap();
            framed.push(0x00);
            let message = avro::from_single_object_slice(&framed, &schema)
                .unwrap_err()
                .to_string();
            assert!(message.contains("end after its datum"), "{message}");
        }
    }

    mod snapshots {

        use yggdryl::Scalar;
        use yggdryl::avro;

        /// The single-object framing is fixed by the specification.
        #[test]
        fn a_single_object_datum_encodes_to_exactly_these_bytes() {
            let schema = avro::Schema::from_str("\"long\"").unwrap();
            let framed = avro::into_single_object_vec(&schema, &Scalar::from(3)).unwrap();
            let hex: String = framed.iter().map(|byte| format!("{byte:02x}")).collect();
            // C3 01, the little-endian Rabin fingerprint of "long" (the value
            // fastavro computes for the same schema), then zig-zag 3.
            assert_eq!(hex, "c301b71df49344e154d006");
        }
    }
}
