/* Probe 65b: multiboot transfer with two slaves, child side. Each
 * of the two slaves runs this ROM; both answer 0x42 as their random, so
 * the CRC finish mixes randoms {0x42, 0x42, 0xFF} exactly as the parent
 * collects them with client_bit 0x06.
 *
 * Enters multiplayer slave mode at baud 3 with SIOMLT_SEND preloaded to
 * 0x7342, then answers the parent's SWI 25h transfer with the agreed
 * parameters (client_bit 0x02, palette 0x55, clients 0x22/0x33/0x44,
 * handshake 0x11, random 0x42): 0x73rr to the length word, 0xC0+2j echoes
 * to the 128 data halves (each verified against a C replica of the
 * multiboot encoder over payload word[i] = (i+1)*0x11111111), 0x75 to the
 * 0x65/0x66 CRC handshake, and the computed CRC back to the CRC word.
 * Reports the mismatch count at idle; the validator expects zero over the
 * full 132-transfer exchange (length, 128 data halves, 0x65, 0x66, CRC).
 * See multiboot_2slave_parent.c. HLE boot only.
 */

#include "gba.h"

#define REG_SIOCNT 0x128
#define REG_SIOMLT_SEND 0x12A
#define REG_SIOMULTI0 0x120
#define REG_RCNT 0x134

#define SIO_START 0x0080u
#define SIO_MULTIPLAYER 0x2000u
#define SIO_BAUD3 0x0003u

#define RANDOM_BYTE 0x42u
#define EXPECTED_LENGTH 0x003Cu

void report_and_idle(uint32_t value);

static uint32_t checksum_state;
static uint32_t key_state;

static uint32_t checksum_word(uint32_t initial) {
    uint32_t checksum = initial;
    for (uint32_t i = 0; i < 32u; i++) {
        uint32_t carry = checksum & 1u;
        checksum >>= 1;
        if (carry) {
            checksum ^= 0xA517u;
        }
    }
    return checksum;
}

static uint32_t encode_word(uint32_t plain, uint32_t file_offset) {
    checksum_state = checksum_word(checksum_state ^ plain);
    key_state = key_state * 0x6F646573u + 1u;
    return plain ^ (0u - 0x02000000u - file_offset) ^ key_state ^ 0x6465646Fu;
}

static void await_transfer(void) {
    while (!(MEM_IO[REG_SIOCNT / 2] & SIO_START)) {
    }
    while (MEM_IO[REG_SIOCNT / 2] & SIO_START) {
    }
}

int main(void) {
    MEM_IO[REG_RCNT / 2] = 0u;
    MEM_IO[REG_SIOMLT_SEND / 2] = (uint16_t)(0x7300u | RANDOM_BYTE);
    MEM_IO[REG_SIOCNT / 2] = (uint16_t)(SIO_MULTIPLAYER | SIO_BAUD3);

    uint32_t mismatches = 0;

    checksum_state = 0xFFF8u;
    key_state = 0x55u | (0x22u << 8) | (0x33u << 16) | (0x44u << 24);

    /* Precompute the CRC before the time-critical loop: the master leaves
     * only ~600 cycles between transfers, so every response below is a
     * positional immediate and verification always runs after the send
     * word is already latched. */
    for (uint32_t i = 0; i < 64u; i++) {
        encode_word((i + 1u) * 0x11111111u, 0xC0u + i * 4u);
    }
    uint16_t precomputed_crc =
        (uint16_t)checksum_word(checksum_state ^ (0x11u | (RANDOM_BYTE << 8) | (RANDOM_BYTE << 16) | 0xFF000000u));

    checksum_state = 0xFFF8u;
    key_state = 0x55u | (0x22u << 8) | (0x33u << 16) | (0x44u << 24);

    await_transfer();
    if (MEM_IO[REG_SIOMULTI0 / 2] != EXPECTED_LENGTH) {
        mismatches++;
    }
    MEM_IO[REG_SIOMLT_SEND / 2] = 0x00C0u;

    uint32_t encoded = 0;
    for (uint32_t j = 0; j < 128u; j++) {
        await_transfer();
        /* Snapshot the word first: the next launch re-clears the receive
         * registers, so no slow work may come before this read. */
        uint16_t incoming = MEM_IO[REG_SIOMULTI0 / 2];
        if (j < 127u) {
            MEM_IO[REG_SIOMLT_SEND / 2] = (uint16_t)(0x00C0u + 2u * (j + 1u));
        } else {
            MEM_IO[REG_SIOMLT_SEND / 2] = 0x0075u;
        }
        /* The master encodes once per word and ships halves across two
         * transfers; encode on even halves only to stay in stream. */
        if (!(j & 1u)) {
            uint32_t word = j / 2u;
            uint32_t plain = (word + 1u) * 0x11111111u;
            encoded = encode_word(plain, 0xC0u + word * 4u);
        }
        uint32_t expected = (j & 1u) ? (encoded >> 16) : (encoded & 0xFFFFu);
        if (incoming != (uint16_t)expected) {
            mismatches++;
        }
    }

    /* Exactly one 0x65 then one 0x66: the 0x75 answers are positional
     * immediates latched before each launch, so the master never repeats. */
    await_transfer();
    if (MEM_IO[REG_SIOMULTI0 / 2] != 0x0065u) {
        mismatches++;
    }
    MEM_IO[REG_SIOMLT_SEND / 2] = 0x0075u;

    await_transfer();
    if (MEM_IO[REG_SIOMULTI0 / 2] != 0x0066u) {
        mismatches++;
    }
    /* Latch the CRC answer now: the master snapshots it when it launches
     * the CRC-word transfer, so writing after receiving would be too late
     * (the stale 0x75 would fail the master's equality check). */
    MEM_IO[REG_SIOMLT_SEND / 2] = precomputed_crc;

    await_transfer();
    MEM_IO[REG_SIOMLT_SEND / 2] = precomputed_crc;
    /* The master checks CRC equality itself (its r0 proves the round-trip),
     * so the slave only answers here. */

    report_and_idle(mismatches);
    return 0;
}
