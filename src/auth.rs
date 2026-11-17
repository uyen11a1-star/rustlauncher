//! Đăng nhập Ely.by (mật khẩu) và Microsoft (mã thiết bị) trên luồng nền; tiến trình báo qua `Status`.
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::{json, Value};

use crate::{
    account::{Account, Kind},
    mc,
};

#[derive(Clone)]
pub enum Status {
    Idle,
    Working(String),
    Code { code: String, url: String },
    NeedTotp,
    Done(Account),
    Failed(String),
}

pub type Shared = Arc<Mutex<Status>>;

pub fn new_shared() -> Shared {
    Arc::new(Mutex::new(Status::Idle))
}

enum AErr {
    Totp,
    Msg(String),
}

impl From<reqwest::Error> for AErr {
    fn from(e: reqwest::Error) -> AErr {
        AErr::Msg(format!("Lỗi mạng: {e}"))
    }
}

fn set(st: &Shared, s: Status) {
    *st.lock().unwrap() = s;
}

fn run<F: std::future::Future<Output = Result<Account, AErr>> + Send + 'static>(st: Shared, f: F) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(r) => r,
            Err(e) => {
                set(&st, Status::Failed(format!("Không tạo được luồng mạng: {e}")));
                return;
            }
        };
        match rt.block_on(f) {
            Ok(a) => set(&st, Status::Done(a)),
            Err(AErr::Totp) => set(&st, Status::NeedTotp),
            Err(AErr::Msg(m)) => set(&st, Status::Failed(m)),
        }
    });
}

fn client_token() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("rl{:032x}", n ^ ((std::process::id() as u128) << 64))
}

async fn post_json(c: &reqwest::Client, url: &str, body: &Value) -> Result<(u16, Value), AErr> {
    let r = c.post(url).header("Accept", "application/json").json(body).send().await?;
    let code = r.status().as_u16();
    let v = r.json::<Value>().await.unwrap_or(Value::Null);
    Ok((code, v))
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// Ely.by: `totp` để trống nếu tài khoản không bật xác thực 2 lớp.
pub fn ely_login(st: Shared, user: String, pass: String, totp: String) {
    set(&st, Status::Working("Đang đăng nhập Ely.by...".into()));
    run(st, async move {
        let c = mc::make_client().map_err(|e| AErr::Msg(e.to_string()))?;
        let password = if totp.is_empty() { pass } else { format!("{pass}:{totp}") };
        let ct = client_token();
        let body = json!({ "username": user, "password": password, "clientToken": ct, "requestUser": false });
        let (code, v) = post_json(&c, "https://authserver.ely.by/auth/authenticate", &body).await?;
        if code == 200 {
            let p = v.get("selectedProfile").cloned().unwrap_or(Value::Null);
            let (name, uuid, token) = (text(&p, "name"), text(&p, "id"), text(&v, "accessToken"));
            if name.is_empty() || token.is_empty() {
                return Err(AErr::Msg("Ely.by không trả về hồ sơ người chơi".into()));
            }
            return Ok(Account { kind: Kind::ElyBy, name, uuid, token, refresh: String::new(), extra: ct });
        }
        let msg = text(&v, "errorMessage");
        if msg.to_lowercase().contains("two factor") {
            return Err(AErr::Totp);
        }
        Err(AErr::Msg(if msg.is_empty() { format!("Ely.by trả về mã {code}") } else { msg }))
    });
}

/// Microsoft: luồng mã thiết bị → Xbox Live → XSTS → Minecraft Services.
pub fn ms_login(st: Shared, cid: String) {
    set(&st, Status::Working("Đang xin mã đăng nhập...".into()));
    let st2 = st.clone();
    run(st, async move {
        let c = mc::make_client().map_err(|e| AErr::Msg(e.to_string()))?;
        let scope = "XboxLive.signin offline_access";
        let r = c
            .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode")
            .form(&[("client_id", cid.as_str()), ("scope", scope)])
            .send()
            .await?;
        let dc = r.json::<Value>().await.unwrap_or(Value::Null);
        let (device, code, url) = (text(&dc, "device_code"), text(&dc, "user_code"), text(&dc, "verification_uri"));
        if device.is_empty() {
            let d = text(&dc, "error_description");
            return Err(AErr::Msg(if d.is_empty() { "Microsoft từ chối Client ID".into() } else { d }));
        }
        set(&st2, Status::Code { code, url });
        let mut every = dc.get("interval").and_then(|x| x.as_u64()).unwrap_or(5);
        let limit = Duration::from_secs(dc.get("expires_in").and_then(|x| x.as_u64()).unwrap_or(900));
        let t0 = Instant::now();
        let tok = loop {
            if t0.elapsed() > limit {
                return Err(AErr::Msg("Mã đã hết hạn, hãy thử lại".into()));
            }
            tokio::time::sleep(Duration::from_secs(every + 1)).await;
            let r = c
                .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
                .form(&[
                    ("client_id", cid.as_str()),
                    ("scope", scope),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", device.as_str()),
                ])
                .send()
                .await?;
            let ok = r.status().as_u16() == 200;
            let v = r.json::<Value>().await.unwrap_or(Value::Null);
            if ok {
                break v;
            }
            match text(&v, "error").as_str() {
                "authorization_pending" => {}
                "slow_down" => every += 5,
                "authorization_declined" => return Err(AErr::Msg("Bạn đã từ chối đăng nhập".into())),
                other => return Err(AErr::Msg(format!("Microsoft báo lỗi: {other}"))),
            }
        };
        set(&st2, Status::Working("Đang xác thực với Xbox và Minecraft...".into()));
        let (access, refresh) = (text(&tok, "access_token"), text(&tok, "refresh_token"));

        let xbl = json!({
            "Properties": { "AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={access}") },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        });
        let (code, v) = post_json(&c, "https://user.auth.xboxlive.com/user/authenticate", &xbl).await?;
        let xbl_token = text(&v, "Token");
        if code != 200 || xbl_token.is_empty() {
            return Err(AErr::Msg(format!("Xbox Live từ chối (mã {code})")));
        }

        let xsts = json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT"
        });
        let (code, v) = post_json(&c, "https://xsts.auth.xboxlive.com/xsts/authorize", &xsts).await?;
        let xsts_token = text(&v, "Token");
        let uhs = v.pointer("/DisplayClaims/xui/0/uhs").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if code != 200 || xsts_token.is_empty() || uhs.is_empty() {
            let x = v.get("XErr").map(|x| x.to_string()).unwrap_or_else(|| code.to_string());
            return Err(AErr::Msg(format!("Xbox từ chối (XErr {x})")));
        }

        let body = json!({ "identityToken": format!("XBL3.0 x={uhs};{xsts_token}") });
        let (code, v) = post_json(&c, "https://api.minecraftservices.com/authentication/login_with_xbox", &body).await?;
        let mc_token = text(&v, "access_token");
        if code != 200 || mc_token.is_empty() {
            let m = text(&v, "errorMessage");
            return Err(AErr::Msg(format!("Minecraft từ chối (mã {code}) {m}")));
        }

        let r = c.get("https://api.minecraftservices.com/minecraft/profile").bearer_auth(&mc_token).send().await?;
        let v = r.json::<Value>().await.unwrap_or(Value::Null);
        let (name, uuid) = (text(&v, "name"), text(&v, "id"));
        if name.is_empty() {
            return Err(AErr::Msg("Không lấy được hồ sơ Minecraft (tài khoản có thể chưa sở hữu Java Edition)".into()));
        }
        Ok(Account { kind: Kind::Microsoft, name, uuid, token: mc_token, refresh, extra: cid })
    });
}
