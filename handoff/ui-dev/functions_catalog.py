"""The dev server's `GET functions` answer: design §5.6's 159 functions, each
with its category, Excel's signature spelling and a one-line description
(scratchpad, never committed). gen_fixtures.py writes it to
fixtures/functions.json; `python functions_catalog.py` checks the counts.
"""

MATH = "Math and trigonometry"
STAT = "Statistical"
LOGIC = "Logical"
LOOKUP = "Lookup and reference"
TEXT = "Text"
DATE = "Date and time"
INFO = "Information"
FIN = "Financial"

FUNCTIONS = [
    # Math and trigonometry (43)
    (MATH, "ABS(number)", "Returns the absolute value of a number, a number without its sign."),
    (MATH, "ACOS(number)", "Returns the arccosine of a number, in radians from 0 to Pi."),
    (MATH, "ASIN(number)", "Returns the arcsine of a number, in radians from -Pi/2 to Pi/2."),
    (MATH, "ATAN(number)", "Returns the arctangent of a number, in radians from -Pi/2 to Pi/2."),
    (MATH, "ATAN2(x_num, y_num)", "Returns the arctangent of the x- and y-coordinates, in radians from -Pi to Pi, excluding -Pi."),
    (MATH, "CEILING(number, significance)", "Rounds a number up, to the nearest multiple of significance."),
    (MATH, "CEILING.MATH(number, [significance], [mode])", "Rounds a number up, to the nearest integer or to the nearest multiple of significance."),
    (MATH, "COS(number)", "Returns the cosine of an angle."),
    (MATH, "DEGREES(angle)", "Converts radians to degrees."),
    (MATH, "EVEN(number)", "Rounds a positive number up and a negative number down to the nearest even integer."),
    (MATH, "EXP(number)", "Returns e raised to the power of a given number."),
    (MATH, "FACT(number)", "Returns the factorial of a number, equal to 1*2*3*...*number."),
    (MATH, "FLOOR(number, significance)", "Rounds a number down to the nearest multiple of significance."),
    (MATH, "FLOOR.MATH(number, [significance], [mode])", "Rounds a number down, to the nearest integer or to the nearest multiple of significance."),
    (MATH, "GCD(number1, [number2], ...)", "Returns the greatest common divisor."),
    (MATH, "INT(number)", "Rounds a number down to the nearest integer."),
    (MATH, "LCM(number1, [number2], ...)", "Returns the least common multiple."),
    (MATH, "LN(number)", "Returns the natural logarithm of a number."),
    (MATH, "LOG(number, [base])", "Returns the logarithm of a number to the base you specify."),
    (MATH, "LOG10(number)", "Returns the base-10 logarithm of a number."),
    (MATH, "MOD(number, divisor)", "Returns the remainder after a number is divided by a divisor."),
    (MATH, "MROUND(number, multiple)", "Returns a number rounded to the desired multiple."),
    (MATH, "ODD(number)", "Rounds a positive number up and a negative number down to the nearest odd integer."),
    (MATH, "PI()", "Returns the value of Pi, 3.14159265358979, accurate to 15 digits."),
    (MATH, "POWER(number, power)", "Returns the result of a number raised to a power."),
    (MATH, "PRODUCT(number1, [number2], ...)", "Multiplies all the numbers given as arguments."),
    (MATH, "QUOTIENT(numerator, denominator)", "Returns the integer portion of a division."),
    (MATH, "RADIANS(angle)", "Converts degrees to radians."),
    (MATH, "RAND()", "Returns a random number greater than or equal to 0 and less than 1 (changes on recalculation)."),
    (MATH, "RANDBETWEEN(bottom, top)", "Returns a random number between the numbers you specify."),
    (MATH, "ROUND(number, num_digits)", "Rounds a number to a specified number of digits."),
    (MATH, "ROUNDDOWN(number, num_digits)", "Rounds a number down, toward zero."),
    (MATH, "ROUNDUP(number, num_digits)", "Rounds a number up, away from zero."),
    (MATH, "SIGN(number)", "Returns the sign of a number: 1 if positive, zero if zero, or -1 if negative."),
    (MATH, "SIN(number)", "Returns the sine of an angle."),
    (MATH, "SQRT(number)", "Returns the square root of a number."),
    (MATH, "SUBTOTAL(function_num, ref1, [ref2], ...)", "Returns a subtotal in a list or database."),
    (MATH, "SUM(number1, [number2], ...)", "Adds all the numbers in a range of cells."),
    (MATH, "SUMIF(range, criteria, [sum_range])", "Adds the cells specified by a given condition or criteria."),
    (MATH, "SUMIFS(sum_range, criteria_range1, criteria1, [criteria_range2, criteria2], ...)", "Adds the cells specified by a given set of conditions or criteria."),
    (MATH, "SUMPRODUCT(array1, [array2], [array3], ...)", "Returns the sum of the products of corresponding ranges or arrays."),
    (MATH, "TAN(number)", "Returns the tangent of an angle."),
    (MATH, "TRUNC(number, [num_digits])", "Truncates a number to an integer by removing the decimal, or fractional, part of the number."),
    # Statistical (34)
    (STAT, "AVERAGE(number1, [number2], ...)", "Returns the average (arithmetic mean) of its arguments."),
    (STAT, "AVERAGEA(value1, [value2], ...)", "Returns the average of its arguments, evaluating text and FALSE in arguments as 0; TRUE evaluates as 1."),
    (STAT, "AVERAGEIF(range, criteria, [average_range])", "Finds the average (arithmetic mean) for the cells specified by a given condition or criteria."),
    (STAT, "AVERAGEIFS(average_range, criteria_range1, criteria1, [criteria_range2, criteria2], ...)", "Finds the average (arithmetic mean) for the cells specified by a given set of conditions or criteria."),
    (STAT, "COUNT(value1, [value2], ...)", "Counts the number of cells in a range that contain numbers."),
    (STAT, "COUNTA(value1, [value2], ...)", "Counts the number of cells in a range that are not empty."),
    (STAT, "COUNTBLANK(range)", "Counts the number of empty cells in a specified range of cells."),
    (STAT, "COUNTIF(range, criteria)", "Counts the number of cells within a range that meet the given condition."),
    (STAT, "COUNTIFS(criteria_range1, criteria1, [criteria_range2, criteria2], ...)", "Counts the number of cells specified by a given set of conditions or criteria."),
    (STAT, "LARGE(array, k)", "Returns the k-th largest value in a data set."),
    (STAT, "MAX(number1, [number2], ...)", "Returns the largest value in a set of values. Ignores logical values and text."),
    (STAT, "MAXA(value1, [value2], ...)", "Returns the largest value in a set of values. Does not ignore logical values and text."),
    (STAT, "MAXIFS(max_range, criteria_range1, criteria1, [criteria_range2, criteria2], ...)", "Returns the maximum value among cells specified by a given set of conditions or criteria."),
    (STAT, "MEDIAN(number1, [number2], ...)", "Returns the median, or the number in the middle of the set of given numbers."),
    (STAT, "MIN(number1, [number2], ...)", "Returns the smallest number in a set of values. Ignores logical values and text."),
    (STAT, "MINA(value1, [value2], ...)", "Returns the smallest value in a set of values. Does not ignore logical values and text."),
    (STAT, "MINIFS(min_range, criteria_range1, criteria1, [criteria_range2, criteria2], ...)", "Returns the minimum value among cells specified by a given set of conditions or criteria."),
    (STAT, "MODE(number1, [number2], ...)", "Returns the most frequently occurring, or repetitive, value in an array or range of data."),
    (STAT, "MODE.SNGL(number1, [number2], ...)", "Returns the most frequently occurring, or repetitive, value in an array or range of data."),
    (STAT, "PERCENTILE(array, k)", "Returns the k-th percentile of values in a range."),
    (STAT, "PERCENTILE.INC(array, k)", "Returns the k-th percentile of values in a range, where k is in the range 0..1, inclusive."),
    (STAT, "QUARTILE(array, quart)", "Returns the quartile of a data set."),
    (STAT, "QUARTILE.INC(array, quart)", "Returns the quartile of a data set, based on percentile values from 0..1, inclusive."),
    (STAT, "RANK(number, ref, [order])", "Returns the rank of a number in a list of numbers: its size relative to other values in the list."),
    (STAT, "RANK.EQ(number, ref, [order])", "Returns the rank of a number in a list of numbers; if more than one value has the same rank, the top rank of that set is returned."),
    (STAT, "SMALL(array, k)", "Returns the k-th smallest value in a data set."),
    (STAT, "STDEV(number1, [number2], ...)", "Estimates standard deviation based on a sample (ignores logical values and text in the sample)."),
    (STAT, "STDEV.P(number1, [number2], ...)", "Calculates standard deviation based on the entire population given as arguments (ignores logical values and text)."),
    (STAT, "STDEV.S(number1, [number2], ...)", "Estimates standard deviation based on a sample (ignores logical values and text in the sample)."),
    (STAT, "STDEVP(number1, [number2], ...)", "Calculates standard deviation based on the entire population given as arguments (ignores logical values and text)."),
    (STAT, "VAR(number1, [number2], ...)", "Estimates variance based on a sample (ignores logical values and text in the sample)."),
    (STAT, "VAR.P(number1, [number2], ...)", "Calculates variance based on the entire population (ignores logical values and text in the population)."),
    (STAT, "VAR.S(number1, [number2], ...)", "Estimates variance based on a sample (ignores logical values and text in the sample)."),
    (STAT, "VARP(number1, [number2], ...)", "Calculates variance based on the entire population (ignores logical values and text in the population)."),
    # Logical (11)
    (LOGIC, "AND(logical1, [logical2], ...)", "Checks whether all arguments are TRUE, and returns TRUE if all arguments are TRUE."),
    (LOGIC, "FALSE()", "Returns the logical value FALSE."),
    (LOGIC, "IF(logical_test, [value_if_true], [value_if_false])", "Checks whether a condition is met, and returns one value if TRUE, and another value if FALSE."),
    (LOGIC, "IFERROR(value, value_if_error)", "Returns value_if_error if expression is an error and the value of the expression itself otherwise."),
    (LOGIC, "IFNA(value, value_if_na)", "Returns the value you specify if the expression resolves to #N/A, otherwise returns the result of the expression."),
    (LOGIC, "IFS(logical_test1, value_if_true1, [logical_test2, value_if_true2], ...)", "Checks whether one or more conditions are met and returns a value corresponding to the first TRUE condition."),
    (LOGIC, "NOT(logical)", "Changes FALSE to TRUE, or TRUE to FALSE."),
    (LOGIC, "OR(logical1, [logical2], ...)", "Checks whether any of the arguments are TRUE, and returns TRUE or FALSE. Returns FALSE only if all arguments are FALSE."),
    (LOGIC, "SWITCH(expression, value1, result1, [default_or_value2, result2], ...)", "Evaluates an expression against a list of values and returns the result corresponding to the first matching value."),
    (LOGIC, "TRUE()", "Returns the logical value TRUE."),
    (LOGIC, "XOR(logical1, [logical2], ...)", "Returns a logical 'Exclusive Or' of all arguments."),
    # Lookup and reference (14)
    (LOOKUP, "ADDRESS(row_num, column_num, [abs_num], [a1], [sheet_text])", "Creates a cell reference as text, given specified row and column numbers."),
    (LOOKUP, "CHOOSE(index_num, value1, [value2], ...)", "Chooses a value or action to perform from a list of values, based on an index number."),
    (LOOKUP, "COLUMN([reference])", "Returns the column number of a reference."),
    (LOOKUP, "COLUMNS(array)", "Returns the number of columns in an array or reference."),
    (LOOKUP, "HLOOKUP(lookup_value, table_array, row_index_num, [range_lookup])", "Looks for a value in the top row of a table or array of values and returns the value in the same column from a row you specify."),
    (LOOKUP, "INDEX(array, row_num, [column_num])", "Returns a value or reference of the cell at the intersection of a particular row and column, in a given range."),
    (LOOKUP, "INDIRECT(ref_text, [a1])", "Returns the reference specified by a text string."),
    (LOOKUP, "LOOKUP(lookup_value, lookup_vector, [result_vector])", "Looks up a value either from a one-row or one-column range."),
    (LOOKUP, "MATCH(lookup_value, lookup_array, [match_type])", "Returns the relative position of an item in an array that matches a specified value in a specified order."),
    (LOOKUP, "OFFSET(reference, rows, cols, [height], [width])", "Returns a reference to a range that is a given number of rows and columns from a given reference."),
    (LOOKUP, "ROW([reference])", "Returns the row number of a reference."),
    (LOOKUP, "ROWS(array)", "Returns the number of rows in a reference or an array."),
    (LOOKUP, "VLOOKUP(lookup_value, table_array, col_index_num, [range_lookup])", "Looks for a value in the leftmost column of a table, and then returns a value in the same row from a column you specify."),
    (LOOKUP, "XLOOKUP(lookup_value, lookup_array, return_array, [if_not_found], [match_mode], [search_mode])", "Searches a range or an array for a match and returns the corresponding item from a second range or array."),
    # Text (23)
    (TEXT, "CHAR(number)", "Returns the character specified by the code number from the character set for your computer."),
    (TEXT, "CLEAN(text)", "Removes all nonprintable characters from text."),
    (TEXT, "CODE(text)", "Returns a numeric code for the first character in a text string."),
    (TEXT, "CONCAT(text1, [text2], ...)", "Concatenates a list or range of text strings."),
    (TEXT, "CONCATENATE(text1, [text2], ...)", "Joins several text strings into one text string."),
    (TEXT, "EXACT(text1, text2)", "Checks whether two text strings are exactly the same, and returns TRUE or FALSE. EXACT is case-sensitive."),
    (TEXT, "FIND(find_text, within_text, [start_num])", "Returns the starting position of one text string within another text string. FIND is case-sensitive."),
    (TEXT, "LEFT(text, [num_chars])", "Returns the specified number of characters from the start of a text string."),
    (TEXT, "LEN(text)", "Returns the number of characters in a text string."),
    (TEXT, "LOWER(text)", "Converts all letters in a text string to lowercase."),
    (TEXT, "MID(text, start_num, num_chars)", "Returns the characters from the middle of a text string, given a starting position and length."),
    (TEXT, "PROPER(text)", "Converts a text string to proper case; the first letter in each word to uppercase, and all other letters to lowercase."),
    (TEXT, "REPLACE(old_text, start_num, num_chars, new_text)", "Replaces part of a text string with a different text string."),
    (TEXT, "REPT(text, number_times)", "Repeats text a given number of times."),
    (TEXT, "RIGHT(text, [num_chars])", "Returns the specified number of characters from the end of a text string."),
    (TEXT, "SEARCH(find_text, within_text, [start_num])", "Returns the number of the character at which a specific character or text string is first found, reading left to right (not case-sensitive)."),
    (TEXT, "SUBSTITUTE(text, old_text, new_text, [instance_num])", "Replaces existing text with new text in a text string."),
    (TEXT, "T(value)", "Checks whether a value is text, and returns the text if it is, or returns double quotes (empty text) if it is not."),
    (TEXT, "TEXT(value, format_text)", "Converts a value to text in a specific number format."),
    (TEXT, "TEXTJOIN(delimiter, ignore_empty, text1, [text2], ...)", "Concatenates a list or range of text strings using a delimiter."),
    (TEXT, "TRIM(text)", "Removes all spaces from a text string except for single spaces between words."),
    (TEXT, "UPPER(text)", "Converts a text string to all uppercase letters."),
    (TEXT, "VALUE(text)", "Converts a text string that represents a number to a number."),
    # Date and time (16)
    (DATE, "DATE(year, month, day)", "Returns the number that represents the date in the date-time code."),
    (DATE, "DATEVALUE(date_text)", "Converts a date in the form of text to a number that represents the date in the date-time code."),
    (DATE, "DAY(serial_number)", "Returns the day of the month, a number from 1 to 31."),
    (DATE, "DAYS(end_date, start_date)", "Returns the number of days between the two dates."),
    (DATE, "EDATE(start_date, months)", "Returns the serial number of the date that is the indicated number of months before or after the start date."),
    (DATE, "EOMONTH(start_date, months)", "Returns the serial number of the last day of the month before or after a specified number of months."),
    (DATE, "HOUR(serial_number)", "Returns the hour as a number from 0 (12:00 A.M.) to 23 (11:00 P.M.)."),
    (DATE, "MINUTE(serial_number)", "Returns the minute, a number from 0 to 59."),
    (DATE, "MONTH(serial_number)", "Returns the month, a number from 1 (January) to 12 (December)."),
    (DATE, "NOW()", "Returns the current date and time formatted as a date and time."),
    (DATE, "SECOND(serial_number)", "Returns the second, a number from 0 to 59."),
    (DATE, "TIME(hour, minute, second)", "Converts hours, minutes, and seconds given as numbers to a serial number, formatted with a time format."),
    (DATE, "TIMEVALUE(time_text)", "Converts a text time to a serial number for a time, a number from 0 (12:00:00 AM) to 0.999988426 (11:59:59 PM)."),
    (DATE, "TODAY()", "Returns the current date formatted as a date."),
    (DATE, "WEEKDAY(serial_number, [return_type])", "Returns a number from 1 to 7 identifying the day of the week of a date."),
    (DATE, "YEAR(serial_number)", "Returns the year of a date, an integer in the range 1900-9999."),
    # Information (14)
    (INFO, "ERROR.TYPE(error_val)", "Returns a number matching an error value."),
    (INFO, "ISBLANK(value)", "Checks whether a reference is to an empty cell, and returns TRUE or FALSE."),
    (INFO, "ISERR(value)", "Checks whether a value is an error other than #N/A, and returns TRUE or FALSE."),
    (INFO, "ISERROR(value)", "Checks whether a value is an error, and returns TRUE or FALSE."),
    (INFO, "ISEVEN(number)", "Returns TRUE if the number is even."),
    (INFO, "ISLOGICAL(value)", "Checks whether a value is a logical value (TRUE or FALSE), and returns TRUE or FALSE."),
    (INFO, "ISNA(value)", "Checks whether a value is #N/A, and returns TRUE or FALSE."),
    (INFO, "ISNONTEXT(value)", "Checks whether a value is not text (blank cells are not text), and returns TRUE or FALSE."),
    (INFO, "ISNUMBER(value)", "Checks whether a value is a number, and returns TRUE or FALSE."),
    (INFO, "ISODD(number)", "Returns TRUE if the number is odd."),
    (INFO, "ISREF(value)", "Checks whether a value is a reference, and returns TRUE or FALSE."),
    (INFO, "ISTEXT(value)", "Checks whether a value is text, and returns TRUE or FALSE."),
    (INFO, "N(value)", "Converts non-number value to a number, dates to serial numbers, TRUE to 1, anything else to 0 (zero)."),
    (INFO, "NA()", "Returns the error value #N/A (value not available)."),
    # Financial (4)
    (FIN, "FV(rate, nper, pmt, [pv], [type])", "Returns the future value of an investment based on periodic, constant payments and a constant interest rate."),
    (FIN, "NPV(rate, value1, [value2], ...)", "Returns the net present value of an investment based on a discount rate and a series of future payments (negative values) and income (positive values)."),
    (FIN, "PMT(rate, nper, pv, [fv], [type])", "Calculates the payment for a loan based on constant payments and a constant interest rate."),
    (FIN, "PV(rate, nper, pmt, [fv], [type])", "Returns the present value of an investment: the total amount that a series of future payments is worth now."),
]

COUNTS = {MATH: 43, STAT: 34, LOGIC: 11, LOOKUP: 14, TEXT: 23, DATE: 16, INFO: 14, FIN: 4}


def catalog():
    """The §8.3 `functions` answer, in the registry's order."""
    return [{"name": signature[:signature.index("(")], "category": category, "signature": signature,
             "description": description} for category, signature, description in FUNCTIONS]


if __name__ == "__main__":
    items = catalog()
    names = [item["name"] for item in items]
    assert len(names) == len(set(names)) == 159, len(names)
    for category, count in COUNTS.items():
        got = sum(item["category"] == category for item in items)
        assert got == count, (category, got, count)
    print(f"{len(items)} functions in {len(COUNTS)} categories")
