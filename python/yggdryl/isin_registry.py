"""The instrument registry: one row per ISIN and market of the facts it is known by, learned from and filled into market data."""

from __future__ import annotations

from ._native import IsinRegistry

__all__ = ["IsinRegistry"]
