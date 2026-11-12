#include <mfhdf.h>
#include <HdfEosDef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define CHECK(call) do { if ((call) == FAIL) fail(#call, __LINE__); } while (0)

static const int32 types[] = {DFNT_INT8, DFNT_UINT8, DFNT_INT16, DFNT_UINT16,
    DFNT_INT32, DFNT_UINT32, DFNT_FLOAT32, DFNT_FLOAT64, DFNT_CHAR8, DFNT_UCHAR8};
static const char *names[] = {"i8", "u8", "i16", "u16", "i32", "u32", "f32", "f64", "char8", "uchar8"};

typedef union {
    int8_t i8[35];
    uint8_t u8[35];
    int16_t i16[35];
    uint16_t u16[35];
    int32_t i32[35];
    uint32_t u32[35];
    float f32[35];
    double f64[35];
} Values;

static void fail(const char *operation, int line) {
    fprintf(stderr, "gen.c:%d: %s failed\n", line, operation);
    HEprint(stderr, 0);
    exit(EXIT_FAILURE);
}

static void pattern(Values *values, int32 type) {
    memset(values, 0, sizeof(*values));
    for (int i = 0; i < 35; i++) {
        switch (type & DFNT_MASK) {
            case DFNT_CHAR8: values->i8[i] = (int8_t)(65 + i); break;
            case DFNT_INT8: values->i8[i] = (int8_t)(-91 + 5 * i); break;
            case DFNT_UCHAR8:
            case DFNT_UINT8: values->u8[i] = (uint8_t)(137 + 3 * i); break;
            case DFNT_INT16: values->i16[i] = (int16_t)(-0x6123 + i * 0x0107); break;
            case DFNT_UINT16: values->u16[i] = (uint16_t)(0x8123 + i * 0x0107); break;
            case DFNT_INT32: values->i32[i] = -0x61234567 + i * 0x010203; break;
            case DFNT_UINT32: values->u32[i] = 0x81234567U + (uint32_t)i * 0x010203U; break;
            case DFNT_FLOAT32: values->f32[i] = -123.25f + (float)i * 3.125f; break;
            case DFNT_FLOAT64: values->f64[i] = -98765.125 + (double)i * 1.03125; break;
            default: fail("unknown pattern type", __LINE__);
        }
    }
}

static int32 create2(int32 file, const char *name, int32 type) {
    int32 dims[] = {3, 5};
    int32 dataset = SDcreate(file, name, type, 2, dims);
    CHECK(dataset);
    return dataset;
}

static void write2(int32 dataset, int32 type) {
    Values values;
    int32 start[] = {0, 0};
    int32 edges[] = {3, 5};
    pattern(&values, type);
    CHECK(SDwritedata(dataset, start, NULL, edges, &values));
}

static void all_types(int32 file, int32 order) {
    for (size_t i = 0; i < sizeof(types) / sizeof(types[0]); i++) {
        int32 dataset = create2(file, names[i], types[i] | order);
        write2(dataset, types[i]);
        CHECK(SDendaccess(dataset));
    }
}

static void all_attributes(int32 object) {
    for (size_t i = 0; i < sizeof(types) / sizeof(types[0]); i++) {
        Values values;
        pattern(&values, types[i]);
        CHECK(SDsetattr(object, names[i], types[i], 3, &values));
    }
}

static void attributes_fixture(int32 file) {
    int32 dataset = create2(file, "attributes", DFNT_INT16);
    int32 dimension = SDgetdimid(dataset, 0);
    double scale = 0.125;
    int16_t fill = -12345;
    CHECK(dimension);
    all_attributes(file);
    all_attributes(dataset);
    all_attributes(dimension);
    CHECK(SDsetattr(dataset, "scale_factor", DFNT_FLOAT64, 1, &scale));
    CHECK(SDsetfillvalue(dataset, &fill));
    write2(dataset, DFNT_INT16);
    CHECK(SDendaccess(dataset));
}

static void dimensions_fixture(int32 file) {
    int16_t rows[] = {-513, 257, 1027};
    int32 columns[] = {0x010203, 0x020407, 0x030509, 0x04060b, 0x05070d};
    for (int i = 0; i < 2; i++) {
        int32 dataset = create2(file, i == 0 ? "first" : "second", DFNT_INT16);
        int32 row = SDgetdimid(dataset, 0);
        int32 column = SDgetdimid(dataset, 1);
        CHECK(row);
        CHECK(column);
        CHECK(SDsetdimname(row, "row"));
        CHECK(SDsetdimname(column, "column"));
        CHECK(SDsetdimval_comp(row, SD_DIMVAL_BW_COMP));
        CHECK(SDsetdimval_comp(column, SD_DIMVAL_BW_INCOMP));
        CHECK(SDsetdimscale(row, 3, DFNT_INT16, rows));
        CHECK(SDsetdimscale(column, 5, DFNT_INT32, columns));
        write2(dataset, DFNT_INT16);
        CHECK(SDendaccess(dataset));
    }
}

