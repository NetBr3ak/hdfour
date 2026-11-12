"""Read HDF4 scientific datasets without calibration or fill masking.

Use open(path) or from_bytes(data), then select a dataset and call read().
"""

from ._hdfour import (
    Dataset,
    Dimension,
    File,
    FormatError,
    LimitError,
    UnsupportedError,
    from_bytes,
    open,
)

__all__ = [
    "Dataset",
    "Dimension",
    "File",
    "FormatError",
    "LimitError",
    "UnsupportedError",
    "from_bytes",
    "open",
]
