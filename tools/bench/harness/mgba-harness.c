// mGBA adapter for tools/bench/run.mjs. The harness contract is shared
// by every adapter in this directory (see ../README.md):
//
// usage: mgba-harness ROM FRAMES [--bios=PATH] [--dump-frame=PATH]
//
// Without --bios mGBA skips to the cartridge with its HLE BIOS; with --bios it
// maps that image and still skips the boot animation. The timed loop runs
// FRAMES * 280896 master cycles from power-on through runLoop and a stop event
// at each frame boundary (runFrame would end frames at VBlank), with no input,
// no save media, a software video buffer and emulated audio that is never
// drained. No ambient mGBA configuration is loaded; idle-loop detection keeps
// mGBA's default ("detect") and is reported in "settings".
//
// Links pinned mGBA's libmgba (MPL-2.0); no mGBA source is copied here.
// <mgba/flags.h> comes first because struct mCore changes with USE_DEBUGGERS,
// and mTimingGlobalTime only advances in USE_DEBUGGERS builds.
#include <mgba/flags.h>
#ifndef USE_DEBUGGERS
#error "mgba-harness needs a USE_DEBUGGERS libmgba (mTimingGlobalTime)"
#endif

#include <mgba/core/config.h>
#include <mgba/core/core.h>
#include <mgba/core/interface.h>
#include <mgba/core/log.h>
#include <mgba/core/timing.h>
#include <mgba/internal/gba/gba.h>

#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define CYCLES_PER_FRAME 280896

static uint16_t _rgb555(color_t pixel) {
#ifdef COLOR_16_BIT
#ifdef COLOR_5_6_5
    const uint16_t p = pixel;
    return (uint16_t) (((p >> 11) & 31) | (((p >> 6) & 31) << 5) | ((p & 31) << 10));
#else
    return pixel & 0x7FFF;
#endif
#else
    const uint32_t p = pixel;
    return (uint16_t) (((p >> 3) & 31) | (((p >> 11) & 31) << 5) | (((p >> 19) & 31) << 10));
#endif
}

// FNV-1a 64 over visible RGB555 pixels, low byte then high byte.
static uint64_t _framebufferHash(const color_t* pixels) {
    uint64_t hash = UINT64_C(14695981039346656037);
    for (size_t y = 0; y < 160; ++y) {
        for (size_t x = 0; x < 240; ++x) {
            const uint16_t rgb555 = _rgb555(pixels[y * 256 + x]);
            hash = (hash ^ (uint8_t) rgb555) * UINT64_C(1099511628211);
            hash = (hash ^ (uint8_t) (rgb555 >> 8)) * UINT64_C(1099511628211);
        }
    }
    return hash;
}

static int _writePpm(const char* path, const color_t* pixels) {
    FILE* out = fopen(path, "wb");
    if (!out) {
        perror("dump-frame");
        return 1;
    }
    fprintf(out, "P6\n240 160\n255\n");
    for (size_t y = 0; y < 160; ++y) {
        for (size_t x = 0; x < 240; ++x) {
            const uint16_t rgb555 = _rgb555(pixels[y * 256 + x]);
            for (int channel = 0; channel < 3; ++channel) {
                const uint8_t value = (rgb555 >> (5 * channel)) & 31;
                fputc((value << 3) | (value >> 2), out);
            }
        }
    }
    return fclose(out) ? 1 : 0;
}

static void _noopLog(struct mLogger* logger, int category, enum mLogLevel level, const char* format, va_list args) {
    (void) logger;
    (void) category;
    (void) level;
    (void) format;
    (void) args;
}

// mGBA draws line 0 of the next frame by the time the last frame boundary is
// reached, so the reported image is copied when the final frame's VBlank
// starts, the point at which every other adapter's image is also complete.
struct FrameCapture {
    const color_t* source;
    bool armed;
    color_t pixels[256 * 160];
};

static void _captureFrame(void* context) {
    struct FrameCapture* capture = context;
    if (capture->armed) {
        memcpy(capture->pixels, capture->source, sizeof(capture->pixels));
        capture->armed = false;
    }
}

// Only sets earlyExit, which also ends event processing while halted.
static void _stopAtTarget(struct mTiming* timing, void* context, uint32_t cyclesLate) {
    (void) timing;
    (void) cyclesLate;
    ((struct GBA*) context)->earlyExit = true;
}

static int _usage(void) {
    fprintf(stderr, "usage: mgba-harness ROM FRAMES [--bios=PATH] [--dump-frame=PATH]\n");
    return 2;
}

