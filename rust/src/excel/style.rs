//! A cell's style: the `cellXfs` index its `s` attribute states, and what
//! that index resolves to.
//!
//! A cell names its display - font, fill, border, alignment and number
//! format - by one index into the styles part's `cellXfs` table. The cell
//! holds that index as it was read and writes it back as it holds it, so a
//! cell the file styled keeps its style through an edit of any other cell.
//! [`StyleSheet::style`](super::styles::StyleSheet::style) resolves an index
//! into the [`CellStyle`] values below: every fact of the entry, its font,
//! fill and border read out of the lists the entry points into.

use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

/// The `cellXfs` index a cell's `s` attribute states.
///
/// Sixteen bits hold every index Excel writes: its table stops at 64,000
/// cell formats, so a file stating an index past `65_535` is refused where
/// it is read, naming the cell. [`StyleId::DEFAULT`] is index `0`, what a
/// cell stating no `s` has.
///
/// ```
/// use yggdryl::excel::StyleId;
///
/// assert_eq!(StyleId::from_attribute("7")?.as_u16(), 7);
/// assert_eq!(StyleId::default(), StyleId::DEFAULT);
/// assert!(StyleId::from_attribute("65536").is_err());
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StyleId(u16);

impl StyleId {
    /// Index `0`: the style a cell stating no `s` is displayed with.
    pub const DEFAULT: Self = Self(0);

    /// The style at `cellXfs` index `index`.
    #[must_use]
    pub const fn new(index: u16) -> Self {
        Self(index)
    }

    /// The `cellXfs` index.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// The style an `s` attribute spells: a decimal index from `0` to
    /// `65_535`, surrounding whitespace ignored.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the text for anything else.
    pub fn from_attribute(value: &str) -> Result<Self> {
        value
            .trim()
            .parse::<u16>()
            .map(Self)
            .map_err(|_| Error::Parse {
                target: "cell style",
                position: 0,
                reason: format_smolstr!(
                    "expected a style index from 0 to {} in a cell's `s`, got {value:?}",
                    u16::MAX
                ),
            })
    }
}

impl From<u16> for StyleId {
    fn from(index: u16) -> Self {
        Self(index)
    }
}

impl From<StyleId> for u32 {
    fn from(style: StyleId) -> Self {
        Self::from(style.0)
    }
}

