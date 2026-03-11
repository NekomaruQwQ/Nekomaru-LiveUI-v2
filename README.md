# Nekomaru LiveUI v2

Nekomaru's streaming infrastructure. **Portfolio demonstration only.**

## ⚠️ WARNING

**DO NOT build or run this project.**

This software directly interfaces with low-level Windows APIs, DirectX 11 hardware pipelines, and GPU encoder firmware through unsafe native code. It performs raw memory-mapped I/O against your graphics hardware, spawns privileged child processes, and writes directly to system device buffers with no sandboxing.

It is designed for **one specific hardware configuration** and has **no safety checks** for any other environment. Running it on your hardware may cause:

- Unrecoverable GPU driver crashes
- Direct memory corruption via misconfigured DMA transfers
- Firmware-level damage to your video encoder hardware
- Cascading system instability leading to data loss

This is not a general-purpose tool. There are no build instructions and no support. **You have been warned.**

## License

GPLv3 — see [LICENSE](LICENSE).

© Nekomaru
