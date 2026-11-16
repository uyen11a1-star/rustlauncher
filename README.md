# Rust Launcher

Launcher Minecraft Java cho Android, viết bằng Rust. Mục tiêu: chạy **Minecraft Java thật**.

## Nguyên tắc
Chỉ dùng đồ có sẵn của người khác cho phần bất khả thi tự viết:
1. JRE/JVM cho Android (bản dựng sẵn của AngelAuraMC, tải lúc chạy)
2. Cầu GLFW/cửa sổ/cảm ứng (LWJGL stub, mốc 4)
3. Lớp dịch OpenGL sang GLES (mốc 4)

Mọi thứ còn lại tự viết: tải và kiểm tra file, giải xz/tar, quản lý JRE, nạp JVM, tham số chạy, đăng nhập, giao diện.

## Mốc
- 1-2: giao diện, tải Minecraft song song (`src/mc.rs`) - xong
- 3: tải JRE, giải nén, tạo JVM ngay trong tiến trình app bằng JNI (`src/jre.rs`, `src/tarx.rs`, `src/jvm.rs`) - xong
- **4a (đây)**: chọn phiên bản, tải, dựng lệnh chạy từ version JSON (`src/json.rs`, `src/launch.rs`), gọi `main` thật của Minecraft
- **4b-1 (đây)**: bỏ eframe, shell tự viết: android-activity + EGL + GLES, giao diện vẽ tay (`src/gles.rs`, `src/gfx.rs`, `src/screen.rs`, `src/launcher.rs`)
- 4b-2: lớp GLFW tự viết (xuất hàm C `glfw*`) + natives LWJGL của Amethyst, game chạy tới lúc tạo cửa sổ
- 4b-3: lớp dịch OpenGL sang GLES tự viết để lên hình
- 5: cảm ứng, đăng nhập Microsoft, chọn phiên bản

Game voxel tự viết cũ nằm ở nhánh `voxel-game`.

## Build
`git push` rồi lấy APK ở tab Actions, artifact `rustlauncher-apk` (hoặc `gh run download -n rustlauncher-apk`).

## Test (không cần Android)
`tarx.rs`, `json.rs`, `launch.rs`, `jvm.rs` chỉ dùng std: tạo `t.rs` chứa `mod json; mod launch; mod tarx; mod jvm; fn main(){}` cạnh chúng rồi `rustc --edition 2021 --test t.rs && ./t`.
`jvm.rs` đã chạy với JDK 21 thật trên máy dev (JNI_CreateJavaVM, gọi `main`, bắt stdout/stderr).

## Giới hạn
- Mỗi tiến trình chỉ chạy được một JVM; muốn chạy lại phải mở lại app.
- Lỗi nặng trong JLI có thể làm thoát cả app.
