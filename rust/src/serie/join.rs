//! The join verbs: [`Serie::join_with`], [`ChunkedSerie::join_with`] and
//! [`StreamChunkedSerie::join_with`], each one door onto the engine in
//! `crate::join`, which states the semantics.
//!
//! A held serie answers a held serie - its output batches joined once; a
//! chunked serie answers its output batches kept apart; a stream answers a
//! stream, one probe batch at a time, nothing of the probe collected.

use std::sync::Arc;

use crate::expression::IntoJoinKeys;
use crate::join::{JoinKind, JoinOptions, join};
use crate::{ChunkedSerie, Result, Serie, StreamChunkedSerie};

impl Serie {
    /// This serie joined with `other` on `by`, under `how`: one record
    /// column of the matched rows - the left columns, then the right, a key
    /// stated as one bare column on both sides once under the left name, a
    /// colliding right name suffixed, a side the kind makes optional
    /// nullable - the rules [`JoinKind`] and [`JoinOptions`] state.
    ///
    /// `by` is any [`IntoJoinKeys`]: `"id"`, `"venue = market"`, a list, a
    /// pair of selectors. A non-record serie keys as the one child of a `row`
    /// record, under its own name, and the output root is named after it.
    /// The build side is held - the smaller by [`Self::memory_size`] unless
    /// [`JoinOptions::build`] says - and every output batch settles under
    /// [`JoinOptions::spill`].
    ///
    /// ```
    /// use yggdryl::{DataType, Field, JoinKind, JoinOptions, Scalar, Serie, StructType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let trades = Serie::from_scalars(
    ///     DataType::from(StructType::from_fields([
    ///         DataType::Int64.required_field("id"),
    ///         DataType::utf8().required_field("venue"),
    ///     ])?)
    ///     .required_field("trade"),
    ///     [
    ///         Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("XNAS")]),
    ///         Scalar::from_sequence([Scalar::from(2_i64), Scalar::from("XNYS")]),
    ///     ],
    /// )?;
    /// let venues = Serie::from_scalars(
    ///     DataType::from(StructType::from_fields([
    ///         DataType::utf8().required_field("venue"),
    ///         DataType::utf8().required_field("city"),
    ///     ])?)
    ///     .required_field("venue"),
    ///     [Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from("New York")])],
    /// )?;
    /// let joined = trades.join_with(&venues, "venue", JoinKind::Left, &JoinOptions::new())?;
    /// let names: Vec<&str> = joined.require_field()?.fields().iter().map(Field::name).collect();
    /// assert_eq!(names, ["id", "venue", "city"]);
    /// assert_eq!(joined.len(), 2);
    /// assert_eq!(joined.scalar(1)?, Scalar::from_sequence([Scalar::from(2_i64), Scalar::from("XNYS"), Scalar::Null]));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error, before any row is read, for a run on either side,
    /// an empty key list, a key term reaching no column of its side, an
    /// `unnest` or an aggregate in a key, and a key pair whose two terms
    /// share no datatype, naming both; then the spill's refusal of its
    /// folder.
    pub fn join_with(
        &self,
        other: &Self,
        by: impl IntoJoinKeys,
        how: JoinKind,
        options: &JoinOptions,
    ) -> Result<Self> {
        let output = join(self.clone(), other.clone(), by, how, options)?;
        let root = Arc::clone(output.root());
        let chunks = output.collect::<Result<Vec<Self>>>()?;
        // The one join is settled under the join's own bound, never the
        // process default first: a stated `NEVER` keeps the output resident.
        let joined = ChunkedSerie::from_landed(root, chunks).joined()?;
        options.settle(joined)
    }
}

impl ChunkedSerie {
    /// This chunked serie joined with `other` on `by`, under `how`: the
    /// output batches kept apart as chunks, each settled.
    ///
    /// [`Serie::join_with`] states the keys, the sides and the refusals.
    ///
    /// # Errors
    ///
    /// [`Serie::join_with`]'s.
    pub fn join_with(
        &self,
        other: &Self,
        by: impl IntoJoinKeys,
        how: JoinKind,
        options: &JoinOptions,
    ) -> Result<Self> {
        let output = join(
            Serie::from(self.clone()),
            Serie::from(other.clone()),
            by,
            how,
            options,
        )?;
        let root = Arc::clone(output.root());
        let chunks = output.collect::<Result<Vec<Serie>>>()?;
        Ok(Self::from_landed(root, chunks))
    }
}

impl StreamChunkedSerie {
    /// This stream joined with `other` - a held serie, a chunked one or a
    /// stream - on `by`, under `how`: a stream of the output, one probe
    /// batch joined at a time, the held side built and hashed first.
    ///
    /// A stream against a held side probes with the stream, so nothing of
    /// it is collected; two streams hold the right one. [`Serie::join_with`]
    /// states the keys and the refusals.
    ///
    /// # Errors
    ///
    /// [`Serie::join_with`]'s, raised before the first batch is pulled.
    pub fn join_with(
        self,
        other: impl Into<Serie>,
        by: impl IntoJoinKeys,
        how: JoinKind,
        options: &JoinOptions,
    ) -> Result<Self> {
        let output = join(Serie::from(self), other.into(), by, how, options)?;
        let root = Arc::clone(output.root());
        Ok(Self::from_landed_iter(
            root,
            output.map(|record| record.map_err(Into::into)),
        )?)
    }
}
