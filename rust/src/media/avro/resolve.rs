//! Writer/reader schema resolution: the plan, computed once per pair.
//!
//! A container names the schema its writer used; the reader often wants a
//! different one - fewer fields, renamed fields, promoted widths, defaults for
//! what the writer never knew. The specification's resolution matrix decides
//! what is legal, and this module compiles it into a [`Resolution`] once per
//! (writer, reader) pair: the per-record path executes the plan and never
//! consults a schema again. A writer field the reader does not want becomes a
//! skip step, which is what makes projection cheap.

use std::collections::HashMap;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::TimeUnit;
use crate::{Limits, Result, Scalar, Timezone};

use super::datum::{Cursor, DatumCodec, block_count, codec, invalid};
use super::schema::{EnumType, Node, Schema, Wire};

/// A compiled decoding plan from a writer schema to a reader schema.
///
/// Building one validates the whole resolution up front, except inside unions:
/// a writer branch the reader cannot accept only fails when a datum actually
/// takes that branch, which is the specification's rule.
#[derive(Debug)]
pub struct Resolution {
    /// The root operation.
    op: Op,
    /// One plan per resolved (writer record, reader record) pair, so a
    /// recursive schema resolves to a finite plan.
    plans: HashMap<(SmolStr, SmolStr), Arc<RecordPlan>>,
    /// The writer schema's named types, consulted by skip steps.
    writer_names: Arc<HashMap<SmolStr, Node>>,
    /// The reader schema's named types, consulted by leaf reads.
    reader_names: Arc<HashMap<SmolStr, Node>>,
}

/// One step of a compiled plan.
#[derive(Debug)]
enum Op {
    /// Decode the writer's wire shape and present it as the reader's node.
    ///
    /// This is both the identity read and every legal promotion: the wire
    /// says what bytes to consume, the node says what value they become.
    Leaf { from: Wire, reader: Node },
    /// Decode a record through its field plan.
    Record(Arc<RecordPlan>),
    /// Decode a record through the plan registered for this name pair, which
    /// is how a recursive record refers to its own plan.
    RecordRef(SmolStr, SmolStr),
    /// Decode an array of one resolved item plan.
    Array(Box<Op>),
    /// Decode a map of one resolved value plan.
    Map(Box<Op>),
    /// Decode an enum through its symbol mapping.
    Enum(Arc<EnumPlan>),
    /// Read the writer's union branch index, then run that branch's plan.
    FromUnion(Arc<[Op]>),
    /// A branch the reader cannot accept: an error deferred until a datum
    /// actually takes it.
    Fail(SmolStr),
}

/// The compiled plan for one (writer record, reader record) pair.
#[derive(Debug)]
struct RecordPlan {
    /// One step per writer field, in encoding order.
    steps: Vec<Step>,
    /// Reader defaults for fields the writer never wrote: slot and value.
    fills: Vec<(usize, Scalar)>,
    /// The reader's field names, in reader order, for assembling the row.
    reader_fields: Arc<[SmolStr]>,
}

/// What to do with one writer field.
#[derive(Debug)]
enum Step {
    /// Decode into the reader slot at `index`.
    Decode { index: usize, op: Op },
    /// Skip the writer's bytes without decoding them.
    Skip(Node),
}

/// The compiled symbol mapping for one (writer enum, reader enum) pair.
#[derive(Debug)]
struct EnumPlan {
    /// Reader symbol per writer index; `None` falls back to the default.
    mapping: Vec<Option<SmolStr>>,
    /// The reader's declared default symbol, when it has one.
    default: Option<SmolStr>,
    /// The writer's symbols, for naming the failure.
    writer_symbols: Arc<[SmolStr]>,
}

impl Resolution {
    /// Compile the plan that reads data written with `writer` as `reader`.
    ///
    /// # Errors
    ///
    /// Returns an error when the schemas do not resolve, naming the writer
    /// construct, the reader construct, and the field path where they meet.
    pub fn from_schemas(writer: &Schema, reader: &Schema) -> Result<Self> {
        let mut builder = Builder {
            writer_names: &writer.names,
            reader_names: &reader.names,
            plans: HashMap::new(),
        };
        let op = builder.resolve(&writer.node, &reader.node)?;
        let plans = builder
            .plans
            .into_iter()
            .filter_map(|(key, plan)| plan.map(|plan| (key, plan)))
            .collect();
        Ok(Self {
            op,
            plans,
            writer_names: writer.names.clone(),
            reader_names: reader.names.clone(),
        })
    }

