# Android NativeActivity 生命周期补丁

来源：crates.io 的 `winit` 0.30.13，Apache-2.0 许可（见 LICENSE）。
上游提交：`e9809ef54b18499bb4f2cac945719ecc2a61061b`。

本补丁只在 Android 后端范围内改了三处：

- 收到 `MainEvent::Destroy` 时退出事件循环。NativeActivity 的 `onDestroy`
  会等待 `android_main` 返回，忽略该事件会阻塞 UI 线程。
- Android 事件循环被丢弃时释放“单事件循环”守卫，使同一进程里后续创建的
  Activity 能够建立自己的事件循环。
- 缓存上一次的缩放因子，只有密度真正变化时才发出 `ScaleFactorChanged`；
  连续两次读取当前值无法检测出变化。

桌面端与 Web 运行路径不受影响。此处保留上游实现源码以保证构建可复现，
因此不需要修改 Cargo 的全局缓存。上游的示例、独立集成测试及其开发依赖、
文档图片、重复的 manifest/lockfile、Cargo 元数据以及叙述性文档均已省略；
changelog 模块改为链接上游文档，而不是内嵌发布说明。Apache-2.0 许可予以保留。

回归覆盖：`android/tests/SmokeInstrumentation.java` 会结束 Activity、确认 UI
线程仍能响应、再打开第二个 Activity 并验证原生编辑框仍可正常使用。
未打补丁的版本会在 `finish()` 之后立即出现 15 秒主线程超时。
