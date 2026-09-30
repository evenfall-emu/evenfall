// VBA-M adapter for tools/bench/run.mjs, set up the way Delta's
// GBADeltaCore bridge sets it up. The harness contract is shared by every
// adapter in this directory (see ../README.md):
//
// usage: vbam-harness ROM FRAMES [--bios=PATH] [--dump-frame=PATH]
//
// Without --bios VBA-M runs its HLE BIOS, which is what Delta ships ("Delta
// doesn't use BIOS files"); with --bios it maps that image and skips the boot
// animation. Like GBADeltaCore it loads the ROM with CPULoadRomData, enables
// the RTC, renders 32-bit pixels with Delta's colour shifts and frame skip 0,
// and produces sound at 32768 Hz into a driver; here the driver discards the
// samples instead of queueing them for playback. No input, no save media.
//
// VBA-M has no global cycle counter. CPULoop(ticks) stops at the first event
// boundary at or after `ticks`, with every elapsed cycle accounted to the LCD
// counters, so the harness derives guest time from the VBlank count and the
// LCD position and aims each frame at its absolute boundary.
//
// Links pinned VBA-M (GPL-2.0) and the SFML subset GBADeltaCore bundles
// (zlib) into a local binary that is never distributed; no source from
// either is copied here.
#include "System.h"
#include "Util.h"
#include "common/ConfigManager.h"
#include "common/SoundDriver.h"
#include "gba/GBA.h"
#include "gba/Globals.h"
#include "gba/RTC.h"
#include "gba/Sound.h"

#include <cstdarg>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <fstream>
#include <iterator>
#include <vector>

// Globals VBA-M expects its frontend to define, with GBADeltaCore's values.
int systemRedShift = 19;
int systemGreenShift = 11;
int systemBlueShift = 3;
int systemColorDepth = 32;
int systemVerbose;
int systemSaveUpdateCounter = 0;
int systemFrameSkip;
uint32_t systemColorMap32[0x10000];
uint16_t systemColorMap16[0x10000];
uint16_t systemGbPalette[24];
int emulating;
int RGB_LOW_BITS_MASK;

// Defined without a header in gba/GBA.cpp.
extern int lcdTicks;

namespace {

constexpr uint64_t kCyclesPerLine = 1232;
constexpr uint64_t kCyclesPerFrame = 228 * kCyclesPerLine;

uint64_t vblanks = 0;
uint64_t draws = 0;
uint64_t capture_draw = 0;
bool captured = false;
uint32_t pixels[240 * 160];

// Cycles since the start of the current frame's VBlank (VCOUNT 160, dot 0).
// The LCD counts 1008 cycles of HDraw, then 224 of HBlank (DISPSTAT bit 1).
uint64_t FramePosition() {
    const uint64_t line = (uint64_t(VCOUNT) + 228 - 160) % 228;
    const uint64_t dot = (DISPSTAT & 2) ? 1008 + (224 - lcdTicks) : 1008 - lcdTicks;
    return line * kCyclesPerLine + dot;
}

uint64_t GuestTime(uint64_t origin) {
    return vblanks * kCyclesPerFrame + FramePosition() - origin;
}

uint16_t Rgb555(uint32_t p) {
    return uint16_t(((p >> 19) & 31) | (((p >> 11) & 31) << 5) | (((p >> 3) & 31) << 10));
}

// FNV-1a 64 over visible RGB555 pixels, low byte then high byte.
uint64_t FramebufferHash() {
    uint64_t hash = 14695981039346656037ull;
    for (uint32_t p : pixels) {
        const uint16_t rgb555 = Rgb555(p);
        hash = (hash ^ uint8_t(rgb555)) * 1099511628211ull;
        hash = (hash ^ uint8_t(rgb555 >> 8)) * 1099511628211ull;
    }
    return hash;
}

bool WritePpm(const char* path) {
    std::FILE* out = std::fopen(path, "wb");
    if (!out) {
        std::perror("dump-frame");
        return false;
    }
    std::fprintf(out, "P6\n240 160\n255\n");
    for (uint32_t p : pixels) {
        const uint16_t rgb555 = Rgb555(p);
        for (int channel = 0; channel < 3; channel++) {
            const uint8_t value = (rgb555 >> (5 * channel)) & 31;
            std::fputc((value << 3) | (value >> 2), out);
        }
    }
    return std::fclose(out) == 0;
}

class DiscardSoundDriver final : public SoundDriver {
  public:
    bool init(long) override { return true; }
    void pause() override {}
    void reset() override {}
    void resume() override {}
    void write(uint16_t*, int) override {}
};

int Usage() {
    std::fprintf(stderr, "usage: vbam-harness ROM FRAMES [--bios=PATH] [--dump-frame=PATH]\n");
    return 2;
}

} // namespace

// Frontend callbacks. VBA-M calls systemFrame at every VBlank and
// systemDrawScreen after each frame it renders (every frame at frame skip 0).
void systemFrame() { vblanks++; }

void systemDrawScreen() {
    if (++draws != capture_draw) return;
    // 32-bit rows are 241 pixels wide, starting one row down (as GBADeltaCore reads them).
    for (int y = 0; y < 160; y++) {
        std::memcpy(pixels + y * 240, pix + (y + 1) * 241 * 4, 240 * 4);
    }
    captured = true;
}

