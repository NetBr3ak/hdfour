from os import PathLike
from typing import Literal, TypeAlias, TypedDict

import numpy as np
from numpy.typing import NDArray

Attrs: TypeAlias = dict[str, str | NDArray[np.generic]]
DType: TypeAlias = Literal[
    "i8", "u8", "i16", "u16", "i32", "u32", "f32", "f64", "char8", "uchar8"
]
Codec: TypeAlias = Literal["none", "rle", "deflate"] | int

class Storage(TypedDict, total=False):
    kind: Literal["empty", "contiguous", "linked", "chunked", "external"]
    chunk_shape: list[int]

class EosField(TypedDict):
    name: str
    dims: list[str]
    geolocation: bool
    dataset: int | None

class DimensionMap(TypedDict):
    geo_dim: str
    data_dim: str
    offset: int
    increment: int

class EosObject(TypedDict):
    name: str
    dims: list[tuple[str, int]]
    fields: list[EosField]
    dimension_maps: list[DimensionMap]
    params: list[tuple[str, str]]

class Eos(TypedDict):
    grids: list[EosObject]
    swaths: list[EosObject]

class FormatError(ValueError): ...
class UnsupportedError(NotImplementedError): ...
class LimitError(ValueError): ...

def open(path: str | PathLike[str]) -> File: ...
def from_bytes(data: bytes) -> File: ...

class File:
    @property
    def datasets(self) -> list[Dataset]: ...
    @property
    def attrs(self) -> Attrs: ...
    @property
    def skipped(self) -> list[tuple[str, str]]: ...
    @property
    def eos(self) -> Eos | None: ...
    def __getitem__(self, name: str) -> Dataset: ...

class Dataset:
    @property
    def name(self) -> str: ...
    @property
    def dtype(self) -> DType: ...
    @property
    def shape(self) -> list[int]: ...
    @property
    def dims(self) -> list[Dimension]: ...
    @property
    def attrs(self) -> Attrs: ...
    @property
    def is_coordinate(self) -> bool: ...
    @property
    def storage(self) -> Storage: ...
    @property
    def codec(self) -> Codec: ...
    def read(self) -> NDArray[np.generic]: ...

class Dimension:
    @property
    def name(self) -> str: ...
    @property
    def size(self) -> int: ...
    @property
    def unlimited(self) -> bool: ...
    @property
    def attrs(self) -> Attrs: ...
