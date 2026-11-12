# Real-file corpus

The manifest pins each anonymous source to an upstream commit. Each entry has its
original filename, download URL, byte length and SHA-256. The fetcher checks both
length and hash, including for a file already present. It does not use mirrors,
authentication or a retry loop.

```sh
python3 -I corpus/fetch.py
python3 -I corpus/fetch.py R02 R03
```

R02 and R03 are repository data, redistributed with the complete upstream
[COPYING](COPYING). All other anonymous files go into the ignored `cache/`
directory. Their golden bytes stay there too. A package software licence alone
does not establish the modification history or terms of a satellite data file.

## Anonymous public files

URLs and hashes below are also recorded in [manifest.json](manifest.json).
SHA-256 values describe the downloaded bytes, not a decoded array.

| ID | File and provenance | Licence and handling | Expectation |
|---|---|---|---|
| R01 | MOD13A2.A2016145.h18v04.006.2016166145124.hdf, from the CRAN MODIS package, 282,525 bytes | Specification classifies this as NASA data in a GPL package. The pinned package DESCRIPTION instead says MIT + file LICENSE. Data-specific modifications and terms remain unconfirmed. Fetch-only. | Values; hand-written EOS expectation in fixtures/expect.json |
| R02 | litend.dat, HDF4 test suite, 2,600 bytes | HDF4 COPYING; committed without edits as R02.hdf | Eight `legacy-sd-layout` skipped NDG objects, refs 2 to 9; no exposed datasets |
| R03 | sds_szipped.dat, HDF4 test suite, 3,090 bytes | HDF4 COPYING; committed without edits as R03.hdf | One dataset with `unsupported:szip` |
| R04a | SDS.hdf, GDAL autotest/gcore/data, 4,613 bytes | GDAL has a general MIT-style licence; data-specific terms remain unconfirmed. Fetch-only. | Raw SD values, as returned by sdsdump |
| R04b | SDSUNLIMITED.hdf, same GDAL directory, 5,741 bytes | The same fetch-only restriction | Raw SD values, as returned by sdsdump |
| R04c | int16_3.hdf, same GDAL directory, 4,634 bytes | The same fetch-only restriction; creation history is incomplete | Raw SD values, as returned by sdsdump |

### Pinned URLs and SHA-256

| ID | Original source URL | SHA-256 |
|---|---|---|
| R01 | [CRAN MODIS file](https://raw.githubusercontent.com/cran/MODIS/3e9f089221aedbdb9bd388a758f78dac64221f76/inst/external/MOD13A2.A2016145.h18v04.006.2016166145124.hdf) | `89cdf5983af54d3bdcb31d61636d5c385c6236743d4edf18d5ee85e392a9566f` |
| R02 | [HDF4 litend.dat](https://raw.githubusercontent.com/HDFGroup/hdf4/2c46ad56bea5aab05e996e473a81dcbd9a66b125/hdf/test/test_files/litend.dat) | `d612b6d92f8f1f7b0bd94acee684411dc92612493e27b53a4f222b71bf3defbf` |
| R03 | [HDF4 sds_szipped.dat](https://raw.githubusercontent.com/HDFGroup/hdf4/2c46ad56bea5aab05e996e473a81dcbd9a66b125/mfhdf/test/sds_szipped.dat) | `50b87a1b2566f3bdded31b9747e38e60643bdc7629d29fb58c2156ccb4841cfa` |
| R04a | [GDAL SDS.hdf](https://raw.githubusercontent.com/OSGeo/gdal/a077f4a553b53bb2d1364259d3018201d11f882a/autotest/gcore/data/SDS.hdf) | `b2079e20b509eec8e46e8edc278d02abfc7bf44c3685946f5a022c7c21b1c79d` |
| R04b | [GDAL SDSUNLIMITED.hdf](https://raw.githubusercontent.com/OSGeo/gdal/a077f4a553b53bb2d1364259d3018201d11f882a/autotest/gcore/data/SDSUNLIMITED.hdf) | `f47be5c7b50d9e18fab3e7d32d7a4876580ea9d18ae87586fb1c45a1c44391b0` |
| R04c | [GDAL int16_3.hdf](https://raw.githubusercontent.com/OSGeo/gdal/a077f4a553b53bb2d1364259d3018201d11f882a/autotest/gcore/data/int16_3.hdf) | `18cc163d5c41ec615317d70a3f47cd6eddb7167bf56f0cef3ce0989027f3cf0e` |

The R02 generator is upstream `hdf/test/litend.c`. It writes little-endian DFSD
arrays, rather than the modern CDF0.0 SD layout. pyhdf exposes synthetic
coordinate variables for those arrays; they do not add skipped objects. The R03
reference is `mfhdf/test/tszip.c`, which identifies the SZIP test array.

[The GDAL creation commit](https://github.com/OSGeo/gdal/commit/6e8bb3c6a526613c01d34f6ff4373ae1f74e422c)
identifies SDS.hdf and SDSUNLIMITED.hdf as outputs of HDF4 examples.
int16_3.hdf was present at the import of the autotest tree in 2007; that does not
establish its data licence. General upstream terms are in
[HDF4 COPYING](https://github.com/HDFGroup/hdf4/blob/2c46ad56bea5aab05e996e473a81dcbd9a66b125/COPYING),
[GDAL LICENSE.TXT](https://github.com/OSGeo/gdal/blob/a077f4a553b53bb2d1364259d3018201d11f882a/LICENSE.TXT)
and [CRAN MODIS DESCRIPTION](https://github.com/cran/MODIS/blob/3e9f089221aedbdb9bd388a758f78dac64221f76/DESCRIPTION).

The retained HDF4 notice includes separate netCDF and XDR notices. It does not
replace this project's own licence. The repository-level NOTICE must retain
these notices when the fixtures are distributed.

## Local-only files

These files require an authenticated data download. No file, access token or
private download URL is included here. No exact granule has been supplied, so no
SHA-256 can be stated. A product label is not a substitute for a file hash.

| ID | Product and original public catalogue | Local path | Expectation |
|---|---|---|---|
| R05 | [MODIS MOD021KM](https://ladsweb.modaps.eosdis.nasa.gov/missions-and-measurements/products/MOD021KM) | local/R05.hdf | Inspect the chosen granule's codecs before setting read expectations |
| R06 | [CERES ES4](https://asdc.larc.nasa.gov/project/CERES) | local/R06.hdf | Inspect the chosen granule's grids and codecs before setting expectations |
| R07 | [AIRS L2](https://airs.jpl.nasa.gov/data/) | local/R07.hdf | Includes 1D Vdata fields, expected as `vdata-field` refusals |

Authentication and granule selection remain local steps. Record a supplied
file's original filename, full SHA-256, product version, source and applicable
terms before using it in a comparison. The fetcher rejects R05 to R07, rather
than attempting to download an arbitrary substitute.
