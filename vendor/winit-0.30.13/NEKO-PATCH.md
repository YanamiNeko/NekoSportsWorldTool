# Android NativeActivity lifecycle patch

Source: crates.io `winit` 0.30.13, Apache-2.0 (see LICENSE).
Upstream commit: `e9809ef54b18499bb4f2cac945719ecc2a61061b`.

Three changes are scoped to the Android backend:

- Exit the event loop on `MainEvent::Destroy`. NativeActivity's `onDestroy`
  waits for `android_main` to return, so ignoring the event blocks the UI thread.
- Release the one-event-loop guard when the Android event loop is dropped,
  allowing a later Activity to create its own loop in the same process.
- Cache the previous scale factor and emit `ScaleFactorChanged` only when
  density actually changes; reading the current value twice cannot detect changes.

The desktop and web runtime paths are unchanged. Upstream implementation sources
are kept here for reproducible builds; no modifications to Cargo's global cache
are needed. Upstream examples, standalone integration tests and their development
dependencies, documentation images, duplicate manifests/lockfiles, Cargo metadata,
and narrative documentation are omitted. The changelog module links to upstream
documentation instead of embedding release notes. The Apache-2.0 license is retained.

Regression coverage: `android/tests/SmokeInstrumentation.java` finishes the
Activity, checks that the UI thread responds, opens a second Activity and verifies
the native editor remains usable. The unpatched version fails with a 15-second
main-thread timeout immediately after `finish()`.