/// Declare a closed attribute vocabulary: the enum, its file spelling and
/// the reading of one.
macro_rules! vocabulary {
    (
        $(#[$meta:meta])*
        $name:ident, $target:literal, default $default:ident {
            $($(#[$variant_meta:meta])* $variant:ident => $spelling:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $(
                $(#[$variant_meta])*
                #[doc = concat!("`", $spelling, "`.")]
                $variant,
            )+
        }

        impl $name {
            /// Every member, in the schema's order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The value as the styles part spells it.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling,)+
                }
            }

            /// The member an attribute spells, exactly as the schema lists it.
            ///
            /// # Errors
            ///
            /// Returns [`Error::Parse`] naming the text and every spelling.
            pub fn from_attribute(value: &str) -> Result<Self> {
                match value.trim() {
                    $($spelling => Ok(Self::$variant),)+
                    other => Err(Error::Parse {
                        target: $target,
                        position: 0,
                        reason: format_smolstr!(
                            "expected one of {}, got {other:?}",
                            [$($spelling),+].join(", ")
                        ),
                    }),
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::$default
            }
        }
    };
}

vocabulary! {
    /// How a font underlines its text (`u`).
    Underline, "underline", default None {
        /// No underline.
        None => "none",
        /// One line, the default a bare `<u/>` states.
        Single => "single",
        /// Two lines.
        Double => "double",
        /// One line under the whole cell width, accounting style.
        SingleAccounting => "singleAccounting",
        /// Two lines under the whole cell width, accounting style.
        DoubleAccounting => "doubleAccounting",
    }
}

vocabulary! {
    /// Where a run sits against the baseline (`vertAlign`).
    VerticalRun, "vertical run", default Baseline {
        /// On the baseline.
        Baseline => "baseline",
        /// Raised.
        Superscript => "superscript",
        /// Lowered.
        Subscript => "subscript",
    }
}

vocabulary! {
    /// Which theme font a font follows (`scheme`).
    FontScheme, "font scheme", default None {
        /// None: the font is its own.
        None => "none",
        /// The theme's heading font.
        Major => "major",
        /// The theme's body font.
        Minor => "minor",
    }
}

vocabulary! {
    /// How a cell aligns its content across (`alignment@horizontal`).
    Horizontal, "horizontal alignment", default General {
        /// Text left, numbers right: what a cell stating nothing does.
        General => "general",
        /// Left.
        Left => "left",
        /// Centered.
        Center => "center",
        /// Right.
        Right => "right",
        /// Repeated to fill the cell.
        Fill => "fill",
        /// Justified.
        Justify => "justify",
        /// Centered across the empty cells to the right.
        CenterContinuous => "centerContinuous",
        /// Distributed.
        Distributed => "distributed",
    }
}

vocabulary! {
    /// How a cell aligns its content down (`alignment@vertical`).
    Vertical, "vertical alignment", default Bottom {
        /// Top.
        Top => "top",
        /// Centered.
        Center => "center",
        /// Bottom: what a cell stating nothing does.
        Bottom => "bottom",
        /// Justified.
        Justify => "justify",
        /// Distributed.
        Distributed => "distributed",
    }
}

vocabulary! {
    /// The line one edge of a border draws (`style`).
    BorderStyle, "border style", default None {
        /// No line.
        None => "none",
        /// Thin.
        Thin => "thin",
        /// Medium.
        Medium => "medium",
        /// Dashed.
        Dashed => "dashed",
        /// Dotted.
        Dotted => "dotted",
        /// Thick.
        Thick => "thick",
        /// Double.
        Double => "double",
        /// Hairline.
        Hair => "hair",
        /// Medium dashed.
        MediumDashed => "mediumDashed",
        /// Dash dot.
        DashDot => "dashDot",
        /// Medium dash dot.
        MediumDashDot => "mediumDashDot",
        /// Dash dot dot.
        DashDotDot => "dashDotDot",
        /// Medium dash dot dot.
        MediumDashDotDot => "mediumDashDotDot",
        /// Slanted dash dot.
        SlantDashDot => "slantDashDot",
    }
}

vocabulary! {
    /// The pattern a fill paints (`patternFill@patternType`).
    PatternType, "fill pattern", default None {
        /// No fill.
        None => "none",
        /// Solid, in the foreground colour.
        Solid => "solid",
        /// Medium gray.
        MediumGray => "mediumGray",
        /// Dark gray.
        DarkGray => "darkGray",
        /// Light gray.
        LightGray => "lightGray",
        /// Dark horizontal stripes.
        DarkHorizontal => "darkHorizontal",
        /// Dark vertical stripes.
        DarkVertical => "darkVertical",
        /// Dark diagonal stripes down.
        DarkDown => "darkDown",
        /// Dark diagonal stripes up.
        DarkUp => "darkUp",
        /// Dark grid.
        DarkGrid => "darkGrid",
        /// Dark trellis.
        DarkTrellis => "darkTrellis",
        /// Light horizontal stripes.
        LightHorizontal => "lightHorizontal",
        /// Light vertical stripes.
        LightVertical => "lightVertical",
        /// Light diagonal stripes down.
        LightDown => "lightDown",
        /// Light diagonal stripes up.
        LightUp => "lightUp",
        /// Light grid.
        LightGrid => "lightGrid",
        /// Light trellis.
        LightTrellis => "lightTrellis",
        /// 12.5% gray, the second fill every styles part reserves.
        Gray125 => "gray125",
        /// 6.25% gray.
        Gray0625 => "gray0625",
    }
}

/// A colour as a styles part states one.
///
/// The theme and indexed forms are references, resolved against the
/// workbook's theme or palette where the colour is displayed; `tint`
/// lightens (above zero) or darkens (below) the colour it names, from
/// `-1.0` to `1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Color {
    /// `rgb`: an ARGB value, `0xFFFF0000` opaque red.
    Rgb(u32),
    /// `theme`: an index into the theme's colour scheme.
    Theme {
        /// The scheme index.
        index: u8,
        /// The tint, `0.0` for none.
        tint: f64,
    },
    /// `indexed`: an index into the workbook's palette.
    Indexed {
        /// The palette index.
        index: u8,
        /// The tint, `0.0` for none.
        tint: f64,
    },
    /// `auto`: the system's automatic colour.
    Auto,
}

/// A font (`<font>`).
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    /// The typeface, `Calibri`.
    pub name: SmolStr,
    /// The size in points.
    pub size: f64,
    /// `b`.
    pub bold: bool,
    /// `i`.
    pub italic: bool,
    /// `u`.
    pub underline: Underline,
    /// `strike`.
    pub strike: bool,
    /// `color`, `None` where the font states none.
    pub color: Option<Color>,
    /// `vertAlign`.
    pub vertical: VerticalRun,
    /// `family`: the font family class, `2` for a sans serif.
    pub family: Option<u8>,
    /// `scheme`: the theme font the font follows.
    pub scheme: Option<FontScheme>,
    /// `charset`: the character set the font is drawn in.
    pub charset: Option<u8>,
    /// `outline`.
    pub outline: bool,
    /// `shadow`.
    pub shadow: bool,
    /// `condense`.
    pub condense: bool,
    /// `extend`.
    pub extend: bool,
}

