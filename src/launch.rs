//! Dựng lệnh chạy Minecraft từ file version JSON của Mojang: luật (rules), tham số JVM/game, classpath.
use std::path::{Path, PathBuf};

use crate::json::{self, Value};

pub struct Opts {
    pub player_name: String,
    pub uuid: String,
    pub access_token: String,
    pub user_type: String,
    pub game_dir: PathBuf,
    pub assets_root: PathBuf,
    pub natives_dir: PathBuf,
    /// Jar thay cho các `org.lwjgl:*` của Mojang (bản LWJGL đã vá cho Android), đặt trước classpath.
    pub lwjgl_jars: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct Plan {
    pub java_major: u32,
    pub main_class: String,
    /// Tuỳ chọn JVM đã thay biến, KHÔNG gồm -cp và java.library.path.
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    pub classpath: Vec<PathBuf>,
    pub asset_index: String,
}

const OS_NAME: &str = "linux";
const OS_ARCH: &str = "arm64";

/// Một luật có áp dụng cho máy này không? Mọi `features` coi như tắt (không demo, không độ phân giải tuỳ chỉnh...).
fn rule_matches(rule: &Value) -> bool {
    if let Some(os) = rule.get("os") {
        if let Some(n) = os.get("name").and_then(|v| v.as_str()) {
            if n != OS_NAME {
                return false;
            }
        }
        if let Some(a) = os.get("arch").and_then(|v| v.as_str()) {
            if a != OS_ARCH {
                return false;
            }
        }
        // os.version chỉ dùng cho Windows 10, không áp dụng ở đây.
        if os.get("version").is_some() {
            return false;
        }
    }
    if let Some(Value::Obj(f)) = rule.get("features") {
        if f.values().any(|v| v.as_bool() == Some(true)) {
            return false;
        }
    }
    true
}

/// Danh sách luật rỗng = cho phép. Ngược lại: bắt đầu cấm, luật cuối cùng khớp quyết định.
pub fn rules_allow(rules: Option<&Value>) -> bool {
    let list = match rules.and_then(|r| r.as_arr()) {
        Some(l) if !l.is_empty() => l,
        _ => return true,
    };
    let mut allowed = false;
    for r in list {
        if rule_matches(r) {
            allowed = r.get("action").and_then(|a| a.as_str()) == Some("allow");
        }
    }
    allowed
}

fn collect_args(arr: Option<&Value>, out: &mut Vec<String>) {
    let Some(list) = arr.and_then(|a| a.as_arr()) else { return };
    for item in list {
        match item {
            Value::Str(s) => out.push(s.clone()),
            Value::Obj(_) => {
                if !rules_allow(item.get("rules")) {
                    continue;
                }
                match item.get("value") {
                    Some(Value::Str(s)) => out.push(s.clone()),
                    Some(Value::Arr(a)) => out.extend(a.iter().filter_map(|v| v.as_str().map(String::from))),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// Đường dẫn jar của một library (ưu tiên `downloads.artifact.path`, không có thì suy từ tên Maven).
fn lib_path(lib: &Value) -> Option<String> {
    if let Some(p) = lib.get("downloads").and_then(|d| d.get("artifact")).and_then(|a| a.get("path")).and_then(|p| p.as_str()) {
        return Some(p.to_string());
    }
    let name = lib.get("name")?.as_str()?;
    let mut it = name.split(':');
    let (g, a, v) = (it.next()?, it.next()?, it.next()?);
    Some(format!("{}/{a}/{v}/{a}-{v}.jar", g.replace('.', "/")))
}

fn subst(s: &str, vars: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        match rest[i..].find('}') {
            Some(j) => {
                let key = &rest[i + 2..i + j];
                if let Some((_, v)) = vars.iter().find(|(k, _)| *k == key) {
                    out.push_str(v);
                }
                rest = &rest[i + j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn read_version(root: &Path, id: &str) -> Result<Value, String> {
    let p = root.join("versions").join(id).join(format!("{id}.json"));
    let t = std::fs::read_to_string(&p).map_err(|e| format!("không đọc được {}: {e}", p.display()))?;
    json::parse(&t)
}

pub fn build_plan(root: &Path, id: &str, o: &Opts) -> Result<Plan, String> {
    let v = read_version(root, id)?;
    // Phiên bản "kế thừa" (Fabric, Forge...) chưa hỗ trợ ở mốc này.
    if v.get("inheritsFrom").is_some() {
        return Err("phiên bản kế thừa (inheritsFrom) chưa hỗ trợ".into());
    }
    let main_class = v.get("mainClass").and_then(|x| x.as_str()).ok_or("thiếu mainClass")?.to_string();
    let java_major = v
        .get("javaVersion")
        .and_then(|j| j.get("majorVersion"))
        .and_then(|m| m.as_f64())
        .map(|m| m as u32)
        .unwrap_or(8);
    let asset_index = v
        .get("assetIndex")
        .and_then(|a| a.get("id"))
        .and_then(|i| i.as_str())
        .unwrap_or(id)
        .to_string();

    // classpath: lwjgl vá -> thư viện (bỏ org.lwjgl của Mojang) -> client jar
    let mut classpath: Vec<PathBuf> = o.lwjgl_jars.clone();
    if let Some(libs) = v.get("libraries").and_then(|l| l.as_arr()) {
        for lib in libs {
            if !rules_allow(lib.get("rules")) {
                continue;
            }
            let name = lib.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name.starts_with("org.lwjgl") && !o.lwjgl_jars.is_empty() {
                continue;
            }
            // Bản native cũ (classifier) chưa hỗ trợ: chỉ nhận jar chính.
            if name.matches(':').count() >= 3 {
                continue;
            }
            if let Some(p) = lib_path(lib) {
                let full = root.join("libraries").join(p);
                if !classpath.contains(&full) {
                    classpath.push(full);
                }
            }
        }
    }
    classpath.push(root.join("versions").join(id).join(format!("{id}.jar")));

    let sep = ":";
    let cp_str = classpath.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(sep);
    let vars: Vec<(&str, String)> = vec![
        ("auth_player_name", o.player_name.clone()),
        ("version_name", id.to_string()),
        ("game_directory", o.game_dir.display().to_string()),
        ("assets_root", o.assets_root.display().to_string()),
        ("assets_index_name", asset_index.clone()),
        ("auth_uuid", o.uuid.clone()),
        ("auth_access_token", o.access_token.clone()),
        ("auth_session", o.access_token.clone()),
        ("clientid", String::new()),
        ("auth_xuid", String::new()),
        ("user_type", o.user_type.clone()),
        ("user_properties", "{}".into()),
        ("version_type", v.get("type").and_then(|t| t.as_str()).unwrap_or("release").to_string()),
        ("natives_directory", o.natives_dir.display().to_string()),
        ("launcher_name", "RustLauncher".into()),
        ("launcher_version", "0.4".into()),
        ("classpath", cp_str),
        ("classpath_separator", sep.into()),
        ("library_directory", root.join("libraries").display().to_string()),
        ("game_assets", o.assets_root.join("virtual/legacy").display().to_string()),
    ];

    let mut jvm_raw: Vec<String> = Vec::new();
    let mut game_raw: Vec<String> = Vec::new();
    if let Some(args) = v.get("arguments") {
        collect_args(args.get("jvm"), &mut jvm_raw);
        collect_args(args.get("game"), &mut game_raw);
    } else if let Some(m) = v.get("minecraftArguments").and_then(|m| m.as_str()) {
        game_raw = m.split_whitespace().map(String::from).collect();
    }

    // Bỏ -cp/-classpath và java.library.path (launcher tự đặt), bỏ tham số macOS còn sót.
    let mut jvm_args = Vec::new();
    let mut it = jvm_raw.into_iter().map(|a| subst(&a, &vars)).peekable();
    while let Some(a) = it.next() {
        if a == "-cp" || a == "-classpath" {
            it.next();
            continue;
        }
        if a.starts_with("-Djava.library.path=") || a.starts_with("-XstartOnFirstThread") {
            continue;
        }
        if a.is_empty() {
            continue;
        }
        jvm_args.push(a);
    }
    let game_args: Vec<String> = game_raw.into_iter().map(|a| subst(&a, &vars)).collect();

    Ok(Plan { java_major, main_class, jvm_args, game_args, classpath, asset_index })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "id":"1.21.10","type":"release","mainClass":"net.minecraft.client.main.Main",
      "javaVersion":{"component":"java-runtime-delta","majorVersion":21},
      "assetIndex":{"id":"27","sha1":"x","size":1,"url":"u"},
      "arguments":{
        "game":["--username","${auth_player_name}","--version","${version_name}","--gameDir","${game_directory}",
                "--assetsDir","${assets_root}","--assetIndex","${assets_index_name}","--uuid","${auth_uuid}",
                "--accessToken","${auth_access_token}","--userType","${user_type}",
                {"rules":[{"action":"allow","features":{"is_demo_user":true}}],"value":"--demo"},
                {"rules":[{"action":"allow","features":{"has_custom_resolution":true}}],"value":["--width","${resolution_width}"]}],
        "jvm":[
          {"rules":[{"action":"allow","os":{"name":"osx"}}],"value":["-XstartOnFirstThread"]},
          {"rules":[{"action":"allow","os":{"name":"windows"}}],"value":"-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump"},
          {"rules":[{"action":"allow","os":{"arch":"x86"}}],"value":"-Xss1M"},
          {"rules":[{"action":"allow","os":{"name":"linux"}}],"value":"-Dlinux.only=1"},
          "-Djava.library.path=${natives_directory}",
          "-Dminecraft.launcher.brand=${launcher_name}",
          "-cp","${classpath}"]
      },
      "libraries":[
        {"name":"org.lwjgl:lwjgl:3.3.3","downloads":{"artifact":{"path":"org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar","url":"u","sha1":"s","size":1}}},
        {"name":"org.lwjgl:lwjgl:3.3.3:natives-linux","rules":[{"action":"allow","os":{"name":"linux"}}],"downloads":{"artifact":{"path":"org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-linux.jar","url":"u","sha1":"s","size":1}}},
        {"name":"com.google.code.gson:gson:2.10.1","downloads":{"artifact":{"path":"com/google/code/gson/gson/2.10.1/gson-2.10.1.jar","url":"u","sha1":"s","size":1}}},
        {"name":"ca.weblite:java-objc-bridge:1.1","rules":[{"action":"allow","os":{"name":"osx"}}],"downloads":{"artifact":{"path":"ca/weblite/x.jar","url":"u","sha1":"s","size":1}}},
        {"name":"net.java.dev.jna:jna:5.14.0","rules":[{"action":"allow"},{"action":"disallow","os":{"name":"windows"}}],"downloads":{"artifact":{"path":"net/java/dev/jna/jna/5.14.0/jna-5.14.0.jar","url":"u","sha1":"s","size":1}}}
      ]
    }"#;

    fn opts() -> Opts {
        Opts {
            player_name: "Steve".into(),
            uuid: "uuid-1".into(),
            access_token: "0".into(),
            user_type: "legacy".into(),
            game_dir: "/g".into(),
            assets_root: "/r/assets".into(),
            natives_dir: "/n".into(),
            lwjgl_jars: vec!["/lw/lwjgl.jar".into()],
        }
    }

    fn plan_from(json_text: &str) -> Plan {
        let root = std::env::temp_dir().join(format!("launch_{}_{}", std::process::id(), json_text.len()));
        let dir = root.join("versions/1.21.10");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1.21.10.json"), json_text).unwrap();
        build_plan(&root, "1.21.10", &opts()).unwrap()
    }

    #[test]
    fn modern_plan() {
        let p = plan_from(SAMPLE);
        assert_eq!(p.java_major, 21);
        assert_eq!(p.main_class, "net.minecraft.client.main.Main");
        assert_eq!(p.asset_index, "27");
        // jvm: chỉ còn luật linux + brand; không còn -cp, java.library.path, macOS, windows, x86
        assert_eq!(p.jvm_args, vec!["-Dlinux.only=1", "-Dminecraft.launcher.brand=RustLauncher"]);
        // game: không có --demo / --width
        let g = p.game_args.join(" ");
        assert!(g.starts_with("--username Steve --version 1.21.10 --gameDir /g --assetsDir /r/assets --assetIndex 27 --uuid uuid-1 --accessToken 0 --userType legacy"), "{g}");
        assert!(!g.contains("--demo") && !g.contains("--width"));
        // classpath: lwjgl vá, gson, jna, client jar; không có lwjgl của Mojang, không có objc-bridge
        let names: Vec<String> = p.classpath.iter().map(|x| x.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["lwjgl.jar", "gson-2.10.1.jar", "jna-5.14.0.jar", "1.21.10.jar"]);
    }

    #[test]
    fn legacy_arguments_string() {
        let t = r#"{"id":"1.8.9","type":"release","mainClass":"net.minecraft.client.main.Main",
          "minecraftArguments":"--username ${auth_player_name} --version ${version_name} --gameDir ${game_directory}",
          "assetIndex":{"id":"1.8","sha1":"x","size":1,"url":"u"},"libraries":[]}"#;
        let p = plan_from(t);
        assert_eq!(p.java_major, 8);
        assert_eq!(p.game_args, vec!["--username", "Steve", "--version", "1.21.10", "--gameDir", "/g"]);
        assert!(p.jvm_args.is_empty());
    }

    #[test]
    fn rules_logic() {
        let v = json::parse(r#"[{"action":"allow"},{"action":"disallow","os":{"name":"linux"}}]"#).unwrap();
        assert!(!rules_allow(Some(&v)));
        let v = json::parse(r#"[{"action":"disallow"},{"action":"allow","os":{"name":"linux","arch":"arm64"}}]"#).unwrap();
        assert!(rules_allow(Some(&v)));
        assert!(rules_allow(None));
    }

    #[test]
    fn subst_edge_cases() {
        let vars = vec![("a", "1".to_string())];
        assert_eq!(subst("x${a}y${b}z${a", &vars), "x1yz${a");
        assert_eq!(subst("plain", &vars), "plain");
    }

    #[test]
    fn inherits_rejected() {
        let root = std::env::temp_dir().join(format!("launch_inh_{}", std::process::id()));
        let dir = root.join("versions/f");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f.json"), r#"{"inheritsFrom":"1.21","mainClass":"x"}"#).unwrap();
        assert!(build_plan(&root, "f", &opts()).is_err());
    }
}
