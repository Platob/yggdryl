"""Q7 — Bit-stream manipulation.

Convert decimal numbers into fixed-length bit streams (left-padding with zeros
when the number is smaller than the width). Then, per stream, count the zeros
and the number of *new* numbers reachable by flipping zero bits to one.

Two flip interpretations are provided (the prep wording is ambiguous):
- flip exactly one zero  -> ``z`` new numbers (one per zero bit).
- flip any non-empty set  -> ``2**z - 1`` new numbers.
"""

from typing import List


def to_bitstream(n: int, length: int) -> str:
    """Binary string of ``n`` padded to ``length`` bits (MSB first)."""
    if n < 0:
        raise ValueError(f"expected a non-negative integer, got {n}")
    bits = format(n, "b")
    if len(bits) > length:
        raise ValueError(f"{n} needs {len(bits)} bits, more than the width {length}")
    return bits.zfill(length)


def expand_all(numbers: List[int], length: int) -> List[str]:
    """Convert every number to a fixed-``length`` bit stream."""
    return [to_bitstream(n, length) for n in numbers]


def count_zeros(bitstream: str) -> int:
    """Number of ``0`` bits in the stream."""
    return bitstream.count("0")


def new_numbers_flip_one_zero(bitstream: str) -> int:
    """Distinct numbers made by flipping exactly one ``0`` to ``1``."""
    return count_zeros(bitstream)


def new_numbers_flip_any_zeros(bitstream: str) -> int:
    """Distinct new numbers from flipping any non-empty subset of zeros to ones.

    Each zero independently stays or flips (``2**z`` combinations); subtract the
    all-stay combination, which is the original number.
    """
    return (1 << count_zeros(bitstream)) - 1
