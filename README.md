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
- **3 (đây)**: tải JRE 21 arm64, giải nén, chạy `java -version` ngay trong tiến trình app (`src/jre.rs`, `src/tarx.rs`, `src/jvm.rs`)
- 4: cầu native + tải vanilla và mở cửa sổ game đầu tiên
- 5: cảm ứng, đăng nhập Microsoft, chọn phiên bản

Game voxel tự viết cũ nằm ở nhánh `voxel-game`.

## Build
`git push` rồi lấy APK ở tab Actions, artifact `rustlauncher-apk` (hoặc `gh run download -n rustlauncher-apk`).

## Test (không cần Android)
`tarx.rs` và `jvm.rs` chỉ dùng std: `rustc --edition 2021 --test src/tarx.rs && ./tarx`.
Test `jvm.rs` đã được chạy với JDK 21 thật trên máy dev (JLI_Launch, bắt stdout/stderr).

## Giới hạn
- Mỗi tiến trình chỉ chạy được một JVM; muốn chạy lại phải mở lại app.
- Lỗi nặng trong JLI có thể làm thoát cả app.
