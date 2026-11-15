# Rust Voxel

Game khối vuông viết từ đầu bằng Rust, chạy trên Android. Không dùng mã nguồn hay tài nguyên của Minecraft:
địa hình, mesh, vật lý, điều khiển, giao diện đều tự viết. Chỉ dùng hai thư viện nền không thể thiếu:
`winit` (cửa sổ và cảm ứng) và `wgpu` (nói chuyện với GPU qua Vulkan).

## Build

Code trên Termux, `git push`, GitHub Actions build APK. Lấy APK ở tab **Actions** → run mới nhất → **Artifacts** → `rustlauncher-apk`.
Hoặc trong Termux: `gh run download -n rustlauncher-apk`.

## Điều khiển

- **Nửa trái màn hình**: đặt ngón tay rồi kéo để đi (cần điều khiển hiện ra).
- **Nửa phải màn hình**: kéo để xoay camera.
  - **Chạm nhanh** (không kéo): đặt khối đang chọn lên mặt khối đang nhắm.
  - **Giữ tay** (không kéo, khoảng 0,3 giây): đập khối đang nhắm; giữ tiếp thì đào liên tục.
- **Nút mũi tên lớn** (bên phải, trên thanh vật phẩm): nhảy. Khi đang bay thì giữ để bay lên.
- **Nút hai mũi tên** (góc trên phải): bật/tắt chế độ bay. Bay: đẩy cần tới theo hướng nhìn (nhìn xuống thì hạ độ cao).
- **Thanh vật phẩm** (dưới cùng): chạm để chọn loại khối.
- Khối đang nhắm có khung viền đen. Chỉ nhắm được trong tầm 5 khối. Lớp đáy không đập được.

## Cấu trúc

- `src/world.rs`: sinh địa hình bằng noise, chunk theo yêu cầu, sửa khối, dựng mesh (chỉ vẽ mặt lộ ra), khung viền. Có test.
- `src/game.rs`: người chơi, va chạm, trọng lực, nhảy, tia nhắm khối, đập/đặt khối, cảm ứng, camera. Có test.
- `src/ui.rs`: bố cục và vẽ nút, thanh vật phẩm, tâm ngắm, cần điều khiển. Có test.
- `src/gpu.rs`: wgpu, pipeline, sương mù, mesh chunk động, giao diện trong suốt.
- `src/lib.rs`: vòng lặp ứng dụng Android, nạp/bỏ chunk theo vị trí người chơi.

Test logic (không cần Android, không cần mạng): đặt `world.rs`, `game.rs`, `ui.rs` vào một thư mục cùng file
`t.rs` chứa `mod world; mod game; mod ui; fn main(){}` rồi `rustc --edition 2021 --test t.rs && ./t`.

## Giới hạn hiện tại

- Dữ liệu khối của chunk đã sinh được giữ trong bộ nhớ (để không mất phần đã sửa), chưa lưu ra file: thoát app là mất.
- Chưa có texture, ánh sáng, nước, cây, âm thanh. Màu khối là màu phẳng đổ bóng theo hướng mặt.

## Lộ trình

1. Địa hình + camera + cảm ứng (xong)
2. Va chạm, nhảy, bay, đập/đặt khối, thế giới vô hạn, thanh vật phẩm (đây)
3. Lưu/tải thế giới ra file, nhiều chunk hơn trên luồng riêng
4. Texture tự vẽ (atlas), ánh sáng/AO, nước, cây
5. Âm thanh, sinh vật, túi đồ, chế tạo

## Chữ ký APK (tuỳ chọn)

Mặc định mỗi lần build tạo keystore mới, nên chữ ký APK các lần build khác nhau và phải gỡ bản cũ trước khi cài bản mới.
Muốn cài đè được, tạo keystore một lần rồi thêm 2 secret vào repo:

```sh
pkg install openjdk-17
keytool -genkey -v -keystore release.keystore -alias key -keyalg RSA -keysize 2048 -validity 10000
base64 -w0 release.keystore   # dán vào secret KEYSTORE_BASE64
```

Secret `KEYSTORE_PASSWORD` là mật khẩu keystore (key password phải giống store password).
