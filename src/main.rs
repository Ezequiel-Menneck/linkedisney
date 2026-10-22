mod ai;
mod auth;
mod db;
mod linkedin;
mod log;
mod models;
mod tui;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "linkedisney",
    about = "Seu ghostwriter de posts do LinkedIn com IA",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Configure perfil LinkedIn + conexão de IA
    Configure,
    /// Conversa iterativa com a IA para criar posts
    Generate,
    /// Posts prontos: postar, agendar, editar ou excluir
    Post,
    /// Login OAuth no LinkedIn (gera access token + author URN)
    Login,
}

fn main() {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();
    let res = match cli.command {
        None => tui::hub::HubApp::new().run(),
        Some(Commands::Configure) => tui::ConfigureApp::new().run(),
        Some(Commands::Generate) => tui::GenerateApp::new().run(),
        Some(Commands::Post) => tui::PostApp::new().run(),
        Some(Commands::Login) => do_login(),
    };
    if let Err(e) = res {
        eprintln!("erro: {e:#}");
        std::process::exit(1);
    }
}

fn do_login() -> Result<()> {
    let cfg = db::load_config().resolved();
    let logged = tui::block_on(auth::login(&cfg))?;
    let mut cfg = db::load_config();
    cfg.linkedin_access_token = logged.access_token;
    cfg.linkedin_author_urn = logged.author_urn.clone();
    db::save_config(&cfg).context("falha ao salvar config")?;
    println!(
        "conectado como {} · urn: {} · token expira em {}s\nsalvo em {}",
        logged.name,
        logged.author_urn,
        logged.expires_in,
        db::config_path().display()
    );
    Ok(())
}
