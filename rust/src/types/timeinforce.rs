//! How long an order stands.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::types;
use crate::types::code::{CodeValue, code_leaf, code_value};
use crate::types::typed::define_field_types;
use crate::{DataType, Result, Scalar, Value};

code_leaf!(TimeInForce, TIMEINFORCE_WIDTH);

code_value!(TimeInForce, TimeInForce, TIMEINFORCE_WIDTH);

/// The Arrow extension name of how long an order stands.
pub(crate) const TIMEINFORCE_EXTENSION_NAME: &str = "yggdryl.timeinforce";

/// The most bytes how long an order stands may be.
///
/// The standard's values are one character; eight leaves room for venue codes.
pub(crate) const TIMEINFORCE_WIDTH: usize = 8;

// /// A field declared as how long an order stands.
define_field_types!(TimeInForceType, TimeInForce);
