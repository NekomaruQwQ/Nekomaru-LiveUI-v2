# Build live-app, copy it as live-app.<id>.exe, and execute the copy.
# Separate instance IDs allow multiple live-app processes (e.g. frontend
# and youtube-music) to run simultaneously without file lock conflicts.
export def --wrapped run-app [id: string, ...args]: nothing -> nothing {
    cargo build -p live-app;
    cp -f $"target/debug/live-app.exe" $"target/debug/live-app.($id).exe";
    ^$"target/debug/live-app.($id).exe" ...$args
}