    /// Decode one datum through the plan.
    pub(crate) fn decode(
        &self,
        cursor: &mut Cursor<'_>,
        limits: Limits,
        budget: &mut usize,
    ) -> Result<Scalar> {
        let runner = Runner {
            resolution: self,
            reader: DatumCodec {
                names: &self.reader_names,
                limits,
            },
            writer: DatumCodec {
                names: &self.writer_names,
                limits,
            },
        };
        runner.run(&self.op, cursor, 0, budget)
    }
}

/// The plan compiler.
struct Builder<'schemas> {
    /// The writer schema's named types.
    writer_names: &'schemas HashMap<SmolStr, Node>,
    /// The reader schema's named types.
    reader_names: &'schemas HashMap<SmolStr, Node>,
    /// Plans per record pair; `None` marks one being built, which is what a
    /// recursive reference resolves to.
    plans: HashMap<(SmolStr, SmolStr), Option<Arc<RecordPlan>>>,
}

impl Builder<'_> {
    /// Resolve one writer node against one reader node.
    fn resolve(&mut self, writer: &Node, reader: &Node) -> Result<Op> {
        // References resolve to what they name before any matching happens.
        if let Node::Ref(name) = writer {
            let target = self.writer_names.get(name).cloned().ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a declared Avro type named {name:?}"
                ))
            })?;
            return self.resolve(&target, reader);
        }
        if let Node::Ref(name) = reader {
            let target = self.reader_names.get(name).cloned().ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a declared Avro type named {name:?}"
                ))
            })?;
            return self.resolve(writer, &target);
        }

        // A writer union is resolved branch by branch; a branch the reader
        // cannot take becomes an error deferred until data takes it.
        if let Node::Union(branches) = writer {
            let mut ops = Vec::with_capacity(branches.len());
            for branch in branches.iter() {
                let saved = self.plans.clone();
                ops.push(match self.resolve(branch, reader) {
                    Ok(op) => op,
                    Err(error) => {
                        // A failed attempt may have registered partial plans;
                        // restore the registry so the failure stays inside
                        // this branch instead of poisoning later pairs.
                        self.plans = saved;
                        Op::Fail(SmolStr::new(error.to_string()))
                    }
                });
            }
            return Ok(Op::FromUnion(ops.into()));
        }

        // A reader union accepts the first branch the writer resolves into;
        // the value model carries the branch value directly, so no wrapper.
        if let Node::Union(branches) = reader {
            let saved = self.plans.clone();
            // A branch whose fullname (or alias) matches the writer exactly is
            // tried before every other branch, so the lenient bare-name
            // comparison can never shadow an exact match later in the union.
            let mut order: Vec<&Node> = Vec::with_capacity(branches.len());
            if writer.name().is_some() {
                for branch in branches.iter() {
                    if self.reader_names_match_exactly(writer, branch) {
                        order.push(branch);
                    }
                }
            }
            for branch in branches.iter() {
                if !order.iter().any(|kept| std::ptr::eq(*kept, branch)) {
                    order.push(branch);
                }
            }
            for branch in order {
                match self.resolve(writer, branch) {
                    Ok(op) => return Ok(op),
                    // A failed attempt may have registered partial plans;
                    // restore the registry before trying the next branch.
                    Err(_) => self.plans = saved.clone(),
                }
            }
            return Err(invalid(format_smolstr!(
                "expected a reader union branch resolving Avro {}, got none of {} branches",
                writer.kind(),
                branches.len()
            )));
        }

        match (writer, reader) {
            (Node::Record(from), Node::Record(to)) => {
                if !names_match(writer, reader) {
                    return Err(no_match(writer, reader));
                }
                let key = (from.name.clone(), to.name.clone());
                if self.plans.contains_key(&key) {
                    return Ok(Op::RecordRef(key.0, key.1));
                }
                self.plans.insert(key.clone(), None);
                let plan = self.record_plan(from, to)?;
                self.plans.insert(key.clone(), Some(plan.clone()));
                Ok(Op::Record(plan))
            }
            (Node::Enum(from), Node::Enum(to)) => {
                if !names_match(writer, reader) {
                    return Err(no_match(writer, reader));
                }
                let mapping = from
                    .symbols
                    .iter()
                    .map(|symbol| {
                        to.symbols
                            .iter()
                            .find(|candidate| *candidate == symbol)
                            .cloned()
                    })
                    .collect();
                Ok(Op::Enum(Arc::new(EnumPlan {
                    mapping,
                    default: enum_default(to),
                    writer_symbols: from.symbols.clone(),
                })))
            }
            (Node::Array(from), Node::Array(to)) => {
                Ok(Op::Array(Box::new(self.resolve(from, to)?)))
            }
            (Node::Map(from), Node::Map(to)) => Ok(Op::Map(Box::new(self.resolve(from, to)?))),
            _ => self.leaf(writer, reader),
        }
    }

    /// Resolve two leaf nodes: an identity read or a legal promotion.
    fn leaf(&self, writer: &Node, reader: &Node) -> Result<Op> {
        let from = writer.wire();
        let to = reader.wire();
        let legal = match (from, to) {
            // The same wire shape reads directly as the reader's node.
            (Wire::Fixed(a), Wire::Fixed(b)) => a == b && names_match(writer, reader),
            (a, b) if a == b => !matches!(a, Wire::Record | Wire::Enum | Wire::Array | Wire::Map),
            // The promotion matrix: int -> long, float, double; long -> float,
            // double; float -> double; string <-> bytes.
            (Wire::Int, Wire::Long | Wire::Float | Wire::Double)
            | (Wire::Long, Wire::Float | Wire::Double)
            | (Wire::Float, Wire::Double)
            | (Wire::String, Wire::Bytes)
            | (Wire::Bytes, Wire::String) => true,
            _ => false,
        };
        if !legal {
            return Err(no_match(writer, reader));
        }
        Ok(Op::Leaf {
            from,
            reader: reader.clone(),
        })
    }

    /// Return whether a reader branch names the writer exactly.
    ///
    /// Exact means fullname equality or a reader alias naming the writer's
    /// fullname - never the bare-name fallback [`names_match`] allows.
    fn reader_names_match_exactly(&self, writer: &Node, branch: &Node) -> bool {
        let resolved;
        let target = match branch {
            Node::Ref(name) => match self.reader_names.get(name) {
                Some(node) => {
                    resolved = node.clone();
                    &resolved
                }
                None => return false,
            },
            other => other,
        };
        let (Some(from), Some(to)) = (writer.name(), target.name()) else {
            return false;
        };
        from == to || target.aliases().contains(from)
    }

    /// Compile the field plan for one record pair.
    fn record_plan(
        &mut self,
        writer: &super::schema::RecordType,
        reader: &super::schema::RecordType,
    ) -> Result<Arc<RecordPlan>> {
        let mut steps = Vec::with_capacity(writer.fields.len());
        let mut matched = vec![false; reader.fields.len()];
        for from in &writer.fields {
            // A reader field matches by name, or by declaring the writer's
            // name among its aliases; an exact name wins over an alias
            // wherever both appear.
            let slot = reader
                .fields
                .iter()
                .position(|to| to.name == from.name)
                .or_else(|| {
                    reader
                        .fields
                        .iter()
                        .position(|to| to.aliases.contains(&from.name))
                });
            match slot {
                Some(index) => {
                    matched[index] = true;
                    let op = self
                        .resolve(&from.schema, &reader.fields[index].schema)
                        .map_err(|error| locate(error, &reader.name, &reader.fields[index].name))?;
                    steps.push(Step::Decode { index, op });
                }
                None => steps.push(Step::Skip(from.schema.clone())),
            }
        }

        let mut fills = Vec::new();
        for (index, to) in reader.fields.iter().enumerate() {
            if matched[index] {
                continue;
            }
            let default = to.default.as_ref().ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected the writer to carry {}.{} or the reader to declare a default",
                    reader.name,
                    to.name
                ))
            })?;
            let value = default_value(&to.schema, default, self.reader_names)
                .map_err(|error| locate(error, &reader.name, &to.name))?;
            fills.push((index, value));
        }

        Ok(Arc::new(RecordPlan {
            steps,
            fills,
            reader_fields: reader
                .fields
                .iter()
                .map(|field| field.name.clone())
                .collect(),
        }))
    }
}

