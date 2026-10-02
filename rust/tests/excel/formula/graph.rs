//! rust/src/excel/formula/graph.rs: sparse predecessor membership,
//! inclusive geometry, independent borrowed queries and exact removal.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::Error;
    use yggdryl::excel::{CellRange, CellRef, MAX_COLUMNS, MAX_ROWS, SheetKey, Workbook};
    use yggdryl::internals::excel_formula_graph::Index;

    fn sheets() -> [SheetKey; 2] {
        let mut workbook = Workbook::new();
        workbook.add_sheet("First").unwrap();
        workbook.add_sheet("Second").unwrap();
        ["First", "Second"].map(|name| workbook.sheet_key(name).unwrap())
    }

    fn at(text: &str) -> CellRef {
        text.parse().unwrap()
    }

    fn range(text: &str) -> CellRange {
        text.parse().unwrap()
    }

    fn matched(index: &Index, sheet: SheetKey, at: CellRef) -> Vec<usize> {
        let mut nodes: Vec<_> = index.dependents(sheet, at).collect();
        nodes.sort_unstable();
        nodes
    }

    #[test]
    fn dependency_index_covers_absent_cells_inclusive_edges_and_whole_axes() {
        let [first, second] = sheets();
        let mut index = Index::default();
        index.insert(first, range("B2:D4"), 10).unwrap();
        index.insert(first, range("C3"), 11).unwrap();
        index.insert(first, range("3:3"), 12).unwrap();
        index.insert(first, range("D:D"), 13).unwrap();
        index.insert(second, range("C3"), 20).unwrap();
        for (cell, expected) in [
            ("A1", vec![]),
            ("B2", vec![10]),
            ("D4", vec![10, 13]),
            ("E4", vec![]),
            ("C3", vec![10, 11, 12]),
            ("D3", vec![10, 12, 13]),
            ("XFD3", vec![12]),
            ("D1048576", vec![13]),
        ] {
            assert_eq!(matched(&index, first, at(cell)), expected, "{cell}");
        }
        assert_eq!(matched(&index, second, at("C3")), [20]);
        assert!(index.dependents(second, at("D3")).next().is_none());
        // No actual worksheet cells were needed to register blank precedents.
        assert_eq!(index.counts().0, 5);
    }

    #[test]
    fn dependency_index_matches_every_rectangle_without_duplicate_cover_hits() {
        let [sheet, _] = sheets();
        let mut index = Index::default();
        let mut rectangles = Vec::new();
        for first_row in 0..6 {
            for last_row in first_row..6 {
                for first_column in 0..6 {
                    for last_column in first_column..6 {
                        let rectangle = CellRange::new(
                            CellRef::new(first_row, first_column),
                            CellRef::new(last_row, last_column),
                        );
                        index.insert(sheet, rectangle, rectangles.len()).unwrap();
                        rectangles.push(rectangle);
                    }
                }
            }
        }
        for row in 0..8 {
            for column in 0..8 {
                let cell = CellRef::new(row, column);
                let expected: Vec<_> = rectangles
                    .iter()
                    .enumerate()
                    .filter_map(|(id, range)| range.contains(cell).then_some(id))
                    .collect();
                assert_eq!(matched(&index, sheet, cell), expected, "{cell}");
            }
        }
    }

    #[test]
    fn dependency_index_keeps_registration_multiplicity_and_removes_only_its_handle() {
        let [sheet, _] = sheets();
        let mut index = Index::default();
        let point = index.insert(sheet, range("C3"), 7).unwrap();
        let area = index.insert(sheet, range("B2:D4"), 7).unwrap();
        let duplicate = index.insert(sheet, range("B2:D4"), 7).unwrap();
        let other = index.insert(sheet, range("C3"), 8).unwrap();
        assert_eq!(matched(&index, sheet, at("C3")), [7, 7, 7, 8]);
        index.remove(area).unwrap();
        assert_eq!(matched(&index, sheet, at("C3")), [7, 7, 8]);
        index.remove(point).unwrap();
        assert_eq!(matched(&index, sheet, at("C3")), [7, 8]);
        index.remove(duplicate).unwrap();
        index.remove(other).unwrap();
        assert_eq!(index.counts(), (0, 4, 0, 0));
        for _ in 0..128 {
            let next = index.insert(sheet, range("XFD1048576"), 99).unwrap();
            assert_eq!(matched(&index, sheet, at("XFD1048576")), [99]);
            index.remove(next).unwrap();
            assert_eq!(index.counts(), (0, 4, 0, 0));
        }
    }

    #[test]
    fn dependency_index_rejects_foreign_handles_before_touching_same_numbered_slots() {
        let [sheet, _] = sheets();
        let mut first = Index::default();
        let mut second = Index::default();
        let foreign = first.insert(sheet, range("B2:D4"), 10).unwrap();
        let own = second.insert(sheet, range("B2:D4"), 20).unwrap();
        let before = (first.counts(), second.counts());
        match second.remove(foreign).unwrap_err() {
            Error::Conflict {
                expected,
                actual,
                path,
            } => {
                assert_eq!(expected, "a registration from this dependency index");
                assert_eq!(actual, "a registration from another dependency index");
                assert_eq!(path, "$.formula.dependencies[0]");
            }
            other => panic!("expected a located owner conflict, got {other:?}"),
        }
        assert_eq!((first.counts(), second.counts()), before);
        assert_eq!(matched(&first, sheet, at("C3")), [10]);
        assert_eq!(matched(&second, sheet, at("C3")), [20]);
        second.remove(own).unwrap();
        assert_eq!(second.counts(), (0, 1, 0, 0));
        assert_eq!(matched(&first, sheet, at("C3")), [10]);

        // A foreign slot beyond the receiving slab must refuse without indexing it.
        let foreign = first.insert(sheet, range("A1"), 30).unwrap();
        let mut empty = Index::default();
        assert!(matches!(empty.remove(foreign), Err(Error::Conflict { .. })));
        assert_eq!(empty.counts(), (0, 0, 0, 0));
        assert_eq!(first.counts().0, 2);
        assert_eq!(matched(&first, sheet, at("A1")), [30]);
    }

    #[test]
    fn dependency_index_point_cursor_visits_37_buckets_and_filters_candidates() {
        let [sheet, other] = sheets();
        let mut index = Index::default();
        assert_eq!(index.query_counts(sheet, at("C3")), (37, 0, 0));
        for (id, area) in [
            CellRange::all(),
            range("3:3"),
            range("C:C"),
            range("C3"),
            range("B2:D4"),
            range("F2:H4"),
        ]
        .into_iter()
        .enumerate()
        {
            index.insert(sheet, area, id).unwrap();
        }
        // The F:H rectangle shares row buckets but fails the column predicate;
        // this pins the documented candidate-sensitive, not output-only, cost.
        assert_eq!(index.query_counts(sheet, at("C3")), (37, 6, 5));
        assert_eq!(index.query_counts(sheet, at("B3")), (37, 4, 3));
        assert_eq!(index.query_counts(sheet, at("XFD1048576")), (37, 1, 1));
        assert_eq!(index.query_counts(other, at("C3")), (37, 0, 0));
        assert_eq!(
            index.query_counts(sheet, CellRef::new(u32::MAX, u32::MAX)),
            (0, 0, 0)
        );
    }

    #[test]
    fn dependency_index_memberships_depend_on_axis_depth_not_rectangle_area() {
        let [sheet, _] = sheets();
        let mut all = Index::default();
        all.insert(sheet, CellRange::all(), 1).unwrap();
        assert_eq!(all.counts(), (1, 1, 1, 1));
        for at in [
            CellRef::new(0, 0),
            CellRef::new(MAX_ROWS - 1, MAX_COLUMNS - 1),
        ] {
            assert_eq!(matched(&all, sheet, at), [1]);
        }

        let mut narrow = Index::default();
        let mut wide = Index::default();
        for id in 0..64 {
            let start = 1 + id as u32 * 127;
            let end = start + 94;
            narrow
                .insert(
                    sheet,
                    CellRange::new(CellRef::new(start, 2), CellRef::new(end, 3)),
                    id,
                )
                .unwrap();
            wide.insert(
                sheet,
                CellRange::new(CellRef::new(start, 2), CellRef::new(end, MAX_COLUMNS - 1)),
                id,
            )
            .unwrap();
        }
        // Widening by four orders of magnitude neither copies registration
        // records nor creates column buckets for ordinary rectangles.
        assert_eq!(narrow.counts(), wide.counts());
        assert_eq!(wide.counts().0, 64);
        assert!(wide.counts().3 <= 64 * 2 * MAX_ROWS.ilog2() as usize);

        let mut columns = Index::default();
        columns
            .insert(
                sheet,
                CellRange::new(
                    CellRef::new(0, 1),
                    CellRef::new(MAX_ROWS - 1, MAX_COLUMNS - 2),
                ),
                2,
            )
            .unwrap();
        assert!(columns.counts().3 <= 2 * MAX_COLUMNS.ilog2() as usize);
        assert_eq!(matched(&columns, sheet, CellRef::new(MAX_ROWS - 1, 1)), [2]);
        assert!(
            columns
                .dependents(sheet, CellRef::new(MAX_ROWS - 1, 0))
                .next()
                .is_none()
        );
    }

    #[test]
    fn dependency_index_queries_are_independent_borrowed_cursors() {
        let [sheet, _] = sheets();
        let mut index = Index::default();
        for (id, text) in ["A1", "A1:C3", "1:3", "A:C"].into_iter().enumerate() {
            index.insert(sheet, range(text), id).unwrap();
        }
        let mut first = index.dependents(sheet, at("A1"));
        let mut second = index.dependents(sheet, at("B2"));
        let mut left = vec![first.next().unwrap()];
        let mut right = vec![second.next().unwrap()];
        let mut restarted_tail: Vec<_> = first.clone().collect();
        restarted_tail.sort_unstable();
        assert_eq!(restarted_tail, [1, 2, 3]);
        left.extend(first);
        right.extend(second);
        left.sort_unstable();
        right.sort_unstable();
        assert_eq!(left, [0, 1, 2, 3]);
        assert_eq!(right, [1, 2, 3]);
        assert_eq!(matched(&index, sheet, at("A1")), left);
    }

    #[test]
    fn dependency_index_refuses_out_of_grid_before_publishing_any_membership() {
        let [sheet, _] = sheets();
        let mut index = Index::default();
        index.insert(sheet, range("A1:C3"), 1).unwrap();
        let before = index.counts();
        for end in [
            CellRef::new(MAX_ROWS, 0),
            CellRef::new(0, MAX_COLUMNS),
            CellRef::new(u32::MAX, u32::MAX),
        ] {
            let result = index.insert(sheet, CellRange::new(CellRef::new(0, 0), end), 2);
            match result {
                Err(Error::InvalidRecord { path, reason }) => {
                    assert_eq!(path, format!("$.sheets[{}].dependencies", sheet.as_u32()));
                    assert!(reason.contains("expected a dependency within"));
                    assert!(reason.contains("got rows"));
                }
                _ => panic!("expected a located dependency refusal"),
            }
            assert_eq!(index.counts(), before);
            assert_eq!(matched(&index, sheet, at("A1")), [1]);
        }
        assert!(
            index
                .dependents(sheet, CellRef::new(u32::MAX, u32::MAX))
                .next()
                .is_none()
        );
    }
    fn settled(graph: &mut yggdryl::internals::excel_formula_graph::Scheduler) {
        let pass = graph.prepare(true).unwrap();
        let outcomes = vec![true; pass.ordered().len()];
        graph.acknowledge(pass, &outcomes).unwrap();
    }

    #[test]
    fn dependency_schedule_orders_a_diamond_once_and_distinguishes_full_passes() {
        use yggdryl::internals::excel_formula_graph::Scheduler;
        let [sheet, other] = sheets();
        let mut graph = Scheduler::default();
        graph.set(sheet, at("A1"), &[], false).unwrap();
        graph
            .set(sheet, at("B1"), &[(sheet, range("A1"))], false)
            .unwrap();
        graph
            .set(
                sheet,
                at("C1"),
                &[(sheet, range("A1")), (sheet, range("A1"))],
                false,
            )
            .unwrap();
        graph
            .set(
                sheet,
                at("D1"),
                &[(sheet, range("B1:C1")), (sheet, range("C1"))],
                false,
            )
            .unwrap();
        graph.set(other, at("A1"), &[], false).unwrap();
        let initial = graph.prepare(false).unwrap();
        assert_eq!(initial.ordered().len(), 5);
        let position = |cell| {
            initial
                .ordered()
                .iter()
                .position(|&value| value == (sheet, at(cell)))
                .unwrap()
        };
        assert!(position("A1") < position("B1"));
        assert!(position("A1") < position("C1"));
        assert!(position("B1") < position("D1"));
        assert!(position("C1") < position("D1"));
        assert!(initial.circular().is_empty());
        assert!(initial.blocked().is_empty());
        graph.acknowledge(initial, &[true; 5]).unwrap();
        assert!(graph.prepare(false).unwrap().ordered().is_empty());
        assert_eq!(graph.prepare(true).unwrap().ordered().len(), 5);
        graph.changed(sheet, at("A1")).unwrap();
        let incremental = graph.prepare(false).unwrap();
        assert_eq!(
            incremental.ordered(),
            &[
                (sheet, at("A1")),
                (sheet, at("B1")),
                (sheet, at("C1")),
                (sheet, at("D1")),
            ]
        );
        assert_eq!(graph.status(other, at("A1")), Some("computed"));
    }

    #[test]
    fn dependency_schedule_patches_edges_and_seeds_removed_addresses_before_slot_reuse() {
        use yggdryl::internals::excel_formula_graph::Scheduler;
        let [sheet, _] = sheets();
        let mut graph = Scheduler::default();
        graph.set(sheet, at("A1"), &[], false).unwrap();
        graph
            .set(sheet, at("B1"), &[(sheet, range("A1"))], false)
            .unwrap();
        graph
            .set(sheet, at("C1"), &[(sheet, range("B1"))], false)
            .unwrap();
        settled(&mut graph);
        assert!(graph.remove(sheet, at("A1")));
        assert_eq!(graph.status(sheet, at("A1")), None);
        assert_eq!(
            graph.prepare(false).unwrap().ordered(),
            &[(sheet, at("B1")), (sheet, at("C1"))]
        );
        // Reusing A1's internal node slot must not replace its address in the
        // already-seeded consumers or add a false edge to an unrelated formula.
        graph.set(sheet, at("Z9"), &[], false).unwrap();
        let pass = graph.prepare(false).unwrap();
        assert_eq!(pass.ordered().len(), 3);
        assert!(!pass.ordered().contains(&(sheet, at("A1"))));
        graph.acknowledge(pass, &[true; 3]).unwrap();
        graph
            .set(sheet, at("B1"), &[(sheet, range("H1"))], false)
            .unwrap();
        settled(&mut graph);
        graph.changed(sheet, at("A1")).unwrap();
        assert!(graph.prepare(false).unwrap().ordered().is_empty());
        graph.changed(sheet, at("H1")).unwrap();
        assert_eq!(
            graph.prepare(false).unwrap().ordered(),
            &[(sheet, at("B1")), (sheet, at("C1"))]
        );
    }

    #[test]
    fn dependency_schedule_seeds_blank_ranges_whole_axes_and_volatile_closures() {
        use yggdryl::internals::excel_formula_graph::Scheduler;
        let [sheet, other] = sheets();
        let mut graph = Scheduler::default();
        graph.set(sheet, at("A1"), &[], true).unwrap();
        graph
            .set(sheet, at("B1"), &[(sheet, range("A1"))], false)
            .unwrap();
        graph
            .set(sheet, at("C1"), &[(other, range("D:D"))], false)
            .unwrap();
        graph
            .set(sheet, at("E1"), &[(other, range("7:7"))], false)
            .unwrap();
        settled(&mut graph);
        assert_eq!(
            graph.prepare(false).unwrap().ordered(),
            &[(sheet, at("A1")), (sheet, at("B1"))]
        );
        graph.changed(other, at("D1048576")).unwrap();
        let pass = graph.prepare(false).unwrap();
        assert!(pass.ordered().contains(&(sheet, at("C1"))));
        assert!(!pass.ordered().contains(&(sheet, at("E1"))));
        let count = pass.ordered().len();
        graph.acknowledge(pass, &vec![true; count]).unwrap();
        graph.changed(other, at("XFD7")).unwrap();
        assert!(
            graph
                .prepare(false)
                .unwrap()
                .ordered()
                .contains(&(sheet, at("E1")))
        );
        graph.set(sheet, at("A1"), &[], false).unwrap();
        settled(&mut graph);
        assert!(graph.prepare(false).unwrap().ordered().is_empty());
    }

    #[test]
    fn dependency_schedule_classifies_only_true_cycles_in_every_three_node_graph() {
        use yggdryl::internals::excel_formula_graph::Scheduler;
        let [sheet, _] = sheets();
        for mask in 0..(1_u16 << 9) {
            let mut graph = Scheduler::default();
            let mut reaches = [[false; 3]; 3];
            for (from, row) in reaches.iter_mut().enumerate() {
                for (to, edge) in row.iter_mut().enumerate() {
                    *edge = mask & (1 << (from * 3 + to)) != 0;
                }
            }
            for to in 0..3 {
                let precedents: Vec<_> = (0..3)
                    .filter(|&from| reaches[from][to])
                    .map(|from| {
                        let cell = CellRef::new(0, from as u32);
                        (sheet, CellRange::new(cell, cell))
                    })
                    .collect();
                graph
                    .set(sheet, CellRef::new(0, to as u32), &precedents, false)
                    .unwrap();
            }
            // Independent transitive closure classifies SCC membership and
            // downstream residue, without mirroring Tarjan or Kahn.
            for through in 0..3 {
                for from in 0..3 {
                    for to in 0..3 {
                        let path = reaches[from][through] && reaches[through][to];
                        reaches[from][to] |= path;
                    }
                }
            }
            let mut cycles = Vec::new();
            let mut blocked = Vec::new();
            let mut available = Vec::new();
            for to in 0..3 {
                let address = (sheet, CellRef::new(0, to as u32));
                if reaches[to][to] {
                    cycles.push(address);
                } else if (0..3).any(|from| reaches[from][from] && reaches[from][to]) {
                    blocked.push(address);
                } else {
                    available.push(address);
                }
            }
            let pass = graph.prepare(true).unwrap();
            assert_eq!(pass.circular(), cycles, "mask {mask}");
            assert_eq!(pass.blocked(), blocked, "mask {mask}");
            let mut ordered = pass.ordered().to_vec();
            ordered.sort_unstable();
            assert_eq!(ordered, available, "mask {mask}");
            let count = pass.ordered().len();
            graph.acknowledge(pass, &vec![true; count]).unwrap();
            for address in cycles {
                assert_eq!(graph.status(address.0, address.1), Some("circular"));
            }
            for address in blocked {
                assert_eq!(graph.status(address.0, address.1), Some("held"));
            }
            for address in available {
                assert_eq!(graph.status(address.0, address.1), Some("computed"));
            }
        }
    }

    #[test]
    fn dependency_schedule_distinguishes_clean_held_inputs_from_dirty_overlay_receipts() {
        use yggdryl::internals::excel_formula_graph::Scheduler;
        let [sheet, _] = sheets();
        let mut graph = Scheduler::default();
        graph.set(sheet, at("A1"), &[], false).unwrap();
        graph
            .set(sheet, at("B1"), &[(sheet, range("A1"))], false)
            .unwrap();
        let pass = graph.prepare(true).unwrap();
        assert_eq!(pass.ordered(), &[(sheet, at("A1")), (sheet, at("B1"))]);
        // The evaluator reports A1 held; B1 must see that new overlay status,
        // not A1's old cache. The graph does not perform a second evaluation.
        graph.acknowledge(pass, &[false, false]).unwrap();
        assert_eq!(graph.status(sheet, at("A1")), Some("held"));
        assert_eq!(graph.status(sheet, at("B1")), Some("held"));
        graph
            .set(sheet, at("C1"), &[(sheet, range("A1"))], false)
            .unwrap();
        let pass = graph.prepare(false).unwrap();
        assert_eq!(pass.ordered(), &[(sheet, at("C1"))]);
        // C1's reference read consults clean A1's retained status, so its
        // receipt is Held even though A1 is outside this induced dirty graph.
        graph.acknowledge(pass, &[false]).unwrap();
        assert_eq!(graph.status(sheet, at("C1")), Some("held"));
        graph.changed(sheet, at("A1")).unwrap();
        let pass = graph.prepare(false).unwrap();
        assert_eq!(pass.ordered().len(), 3);
        graph.acknowledge(pass, &[true; 3]).unwrap();
        for cell in ["A1", "B1", "C1"] {
            assert_eq!(graph.status(sheet, at(cell)), Some("computed"));
        }
    }

    #[test]
    fn dependency_schedule_refuses_foreign_stale_and_incomplete_acknowledgments_atomically() {
        use yggdryl::internals::excel_formula_graph::Scheduler;
        let [sheet, _] = sheets();
        let mut first = Scheduler::default();
        let mut second = Scheduler::default();
        first.set(sheet, at("A1"), &[], false).unwrap();
        second
            .set(sheet, at("A1"), &[(sheet, range("D1"))], false)
            .unwrap();
        let foreign = first.prepare(true).unwrap();
        assert!(matches!(
            second.acknowledge(foreign, &[true]),
            Err(Error::Conflict { .. })
        ));
        assert_eq!(second.status(sheet, at("A1")), Some("held"));
        assert_eq!(
            second.prepare(false).unwrap().ordered(),
            &[(sheet, at("A1"))]
        );
        let stale = second.prepare(false).unwrap();
        second.changed(sheet, at("A1")).unwrap();
        assert!(matches!(
            second.acknowledge(stale, &[true]),
            Err(Error::Conflict { .. })
        ));
        let incomplete = second.prepare(false).unwrap();
        match second.acknowledge(incomplete, &[]).unwrap_err() {
            Error::InvalidRecord { path, reason } => {
                assert_eq!(path, "$.formula.calculation.outcomes");
                assert_eq!(reason, "expected 1 ordered outcomes, got 0");
            }
            other => panic!("expected a located outcome refusal, got {other:?}"),
        }
        assert_eq!(second.status(sheet, at("A1")), Some("held"));
        assert_eq!(
            second.prepare(false).unwrap().ordered(),
            &[(sheet, at("A1"))]
        );
        let bad = CellRange::new(CellRef::new(0, 0), CellRef::new(MAX_ROWS, 0));
        assert!(matches!(
            second.set(
                sheet,
                at("A1"),
                &[(sheet, range("B1")), (sheet, bad)],
                false
            ),
            Err(Error::InvalidRecord { .. })
        ));
        let pass = second.prepare(false).unwrap();
        second.acknowledge(pass, &[true]).unwrap();
        second.changed(sheet, at("B1")).unwrap();
        assert!(
            second.prepare(false).unwrap().ordered().is_empty(),
            "invalid replacement published its first edge"
        );
        assert_eq!(second.status(sheet, at("A1")), Some("computed"));
        second.changed(sheet, at("D1")).unwrap();
        assert_eq!(
            second.prepare(false).unwrap().ordered(),
            &[(sheet, at("A1"))],
            "invalid replacement removed the original edge"
        );
    }

    #[test]
    fn dependency_schedule_uses_explicit_stacks_for_deep_cycles_and_long_blocked_tails() {
        use yggdryl::internals::excel_formula_graph::Scheduler;
        let [sheet, _] = sheets();
        let mut graph = Scheduler::default();
        const CYCLE: u32 = 2_048;
        const TAIL: u32 = 2_048;
        for row in 0..CYCLE + TAIL {
            let previous = if row == 0 { CYCLE - 1 } else { row - 1 };
            let cell = CellRef::new(previous, 0);
            graph
                .set(
                    sheet,
                    CellRef::new(row, 0),
                    &[(sheet, CellRange::new(cell, cell))],
                    false,
                )
                .unwrap();
        }
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || {
                let pass = graph.prepare(true).unwrap();
                assert!(pass.ordered().is_empty());
                assert_eq!(pass.circular().len(), CYCLE as usize);
                assert_eq!(pass.blocked().len(), TAIL as usize);
                assert_eq!(pass.circular().first(), Some(&(sheet, CellRef::new(0, 0))));
                assert_eq!(
                    pass.blocked().last(),
                    Some(&(sheet, CellRef::new(CYCLE + TAIL - 1, 0)))
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn dependency_schedule_reused_workspace_discards_prior_order_and_cycle_residue() {
        use yggdryl::excel::{CellRange, CellRef, Workbook};
        use yggdryl::internals::excel_formula_graph::{Scheduler, Workspace};
        let mut book = Workbook::new();
        book.add_sheet("Data").unwrap();
        let sheet = book.sheet_key("Data").unwrap();
        let a = CellRef::new(0, 0);
        let b = CellRef::new(0, 1);
        let c = CellRef::new(0, 2);
        let point = |at| (sheet, CellRange::new(at, at));
        let mut graph = Scheduler::default();
        graph.set(sheet, a, &[], false).unwrap();
        graph.set(sheet, b, &[point(a)], false).unwrap();
        graph.set(sheet, c, &[point(b)], false).unwrap();
        let mut workspace = Workspace::default();
        graph.prepare_reusing(true, &mut workspace).unwrap();
        assert_eq!(workspace.ordered(), &[(sheet, a), (sheet, b), (sheet, c)]);
        graph
            .acknowledge_reusing(&mut workspace, &[true; 3])
            .unwrap();
        graph.remove(sheet, a);
        graph.prepare_reusing(false, &mut workspace).unwrap();
        assert_eq!(workspace.ordered(), &[(sheet, b), (sheet, c)]);
        assert!(workspace.circular().is_empty());
        graph
            .acknowledge_reusing(&mut workspace, &[true; 2])
            .unwrap();
        graph.set(sheet, b, &[point(c)], false).unwrap();
        graph.prepare_reusing(false, &mut workspace).unwrap();
        assert!(workspace.ordered().is_empty());
        assert_eq!(workspace.circular(), &[(sheet, b), (sheet, c)]);
        graph.acknowledge_reusing(&mut workspace, &[]).unwrap();
        graph.set(sheet, b, &[], false).unwrap();
        graph.prepare_reusing(false, &mut workspace).unwrap();
        assert_eq!(workspace.ordered(), &[(sheet, b), (sheet, c)]);
        assert!(workspace.circular().is_empty());
        assert!(workspace.blocked().is_empty());
        graph
            .acknowledge_reusing(&mut workspace, &[true; 2])
            .unwrap();
        graph.prepare_reusing(false, &mut workspace).unwrap();
        assert!(workspace.ordered().is_empty());
        assert!(workspace.circular().is_empty());
        assert!(workspace.blocked().is_empty());
    }

    #[test]
    fn dependency_schedule_status_counts_include_clean_held_and_circular_nodes() {
        use yggdryl::internals::excel_formula_graph::Scheduler;
        let [sheet, _] = sheets();
        let a = CellRef::new(0, 0);
        let b = CellRef::new(0, 1);
        let c = CellRef::new(0, 2);
        let d = CellRef::new(0, 3);
        let e = CellRef::new(0, 4);
        let point = |at| (sheet, CellRange::new(at, at));
        let mut graph = Scheduler::default();
        for (at, precedents) in [
            (a, vec![point(a)]),
            (b, vec![point(a)]),
            (c, vec![]),
            (d, vec![]),
            (e, vec![point(d)]),
        ] {
            graph.set(sheet, at, &precedents, false).unwrap();
        }
        assert_eq!(graph.status_counts(), (5, 0));
        let full = graph.prepare(true).unwrap();
        let outcomes: Vec<_> = full.ordered().iter().map(|&(_, at)| at == c).collect();
        graph.acknowledge(full, &outcomes).unwrap();
        assert_eq!(graph.status_counts(), (4, 1));
        assert_eq!(graph.circular_cells().collect::<Vec<_>>(), vec![(sheet, a)]);
        let empty = graph.prepare(false).unwrap();
        assert!(empty.ordered().is_empty());
        graph.acknowledge(empty, &[]).unwrap();
        assert_eq!(
            graph.status_counts(),
            (4, 1),
            "unchanged passes retain global status"
        );
        graph.remove(sheet, a);
        let repaired = graph.prepare(false).unwrap();
        assert_eq!(repaired.ordered(), &[(sheet, b)]);
        graph.acknowledge(repaired, &[true]).unwrap();
        assert_eq!(graph.status_counts(), (2, 0));
        assert!(graph.circular_cells().next().is_none());
        graph.set(sheet, d, &[], false).unwrap();
        let repaired = graph.prepare(false).unwrap();
        assert_eq!(repaired.ordered(), &[(sheet, d), (sheet, e)]);
        graph.acknowledge(repaired, &[true; 2]).unwrap();
        assert_eq!(graph.status_counts(), (0, 0));
        graph.set(sheet, c, &[], false).unwrap();
        assert_eq!(
            graph.status_counts(),
            (1, 0),
            "replacing a computed formula invalidates its old status"
        );
        graph.remove(sheet, c);
        assert_eq!(graph.status_counts(), (0, 0));
    }
}
