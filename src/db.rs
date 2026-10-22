use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

use crate::models::{Config, Fact, Post, Profile, FACT, PREF};

/// Procura a raiz do projeto (pasta com Cargo.toml ou .git) a partir do cwd.
fn project_root() -> PathBuf {
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if dir.join("Cargo.toml").exists() || dir.join(".git").exists() {
            return dir;
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => return std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

pub fn data_dir() -> PathBuf {
    let dir = project_root().join(".linkedisney");
    fs::create_dir_all(&dir).expect("falha ao criar .linkedisney");
    dir
}

pub fn db_path() -> PathBuf {
    data_dir().join("linkedisney.db")
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

pub fn connect() -> Result<Connection> {
    let conn = Connection::open(db_path()).context("falha ao abrir sqlite")?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS profile (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            name TEXT NOT NULL DEFAULT '',
            role TEXT NOT NULL DEFAULT '',
            bio TEXT NOT NULL DEFAULT '',
            industry TEXT NOT NULL DEFAULT '',
            audience TEXT NOT NULL DEFAULT '',
            topics TEXT NOT NULL DEFAULT '',
            tone TEXT NOT NULL DEFAULT '',
            language TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS posts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'draft',
            scheduled_at TEXT,
            posted_at TEXT,
            linkedin_post_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )?;

    // Migração da v1 (schema antigo com headline/about/style_rules/cta_preference).
    let cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(profile)")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    if cols.iter().any(|c| c == "headline") {
        create_profile_new(conn)?;
        let _ = conn.execute_batch(
            "INSERT OR REPLACE INTO profile_new (id, name, role, bio, industry, audience, topics, tone, language)
             SELECT id, name, headline, about, industry, audience, style_rules, tone, 'português' FROM profile WHERE id = 1;
             DROP TABLE profile;
             ALTER TABLE profile_new RENAME TO profile;",
        );
    }

    // Campos novos do perfil (dossiê: Voice/Strategy) — compatível com DBs existentes.
    let cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(profile)")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (col, ddl) in [
        ("voice_prefs", "ALTER TABLE profile ADD COLUMN voice_prefs TEXT NOT NULL DEFAULT ''"),
        ("goals", "ALTER TABLE profile ADD COLUMN goals TEXT NOT NULL DEFAULT ''"),
    ] {
        if !cols.iter().any(|c| c == col) {
            conn.execute_batch(&format!("{ddl};"))?;
        }
    }

    // Base de conhecimento pessoal (fatos + preferências editoriais).
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS facts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL DEFAULT 'fact',
            content TEXT NOT NULL,
            topics TEXT NOT NULL DEFAULT '',
            origin TEXT NOT NULL DEFAULT 'user',
            verified INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn create_profile_new(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS profile_new (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            name TEXT NOT NULL DEFAULT '',
            role TEXT NOT NULL DEFAULT '',
            bio TEXT NOT NULL DEFAULT '',
            industry TEXT NOT NULL DEFAULT '',
            audience TEXT NOT NULL DEFAULT '',
            topics TEXT NOT NULL DEFAULT '',
            tone TEXT NOT NULL DEFAULT '',
            language TEXT NOT NULL DEFAULT ''
        );",
    )?;
    Ok(())
}

pub fn load_config() -> Config {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(cfg: &Config) -> Result<()> {
    fs::write(config_path(), serde_json::to_string_pretty(cfg)?)?;
    Ok(())
}

pub fn load_profile() -> Profile {
    match connect() {
        Ok(conn) => conn
            .query_row(
                "SELECT name, role, bio, industry, audience, topics, tone, language, voice_prefs, goals FROM profile WHERE id = 1",
                [],
                |r| {
                    Ok(Profile {
                        name: r.get(0)?,
                        role: r.get(1)?,
                        bio: r.get(2)?,
                        industry: r.get(3)?,
                        audience: r.get(4)?,
                        topics: r.get(5)?,
                        tone: r.get(6)?,
                        language: r.get(7)?,
                        voice_prefs: r.get(8)?,
                        goals: r.get(9)?,
                    })
                },
            )
            .unwrap_or_default(),
        Err(_) => Profile::default(),
    }
}

pub fn save_profile(p: &Profile) -> Result<()> {
    let conn = connect()?;
    conn.execute(
        "INSERT INTO profile (id, name, role, bio, industry, audience, topics, tone, language, voice_prefs, goals)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET name=?1, role=?2, bio=?3, industry=?4, audience=?5, topics=?6, tone=?7, language=?8, voice_prefs=?9, goals=?10",
        params![p.name, p.role, p.bio, p.industry, p.audience, p.topics, p.tone, p.language, p.voice_prefs, p.goals],
    )?;
    Ok(())
}

const POST_COLS: &str =
    "id, content, status, scheduled_at, posted_at, created_at, updated_at";

fn row_to_post(r: &rusqlite::Row) -> rusqlite::Result<Post> {
    let dt = |i: usize| -> DateTime<Utc> {
        r.get::<_, String>(i).ok().and_then(|s| parse_ts(s).ok()).unwrap_or_else(Utc::now)
    };
    Ok(Post {
        id: r.get(0)?,
        content: r.get(1)?,
        status: r.get(2)?,
        scheduled_at: r.get::<_, Option<String>>(3)?.and_then(|s| parse_ts(s).ok()),
        posted_at: r.get::<_, Option<String>>(4)?.and_then(|s| parse_ts(s).ok()),
        created_at: dt(5),
        updated_at: dt(6),
    })
}

fn parse_ts(s: String) -> Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(&s).map(|d| d.with_timezone(&Utc))
}

pub fn insert_post(content: &str) -> Result<i64> {
    let conn = connect()?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO posts (content, status, created_at, updated_at) VALUES (?1, 'draft', ?2, ?2)",
        params![content, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_posts(statuses: &[&str]) -> Result<Vec<Post>> {
    let conn = connect()?;
    let placeholders = statuses.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT {POST_COLS} FROM posts WHERE status IN ({placeholders}) ORDER BY updated_at DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let posts = stmt
        .query_map(rusqlite::params_from_iter(statuses.iter()), row_to_post)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(posts)
}

pub fn get_post(id: i64) -> Result<Option<Post>> {
    let conn = connect()?;
    let sql = format!("SELECT {POST_COLS} FROM posts WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query_map([id], row_to_post)?;
    Ok(rows.next().transpose()?)
}

pub fn update_post_content(id: i64, content: &str) -> Result<()> {
    let conn = connect()?;
    conn.execute(
        "UPDATE posts SET content = ?1, updated_at = ?2 WHERE id = ?3",
        params![content, Utc::now().to_rfc3339(), id],
    )?;
    Ok(())
}

pub fn update_post_status_raw(id: i64, status: &str) -> Result<()> {
    let conn = connect()?;
    conn.execute(
        "UPDATE posts SET status = ?1, scheduled_at = NULL, updated_at = ?2 WHERE id = ?3",
        params![status, Utc::now().to_rfc3339(), id],
    )?;
    Ok(())
}

pub fn schedule_post(id: i64, at: DateTime<Utc>) -> Result<()> {
    let conn = connect()?;
    conn.execute(
        "UPDATE posts SET status = 'scheduled', scheduled_at = ?1, updated_at = ?2 WHERE id = ?3",
        params![at.to_rfc3339(), Utc::now().to_rfc3339(), id],
    )?;
    Ok(())
}

pub fn mark_posted(id: i64, linkedin_id: &str) -> Result<()> {
    let conn = connect()?;
    conn.execute(
        "UPDATE posts SET status = 'posted', posted_at = ?1, linkedin_post_id = ?2, updated_at = ?1 WHERE id = ?3",
        params![Utc::now().to_rfc3339(), linkedin_id, id],
    )?;
    Ok(())
}

pub fn delete_post(id: i64) -> Result<()> {
    let conn = connect()?;
    conn.execute("DELETE FROM posts WHERE id = ?1", [id])?;
    Ok(())
}

// ---------- base de conhecimento (fatos + preferências) ----------

const FACT_COLS: &str = "id, kind, content, topics";

fn row_to_fact(r: &rusqlite::Row) -> rusqlite::Result<Fact> {
    Ok(Fact {
        id: r.get(0)?,
        kind: r.get(1)?,
        content: r.get(2)?,
        topics: r.get(3)?,
    })
}

pub fn insert_fact(kind: &str, content: &str, topics: &str) -> Result<i64> {
    let kind = match kind {
        PREF => PREF,
        _ => FACT,
    };
    let conn = connect()?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO facts (kind, content, topics, origin, verified, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'user', 1, ?4, ?4)",
        params![kind, content, topics, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_facts() -> Result<Vec<Fact>> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {FACT_COLS} FROM facts ORDER BY id ASC"
    ))?;
    let facts = stmt.query_map([], row_to_fact)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(facts)
}

pub fn update_fact(id: i64, content: &str, topics: &str) -> Result<()> {
    let conn = connect()?;
    conn.execute(
        "UPDATE facts SET content = ?1, topics = ?2, updated_at = ?3 WHERE id = ?4",
        params![content, topics, Utc::now().to_rfc3339(), id],
    )?;
    Ok(())
}

pub fn delete_fact(id: i64) -> Result<()> {
    let conn = connect()?;
    conn.execute("DELETE FROM facts WHERE id = ?1", [id])?;
    Ok(())
}
