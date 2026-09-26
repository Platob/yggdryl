//! [`MarketView`]: the named readings of a `marketdata` stream, each one
//! [`Plan`] over the rows [`MarketData::arrow_reader`] writes.
//!
//! A view is not a second reader: it is a plan built structurally - columns,
//! literals, a membership test, a selector and an ordering - and applied by
//! the one expression engine, so what a view publishes is exactly what its
//! plan's text says and the text reads back as the same plan. Every view
//! keeps the rows of the kinds it names and drops the nested columns it does
//! not read; the composite leaves are laid flat by `unnest`, a trade's
//! executions one row each and a book's two sides one row each.
//!
//! A *lift* is a [`FieldPath`] into the root row appended to the view's
//! projections, so a fact kept inside a nested column - an identifier in
//! `securityids`, say - becomes a column of its own: `securityids['ISIN'] as
//! isin` reads the key exactly as it is stored, a row without it reads null,
//! and a path naming a column the root does not hold is refused where the
//! plan binds.

use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::arrow::{ASKSIDE, BIDSIDE, DELTAS, EXECUTIONS, KIND, LIMITS, LIVE, SNAPSHOT_PARTITIONS};
use super::{EventColumn, MarketData, MarketKind};
use crate::arrow::BatchReader;
use crate::expression::{FieldPath, Function, Ordering, Plan, Projection, Selector, Term};
use crate::{Error, Result};

/// Every nested column of the root row: what a flat view drops.
const NESTED: [&str; 7] = [
    EXECUTIONS,
    BIDSIDE,
    ASKSIDE,
    SNAPSHOT_PARTITIONS,
    LIVE,
    DELTAS,
    LIMITS,
];

/// One named reading of a `marketdata` stream.
///
/// | View | Rows | Columns |
/// | --- | --- | --- |
/// | `orders`, `quotes`, `executions` | the undated and dated leaves of that operation kind | every flat root column |
/// | `trades` | one per execution of a trade | the trade's flat columns, then `execution.<column>` per execution column |
/// | `book_sides` | two per book, bid then ask | `currunix`, `snapunix`, then `side.<column>` per side column |
/// | `books` | one per book | every root column but the executions and a book side's own nested rows |
/// | `lifecycle` | every leaf of one `crosscode`, ordered by `currunix`, tied instants in arrival order | every flat root column |
///
/// Lifts are appended after the view's own columns in every view.
///
/// ```
/// use yggdryl::graph::{MarketData, MarketView};
///
/// # fn main() -> yggdryl::Result<()> {
/// let view = MarketView::read("Trades", None)?;
/// assert_eq!(view, MarketView::Trades);
/// let plan = MarketData::plan(&view, &["securityids['ISIN'] as isin".parse()?])?;
/// assert!(plan.to_string().starts_with("select * exclude (executions, "));
/// assert_eq!(plan.to_string().parse::<yggdryl::Plan>()?, plan);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum MarketView {
    /// Orders, undated and dated.
    Orders,
    /// Quotes, undated and dated.
    Quotes,
    /// Executions, undated and dated.
    Executions,
    /// One row per execution of every trade, the trade's columns beside it.
    Trades,
    /// One row per side of every book, bid then ask.
    BookSides,
    /// One row per book, its sides and partitions kept nested.
    Books,
    /// Every leaf of one element's chain, in the order it happened: ordered
    /// by `currunix`, the leaves that share an instant kept in the order the
    /// stream states them, because the ordering is stable.
    ///
    /// The ordering collects the stream it is applied to: the bound on what
    /// it holds is the one chain the `crosscode` names, after the `where`
    /// has kept it.
    Lifecycle {
        /// The `crosscode` every row of the chain states.
        crosscode: SmolStr,
    },
}

impl MarketView {
    /// Every view's spelling, in declaration order: the enumeration both
    /// bindings list.
    pub const ALL: [&'static str; 7] = [
        "orders",
        "quotes",
        "executions",
        "trades",
        "book_sides",
        "books",
        "lifecycle",
    ];

