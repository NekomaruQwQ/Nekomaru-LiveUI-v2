set shell := ["nu", "-c"]

alias i := install

list:
    just --list
refresh:
    curl -X POST $"http://localhost:($env.LIVE_PORT)/api/v1/refresh"

server:
    use .mod.nu run; \
    run live-capture app --help
    cd server; bun --hot index.ts;

app *args:
    use .mod.nu run; \
    run live-app app -x 1280 -y 720 {{args}}
youtube-music *args:
    use .mod.nu run; \
    run live-app youtube-music \
        "https://music.youtube.com/" \
        -t "YouTube Music" \
        -x 1280 -y 720 -s 2 \
        {{args}}

install: install-frontend install-server
install-frontend:
    cd frontend; bun i;
install-server:
    cd server; bun i;
