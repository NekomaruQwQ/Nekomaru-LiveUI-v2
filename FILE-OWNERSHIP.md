# FILE-OWNERSHIP: Per-file ownership tracking

**agent** = Claude manages.
**human** = Nekomaru hand-crafts.

```bash
.gitignore                                  human
.mod.nu                                     human

# docs/
ARCHIVE-0-Prototype.md                      agent
ARCHIVE-1-StreamID.md                       agent
PLAN-UI-AudioMeter.md                       agent
PLAN-UI-KPMMeter.md                         agent
README.md                                   agent
README-Audio.md                             agent
FILE-OWNERSHIP.md                           human

# core/live-app/
src/main.rs                                 human

# core/live-capture/
Cargo.toml                                  human
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

# core/live-audio/
src/lib.rs                                  agent
src/main.rs                                 agent

# crates/enumerate-windows/
src/lib.rs                                  human

# server/
api.ts                                      agent
audio.ts                                    agent
audio-api.ts                                agent
audio-buffer.ts                             agent
audio-protocol.ts                           agent
buffer.ts                                   agent
common.ts                                   agent
index.ts                                    agent
log.ts                                      agent
persist.ts                                  agent
process.ts                                  agent
protocol.ts                                 agent
selector.ts                                 agent
strings.ts                                  agent
youtube-music.ts                            agent

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

# config files
biome.json                                  human
Cargo.toml                                  human
core/live-app/Cargo.toml                    human
core/live-audio/Cargo.toml                  human
core/live-capture/Cargo.toml                human
crates/enumerate-windows/Cargo.toml         human
frontend/package.json                       human
frontend/tsconfig.json                      human
frontend/vite.config.ts                     human
server/package.json                         human
server/tsconfig.json                        human
```