# Security policy

## Reporting a vulnerability

Report vulnerabilities privately through [GitHub security advisories](https://github.com/evenfall-emu/evenfall/security/advisories/new).
If you can't use GitHub, email [security@evenfall-emu.org](mailto:security@evenfall-emu.org).
Don't open a public issue.
Include the affected revision, the input that triggers the problem (a minimal crafted file rather than a commercial ROM), and its impact.
You should get an acknowledgement within a week.

## Scope

Evenfall treats these inputs as untrusted, and should reject them safely, without memory corruption, unbounded allocation or hangs outside emulated time:

- ROM images, ZIP archives and BIOS images,
- battery saves, save states and rewind bundles,
- scenario manifests, failure bundles and input movies read by the tools.

The core is written in Rust and denies `unsafe` code except in modules with a measured reason.
A panic or abort reachable from a crafted input is still a bug worth reporting, and so is a state that loads but makes the host hang.

Out of scope:

- The JavaScript SDK is an API for trusted application code.
  It doesn't sandbox untrusted JavaScript or untrusted game bridges.
- Guest software running inside the emulated GBA is expected to be arbitrary.
  Emulated behavior is only a security issue when it escapes the emulator's memory or resource bounds.
