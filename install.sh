cargo build --release --bin theme-set && cargo build --release --bin theme-daemon
killall theme-daemon 2> /dev/null && killall theme-set 2> /dev/null
cp ./target/release/theme-daemon ./target/release/theme-set ~/.local/bin/
setsid theme-daemon > ./output.log 2>&1 < /dev/null &
