# Build live-app, copy it as live-app.<id>.exe, and execute the copy.
# Separate instance IDs allow multiple live-app processes (e.g. frontend
# and youtube-music) to run simultaneously without file lock conflicts.
export def --wrapped run-app [id: string, ...args]: nothing -> nothing {
    cargo build --release -p live-app;
    cp -f $"target/release/live-app.exe" $"target/release/live-app.($id).exe";
    ^$"target/release/live-app.($id).exe" ...$args
}
