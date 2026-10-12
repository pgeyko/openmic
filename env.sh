#!/bin/sh
# OpenCam isolated env: SDK + JDK live here, rm -rf to clean.
# Usage: . ./env.sh
ROOT="$(cd "$(dirname "$0")" && pwd)"
export JAVA_HOME="$ROOT/.jdk/jdk-17.0.20.1+1"
export ANDROID_SDK_ROOT="$ROOT/.android-sdk"
export ANDROID_HOME="$ANDROID_SDK_ROOT"
export PATH="$JAVA_HOME/bin:$ANDROID_SDK_ROOT/cmdline-tools/latest/bin:$ANDROID_SDK_ROOT/platform-tools:$ANDROID_SDK_ROOT/build-tools/34.0.0:$PATH"
# Gradle (изолированный, без sudo): .gradle-dist/gradle-8.10.2/bin/gradle
[ -d "$ROOT/.gradle-dist/gradle-8.10.2/bin" ] && export PATH="$ROOT/.gradle-dist/gradle-8.10.2/bin:$PATH"
