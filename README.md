# Rust Launcher

Launcher Minecraft Java cho Android, viết bằng Rust (egui). Build APK bằng GitHub Actions, code trên Termux.

## Đẩy lên GitHub (trong Termux)

```sh
pkg install git
cd rustlauncher
git init -b main
git add .
git commit -m "init"
git remote add origin https://github.com/<user>/<repo>.git
git push -u origin main
```

Mỗi lần push lên `main`, workflow `Build APK` chạy. Lấy APK ở tab **Actions** → run mới nhất → **Artifacts** → `rustlauncher-apk`.
Cũng có thể bấm **Run workflow** để build tay.

## Chữ ký APK (tuỳ chọn)

Mặc định mỗi lần build tạo keystore mới, nên APK các lần build có chữ ký khác nhau và phải gỡ bản cũ trước khi cài bản mới.
Muốn cài đè được, tạo keystore một lần rồi thêm 2 secret vào repo:

```sh
pkg install openjdk-17
keytool -genkey -v -keystore release.keystore -alias key -keyalg RSA -keysize 2048 -validity 10000
base64 -w0 release.keystore   # dán vào secret KEYSTORE_BASE64
```

Secret `KEYSTORE_PASSWORD` là mật khẩu keystore (key password phải giống store password).

## Lộ trình

1. Khung repo + build APK (đang ở đây)
2. Core: tải manifest/libraries/assets song song, cache hash
3. Tích hợp JVM + runtime GL, chạy bản vanilla
4. UI đầy đủ, tài khoản, mod loader
