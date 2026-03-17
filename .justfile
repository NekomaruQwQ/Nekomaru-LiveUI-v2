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

server *args:
    cargo build
    cargo run -p live-server -- {{args}}
app *args:
    use .mod.nu run-app; \
    run-app app \
        -x 1280 -y 720 $"{{base_url}}" {{args}}
youtube-music *args:
    use .mod.nu run-app; \
    run-app youtube-music \
        -x 1280 -y 720 -s 2 -t "YouTube Music" "https://music.youtube.com/" {{args}}
