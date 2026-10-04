//! `rust/src/spill.rs`: the bound a column stays resident under, and the
//! walk that writes a column's buffers to a private file and maps them back.
//!
//! `SpillOptions` is what a caller states, so it is pinned at the top level
//! and runs in a default build. The walk (`spill_array`), the mapping it
//! answers and the pure reading of the two environment variables
//! (`parse_env`) are crate-private, reached through
//! `yggdryl::internals::spill` in `mod internal`.

use yggdryl::local::LocalFolder;
use yggdryl::{DEFAULT_SPILL_BYTE_SIZE, SpillOptions};

#[test]
fn new_options_are_the_default_bound_over_the_platform_temporary_folder() {
    let options = SpillOptions::new();
    assert_eq!(DEFAULT_SPILL_BYTE_SIZE, 64 * 1024 * 1024);
    assert_eq!(options.byte_size(), DEFAULT_SPILL_BYTE_SIZE);
    assert!(options.folder().is_none());
    assert!(!options.is_never());
    assert_eq!(SpillOptions::default(), options);
}

#[test]
fn with_byte_size_and_with_folder_each_change_one_fact() -> yggdryl::Result<()> {
    let folder = LocalFolder::new("/tmp/yggdryl-spill-options-a")?;
    let options = SpillOptions::new()
        .with_byte_size(4096)
        .with_folder(folder.clone());
    assert_eq!(options.byte_size(), 4096);
    assert_eq!(
        options.folder().map(|held| held.url().to_string()),
        Some(folder.url().to_string())
    );

    // Each setter leaves the other fact where it was.
    let bound = SpillOptions::new().with_byte_size(4096);
    assert!(bound.folder().is_none());
    let placed = SpillOptions::new().with_folder(folder);
    assert_eq!(placed.byte_size(), DEFAULT_SPILL_BYTE_SIZE);

    // A clone is the same options.
    assert_eq!(options.clone(), options);
    Ok(())
}

#[test]
fn never_is_the_largest_bound_and_zero_spills_everything_rather_than_nothing() {
    assert_eq!(SpillOptions::NEVER, u64::MAX);
    assert!(
        SpillOptions::new()
            .with_byte_size(SpillOptions::NEVER)
            .is_never()
    );
    assert!(!SpillOptions::new().with_byte_size(0).is_never());
    assert!(
        !SpillOptions::new()
            .with_byte_size(SpillOptions::NEVER - 1)
            .is_never()
    );
}

#[test]
fn options_over_one_folder_path_are_equal_and_over_two_are_not() -> yggdryl::Result<()> {
    let over = |path: &str| -> yggdryl::Result<SpillOptions> {
        Ok(SpillOptions::new().with_folder(LocalFolder::new(path)?))
    };
    // Two handles built apart over one path name one location.
    assert_eq!(
        over("/tmp/yggdryl-spill-options-a")?,
        over("/tmp/yggdryl-spill-options-a")?
    );
    assert_ne!(
        over("/tmp/yggdryl-spill-options-a")?,
        over("/tmp/yggdryl-spill-options-b")?
    );
    // A stated folder is not the platform default, even when it is the same
    // directory: `None` is the absence of a statement, not a location.
    assert_ne!(over("/tmp/yggdryl-spill-options-a")?, SpillOptions::new());
    // The bound takes part too.
    assert_ne!(
        over("/tmp/yggdryl-spill-options-a")?,
        over("/tmp/yggdryl-spill-options-a")?.with_byte_size(1)
    );
    assert_ne!(SpillOptions::new(), SpillOptions::new().with_byte_size(0));
    Ok(())
}

