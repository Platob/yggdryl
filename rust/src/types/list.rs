//! The five sequence layouts over one item field.

use std::sync::Arc;

use crate::types::dtype::validate_non_negative;
use crate::types::typed::define_field_types;
use crate::{
    DataType, Field, Result,

};

impl DataType {

    /// Creates a 32-bit variable list.
    pub fn list(item: Field) -> Self {
        Self::List(Arc::new(item))
    }

    /// Creates a 32-bit variable list-view.
    pub fn list_view(item: Field) -> Self {
        Self::ListView(Arc::new(item))
    }

    /// Creates a fixed-size list after validating its element count.
    pub fn fixed_size_list(item: Field, length: i32) -> Result<Self> {
        validate_non_negative("FixedSizeList", "length", length)?;
        Ok(Self::FixedSizeList(Arc::new(item), length))
    }

    /// Creates a 64-bit variable list.
    pub fn large_list(item: Field) -> Self {
        Self::LargeList(Arc::new(item))
    }

    /// Creates a 64-bit variable list-view.
    pub fn large_list_view(item: Field) -> Self {
        Self::LargeListView(Arc::new(item))
    }

    /// Returns the item field of a list-shaped datatype.
    ///
    /// The five list layouts hold exactly one child, and a dotted path treats
    /// that child as a step it need not spell; this is the one place they are
    /// recognized as such.
    pub(crate) fn list_item(&self) -> Option<&Field> {
        match self {
            Self::List(field)
            | Self::ListView(field)
            | Self::FixedSizeList(field, _)
            | Self::LargeList(field)
            | Self::LargeListView(field) => Some(field),
            _ => None,
        }
    }
}

define_field_types!(ListType, List, crate::DataType::List(_));

define_field_types!(ListViewType, ListView, crate::DataType::ListView(_));

define_field_types!(LargeListType, LargeList, crate::DataType::LargeList(_));
