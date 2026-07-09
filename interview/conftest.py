"""Pytest configuration for the interview prep suite.

Adds this directory to ``sys.path`` so the test modules under ``tests/`` can
import the solution modules directly, e.g. ``from q1_star_schema_max_sum import
max_star_sum``.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
