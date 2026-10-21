# UI: teal-themed card-based redesign (Windows + Android)

## Summary
This PR rebuilds the UI layer to match the design spec (screenshots below) while keeping **all business logic unchanged**. It targets both the Windows desktop build and the Android build from the single shared `eframe/egui` codebase.

## What changed (UI only)
- `src/ui/theme.rs`: design tokens — teal accent scale, 5-step type ramp, 4pt spacing grid, radius tiers, card shadow.
- `src/ui/components.rs` (new): shared `card` / `chip` / `segmented` / `notice` / `stat` components.
- `src/ui/mobile.rs`: grouped side nav for desktop, bottom 5-tab nav for phones, sub-page back header; responsive tiers (phone / tablet / desktop).
- `src/ui/mod.rs`: login moved into a modal, top status chips, app skeleton.
- Card-ified all 9 pages: run, AI, records, data, user, device, log, osm (map), about.

## Post-review fixes
- **Android navigation**: the tool pages (设备信息 / 运行日志 / 路网 / 关于与更新) are reachable from the 我的 page as a 更多设置 section that opens a sub-page with a `← 我的` back bar. All 9 pages stay reachable, exactly as before — UI form only, no logic changes.
- **Android blurry rendering**: some devices report the default density (160) through `AConfiguration`, so egui rendered at 1x and the whole UI looked blurry. The app now publishes the authoritative `DisplayMetrics.density` from the activity (refreshed on configuration changes) and the vendored winit prefers it in `scale_factor()`.
  Files: `src/android.rs`, `android/java/org/nekosportsworld/tool/MainActivity.java`, `vendor/winit-0.30.13/src/platform_impl/android/mod.rs`.
- **Compact top bar**: the status chips + login button no longer collapse into a vertical column on narrow screens; on phones the bar is split into two rows.

## Unchanged (verified)
- No protocol / api / crypto / track / update logic touched.
- `src/ui/osm.rs`, `src/ui/map.rs`, and `crates/route-planner` are **original** to the repo — this PR only restyles them, it does not add routing / network features.
- `cargo test --lib`: 90 passed.

## Screenshots

### Windows (desktop, 1218×764)
| 跑步 | AI 教练 |
|---|---|
| ![run](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/win_run.png) | ![ai](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/win_ai.png) |

### Android (phone layout, 378×844)
| 首页（底部 5 栏） | 我的（更多设置入口） | 设备信息二级页 |
|---|---|---|
| ![phone home](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/phone_home.png) | ![phone user](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/phone_user.png) | ![phone device](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/phone_device.png) |

### Design mockup (`docs/ui-redesign/preview.html`, kept out of the repo per `.gitignore`)
| Desktop | Android |
|---|---|
| ![desktop mockup](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/preview_desktop.png) | ![android mockup](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/preview_android.png) |

## Test plan
- [x] Windows: run `nekosportsworldtool.exe`, check all 9 pages render with the teal theme.
- [x] Android: install the arm64 apk, verify bottom nav + cards on a phone.
- [x] Confirm login modal, status chips, side / bottom nav behave functionally as before.
- [x] All 9 pages reachable on Android (tool pages under 我的 → 更多设置).