static void append_fixture(int32 file) {
    int32 dims[] = {SD_UNLIMITED, 5};
    int32 dataset = SDcreate(file, "records", DFNT_INT16, 2, dims);
    CHECK(dataset);
    for (int row = 0; row < 5; row++) {
        int16_t values[5];
        int32 start[] = {row, 0};
        int32 edges[] = {1, 5};
        for (int col = 0; col < 5; col++) values[col] = (int16_t)(0x1234 + row * 0x0301 + col * 0x0107);
        CHECK(SDwritedata(dataset, start, NULL, edges, values));
    }
    CHECK(SDendaccess(dataset));
}

static void compressed_fixture(int32 file, comp_coder_t codec) {
    int32 dataset = create2(file, "compressed", DFNT_INT32);
    comp_info info;
    memset(&info, 0, sizeof(info));
    if (codec == COMP_CODE_DEFLATE) info.deflate.level = 6;
    if (codec == COMP_CODE_SKPHUFF) info.skphuff.skp_size = 4;
    CHECK(SDsetcompress(dataset, codec, &info));
    write2(dataset, DFNT_INT32);
    CHECK(SDendaccess(dataset));
    if (codec != COMP_CODE_DEFLATE) return;
    dataset = create2(file, "compressed", DFNT_INT32);
    write2(dataset, DFNT_INT32);
    CHECK(SDendaccess(dataset));
}

static void chunks_fixture(int32 file, int fixture) {
    int32 dims[] = {5, 7};
    int32 dataset = SDcreate(file, "chunks", DFNT_INT16, 2, dims);
    int16_t fill = -12345;
    HDF_CHUNK_DEF chunk;
    CHECK(dataset);
    CHECK(SDsetfillvalue(dataset, &fill));
    memset(&chunk, 0, sizeof(chunk));
    chunk.chunk_lengths[0] = 2;
    chunk.chunk_lengths[1] = 3;
    int32 flags = HDF_CHUNK;
    if (fixture == 8) {
        chunk.comp.comp_type = COMP_CODE_DEFLATE;
        chunk.comp.cinfo.deflate.level = 6;
        flags |= HDF_COMP;
    }
    CHECK(SDsetchunk(dataset, chunk, flags));
    if (fixture == 19) {
        int32 origin[] = {1, 1};
        int16_t values[] = {0x2134, 0x2235, 0x2336, 0x2437, 0x2538, 0x2639};
        CHECK(SDwritechunk(dataset, origin, values));
    } else {
        int16_t values[35];
        int32 start[] = {0, 0};
        int32 edges[] = {5, 7};
        for (int row = 0; row < 5; row++) {
            for (int col = 0; col < 7; col++) values[row * 7 + col] = (int16_t)(0x1234 + row * 0x0301 + col * 0x0107);
        }
        CHECK(SDwritedata(dataset, start, NULL, edges, values));
    }
    CHECK(SDendaccess(dataset));
}

static void empty_fixture(int32 file) {
    for (size_t i = 0; i < sizeof(types) / sizeof(types[0]); i++) {
        int32 dataset = create2(file, names[i], types[i]);
        CHECK(SDendaccess(dataset));
    }
    int32 dataset = create2(file, "custom_fill", DFNT_INT16);
    int16_t fill = -12345;
    CHECK(SDsetfillvalue(dataset, &fill));
    CHECK(SDendaccess(dataset));
    int32 dims[] = {SD_UNLIMITED, 5};
    dataset = SDcreate(file, "empty_records", DFNT_INT16, 2, dims);
    CHECK(dataset);
    CHECK(SDendaccess(dataset));
}