#[test]
fn install_env_settles_the_process_default_once_and_refuses_a_second() {
    // The process default is one `OnceLock` shared by every test of this
    // harness, and the tests run on parallel threads in no fixed order: a
    // test landing a column through `Serie` may resolve the default from the
    // environment before this one installs, so the first install may
    // already conflict. Either way the root harness never spills - `NEVER`
    // is installed, or the 64 MiB default stands, which no root test's
    // column reaches - so what is pinned is the rule, not who won.
    let first = SpillOptions::install_env(SpillOptions::new().with_byte_size(SpillOptions::NEVER));
    let resolved = SpillOptions::from_env().expect("the process default resolves");
    match &first {
        Ok(()) => assert!(resolved.is_never(), "{resolved:?}"),
        Err(error) => assert!(error.to_string().contains("already resolved"), "{error}"),
    }
    // Every later call answers the very value the first one settled.
    let again = SpillOptions::from_env().expect("the process default resolves");
    assert!(std::ptr::eq(resolved, again));
    assert_eq!(resolved, again);

    // Whatever happened first, the default is settled now, and the value
    // every caller saw cannot change underneath them.
    let error = SpillOptions::install_env(SpillOptions::new())
        .expect_err("a second install conflicts with the settled default");
    assert!(error.is_conflict(), "{error:?}");
    assert!(error.to_string().contains("already resolved"), "{error}");
    assert_eq!(
        SpillOptions::from_env().expect("the process default resolves"),
        resolved
    );
}

#[cfg(feature = "internals")]
mod internal {
    use std::ffi::OsString;
    use std::sync::Arc;