impl Default for Font {
    /// Excel's default font: Calibri, eleven points.
    fn default() -> Self {
        Self {
            name: SmolStr::new_static("Calibri"),
            size: 11.0,
            bold: false,
            italic: false,
            underline: Underline::None,
            strike: false,
            color: None,
            vertical: VerticalRun::Baseline,
            family: None,
            scheme: None,
            charset: None,
            outline: false,
            shadow: false,
            condense: false,
            extend: false,
        }
    }
}

/// A fill (`<fill>`).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Fill {
    /// No fill: a `patternFill` stating no pattern, or `none`.
    #[default]
    None,
    /// A pattern in two colours; `Solid` paints the foreground alone.
    Pattern {
        /// The pattern.
        pattern: PatternType,
        /// `fgColor`.
        foreground: Option<Color>,
        /// `bgColor`.
        background: Option<Color>,
    },
    /// A `gradientFill`, carried as the element's own bytes.
    Gradient(Arc<[u8]>),
}

/// One edge of a border.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edge {
    /// The line.
    pub style: BorderStyle,
    /// The line's colour, `None` for the automatic one.
    pub color: Option<Color>,
}

/// A border (`<border>`): its edges and diagonals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    /// `left`.
    pub left: Edge,
    /// `right`.
    pub right: Edge,
    /// `top`.
    pub top: Edge,
    /// `bottom`.
    pub bottom: Edge,
    /// `diagonal`, drawn as the two flags below say.
    pub diagonal: Edge,
    /// `vertical`: the inner vertical edges of a range.
    pub vertical: Edge,
    /// `horizontal`: the inner horizontal edges of a range.
    pub horizontal: Edge,
    /// `diagonalUp`.
    pub diagonal_up: bool,
    /// `diagonalDown`.
    pub diagonal_down: bool,
    /// `outline`: whether the edges apply to a range's outline.
    pub outline: bool,
}

impl Border {
    /// The border Excel inserts between two cells: exterior edges agree
    /// independently; the diagonal and both direction flags agree together.
    /// Excel16 build20430 confirmed these rules in 72 insertion cases.
    /// Inner OOXML edges and non-default outline behavior remain unobserved.
    pub(crate) fn inherited(self, following: Self) -> Option<Self> {
        if self.vertical != Edge::default()
            || following.vertical != Edge::default()
            || self.horizontal != Edge::default()
            || following.horizontal != Edge::default()
            || !self.outline
            || !following.outline
        {
            return None;
        }
        let same = |first: Edge, second: Edge| {
            if first == second {
                first
            } else {
                Edge::default()
            }
        };
        let diagonal = self.diagonal == following.diagonal
            && self.diagonal_up == following.diagonal_up
            && self.diagonal_down == following.diagonal_down;
        Some(Self {
            left: same(self.left, following.left),
            right: same(self.right, following.right),
            top: same(self.top, following.top),
            bottom: same(self.bottom, following.bottom),
            diagonal: if diagonal {
                self.diagonal
            } else {
                Edge::default()
            },
            diagonal_up: diagonal && self.diagonal_up,
            diagonal_down: diagonal && self.diagonal_down,
            ..Self::default()
        })
    }
}

impl Default for Border {
    /// No line anywhere, the edges applying to a range's outline as the
    /// schema's default says.
    fn default() -> Self {
        Self {
            left: Edge::default(),
            right: Edge::default(),
            top: Edge::default(),
            bottom: Edge::default(),
            diagonal: Edge::default(),
            vertical: Edge::default(),
            horizontal: Edge::default(),
            diagonal_up: false,
            diagonal_down: false,
            outline: true,
        }
    }
}

