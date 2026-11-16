# Rust Voxel (kiểu Horizon)

Game voxel viết bằng Rust cho Android, **không dùng JVM**, khởi động nhanh. Đọc texture, model và (sau này) thế giới từ bản Minecraft Java của chính người dùng.

Mọi thứ tự viết: giải PNG, giải nén zip/deflate, JSON, EGL/GLES, vẽ chữ, dựng mesh, camera.
Phần launcher chạy JVM cũ nằm ở nhánh `launcher-jvm`, game voxel đời đầu ở nhánh `voxel-game`.

## Lộ trình
- **G1 (đây)**: đọc client.jar, giải PNG (`src/png.rs`), dựng atlas, đọc model khối thật (`src/assets.rs`), vẽ 3D bằng GLES (`src/view.rs`)
- G2: đọc thế giới Java thật (NBT, file .mca) và vẽ
- G3: ánh sáng, nước, lá, sương mù, bầu trời, dựng mesh đa luồng
- G4: vật lý và điều khiển giống Java
- G5: đập/đặt khối, túi đồ, chế tạo từ dữ liệu thật
- G6: sinh địa hình, mob, âm thanh

## Build
`git push` rồi lấy APK ở tab Actions, artifact `rustlauncher-apk` (hoặc `gh run download -n rustlauncher-apk`).

## Cấu trúc `src/`
`lib.rs` vòng lặp chính · `gles.rs` EGL/GLES · `gfx.rs` vẽ 2D · `screen.rs` menu · `launcher.rs` luồng nền · `mc.rs` tải client.jar · `zipr.rs`/`inflate.rs` đọc zip · `png.rs` · `json.rs` · `assets.rs` atlas và model · `scene.rs` cảnh thử · `view.rs` camera 3D · `math.rs`