static void split_metadata(const char *path, int parts) {
    int32 file = SDstart(path, DFACC_RDWR);
    CHECK(file);
    int index = SDfindattr(file, "StructMetadata.0");
    int32 type, count;
    char name[H4_MAX_NC_NAME + 1];
    CHECK(index);
    CHECK(SDattrinfo(file, index, name, &type, &count));
    char *text = calloc((size_t)count + 1, 1);
    if (text == NULL) fail("calloc metadata", __LINE__);
    CHECK(SDreadattr(file, index, text));
    size_t length = strlen(text);
    /* EHapi.c 1117-1160: concatenate consecutive attributes without embedded padding. */
    for (int part = 0; part < parts; part++) {
        size_t start = length * (size_t)part / (size_t)parts;
        size_t end = length * (size_t)(part + 1) / (size_t)parts;
        snprintf(name, sizeof(name), "StructMetadata.%d", part);
        CHECK(SDsetattr(file, name, DFNT_CHAR8, (int32)(end - start), text + start));
    }
    free(text);
    CHECK(SDend(file));
}

static void grid_fixture(char *path, int parts) {
    int32 file = GDopen(path, DFACC_CREATE);
    float64 upper[] = {0, 3000000};
    float64 lower[] = {5000000, 0};
    int16_t values[15];
    CHECK(file);
    int32 grid = GDcreate(file, "TinyGrid", 5, 3, upper, lower);
    CHECK(grid);
    CHECK(GDdefproj(grid, GCTP_GEO, 0, 0, NULL));
    CHECK(GDdeffield(grid, "temperature", "YDim,XDim", DFNT_INT16, HDFE_NOMERGE));
    for (int i = 0; i < 15; i++) values[i] = (int16_t)(0x1234 + i * 0x0107);
    CHECK(GDwritefield(grid, "temperature", NULL, NULL, NULL, values));
    CHECK(GDdetach(grid));
    CHECK(GDclose(file));
    split_metadata(path, parts);
}

static void swath_fixture(const char *path) {
    int32 file = SWopen(path, DFACC_CREATE);
    CHECK(file);
    int32 swath = SWcreate(file, "TinySwath");
    int32 indices[] = {0, 2, 4};
    CHECK(swath);
    CHECK(SWdefdim(swath, "GeoTrack", 3));
    CHECK(SWdefdim(swath, "DataTrack", 3));
    CHECK(SWdefdim(swath, "XTrack", 5));
    CHECK(SWdefdimmap(swath, "GeoTrack", "DataTrack", -1, 2));
    CHECK(SWdefidxmap(swath, "GeoTrack", "XTrack", indices));
    CHECK(SWdefgeofield(swath, "Latitude", "GeoTrack,XTrack", DFNT_FLOAT32, HDFE_NOMERGE));
    CHECK(SWdefdatafield(swath, "signal", "DataTrack,XTrack", DFNT_INT16, HDFE_NOMERGE));
    CHECK(SWdefdatafield(swath, "Time", "DataTrack", DFNT_INT32, HDFE_NOMERGE));
    CHECK(SWdefdatafield(swath, "MergedA", "DataTrack,XTrack", DFNT_INT16, HDFE_AUTOMERGE));
    CHECK(SWdefdatafield(swath, "MergedB", "DataTrack,XTrack", DFNT_INT16, HDFE_AUTOMERGE));
    /* HDF-EOS2 SWapi.c 12744-12820: detach materializes auto-merged SDS storage. */
    CHECK(SWdetach(swath));
    swath = SWattach(file, "TinySwath");
    CHECK(swath);
    Values values;
    pattern(&values, DFNT_FLOAT32);
    CHECK(SWwritefield(swath, "Latitude", NULL, NULL, NULL, &values));
    pattern(&values, DFNT_INT16);
    CHECK(SWwritefield(swath, "signal", NULL, NULL, NULL, &values));
    CHECK(SWwritefield(swath, "MergedA", NULL, NULL, NULL, &values));
    for (int i = 0; i < 15; i++) values.i16[i] = (int16_t)(0x2134 + i * 0x0107);
    CHECK(SWwritefield(swath, "MergedB", NULL, NULL, NULL, &values));
    pattern(&values, DFNT_INT32);
    CHECK(SWwritefield(swath, "Time", NULL, NULL, NULL, &values));
    CHECK(SWdetach(swath));
    CHECK(SWclose(file));
    split_metadata(path, 1);
}

static void v4_fixture(const char *path) {
    int32 file = Hopen(path, DFACC_RDWR, 0);
    CHECK(file);
    CHECK(Vstart(file));
    int32 ref = Vfind(file, "v4");
    CHECK(ref);
    int32 variable = Vattach(file, ref, "w");
    CHECK(variable);
    int32 marker[] = {0x12345678, 0x23456701};
    CHECK(Vsetattr(variable, "vgroup_marker", DFNT_INT32, 2, marker));
    CHECK(Vdetach(variable));
    CHECK(Vend(file));
    CHECK(Hclose(file));
}