int main(int argc, char** argv) {
    const char* bios_path = NULL;
    const char* dump_frame_path = NULL;
    const char* positional[2];
    int positional_count = 0;
    for (int i = 1; i < argc; ++i) {
        if (!strncmp(argv[i], "--bios=", 7) && argv[i][7] && !bios_path) {
            bios_path = argv[i] + 7;
        } else if (!strncmp(argv[i], "--dump-frame=", 13) && argv[i][13] && !dump_frame_path) {
            dump_frame_path = argv[i] + 13;
        } else if (strncmp(argv[i], "--", 2) && positional_count < 2) {
            positional[positional_count++] = argv[i];
        } else {
            return _usage();
        }
    }
    if (positional_count != 2) {
        return _usage();
    }
    const char* rom_path = positional[0];
    char* end = NULL;
    const unsigned long frames_arg = strtoul(positional[1], &end, 10);
    if (!positional[1][0] || *end || !frames_arg || frames_arg > UINT32_MAX) {
        return _usage();
    }
    const uint32_t frames = (uint32_t) frames_arg;

    static struct mLogger noopLogger = { .log = _noopLog, .filter = NULL };
    mLogSetDefaultLogger(&noopLogger);

    struct mCore* core = mCoreFind(rom_path);
    if (!core || !core->init(core)) {
        fprintf(stderr, "mGBA core init failed for %s\n", rom_path);
        return 1;
    }
    static color_t vbuf[256 * 160];
    core->setVideoBuffer(core, vbuf, 256);
    if (!mCoreLoadFile(core, rom_path)) {
        fprintf(stderr, "mCoreLoadFile failed\n");
        return 1;
    }
    mCoreConfigInit(&core->config, "emu-bench");
    // Deliberately no mCoreConfigLoad: the ambient mGBA config must not leak in.
    mCoreConfigSetOverrideIntValue(&core->config, "skipBios", 1);
    mCoreConfigSetOverrideIntValue(&core->config, "useBios", bios_path ? 1 : 0);
    if (bios_path) {
        mCoreConfigSetOverrideValue(&core->config, "bios", bios_path);
    }
    mCoreConfigSetOverrideIntValue(&core->config, "audioSync", 0);
    mCoreConfigSetOverrideIntValue(&core->config, "videoSync", 0);
    mCoreConfigSetOverrideValue(&core->config, "idleOptimization", "detect");
    struct mCoreOptions opts = {0};
    mCoreConfigMap(&core->config, &opts);
    opts.audioSync = false;
    opts.videoSync = false;
    mCoreConfigLoadDefaults(&core->config, &opts);
    mCoreLoadConfig(core);
    core->reset(core);

    struct GBA* gba = core->board;
    if (bios_path && !gba->biosVf) {
        fprintf(stderr, "mGBA did not accept the BIOS image %s\n", bios_path);
        return 1;
    }

    static struct FrameCapture capture;
    capture.source = vbuf;
    struct mCoreCallbacks callbacks = { .context = &capture, .videoFrameEnded = _captureFrame };
    core->addCoreCallbacks(core, &callbacks);

    struct mTimingEvent stop_event = {
        .context = gba, .callback = _stopAtTarget, .name = "emu-bench frame target", .priority = 0xFF,
    };
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (uint64_t frame = 1; frame <= frames; ++frame) {
        const uint64_t target = frame * CYCLES_PER_FRAME;
        capture.armed = frame == frames;
        const uint64_t now = mTimingGlobalTime(core->timing);
        if (now < target) {
            mTimingDeschedule(core->timing, &stop_event);
            mTimingSchedule(core->timing, &stop_event, (int32_t) (target - now));
        }
        while (mTimingGlobalTime(core->timing) < target) {
            core->runLoop(core);
        }
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    if (capture.armed) {
        fprintf(stderr, "mGBA ended the run without drawing its final frame\n");
        return 1;
    }

    const double seconds = (double) (t1.tv_sec - t0.tv_sec) + (double) (t1.tv_nsec - t0.tv_nsec) / 1e9;
    if (!(seconds > 0.0)) {
        fprintf(stderr, "benchmark timer did not advance\n");
        return 1;
    }
    printf("benchmark: {\"schema\":1,\"emulator\":\"mgba\",\"frames\":%u,\"seconds\":%.9f,\"cycle\":\"%llu\",\"boot\":\"%s\",\"video\":\"full\",\"audio\":\"exact\",\"framebufferHash\":\"%016llx\",\"settings\":{\"idleOptimization\":\"detect\"}}\n",
           frames, seconds, (unsigned long long) mTimingGlobalTime(core->timing), bios_path ? "skip-bios" : "hle",
           (unsigned long long) _framebufferHash(capture.pixels));
    const int dump_status = dump_frame_path ? _writePpm(dump_frame_path, capture.pixels) : 0;
    core->deinit(core);
    return dump_status;
}