/// The plan executor.
struct Runner<'plan> {
    /// The compiled plan and its registries.
    resolution: &'plan Resolution,
    /// Decode context over the reader's named types.
    reader: DatumCodec<'plan>,
    /// Skip context over the writer's named types.
    writer: DatumCodec<'plan>,
}

impl Runner<'_> {
    /// Execute one operation.
    fn run(
        &self,
        op: &Op,
        cursor: &mut Cursor<'_>,
        depth: usize,
        budget: &mut usize,
    ) -> Result<Scalar> {
        match op {
            Op::Leaf { from, reader } => {
                self.reader.spend(budget)?;
                read_leaf(*from, reader, cursor)
            }
            Op::Record(plan) => self.run_record(plan, cursor, depth, budget),
            Op::RecordRef(from, to) => {
                let plan = self
                    .resolution
                    .plans
                    .get(&(from.clone(), to.clone()))
                    .ok_or_else(|| {
                        invalid(format_smolstr!(
                            "expected a compiled plan for {from} as {to}"
                        ))
                    })?;
                let depth = self.reader.descend(depth)?;
                self.run_record(plan, cursor, depth, budget)
            }
            Op::Array(items) => {
                self.reader.spend(budget)?;
                let depth = self.reader.descend(depth)?;
                let mut values = Vec::new();
                loop {
                    let (count, _) = block_count(cursor)?;
                    if count == 0 {
                        break;
                    }
                    for _ in 0..count {
                        values.push(self.run(items, cursor, depth, budget)?);
                    }
                }
                Ok(Scalar::from_sequence(values))
            }
            Op::Map(values) => {
                self.reader.spend(budget)?;
                let depth = self.reader.descend(depth)?;
                let mut entries = Vec::new();
                loop {
                    let (count, _) = block_count(cursor)?;
                    if count == 0 {
                        break;
                    }
                    for _ in 0..count {
                        self.reader.spend(budget)?;
                        let key = std::str::from_utf8(cursor.bytes()?).map_err(|error| {
                            codec(
                                cursor.position,
                                format_smolstr!("expected UTF-8 in an Avro map key, got {error}"),
                            )
                        })?;
                        entries.push((Scalar::from(key), self.run(values, cursor, depth, budget)?));
                    }
                }
                Scalar::from_mapping(entries)
            }
            Op::Enum(plan) => {
                self.reader.spend(budget)?;
                let index = cursor.long()?;
                let slot = usize::try_from(index)
                    .ok()
                    .and_then(|index| plan.mapping.get(index))
                    .ok_or_else(|| {
                        codec(
                            cursor.position,
                            format_smolstr!(
                                "expected an Avro enum index below {}, got {index}",
                                plan.mapping.len()
                            ),
                        )
                    })?;
                let symbol = slot.as_ref().or(plan.default.as_ref()).ok_or_else(|| {
                    codec(
                        cursor.position,
                        format_smolstr!(
                            "expected a reader symbol or default for {:?}",
                            plan.writer_symbols
                                .get(index.unsigned_abs() as usize)
                                .map_or("", |symbol| symbol.as_str())
                        ),
                    )
                })?;
                Ok(Scalar::from(symbol.clone()))
            }
            Op::FromUnion(branches) => {
                self.reader.spend(budget)?;
                let depth = self.reader.descend(depth)?;
                let index = cursor.long()?;
                let branch = usize::try_from(index)
                    .ok()
                    .and_then(|index| branches.get(index))
                    .ok_or_else(|| {
                        codec(
                            cursor.position,
                            format_smolstr!(
                                "expected an Avro union branch below {}, got {index}",
                                branches.len()
                            ),
                        )
                    })?;
                self.run(branch, cursor, depth, budget)
            }
            Op::Fail(reason) => Err(codec(cursor.position, reason.clone())),
        }
    }

    /// Execute one record plan.
    fn run_record(
        &self,
        plan: &RecordPlan,
        cursor: &mut Cursor<'_>,
        depth: usize,
        budget: &mut usize,
    ) -> Result<Scalar> {
        self.reader.spend(budget)?;
        let depth = self.reader.descend(depth)?;
        let mut values: Vec<Scalar> = vec![Scalar::Null; plan.reader_fields.len()];
        for (index, default) in &plan.fills {
            values[*index] = default.clone();
        }
        for step in &plan.steps {
            match step {
                Step::Decode { index, op } => {
                    values[*index] = self.run(op, cursor, depth, budget)?;
                }
                Step::Skip(node) => self.writer.skip(node, cursor, depth, budget)?,
            }
        }
        Scalar::from_record(
            plan.reader_fields
                .iter()
                .zip(values)
                .map(|(name, value)| (name.clone(), value)),
        )
    }
}