static void truncate_fixture(const char *path) {
    int32 file = Hopen(path, DFACC_RDWR, 0);
    CHECK(file);
    /* hfile.c 1777-1819: shorten the descriptor; SD_NOFILL alone preallocates. */
    int32 access = Hstartaccess(file, DFTAG_SD, DFREF_WILDCARD, DFACC_RDWR);
    CHECK(access);
    CHECK(Htrunc(access, 4));
    CHECK(Hendaccess(access));
    CHECK(Hclose(file));
}

static void generate(int fixture) {
    char path[32];
    snprintf(path, sizeof(path), "F%02d.hdf", fixture);
    if (fixture == 15 || fixture == 18) {
        grid_fixture(path, fixture == 18 ? 3 : 1);
        return;
    }
    if (fixture == 16) {
        swath_fixture(path);
        return;
    }
    if (fixture == 14) {
        int32 dims[] = {3, 5};
        int16_t values[15];
        for (int i = 0; i < 15; i++) values[i] = (int16_t)(0x1234 + i * 0x0107);
        CHECK(DFSDsetNT(DFNT_INT16));
        CHECK(DFSDadddata(path, 2, dims, values));
        return;
    }
    int32 file = SDstart(path, DFACC_CREATE);
    CHECK(file);
    switch (fixture) {
        case 1: all_types(file, 0); break;
        case 2: all_types(file, DFNT_LITEND); break;
        case 3: attributes_fixture(file); break;
        case 4: dimensions_fixture(file); break;
        case 5: append_fixture(file); break;
        case 6: compressed_fixture(file, COMP_CODE_DEFLATE); break;
        case 7:
        case 8:
        case 19: chunks_fixture(file, fixture); break;
        case 9: compressed_fixture(file, COMP_CODE_RLE); break;
        case 10: compressed_fixture(file, COMP_CODE_SKPHUFF); break;
        case 11: {
            int32 dataset = create2(file, "nbit", DFNT_INT16);
            CHECK(SDsetnbitdataset(dataset, 14, 15, 1, 0));
            write2(dataset, DFNT_INT16);
            CHECK(SDendaccess(dataset));
            break;
        }
        case 12: {
            int32 dataset = create2(file, "external", DFNT_INT16);
            CHECK(SDsetexternalfile(dataset, "F12.raw", 0));
            write2(dataset, DFNT_INT16);
            CHECK(SDendaccess(dataset));
            break;
        }
        case 13: empty_fixture(file); break;
        case 17: {
            int32 dataset = create2(file, "v4", DFNT_INT16);
            write2(dataset, DFNT_INT16);
            CHECK(SDendaccess(dataset));
            break;
        }
        case 20: {
            int32 dataset = create2(file, "unwritten_deflate", DFNT_INT16);
            comp_info info;
            int16_t fill = -12345;
            memset(&info, 0, sizeof(info));
            info.deflate.level = 6;
            CHECK(SDsetfillvalue(dataset, &fill));
            CHECK(SDsetcompress(dataset, COMP_CODE_DEFLATE, &info));
            CHECK(SDendaccess(dataset));
            break;
        }
        case 21: {
            CHECK(SDsetfillmode(file, SD_NOFILL));
            int32 dataset = create2(file, "short", DFNT_INT16);
            int32 start[] = {0, 0};
            int32 edges[] = {1, 2};
            int16_t values[] = {0x1234, 0x5678};
            CHECK(SDwritedata(dataset, start, NULL, edges, values));
            CHECK(SDendaccess(dataset));
            break;
        }
        default: fail("unknown fixture", __LINE__);
    }
    CHECK(SDend(file));
    if (fixture == 17) v4_fixture(path);
    if (fixture == 21) truncate_fixture(path);
}

int main(void) {
    /* DFSDadddata appends by definition, so rebuilding starts from absent files. */
    for (int fixture = 1; fixture <= 21; fixture++) {
        char path[32];
        snprintf(path, sizeof(path), "F%02d.hdf", fixture);
        if (access(path, F_OK) == 0 && unlink(path) != 0) fail("unlink fixture", __LINE__);
    }
    if (access("F12.raw", F_OK) == 0 && unlink("F12.raw") != 0) fail("unlink external data", __LINE__);
    for (int fixture = 1; fixture <= 21; fixture++) generate(fixture);
    puts("Generated F01-F21 through the HDF4 C API.");
    return EXIT_SUCCESS;
}
