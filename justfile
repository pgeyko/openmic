# OpenMic shortcuts. Requires: cargo, and `. ./env.sh` for Android builds.
build:
    cargo build

test:
    cargo test

run-daemon:
    ./target/debug/openmicd

doctor:
    ./target/debug/openmic doctor

tonesend:
    ./target/debug/openmic tonesend 127.0.0.1

apk:
    . ./env.sh && cd android && gradle assembleDebug

status:
    ./target/debug/openmic status

gui:
    ./target/debug/openmic-gtk
