import pytest

from q7_bit_stream_manipulation import (
    count_zeros,
    expand_all,
    new_numbers_flip_any_zeros,
    new_numbers_flip_one_zero,
    to_bitstream,
)


def test_to_bitstream_pads():
    assert to_bitstream(5, 8) == "00000101"
    assert to_bitstream(0, 4) == "0000"
    assert to_bitstream(15, 4) == "1111"


def test_to_bitstream_errors():
    with pytest.raises(ValueError):
        to_bitstream(-1, 4)
    with pytest.raises(ValueError):
        to_bitstream(16, 4)  # needs 5 bits


def test_expand_all():
    assert expand_all([1, 2, 3], 4) == ["0001", "0010", "0011"]


def test_count_zeros():
    assert count_zeros("00000101") == 6
    assert count_zeros("1111") == 0


def test_flip_counts():
    assert new_numbers_flip_one_zero("00000101") == 6
    assert new_numbers_flip_any_zeros("00000101") == (1 << 6) - 1  # 63
    assert new_numbers_flip_one_zero("1111") == 0
    assert new_numbers_flip_any_zeros("1111") == 0
    assert new_numbers_flip_any_zeros("0000") == 15
