//! The one crate-private catalog of Excel function names and metadata.
//!
//! A listed function has a typed signature. Eleven held wire spellings have
//! only a prefix. The catalog does not evaluate either kind.

use super::lexer::Prefix;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Category {
    Math,
    Statistical,
    Logical,
    Lookup,
    Text,
    DateTime,
    Information,
    Financial,
}

impl Category {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Math => "Math and trigonometry",
            Self::Statistical => "Statistical",
            Self::Logical => "Logical",
            Self::Lookup => "Lookup and reference",
            Self::Text => "Text",
            Self::DateTime => "Date and time",
            Self::Information => "Information",
            Self::Financial => "Financial",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Arity {
    pub(crate) min: u8,
    pub(crate) max: u8,
    pub(crate) step: u8,
}

impl Arity {
    pub(crate) const fn accepts(self, count: usize) -> bool {
        count >= self.min as usize
            && count <= self.max as usize
            && (count - self.min as usize).is_multiple_of(self.step as usize)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Signature {
    pub(crate) category: Category,
    pub(crate) text: &'static str,
    pub(crate) description: &'static str,
    pub(crate) arity: Arity,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FunctionInfo {
    pub(crate) function: Function,
    pub(crate) name: &'static str,
    pub(crate) prefix: Prefix,
    pub(crate) volatile: bool,
    pub(crate) signature: Option<Signature>,
}

// One alphabetical row emits the enum and all metadata. Lookup is a binary
// search over ASCII spellings, without per-formula allocation.
macro_rules! registry {
    ($( $variant:ident => ($name:literal, $prefix:expr, $volatile:expr, $signature:expr), )+) => {
        #[repr(u8)]
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub(crate) enum Function { $( $variant, )+ }
        pub(crate) const FUNCTIONS: &[FunctionInfo] = &[
            $( FunctionInfo { function: Function::$variant, name: $name,
                prefix: $prefix, volatile: $volatile, signature: $signature }, )+
        ];
    };
}

registry! {
    Abs => ("ABS", Prefix::None, false, Some(Signature { category: Category::Math, text: "ABS(number)", description: "Returns the absolute value of a number, a number without its sign.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Acos => ("ACOS", Prefix::None, false, Some(Signature { category: Category::Math, text: "ACOS(number)", description: "Returns the arccosine of a number, in radians from 0 to Pi.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Address => ("ADDRESS", Prefix::None, false, Some(Signature { category: Category::Lookup, text: "ADDRESS(row_num, column_num, [abs_num], [a1], [sheet_text])", description: "Creates a cell reference as text, given specified row and column numbers.", arity: Arity { min: 2, max: 5, step: 1 } })),
    Aggregate => ("AGGREGATE", Prefix::Future, false, None),
    Anchorarray => ("ANCHORARRAY", Prefix::Future, false, None),
    And => ("AND", Prefix::None, false, Some(Signature { category: Category::Logical, text: "AND(logical1, [logical2], ...)", description: "Checks whether all arguments are TRUE, and returns TRUE if all arguments are TRUE.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Asin => ("ASIN", Prefix::None, false, Some(Signature { category: Category::Math, text: "ASIN(number)", description: "Returns the arcsine of a number, in radians from -Pi/2 to Pi/2.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Atan => ("ATAN", Prefix::None, false, Some(Signature { category: Category::Math, text: "ATAN(number)", description: "Returns the arctangent of a number, in radians from -Pi/2 to Pi/2.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Atan2 => ("ATAN2", Prefix::None, false, Some(Signature { category: Category::Math, text: "ATAN2(x_num, y_num)", description: "Returns the arctangent of the x- and y-coordinates, in radians from -Pi to Pi, excluding -Pi.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Average => ("AVERAGE", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "AVERAGE(number1, [number2], ...)", description: "Returns the average (arithmetic mean) of its arguments.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Averagea => ("AVERAGEA", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "AVERAGEA(value1, [value2], ...)", description: "Returns the average of its arguments, evaluating text and FALSE in arguments as 0; TRUE evaluates as 1.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Averageif => ("AVERAGEIF", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "AVERAGEIF(range, criteria, [average_range])", description: "Finds the average (arithmetic mean) for the cells specified by a given condition or criteria.", arity: Arity { min: 2, max: 3, step: 1 } })),
    Averageifs => ("AVERAGEIFS", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "AVERAGEIFS(average_range, criteria_range1, criteria1, [criteria_range2, criteria2], ...)", description: "Finds the average (arithmetic mean) for the cells specified by a given set of conditions or criteria.", arity: Arity { min: 3, max: 255, step: 2 } })),
    Ceiling => ("CEILING", Prefix::None, false, Some(Signature { category: Category::Math, text: "CEILING(number, significance)", description: "Rounds a number up, to the nearest multiple of significance.", arity: Arity { min: 2, max: 2, step: 1 } })),
    CeilingDotMath => ("CEILING.MATH", Prefix::Future, false, Some(Signature { category: Category::Math, text: "CEILING.MATH(number, [significance], [mode])", description: "Rounds a number up, to the nearest integer or to the nearest multiple of significance.", arity: Arity { min: 1, max: 3, step: 1 } })),
    Char => ("CHAR", Prefix::None, false, Some(Signature { category: Category::Text, text: "CHAR(number)", description: "Returns the character specified by the code number from the character set for your computer.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Choose => ("CHOOSE", Prefix::None, false, Some(Signature { category: Category::Lookup, text: "CHOOSE(index_num, value1, [value2], ...)", description: "Chooses a value or action to perform from a list of values, based on an index number.", arity: Arity { min: 2, max: 255, step: 1 } })),
    Clean => ("CLEAN", Prefix::None, false, Some(Signature { category: Category::Text, text: "CLEAN(text)", description: "Removes all nonprintable characters from text.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Code => ("CODE", Prefix::None, false, Some(Signature { category: Category::Text, text: "CODE(text)", description: "Returns a numeric code for the first character in a text string.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Column => ("COLUMN", Prefix::None, false, Some(Signature { category: Category::Lookup, text: "COLUMN([reference])", description: "Returns the column number of a reference.", arity: Arity { min: 0, max: 1, step: 1 } })),
    Columns => ("COLUMNS", Prefix::None, false, Some(Signature { category: Category::Lookup, text: "COLUMNS(array)", description: "Returns the number of columns in an array or reference.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Concat => ("CONCAT", Prefix::Future, false, Some(Signature { category: Category::Text, text: "CONCAT(text1, [text2], ...)", description: "Concatenates a list or range of text strings.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Concatenate => ("CONCATENATE", Prefix::None, false, Some(Signature { category: Category::Text, text: "CONCATENATE(text1, [text2], ...)", description: "Joins several text strings into one text string.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Cos => ("COS", Prefix::None, false, Some(Signature { category: Category::Math, text: "COS(number)", description: "Returns the cosine of an angle.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Count => ("COUNT", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "COUNT(value1, [value2], ...)", description: "Counts the number of cells in a range that contain numbers.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Counta => ("COUNTA", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "COUNTA(value1, [value2], ...)", description: "Counts the number of cells in a range that are not empty.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Countblank => ("COUNTBLANK", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "COUNTBLANK(range)", description: "Counts the number of empty cells in a specified range of cells.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Countif => ("COUNTIF", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "COUNTIF(range, criteria)", description: "Counts the number of cells within a range that meet the given condition.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Countifs => ("COUNTIFS", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "COUNTIFS(criteria_range1, criteria1, [criteria_range2, criteria2], ...)", description: "Counts the number of cells specified by a given set of conditions or criteria.", arity: Arity { min: 2, max: 254, step: 2 } })),
    Date => ("DATE", Prefix::None, false, Some(Signature { category: Category::DateTime, text: "DATE(year, month, day)", description: "Returns the number that represents the date in the date-time code.", arity: Arity { min: 3, max: 3, step: 1 } })),
    Datevalue => ("DATEVALUE", Prefix::None, false, Some(Signature { category: Category::DateTime, text: "DATEVALUE(date_text)", description: "Converts a date in the form of text to a number that represents the date in the date-time code.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Day => ("DAY", Prefix::None, false, Some(Signature { category: Category::DateTime, text: "DAY(serial_number)", description: "Returns the day of the month, a number from 1 to 31.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Days => ("DAYS", Prefix::Future, false, Some(Signature { category: Category::DateTime, text: "DAYS(end_date, start_date)", description: "Returns the number of days between the two dates.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Degrees => ("DEGREES", Prefix::None, false, Some(Signature { category: Category::Math, text: "DEGREES(angle)", description: "Converts radians to degrees.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Edate => ("EDATE", Prefix::None, false, Some(Signature { category: Category::DateTime, text: "EDATE(start_date, months)", description: "Returns the serial number of the date that is the indicated number of months before or after the start date.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Eomonth => ("EOMONTH", Prefix::None, false, Some(Signature { category: Category::DateTime, text: "EOMONTH(start_date, months)", description: "Returns the serial number of the last day of the month before or after a specified number of months.", arity: Arity { min: 2, max: 2, step: 1 } })),
    ErrorDotType => ("ERROR.TYPE", Prefix::None, false, Some(Signature { category: Category::Information, text: "ERROR.TYPE(error_val)", description: "Returns a number matching an error value.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Even => ("EVEN", Prefix::None, false, Some(Signature { category: Category::Math, text: "EVEN(number)", description: "Rounds a positive number up and a negative number down to the nearest even integer.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Exact => ("EXACT", Prefix::None, false, Some(Signature { category: Category::Text, text: "EXACT(text1, text2)", description: "Checks whether two text strings are exactly the same, and returns TRUE or FALSE. EXACT is case-sensitive.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Exp => ("EXP", Prefix::None, false, Some(Signature { category: Category::Math, text: "EXP(number)", description: "Returns e raised to the power of a given number.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Fact => ("FACT", Prefix::None, false, Some(Signature { category: Category::Math, text: "FACT(number)", description: "Returns the factorial of a number, equal to 1*2*3*...*number.", arity: Arity { min: 1, max: 1, step: 1 } })),
    False => ("FALSE", Prefix::None, false, Some(Signature { category: Category::Logical, text: "FALSE()", description: "Returns the logical value FALSE.", arity: Arity { min: 0, max: 0, step: 1 } })),
    Filter => ("FILTER", Prefix::FutureWorksheet, false, None),
    Find => ("FIND", Prefix::None, false, Some(Signature { category: Category::Text, text: "FIND(find_text, within_text, [start_num])", description: "Returns the starting position of one text string within another text string. FIND is case-sensitive.", arity: Arity { min: 2, max: 3, step: 1 } })),
    Floor => ("FLOOR", Prefix::None, false, Some(Signature { category: Category::Math, text: "FLOOR(number, significance)", description: "Rounds a number down to the nearest multiple of significance.", arity: Arity { min: 2, max: 2, step: 1 } })),
    FloorDotMath => ("FLOOR.MATH", Prefix::Future, false, Some(Signature { category: Category::Math, text: "FLOOR.MATH(number, [significance], [mode])", description: "Rounds a number down, to the nearest integer or to the nearest multiple of significance.", arity: Arity { min: 1, max: 3, step: 1 } })),
    Fv => ("FV", Prefix::None, false, Some(Signature { category: Category::Financial, text: "FV(rate, nper, pmt, [pv], [type])", description: "Returns the future value of an investment based on periodic, constant payments and a constant interest rate.", arity: Arity { min: 3, max: 5, step: 1 } })),
    Gcd => ("GCD", Prefix::None, false, Some(Signature { category: Category::Math, text: "GCD(number1, [number2], ...)", description: "Returns the greatest common divisor.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Hlookup => ("HLOOKUP", Prefix::None, false, Some(Signature { category: Category::Lookup, text: "HLOOKUP(lookup_value, table_array, row_index_num, [range_lookup])", description: "Looks for a value in the top row of a table or array of values and returns the value in the same column from a row you specify.", arity: Arity { min: 3, max: 4, step: 1 } })),
    Hour => ("HOUR", Prefix::None, false, Some(Signature { category: Category::DateTime, text: "HOUR(serial_number)", description: "Returns the hour as a number from 0 (12:00 A.M.) to 23 (11:00 P.M.).", arity: Arity { min: 1, max: 1, step: 1 } })),
    If => ("IF", Prefix::None, false, Some(Signature { category: Category::Logical, text: "IF(logical_test, value_if_true, [value_if_false])", description: "Checks whether a condition is met, and returns one value if TRUE, and another value if FALSE.", arity: Arity { min: 2, max: 3, step: 1 } })),
    Iferror => ("IFERROR", Prefix::None, false, Some(Signature { category: Category::Logical, text: "IFERROR(value, value_if_error)", description: "Returns value_if_error if expression is an error and the value of the expression itself otherwise.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Ifna => ("IFNA", Prefix::Future, false, Some(Signature { category: Category::Logical, text: "IFNA(value, value_if_na)", description: "Returns the value you specify if the expression resolves to #N/A, otherwise returns the result of the expression.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Ifs => ("IFS", Prefix::Future, false, Some(Signature { category: Category::Logical, text: "IFS(logical_test1, value_if_true1, [logical_test2, value_if_true2], ...)", description: "Checks whether one or more conditions are met and returns a value corresponding to the first TRUE condition.", arity: Arity { min: 2, max: 254, step: 2 } })),
    Index => ("INDEX", Prefix::None, false, Some(Signature { category: Category::Lookup, text: "INDEX(reference, row_num, [column_num], [area_num])", description: "Returns a value or reference of the cell at the intersection of a particular row and column, in a given range.", arity: Arity { min: 2, max: 4, step: 1 } })),
    Indirect => ("INDIRECT", Prefix::None, true, Some(Signature { category: Category::Lookup, text: "INDIRECT(ref_text, [a1])", description: "Returns the reference specified by a text string.", arity: Arity { min: 1, max: 2, step: 1 } })),
    Int => ("INT", Prefix::None, false, Some(Signature { category: Category::Math, text: "INT(number)", description: "Rounds a number down to the nearest integer.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Isblank => ("ISBLANK", Prefix::None, false, Some(Signature { category: Category::Information, text: "ISBLANK(value)", description: "Checks whether a reference is to an empty cell, and returns TRUE or FALSE.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Iserr => ("ISERR", Prefix::None, false, Some(Signature { category: Category::Information, text: "ISERR(value)", description: "Checks whether a value is an error other than #N/A, and returns TRUE or FALSE.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Iserror => ("ISERROR", Prefix::None, false, Some(Signature { category: Category::Information, text: "ISERROR(value)", description: "Checks whether a value is an error, and returns TRUE or FALSE.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Iseven => ("ISEVEN", Prefix::None, false, Some(Signature { category: Category::Information, text: "ISEVEN(number)", description: "Returns TRUE if the number is even.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Islogical => ("ISLOGICAL", Prefix::None, false, Some(Signature { category: Category::Information, text: "ISLOGICAL(value)", description: "Checks whether a value is a logical value (TRUE or FALSE), and returns TRUE or FALSE.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Isna => ("ISNA", Prefix::None, false, Some(Signature { category: Category::Information, text: "ISNA(value)", description: "Checks whether a value is #N/A, and returns TRUE or FALSE.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Isnontext => ("ISNONTEXT", Prefix::None, false, Some(Signature { category: Category::Information, text: "ISNONTEXT(value)", description: "Checks whether a value is not text (blank cells are not text), and returns TRUE or FALSE.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Isnumber => ("ISNUMBER", Prefix::None, false, Some(Signature { category: Category::Information, text: "ISNUMBER(value)", description: "Checks whether a value is a number, and returns TRUE or FALSE.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Isodd => ("ISODD", Prefix::None, false, Some(Signature { category: Category::Information, text: "ISODD(number)", description: "Returns TRUE if the number is odd.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Isref => ("ISREF", Prefix::None, false, Some(Signature { category: Category::Information, text: "ISREF(value)", description: "Checks whether a value is a reference, and returns TRUE or FALSE.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Istext => ("ISTEXT", Prefix::None, false, Some(Signature { category: Category::Information, text: "ISTEXT(value)", description: "Checks whether a value is text, and returns TRUE or FALSE.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Lambda => ("LAMBDA", Prefix::Future, false, None),
    Large => ("LARGE", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "LARGE(array, k)", description: "Returns the k-th largest value in a data set.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Lcm => ("LCM", Prefix::None, false, Some(Signature { category: Category::Math, text: "LCM(number1, [number2], ...)", description: "Returns the least common multiple.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Left => ("LEFT", Prefix::None, false, Some(Signature { category: Category::Text, text: "LEFT(text, [num_chars])", description: "Returns the specified number of characters from the start of a text string.", arity: Arity { min: 1, max: 2, step: 1 } })),
    Len => ("LEN", Prefix::None, false, Some(Signature { category: Category::Text, text: "LEN(text)", description: "Returns the number of characters in a text string.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Let => ("LET", Prefix::Future, false, None),
    Ln => ("LN", Prefix::None, false, Some(Signature { category: Category::Math, text: "LN(number)", description: "Returns the natural logarithm of a number.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Log => ("LOG", Prefix::None, false, Some(Signature { category: Category::Math, text: "LOG(number, [base])", description: "Returns the logarithm of a number to the base you specify.", arity: Arity { min: 1, max: 2, step: 1 } })),
    Log10 => ("LOG10", Prefix::None, false, Some(Signature { category: Category::Math, text: "LOG10(number)", description: "Returns the base-10 logarithm of a number.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Lookup => ("LOOKUP", Prefix::None, false, Some(Signature { category: Category::Lookup, text: "LOOKUP(lookup_value, lookup_vector, [result_vector])", description: "Looks up a value either from a one-row or one-column range.", arity: Arity { min: 2, max: 3, step: 1 } })),
    Lower => ("LOWER", Prefix::None, false, Some(Signature { category: Category::Text, text: "LOWER(text)", description: "Converts all letters in a text string to lowercase.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Match => ("MATCH", Prefix::None, false, Some(Signature { category: Category::Lookup, text: "MATCH(lookup_value, lookup_array, [match_type])", description: "Returns the relative position of an item in an array that matches a specified value in a specified order.", arity: Arity { min: 2, max: 3, step: 1 } })),
    Max => ("MAX", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "MAX(number1, [number2], ...)", description: "Returns the largest value in a set of values. Ignores logical values and text.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Maxa => ("MAXA", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "MAXA(value1, [value2], ...)", description: "Returns the largest value in a set of values. Does not ignore logical values and text.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Maxifs => ("MAXIFS", Prefix::Future, false, Some(Signature { category: Category::Statistical, text: "MAXIFS(max_range, criteria_range1, criteria1, [criteria_range2, criteria2], ...)", description: "Returns the maximum value among cells specified by a given set of conditions or criteria.", arity: Arity { min: 3, max: 255, step: 2 } })),
    Median => ("MEDIAN", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "MEDIAN(number1, [number2], ...)", description: "Returns the median, or the number in the middle of the set of given numbers.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Mid => ("MID", Prefix::None, false, Some(Signature { category: Category::Text, text: "MID(text, start_num, num_chars)", description: "Returns the characters from the middle of a text string, given a starting position and length.", arity: Arity { min: 3, max: 3, step: 1 } })),
    Min => ("MIN", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "MIN(number1, [number2], ...)", description: "Returns the smallest number in a set of values. Ignores logical values and text.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Mina => ("MINA", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "MINA(value1, [value2], ...)", description: "Returns the smallest value in a set of values. Does not ignore logical values and text.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Minifs => ("MINIFS", Prefix::Future, false, Some(Signature { category: Category::Statistical, text: "MINIFS(min_range, criteria_range1, criteria1, [criteria_range2, criteria2], ...)", description: "Returns the minimum value among cells specified by a given set of conditions or criteria.", arity: Arity { min: 3, max: 255, step: 2 } })),
    Minute => ("MINUTE", Prefix::None, false, Some(Signature { category: Category::DateTime, text: "MINUTE(serial_number)", description: "Returns the minute, a number from 0 to 59.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Mod => ("MOD", Prefix::None, false, Some(Signature { category: Category::Math, text: "MOD(number, divisor)", description: "Returns the remainder after a number is divided by a divisor.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Mode => ("MODE", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "MODE(number1, [number2], ...)", description: "Returns the most frequently occurring, or repetitive, value in an array or range of data.", arity: Arity { min: 1, max: 255, step: 1 } })),
    ModeDotSngl => ("MODE.SNGL", Prefix::Future, false, Some(Signature { category: Category::Statistical, text: "MODE.SNGL(number1, [number2], ...)", description: "Returns the most frequently occurring, or repetitive, value in an array or range of data.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Month => ("MONTH", Prefix::None, false, Some(Signature { category: Category::DateTime, text: "MONTH(serial_number)", description: "Returns the month, a number from 1 (January) to 12 (December).", arity: Arity { min: 1, max: 1, step: 1 } })),
    Mround => ("MROUND", Prefix::None, false, Some(Signature { category: Category::Math, text: "MROUND(number, multiple)", description: "Returns a number rounded to the desired multiple.", arity: Arity { min: 2, max: 2, step: 1 } })),
    N => ("N", Prefix::None, false, Some(Signature { category: Category::Information, text: "N(value)", description: "Converts non-number value to a number, dates to serial numbers, TRUE to 1, anything else to 0 (zero).", arity: Arity { min: 1, max: 1, step: 1 } })),
    Na => ("NA", Prefix::None, false, Some(Signature { category: Category::Information, text: "NA()", description: "Returns the error value #N/A (value not available).", arity: Arity { min: 0, max: 0, step: 1 } })),
    Not => ("NOT", Prefix::None, false, Some(Signature { category: Category::Logical, text: "NOT(logical)", description: "Changes FALSE to TRUE, or TRUE to FALSE.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Now => ("NOW", Prefix::None, true, Some(Signature { category: Category::DateTime, text: "NOW()", description: "Returns the current date and time formatted as a date and time.", arity: Arity { min: 0, max: 0, step: 1 } })),
    Npv => ("NPV", Prefix::None, false, Some(Signature { category: Category::Financial, text: "NPV(rate, value1, [value2], ...)", description: "Returns the net present value of an investment based on a discount rate and a series of future payments (negative values) and income (positive values).", arity: Arity { min: 2, max: 255, step: 1 } })),
    Odd => ("ODD", Prefix::None, false, Some(Signature { category: Category::Math, text: "ODD(number)", description: "Rounds a positive number up and a negative number down to the nearest odd integer.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Offset => ("OFFSET", Prefix::None, true, Some(Signature { category: Category::Lookup, text: "OFFSET(reference, rows, cols, [height], [width])", description: "Returns a reference to a range that is a given number of rows and columns from a given reference.", arity: Arity { min: 3, max: 5, step: 1 } })),
    Or => ("OR", Prefix::None, false, Some(Signature { category: Category::Logical, text: "OR(logical1, [logical2], ...)", description: "Checks whether any of the arguments are TRUE, and returns TRUE or FALSE. Returns FALSE only if all arguments are FALSE.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Percentile => ("PERCENTILE", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "PERCENTILE(array, k)", description: "Returns the k-th percentile of values in a range.", arity: Arity { min: 2, max: 2, step: 1 } })),
    PercentileDotInc => ("PERCENTILE.INC", Prefix::Future, false, Some(Signature { category: Category::Statistical, text: "PERCENTILE.INC(array, k)", description: "Returns the k-th percentile of values in a range, where k is in the range 0..1, inclusive.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Pi => ("PI", Prefix::None, false, Some(Signature { category: Category::Math, text: "PI()", description: "Returns the value of Pi, 3.14159265358979, accurate to 15 digits.", arity: Arity { min: 0, max: 0, step: 1 } })),
    Pmt => ("PMT", Prefix::None, false, Some(Signature { category: Category::Financial, text: "PMT(rate, nper, pv, [fv], [type])", description: "Calculates the payment for a loan based on constant payments and a constant interest rate.", arity: Arity { min: 3, max: 5, step: 1 } })),
    Power => ("POWER", Prefix::None, false, Some(Signature { category: Category::Math, text: "POWER(number, power)", description: "Returns the result of a number raised to a power.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Product => ("PRODUCT", Prefix::None, false, Some(Signature { category: Category::Math, text: "PRODUCT(number1, [number2], ...)", description: "Multiplies all the numbers given as arguments.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Proper => ("PROPER", Prefix::None, false, Some(Signature { category: Category::Text, text: "PROPER(text)", description: "Converts a text string to proper case; the first letter in each word to uppercase, and all other letters to lowercase.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Pv => ("PV", Prefix::None, false, Some(Signature { category: Category::Financial, text: "PV(rate, nper, pmt, [fv], [type])", description: "Returns the present value of an investment: the total amount that a series of future payments is worth now.", arity: Arity { min: 3, max: 5, step: 1 } })),
    Quartile => ("QUARTILE", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "QUARTILE(array, quart)", description: "Returns the quartile of a data set.", arity: Arity { min: 2, max: 2, step: 1 } })),
    QuartileDotInc => ("QUARTILE.INC", Prefix::Future, false, Some(Signature { category: Category::Statistical, text: "QUARTILE.INC(array, quart)", description: "Returns the quartile of a data set, based on percentile values from 0..1, inclusive.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Quotient => ("QUOTIENT", Prefix::None, false, Some(Signature { category: Category::Math, text: "QUOTIENT(numerator, denominator)", description: "Returns the integer portion of a division.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Radians => ("RADIANS", Prefix::None, false, Some(Signature { category: Category::Math, text: "RADIANS(angle)", description: "Converts degrees to radians.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Rand => ("RAND", Prefix::None, true, Some(Signature { category: Category::Math, text: "RAND()", description: "Returns a random number greater than or equal to 0 and less than 1 (changes on recalculation).", arity: Arity { min: 0, max: 0, step: 1 } })),
    Randarray => ("RANDARRAY", Prefix::Future, false, None),
    Randbetween => ("RANDBETWEEN", Prefix::None, true, Some(Signature { category: Category::Math, text: "RANDBETWEEN(bottom, top)", description: "Returns a random number between the numbers you specify.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Rank => ("RANK", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "RANK(number, ref, [order])", description: "Returns the rank of a number in a list of numbers: its size relative to other values in the list.", arity: Arity { min: 2, max: 3, step: 1 } })),
    RankDotEq => ("RANK.EQ", Prefix::Future, false, Some(Signature { category: Category::Statistical, text: "RANK.EQ(number, ref, [order])", description: "Returns the rank of a number in a list of numbers; if more than one value has the same rank, the top rank of that set is returned.", arity: Arity { min: 2, max: 3, step: 1 } })),
    Replace => ("REPLACE", Prefix::None, false, Some(Signature { category: Category::Text, text: "REPLACE(old_text, start_num, num_chars, new_text)", description: "Replaces part of a text string with a different text string.", arity: Arity { min: 4, max: 4, step: 1 } })),
    Rept => ("REPT", Prefix::None, false, Some(Signature { category: Category::Text, text: "REPT(text, number_times)", description: "Repeats text a given number of times.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Right => ("RIGHT", Prefix::None, false, Some(Signature { category: Category::Text, text: "RIGHT(text, [num_chars])", description: "Returns the specified number of characters from the end of a text string.", arity: Arity { min: 1, max: 2, step: 1 } })),
    Round => ("ROUND", Prefix::None, false, Some(Signature { category: Category::Math, text: "ROUND(number, num_digits)", description: "Rounds a number to a specified number of digits.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Rounddown => ("ROUNDDOWN", Prefix::None, false, Some(Signature { category: Category::Math, text: "ROUNDDOWN(number, num_digits)", description: "Rounds a number down, toward zero.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Roundup => ("ROUNDUP", Prefix::None, false, Some(Signature { category: Category::Math, text: "ROUNDUP(number, num_digits)", description: "Rounds a number up, away from zero.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Row => ("ROW", Prefix::None, false, Some(Signature { category: Category::Lookup, text: "ROW([reference])", description: "Returns the row number of a reference.", arity: Arity { min: 0, max: 1, step: 1 } })),
    Rows => ("ROWS", Prefix::None, false, Some(Signature { category: Category::Lookup, text: "ROWS(array)", description: "Returns the number of rows in a reference or an array.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Search => ("SEARCH", Prefix::None, false, Some(Signature { category: Category::Text, text: "SEARCH(find_text, within_text, [start_num])", description: "Returns the number of the character at which a specific character or text string is first found, reading left to right (not case-sensitive).", arity: Arity { min: 2, max: 3, step: 1 } })),
    Second => ("SECOND", Prefix::None, false, Some(Signature { category: Category::DateTime, text: "SECOND(serial_number)", description: "Returns the second, a number from 0 to 59.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Sequence => ("SEQUENCE", Prefix::Future, false, None),
    Sign => ("SIGN", Prefix::None, false, Some(Signature { category: Category::Math, text: "SIGN(number)", description: "Returns the sign of a number: 1 if positive, zero if zero, or -1 if negative.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Sin => ("SIN", Prefix::None, false, Some(Signature { category: Category::Math, text: "SIN(number)", description: "Returns the sine of an angle.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Small => ("SMALL", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "SMALL(array, k)", description: "Returns the k-th smallest value in a data set.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Sort => ("SORT", Prefix::FutureWorksheet, false, None),
    Sortby => ("SORTBY", Prefix::Future, false, None),
    Sqrt => ("SQRT", Prefix::None, false, Some(Signature { category: Category::Math, text: "SQRT(number)", description: "Returns the square root of a number.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Stdev => ("STDEV", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "STDEV(number1, [number2], ...)", description: "Estimates standard deviation based on a sample (ignores logical values and text in the sample).", arity: Arity { min: 1, max: 255, step: 1 } })),
    StdevDotP => ("STDEV.P", Prefix::Future, false, Some(Signature { category: Category::Statistical, text: "STDEV.P(number1, [number2], ...)", description: "Calculates standard deviation based on the entire population given as arguments (ignores logical values and text).", arity: Arity { min: 1, max: 255, step: 1 } })),
    StdevDotS => ("STDEV.S", Prefix::Future, false, Some(Signature { category: Category::Statistical, text: "STDEV.S(number1, [number2], ...)", description: "Estimates standard deviation based on a sample (ignores logical values and text in the sample).", arity: Arity { min: 1, max: 255, step: 1 } })),
    Stdevp => ("STDEVP", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "STDEVP(number1, [number2], ...)", description: "Calculates standard deviation based on the entire population given as arguments (ignores logical values and text).", arity: Arity { min: 1, max: 255, step: 1 } })),
    Substitute => ("SUBSTITUTE", Prefix::None, false, Some(Signature { category: Category::Text, text: "SUBSTITUTE(text, old_text, new_text, [instance_num])", description: "Replaces existing text with new text in a text string.", arity: Arity { min: 3, max: 4, step: 1 } })),
    Subtotal => ("SUBTOTAL", Prefix::None, false, Some(Signature { category: Category::Math, text: "SUBTOTAL(function_num, ref1, [ref2], ...)", description: "Returns a subtotal in a list or database.", arity: Arity { min: 2, max: 255, step: 1 } })),
    Sum => ("SUM", Prefix::None, false, Some(Signature { category: Category::Math, text: "SUM(number1, [number2], ...)", description: "Adds all the numbers in a range of cells.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Sumif => ("SUMIF", Prefix::None, false, Some(Signature { category: Category::Math, text: "SUMIF(range, criteria, [sum_range])", description: "Adds the cells specified by a given condition or criteria.", arity: Arity { min: 2, max: 3, step: 1 } })),
    Sumifs => ("SUMIFS", Prefix::None, false, Some(Signature { category: Category::Math, text: "SUMIFS(sum_range, criteria_range1, criteria1, [criteria_range2, criteria2], ...)", description: "Adds the cells specified by a given set of conditions or criteria.", arity: Arity { min: 3, max: 255, step: 2 } })),
    Sumproduct => ("SUMPRODUCT", Prefix::None, false, Some(Signature { category: Category::Math, text: "SUMPRODUCT(array1, [array2], [array3], ...)", description: "Returns the sum of the products of corresponding ranges or arrays.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Switch => ("SWITCH", Prefix::Future, false, Some(Signature { category: Category::Logical, text: "SWITCH(expression, value1, result1, [default_or_value2, result2], ...)", description: "Evaluates an expression against a list of values and returns the result corresponding to the first matching value.", arity: Arity { min: 3, max: 254, step: 1 } })),
    T => ("T", Prefix::None, false, Some(Signature { category: Category::Text, text: "T(value)", description: "Checks whether a value is text, and returns the text if it is, or returns double quotes (empty text) if it is not.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Tan => ("TAN", Prefix::None, false, Some(Signature { category: Category::Math, text: "TAN(number)", description: "Returns the tangent of an angle.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Text => ("TEXT", Prefix::None, false, Some(Signature { category: Category::Text, text: "TEXT(value, format_text)", description: "Converts a value to text in a specific number format.", arity: Arity { min: 2, max: 2, step: 1 } })),
    Textjoin => ("TEXTJOIN", Prefix::Future, false, Some(Signature { category: Category::Text, text: "TEXTJOIN(delimiter, ignore_empty, text1, [text2], ...)", description: "Concatenates a list or range of text strings using a delimiter.", arity: Arity { min: 3, max: 255, step: 1 } })),
    Time => ("TIME", Prefix::None, false, Some(Signature { category: Category::DateTime, text: "TIME(hour, minute, second)", description: "Converts hours, minutes, and seconds given as numbers to a serial number, formatted with a time format.", arity: Arity { min: 3, max: 3, step: 1 } })),
    Timevalue => ("TIMEVALUE", Prefix::None, false, Some(Signature { category: Category::DateTime, text: "TIMEVALUE(time_text)", description: "Converts a text time to a serial number for a time, a number from 0 (12:00:00 AM) to 0.999988426 (11:59:59 PM).", arity: Arity { min: 1, max: 1, step: 1 } })),
    Today => ("TODAY", Prefix::None, true, Some(Signature { category: Category::DateTime, text: "TODAY()", description: "Returns the current date formatted as a date.", arity: Arity { min: 0, max: 0, step: 1 } })),
    Trim => ("TRIM", Prefix::None, false, Some(Signature { category: Category::Text, text: "TRIM(text)", description: "Removes all spaces from a text string except for single spaces between words.", arity: Arity { min: 1, max: 1, step: 1 } })),
    True => ("TRUE", Prefix::None, false, Some(Signature { category: Category::Logical, text: "TRUE()", description: "Returns the logical value TRUE.", arity: Arity { min: 0, max: 0, step: 1 } })),
    Trunc => ("TRUNC", Prefix::None, false, Some(Signature { category: Category::Math, text: "TRUNC(number, [num_digits])", description: "Truncates a number to an integer by removing the decimal, or fractional, part of the number.", arity: Arity { min: 1, max: 2, step: 1 } })),
    Unique => ("UNIQUE", Prefix::Future, false, None),
    Upper => ("UPPER", Prefix::None, false, Some(Signature { category: Category::Text, text: "UPPER(text)", description: "Converts a text string to all uppercase letters.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Value => ("VALUE", Prefix::None, false, Some(Signature { category: Category::Text, text: "VALUE(text)", description: "Converts a text string that represents a number to a number.", arity: Arity { min: 1, max: 1, step: 1 } })),
    Var => ("VAR", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "VAR(number1, [number2], ...)", description: "Estimates variance based on a sample (ignores logical values and text in the sample).", arity: Arity { min: 1, max: 255, step: 1 } })),
    VarDotP => ("VAR.P", Prefix::Future, false, Some(Signature { category: Category::Statistical, text: "VAR.P(number1, [number2], ...)", description: "Calculates variance based on the entire population (ignores logical values and text in the population).", arity: Arity { min: 1, max: 255, step: 1 } })),
    VarDotS => ("VAR.S", Prefix::Future, false, Some(Signature { category: Category::Statistical, text: "VAR.S(number1, [number2], ...)", description: "Estimates variance based on a sample (ignores logical values and text in the sample).", arity: Arity { min: 1, max: 255, step: 1 } })),
    Varp => ("VARP", Prefix::None, false, Some(Signature { category: Category::Statistical, text: "VARP(number1, [number2], ...)", description: "Calculates variance based on the entire population (ignores logical values and text in the population).", arity: Arity { min: 1, max: 255, step: 1 } })),
    Vlookup => ("VLOOKUP", Prefix::None, false, Some(Signature { category: Category::Lookup, text: "VLOOKUP(lookup_value, table_array, col_index_num, [range_lookup])", description: "Looks for a value in the leftmost column of a table, and then returns a value in the same row from a column you specify.", arity: Arity { min: 3, max: 4, step: 1 } })),
    Weekday => ("WEEKDAY", Prefix::None, false, Some(Signature { category: Category::DateTime, text: "WEEKDAY(serial_number, [return_type])", description: "Returns a number from 1 to 7 identifying the day of the week of a date.", arity: Arity { min: 1, max: 2, step: 1 } })),
    Xlookup => ("XLOOKUP", Prefix::Future, false, Some(Signature { category: Category::Lookup, text: "XLOOKUP(lookup_value, lookup_array, return_array, [if_not_found], [match_mode], [search_mode])", description: "Searches a range or an array for a match and returns the corresponding item from a second range or array.", arity: Arity { min: 3, max: 6, step: 1 } })),
    Xmatch => ("XMATCH", Prefix::Future, false, None),
    Xor => ("XOR", Prefix::Future, false, Some(Signature { category: Category::Logical, text: "XOR(logical1, [logical2], ...)", description: "Returns a logical 'Exclusive Or' of all arguments.", arity: Arity { min: 1, max: 255, step: 1 } })),
    Year => ("YEAR", Prefix::None, false, Some(Signature { category: Category::DateTime, text: "YEAR(serial_number)", description: "Returns the year of a date, an integer in the range 1900-9999.", arity: Arity { min: 1, max: 1, step: 1 } })),
}

impl Function {
    pub(crate) fn lookup(name: &str) -> Option<Self> {
        FUNCTIONS
            .binary_search_by(|item| {
                item.name
                    .bytes()
                    .cmp(name.bytes().map(|byte| byte.to_ascii_uppercase()))
            })
            .ok()
            .map(|index| FUNCTIONS[index].function)
    }

    pub(crate) const fn info(self) -> &'static FunctionInfo {
        &FUNCTIONS[self as usize]
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! The mirrored test reads the private catalog, without a caller-facing API.
    use super::{FUNCTIONS, Function, FunctionInfo};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct Fact {
        pub name: &'static str,
        pub category: Option<&'static str>,
        pub signature: Option<&'static str>,
        pub description: Option<&'static str>,
        pub arity: Option<(u8, u8, u8)>,
        pub prefix: &'static str,
        pub volatile: bool,
    }

    fn fact(info: &FunctionInfo) -> Fact {
        let signature = info.signature;
        Fact {
            name: info.name,
            category: signature.map(|value| value.category.label()),
            signature: signature.map(|value| value.text),
            description: signature.map(|value| value.description),
            arity: signature.map(|value| (value.arity.min, value.arity.max, value.arity.step)),
            prefix: info.prefix.as_str(),
            volatile: info.volatile,
        }
    }

    pub fn catalog() -> Vec<Fact> {
        FUNCTIONS.iter().map(fact).collect()
    }
    pub fn lookup(name: &str) -> Option<Fact> {
        Function::lookup(name).map(|found| fact(found.info()))
    }
    pub fn accepts(name: &str, count: usize) -> Option<bool> {
        Function::lookup(name)?
            .info()
            .signature
            .map(|value| value.arity.accepts(count))
    }
}