SoundDriver* systemSoundInit() {
    soundShutdown();
    return new DiscardSoundDriver;
}

void log(const char*, ...) {}
void systemMessage(int, const char*, ...) {}
bool systemReadJoypads() { return true; }
uint32_t systemReadJoypad(int) { return 0; }
void systemShowSpeed(int) {}
void system10Frames(int) {}
void systemSetTitle(const char*) {}
void systemScreenCapture(int) {}
uint32_t systemGetClock() {
    timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return uint32_t(now.tv_sec * 1000 + now.tv_nsec / 1000000);
}
void systemUpdateMotionSensor() {}
uint8_t systemGetSensorDarkness() { return 0xE8; }
int systemGetSensorX() { return 0; }
int systemGetSensorY() { return 0; }
int systemGetSensorZ() { return 0; }
void systemCartridgeRumble(bool) {}
void systemPossibleCartridgeRumble(bool) {}
void systemGbPrint(uint8_t*, int, int, int, int, int) {}
void systemScreenMessage(const char*) {}
bool systemCanChangeSoundQuality() { return false; }
bool systemPauseOnFrame() { return false; }
void systemGbBorderOn() {}
void systemOnSoundShutdown() {}
void systemOnWriteDataToSoundBuffer(const uint16_t*, int) {}

int main(int argc, char** argv) {
    const char* bios_path = nullptr;
    const char* dump_frame_path = nullptr;
    const char* positional[2];
    int positional_count = 0;
    for (int i = 1; i < argc; i++) {
        if (!std::strncmp(argv[i], "--bios=", 7) && argv[i][7] && !bios_path) {
            bios_path = argv[i] + 7;
        } else if (!std::strncmp(argv[i], "--dump-frame=", 13) && argv[i][13] && !dump_frame_path) {
            dump_frame_path = argv[i] + 13;
        } else if (std::strncmp(argv[i], "--", 2) && positional_count < 2) {
            positional[positional_count++] = argv[i];
        } else {
            return Usage();
        }
    }
    if (positional_count != 2) return Usage();
    char* end = nullptr;
    const unsigned long long frames = std::strtoull(positional[1], &end, 10);
    if (!positional[1][0] || *end || frames == 0 || frames > 1000000) return Usage();

    std::ifstream in{positional[0], std::ios::binary};
    std::vector<char> rom{std::istreambuf_iterator<char>{in}, std::istreambuf_iterator<char>{}};
    if (rom.size() < 0xC0 || rom.size() > 0x02000000 || !CPULoadRomData(rom.data(), int(rom.size()))) {
        std::fprintf(stderr, "vbam-harness: cannot load a GBA ROM from %s\n", positional[0]);
        return 1;
    }
    // At this revision CPULoadRomData allocates 240x160 pixels, but the
    // 32-bit renderer writes 241-pixel rows from the second row on, 1600
    // bytes past the end every frame. Use the size CPULoadRom allocates, so
    // the overrun cannot corrupt other emulator state here.
    std::free(pix);
    pix = static_cast<uint8_t*>(std::calloc(1, 4 * 241 * 162));
    utilUpdateSystemColorMaps(false);
    utilGBAFindSave(int(rom.size()));
    rtcEnable(true);
    soundInit();
    soundSetSampleRate(32768);
    soundReset();
    skipBios = 1;
    CPUInit(bios_path ? bios_path : "", bios_path != nullptr);
    if (bios_path && !useBios) {
        std::fprintf(stderr, "vbam-harness: VBA-M did not accept the BIOS image %s\n", bios_path);
        return 1;
    }
    GBASystem.emuReset();
    emulating = 1;

    capture_draw = frames;
    const uint64_t origin = FramePosition();
    const unsigned reset_vcount = VCOUNT;

    timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (uint64_t frame = 1; frame <= frames; frame++) {
        const uint64_t target = frame * kCyclesPerFrame;
        uint64_t now;
        while ((now = GuestTime(origin)) < target) {
            GBASystem.emuMain(int(target - now));
        }
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    const double seconds = double(t1.tv_sec - t0.tv_sec) + double(t1.tv_nsec - t0.tv_nsec) / 1e9;
    if (!(seconds > 0.0)) {
        std::fprintf(stderr, "benchmark timer did not advance\n");
        return 1;
    }
    if (!captured) {
        std::fprintf(stderr, "vbam-harness: expected draw %llu, got %llu\n", frames, (unsigned long long)draws);
        return 1;
    }

    std::printf(
        "benchmark: {\"schema\":1,\"emulator\":\"vbam\",\"frames\":%llu,\"seconds\":%.9f,\"cycle\":\"%llu\",\"boot\":\"%s\",\"video\":\"full\",\"audio\":\"exact\",\"framebufferHash\":\"%016llx\",\"settings\":{\"setup\":\"GBADeltaCore\",\"colorDepth\":32,\"frameSkip\":0,\"sampleRate\":32768,\"rtc\":true,\"resetVcount\":%u}}\n",
        frames, seconds, (unsigned long long)GuestTime(origin), bios_path ? "skip-bios" : "hle",
        (unsigned long long)FramebufferHash(), reset_vcount);
    return dump_frame_path && !WritePpm(dump_frame_path) ? 1 : 0;
}
