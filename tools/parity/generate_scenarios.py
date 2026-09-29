#!/usr/bin/env python3
"""Regenerate manifests from ZiGBA validator cases and a successful HLE run.

Usage: python3 tools/parity/generate_scenarios.py target/parity/baseline-hle-direct.log
       python3 tools/parity/generate_scenarios.py --bios
Validator cycle counts choose *duration*, not expected state bytes. The
second form derives external/skip-BIOS variants from the existing HLE manifests.
The committed manifests are consumed directly; regeneration is not required to run.
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "refs/zigba/test"
OUT = ROOT / "tools/parity/scenarios"
FRAME = 280896
BIOS = "../../../refs/cult-gba-bios/bios.bin"
ODD_SLICES = {
    "probe-halt_stop": "97",
    "probe-keypad_irq": "1232",
    "probe-psg_square": "1232",
    "bench-audio": "1232",
}
EXCLUDED = {
    # These require two or more linked machines on one SIO timeline.
    "link_child", "link_parent", "multi4_child1", "multi4_child2",
    "multi4_child3", "multi4_parent", "multi_parent", "stop_sio_child",
    "uart_rx", "uart_tx", "multiboot_2slave_parent",
    "multiboot_2slave_child", "multiboot_2mhz_parent", "multiboot_normal_child",
}


def manifest(name, rom, observed, *, config=None, inputs=None, host_actions=None, terminal_pc=None, report=None):
    # Two frames after the validator's terminal cycle allow a final steady
    # framebuffer. Continuous benchmark ROMs run for twelve frames instead.
    end_frames = max(4, (observed + 3 * FRAME - 1) // FRAME)
    end = end_frames * FRAME
    checkpoints = [frame * FRAME for frame in range(4, end_frames + 1, 4)]
    if not checkpoints or checkpoints[-1] != end:
        checkpoints.append(end)
    result = {
        "name": name, "assets": {"rom": "../../../" + rom},
        "config": {"boot": "hle", "saveType": "auto", **(config or {})},
        "inputs": inputs or [], "endCycle": str(end),
        "checkpoints": [str(n) for n in checkpoints],
        "sliceCycles": ["70224", "280896"] + ([ODD_SLICES[name]] if name in ODD_SLICES else []),
    }
    if host_actions:
        result["hostActions"] = host_actions
    if terminal_pc is not None:
        result["terminalPc"] = terminal_pc
    if report is not None:
        result["terminalR12"] = report
    (OUT / (name + ".json")).write_text(json.dumps(result, indent=2) + "\n")


def bios_variants():
    # The pinned replacement BIOS runs its startup screen before handing off
    # to the cartridge. The external validator reaches ordinary ROM endpoints
    # about 64 frames later than HLE; retain checkpoints during that boot.
    lead = 64 * FRAME
    for path in sorted(OUT.glob("jsmolka-*.json")):
        if path.stem.endswith("-external_bios"):
            continue
        result = json.loads(path.read_text())
        result["name"] += "-external_bios"
        result["assets"]["bios"] = BIOS
        result["config"]["boot"] = "external_bios"
        result["endCycle"] = str(int(result["endCycle"]) + lead)
        result["checkpoints"] = [str(n * FRAME) for n in (4, 16, 32, 48, 64)] + [
            str(int(n) + lead) for n in result["checkpoints"]
        ]
        (OUT / (result["name"] + ".json")).write_text(json.dumps(result, indent=2) + "\n")

    for path in sorted(OUT.glob("bench-*.json")):
        if path.stem.endswith("-skip_bios"):
            continue
        result = json.loads(path.read_text())
        result["name"] += "-skip_bios"
        result["assets"]["bios"] = BIOS
        result["config"]["boot"] = "skip_bios"
        if path.stem == "bench-idle":
            result["sliceCycles"].append("1232")
        (OUT / (result["name"] + ".json")).write_text(json.dumps(result, indent=2) + "\n")


def main(log):
    text = Path(log).read_text()
    if "71 passed, 0 failed" not in text:
        raise ValueError("supply a complete passing HLE validator log")
    durations = {}
    reports = {}
    for case, detail in re.findall(r"^pass  (\S+)\s+(.+)$", text, re.M):
        cycle = re.search(r"cycles=(\d+)", detail)
        sample = re.search(r"samples=(\d+)", detail)
        durations[case] = int(cycle[1]) if cycle else (int(sample[1]) * 512 if sample else 3 * FRAME)
        r12 = re.search(r"r12=([0-9a-f]{8})\b", detail)
        if r12:
            reports[case] = int(r12[1], 16)

    jsmolka = (SOURCE / "jsmolka_cases.zig").read_text()
    idles = {}
    saves = {}
    for name, block in re.findall(r'report\("([^\"]+)",\s*\.\{(.*?)\}\)', jsmolka, re.S):
        idle = re.search(r"\.idle\s*=\s*(0x[0-9A-Fa-f_]+)", block)
        if idle:
            idles[name] = int(idle[1].replace("_", ""), 16)
        save = re.search(r"\.save_type\s*=\s*\.(\w+)", block)
        if save:
            saves[name] = save[1].replace("_", "")
    for rom in sorted((ROOT / "refs/gba-tests").glob("*/*.gba")):
        case = "save-" + rom.stem if rom.parent.name == "save" else (
            "ppu-" + rom.stem if rom.parent.name == "ppu" else rom.stem
        )
        name = "jsmolka-" + case
        assert name in durations, name
        save_type = saves.get(case)
        if case == "unsafe": save_type = "sram"
        if case == "nes": save_type = "none"
        manifest(name, rom.relative_to(ROOT).as_posix(), durations[name],
                 config={"saveType": save_type} if save_type else None,
                 terminal_pc=idles.get(case))

    probes = (SOURCE / "probe_cases.zig").read_text()
    mapped = {}
    for case, rom in re.findall(r'(?:reports|frame)\("([^\"]+)",\s*"([^\"]+)"', probes):
        mapped[rom] = case
    for case, rom in re.findall(r'custom\("([^\"]+)",\s*&\.\{"([^\"]+)"\}', probes):
        mapped[rom] = case
    seen = set()
    for rom in sorted((ROOT / "probe/build").glob("*.gba")):
        if rom.stem in EXCLUDED:
            continue
        case = mapped.get(rom.stem)
        if case is None:
            raise ValueError(f"unmapped probe {rom.stem}")
        seen.add(rom.stem)
        name = "probe-" + rom.stem
        config = {}
        if rom.stem in ("rtc_datetime", "rtc_write"):
            config["rtcEpoch"] = 981205445
        if rom.stem == "gyro_sample": config["gyroSample"] = 0x6C0
        if rom.stem == "solar_count": config["solarLevel"] = 252
        if rom.stem == "tilt_sample": config.update(saveType="none", tilt=[0x392, 0x3A0])
        if rom.stem == "rumble_flag": config["rumble"] = True
        inputs = []
        if rom.stem == "keypad_irq":
            # Cycle-scheduled approximation of validator's 2000-instruction
            # held/released pulses; terminal r12 confirms all three IRQ edges.
            inputs = [{"at": str(n * 24000), "change": {"keys": 8 if n == 7 else (1 if n % 2 else 0)}} for n in range(1, 8)]
        if rom.stem == "halt_stop":
            inputs = [{"at": "700", "change": {"keys": 1}}]
        host_actions = []
        if rom.stem == "joybus_slave":
            host_actions = [{"at": "20000", "command": {"joybus_write": 0x44332211}}]
        if rom.stem == "joybus_tx":
            host_actions = [{"at": "20000", "command": {"joybus_read": True}}]
        manifest(name, rom.relative_to(ROOT).as_posix(), durations["probe-" + case],
                 config=config, inputs=inputs, host_actions=host_actions, terminal_pc=0x08000304,
                 report=reports.get("probe-" + case))
    assert len(seen) == len(list((ROOT / "probe/build").glob("*.gba"))) - len(EXCLUDED)

    for rom in sorted((ROOT / "tools/bench/roms/build").glob("*.gba")):
        # These are deliberately continuous workloads, not terminal tests.
        manifest("bench-" + rom.stem, rom.relative_to(ROOT).as_posix(), 10 * FRAME)
    bios_variants()


if __name__ == "__main__":
    if sys.argv[1:] == ["--bios"]:
        bios_variants()
    else:
        main(sys.argv[1])
