#include <mfhdf.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(call) do { if ((call) == FAIL) fail(#call); } while (0)

enum Mode { WRITE, VERIFY };

typedef struct {
    const char *directory;
    FILE *status;
    enum Mode mode;
} Dump;

static void fail(const char *operation) {
    fprintf(stderr, "sdsdump: %s failed\n", operation);
    HEprint(stderr, 0);
    exit(EXIT_FAILURE);
}

static void little_endian(uint8_t *bytes, size_t count, size_t width) {
    const uint16_t probe = 1;
    if (*(const uint8_t *)&probe == 1) return;
    for (size_t element = 0; element < count; element++) {
        for (size_t left = 0; left < width / 2; left++) {
            size_t right = width - 1 - left;
            uint8_t saved = bytes[element * width + left];
            bytes[element * width + left] = bytes[element * width + right];
            bytes[element * width + right] = saved;
        }
    }
}

static void verify_values(const char *path, const uint8_t *values, size_t length) {
    FILE *input = fopen(path, "rb");
    if (input == NULL) fail("missing values");
    for (size_t i = 0; i < length; i++) {
        if (fgetc(input) != values[i]) fail("stale values");
    }
    if (fgetc(input) != EOF || ferror(input)) fail("stale value length");
    if (fclose(input) != 0) fail("fclose values");
}

static void dump_dataset(int32 dataset, int index, const Dump *dump) {
    char name[H4_MAX_NC_NAME + 1];
    int32 rank, dims[H4_MAX_VAR_DIMS], type, attributes;
    int32 start[H4_MAX_VAR_DIMS] = {0};
    CHECK(SDgetinfo(dataset, name, &rank, dims, &type, &attributes));
    int width = DFKNTsize((type & DFNT_MASK) | DFNT_NATIVE);
    if (width <= 0) fail("DFKNTsize");
    size_t count = 1;
    for (int32 axis = 0; axis < rank; axis++) {
        if (dims[axis] < 0 || (count != 0 && (size_t)dims[axis] > SIZE_MAX / count)) fail("shape overflow");
        count *= (size_t)dims[axis];
    }
    if (count > SIZE_MAX / (size_t)width) fail("byte count overflow");
    size_t length = count * (size_t)width;
    uint8_t *values = calloc(length == 0 ? 1 : length, 1);
    if (values == NULL) fail("calloc");
    int result = count == 0 ? SUCCEED : SDreaddata(dataset, start, NULL, dims, values);
    fprintf(dump->status, "%s\n  {\"index\": %d, \"read\": \"%s\", \"bytes\": %zu}",
            index == 0 ? "" : ",", index, result == FAIL ? "fail" : "ok", length);
    if (result == FAIL) {
        free(values);
        return;
    }
    little_endian(values, count, (size_t)width);
    char path[4096];
    if (snprintf(path, sizeof(path), "%s/%d.bin", dump->directory, index) >= (int)sizeof(path)) fail("path too long");
    if (dump->mode == VERIFY) {
        verify_values(path, values, length);
        free(values);
        return;
    }
    FILE *output = fopen(path, "wb");
    if (output == NULL) fail("fopen values");
    if (fwrite(values, 1, length, output) != length) fail("fwrite values");
    if (fclose(output) != 0) fail("fclose values");
    free(values);
}

int main(int argc, char **argv) {
    enum Mode mode = WRITE;
    if (argc == 4 && strcmp(argv[1], "--check") == 0) {
        mode = VERIFY;
        argc--;
        argv++;
    }
    if (argc != 3) {
        fprintf(stderr, "usage: sdsdump [--check] INPUT OUTPUT_DIRECTORY\n");
        return EXIT_FAILURE;
    }
    int32 file = SDstart(argv[1], DFACC_READ);
    CHECK(file);
    int32 count, attributes;
    CHECK(SDfileinfo(file, &count, &attributes));
    char path[4096];
    if (snprintf(path, sizeof(path), "%s/status.json", argv[2]) >= (int)sizeof(path)) fail("path too long");
    FILE *status = mode == VERIFY ? tmpfile() : fopen(path, "w");
    if (status == NULL) fail("fopen status");
    fputs("[", status);
    const Dump dump = {argv[2], status, mode};
    for (int index = 0; index < count; index++) {
        int32 dataset = SDselect(file, index);
        CHECK(dataset);
        dump_dataset(dataset, index, &dump);
        CHECK(SDendaccess(dataset));
    }
    fputs("\n]\n", status);
    if (mode == VERIFY) {
        FILE *expected = fopen(path, "r");
        if (expected == NULL) fail("missing read status");
        rewind(status);
        int byte;
        while ((byte = fgetc(status)) != EOF) {
            if (byte != fgetc(expected)) fail("stale read status");
        }
        if (fgetc(expected) != EOF || ferror(expected) || ferror(status)) fail("stale status length");
        if (fclose(expected) != 0) fail("fclose expected status");
    }
    if (fclose(status) != 0) fail("fclose status");
    CHECK(SDend(file));
    return EXIT_SUCCESS;
}
