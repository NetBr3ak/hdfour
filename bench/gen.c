#include <mfhdf.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(call) do { if ((call) == FAIL) fail(#call, __LINE__); } while (0)

typedef struct {
    const char *root;
    int32 chunked_side;
    int32 chunk_side;
    int32 contiguous_side;
} Options;

static void fail(const char *operation, int line) {
    fprintf(stderr, "gen.c:%d: %s failed\n", line, operation);
    HEprint(stderr, 0);
    exit(EXIT_FAILURE);
}

static FILE *create_output(const char *path) {
    FILE *output = fopen(path, "wb");
    if (output == NULL) fail("fopen", __LINE__);
    return output;
}

static void pattern(void *buffer, int32 side, int dataset, int chunked) {
    for (int32 row = 0; row < side; row++) {
        for (int32 column = 0; column < side; column++) {
            size_t index = (size_t)row * (size_t)side + (size_t)column;
            if (chunked) {
                ((int16_t *)buffer)[index] = (int16_t)(
                    (row * 17 + column * 31 + dataset * 101) % 60001 - 30000);
            } else {
                ((float *)buffer)[index] = (float)(index % 65536) * 0.125f - 4096.0f;
            }
        }
    }
}

static void write_dataset(int32 file, const Options *options, int index, int chunked) {
    char name[32];
    snprintf(name, sizeof(name), chunked ? "chunked_%d" : "contiguous_%d", index);
    int32 side = chunked ? options->chunked_side : options->contiguous_side;
    int32 shape[] = {side, side};
    int32 start[] = {0, 0};
    int32 type = chunked ? DFNT_INT16 : DFNT_FLOAT32;
    int32 dataset = SDcreate(file, name, type, 2, shape);
    CHECK(dataset);
    if (chunked) {
        HDF_CHUNK_DEF chunk;
        memset(&chunk, 0, sizeof(chunk));
        chunk.chunk_lengths[0] = options->chunk_side;
        chunk.chunk_lengths[1] = options->chunk_side;
        chunk.comp.comp_type = COMP_CODE_DEFLATE;
        chunk.comp.cinfo.deflate.level = 6;
        CHECK(SDsetchunk(dataset, chunk, HDF_CHUNK | HDF_COMP));
    }
    size_t width = chunked ? sizeof(int16_t) : sizeof(float);
    void *buffer = malloc((size_t)side * (size_t)side * width);
    if (buffer == NULL) fail("malloc", __LINE__);
    pattern(buffer, side, index, chunked);
    CHECK(SDwritedata(dataset, start, NULL, shape, buffer));
    free(buffer);
    CHECK(SDendaccess(dataset));
}

static void write_little_endian(FILE *output, void *buffer, size_t count, size_t width) {
    const uint16_t one = 1;
    unsigned char *bytes = buffer;
    if (*(const unsigned char *)&one != 1) {
        for (size_t offset = 0; offset < count * width; offset += width) {
            for (size_t i = 0; i < width / 2; i++) {
                unsigned char saved = bytes[offset + i];
                bytes[offset + i] = bytes[offset + width - i - 1];
                bytes[offset + width - i - 1] = saved;
            }
        }
    }
    if (fwrite(bytes, width, count, output) != count) fail("fwrite", __LINE__);
    if (fclose(output) != 0) fail("fclose", __LINE__);
}

static void dump_golden(const char *path, const char *directory) {
    int32 file = SDstart(path, DFACC_READ);
    CHECK(file);
    int32 datasets, attributes;
    CHECK(SDfileinfo(file, &datasets, &attributes));
    char destination[4096];
    if (snprintf(destination, sizeof(destination), "%s/manifest.json", directory) >=
        (int)sizeof(destination)) fail("output path too long", __LINE__);
    FILE *manifest = create_output(destination);
    fputs("[\n", manifest);
    for (int32 index = 0; index < datasets; index++) {
        int32 dataset = SDselect(file, index);
        CHECK(dataset);
        char name[H4_MAX_NC_NAME + 1];
        int32 rank, shape[H4_MAX_VAR_DIMS], type, attrs;
        CHECK(SDgetinfo(dataset, name, &rank, shape, &type, &attrs));
        if (rank != 2) fail("expected rank two", __LINE__);
        size_t width = type == DFNT_INT16 ? sizeof(int16_t) : sizeof(float);
        size_t count = (size_t)shape[0] * (size_t)shape[1];
        void *buffer = malloc(count * width);
        if (buffer == NULL) fail("malloc", __LINE__);
        int32 start[] = {0, 0};
        CHECK(SDreaddata(dataset, start, NULL, shape, buffer));
        if (snprintf(destination, sizeof(destination), "%s/%d.bin", directory, (int)index) >=
            (int)sizeof(destination)) fail("output path too long", __LINE__);
        write_little_endian(create_output(destination), buffer, count, width);
        free(buffer);
        if (fprintf(manifest,
            "%s{\"name\":\"%s\",\"dtype\":\"%s\",\"shape\":[%d,%d]}",
            index == 0 ? "" : ",\n", name, type == DFNT_INT16 ? "<i2" : "<f4",
            (int)shape[0], (int)shape[1]) < 0) fail("fprintf", __LINE__);
        CHECK(SDendaccess(dataset));
    }
    fputs("\n]\n", manifest);
    if (fclose(manifest) != 0) fail("fclose", __LINE__);
    CHECK(SDend(file));
}

static void generate(const Options *options, int chunked) {
    char path[4096], directory[4096];
    const char *kind = chunked ? "chunked" : "contiguous";
    if (snprintf(path, sizeof(path), "%s/%s.hdf", options->root, kind) >=
        (int)sizeof(path)) fail("output path too long", __LINE__);
    if (snprintf(directory, sizeof(directory), "%s/%s", options->root, kind) >=
        (int)sizeof(directory)) fail("output path too long", __LINE__);
    int32 file = SDstart(path, DFACC_CREATE);
    CHECK(file);
    for (int index = 0; index < (chunked ? 4 : 1); index++) {
        write_dataset(file, options, index, chunked);
    }
    CHECK(SDend(file));
    dump_golden(path, directory);
}

int main(int argc, char **argv) {
    if (argc != 3 || (strcmp(argv[2], "small") != 0 && strcmp(argv[2], "full") != 0)) {
        fprintf(stderr, "Usage: gen OUTPUT_DIRECTORY small|full\n");
        return EXIT_FAILURE;
    }
    int small = strcmp(argv[2], "small") == 0;
    Options options = {argv[1], small ? 48 : 4800, small ? 16 : 480, small ? 64 : 8192};
    generate(&options, 1);
    generate(&options, 0);
    puts("PASS generated files and SDreaddata golden bytes");
    return EXIT_SUCCESS;
}
