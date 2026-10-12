# Shortcuts for OpenMic development and running

# Build all Rust crates
build:
    cargo build

# Build optimized release binaries
release:
    cargo build --release

# Run all unit and integration tests
test:
    cargo test

# Run the GTK4 GUI (starts daemon automatically)
gui:
    ./target/debug/openmic-gtk

# Run the background daemon directly
daemon:
    ./target/debug/openmicd

# Run system doctor check
doctor:
    ./target/debug/openmic doctor

# Print pairing status and QR JSON
status:
    ./target/debug/openmic status

# Run test tone through the pipeline
tonesend:
    ./target/debug/openmic tonesend 127.0.0.1

# Install desktop binaries and desktop entry to ~/.local
install:
    ./install.sh

# Uninstall desktop binaries and desktop entry
uninstall:
    ./uninstall.sh

# Build Android APK (debug)
apk:
    . ./env.sh && cd android && ./gradlew assembleDebug

# Install APK to connected phone via ADB
apk-install:
    . ./env.sh && cd android && ./gradlew assembleDebug && adb install -r app/build/outputs/apk/debug/app-debug.apk
