//! Terminal prompts and command wiring. Telegram authorization and secret export stay in the client.
use crate::cli::{AccountCommand, Cli};
use market_telegram_client::{AccountExports, LoginStep, UserLogin};
use std::io::{IsTerminal, Write};
pub async fn manage(args: &Cli, command: &AccountCommand) -> anyhow::Result<()> {
    let AccountCommand::Login { output } = command;
    anyhow::ensure!(
        std::io::stdin().is_terminal(),
        "Run account login in an interactive terminal"
    );
    let output = match output {
        Some(path) => path.clone(),
        None => crate::account_config::export_path(args)
            .await?
            .unwrap_or_else(|| "accounts.json".into()),
    };
    // Validate and lock the destination before asking Telegram to send a code.
    let exports = AccountExports::open(&output)?;
    let id = match args
        .telegram_api_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(id) => id.to_owned(),
        None => prompt("Telegram API ID: ", false)?,
    };
    let id = id
        .trim()
        .parse::<i32>()
        .map_err(|_| anyhow::anyhow!("TELEGRAM_API_ID must be a positive integer"))?;
    let hash = match args
        .telegram_api_hash
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(hash) => hash.to_owned(),
        None => prompt("Telegram API hash: ", true)?.trim().to_owned(),
    };
    let scheduler = market_invocations::Runtime::start(Default::default())?;
    let result = async {
        let mut login = UserLogin::start(id, hash, scheduler.handle())?;
        let result = authorize(&mut login, exports, &output).await;
        login.close().await;
        result
    }
    .await;
    scheduler.shutdown().await;
    result
}
async fn authorize(
    login: &mut UserLogin,
    exports: AccountExports,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    let phone = prompt("Phone number (+country code): ", false)?
        .split_whitespace()
        .collect::<String>();
    login.request_code(phone).await?;
    eprintln!("Code requested. Check Telegram or the delivery method selected by Telegram.");
    let mut step = LoginStep::CodeRequired;
    loop {
        step = match step {
            LoginStep::CodeRequired | LoginStep::InvalidCode => {
                if matches!(step, LoginStep::InvalidCode) {
                    eprintln!(
                        "Invalid or expired code. Try again, or cancel to request a new code."
                    );
                }
                let code = prompt("Login code: ", true)?.trim().to_owned();
                anyhow::ensure!(!code.is_empty(), "Login cancelled: empty code");
                login.submit_code(code).await?
            }
            LoginStep::PasswordRequired | LoginStep::InvalidPassword => {
                if matches!(step, LoginStep::InvalidPassword) {
                    eprintln!("Incorrect two-factor password. Try again.");
                }
                let password = prompt("Two-factor password: ", true)?;
                anyhow::ensure!(!password.is_empty(), "Login cancelled: empty password");
                login.submit_password(password).await?
            }
            LoginStep::Authorized { user_id, username } => {
                exports.save(login)?;
                let identity = match username {
                    Some(name) => format!("@{name} (ID {user_id})"),
                    None => format!("ID {user_id}"),
                };
                println!("Saved user session for {identity} to {}", output.display());
                return Ok(());
            }
        };
    }
}

fn prompt(label: &str, secret: bool) -> anyhow::Result<String> {
    if secret {
        return rpassword::prompt_password(label)
            .map_err(|_| anyhow::anyhow!("Cannot read secret input from terminal"));
    }
    eprint!("{label}");
    std::io::stderr().flush()?;
    let mut input = String::new();
    anyhow::ensure!(
        std::io::stdin().read_line(&mut input)? > 0,
        "Login cancelled: end of input"
    );
    Ok(input.trim_end_matches(['\r', '\n']).to_owned())
}
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn login_accepts_credential_aliases_after_subcommands() {
        for (id_flag, hash_flag) in [
            ("--api-id", "--api-hash"),
            ("--telegram-api-id", "--telegram-api-hash"),
        ] {
            let args = Cli::try_parse_from([
                "market-app",
                "account",
                "login",
                id_flag,
                "12345",
                hash_flag,
                "0123456789abcdef0123456789abcdef",
            ])
            .unwrap();
            assert_eq!(args.telegram_api_id.as_deref(), Some("12345"));
            assert_eq!(
                args.telegram_api_hash.as_deref(),
                Some("0123456789abcdef0123456789abcdef")
            );
            assert!(matches!(
                args.command,
                Some(crate::cli::Command::Account { .. })
            ));
        }
    }

    #[test]
    fn login_parses_without_bot_or_hub_credentials() {
        let args = Cli::try_parse_from([
            "market-app",
            "account",
            "login",
            "--output",
            "new-accounts.json",
        ])
        .unwrap();
        let Some(crate::cli::Command::Account {
            command: AccountCommand::Login { output },
        }) = args.command
        else {
            panic!()
        };
        assert_eq!(
            output.unwrap(),
            std::path::PathBuf::from("new-accounts.json")
        );
    }
}