    use arrow_array::builder::{Int64Builder, ListBuilder, MapBuilder, StringBuilder};
    use arrow_array::cast::AsArray as _;
    use arrow_array::types::{Int32Type, Int64Type};
    use arrow_array::{
        Array, ArrayRef, BooleanArray, Decimal128Array, DictionaryArray, FixedSizeBinaryArray,
        Float64Array, Int32Array, Int64Array, RunArray, StringArray, StringViewArray, StructArray,
        UnionArray,
    };
    use arrow_buffer::{NullBuffer, ScalarBuffer};
    use arrow_data::ArrayData;
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields, UnionFields};
    use yggdryl::arrow::array_memory_size;
    use yggdryl::internals::spill::{Mapping, parse_env, spill_array};
    use yggdryl::local::LocalFolder;
    use yggdryl::{ArrowCastOptions, DEFAULT_SPILL_BYTE_SIZE, Serie, SpillOptions};

    /// One array of every layout the walk reaches: flat, bit-packed at a bit
    /// offset, fixed width, offsets, views, lists cut past their first
    /// offset, maps, structs with their own validity, dictionaries, run-end
    /// and dense union.
    fn layouts() -> Vec<(&'static str, ArrayRef)> {
        let int64: ArrayRef = Arc::new(Int64Array::from(vec![
            Some(1),
            None,
            Some(-3),
            Some(i64::MAX),
            None,
        ]));

        // Twenty rows cut at three: the validity and the values both read
        // from bit three of their first byte.
        let booleans: BooleanArray = (0..20)
            .map(|row| (row % 5 != 4).then_some(row % 3 == 0))
            .collect();
        let boolean: ArrayRef = Arc::new(booleans.slice(3, 11));

        let decimal: ArrayRef = Arc::new(
            Decimal128Array::from(vec![Some(12_345_i128), None, Some(-12_345_678_901)])
                .with_precision_and_scale(12, 4)
                .expect("a decimal128(12, 4)"),
        );

        let utf8: ArrayRef = Arc::new(StringArray::from(vec![
            Some("alpha"),
            None,
            Some(""),
            Some("\u{3b4}\u{3ad}\u{3bb}\u{3c4}\u{3b1}"),
        ]));

        // One run held inline in its view, one past twelve bytes in a data
        // buffer.
        let utf8_view: ArrayRef = Arc::new(StringViewArray::from(vec![
            Some("inline"),
            Some("a run longer than twelve bytes"),
            None,
            Some(""),
        ]));

        let fixed: ArrayRef = Arc::new(
            FixedSizeBinaryArray::try_from_sparse_iter_with_size(
                vec![Some([1_u8; 16]), None, Some([0xab_u8; 16])].into_iter(),
                16,
            )
            .expect("a fixed_size_binary(16)"),
        );

        // Cut at one, so the offsets the cut list reads start past zero.
        let mut lists = ListBuilder::new(StringBuilder::new());
        lists.values().append_value("a");
        lists.values().append_value("b");
        lists.append(true);
        lists.append(false);
        lists.values().append_value("c");
        lists.values().append_null();
        lists.values().append_value("e");
        lists.append(true);
        lists.values().append_value("f");
        lists.append(true);
        let list: ArrayRef = Arc::new(lists.finish().slice(1, 3));

        let mut maps = MapBuilder::new(None, StringBuilder::new(), Int64Builder::new());
        maps.keys().append_value("bid");
        maps.values().append_value(10);
        maps.keys().append_value("ask");
        maps.values().append_null();
        maps.append(true).expect("a map row");
        maps.append(false).expect("a null map row");
        maps.keys().append_value("last");
        maps.values().append_value(-4);
        maps.append(true).expect("a map row");
        let map: ArrayRef = Arc::new(maps.finish());

        let structure: ArrayRef = Arc::new(
            StructArray::try_new(
                Fields::from(vec![
                    ArrowField::new("a", ArrowDataType::Int64, true),
                    ArrowField::new("b", ArrowDataType::Utf8, true),
                ]),
                vec![
                    Arc::new(Int64Array::from(vec![Some(1), None, Some(3)])) as ArrayRef,
                    Arc::new(StringArray::from(vec![Some("x"), None, None])) as ArrayRef,
                ],
                Some(NullBuffer::from(vec![true, false, true])),
            )
            .expect("a struct<a: int64, b: utf8>"),
        );

        let dictionary: ArrayRef = Arc::new(
            vec![Some("buy"), Some("sell"), None, Some("buy")]
                .into_iter()
                .collect::<DictionaryArray<Int32Type>>(),
        );

        let runs: ArrayRef = Arc::new(
            RunArray::<Int32Type>::try_new(
                &Int32Array::from(vec![2, 5, 6]),
                &Int64Array::from(vec![Some(7), None, Some(9)]),
            )
            .expect("a run-end int64"),
        );

        let union: ArrayRef = Arc::new(
            UnionArray::try_new(
                UnionFields::try_new(
                    vec![0_i8, 1],
                    vec![
                        ArrowField::new("int", ArrowDataType::Int32, false),
                        ArrowField::new("text", ArrowDataType::Utf8, false),
                    ],
                )
                .expect("two union members"),
                ScalarBuffer::from(vec![0_i8, 1, 0, 1]),
                Some(ScalarBuffer::from(vec![0_i32, 0, 1, 1])),
                vec![
                    Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef,
                    Arc::new(StringArray::from(vec!["one", "two"])) as ArrayRef,
                ],
            )
            .expect("a dense union"),
        );

        vec![
            ("int64", int64),
            ("boolean[3..14]", boolean),
            ("decimal128(12, 4)", decimal),
            ("utf8", utf8),
            ("utf8_view", utf8_view),
            ("fixed_size_binary(16)", fixed),
            ("list<utf8>[1..4]", list),
            ("map<utf8, int64>", map),
            ("struct<a: int64, b: utf8>", structure),
            ("dictionary<int32, utf8>", dictionary),
            ("run_end<int32, int64>", runs),
            ("dense_union<int32, utf8>", union),
        ]
    }

    /// Every buffer of `data`'s tree - the validity, each buffer, every child
    /// - lies inside `mapping`, on a 64-byte boundary.
    fn assert_mapped(data: &ArrayData, mapping: &Mapping, path: &str) {
        let check = |what: String, ptr: *const u8, len: usize| {
            assert!(
                mapping.contains(ptr, len),
                "{what}: {len} bytes at {ptr:p} lie outside the mapping of {} bytes",
                mapping.len()
            );
            assert_eq!(
                ptr as usize % 64,
                0,
                "{what}: {ptr:p} is not 64-byte aligned"
            );
        };
        if let Some(nulls) = data.nulls() {
            let bits = nulls.inner().inner();
            check(format!("{path}.nulls"), bits.as_ptr(), bits.len());
        }
        for (index, buffer) in data.buffers().iter().enumerate() {
            check(
                format!("{path}.buffers[{index}]"),
                buffer.as_ptr(),
                buffer.len(),
            );
        }
        for (index, child) in data.child_data().iter().enumerate() {
            assert_mapped(child, mapping, &format!("{path}.children[{index}]"));
        }
    }

    /// The first buffer of `data`'s tree that holds a byte, depth first.
    fn first_bytes(data: &ArrayData) -> Option<(*const u8, usize)> {
        data.nulls()
            .map(|nulls| nulls.inner().inner())
            .into_iter()
            .chain(data.buffers())
            .find(|buffer| !buffer.is_empty())
            .map(|buffer| (buffer.as_ptr(), buffer.len()))
            .or_else(|| data.child_data().iter().find_map(first_bytes))
    }

    #[test]
    fn every_layout_reads_back_as_itself_over_the_mapping() {
        for (name, array) in layouts() {
            let (rebuilt, mapping) = spill_array(&array, None)
                .unwrap_or_else(|error| panic!("{name}: {error}"))
                .unwrap_or_else(|| panic!("{name}: a tree with bytes spills"));
            assert!(!mapping.is_empty(), "{name}");
            // The heap bytes the array held are not the mapping's.
            let (heap, len) = first_bytes(&array.to_data()).expect("a tree with bytes");
            assert!(!mapping.contains(heap, len), "{name}");

            assert_mapped(&rebuilt.to_data(), &mapping, name);

            assert_eq!(rebuilt.len(), array.len(), "{name}");
            assert_eq!(rebuilt.offset(), array.offset(), "{name}");
            assert_eq!(rebuilt.null_count(), array.null_count(), "{name}");
            assert_eq!(rebuilt.data_type(), array.data_type(), "{name}");

            // The rows, three ways: Arrow's own logical equality, the
            // rendering, and the column the crate lands each as.
            assert_eq!(rebuilt.to_data(), array.to_data(), "{name}");
            assert_eq!(format!("{rebuilt:?}"), format!("{array:?}"), "{name}");
            let landed = Serie::from_arrow_array(None, Arc::clone(&array), ArrowCastOptions::new())
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let field = landed.field().expect("a landed column has a field").clone();
            let spilled = Serie::from_arrow_array(
                Some(&field),
                Arc::clone(&rebuilt),
                ArrowCastOptions::new(),
            )
            .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(spilled, landed, "{name}");

            // A spill moves bytes, never the measure a byte bound reads - save
            // a dictionary's values, which the measure counts whole by their
            // allocation: a spill writes the bytes the buffers hold and leaves
            // a builder's spare capacity behind, so a spilled dictionary
            // measures no more than it did on the heap, and here less.
            if matches!(array.data_type(), ArrowDataType::Dictionary(..)) {
                assert!(
                    array_memory_size(&rebuilt) <= array_memory_size(&array),
                    "{name}: {} > {}",
                    array_memory_size(&rebuilt),
                    array_memory_size(&array)
                );
            } else {
                assert_eq!(
                    array_memory_size(&rebuilt),
                    array_memory_size(&array),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn a_tree_with_no_byte_to_write_spills_nothing() {
        let empty: ArrayRef = Arc::new(Int64Array::from(Vec::<i64>::new()));
        assert!(
            spill_array(&empty, None)
                .expect("an empty column spills nothing")
                .is_none()
        );

        let both_empty: ArrayRef = Arc::new(
            StructArray::try_new(
                Fields::from(vec![
                    ArrowField::new("a", ArrowDataType::Int64, true),
                    ArrowField::new("b", ArrowDataType::Float64, true),
                ]),
                vec![
                    Arc::new(Int64Array::from(Vec::<i64>::new())) as ArrayRef,
                    Arc::new(Float64Array::from(Vec::<f64>::new())) as ArrayRef,
                ],
                None,
            )
            .expect("a struct of two empty children"),
        );
        assert!(
            spill_array(&both_empty, None)
                .expect("an empty struct spills nothing")
                .is_none()
        );
    }

    #[test]
    fn a_missing_folder_is_an_io_failure_naming_it() -> yggdryl::Result<()> {
        let folder = LocalFolder::new("/nonexistent/yggdryl-spill-test-folder")?;
        let array: ArrayRef = Arc::new(Int64Array::from(vec![1, 2, 3]));
        // The mapping handle is no `Debug`, so the refusal is matched out.
        let Err(error) = spill_array(&array, Some(&folder)) else {
            panic!("no file is created under a folder that is not there");
        };
        assert!(matches!(error, yggdryl::Error::Io(_)), "{error:?}");
        assert!(
            error.to_string().contains(&folder.url().to_string()),
            "{error}"
        );
        // The array is untouched.
        assert_eq!(array.as_primitive::<Int64Type>().values(), &[1, 2, 3]);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_spill_file_is_gone_from_its_folder_while_it_is_mapped() -> yggdryl::Result<()> {
        let dir = std::env::temp_dir().join(format!("yggdryl-spill-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;

        let values = vec![Some("held"), None, Some("a run longer than twelve bytes")];
        let array: ArrayRef = Arc::new(StringArray::from(values.clone()));
        let (rebuilt, mapping) = spill_array(&array, Some(&LocalFolder::new(&dir)?))?
            .expect("a column with bytes spills");

        let listed = std::fs::read_dir(&dir)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        assert!(
            listed.is_empty(),
            "the spill file is unlinked as soon as it is open: {listed:?}"
        );

        // Unlinked, and the rows still read through the mapping.
        assert!(!mapping.is_empty());
        let read = rebuilt.as_string::<i32>().iter().collect::<Vec<_>>();
        assert_eq!(read, values);

        drop((rebuilt, mapping));
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_mapping_outlives_the_array_that_spilled() {
        let numbers = vec![Some(11_i64), None, Some(-7), Some(0), Some(i64::MIN)];
        let texts = vec![Some("first"), Some("a run longer than twelve bytes"), None];
        let (numbers_rebuilt, texts_rebuilt) = {
            let number_array: ArrayRef = Arc::new(Int64Array::from(numbers.clone()));
            let text_array: ArrayRef = Arc::new(StringViewArray::from(texts.clone()));
            let (numbers_rebuilt, numbers_mapping) = spill_array(&number_array, None)
                .expect("the numbers spill")
                .expect("the numbers have bytes");
            let (texts_rebuilt, texts_mapping) = spill_array(&text_array, None)
                .expect("the texts spill")
                .expect("the texts have bytes");
            // The arrays that spilled and the handles on their mappings go;
            // the rebuilt buffers keep the mappings alive on their own.
            drop((number_array, text_array, numbers_mapping, texts_mapping));
            (numbers_rebuilt, texts_rebuilt)
        };
        assert_eq!(
            numbers_rebuilt
                .as_primitive::<Int64Type>()
                .iter()
                .collect::<Vec<_>>(),
            numbers
        );
        assert_eq!(
            texts_rebuilt.as_string_view().iter().collect::<Vec<_>>(),
            texts
        );
        // A slice of the rebuilt column reads the same mapping.
        assert_eq!(
            numbers_rebuilt
                .slice(1, 3)
                .as_primitive::<Int64Type>()
                .iter()
                .collect::<Vec<_>>(),
            numbers[1..4].to_vec()
        );
    }

    /// The spill files this process holds open, read off `/proc/self/fd`.
    ///
    /// Only spill files count: the harness runs its other tests on threads
    /// of this same process, and any of them may hold a file open meanwhile.
    /// The count is the least of a few reads a millisecond apart, because
    /// another test's spill holds its file only while it writes, where a
    /// mapping that kept its file would hold it at every read.
    #[cfg(target_os = "linux")]
    fn open_spill_descriptors() -> usize {
        let spill = format!("yggdryl-spill-{}-", std::process::id());
        let count = || {
            std::fs::read_dir("/proc/self/fd")
                .expect("the process lists its descriptors")
                .filter_map(|entry| std::fs::read_link(entry.ok()?.path()).ok())
                .filter(|target| target.to_string_lossy().contains(&spill))
                .count()
        };
        (0..20)
            .map(|_| {
                std::thread::sleep(std::time::Duration::from_millis(1));
                count()
            })
            .min()
            .unwrap_or_default()
    }

    /// A mapping holds the pages, never the file: the descriptor the spill
    /// wrote through is closed once the file is mapped, so the live spilled
    /// units of a process are bounded by memory and disk, not by its
    /// descriptor limit.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_live_mapping_holds_no_descriptor() {
        let array: ArrayRef = Arc::new(Int64Array::from((0..64_i64).collect::<Vec<_>>()));
        let before = open_spill_descriptors();
        let held = (0..64)
            .map(|_| {
                spill_array(&array, None)
                    .expect("the numbers spill")
                    .expect("the numbers have bytes")
            })
            .collect::<Vec<_>>();
        let during = open_spill_descriptors();
        assert_eq!(
            during,
            before,
            "sixty-four live mappings held {} spill files open",
            during.saturating_sub(before)
        );
        for (rebuilt, mapping) in &held {
            assert_eq!(rebuilt.len(), 64);
            assert!(mapping.len() >= 64 * 8, "the sixty-four values are mapped");
        }
        drop(held);
        assert_eq!(open_spill_descriptors(), before);
    }

    fn some(text: &str) -> Option<OsString> {
        Some(OsString::from(text))
    }

    #[test]
    fn an_unset_or_empty_environment_is_the_default() {
        assert_eq!(parse_env(None, None).expect("unset"), SpillOptions::new());
        assert_eq!(
            parse_env(some(""), None).expect("empty"),
            SpillOptions::new()
        );
        assert_eq!(
            parse_env(None, some("")).expect("empty folder"),
            SpillOptions::new()
        );
        assert!(
            parse_env(None, some(""))
                .expect("empty folder")
                .folder()
                .is_none()
        );
    }

    #[test]
    fn the_bound_is_a_byte_count_or_never_in_any_case() {
        let options = parse_env(some("1048576"), None).expect("a byte count");
        assert_eq!(options.byte_size(), 1024 * 1024);
        assert!(options.folder().is_none());
        assert_eq!(
            parse_env(some("0"), None).expect("zero").byte_size(),
            0,
            "zero spills everything"
        );
        for never in ["NEVER", "never", " never ", "Never"] {
            assert!(
                parse_env(some(never), None)
                    .unwrap_or_else(|error| panic!("{never:?}: {error}"))
                    .is_never(),
                "{never:?}"
            );
        }
        assert_eq!(
            parse_env(some(" 4096 "), None)
                .expect("a padded count")
                .byte_size(),
            4096
        );
        // The count is the one integer grammar's: a stated sign is part of it.
        assert_eq!(
            parse_env(some("+4096"), None)
                .expect("a signed count")
                .byte_size(),
            4096
        );
        assert_eq!(
            parse_env(None, None).expect("unset").byte_size(),
            DEFAULT_SPILL_BYTE_SIZE
        );
    }

    #[test]
    fn a_bound_that_is_no_byte_count_is_refused_naming_the_variable_and_the_text() {
        for text in [
            "12 MiB",
            "-1",
            "abc",
            "1.5",
            "18446744073709551616",
            "1e3",
            "1_000",
        ] {
            let error = parse_env(some(text), None)
                .expect_err("a bound that is neither a count nor `never` is refused");
            let message = error.to_string();
            assert!(message.contains("YGGDRYL_SPILL_BYTE_SIZE"), "{message}");
            assert!(message.contains(text), "{message}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_bound_that_is_not_utf8_is_refused_naming_the_variable() {
        use std::os::unix::ffi::OsStringExt as _;
        let error = parse_env(Some(OsString::from_vec(vec![0xff])), None)
            .expect_err("a bound that is not UTF-8 is refused");
        assert!(
            error.to_string().contains("YGGDRYL_SPILL_BYTE_SIZE"),
            "{error}"
        );
    }

    #[test]
    fn the_folder_is_the_path_the_variable_names() -> yggdryl::Result<()> {
        let dir = std::env::temp_dir();
        let options = parse_env(None, Some(dir.clone().into_os_string()))?;
        assert_eq!(
            options.folder().map(|folder| folder.url().to_string()),
            Some(LocalFolder::new(&dir)?.url().to_string())
        );
        assert_eq!(options.byte_size(), DEFAULT_SPILL_BYTE_SIZE);
        assert_eq!(
            options,
            SpillOptions::new().with_folder(LocalFolder::new(&dir)?)
        );

        // Both at once.
        let both = parse_env(some("never"), Some(dir.clone().into_os_string()))?;
        assert!(both.is_never());
        assert!(both.folder().is_some());
        Ok(())
    }
}