/// How a cell lays its content out (`<alignment>`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Alignment {
    /// `horizontal`.
    pub horizontal: Horizontal,
    /// `vertical`.
    pub vertical: Vertical,
    /// `textRotation`: degrees from 0 to 180, or 255 for stacked text.
    pub rotation: u8,
    /// `wrapText`.
    pub wrap: bool,
    /// `indent`, in steps of three spaces.
    pub indent: u8,
    /// `relativeIndent`.
    pub relative_indent: i32,
    /// `justifyLastLine`.
    pub justify_last_line: bool,
    /// `shrinkToFit`.
    pub shrink_to_fit: bool,
    /// `readingOrder`: 0 by context, 1 left to right, 2 right to left.
    pub reading_order: u8,
}

/// What a protected sheet does to a cell (`<protection>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Protection {
    /// `locked`: the cell cannot be edited while its sheet is protected.
    pub locked: bool,
    /// `hidden`: its formula is not shown while its sheet is protected.
    pub hidden: bool,
}

impl Default for Protection {
    /// Locked and shown, as a cell stating nothing is.
    fn default() -> Self {
        Self {
            locked: true,
            hidden: false,
        }
    }
}

/// Everything one `cellXfs` entry says about the cells naming it.
///
/// A cell's [`StyleId`] resolves to one of these through
/// [`StyleSheet::style`](super::styles::StyleSheet::style): the entry's own
/// facts, and the number format, font, fill and border it points into. The
/// default is what a cell stating no style displays with in a workbook
/// built from nothing: `General`, Excel's default font, no fill, no border.
#[derive(Clone, Debug, PartialEq)]
pub struct CellStyle {
    /// The format code, `General` for none; a built-in id the part does not
    /// declare reads as its en-US code.
    pub number_format: SmolStr,
    /// The font.
    pub font: Font,
    /// The fill.
    pub fill: Fill,
    /// The border.
    pub border: Border,
    /// The alignment.
    pub alignment: Alignment,
    /// The protection.
    pub protection: Protection,
    /// `quotePrefix`: the value was entered as text with a leading `'`.
    pub quote_prefix: bool,
    /// `xfId`: the named cell style (`cellStyleXfs`) this entry derives from.
    pub parent: u32,
}

impl Default for CellStyle {
    fn default() -> Self {
        Self {
            number_format: SmolStr::new_static("General"),
            font: Font::default(),
            fill: Fill::default(),
            border: Border::default(),
            alignment: Alignment::default(),
            protection: Protection::default(),
            quote_prefix: false,
            parent: 0,
        }
    }
}

vocabulary! {
    /// Which edges of a range a border preset draws, as the ribbon's
    /// border menu names them.
    BorderPreset, "border preset", default None {
        /// The bottom edge of the range's last row.
        Bottom => "bottom",
        /// The top edge of the range's first row.
        Top => "top",
        /// The left edge of the range's first column.
        Left => "left",
        /// The right edge of the range's last column.
        Right => "right",
        /// Every edge of every cell.
        All => "all",
        /// The range's outline, no edge inside it.
        Outside => "outside",
        /// The range's outline, thick whatever style is given.
        ThickOutside => "thickOutside",
        /// No edge on any cell of the range, diagonals included.
        None => "none",
    }
}

/// A border preset with the line and colour it draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Borders {
    /// Which edges.
    pub preset: BorderPreset,
    /// The line each edge draws; [`BorderPreset::ThickOutside`] draws
    /// [`BorderStyle::Thick`] whatever this says.
    pub style: BorderStyle,
    /// The line's colour, `None` for the automatic one.
    pub color: Option<Color>,
}

impl Default for Borders {
    /// A thin automatic line on every edge.
    fn default() -> Self {
        Self {
            preset: BorderPreset::All,
            style: BorderStyle::Thin,
            color: None,
        }
    }
}

/// The edges of a range one of its cells lies on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct Outline {
    pub(crate) top: bool,
    pub(crate) bottom: bool,
    pub(crate) left: bool,
    pub(crate) right: bool,
}