/// Decode one leaf: read the writer's wire shape, present the reader's value.
fn read_leaf(from: Wire, reader: &Node, cursor: &mut Cursor<'_>) -> Result<Scalar> {
    // Read the raw writer value first; every legal (wire, reader) pairing was
    // proven when the plan was built.
    Ok(match reader {
        Node::Null => Scalar::Null,
        Node::Boolean => Scalar::from(cursor.take(1)?.first().is_some_and(|byte| *byte != 0)),
        Node::Int => Scalar::from(cursor.int()?),
        Node::Long => Scalar::from(read_integer(from, cursor)?),
        Node::Float => {
            let value = match from {
                Wire::Float => cursor.float()?,
                _ => read_integer(from, cursor)? as f32,
            };
            Scalar::from(value)
        }
        Node::Double => {
            let value = match from {
                Wire::Double => cursor.double()?,
                Wire::Float => f64::from(cursor.float()?),
                _ => read_integer(from, cursor)? as f64,
            };
            Scalar::from(value)
        }
        Node::Bytes => Scalar::from(cursor.bytes()?),
        Node::String | Node::Uuid => Scalar::from(SmolStr::new(cursor.string()?)),
        Node::Date => Scalar::date32(cursor.int()?),
        Node::TimeMillis => Scalar::time32(cursor.int()?, TimeUnit::Millisecond, Timezone::NAIVE)?,
        Node::TimeMicros => Scalar::time64(
            read_integer(from, cursor)?,
            TimeUnit::Microsecond,
            Timezone::NAIVE,
        )?,
        Node::TimestampMillis => Scalar::datetime64(
            read_integer(from, cursor)?,
            TimeUnit::Millisecond,
            Timezone::UTC,
        )?,
        Node::TimestampMicros => Scalar::datetime64(
            read_integer(from, cursor)?,
            TimeUnit::Microsecond,
            Timezone::UTC,
        )?,
        Node::TimestampNanos => Scalar::datetime64(
            read_integer(from, cursor)?,
            TimeUnit::Nanosecond,
            Timezone::UTC,
        )?,
        Node::LocalTimestampMillis => Scalar::datetime64(
            read_integer(from, cursor)?,
            TimeUnit::Millisecond,
            Timezone::NAIVE,
        )?,
        Node::LocalTimestampMicros => Scalar::datetime64(
            read_integer(from, cursor)?,
            TimeUnit::Microsecond,
            Timezone::NAIVE,
        )?,
        Node::LocalTimestampNanos => Scalar::datetime64(
            read_integer(from, cursor)?,
            TimeUnit::Nanosecond,
            Timezone::NAIVE,
        )?,
        Node::Decimal(decimal) => {
            let bytes = match from {
                Wire::Fixed(size) => cursor.take(size)?,
                _ => cursor.bytes()?,
            };
            let unscaled = super::datum::decimal_from_bytes(bytes).ok_or_else(|| {
                codec(
                    cursor.position,
                    format_smolstr!(
                        "expected a decimal of at most 38 digits, got {} bytes",
                        bytes.len()
                    ),
                )
            })?;
            Scalar::d128(unscaled, decimal.scale as i8)
        }
        Node::Duration(_) | Node::UuidFixed(_) | Node::Fixed(_) => {
            let size = match from {
                Wire::Fixed(size) => size,
                _ => 0,
            };
            Scalar::from(cursor.take(size)?)
        }
        // Containers never reach a leaf op; the builder proved as much.
        other => {
            return Err(invalid(format_smolstr!(
                "expected a leaf reader schema, got {}",
                other.kind()
            )));
        }
    })
}

