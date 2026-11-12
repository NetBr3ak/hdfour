# Structural metadata inputs

These hand-written texts exercise ODL and EOS metadata mapping without an HDF
container. They are not C-library fixtures and do not establish dataset support.

| Input | Expected result |
| --- | --- |
| `grid.odl` | One grid, two data fields, two horizontal dimensions and one band dimension. Projection entries keep their raw text. |
| `swath.odl` | One swath, geolocation and data fields, and a linear dimension map. Two merged fields and one index dimension map are skipped. |
| `point.odl` | One point structure, reported as `eos-point`. |
| `too-deep.odl` | A grid with synthetic nested groups. Its 65th level exceeds the default group depth limit. |

The layout follows these source locations in **GDAL 1.11.5**:

- [GDapi.c, lines 467-515](https://github.com/OSGeo/gdal/blob/v1.11.5/gdal/frmts/hdf4/hdf-eos/GDapi.c#L467-L515): grid name, horizontal dimensions, corners and child groups.
- [SWapi.c, lines 388-408](https://github.com/OSGeo/gdal/blob/v1.11.5/gdal/frmts/hdf4/hdf-eos/SWapi.c#L388-L408): swath name and child groups.
- [EHapi.c, lines 1902-1944](https://github.com/OSGeo/gdal/blob/v1.11.5/gdal/frmts/hdf4/hdf-eos/EHapi.c#L1902-L1944): dimension and linear dimension map objects.
- [EHapi.c, lines 1976-1980](https://github.com/OSGeo/gdal/blob/v1.11.5/gdal/frmts/hdf4/hdf-eos/EHapi.c#L1976-L1980): index dimension map objects.
- [EHapi.c, lines 2024-2028 and 2086-2102](https://github.com/OSGeo/gdal/blob/v1.11.5/gdal/frmts/hdf4/hdf-eos/EHapi.c#L2024-L2102): geolocation and data field objects, including dimension lists.
- [EHapi.c, lines 2141-2212](https://github.com/OSGeo/gdal/blob/v1.11.5/gdal/frmts/hdf4/hdf-eos/EHapi.c#L2141-L2212): merged field lists and point fields.

The older tag includes the write functions removed from current GDAL. Current
`SWapi.c` still reads index maps at lines 2152-2160 and merged fields at lines
2501-2554. Names, dimensions and values here are synthetic. No upstream source
text or sample data was copied.

Mapping conventions: `XDim` and `YDim` become grid dimensions. Object names and
those two sizes are not repeated in `params`. Other direct assignments, including
list-valued projection entries, retain their exact value text in `params`.
Merged field refusals name each constituent field, not its backing SDS.
