package org.nekosportsworld.tool;

import android.app.AlertDialog;
import android.app.NativeActivity;
import android.os.Bundle;
import android.text.InputType;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import android.widget.FrameLayout;

/** Android UI thread owns the real text editor; Rust owns application state. */
public final class MainActivity extends NativeActivity {
    static { System.loadLibrary("nekosportsworldtool"); }
    private AlertDialog editorDialog;
    private AlertDialog deviceInfoDialog;
    private boolean taskActive;
    private boolean resumed;

    private native void nativeSubmitEdit(long id, String value);
    private native void nativeSetInsets(int left, int top, int right, int bottom);
    private native void nativeDeviceInfo(boolean initial, String info, String error);

    public void requestDeviceInfo(boolean initial) {
        runOnUiThread(() -> {
            android.content.SharedPreferences preferences = getSharedPreferences("device_info", MODE_PRIVATE);
            if (isFinishing() || isDestroyed() || deviceInfoDialog != null) return;
            if (initial && preferences.getBoolean("asked", false)) return;
            String purpose = initial
                ? "允许后会保存到设备身份，用于后续登录和业务请求。"
                : "允许后只填写设备页，点击“保存”后用于后续登录和业务请求。";
            AlertDialog dialog = new AlertDialog.Builder(this)
                .setTitle("读取本机信息")
                .setMessage("是否允许读取本机品牌、型号和 Android 系统版本？\n\n" + purpose
                    + "品牌仅用于设备页展示。"
                    + "\n\n设备 UUID 继续沿用本应用首次生成并保存的值，不会更换。拒绝后仍可手动填写，也可在设备页再次读取。")
                .setPositiveButton("允许读取", (ignored, which) -> {
                    // First-run completion is acknowledged only after Rust saves it.
                    // If the Activity/process exits earlier, the next launch asks again.
                    if (!initial) preferences.edit().putBoolean("asked", true).apply();
                    try {
                        // These fields are read only after the user accepts this dialog.
                        org.json.JSONObject info = new org.json.JSONObject();
                        info.put("manufacturer", android.os.Build.MANUFACTURER);
                        info.put("model", android.os.Build.MODEL);
                        info.put("os_version", android.os.Build.VERSION.RELEASE);
                        nativeDeviceInfo(initial, info.toString(), "");
                    } catch (Exception error) {
                        nativeDeviceInfo(initial, "", "读取本机信息失败，请在设备页重试或手动填写");
                    }
                })
                .setNegativeButton("暂不读取", (ignored, which) -> {
                    preferences.edit().putBoolean("asked", true).apply();
                    nativeDeviceInfo(initial, "", "");
                })
                .create();
            dialog.setOnCancelListener(ignored -> {
                preferences.edit().putBoolean("asked", true).apply();
                nativeDeviceInfo(initial, "", "");
            });
            dialog.setOnDismissListener(ignored -> deviceInfoDialog = null);
            deviceInfoDialog = dialog;
            dialog.show();
        });
    }

    public void completeDeviceInfo() {
        getSharedPreferences("device_info", MODE_PRIVATE).edit().putBoolean("asked", true).apply();
    }

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        // NativeActivity's rendering surface must stay inside system bars.
        View content = findViewById(android.R.id.content);
        content.setOnApplyWindowInsetsListener((view, insets) -> {
            if (android.os.Build.VERSION.SDK_INT >= 30) {
                android.graphics.Insets bars = insets.getInsets(
                    WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                nativeSetInsets(bars.left, bars.top, bars.right, bars.bottom);
            } else {
                nativeSetInsets(insets.getSystemWindowInsetLeft(), insets.getSystemWindowInsetTop(),
                    insets.getSystemWindowInsetRight(), insets.getSystemWindowInsetBottom());
            }
            return insets;
        });
        content.requestApplyInsets();
        applySystemBarStyle();
    }

    @Override public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) applySystemBarStyle();
    }

    private void applySystemBarStyle() {
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            getWindow().getInsetsController().setSystemBarsAppearance(
                android.view.WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS |
                android.view.WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,
                android.view.WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS |
                android.view.WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS);
        } else {
            getWindow().getDecorView().setSystemUiVisibility(
                View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR);
        }
    }

    public void openEditor(long id, String initialValue, int kind) {
        runOnUiThread(() -> {
            if (isFinishing() || isDestroyed() || editorDialog != null) return;
            EditText input = new EditText(this);
            input.setId(android.R.id.edit);
            int inputType;
            switch (kind) {
                case 1: inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD; break;
                case 2: inputType = InputType.TYPE_CLASS_NUMBER | InputType.TYPE_NUMBER_FLAG_SIGNED; break;
                case 3: inputType = InputType.TYPE_CLASS_NUMBER | InputType.TYPE_NUMBER_FLAG_SIGNED | InputType.TYPE_NUMBER_FLAG_DECIMAL; break;
                default: inputType = InputType.TYPE_CLASS_TEXT;
            }
            input.setInputType(inputType);
            input.setSingleLine(true);
            input.setImeOptions(EditorInfo.IME_ACTION_DONE | EditorInfo.IME_FLAG_NO_EXTRACT_UI);
            input.setText(initialValue);
            input.selectAll();
            int margin = Math.round(20 * getResources().getDisplayMetrics().density);
            FrameLayout container = new FrameLayout(this);
            container.setPadding(margin, 0, margin, 0);
            container.addView(input, new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.WRAP_CONTENT));
            AlertDialog dialog = new AlertDialog.Builder(this)
                .setTitle(kind == 1 ? "输入密码" : "编辑内容")
                .setView(container)
                .setPositiveButton("确定", (ignored, which) -> nativeSubmitEdit(id, input.getText().toString()))
                .setNegativeButton("取消", null)
                .create();
            editorDialog = dialog;
            dialog.setOnDismissListener(ignored -> {
                InputMethodManager ime = (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
                ime.hideSoftInputFromWindow(input.getWindowToken(), 0);
                editorDialog = null;
            });
            input.setOnEditorActionListener((view, action, event) -> {
                if (action != EditorInfo.IME_ACTION_DONE) return false;
                nativeSubmitEdit(id, input.getText().toString());
                dialog.dismiss();
                return true;
            });
            dialog.setOnShowListener(ignored -> {
                input.requestFocus();
                dialog.getWindow().setSoftInputMode(
                    WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE |
                    WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
                input.post(() -> ((InputMethodManager) getSystemService(INPUT_METHOD_SERVICE))
                    .showSoftInput(input, InputMethodManager.SHOW_IMPLICIT));
            });
            dialog.show();
        });
    }

    public void setTaskActive(boolean active) {
        runOnUiThread(() -> {
            taskActive = active;
            updateScreenPolicy();
        });
    }

    public void copyText(String text) {
        runOnUiThread(() -> {
            android.content.ClipboardManager clipboard =
                (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
            clipboard.setPrimaryClip(android.content.ClipData.newPlainText("NekoSportsWorldTool", text));
        });
    }

    private void updateScreenPolicy() {
        if (taskActive && resumed) getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        else getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
    }

    @Override protected void onResume() {
        super.onResume();
        resumed = true;
        updateScreenPolicy();
    }

    @Override protected void onPause() {
        resumed = false;
        updateScreenPolicy();
        super.onPause();
    }

    @Override protected void onDestroy() {
        if (editorDialog != null) editorDialog.dismiss();
        if (deviceInfoDialog != null) deviceInfoDialog.dismiss();
        taskActive = false;
        updateScreenPolicy();
        super.onDestroy();
    }
}
