# Rust Voxel

Game khối vuông viết từ đầu bằng Rust, chạy trên Android. Không dùng mã nguồn hay tài nguyên của Minecraft:
địa hình, mesh, camera, điều khiển đều tự viết. Chỉ dùng hai thư viện nền không thể thiếu:
`winit` (cửa sổ và cảm ứng) và `wgpu` (nói chuyện với GPU qua Vulkan).

## Build

Code trên Termux, `git push`, GitHub Actions build APK. Lấy APK ở tab **Actions** → run mới nhất → **Artifacts** → `rustlauncher-apk`.
Hoặc trong Termux: `gh run download -n rustlauncher-apk`.

## Điều khiển (mốc 1)

- Nửa trái màn hình: đặt ngón tay rồi kéo để đi (như cần điều khiển).
- Nửa phải màn hình: kéo để xoay camera.

## Cấu trúc

- `src/world.rs`: sinh địa hình bằng noise, chia chunk, dựng mesh (chỉ vẽ mặt lộ ra). Có test.
- `src/game.rs`: người chơi, cảm ứng, ma trận camera. Có test.
- `src/gpu.rs`: wgpu, pipeline, vẽ.
- `src/lib.rs`: vòng lặp ứng dụng Android.

Test logic (không cần Android): đặt `world.rs` và `game.rs` vào một thư mục cùng file
`t.rs` chứa `mod world; mod game; fn main(){}` rồi `rustc --edition 2021 --test t.rs && ./t`.

## Lộ trình

1. Địa hình + camera + cảm ứng (đang ở đây)
2. Va chạm, trọng lực, nhảy; đập và đặt khối (raycast), thanh vật phẩm
3. Texture tự vẽ (atlas), ánh sáng/AO, nước, tải chunk theo khoảng cách
4. Lưu/tải thế giới, âm thanh
5. Sinh vật, túi đồ, chế tạo

## Chữ ký APK (tuỳ chọn)

Mặc định mỗi lần build tạo keystore mới, nên chữ ký APK các lần build khác nhau và phải gỡ bản cũ trước khi cài bản mới.
Muốn cài đè được, tạo keystore một lần rồi thêm 2 secret vào repo:

```sh
pkg install openjdk-17
keytool -genkey -v -keystore release.keystore -alias key -keyalg RSA -keysize 2048 -validity 10000
base64 -w0 release.keystore   # dán vào secret KEYSTORE_BASE64
```

Secret `KEYSTORE_PASSWORD` là mật khẩu keystore (key password phải giống store password).
