//! Hive partition components carried by URLs.

use crate::Url;

impl Url {
    /// Read the Hive partition pairs a location's path spells out.
    ///
    /// A Hive layout writes one directory per partition column, named
    /// `column=value`. Reading them back is what lets a partitioned read
    /// restore the columns the directory names replaced.
    ///
    /// ```
    /// use yggdryl::Url;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let url = Url::from_str("file:///lake/trades/year=2024/month=01/part-0.parquet")?;
    ///
    /// assert_eq!(
    ///     url.hive_partitions(),
    ///     vec![("year".to_owned(), "2024".to_owned()), ("month".to_owned(), "01".to_owned())]
    /// );
    /// assert!(url.is_hive_partitioned());
    /// # Ok(())
    /// # }
    /// ```
    pub fn hive_partitions(&self) -> Vec<(String, String)> {
        self.path_segments().filter_map(hive_pair).collect()
    }

    /// Read the Hive partition pairs this location spells out *below* `root`.
    ///
    /// A lake is addressed at some level, and only the directories below that
    /// level are its partition columns: `year` is a partition of `/lake` but a
    /// fixed part of the address `/lake/year=2024`. A location outside `root`
    /// spells out nothing.
    ///
    /// ```
    /// use yggdryl::Url;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let url = Url::from_str("file:///lake/year=2024/month=01/part-0.parquet")?;
    ///
    /// assert_eq!(
    ///     url.hive_partitions_under(&Url::from_str("file:///lake")?),
    ///     vec![("year".to_owned(), "2024".to_owned()), ("month".to_owned(), "01".to_owned())]
    /// );
    /// assert_eq!(
    ///     url.hive_partitions_under(&Url::from_str("file:///lake/year=2024")?),
    ///     vec![("month".to_owned(), "01".to_owned())]
    /// );
    /// assert!(url.hive_partitions_under(&Url::from_str("file:///elsewhere")?).is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn hive_partitions_under(&self, root: &Self) -> Vec<(String, String)> {
        self.segments_under(root)
            .map(|segments| segments.into_iter().filter_map(hive_pair).collect())
            .unwrap_or_default()
    }

    /// Return whether any path segment is a Hive partition directory.
    pub fn is_hive_partitioned(&self) -> bool {
        !self.hive_partitions().is_empty()
    }

    /// Return the value of one Hive partition column, when the path carries it.
    pub fn hive_partition(&self, column: &str) -> Option<String> {
        self.hive_partitions()
            .into_iter()
            .find_map(|(key, value)| (key == column).then_some(value))
    }

    /// Extend this location with one Hive partition directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the column or value cannot form a path segment.
    pub fn with_hive_partition(&self, column: &str, value: &str) -> crate::Result<Self> {
        self.joinpath(&format!("{column}={value}"))
    }
}

/// Read the `column=value` pair one path segment spells out.
fn hive_pair(segment: &str) -> Option<(String, String)> {
    let (key, value) = segment.split_once('=')?;
    // A key must look like a column name, not an escaped byte.
    (!key.is_empty() && !key.contains(' ')).then(|| (key.to_owned(), value.to_owned()))
}
