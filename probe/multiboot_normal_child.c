/* Probe 62b: multiboot transfer in Normal 32-bit mode, child side.
 *
 * Enters 32-bit Normal slave mode (external clock, START set) with
 * SIODATA32 preloaded to 0x73420000, then answers the parent's SWI 25h
 * transfer word by word: 0x73rr to the length word, 0xC0+4i echoes to the
 * 64 data words (each verified afterwards against a C replica of the
 * Normal-mode multiboot encoder over payload word[i] =
 * (i+1)*0x11111111), 0x75 to the 0x65/0x66 handshake, and the computed CRC
 * back to the CRC word with the answer latched one transfer early. Echoes
 * ride in the high half, which is all the master reads. The loop only
 * snapshots words and latches responses; verification runs untimed after
 * the exchange because a 2 MHz transfer is shorter than encoder
 * verification. Reports data mismatches; the validator expects zero
 * (the master proves handshake sync and CRC equality via its own result).
 * See multiboot_2mhz_parent.c. HLE boot only.
 */

#include "gba.h"

#define REG_SIOCNT 0x128
#define REG_SIODATA32 ((volatile uint32_t *)0x04000120)
#define REG_RCNT 0x134

#define SIO_START 0x0080u
#define SIO_NORMAL32 (0x1000u | 0x0008u)

#define RANDOM_BYTE 0x42u
void report_and_idle(uint32_t value);

static uint32_t checksum_state;
static uint32_t key_state;
/* Incoming data snapshot: at 2 MHz a transfer (256 cycles) is shorter
 * than encoder verification, so the time-critical loop only snapshots
 * words and latches positional responses; verification runs untimed
 * after the exchange. */
static uint32_t seen_words[64];

static uint32_t checksum_word(uint32_t initial) {
    uint32_t checksum = initial;
    for (uint32_t i = 0; i < 32u; i++) {
        uint32_t carry = checksum & 1u;
        checksum >>= 1;
        if (carry) {
            checksum ^= 0xC37Bu;
        }
    }
    return checksum;
}

static uint32_t encode_word(uint32_t plain, uint32_t file_offset) {
    checksum_state = checksum_word(checksum_state ^ plain);
    key_state = key_state * 0x6F646573u + 1u;
    return plain ^ (0u - 0x02000000u - file_offset) ^ key_state ^ 0x43202F2Fu;
}

static void await_transfer(void) {
    while (MEM_IO[REG_SIOCNT / 2] & SIO_START) {
    }
}

int main(void) {
    MEM_IO[REG_RCNT / 2] = 0u;
    /* Enter Normal-32 mode before touching SIODATA32: data writes are
     * gated on the submode, so preloading first would vanish. */
    MEM_IO[REG_SIOCNT / 2] = (uint16_t)SIO_NORMAL32;
    *REG_SIODATA32 = (uint32_t)(0x7300u | RANDOM_BYTE) << 16;
    MEM_IO[REG_SIOCNT / 2] = (uint16_t)(SIO_NORMAL32 | SIO_START);

    uint32_t mismatches = 0;

    checksum_state = 0xC387u;
    key_state = 0x55u | (0x22u << 8) | (0x33u << 16) | (0x44u << 24);

    /* Precompute the CRC before the time-critical loop; verification always
     * runs after the send word is already latched. */
    for (uint32_t i = 0; i < 64u; i++) {
        encode_word((i + 1u) * 0x11111111u, 0xC0u + i * 4u);
    }
    uint32_t final_word = 0x11u | (RANDOM_BYTE << 8) | (0xFFu << 16) | (0xFFu << 24);
    uint16_t precomputed_crc = (uint16_t)checksum_word(checksum_state ^ final_word);

    checksum_state = 0xC387u;
    key_state = 0x55u | (0x22u << 8) | (0x33u << 16) | (0x44u << 24);

    await_transfer();
    *REG_SIODATA32 = 0x00C0u << 16;
    MEM_IO[REG_SIOCNT / 2] = (uint16_t)(SIO_NORMAL32 | SIO_START);

    for (uint32_t i = 0; i < 64u; i++) {
        await_transfer();
        seen_words[i] = *REG_SIODATA32;
        if (i < 63u) {
            *REG_SIODATA32 = (0x00C0u + 4u * (i + 1u)) << 16;
        } else {
            *REG_SIODATA32 = 0x0075u << 16;
        }
        MEM_IO[REG_SIOCNT / 2] = (uint16_t)(SIO_NORMAL32 | SIO_START);
    }

    await_transfer();
    *REG_SIODATA32 = 0x0075u << 16;
    MEM_IO[REG_SIOCNT / 2] = (uint16_t)(SIO_NORMAL32 | SIO_START);

    await_transfer();
    *REG_SIODATA32 = (uint32_t)precomputed_crc << 16;
    MEM_IO[REG_SIOCNT / 2] = (uint16_t)(SIO_NORMAL32 | SIO_START);

    await_transfer();
    /* The master checks CRC equality itself (its r0 proves the round-trip). */

    checksum_state = 0xC387u;
    key_state = 0x55u | (0x22u << 8) | (0x33u << 16) | (0x44u << 24);
    for (uint32_t i = 0; i < 64u; i++) {
        uint32_t plain = (i + 1u) * 0x11111111u;
        if (seen_words[i] != encode_word(plain, 0xC0u + i * 4u)) {
            mismatches++;
        }
    }

    report_and_idle(mismatches);
    return 0;
}
