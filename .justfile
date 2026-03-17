set shell := ["nu", "-c"]

base_url := "http://localhost:($env.LIVE_PORT)"

alias i := install

list:
    just --list
install:
    cargo build;
    cd frontend; bun i;
refresh:
    http post $"{{base_url}}/api/v1/refresh" ""
capture name:
    http put $"{{base_url}}/api/v1/streams/auto/config/preset" "{{name}}"

app *args:
    cargo run -p live-app -- -x 1280 -y 720 "{{base_url}}" {{args}}
youtube-music *args:
    cargo run -p live-app -- -x 1280 -y 720 -s 2 \
        "https://music.youtube.com/" -t "YouTube Music" {{args}}
server *args:
    cargo run -p live-server -- \
        --port      $env.LIVE_CORE_PORT \
        --vite-port $env.LIVE_PORT \
        {{args}}