/// Read the writer's integer wire value, whichever width it was written at.
fn read_integer(from: Wire, cursor: &mut Cursor<'_>) -> Result<i64> {
    match from {
        Wire::Int => Ok(i64::from(cursor.int()?)),
        _ => cursor.long(),
    }
}

/// Return whether two named types match by fullname or alias.
///
/// The reader's aliases are consulted, as the specification says; when
/// neither fullname nor alias matches, the unqualified names are compared as
/// a last resort, because implementations disagree about whether a nested
/// record inherits its namespace and a byte-identical file should not become
/// unreadable over that disagreement.
fn names_match(writer: &Node, reader: &Node) -> bool {
    let (Some(from), Some(to)) = (writer.name(), reader.name()) else {
        return false;
    };
    if from == to || reader.aliases().iter().any(|alias| alias == from) {
        return true;
    }
    let bare = |name: &str| {
        name.rsplit_once('.')
            .map_or_else(|| SmolStr::new(name), |(_, bare)| SmolStr::new(bare))
    };
    bare(from) == bare(to)
}

/// Return the reader default an unmapped writer symbol falls back to.
fn enum_default(reader: &EnumType) -> Option<SmolStr> {
    reader
        .default
        .as_ref()
        .filter(|symbol| reader.symbols.iter().any(|candidate| candidate == *symbol))
        .cloned()
}

