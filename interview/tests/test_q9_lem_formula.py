from q9_lem_formula import (
    sum_first_n_formula,
    sum_of_squares_algorithm,
    sum_of_squares_formula,
    weighted_power_sum,
)


def test_sum_of_squares_formula_matches_algorithm():
    for n in range(0, 50):
        assert sum_of_squares_formula(n) == sum_of_squares_algorithm(n)


def test_known_values():
    assert sum_of_squares_formula(3) == 14  # 1 + 4 + 9
    assert sum_first_n_formula(10) == 55


def test_sum_first_n_matches_builtin():
    for n in range(0, 50):
        assert sum_first_n_formula(n) == sum(range(1, n + 1))


def test_weighted_power_sum_generalises():
    assert weighted_power_sum([1, 1, 1], 2) == 14  # 1**2 + 2**2 + 3**2
    assert weighted_power_sum([1, 1, 1, 1], 1) == 10  # 1 + 2 + 3 + 4
