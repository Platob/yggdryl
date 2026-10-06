//! The role of a FIX plugin: the side of the session a dialect's plugin
//! stands on, as one enum stored as a `uint8`.

use crate::code::folded_spelling;
use crate::enums::enum_leaf;
use crate::typed::define_field_types;

enum_leaf! {
    /// The role of the FIX plugin a dialect configures: `BUYS` for a
    /// Buy-Side plugin, `SELL` for a Sell-Side one, and `UKNW` where the
    /// dialect states none.
    ///
    /// A session fact, read once off a dialect's source entry - a CBlock's
    /// root `type` attribute names the plugin's class, and
    /// [`Self::from_plugin_type`] reads its role off that name - and
    /// stamped on every message read under that source as `msgpluginside`;
    /// never read off a FIX tag, and independent of `Side(54)`: a
    /// Sell-Side plugin receives orders of either side. A separate enum
    /// from [`Side`](crate::Side) though `BUYS` and `SELL` are spelled
    /// alike, never shared and never cast one into the other.
    ///
    /// ```
    /// use yggdryl::PluginSide;
    ///
    /// assert_eq!(PluginSide::BuySide.as_str(), "BUYS");
    /// assert_eq!(PluginSide::BuySide.code(), 1);
    /// assert_eq!(PluginSide::from_spelling("sell-side"), Some(PluginSide::SellSide));
    /// assert_eq!(PluginSide::from_spelling("buys"), Some(PluginSide::BuySide));
    /// assert_eq!(
    ///     PluginSide::from_plugin_type(
    ///         "com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.SellSideFIXCPluginCBlock"
    ///     ),
    ///     PluginSide::SellSide
    /// );
    /// assert_eq!(PluginSide::from_plugin_type(""), PluginSide::Unknown);
    /// ```
    #[non_exhaustive]
    pub enum PluginSide: u8, kind = "pluginside", extension = PLUGINSIDE_EXTENSION_NAME, aliases = pluginside_aliases {
        #[default]
        Unknown = 0 as "UKNW": "No plugin role found, or one no member names.",
        BuySide = 1 as "BUYS": "A Buy-Side plugin: it originates orders and cancels and receives execution reports.",
        SellSide = 2 as "SELL": "A Sell-Side plugin: it receives orders and cancels and answers with execution reports.",
    }
}

impl PluginSide {
    /// The role one plugin class name states: the last `.`-separated
    /// segment of a CBlock's root `type` attribute, folded the way every
    /// name in this crate folds - case, `_`, `-` and blanks dropped -
    /// holding `buyside` is [`Self::BuySide`], one holding `sellside` is
    /// [`Self::SellSide`], and any other - a class naming neither, or no
    /// attribute at all - is [`Self::Unknown`]. Never refuses: a plugin
    /// whose role the name does not spell is a plugin of no stated role.
    ///
    /// ```
    /// use yggdryl::PluginSide;
    ///
    /// let buy = "com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.BuySideFIXCPluginCBlock";
    /// assert_eq!(PluginSide::from_plugin_type(buy), PluginSide::BuySide);
    /// assert_eq!(PluginSide::from_plugin_type("SellSideFIXCPluginCBlock"), PluginSide::SellSide);
    /// assert_eq!(PluginSide::from_plugin_type("x.Buy_Side_FIXCPluginCBlock"), PluginSide::BuySide);
    /// assert_eq!(PluginSide::from_plugin_type("x.FIXCPluginCBlock"), PluginSide::Unknown);
    /// // Only the last segment is read: a package naming a side names no role.
    /// assert_eq!(PluginSide::from_plugin_type("buyside.FIXCPluginCBlock"), PluginSide::Unknown);
    /// ```
    #[must_use]
    pub fn from_plugin_type(class: &str) -> Self {
        let last = class.rsplit('.').next().unwrap_or(class);
        let folded = folded_spelling(last);
        if folded.contains("buyside") {
            Self::BuySide
        } else if folded.contains("sellside") {
            Self::SellSide
        } else {
            Self::Unknown
        }
    }

    /// The member one spelling names, or `None` where none does.
    ///
    /// The stored name in any case - `BUYS`, `buys` - and the role's own
    /// name folded the way every name in this crate folds: `BuySide`,
    /// `buy-side`, `buy_side`, `SELL SIDE`. A stored code is an integer and
    /// never text, and [`Self::from_code`] reads it.
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        if let Some(held) = Self::from_name(spelling) {
            return Some(held);
        }
        let folded = folded_spelling(spelling);
        PLUGINSIDE_NAMES
            .iter()
            .find(|(name, _)| *name == folded.as_str())
            .map(|(_, side)| *side)
            .or_else(|| Self::from_pattern(spelling))
    }
}

/// The names a role goes by beside its stored name, which its spelling
/// patterns read the words of.
fn pluginside_aliases() -> Vec<(&'static str, PluginSide)> {
    PLUGINSIDE_NAMES.to_vec()
}

/// Every name that reaches a role, folded: the stored names and the roles'
/// own names.
static PLUGINSIDE_NAMES: &[(&str, PluginSide)] = &[
    ("buys", PluginSide::BuySide),
    ("buyside", PluginSide::BuySide),
    ("sell", PluginSide::SellSide),
    ("sellside", PluginSide::SellSide),
    ("uknw", PluginSide::Unknown),
    ("unknown", PluginSide::Unknown),
];

/// The Arrow extension name of a plugin's role, over `uint8` storage.
pub(crate) const PLUGINSIDE_EXTENSION_NAME: &str = "yggdryl.pluginside";

impl crate::DataType {
    /// Creates the plugin-side datatype: the role of a FIX plugin.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::pluginside(), DataType::PluginSide);
    /// assert_eq!(DataType::pluginside().to_string(), "pluginside");
    /// assert!(DataType::pluginside().is_enum());
    /// ```
    #[must_use]
    pub const fn pluginside() -> Self {
        Self::PluginSide
    }
}

// /// A field declared as the role of a FIX plugin.
define_field_types!(PluginSideType, PluginSide);
