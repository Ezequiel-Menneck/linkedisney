use std::{fs::OpenOptions, io::Write, path::PathBuf};

/// Loga quando foi a última vez que truncamos (evita truncar toda hora).
const MAX_LOG_SIZE: u64 = 512 * 1024; // 512KB antes de rotacionar

fn log_path() -> PathBuf {
    crate::db::data_dir().join("linkedisney.log")
}

pub fn log(level: &str, msg: &str) {
    // sempre silencioso — logging nunca pode quebrar o app
    let dir = crate::db::data_dir();
    let path = log_path();
    let _existing_len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if _existing_len > MAX_LOG_SIZE {
        // rotação simples: guarda o último pedaço
        let _ = std::fs::rename(&path, dir.join("linkedisney.log.1"));
    }
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!("{ts} [{level}] {msg}\n");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }}

pub fn info(msg: &str) {
    log("INFO", msg);
}

pub fn warn(msg: &str) {
    log("WARN", msg);
}

pub fn error(msg: &str) {
    log("ERRO", msg);
}

pub fn debug(msg: &str) {
    log("DEBUG", msg);
}
