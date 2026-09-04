pub(crate) mod arrow;
pub(crate) mod ascii;
pub(crate) mod default;
pub(crate) mod floating;
pub(crate) mod geospatial;
pub(crate) mod nested;
pub(crate) mod parser;
pub(crate) mod temporal;
pub(crate) mod value;

pub(crate) const NESTED_SQL: &str = concat!(
    "struct<id:bigint,events:array<struct<timestamp:timestamp(us,UTC),",
    "attributes:map<string,string>,amount:decimal(38,18)>>,",
    "source:string>"
);