/// Report two schema nodes that do not resolve.
fn no_match(writer: &Node, reader: &Node) -> crate::Error {
    let name = |node: &Node| {
        node.name().map_or_else(
            || SmolStr::new_static(""),
            |name| format_smolstr!(" {name:?}"),
        )
    };
    invalid(format_smolstr!(
        "expected a reader type resolving Avro {}{}, got {}{}",
        writer.kind(),
        name(writer),
        reader.kind(),
        name(reader)
    ))
}

/// Locate a resolution failure at the record field it happened under.
fn locate(error: crate::Error, record: &str, field: &str) -> crate::Error {
    match error {
        crate::Error::Codec {
            format,
            position,
            reason,
        } => crate::Error::Codec {
            format,
            position,
            reason: format_smolstr!("{record}.{field}: {reason}"),
        },
        other => other,
    }
}

/// How deep a default walk may recurse.
///
/// A default is a small document literal inside the schema, not a data
/// stream, so this sits far below [`super::schema::MAX_SCHEMA_DEPTH`]: a
/// recursive type lets a hostile schema nest its default without limit, and
/// this walk descends through union and reference hops that cost calls
/// without costing document nesting.
const MAX_DEFAULT_DEPTH: usize = 64;

/// Convert a schema-declared JSON default into the value a reader fills with.
fn default_value(node: &Node, default: &Scalar, names: &HashMap<SmolStr, Node>) -> Result<Scalar> {
    default_value_at(node, default, names, 0)
}