    /// The view's spelling.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Orders => "orders",
            Self::Quotes => "quotes",
            Self::Executions => "executions",
            Self::Trades => "trades",
            Self::BookSides => "book_sides",
            Self::Books => "books",
            Self::Lifecycle { .. } => "lifecycle",
        }
    }

    /// The view a spelling names, ignoring ASCII case, with the `crosscode`
    /// the lifecycle view follows.
    ///
    /// # Errors
    ///
    /// Returns an error for a spelling [`Self::ALL`] does not hold, for
    /// `lifecycle` without a `crosscode`, and for any other view given one.
    pub fn read(text: &str, crosscode: Option<&str>) -> Result<Self> {
        let Some(spelling) = Self::ALL
            .iter()
            .find(|spelling| spelling.eq_ignore_ascii_case(text))
        else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.view"),
                reason: crate::text::expected_got(
                    format_args!("one of {}", Self::ALL.join(", ")),
                    format_args!("{:?}", crate::text::elide_to(text, 64)),
                ),
            });
        };
        let view = match *spelling {
            "orders" => Self::Orders,
            "quotes" => Self::Quotes,
            "executions" => Self::Executions,
            "trades" => Self::Trades,
            "book_sides" => Self::BookSides,
            "books" => Self::Books,
            _ => {
                let Some(crosscode) = crosscode else {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new_static("$.crosscode"),
                        reason: SmolStr::new_static(
                            "expected the crosscode of the chain a lifecycle view follows, got none",
                        ),
                    });
                };
                return Ok(Self::Lifecycle {
                    crosscode: SmolStr::new(crosscode),
                });
            }
        };
        if let Some(crosscode) = crosscode {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.crosscode"),
                reason: format_smolstr!(
                    "expected a crosscode only for the lifecycle view, got {:?} for `{}`",
                    crate::text::elide_to(crosscode, 64),
                    view.as_str()
                ),
            });
        }
        Ok(view)
    }
}

impl std::fmt::Display for MarketView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// `kind in (...)`: the rows of these leaves.
fn kinds(kinds: &[MarketKind]) -> Term {
    Term::column(KIND).is_in(kinds.iter().map(|kind| Term::literal(kind.as_str())))
}

/// `unnest(<serie>) as <name>`: the projection that lays a serie flat.
fn unnested(serie: Term, name: &str) -> Projection {
    Projection::aliased(Term::call(Function::Unnest, [serie]), name)
}

impl MarketData {
    /// The plan one view is over a `marketdata` stream, with `lifts`
    /// appended after its own columns.
    ///
    /// Built structurally, never from text, and its text reads back as the
    /// same plan. [`MarketView`] lists what each view keeps.
    ///
    /// # Errors
    ///
    /// Returns an error when a section cannot be set, which the views this
    /// crate builds never cause.
    pub fn plan(view: &MarketView, lifts: &[FieldPath]) -> Result<Plan> {
        let flat = || Selector::all_except(NESTED);
        let (mut selector, filter) = match view {
            MarketView::Orders => (flat(), kinds(&[MarketKind::Order, MarketKind::OrderEvent])),
            MarketView::Quotes => (flat(), kinds(&[MarketKind::Quote, MarketKind::QuoteEvent])),
            MarketView::Executions => (
                flat(),
                kinds(&[MarketKind::Execution, MarketKind::ExecutionEvent]),
            ),
            MarketView::Trades => (
                flat().with_projection(unnested(Term::column(EXECUTIONS), "execution")),
                Term::column(KIND).eq(Term::literal(MarketKind::TradeEvent.as_str())),
            ),
            MarketView::BookSides => (
                Selector::new([
                    Projection::column(EventColumn::CurrUnix.name()),
                    Projection::column(EventColumn::SnapUnix.name()),
                    unnested(
                        Term::Serie(Arc::from([Term::column(BIDSIDE), Term::column(ASKSIDE)])),
                        "side",
                    ),
                ]),
                Term::column(KIND).eq(Term::literal(MarketKind::BookEvent.as_str())),
            ),
            // A book keeps its sides and partitions; its executions and a
            // book side's own nested rows are not a book's.
            MarketView::Books => (
                Selector::all_except([EXECUTIONS, LIVE, DELTAS, LIMITS]),
                Term::column(KIND).eq(Term::literal(MarketKind::BookEvent.as_str())),
            ),
            MarketView::Lifecycle { crosscode } => (
                flat(),
                Term::column(EventColumn::CrossCode.name()).eq(Term::literal(crosscode.as_str())),
            ),
        };
        for lift in lifts {
            selector = selector.with_projection(Projection::from(lift.clone()));
        }
        let plan = Plan::new().select(selector)?.filter(filter)?;
        Ok(match view {
            MarketView::Lifecycle { .. } => {
                plan.order_by([Ordering::asc(Term::column(EventColumn::CurrUnix.name()))])
            }
            _ => plan,
        })
    }

    /// One view over a `marketdata` stream: exactly
    /// [`Self::plan`]`(view, lifts)` applied to `reader`, bound once
    /// against the reader's schema.
    ///
    /// # Errors
    ///
    /// Returns an error when the plan does not bind against the reader's
    /// schema - a lift naming a column it does not hold - or cannot be
    /// built.
    pub fn apply_view(
        view: &MarketView,
        lifts: &[FieldPath],
        reader: BatchReader,
    ) -> Result<BatchReader> {
        Self::plan(view, lifts)?.apply_arrow_reader(reader)
    }
}
