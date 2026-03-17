# FILE-OWNERSHIP: Per-file ownership tracking

**agent** = Claude manages.
**human** = Nekomaru hand-crafts.

```bash
# config files at repo root
.gitignore                                  human
.justfile                                   human
.mod.nu                                     human
biome.json                                  human
Cargo.toml                                  human
FILE-OWNERSHIP.md                           human

# config files
live-app/Cargo.toml                         human
live-video/Cargo.toml                       human
live-audio/Cargo.toml                       human
live-kpm/Cargo.toml                         human
live-server/Cargo.toml                      human
crates/enumerate-windows/Cargo.toml         human
crates/set-dpi-awareness/Cargo.toml         human
crates/job-object/Cargo.toml                human
frontend/package.json                       human
frontend/tsconfig.json                      human
frontend/vite.config.ts                     human

# docs/
ARCHIVE-0-Prototype.md                      agent
ARCHIVE-1-StreamID.md                       agent
PLAN-UI-AudioMeter.md                       agent
PLAN-UI-KPMMeter.md                         agent
README.md                                   agent
README-Audio.md                             agent

# live-app/
src/main.rs                                 human

# live-video/
src/capture.rs                              agent
src/converter.rs                            agent
src/d3d11.rs                                agent
src/encoder.rs                              agent
src/encoder/debug.rs                        agent
src/encoder/helper.rs                       agent
src/lib.rs                                  agent
src/main.rs                                 agent
src/resample.hlsl                           human
src/resample.rs                             human

# live-audio/
src/lib.rs                                  agent
src/main.rs                                 agent

# live-kpm/
src/lib.rs                                  agent
src/main.rs                                 agent

# live-server/
src/main.rs                                 agent
src/state.rs                                agent
src/windows.rs                              agent
src/video/buffer.rs                         agent
src/video/process.rs                        agent
src/video/routes.rs                         agent
src/audio/buffer.rs                         agent
src/audio/process.rs                        agent
src/audio/routes.rs                         agent
src/kpm/calculator.rs                       agent
src/kpm/process.rs                          agent
src/kpm/routes.rs                           agent
src/selector/config.rs                      agent
src/selector/manager.rs                     agent
src/selector/routes.rs                      agent
src/strings/store.rs                        agent
src/strings/routes.rs                       agent
src/ytm/manager.rs                          agent

# crates/enumerate-windows/
src/lib.rs                                  human

# crates/set-dpi-awareness/
src/lib.rs                                  human

# crates/job-object/
src/lib.rs                                  human

# frontend/src/stream/
chroma-key.ts                               agent
decoder.ts                                  agent
index.tsx                                   agent

# frontend/src/
api.ts                                      agent
streams.ts                                  agent
strings.ts                                  agent
strings-api.ts                              agent
app.tsx                                     human

# frontend/src/components/
grid.tsx                                    human
marquee.tsx                                 agent

# frontend/src/audio/
index.tsx                                   agent
worklet.ts                                  agent

# frontend/src/kpm/
index.tsx                                   agent

# frontend/src/widgets/
common.tsx                                  agent
index.tsx                                   agent

# frontend/
debug.ts                                    human
global.css                                  human
global.effects.css                          human
global.tailwind.css                         human
index.html                                  human
index.tsx                                   human
```
