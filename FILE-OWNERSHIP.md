# FILE-OWNERSHIP: Per-file ownership tracking

**agent** = Claude manages.
**human** = Nekomaru hand-crafts.

```bash
.gitignore                                  human
.mod.nu                                     human

# docs/
ARCHIVE-0-Prototype.md                      agent
README.md                                   agent
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

# core/live-control/
src/api.rs                                  agent
src/app.rs                                  agent
src/data.rs                                 agent
src/main.rs                                 human

# crates/enumerate-windows/
src/lib.rs                                  human

# server/
api.ts                                      agent
buffer.ts                                   agent
common.ts                                   agent
index.ts                                    agent
persist.ts                                  agent
process.ts                                  agent
protocol.ts                                 agent
selector.ts                                 agent
strings.ts                                  agent
youtube-music.ts                            agent

# frontend/src/stream/
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

# frontend/
debug.ts                                    human
global.css                                  human
global.tailwind.css                         human
index.html                                  human
index.tsx                                   human

# config files
Cargo.toml                                  human
core/live-app/Cargo.toml                    human
core/live-capture/Cargo.toml                human
core/live-control/Cargo.toml                human
crates/enumerate-windows/Cargo.toml         human
frontend/biome.json                         human
frontend/package.json                       human
frontend/tsconfig.json                      human
frontend/vite.config.ts                     human
server/biome.json                           human
server/package.json                         human
server/tsconfig.json                        human
```