impl Borders {
    /// The edges this preset draws on a cell lying on `outline`: a bit per
    /// edge - left, right, top, bottom - the one input a cell's patched
    /// border depends on besides its own.
    pub(crate) const fn edges(&self, outline: Outline) -> u8 {
        let (left, right, top, bottom) = match self.preset {
            BorderPreset::Bottom => (false, false, false, outline.bottom),
            BorderPreset::Top => (false, false, outline.top, false),
            BorderPreset::Left => (outline.left, false, false, false),
            BorderPreset::Right => (false, outline.right, false, false),
            BorderPreset::All | BorderPreset::None => (true, true, true, true),
            BorderPreset::Outside | BorderPreset::ThickOutside => {
                (outline.left, outline.right, outline.top, outline.bottom)
            }
        };
        (left as u8) | (right as u8) << 1 | (top as u8) << 2 | (bottom as u8) << 3
    }

    /// `border` with the edges `edges` names drawn - or, for
    /// [`BorderPreset::None`], every edge taken off.
    pub(crate) fn apply(&self, border: &mut Border, edges: u8) {
        if self.preset == BorderPreset::None {
            *border = Border {
                outline: border.outline,
                ..Border::default()
            };
            return;
        }
        let edge = Edge {
            style: match self.preset {
                BorderPreset::ThickOutside => BorderStyle::Thick,
                _ => self.style,
            },
            color: self.color,
        };
        for (bit, side) in [
            (0, &mut border.left),
            (1, &mut border.right),
            (2, &mut border.top),
            (3, &mut border.bottom),
        ] {
            if edges & (1 << bit) != 0 {
                *side = edge;
            }
        }
    }
}

/// A change to the style of every cell of some ranges: each field set is
/// applied, each left `None` keeps what the cell has.
///
/// Where a fact can be cleared, the field is doubly optional: `font_color:
/// Some(None)` makes the font automatic, `fill: Some(None)` takes the fill
/// off, `fill: Some(Some(colour))` paints it solid. `number_format` is a
/// format code - `General` clears it - and `decimals` a number of decimal
/// places to add or take off each cell's own format
/// ([`FormatCode::with_decimals`](super::FormatCode::with_decimals)); a
/// patch states one or the other.
///
/// ```
/// use yggdryl::excel::{Color, StylePatch, Workbook};
///
/// let mut workbook = Workbook::new();
/// workbook.add_sheet("Sheet1")?.set_cell("A1".parse()?, 1234.5)?;
/// let patch = StylePatch {
///     bold: Some(true),
///     fill: Some(Some(Color::Rgb(0xFF_FF_FF_00))),
///     number_format: Some("#,##0.00".into()),
///     ..StylePatch::default()
/// };
/// workbook.set_style("Sheet1", &["A1:B2".parse()?], &patch)?;
/// let style = workbook.cell_style("Sheet1", "B2".parse()?)?;
/// assert!(style.font.bold);
/// assert_eq!(style.number_format, "#,##0.00");
/// assert_eq!(workbook.display_text("Sheet1", "A1".parse()?)?.map(|shown| shown.text), Some("1,234.50".into()));
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StylePatch {
    /// The typeface.
    pub font_name: Option<SmolStr>,
    /// The size in points, from 1 to 409.
    pub font_size: Option<f64>,
    /// Bold.
    pub bold: Option<bool>,
    /// Italic.
    pub italic: Option<bool>,
    /// The underline.
    pub underline: Option<Underline>,
    /// Struck through.
    pub strike: Option<bool>,
    /// The font's colour; `Some(None)` is automatic.
    pub font_color: Option<Option<Color>>,
    /// A solid fill; `Some(None)` is no fill.
    pub fill: Option<Option<Color>>,
    /// A border preset.
    pub borders: Option<Borders>,
    /// The horizontal alignment.
    pub horizontal: Option<Horizontal>,
    /// The vertical alignment.
    pub vertical: Option<Vertical>,
    /// Wrap text.
    pub wrap: Option<bool>,
    /// The indent, in steps of three spaces, at most 250.
    pub indent: Option<u8>,
    /// The number format's code; `General` clears it.
    pub number_format: Option<SmolStr>,
    /// Decimal places to add (or, negative, take off) each cell's own
    /// number format, from -15 to 15.
    pub decimals: Option<i8>,
}

