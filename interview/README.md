# QRT HackerRank — interview prep

Runnable Python solutions and pytest tests for every exercise in the QRT prep
email (Python role: 1 MCQ + 4 algorithmic tasks, drawn from the pool below).

Each solution lives in its own module at the top level; its tests live in
`tests/test_<name>.py`.

## Run

```bash
cd interview
pip install -r requirements.txt   # just pytest
pytest -q
```

`conftest.py` puts this folder on `sys.path`, so the tests import the solution
modules by name and you can also drop into a REPL:

```python
>>> from q1_star_schema_max_sum import max_star_sum
>>> max_star_sum([1, 2, 3, 4, 10, -10, -20], [[0,1],[1,2],[1,3],[3,4],[3,5],[3,6]], 2)
16
```

## Map: email question → file → technique

| Email item | Module | Technique |
| --- | --- | --- |
| MCQ – list comprehension | `mcq_list_comprehension.py` | map / filter / flatten / zip / dict-comp |
| Q1 – Star Schema max sum | `q1_star_schema_max_sum.py` | adjacency dict + top-k positive neighbours (LeetCode 2497) |
| Q2 – Maximal knot path | `q2_maximal_knot_path.py` | tree DP, two best branches (LeetCode 124, iterative) |
| Q2 – Cut the Tree | `q2b_cut_the_tree.py` | subtree sums, minimise `|total − 2·sub|` |
| Q3 – Segment intersections | `q3_segment_intersections.py` | sort + merge intervals, sweep-line max overlap |
| Q4 – Interval matching | `q4_interval_matching.py` | merge + binary search count |
| Q5 – Connected components | `q5_connected_components.py` | union-find, min/max per component |
| Q6 – Strongly connected groups | `q6_strongly_connected_groups.py` | Kosaraju two-pass SCC |
| Q7 – Bit stream manipulation | `q7_bit_stream_manipulation.py` | fixed-width binary, zero counting, flip combinatorics |
| Q8 – Values between min/max | `q8_array_between_min_max.py` | array filter (strict / inclusive) |
| Q9 – Formula to algorithm (LEM) | `q9_lem_formula.py` | closed form ⇄ iterative equivalence |
| Extra – Friend Circle Queries | `friend_circle_queries.py` | union-find, running max size |
| Extra – Interval Selection | `interval_selection.py` | greedy + lazy segment tree (≤ k overlap) |
| Extra – Journey Scheduling | `journey_scheduling.py` | tree diameter + eccentricity |

## Notes on ambiguous questions

- **Q7 (flips):** the prep wording is ambiguous, so both readings are provided —
  flip exactly one zero (`z` new numbers) and flip any non-empty subset of zeros
  (`2**z − 1`).
- **Q9 (LEM):** the exact formula was not given. The module demonstrates the
  *technique* (turn a closed-form formula into an iterative algorithm and prove
  they agree) with the sum-of-squares example and a generalised power-sum
  template — swap in the real formula on the day.
- Graph/tree nodes are **0-indexed**; HackerRank inputs are often 1-indexed, so
  subtract 1 when reading them.
- Tree traversals are **iterative** to avoid Python's recursion limit on deep or
  unbalanced trees (an edge case the prep flags).

## Exam-day reminders (from the email)

- Once started you must finish; no other tabs may be open.
- Do any exercise branded **Mandatory** first.
- If stuck, move on and come back.