/// [`default_value`], bounded like every other recursive walk in the codec.
fn default_value_at(
    node: &Node,
    default: &Scalar,
    names: &HashMap<SmolStr, Node>,
    depth: usize,
) -> Result<Scalar> {
    if depth >= MAX_DEFAULT_DEPTH {
        return Err(invalid(format_smolstr!(
            "expected a default at most {MAX_DEFAULT_DEPTH} levels deep"
        )));
    }
    let depth = depth + 1;
    Ok(match node {
        Node::Null => {
            if !default.is_null() {
                return Err(bad_default("null", default));
            }
            Scalar::Null
        }
        Node::Boolean => Scalar::from(
            default
                .as_bool()
                .ok_or_else(|| bad_default("boolean", default))?,
        ),
        Node::Int => Scalar::from(default_int(default, "int")?),
        Node::Long => Scalar::from(
            default
                .as_i64()
                .ok_or_else(|| bad_default("long", default))?,
        ),
        Node::Float => Scalar::from(
            default
                .as_f64()
                .or_else(|| default.as_i64().map(|value| value as f64))
                .ok_or_else(|| bad_default("float", default))? as f32,
        ),
        Node::Double => Scalar::from(
            default
                .as_f64()
                .or_else(|| default.as_i64().map(|value| value as f64))
                .ok_or_else(|| bad_default("double", default))?,
        ),
        // A bytes default is a JSON string whose code points are the bytes.
        Node::Bytes => Scalar::from(default_bytes(default)?),
        Node::Fixed(fixed) | Node::Duration(fixed) | Node::UuidFixed(fixed) => {
            let bytes = default_bytes(default)?;
            if bytes.len() != fixed.size {
                return Err(invalid(format_smolstr!(
                    "expected a fixed default of {} bytes, got {}",
                    fixed.size,
                    bytes.len()
                )));
            }
            Scalar::from(bytes)
        }
        Node::Decimal(decimal) => {
            let bytes = default_bytes(default)?;
            let unscaled = super::datum::decimal_from_bytes(&bytes)
                .ok_or_else(|| bad_default("decimal", default))?;
            Scalar::d128(unscaled, decimal.scale as i8)
        }
        Node::String | Node::Uuid | Node::Enum(_) => Scalar::from(SmolStr::new(
            default
                .as_str()
                .ok_or_else(|| bad_default(node.kind(), default))?,
        )),
        Node::Date => Scalar::date32(default_int(default, "date")?),
        Node::TimeMillis => Scalar::time32(
            default_int(default, "time-millis")?,
            TimeUnit::Millisecond,
            Timezone::NAIVE,
        )?,
        Node::TimeMicros => Scalar::time64(
            default
                .as_i64()
                .ok_or_else(|| bad_default("time-micros", default))?,
            TimeUnit::Microsecond,
            Timezone::NAIVE,
        )?,
        Node::TimestampMillis | Node::TimestampMicros | Node::TimestampNanos => {
            let count = default
                .as_i64()
                .ok_or_else(|| bad_default(node.kind(), default))?;
            let unit = match node {
                Node::TimestampMillis => TimeUnit::Millisecond,
                Node::TimestampNanos => TimeUnit::Nanosecond,
                _ => TimeUnit::Microsecond,
            };
            Scalar::datetime64(count, unit, Timezone::UTC)?
        }
        Node::LocalTimestampMillis | Node::LocalTimestampMicros | Node::LocalTimestampNanos => {
            let count = default
                .as_i64()
                .ok_or_else(|| bad_default(node.kind(), default))?;
            let unit = match node {
                Node::LocalTimestampMillis => TimeUnit::Millisecond,
                Node::LocalTimestampNanos => TimeUnit::Nanosecond,
                _ => TimeUnit::Microsecond,
            };
            Scalar::datetime64(count, unit, Timezone::NAIVE)?
        }
        // A union default always describes the union's first branch.
        Node::Union(branches) => match branches.first() {
            Some(first) => default_value_at(first, default, names, depth)?,
            None => return Err(bad_default("union", default)),
        },
        Node::Array(items) => {
            let values = default
                .as_sequence()
                .ok_or_else(|| bad_default("array", default))?;
            let mut converted = Vec::with_capacity(values.len());
            for value in values {
                converted.push(default_value_at(items, value, names, depth)?);
            }
            Scalar::from_sequence(converted)
        }
        Node::Map(values) => {
            let entries = default
                .as_mapping()
                .ok_or_else(|| bad_default("map", default))?;
            let mut converted = Vec::with_capacity(entries.len());
            for (key, value) in entries {
                converted.push((key.clone(), default_value_at(values, value, names, depth)?));
            }
            Scalar::from_mapping(converted)?
        }
        Node::Record(record) => {
            if default.as_record().is_none() && default.as_mapping().is_none() {
                return Err(bad_default("record", default));
            }
            let mut entries = Vec::with_capacity(record.fields.len());
            for field in &record.fields {
                let value = match default.get_key_str(&field.name) {
                    Some(value) => default_value_at(&field.schema, value, names, depth)?,
                    None => match &field.default {
                        Some(value) => default_value_at(&field.schema, value, names, depth)?,
                        None => {
                            return Err(invalid(format_smolstr!(
                                "expected a default for {}.{} inside the record default",
                                record.name,
                                field.name
                            )));
                        }
                    },
                };
                entries.push((field.name.clone(), value));
            }
            Scalar::from_record(entries)?
        }
        Node::Ref(name) => {
            let target = names
                .get(name)
                .cloned()
                .ok_or_else(|| bad_default("reference", default))?;
            default_value_at(&target, default, names, depth)?
        }
    })
}

/// Read an integer default that must fit 32 bits.
fn default_int(default: &Scalar, expected: &str) -> Result<i32> {
    default
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| bad_default(expected, default))
}

/// Read a bytes default: a JSON string whose code points are the bytes.
fn default_bytes(default: &Scalar) -> Result<Vec<u8>> {
    let text = default
        .as_str()
        .ok_or_else(|| bad_default("bytes", default))?;
    let mut bytes = Vec::with_capacity(text.len());
    for character in text.chars() {
        let code = character as u32;
        if code > 0xFF {
            return Err(bad_default("bytes", default));
        }
        bytes.push(code as u8);
    }
    Ok(bytes)
}

/// Report a default that does not fit the schema it defaults.
fn bad_default(expected: &str, default: &Scalar) -> crate::Error {
    invalid(format_smolstr!(
        "expected an Avro {expected} default, got {}",
        default.kind()
    ))
}
