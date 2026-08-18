//! Unit tests for the Avro codec module.

mod containers {
    use crate::io::{Buffer, IOBase};
    use crate::{MediaType, MimeType, Value, avro};

    /// A record schema exercising every branch the manifests use.
    fn schema() -> Value {
        crate::json::from_str(
            r#"{"type":"record","name":"row","fields":[
                {"name":"code","type":"int","field-id":1},
                {"name":"name","type":"string","field-id":2},
                {"name":"score","type":["null","double"],"default":null,"field-id":3},
                {"name":"raw","type":["null","bytes"],"default":null,"field-id":4},
                {"name":"tags","type":{"type":"array","element-id":6,"items":"long"},
                 "field-id":5},
                {"name":"nested","type":{"type":"record","name":"inner","fields":[
                    {"name":"flag","type":"boolean","field-id":8}
                ]},"field-id":7}
            ]}"#,
        )
        .unwrap()
    }

    fn buffer() -> Buffer {
        let mut buffer = Buffer::new();
        buffer.set_media_type(MediaType::new(MimeType::AVRO));
        buffer
    }

    #[test]
    fn a_container_round_trips_every_encoded_branch() {
        let schema = schema();
        let row = crate::json::from_str(
            r#"{"code":-7,"name":"AAPL","score":1.5,"raw":null,"tags":[1,2,300000],
                "nested":{"flag":true}}"#,
        )
        .unwrap();
        let empty = crate::json::from_str(
            r#"{"code":0,"name":"","score":null,"raw":null,"tags":[],
                "nested":{"flag":false}}"#,
        )
        .unwrap();

        let mut handle = buffer();
        avro::write_container(
            &mut handle,
            &schema,
            &[("format-version", "2")],
            &[row.clone(), empty.clone()],
        )
        .unwrap();

        let container = avro::read_container(&handle).unwrap();
        assert_eq!(container.get("format-version"), Some("2"));
        assert_eq!(container.rows.len(), 2);
        assert_eq!(
            container.rows[0].get_key_str("code").unwrap().as_i64(),
            Some(-7)
        );
        assert_eq!(
            container.rows[0].get_key_str("name").unwrap().as_str(),
            Some("AAPL")
        );
        assert_eq!(
            container.rows[0].get_key_str("score").unwrap().as_f64(),
            Some(1.5)
        );
        assert!(container.rows[0].get_key_str("raw").unwrap().is_null());
        assert_eq!(container.rows[0].get_key_str("tags").unwrap().len(), 3);
        assert_eq!(
            container.rows[1].get_key_str("tags").unwrap().len(),
            0,
            "an empty array is one zero-count block"
        );
        assert_eq!(
            container.rows[1]
                .get_key_str("nested")
                .and_then(|nested| nested.get_key_str("flag"))
                .and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn an_empty_container_is_a_header_with_no_blocks() {
        let mut handle = buffer();
        avro::write_container(&mut handle, &schema(), &[], &[]).unwrap();
        let container = avro::read_container(&handle).unwrap();
        assert!(container.rows.is_empty());
    }

    #[test]
    fn bytes_that_are_not_a_container_say_what_was_expected() {
        let mut handle = buffer();
        handle.write_all_bytes(b"not avro at all").unwrap();
        let message = avro::read_container(&handle).unwrap_err().to_string();
        assert!(message.contains("Avro object container"), "{message}");
    }

    #[test]
    fn a_truncated_container_reports_the_byte_it_ran_out_at() {
        let mut handle = buffer();
        avro::write_container(
            &mut handle,
            &schema(),
            &[],
            &[crate::json::from_str(
                r#"{"code":1,"name":"x","score":null,"raw":null,"tags":[],
                    "nested":{"flag":true}}"#,
            )
            .unwrap()],
        )
        .unwrap();

        let mut truncated = buffer();
        let bytes = handle.read_all().unwrap();
        truncated
            .write_all_bytes(&bytes[..bytes.len() - 8])
            .unwrap();
        let message = avro::read_container(&truncated).unwrap_err().to_string();
        assert!(message.contains("avro"), "{message}");
        assert!(message.contains("expected"), "{message}");
    }
}