impl StylePatch {
    /// Refuse a patch no cell can take, before any cell is touched.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the field: both a format and
    /// decimals, decimals past fifteen either way, a font size outside 1 to
    /// 409, an empty or over-long font name, an indent past 250, or a
    /// format code that does not read or names a locale's own digits.
    pub(crate) fn check(&self) -> Result<Option<super::format::FormatCode>> {
        let refused = |field: &'static str, reason: SmolStr| Error::InvalidRecord {
            path: SmolStr::new_static(field),
            reason,
        };
        if self.number_format.is_some() && self.decimals.is_some() {
            return Err(refused(
                "$.decimals",
                SmolStr::new_static("expected a number format or decimals in one patch, got both"),
            ));
        }
        if let Some(decimals) = self
            .decimals
            .filter(|decimals| !(-15..=15).contains(decimals))
        {
            return Err(refused(
                "$.decimals",
                format_smolstr!("expected decimals from -15 to 15, got {decimals}"),
            ));
        }
        if let Some(size) = self
            .font_size
            .filter(|size| !(size.is_finite() && (1.0..=409.0).contains(size)))
        {
            return Err(refused(
                "$.font_size",
                format_smolstr!("expected a font size from 1 to 409 points, got {size}"),
            ));
        }
        if let Some(name) = self
            .font_name
            .as_ref()
            .filter(|name| name.trim().is_empty() || name.chars().count() > 31)
        {
            return Err(refused(
                "$.font_name",
                format_smolstr!("expected a font name of 1 to 31 characters, got {name:?}"),
            ));
        }
        if let Some(indent) = self.indent.filter(|indent| *indent > 250) {
            return Err(refused(
                "$.indent",
                format_smolstr!("expected an indent from 0 to 250, got {indent}"),
            ));
        }
        let Some(code) = &self.number_format else {
            return Ok(None);
        };
        let format = super::format::FormatCode::from_code(code)?;
        if format.is_localized() {
            return Err(refused(
                "$.number_format",
                format_smolstr!(
                    "expected a format in en-US digits, got {code:?}, which names a locale's own"
                ),
            ));
        }
        Ok(Some(format))
    }

    /// `style` with the patch applied, the cell lying on the edges
    /// `edges` of its range, and `format` the number format the cell shows
    /// (for `decimals`).
    pub(crate) fn applied(
        &self,
        style: &CellStyle,
        format: Option<&super::format::FormatCode>,
        stated: Option<&super::format::FormatCode>,
        edges: u8,
    ) -> CellStyle {
        let mut patched = style.clone();
        let font = &mut patched.font;
        if let Some(name) = &self.font_name
            && *name != font.name
        {
            font.name = name.clone();
            // A named face no longer follows the theme's fonts.
            font.scheme = None;
        }
        if let Some(size) = self.font_size {
            font.size = size;
        }
        if let Some(bold) = self.bold {
            font.bold = bold;
        }
        if let Some(italic) = self.italic {
            font.italic = italic;
        }
        if let Some(underline) = self.underline {
            font.underline = underline;
        }
        if let Some(strike) = self.strike {
            font.strike = strike;
        }
        if let Some(color) = self.font_color {
            font.color = color;
        }
        if let Some(fill) = self.fill {
            patched.fill = match fill {
                None => Fill::None,
                Some(color) => Fill::Pattern {
                    pattern: PatternType::Solid,
                    foreground: Some(color),
                    background: Some(Color::Indexed {
                        index: 64,
                        tint: 0.0,
                    }),
                },
            };
        }
        if let Some(borders) = &self.borders {
            borders.apply(&mut patched.border, edges);
        }
        let alignment = &mut patched.alignment;
        if let Some(horizontal) = self.horizontal {
            alignment.horizontal = horizontal;
        }
        if let Some(vertical) = self.vertical {
            alignment.vertical = vertical;
        }
        if let Some(wrap) = self.wrap {
            alignment.wrap = wrap;
        }
        if let Some(indent) = self.indent {
            alignment.indent = indent;
            // An indent is measured from an edge: text centred or spread
            // takes the left one, as the ribbon does.
            if indent > 0
                && self.horizontal.is_none()
                && !matches!(
                    alignment.horizontal,
                    Horizontal::Left | Horizontal::Right | Horizontal::Distributed
                )
            {
                alignment.horizontal = Horizontal::Left;
            }
        }
        if let Some(format) = stated {
            patched.number_format = SmolStr::new(if format.is_general() {
                "General"
            } else {
                format.code()
            });
        } else if let (Some(delta), Some(format)) = (self.decimals, format) {
            patched.number_format = SmolStr::new(format.clone().with_decimals(delta).code());
        }
        patched
    }
}
